//! Native HTTP and secure-WebSocket transport boundary.

use std::{collections::BTreeSet, path::PathBuf, sync::Arc};

use axum::{
    Router,
    extract::{
        Path, Query, State, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Redirect, Response},
    routing::get,
};
use tokio::sync::Mutex;

use crate::{
    AccountId, ChallengeId, ConnectionId, GoogleOidcClient, Handle, InvitationId, LobbyId, MatchId,
    MatchService, NewOidcAttempt, Participants, SessionCookiePolicy, SubscriptionRegistry,
    SwitchyCommandJournal, accept_challenge_into_lobby, account_for_handle, assign_handle,
    cancel_lobby, claim_oidc_attempt, create_challenge, create_oidc_attempt, create_stored_session,
    generate_invitation, handle_for_account, link_google_identity, load_lobby,
    pending_challenges_for, redeem_invitation_token_into_lobby, resolve_session_token,
    revoke_session_token,
};
use pwmtf_protocol::{
    CommandEnvelope, MAX_FRAME_BYTES, MAX_SNAPSHOT_FRAME_BYTES, SnapshotEnvelope, negotiate_version,
};
use switchy_database::Database;
use thiserror::Error;
use tower_http::services::{ServeDir, ServeFile};

/// Canonical production web origin.
pub const CANONICAL_ORIGIN: &str = "https://pwmtf.hyperchad.dev";
/// OIDC callback path under the canonical origin.
pub const OIDC_CALLBACK_PATH: &str = "/auth/google/callback";
const OIDC_BINDING_COOKIE: &str = "__Host-pwmtf_oidc";
const OIDC_ATTEMPT_LIFETIME_MS: u64 = 10 * 60 * 1_000;
const SESSION_LIFETIME_MS: u64 = 7 * 24 * 60 * 60 * 1_000;
const INVITATION_LIFETIME_MS: u64 = 24 * 60 * 60 * 1_000;
const MAX_JSON_BYTES: usize = 1_024;

/// Runtime services required by native HTTP/OIDC/WebSocket entry points.
pub struct HttpState {
    db: Arc<dyn Database>,
    oidc: Arc<GoogleOidcClient>,
    sessions: SessionCookiePolicy,
    origins: BTreeSet<String>,
    subscriptions: Mutex<SubscriptionRegistry>,
    matches: Mutex<MatchService<SwitchyCommandJournal>>,
    web_root: Option<PathBuf>,
}

impl HttpState {
    /// Polls and durably applies every currently due authoritative deadline.
    ///
    /// # Errors
    ///
    /// Returns [`TransportError`] when a timeout cannot be durably accepted.
    pub async fn poll_due_deadlines(
        &self,
        now: crate::DeadlineMillis,
    ) -> Result<usize, TransportError> {
        let mut matches = self.matches.lock().await;
        let count = matches
            .poll_due_deadlines(now)
            .await
            .map_err(|_| TransportError::Recovery)?
            .len();
        drop(matches);
        Ok(count)
    }

    /// Returns the next currently scheduled deadline.
    pub async fn next_deadline(&self) -> Option<crate::ScheduledDeadline> {
        let matches = self.matches.lock().await;
        let deadline = matches.next_deadline().map(|(_, deadline)| deadline);
        drop(matches);
        deadline
    }

    /// Executes one already-authenticated transport command through the
    /// authoritative durable service and returns the resulting snapshot.
    ///
    /// This is the non-socket boundary used by impairment and transport tests.
    ///
    /// # Errors
    ///
    /// Returns [`TransportError`] for internal clock or snapshot failures.
    pub async fn apply_command_frame(
        &self,
        match_id: MatchId,
        account: AccountId,
        bytes: &[u8],
    ) -> Result<Option<SnapshotEnvelope>, TransportError> {
        match apply_transport_command(self, match_id, account, bytes).await? {
            TransportCommandResponse::Accepted { snapshot } => Ok(Some(snapshot)),
            TransportCommandResponse::Rejected => Ok(None),
        }
    }

    async fn load_match(
        &self,
        match_id: MatchId,
        participants: Participants,
        state: pwmtf_game_domain::MatchState,
        deadline: Option<crate::ScheduledDeadline>,
    ) -> Result<(), TransportError> {
        let mut matches = self.matches.lock().await;
        if matches.participants(match_id).is_none() {
            matches.insert_match(match_id, participants, state);
            matches
                .set_deadline(match_id, deadline)
                .map_err(|_| TransportError::Recovery)?;
        }
        drop(matches);
        Ok(())
    }

    /// Returns an initial/reconnect snapshot, recovering the durable match on
    /// first access.
    ///
    /// # Errors
    ///
    /// Returns [`TransportError`] for missing/corrupt durable records or
    /// unauthorized/mismatched participants.
    pub async fn reconnect_snapshot(
        &self,
        match_id: MatchId,
        account: AccountId,
        participants: Participants,
    ) -> Result<SnapshotEnvelope, TransportError> {
        if account != participants.player_one && account != participants.player_two {
            return Err(TransportError::InvalidSubscription);
        }
        let mut matches = self.matches.lock().await;
        match matches.participants(match_id) {
            Some(stored) if stored == participants => {}
            Some(_) => return Err(TransportError::InvalidSubscription),
            None => {
                let journal = SwitchyCommandJournal::new(Arc::clone(&self.db));
                let stored = journal
                    .participants(match_id)
                    .await
                    .map_err(|_| TransportError::MatchNotFound)?;
                if stored != participants {
                    return Err(TransportError::InvalidSubscription);
                }
                matches
                    .recover_match(match_id, participants)
                    .await
                    .map_err(|_| TransportError::Recovery)?
                    .ok_or(TransportError::MatchNotFound)?;
            }
        }
        let snapshot = matches
            .snapshot(match_id)
            .map_err(|_| TransportError::MatchNotFound)?;
        drop(matches);
        Ok(snapshot)
    }

    /// Recovers every durable match before serving traffic.
    ///
    /// # Errors
    ///
    /// Returns [`TransportError`] when any canonical row or command history is
    /// absent, malformed, or incompatible.
    pub async fn recover_all_matches(&self) -> Result<usize, TransportError> {
        let journal = SwitchyCommandJournal::new(Arc::clone(&self.db));
        let ids = journal
            .match_ids()
            .await
            .map_err(|_| TransportError::Recovery)?;
        let mut recovered = 0;
        for match_id in ids {
            let journal = SwitchyCommandJournal::new(Arc::clone(&self.db));
            let participants = journal
                .participants(match_id)
                .await
                .map_err(|_| TransportError::Recovery)?;
            let records = SwitchyCommandJournal::new(Arc::clone(&self.db));
            let has_commands = !crate::CommandJournal::load(&records, match_id)
                .await
                .map_err(|_| TransportError::Recovery)?
                .is_empty();
            if has_commands {
                let mut matches = self.matches.lock().await;
                matches
                    .recover_match(match_id, participants)
                    .await
                    .map_err(|_| TransportError::Recovery)?;
                drop(matches);
            } else {
                let initial = SwitchyCommandJournal::new(Arc::clone(&self.db));
                let (_, state, deadline) = initial
                    .initial_match(match_id)
                    .await
                    .map_err(|_| TransportError::Recovery)?;
                self.load_match(match_id, participants, state, deadline)
                    .await?;
            }
            recovered += 1;
        }
        Ok(recovered)
    }

    /// Creates production transport state with exact origin validation.
    #[must_use]
    pub fn production(db: Arc<dyn Database>, oidc: Arc<GoogleOidcClient>) -> Self {
        Self::new(db, oidc, [CANONICAL_ORIGIN])
    }

    /// Creates state with an explicit exact-origin allowlist for tests/local adapters.
    #[must_use]
    pub fn new(
        db: Arc<dyn Database>,
        oidc: Arc<GoogleOidcClient>,
        origins: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        let matches = MatchService::new(SwitchyCommandJournal::new(Arc::clone(&db)));
        Self {
            db,
            oidc,
            sessions: SessionCookiePolicy::production(),
            origins: origins.into_iter().map(Into::into).collect(),
            subscriptions: Mutex::new(SubscriptionRegistry::default()),
            matches: Mutex::new(matches),
            web_root: None,
        }
    }

    /// Configures the verified production web-bundle directory.
    #[must_use]
    pub fn with_web_root(mut self, path: impl Into<PathBuf>) -> Self {
        self.web_root = Some(path.into());
        self
    }

    /// Returns the configured production web-bundle directory.
    #[must_use]
    pub fn web_root(&self) -> Option<&std::path::Path> {
        self.web_root.as_deref()
    }

    /// Creates transport state with an already-constructed authoritative service.
    ///
    /// This supports deterministic integration/impairment tests and process
    /// startup recovery without introducing a second authority path.
    #[must_use]
    pub fn with_match_service(
        db: Arc<dyn Database>,
        oidc: Arc<GoogleOidcClient>,
        origins: impl IntoIterator<Item = impl Into<String>>,
        matches: MatchService<SwitchyCommandJournal>,
    ) -> Self {
        Self {
            db,
            oidc,
            sessions: SessionCookiePolicy::production(),
            origins: origins.into_iter().map(Into::into).collect(),
            subscriptions: Mutex::new(SubscriptionRegistry::default()),
            matches: Mutex::new(matches),
            web_root: None,
        }
    }
}

/// Builds the native HTTP/OIDC/WebSocket router.
pub fn router(state: Arc<HttpState>) -> Router {
    let web_root = state.web_root().map(PathBuf::from);
    let router = Router::new()
        .route("/healthz", get(health))
        .route("/auth/google/start", get(google_start))
        .route(OIDC_CALLBACK_PATH, get(google_callback))
        .route("/api/session", get(session_profile).delete(logout))
        .route("/api/profile/handle", axum::routing::put(set_handle))
        .route(
            "/api/challenges",
            get(list_pending_challenges).post(challenge_handle),
        )
        .route(
            "/api/challenges/{challenge_id}/accept",
            axum::routing::post(accept_challenge),
        )
        .route(
            "/api/invitations",
            axum::routing::post(create_invitation_link),
        )
        .route(
            "/api/invitations/redeem",
            axum::routing::post(redeem_invitation_link),
        )
        .route(
            "/api/lobbies/{lobby_id}",
            get(lobby_status).delete(cancel_waiting_lobby),
        )
        .route("/ws", get(websocket));
    let router = if let Some(root) = web_root {
        router.fallback_service(
            ServeDir::new(&root).not_found_service(ServeFile::new(root.join("index.html"))),
        )
    } else {
        router
    };
    router.with_state(state)
}

async fn health() -> &'static str {
    "ok"
}

#[derive(Debug, serde::Serialize)]
struct SessionProfile {
    account_id: String,
    handle: Option<String>,
}

async fn session_profile(
    State(state): State<Arc<HttpState>>,
    headers: HeaderMap,
) -> Result<axum::Json<SessionProfile>, TransportError> {
    let account = authenticated_account(&state, &headers).await?;
    let handle = handle_for_account(&*state.db, account).await?;
    Ok(axum::Json(session_profile_response(account, handle)))
}

async fn logout(
    State(state): State<Arc<HttpState>>,
    headers: HeaderMap,
) -> Result<Response, TransportError> {
    let cookie = headers
        .get(header::COOKIE)
        .and_then(|value| value.to_str().ok())
        .ok_or(TransportError::Unauthenticated)?;
    let token = state.sessions.parse_request(cookie)?;
    revoke_session_token(&*state.db, token.expose()).await?;
    let mut response = StatusCode::NO_CONTENT.into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_str(&state.sessions.clear_cookie())
            .map_err(|_| TransportError::Internal)?,
    );
    Ok(response)
}

async fn authenticated_account(
    state: &HttpState,
    headers: &HeaderMap,
) -> Result<AccountId, TransportError> {
    let cookie = headers
        .get(header::COOKIE)
        .and_then(|value| value.to_str().ok())
        .ok_or(TransportError::Unauthenticated)?;
    let token = state.sessions.parse_request(cookie)?;
    resolve_session_token(&*state.db, token.expose(), unix_millis()?)
        .await?
        .ok_or(TransportError::Unauthenticated)
}

async fn google_start(State(state): State<Arc<HttpState>>) -> Result<Response, TransportError> {
    let now = unix_millis()?;
    let attempt = create_oidc_attempt(
        &*state.db,
        now,
        now.saturating_add(OIDC_ATTEMPT_LIFETIME_MS),
    )
    .await?;
    let location = state.oidc.authorization_url(attempt.attempt());
    let mut response = Redirect::temporary(&location).into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_str(&oidc_binding_cookie(&attempt))
            .map_err(|_| TransportError::Internal)?,
    );
    Ok(response)
}

#[derive(Debug, serde::Deserialize)]
struct OidcCallbackQuery {
    code: String,
    state: String,
}

async fn google_callback(
    State(state): State<Arc<HttpState>>,
    Query(query): Query<OidcCallbackQuery>,
    headers: HeaderMap,
) -> Result<Response, TransportError> {
    let binding = parse_cookie(
        headers
            .get(header::COOKIE)
            .and_then(|value| value.to_str().ok())
            .ok_or(TransportError::Unauthenticated)?,
        OIDC_BINDING_COOKIE,
    )?;
    let (attempt_id, binding) = binding
        .split_once('.')
        .ok_or(TransportError::Unauthenticated)?;
    let claimed = claim_oidc_attempt(
        &*state.db,
        attempt_id,
        &query.state,
        binding,
        unix_millis()?,
    )
    .await?;
    let identity = state
        .oidc
        .exchange_callback(&query.code, &query.state, claimed.attempt())
        .await?;
    let account = account_id(&identity);
    link_google_identity(&*state.db, &identity, account).await?;
    let now = unix_millis()?;
    let token = create_stored_session(
        &*state.db,
        crate::Session {
            account,
            expires_at: now.saturating_add(SESSION_LIFETIME_MS),
            last_used_at: now,
        },
    )
    .await?;
    let mut response = Redirect::to("/").into_response();
    response.headers_mut().append(
        header::SET_COOKIE,
        HeaderValue::from_str(
            &state
                .sessions
                .set_cookie(&token, SESSION_LIFETIME_MS / 1_000)?,
        )
        .map_err(|_| TransportError::Internal)?,
    );
    response.headers_mut().append(
        header::SET_COOKIE,
        HeaderValue::from_static(
            "__Host-pwmtf_oidc=; Path=/; Max-Age=0; SameSite=Lax; Secure; HttpOnly",
        ),
    );
    Ok(response)
}

#[derive(Debug, serde::Deserialize)]
struct WebSocketQuery {
    match_id: u128,
    player_one: u128,
    player_two: u128,
}

async fn websocket(
    State(state): State<Arc<HttpState>>,
    Query(query): Query<WebSocketQuery>,
    headers: HeaderMap,
    upgrade: WebSocketUpgrade,
) -> Result<Response, TransportError> {
    validate_origin(&headers, &state.origins)?;
    let account = authenticated_account(&state, &headers).await?;
    let connection = ConnectionId::new(random_u128()?);
    let match_id = MatchId::new(query.match_id);
    let requested_participants = Participants {
        player_one: AccountId::new(query.player_one),
        player_two: AccountId::new(query.player_two),
    };
    let participants = {
        let mut matches = state.matches.lock().await;
        if let Some(participants) = matches.participants(match_id) {
            participants
        } else {
            let journal = SwitchyCommandJournal::new(Arc::clone(&state.db));
            let participants = journal
                .participants(match_id)
                .await
                .map_err(|_| TransportError::MatchNotFound)?;
            if participants != requested_participants {
                return Err(TransportError::InvalidSubscription);
            }
            matches
                .recover_match(match_id, participants)
                .await
                .map_err(|_| TransportError::Recovery)?
                .ok_or(TransportError::MatchNotFound)?;
            drop(matches);
            participants
        }
    };
    if participants != requested_participants {
        return Err(TransportError::InvalidSubscription);
    }
    {
        let mut subscriptions = state.subscriptions.lock().await;
        subscriptions.connect(connection, account);
        subscriptions.subscribe(connection, match_id, participants)?;
    }
    let snapshot = state
        .matches
        .lock()
        .await
        .snapshot(match_id)
        .map_err(|_| TransportError::MatchNotFound)?;
    Ok(upgrade
        .max_message_size(MAX_SNAPSHOT_FRAME_BYTES)
        .max_frame_size(MAX_SNAPSHOT_FRAME_BYTES)
        .on_upgrade(move |socket| {
            websocket_loop(state, socket, connection, match_id, account, snapshot)
        })
        .into_response())
}

async fn websocket_loop(
    state: Arc<HttpState>,
    mut socket: WebSocket,
    connection: ConnectionId,
    match_id: MatchId,
    account: AccountId,
    initial_snapshot: SnapshotEnvelope,
) {
    let mut negotiated = false;
    while let Some(message) = socket.recv().await {
        match message {
            Ok(Message::Text(text)) if !negotiated => {
                let versions = text
                    .split(',')
                    .map(str::parse::<u16>)
                    .collect::<Result<Vec<_>, _>>();
                let Some(version) = versions.ok().and_then(|versions| {
                    negotiate_version(&versions, &[pwmtf_protocol::PROTOCOL_VERSION]).ok()
                }) else {
                    let _ = socket.send(Message::Text("rejected".into())).await;
                    break;
                };
                if socket
                    .send(Message::Text(version.to_string().into()))
                    .await
                    .is_err()
                    || socket
                        .send(Message::Binary(initial_snapshot.to_bytes().into()))
                        .await
                        .is_err()
                {
                    break;
                }
                negotiated = true;
            }
            Ok(Message::Binary(bytes)) if negotiated && bytes.len() <= MAX_FRAME_BYTES => {
                let response = apply_transport_command(&state, match_id, account, &bytes).await;
                let message = match response {
                    Ok(TransportCommandResponse::Accepted { snapshot }) => {
                        Message::Binary(snapshot.to_bytes().into())
                    }
                    Ok(TransportCommandResponse::Rejected) | Err(_) => {
                        Message::Text("rejected".into())
                    }
                };
                if socket.send(message).await.is_err() {
                    break;
                }
            }
            Ok(Message::Text(_) | Message::Binary(_) | Message::Close(_)) | Err(_) => break,
            Ok(_) => {}
        }
    }
    state.subscriptions.lock().await.disconnect(connection);
}

#[derive(Debug)]
enum TransportCommandResponse {
    Accepted { snapshot: SnapshotEnvelope },
    Rejected,
}

async fn apply_transport_command(
    state: &HttpState,
    match_id: MatchId,
    account: AccountId,
    bytes: &[u8],
) -> Result<TransportCommandResponse, TransportError> {
    let Ok(envelope) = CommandEnvelope::from_bytes(bytes) else {
        return Ok(TransportCommandResponse::Rejected);
    };
    let mut matches = state.matches.lock().await;
    let Ok(_acknowledgement) = matches
        .apply_at(
            match_id,
            account,
            envelope,
            crate::DeadlineMillis::new(unix_millis()?),
        )
        .await
    else {
        return Ok(TransportCommandResponse::Rejected);
    };
    let snapshot = matches
        .snapshot(match_id)
        .map_err(|_| TransportError::Internal)?;
    drop(matches);
    Ok(TransportCommandResponse::Accepted { snapshot })
}

fn validate_state_change_origin(
    headers: &HeaderMap,
    origins: &BTreeSet<String>,
) -> Result<(), TransportError> {
    let origin = headers
        .get("x-pwmtf-origin")
        .and_then(|value| value.to_str().ok())
        .ok_or(TransportError::InvalidOrigin)?;
    if origins.contains(origin) {
        Ok(())
    } else {
        Err(TransportError::InvalidOrigin)
    }
}

fn validate_origin(headers: &HeaderMap, origins: &BTreeSet<String>) -> Result<(), TransportError> {
    let origin = headers
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok())
        .ok_or(TransportError::InvalidOrigin)?;
    if origins.contains(origin) {
        Ok(())
    } else {
        Err(TransportError::InvalidOrigin)
    }
}

fn oidc_binding_cookie(attempt: &NewOidcAttempt) -> String {
    format!(
        "{OIDC_BINDING_COOKIE}={}.{}; Path=/; Max-Age=600; SameSite=Lax; Secure; HttpOnly",
        attempt.attempt_id(),
        attempt.browser_binding()
    )
}

fn parse_cookie<'a>(header: &'a str, wanted: &str) -> Result<&'a str, TransportError> {
    if header.len() > 4_096 || header.chars().any(|value| matches!(value, '\r' | '\n')) {
        return Err(TransportError::Unauthenticated);
    }
    let mut found = None;
    for pair in header.split(';') {
        let (name, value) = pair
            .trim()
            .split_once('=')
            .ok_or(TransportError::Unauthenticated)?;
        if name == wanted {
            if found.is_some() {
                return Err(TransportError::Unauthenticated);
            }
            found = Some(value);
        }
    }
    found.ok_or(TransportError::Unauthenticated)
}

fn account_id(identity: &crate::GoogleIdentity) -> AccountId {
    use sha2::{Digest as _, Sha256};
    let mut hash = Sha256::new();
    hash.update(identity.issuer().as_bytes());
    hash.update([0]);
    hash.update(identity.subject().as_bytes());
    let digest: [u8; 32] = hash.finalize().into();
    let mut bytes = [0_u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    AccountId::new(u128::from_be_bytes(bytes))
}

fn random_u128() -> Result<u128, TransportError> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes).map_err(|_| TransportError::Internal)?;
    Ok(u128::from_be_bytes(bytes))
}

fn unix_millis() -> Result<u64, TransportError> {
    let duration = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| TransportError::Internal)?;
    u64::try_from(duration.as_millis()).map_err(|_| TransportError::Internal)
}

/// Secret-safe HTTP/WebSocket transport rejection.
#[derive(Debug, Error)]
pub enum TransportError {
    /// Request has no valid authenticated session.
    #[error("request is not authenticated")]
    Unauthenticated,
    /// WebSocket origin is absent or not exactly allowed.
    #[error("request origin is not allowed")]
    InvalidOrigin,
    /// Request body or path data is malformed or exceeds fixed bounds.
    #[error("request is invalid")]
    InvalidRequest,
    /// Referenced social record does not exist.
    #[error("social record not found")]
    SocialNotFound,
    /// Match does not exist or cannot be recovered.
    #[error("match not found")]
    MatchNotFound,
    /// Requested participants do not match durable canonical membership.
    #[error("match subscription is invalid")]
    InvalidSubscription,
    /// Durable match recovery failed integrity validation.
    #[error("match recovery failed")]
    Recovery,
    /// Internal transport operation failed.
    #[error("transport operation failed")]
    Internal,
    /// Cookie policy failed.
    #[error(transparent)]
    Cookie(#[from] crate::CookieError),
    /// OIDC attempt persistence failed.
    #[error(transparent)]
    Attempt(#[from] crate::OidcAttemptStoreError),
    /// OIDC exchange or verification failed.
    #[error(transparent)]
    Oidc(#[from] crate::GoogleOidcError),
    /// Identity persistence failed.
    #[error(transparent)]
    Identity(#[from] crate::IdentityStoreError),
    /// Session persistence failed.
    #[error(transparent)]
    Session(#[from] crate::SessionStoreError),
    /// Account-profile persistence failed.
    #[error(transparent)]
    Profile(#[from] crate::ProfileStoreError),
    /// Waiting-lobby persistence or lifecycle transition failed.
    #[error(transparent)]
    Lobby(#[from] crate::LobbyStoreError),
    /// Social persistence or lifecycle transition failed.
    #[error(transparent)]
    Social(#[from] crate::SocialStoreError),
    /// Match subscription authorization failed.
    #[error(transparent)]
    Subscription(#[from] crate::SubscriptionError),
}

impl IntoResponse for TransportError {
    fn into_response(self) -> Response {
        let status = match self {
            Self::Unauthenticated => StatusCode::UNAUTHORIZED,
            Self::InvalidOrigin | Self::Subscription(_) | Self::InvalidSubscription => {
                StatusCode::FORBIDDEN
            }
            Self::MatchNotFound | Self::SocialNotFound => StatusCode::NOT_FOUND,
            Self::Attempt(_)
            | Self::Oidc(_)
            | Self::Cookie(_)
            | Self::InvalidRequest
            | Self::Social(_)
            | Self::Lobby(_) => StatusCode::BAD_REQUEST,
            Self::Internal
            | Self::Recovery
            | Self::Identity(_)
            | Self::Session(_)
            | Self::Profile(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        (status, self.to_string()).into_response()
    }
}

fn session_profile_response(account: AccountId, handle: Option<crate::Handle>) -> SessionProfile {
    SessionProfile {
        account_id: account.value().to_string(),
        handle: handle.map(|handle| handle.as_str().to_owned()),
    }
}

#[derive(Debug, serde::Deserialize)]
struct HandleRequest {
    handle: String,
}

#[derive(Debug, serde::Serialize)]
struct HandleResponse {
    handle: String,
}

async fn set_handle(
    State(state): State<Arc<HttpState>>,
    headers: HeaderMap,
    payload: Result<axum::Json<HandleRequest>, axum::extract::rejection::JsonRejection>,
) -> Result<axum::Json<HandleResponse>, TransportError> {
    ensure_json_bound(&headers)?;
    validate_state_change_origin(&headers, &state.origins)?;
    let account = authenticated_account(&state, &headers).await?;
    let request = payload.map_err(|_| TransportError::InvalidRequest)?.0;
    let handle = Handle::new(&request.handle).map_err(|_| TransportError::InvalidRequest)?;
    assign_handle(&*state.db, account, &handle).await?;
    Ok(axum::Json(HandleResponse {
        handle: handle.as_str().to_owned(),
    }))
}

#[derive(Debug, serde::Serialize)]
struct LobbyResponse {
    lobby_id: String,
    player_one: String,
    player_two: String,
}

#[derive(Debug, serde::Serialize)]
struct ChallengeResponse {
    challenge_id: String,
}

#[derive(Debug, serde::Serialize)]
struct PendingChallengeResponse {
    challenge_id: String,
    from_handle: String,
}

async fn list_pending_challenges(
    State(state): State<Arc<HttpState>>,
    headers: HeaderMap,
) -> Result<axum::Json<Vec<PendingChallengeResponse>>, TransportError> {
    let account = authenticated_account(&state, &headers).await?;
    let pending = pending_challenges_for(&*state.db, account).await?;
    let mut response = Vec::with_capacity(pending.len());
    for (challenge_id, challenger) in pending {
        let handle = handle_for_account(&*state.db, challenger)
            .await?
            .ok_or(TransportError::Internal)?;
        response.push(PendingChallengeResponse {
            challenge_id: challenge_id.value().to_string(),
            from_handle: handle.as_str().to_owned(),
        });
    }
    Ok(axum::Json(response))
}

async fn challenge_handle(
    State(state): State<Arc<HttpState>>,
    headers: HeaderMap,
    payload: Result<axum::Json<HandleRequest>, axum::extract::rejection::JsonRejection>,
) -> Result<axum::Json<ChallengeResponse>, TransportError> {
    ensure_json_bound(&headers)?;
    validate_state_change_origin(&headers, &state.origins)?;
    let actor = authenticated_account(&state, &headers).await?;
    let request = payload.map_err(|_| TransportError::InvalidRequest)?.0;
    let handle = Handle::new(&request.handle).map_err(|_| TransportError::InvalidRequest)?;
    let target = account_for_handle(&*state.db, &handle)
        .await?
        .ok_or(TransportError::SocialNotFound)?;
    let id = ChallengeId::new(random_u128()?);
    create_challenge(&*state.db, id, actor, target).await?;
    Ok(axum::Json(ChallengeResponse {
        challenge_id: id.value().to_string(),
    }))
}

async fn accept_challenge(
    State(state): State<Arc<HttpState>>,
    Path(challenge_id): Path<u128>,
    headers: HeaderMap,
) -> Result<axum::Json<LobbyResponse>, TransportError> {
    validate_state_change_origin(&headers, &state.origins)?;
    let actor = authenticated_account(&state, &headers).await?;
    let lobby_id = LobbyId::new(random_u128()?);
    let participants =
        accept_challenge_into_lobby(&*state.db, ChallengeId::new(challenge_id), actor, lobby_id)
            .await?;
    Ok(axum::Json(lobby_response(lobby_id, participants)))
}

#[derive(Debug, serde::Serialize)]
struct InvitationResponse {
    invitation_url: String,
    expires_at_ms: u64,
}

async fn create_invitation_link(
    State(state): State<Arc<HttpState>>,
    headers: HeaderMap,
) -> Result<axum::Json<InvitationResponse>, TransportError> {
    validate_state_change_origin(&headers, &state.origins)?;
    let actor = authenticated_account(&state, &headers).await?;
    let now = unix_millis()?;
    let expires_at = now.saturating_add(INVITATION_LIFETIME_MS);
    let token = generate_invitation(
        &*state.db,
        InvitationId::new(random_u128()?),
        actor,
        expires_at,
        now,
    )
    .await?;
    Ok(axum::Json(InvitationResponse {
        invitation_url: format!("{CANONICAL_ORIGIN}/?invite={}", token.expose()),
        expires_at_ms: expires_at,
    }))
}

#[derive(Debug, serde::Deserialize)]
struct InvitationRedeemRequest {
    token: String,
}

async fn redeem_invitation_link(
    State(state): State<Arc<HttpState>>,
    headers: HeaderMap,
    payload: Result<axum::Json<InvitationRedeemRequest>, axum::extract::rejection::JsonRejection>,
) -> Result<axum::Json<LobbyResponse>, TransportError> {
    ensure_json_bound(&headers)?;
    validate_state_change_origin(&headers, &state.origins)?;
    let actor = authenticated_account(&state, &headers).await?;
    let request = payload.map_err(|_| TransportError::InvalidRequest)?.0;
    let lobby_id = LobbyId::new(random_u128()?);
    let participants = redeem_invitation_token_into_lobby(
        &*state.db,
        &request.token,
        actor,
        lobby_id,
        unix_millis()?,
    )
    .await?;
    Ok(axum::Json(lobby_response(lobby_id, participants)))
}

#[derive(Debug, serde::Serialize)]
struct LobbyStatusResponse {
    lobby_id: String,
    status: &'static str,
    match_id: Option<String>,
    player_one: String,
    player_two: String,
}

async fn lobby_status(
    State(state): State<Arc<HttpState>>,
    Path(lobby_id): Path<u128>,
    headers: HeaderMap,
) -> Result<axum::Json<LobbyStatusResponse>, TransportError> {
    let actor = authenticated_account(&state, &headers).await?;
    let record = load_lobby(&*state.db, LobbyId::new(lobby_id))
        .await?
        .ok_or(TransportError::SocialNotFound)?;
    authorize_lobby_member(record.participants, actor)?;
    Ok(axum::Json(lobby_status_response(record)))
}

async fn cancel_waiting_lobby(
    State(state): State<Arc<HttpState>>,
    Path(lobby_id): Path<u128>,
    headers: HeaderMap,
) -> Result<axum::Json<LobbyStatusResponse>, TransportError> {
    validate_state_change_origin(&headers, &state.origins)?;
    let actor = authenticated_account(&state, &headers).await?;
    let record = cancel_lobby(&*state.db, LobbyId::new(lobby_id), actor).await?;
    Ok(axum::Json(lobby_status_response(record)))
}

fn authorize_lobby_member(
    participants: Participants,
    actor: AccountId,
) -> Result<(), TransportError> {
    if actor == participants.player_one || actor == participants.player_two {
        Ok(())
    } else {
        Err(TransportError::InvalidSubscription)
    }
}

fn lobby_status_response(record: crate::LobbyRecord) -> LobbyStatusResponse {
    let (status, match_id) = match record.status {
        crate::LobbyStatus::Waiting => ("waiting", None),
        crate::LobbyStatus::Started { match_id } => ("started", Some(match_id.value().to_string())),
        crate::LobbyStatus::Cancelled { .. } => ("cancelled", None),
    };
    LobbyStatusResponse {
        lobby_id: record.id.value().to_string(),
        status,
        match_id,
        player_one: record.participants.player_one.value().to_string(),
        player_two: record.participants.player_two.value().to_string(),
    }
}

fn lobby_response(lobby_id: LobbyId, participants: Participants) -> LobbyResponse {
    LobbyResponse {
        lobby_id: lobby_id.value().to_string(),
        player_one: participants.player_one.value().to_string(),
        player_two: participants.player_two.value().to_string(),
    }
}

fn ensure_json_bound(headers: &HeaderMap) -> Result<(), TransportError> {
    let length = headers
        .get(header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<usize>().ok())
        .ok_or(TransportError::InvalidRequest)?;
    if length == 0 || length > MAX_JSON_BYTES {
        Err(TransportError::InvalidRequest)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_profile_exposes_only_public_account_data() {
        let profile = session_profile_response(
            AccountId::new(42),
            Some(crate::Handle::new("player_one").unwrap()),
        );
        assert_eq!(profile.account_id, "42");
        assert_eq!(profile.handle.as_deref(), Some("player_one"));
    }

    #[test]
    fn json_requests_require_a_small_nonzero_content_length() {
        let mut headers = HeaderMap::new();
        assert!(matches!(
            ensure_json_bound(&headers),
            Err(TransportError::InvalidRequest)
        ));
        headers.insert(header::CONTENT_LENGTH, HeaderValue::from_static("32"));
        assert!(ensure_json_bound(&headers).is_ok());
        headers.insert(header::CONTENT_LENGTH, HeaderValue::from_static("1025"));
        assert!(matches!(
            ensure_json_bound(&headers),
            Err(TransportError::InvalidRequest)
        ));
    }

    #[test]
    fn lobby_responses_contain_only_membership_identifiers() {
        let response = lobby_response(
            LobbyId::new(3),
            Participants {
                player_one: AccountId::new(1),
                player_two: AccountId::new(2),
            },
        );
        assert_eq!(response.lobby_id, "3");
        assert_eq!(response.player_one, "1");
        assert_eq!(response.player_two, "2");
    }

    #[test]
    fn lobby_status_responses_preserve_membership_and_started_match() {
        let participants = Participants {
            player_one: AccountId::new(1),
            player_two: AccountId::new(2),
        };
        assert!(authorize_lobby_member(participants, AccountId::new(1)).is_ok());
        assert!(matches!(
            authorize_lobby_member(participants, AccountId::new(3)),
            Err(TransportError::InvalidSubscription)
        ));
        let response = lobby_status_response(crate::LobbyRecord {
            id: LobbyId::new(4),
            participants,
            status: crate::LobbyStatus::Started {
                match_id: MatchId::new(8),
            },
        });
        assert_eq!(response.status, "started");
        assert_eq!(response.match_id.as_deref(), Some("8"));
        assert_eq!(response.player_one, "1");
        assert_eq!(response.player_two, "2");
    }

    #[test]
    fn state_change_origin_header_is_exact_and_fail_closed() {
        let allowed = BTreeSet::from([CANONICAL_ORIGIN.to_owned()]);
        let mut headers = HeaderMap::new();
        assert!(validate_state_change_origin(&headers, &allowed).is_err());
        headers.insert("x-pwmtf-origin", HeaderValue::from_static(CANONICAL_ORIGIN));
        assert!(validate_state_change_origin(&headers, &allowed).is_ok());
        headers.insert(
            "x-pwmtf-origin",
            HeaderValue::from_static("https://evil.example"),
        );
        assert!(validate_state_change_origin(&headers, &allowed).is_err());
    }

    #[test]
    fn websocket_origin_is_exact_and_fail_closed() {
        let allowed = BTreeSet::from([CANONICAL_ORIGIN.to_owned()]);
        let mut headers = HeaderMap::new();
        assert!(validate_origin(&headers, &allowed).is_err());
        headers.insert(header::ORIGIN, HeaderValue::from_static(CANONICAL_ORIGIN));
        assert!(validate_origin(&headers, &allowed).is_ok());
        headers.insert(
            header::ORIGIN,
            HeaderValue::from_static("https://evil.example"),
        );
        assert!(validate_origin(&headers, &allowed).is_err());
    }

    #[test]
    fn oidc_binding_cookie_is_secure_host_only_and_strictly_parsed() {
        let attempt = crate::OidcAttempt::generate().unwrap();
        let created =
            NewOidcAttempt::from_parts("abc".to_owned(), attempt, "def".to_owned()).unwrap();
        let cookie = oidc_binding_cookie(&created);
        assert!(cookie.contains("Secure"));
        assert!(cookie.contains("HttpOnly"));
        assert!(!cookie.contains("Domain="));
        assert_eq!(
            parse_cookie("theme=dark; __Host-pwmtf_oidc=abc.def", OIDC_BINDING_COOKIE).unwrap(),
            "abc.def"
        );
    }
}

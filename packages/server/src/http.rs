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
    cancel_lobby, claim_oidc_attempt, connect_lobby, create_challenge, create_oidc_attempt,
    create_stored_session, disconnect_lobby, generate_invitation, handle_for_account,
    heartbeat_lobby, link_google_identity, load_lobby, lobby_ready, offer_rematch,
    pending_challenges_for, pending_rematches_for, ready_lobby, redeem_invitation_token_into_lobby,
    resolve_session_token, revoke_session_token, start_ready_lobby,
};
use pwmtf_protocol::{
    CommandEnvelope, MAX_FRAME_BYTES, MAX_SNAPSHOT_FRAME_BYTES, SnapshotEnvelope, negotiate_version,
};
use switchy_database::Database;
use thiserror::Error;
use tower_http::{
    services::{ServeDir, ServeFile},
    set_header::{SetResponseHeader, SetResponseHeaderLayer},
};

/// Canonical production web origin.
pub const CANONICAL_ORIGIN: &str = "https://pwmtf.hyperchad.dev";
/// OIDC callback path under the canonical origin.
pub const OIDC_CALLBACK_PATH: &str = "/auth/google/callback";
const OIDC_BINDING_COOKIE: &str = "__Host-pwmtf_oidc";
const OIDC_MAX_CODE_BYTES: usize = 1_024;
const OIDC_MAX_STATE_BYTES: usize = 256;
const OIDC_ATTEMPT_LIFETIME_MS: u64 = 10 * 60 * 1_000;
const SESSION_LIFETIME_MS: u64 = 7 * 24 * 60 * 60 * 1_000;
const INVITATION_LIFETIME_MS: u64 = 24 * 60 * 60 * 1_000;
const MAX_JSON_BYTES: usize = 1_024;
const CACHE_CONTROL_DYNAMIC: &str = "no-store";
const CACHE_CONTROL_ASSET: &str = "no-cache";
const CONTENT_SECURITY_POLICY: &str = "default-src 'none'; script-src 'self' 'wasm-unsafe-eval'; style-src 'self'; connect-src 'self'; img-src 'self'; font-src 'self'; media-src 'self'; worker-src 'self'; manifest-src 'self'; base-uri 'none'; form-action 'self'; frame-ancestors 'none'";
const PERMISSIONS_POLICY: &str = "camera=(), geolocation=(), microphone=()";
const SNAPSHOT_CHANNEL_CAPACITY: usize = 256;
type PublishedSnapshot = (MatchId, SnapshotEnvelope);

/// Runtime services required by native HTTP/OIDC/WebSocket entry points.
pub struct HttpState {
    db: Arc<dyn Database>,
    oidc: Arc<GoogleOidcClient>,
    sessions: SessionCookiePolicy,
    origins: BTreeSet<String>,
    subscriptions: Mutex<SubscriptionRegistry>,
    snapshot_sender: tokio::sync::broadcast::Sender<PublishedSnapshot>,
    matches: Mutex<MatchService<SwitchyCommandJournal>>,
    web_root: Option<PathBuf>,
}

impl HttpState {
    /// Polls and durably applies every currently due authoritative deadline,
    /// publishing each resulting complete snapshot before another command can
    /// advance the same in-process authority.
    ///
    /// # Errors
    ///
    /// Returns [`TransportError`] when a timeout cannot be durably accepted or
    /// its resulting snapshot cannot be encoded.
    pub async fn poll_due_deadlines(
        &self,
        now: crate::DeadlineMillis,
    ) -> Result<usize, TransportError> {
        let mut matches = self.matches.lock().await;
        let accepted = matches
            .poll_due_deadlines(now)
            .await
            .map_err(|_| TransportError::Recovery)?;
        for (match_id, _) in &accepted {
            let snapshot = matches
                .snapshot(*match_id)
                .map_err(|_| TransportError::Recovery)?;
            let _ = self.snapshot_sender.send((*match_id, snapshot));
        }
        let count = accepted.len();
        drop(matches);
        Ok(count)
    }

    /// Deletes expired and consumed durable OIDC attempts.
    ///
    /// # Errors
    ///
    /// Returns [`TransportError`] when stored attempt cleanup fails.
    pub async fn cleanup_oidc_attempts(&self, now: u64) -> Result<(), TransportError> {
        crate::cleanup_oidc_attempts(&*self.db, now)
            .await
            .map_err(Into::into)
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
    /// first access. Durable participants are loaded server-side; the caller
    /// supplies only its authenticated account and opaque match identifier.
    ///
    /// # Errors
    ///
    /// Returns [`TransportError`] for missing/corrupt durable records or an
    /// authenticated account that is not a participant.
    pub async fn reconnect_snapshot(
        &self,
        match_id: MatchId,
        account: AccountId,
    ) -> Result<SnapshotEnvelope, TransportError> {
        let stored = SwitchyCommandJournal::new(Arc::clone(&self.db))
            .participants(match_id)
            .await
            .map_err(|_| TransportError::MatchNotFound)?;
        if account != stored.player_one && account != stored.player_two {
            return Err(TransportError::InvalidSubscription);
        }
        let mut matches = self.matches.lock().await;
        match matches.participants(match_id) {
            Some(loaded) if loaded == stored => {}
            Some(_) => return Err(TransportError::Recovery),
            None => {
                matches
                    .recover_match(match_id, stored)
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
        let (snapshot_sender, _) = tokio::sync::broadcast::channel(SNAPSHOT_CHANNEL_CAPACITY);
        Self {
            db,
            oidc,
            sessions: SessionCookiePolicy::production(),
            origins: origins.into_iter().map(Into::into).collect(),
            subscriptions: Mutex::new(SubscriptionRegistry::default()),
            snapshot_sender,
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
        let (snapshot_sender, _) = tokio::sync::broadcast::channel(SNAPSHOT_CHANNEL_CAPACITY);
        Self {
            db,
            oidc,
            sessions: SessionCookiePolicy::production(),
            origins: origins.into_iter().map(Into::into).collect(),
            subscriptions: Mutex::new(SubscriptionRegistry::default()),
            snapshot_sender,
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
        .route("/pwmtf-bundle-manifest.json", get(hidden_bundle_manifest))
        .route("/auth/google/start", axum::routing::post(google_start))
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
            get(lobby_status)
                .post(connect_waiting_lobby)
                .delete(cancel_waiting_lobby),
        )
        .route(
            "/api/lobbies/{lobby_id}/ready",
            axum::routing::post(ready_waiting_lobby),
        )
        .route(
            "/api/lobbies/{lobby_id}/connections/{connection_id}",
            axum::routing::post(heartbeat_waiting_lobby).delete(disconnect_waiting_lobby),
        )
        .route("/api/rematches", get(list_pending_rematches))
        .route("/api/matches/{match_id}", get(match_access))
        .route(
            "/api/matches/{match_id}/rematch",
            axum::routing::post(create_rematch_offer),
        )
        .route(
            "/api/matches/{match_id}/rematch/accept",
            axum::routing::post(accept_rematch_offer),
        )
        .route("/api/{*path}", axum::routing::any(api_not_found))
        .route("/auth/{*path}", axum::routing::any(api_not_found))
        .route("/ws", get(websocket));
    let router = if let Some(root) = web_root {
        let index = ServeFile::new(root.join("index.html"));
        let assets = SetResponseHeader::overriding(
            ServeDir::new(&root)
                .append_index_html_on_directories(true)
                .fallback(index),
            header::CACHE_CONTROL,
            HeaderValue::from_static(CACHE_CONTROL_ASSET),
        );
        router.fallback_service(assets)
    } else {
        router
    };
    router
        .layer(SetResponseHeaderLayer::if_not_present(
            header::STRICT_TRANSPORT_SECURITY,
            HeaderValue::from_static("max-age=63072000; includeSubDomains"),
        ))
        .layer(SetResponseHeaderLayer::if_not_present(
            header::CACHE_CONTROL,
            HeaderValue::from_static(CACHE_CONTROL_DYNAMIC),
        ))
        .layer(SetResponseHeaderLayer::if_not_present(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static(CONTENT_SECURITY_POLICY),
        ))
        .layer(SetResponseHeaderLayer::if_not_present(
            header::REFERRER_POLICY,
            HeaderValue::from_static("no-referrer"),
        ))
        .layer(SetResponseHeaderLayer::if_not_present(
            header::X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        ))
        .layer(SetResponseHeaderLayer::if_not_present(
            header::HeaderName::from_static("permissions-policy"),
            HeaderValue::from_static(PERMISSIONS_POLICY),
        ))
        .layer(SetResponseHeaderLayer::if_not_present(
            header::HeaderName::from_static("cross-origin-opener-policy"),
            HeaderValue::from_static("same-origin"),
        ))
        .layer(SetResponseHeaderLayer::if_not_present(
            header::HeaderName::from_static("cross-origin-resource-policy"),
            HeaderValue::from_static("same-origin"),
        ))
        .with_state(state)
}

async fn health() -> &'static str {
    "ok"
}

async fn hidden_bundle_manifest() -> StatusCode {
    StatusCode::NOT_FOUND
}

async fn api_not_found() -> StatusCode {
    StatusCode::NOT_FOUND
}

#[derive(Debug, serde::Serialize)]
struct SessionProfile {
    handle: Option<String>,
}

async fn session_profile(
    State(state): State<Arc<HttpState>>,
    headers: HeaderMap,
) -> Result<axum::Json<SessionProfile>, TransportError> {
    let account = authenticated_account(&state, &headers).await?;
    let handle = handle_for_account(&*state.db, account).await?;
    Ok(axum::Json(session_profile_response(handle)))
}

async fn logout(
    State(state): State<Arc<HttpState>>,
    headers: HeaderMap,
) -> Result<Response, TransportError> {
    validate_state_change_origin(&headers, &state.origins)?;
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

async fn google_start(
    State(state): State<Arc<HttpState>>,
    headers: HeaderMap,
) -> Result<Response, TransportError> {
    validate_state_change_origin(&headers, &state.origins)?;
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

fn validate_oidc_callback(query: &OidcCallbackQuery) -> Result<(), TransportError> {
    if query.code.is_empty()
        || query.code.len() > OIDC_MAX_CODE_BYTES
        || query.state.is_empty()
        || query.state.len() > OIDC_MAX_STATE_BYTES
        || query.code.chars().any(char::is_control)
        || query.state.chars().any(char::is_control)
    {
        Err(TransportError::InvalidRequest)
    } else {
        Ok(())
    }
}

async fn google_callback(
    State(state): State<Arc<HttpState>>,
    Query(query): Query<OidcCallbackQuery>,
    headers: HeaderMap,
) -> Result<Response, TransportError> {
    validate_oidc_callback(&query)?;
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
            "__Host-pwmtf_oidc=; Path=/; Max-Age=0; SameSite=Lax; Secure; HttpOnly; Priority=High",
        ),
    );
    Ok(response)
}

#[derive(Debug, serde::Deserialize)]
struct WebSocketQuery {
    match_id: u128,
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
    let durable_participants = SwitchyCommandJournal::new(Arc::clone(&state.db))
        .participants(match_id)
        .await
        .map_err(|_| TransportError::MatchNotFound)?;
    if account != durable_participants.player_one && account != durable_participants.player_two {
        return Err(TransportError::InvalidSubscription);
    }
    let participants = {
        let mut matches = state.matches.lock().await;
        if let Some(loaded) = matches.participants(match_id) {
            if loaded != durable_participants {
                return Err(TransportError::Recovery);
            }
            loaded
        } else {
            matches
                .recover_match(match_id, durable_participants)
                .await
                .map_err(|_| TransportError::Recovery)?
                .ok_or(TransportError::MatchNotFound)?;
            drop(matches);
            durable_participants
        }
    };
    {
        let mut subscriptions = state.subscriptions.lock().await;
        subscriptions.connect(connection, account);
        subscriptions.subscribe(connection, match_id, participants)?;
    }
    let publication = state.snapshot_sender.subscribe();
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
            websocket_loop(
                state,
                socket,
                connection,
                match_id,
                account,
                snapshot,
                publication,
            )
        })
        .into_response())
}

async fn send_snapshot_if_subscribed(
    state: &HttpState,
    socket: &mut WebSocket,
    connection: ConnectionId,
    match_id: MatchId,
    snapshot: &SnapshotEnvelope,
) -> bool {
    let subscribed = state
        .subscriptions
        .lock()
        .await
        .is_subscribed(connection, match_id);
    subscribed
        && socket
            .send(Message::Binary(snapshot.to_bytes().into()))
            .await
            .is_ok()
}

async fn forward_publication(
    state: &HttpState,
    socket: &mut WebSocket,
    connection: ConnectionId,
    match_id: MatchId,
    delivered_revision: &mut u64,
    publication: Result<PublishedSnapshot, tokio::sync::broadcast::error::RecvError>,
) -> bool {
    let snapshot = match publication {
        Ok((published_match, snapshot)) if published_match == match_id => Some(snapshot),
        Ok(_) => return true,
        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
            state.matches.lock().await.snapshot(match_id).ok()
        }
        Err(tokio::sync::broadcast::error::RecvError::Closed) => return false,
    };
    let Some(snapshot) = snapshot else {
        return false;
    };
    if snapshot.revision <= *delivered_revision {
        return true;
    }
    if send_snapshot_if_subscribed(state, socket, connection, match_id, &snapshot).await {
        *delivered_revision = snapshot.revision;
        true
    } else {
        false
    }
}

async fn websocket_loop(
    state: Arc<HttpState>,
    mut socket: WebSocket,
    connection: ConnectionId,
    match_id: MatchId,
    account: AccountId,
    initial_snapshot: SnapshotEnvelope,
    mut publication: tokio::sync::broadcast::Receiver<PublishedSnapshot>,
) {
    let mut negotiated = false;
    let mut delivered_revision = initial_snapshot.revision;
    loop {
        tokio::select! {
            published = publication.recv(), if negotiated => {
                if !forward_publication(
                    &state,
                    &mut socket,
                    connection,
                    match_id,
                    &mut delivered_revision,
                    published,
                )
                .await
                {
                    break;
                }
            }
            message = socket.recv() => {
                let Some(message) = message else {
                    break;
                };
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
                                delivered_revision = delivered_revision.max(snapshot.revision);
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
    let _ = state.snapshot_sender.send((match_id, snapshot.clone()));
    Ok(TransportCommandResponse::Accepted { snapshot })
}

fn validate_state_change_origin(
    headers: &HeaderMap,
    origins: &BTreeSet<String>,
) -> Result<(), TransportError> {
    let origin = headers
        .get(header::ORIGIN)
        .or_else(|| headers.get("x-pwmtf-origin"))
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
        "{OIDC_BINDING_COOKIE}={}.{}; Path=/; Max-Age=600; SameSite=Lax; Secure; HttpOnly; Priority=High",
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

fn random_u64() -> Result<u64, TransportError> {
    let mut bytes = [0_u8; 8];
    getrandom::fill(&mut bytes).map_err(|_| TransportError::Internal)?;
    Ok(u64::from_be_bytes(bytes))
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
    /// Rematch persistence or lifecycle transition failed.
    #[error(transparent)]
    Rematch(#[from] crate::RematchStoreError),
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
            | Self::Lobby(_)
            | Self::Rematch(_) => StatusCode::BAD_REQUEST,
            Self::Internal
            | Self::Recovery
            | Self::Identity(_)
            | Self::Session(_)
            | Self::Profile(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        (status, self.to_string()).into_response()
    }
}

fn session_profile_response(handle: Option<crate::Handle>) -> SessionProfile {
    SessionProfile {
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
    accept_challenge_into_lobby(&*state.db, ChallengeId::new(challenge_id), actor, lobby_id)
        .await?;
    Ok(axum::Json(lobby_response(lobby_id)))
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
    redeem_invitation_token_into_lobby(&*state.db, &request.token, actor, lobby_id, unix_millis()?)
        .await?;
    Ok(axum::Json(lobby_response(lobby_id)))
}

#[derive(Debug, serde::Serialize)]
struct LobbyStatusResponse {
    lobby_id: String,
    status: &'static str,
    match_id: Option<String>,
    both_ready: bool,
    connection_id: Option<String>,
}

#[derive(Debug, serde::Serialize)]
struct MatchAccessResponse {
    player: u8,
}

fn match_access_response(
    participants: Participants,
    actor: AccountId,
) -> Result<MatchAccessResponse, TransportError> {
    let player = if actor == participants.player_one {
        1
    } else if actor == participants.player_two {
        2
    } else {
        return Err(TransportError::InvalidSubscription);
    };
    Ok(MatchAccessResponse { player })
}

async fn match_access(
    State(state): State<Arc<HttpState>>,
    Path(match_id): Path<u128>,
    headers: HeaderMap,
) -> Result<axum::Json<MatchAccessResponse>, TransportError> {
    let actor = authenticated_account(&state, &headers).await?;
    let match_id = MatchId::new(match_id);
    let participants = SwitchyCommandJournal::new(Arc::clone(&state.db))
        .participants(match_id)
        .await
        .map_err(|_| TransportError::MatchNotFound)?;
    Ok(axum::Json(match_access_response(participants, actor)?))
}

#[derive(Debug, serde::Serialize)]
struct PendingRematchResponse {
    previous_match_id: String,
}

async fn list_pending_rematches(
    State(state): State<Arc<HttpState>>,
    headers: HeaderMap,
) -> Result<axum::Json<Vec<PendingRematchResponse>>, TransportError> {
    let actor = authenticated_account(&state, &headers).await?;
    let pending = pending_rematches_for(&*state.db, actor).await?;
    Ok(axum::Json(
        pending
            .into_iter()
            .map(|offer| PendingRematchResponse {
                previous_match_id: offer.previous_match_id.value().to_string(),
            })
            .collect(),
    ))
}

async fn create_rematch_offer(
    State(state): State<Arc<HttpState>>,
    Path(match_id): Path<u128>,
    headers: HeaderMap,
) -> Result<StatusCode, TransportError> {
    validate_state_change_origin(&headers, &state.origins)?;
    let actor = authenticated_account(&state, &headers).await?;
    offer_rematch(&*state.db, MatchId::new(match_id), actor).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn accept_rematch_offer(
    State(state): State<Arc<HttpState>>,
    Path(match_id): Path<u128>,
    headers: HeaderMap,
) -> Result<axum::Json<LobbyStatusResponse>, TransportError> {
    validate_state_change_origin(&headers, &state.origins)?;
    let actor = authenticated_account(&state, &headers).await?;
    let previous_match_id = MatchId::new(match_id);
    let new_match_id = MatchId::new(random_u128()?);
    let now = unix_millis()?;
    let rematch = crate::accept_rematch(
        &*state.db,
        previous_match_id,
        new_match_id,
        actor,
        pwmtf_game_domain::RackSeed::new(random_u64()?),
        crate::DeadlineMillis::new(now.saturating_add(30_000)),
    )
    .await?;
    let journal = SwitchyCommandJournal::new(Arc::clone(&state.db));
    let participants = journal
        .participants(new_match_id)
        .await
        .map_err(|_| TransportError::Recovery)?;
    let active_player = rematch.active_player();
    state
        .load_match(
            new_match_id,
            participants,
            rematch,
            Some(crate::ScheduledDeadline {
                id: crate::DeadlineId {
                    revision: 0,
                    player: active_player,
                },
                due_at: crate::DeadlineMillis::new(now.saturating_add(30_000)),
            }),
        )
        .await?;
    Ok(axum::Json(LobbyStatusResponse {
        lobby_id: String::new(),
        status: "started",
        match_id: Some(new_match_id.value().to_string()),
        both_ready: true,
        connection_id: None,
    }))
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
    let both_ready = record.status == crate::LobbyStatus::Waiting
        && lobby_ready(&*state.db, record.id, unix_millis()?).await?;
    Ok(axum::Json(lobby_status_response(record, both_ready)))
}

async fn connect_waiting_lobby(
    State(state): State<Arc<HttpState>>,
    Path(lobby_id): Path<u128>,
    headers: HeaderMap,
) -> Result<axum::Json<LobbyStatusResponse>, TransportError> {
    validate_state_change_origin(&headers, &state.origins)?;
    let actor = authenticated_account(&state, &headers).await?;
    let connection = ConnectionId::new(random_u128()?);
    let now = unix_millis()?;
    let record = connect_lobby(&*state.db, LobbyId::new(lobby_id), actor, connection, now).await?;
    let mut response = lobby_status_response(record, false);
    response.connection_id = Some(connection.value().to_string());
    Ok(axum::Json(response))
}

async fn heartbeat_waiting_lobby(
    State(state): State<Arc<HttpState>>,
    Path((lobby_id, connection_id)): Path<(u128, u128)>,
    headers: HeaderMap,
) -> Result<axum::Json<LobbyStatusResponse>, TransportError> {
    validate_state_change_origin(&headers, &state.origins)?;
    let actor = authenticated_account(&state, &headers).await?;
    let record = heartbeat_lobby(
        &*state.db,
        LobbyId::new(lobby_id),
        actor,
        ConnectionId::new(connection_id),
        unix_millis()?,
    )
    .await?;
    let both_ready = lobby_ready(&*state.db, record.id, unix_millis()?).await?;
    Ok(axum::Json(lobby_status_response(record, both_ready)))
}

async fn disconnect_waiting_lobby(
    State(state): State<Arc<HttpState>>,
    Path((lobby_id, connection_id)): Path<(u128, u128)>,
    headers: HeaderMap,
) -> Result<axum::Json<LobbyStatusResponse>, TransportError> {
    validate_state_change_origin(&headers, &state.origins)?;
    let actor = authenticated_account(&state, &headers).await?;
    let record = disconnect_lobby(
        &*state.db,
        LobbyId::new(lobby_id),
        actor,
        ConnectionId::new(connection_id),
    )
    .await?;
    let both_ready = lobby_ready(&*state.db, record.id, unix_millis()?).await?;
    Ok(axum::Json(lobby_status_response(record, both_ready)))
}

async fn ready_waiting_lobby(
    State(state): State<Arc<HttpState>>,
    Path(lobby_id): Path<u128>,
    headers: HeaderMap,
) -> Result<axum::Json<LobbyStatusResponse>, TransportError> {
    validate_state_change_origin(&headers, &state.origins)?;
    let actor = authenticated_account(&state, &headers).await?;
    let record = ready_lobby(&*state.db, LobbyId::new(lobby_id), actor).await?;
    let both_ready = lobby_ready(&*state.db, record.id, unix_millis()?).await?;
    let record = if both_ready {
        let now = unix_millis()?;
        let state_match = pwmtf_game_domain::MatchState::new(
            pwmtf_game_domain::RulesProfile::standard(),
            pwmtf_game_domain::PhysicsProfile::standard(),
            pwmtf_game_domain::TableGeometry::standard(),
            pwmtf_game_domain::RackSeed::new(random_u64()?),
            pwmtf_game_domain::Player::One,
        )
        .map_err(|_| TransportError::Internal)?;
        let deadline = crate::ScheduledDeadline {
            id: crate::DeadlineId {
                revision: 0,
                player: state_match.active_player(),
            },
            due_at: crate::DeadlineMillis::new(now.saturating_add(30_000)),
        };
        let match_id = MatchId::new(random_u128()?);
        start_ready_lobby(&*state.db, record.id, match_id, &state_match, deadline).await?
    } else {
        record
    };
    Ok(axum::Json(lobby_status_response(record, both_ready)))
}

async fn cancel_waiting_lobby(
    State(state): State<Arc<HttpState>>,
    Path(lobby_id): Path<u128>,
    headers: HeaderMap,
) -> Result<axum::Json<LobbyStatusResponse>, TransportError> {
    validate_state_change_origin(&headers, &state.origins)?;
    let actor = authenticated_account(&state, &headers).await?;
    let record = cancel_lobby(&*state.db, LobbyId::new(lobby_id), actor).await?;
    Ok(axum::Json(lobby_status_response(record, false)))
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

fn lobby_status_response(record: crate::LobbyRecord, both_ready: bool) -> LobbyStatusResponse {
    let (status, match_id) = match record.status {
        crate::LobbyStatus::Waiting => ("waiting", None),
        crate::LobbyStatus::Started { match_id } => ("started", Some(match_id.value().to_string())),
        crate::LobbyStatus::Cancelled { .. } => ("cancelled", None),
    };
    LobbyStatusResponse {
        lobby_id: record.id.value().to_string(),
        status,
        match_id,
        both_ready,
        connection_id: None,
    }
}

fn lobby_response(lobby_id: LobbyId) -> LobbyResponse {
    LobbyResponse {
        lobby_id: lobby_id.value().to_string(),
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
    fn oidc_callback_query_is_bounded_and_control_free() {
        let valid = OidcCallbackQuery {
            code: "provider-code".to_owned(),
            state: "a".repeat(64),
        };
        assert!(validate_oidc_callback(&valid).is_ok());
        for invalid in [
            OidcCallbackQuery {
                code: String::new(),
                state: "a".repeat(64),
            },
            OidcCallbackQuery {
                code: "x".repeat(OIDC_MAX_CODE_BYTES + 1),
                state: "a".repeat(64),
            },
            OidcCallbackQuery {
                code: "provider\ncode".to_owned(),
                state: "a".repeat(64),
            },
            OidcCallbackQuery {
                code: "provider-code".to_owned(),
                state: "x".repeat(OIDC_MAX_STATE_BYTES + 1),
            },
        ] {
            assert!(matches!(
                validate_oidc_callback(&invalid),
                Err(TransportError::InvalidRequest)
            ));
        }
    }

    #[test]
    fn session_profile_exposes_only_public_profile_data() {
        let profile = session_profile_response(Some(crate::Handle::new("player_one").unwrap()));
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
    fn lobby_responses_do_not_disclose_participant_identifiers() {
        let response = lobby_response(LobbyId::new(3));
        assert_eq!(response.lobby_id, "3");
    }

    #[test]
    fn lobby_status_responses_preserve_authorization_and_started_match() {
        let participants = Participants {
            player_one: AccountId::new(1),
            player_two: AccountId::new(2),
        };
        assert!(authorize_lobby_member(participants, AccountId::new(1)).is_ok());
        assert!(matches!(
            authorize_lobby_member(participants, AccountId::new(3)),
            Err(TransportError::InvalidSubscription)
        ));
        let response = lobby_status_response(
            crate::LobbyRecord {
                id: LobbyId::new(4),
                participants,
                status: crate::LobbyStatus::Started {
                    match_id: MatchId::new(8),
                },
            },
            false,
        );
        assert_eq!(response.status, "started");
        assert_eq!(response.match_id.as_deref(), Some("8"));
    }

    #[test]
    fn match_access_derives_seat_from_durable_membership() {
        let participants = Participants {
            player_one: AccountId::new(1),
            player_two: AccountId::new(2),
        };
        assert_eq!(
            match_access_response(participants, AccountId::new(1))
                .unwrap()
                .player,
            1
        );
        assert_eq!(
            match_access_response(participants, AccountId::new(2))
                .unwrap()
                .player,
            2
        );
        assert!(matches!(
            match_access_response(participants, AccountId::new(3)),
            Err(TransportError::InvalidSubscription)
        ));
    }

    #[test]
    fn logout_and_state_changes_share_exact_origin_policy() {
        let allowed = BTreeSet::from([CANONICAL_ORIGIN.to_owned()]);
        let mut headers = HeaderMap::new();
        assert!(validate_state_change_origin(&headers, &allowed).is_err());
        headers.insert(header::ORIGIN, HeaderValue::from_static(CANONICAL_ORIGIN));
        assert!(validate_state_change_origin(&headers, &allowed).is_ok());
    }

    #[test]
    fn state_change_origin_header_is_exact_and_fail_closed() {
        let allowed = BTreeSet::from([CANONICAL_ORIGIN.to_owned()]);
        let mut headers = HeaderMap::new();
        assert!(validate_state_change_origin(&headers, &allowed).is_err());
        headers.insert(header::ORIGIN, HeaderValue::from_static(CANONICAL_ORIGIN));
        assert!(validate_state_change_origin(&headers, &allowed).is_ok());
        headers.insert(
            header::ORIGIN,
            HeaderValue::from_static("https://evil.example"),
        );
        headers.insert("x-pwmtf-origin", HeaderValue::from_static(CANONICAL_ORIGIN));
        assert!(validate_state_change_origin(&headers, &allowed).is_err());
        headers.remove(header::ORIGIN);
        assert!(validate_state_change_origin(&headers, &allowed).is_ok());
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
        assert!(cookie.contains("Priority=High"));
        assert!(!cookie.contains("Domain="));
        assert_eq!(
            parse_cookie("theme=dark; __Host-pwmtf_oidc=abc.def", OIDC_BINDING_COOKIE).unwrap(),
            "abc.def"
        );
    }
}

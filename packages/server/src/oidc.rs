//! Google `OpenID` Connect authorization-code boundary.

use crate::GoogleIdentity;
use openidconnect::{
    AuthorizationCode, ClientId, ClientSecret, CsrfToken, EndpointMaybeSet, EndpointSet, IssuerUrl,
    Nonce, PkceCodeChallenge, PkceCodeVerifier, RedirectUrl, Scope, TokenResponse as _,
    core::{CoreAuthenticationFlow, CoreClient, CoreProviderMetadata},
};
use thiserror::Error;

/// Canonical verified Google issuer.
pub const GOOGLE_ISSUER: &str = "https://accounts.google.com";
const MAX_ID_TOKEN_AGE_SECS: i64 = 10 * 60;
const MAX_ID_TOKEN_FUTURE_SKEW_SECS: i64 = 5 * 60;
const PROVIDER_CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
const PROVIDER_REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
const OIDC_SECRET_BYTES: usize = 32;

/// One short-lived browser-bound OIDC attempt's secret validation material.
///
/// Debug output deliberately omits every secret.
#[derive(Clone, Eq, PartialEq)]
pub struct OidcAttempt {
    state: String,
    nonce: String,
    pkce_verifier: String,
}

impl OidcAttempt {
    /// Generates state, nonce, and PKCE verifier from independent operating-system entropy.
    ///
    /// # Errors
    ///
    /// Returns [`GoogleOidcError::Randomness`] when secure randomness is unavailable.
    pub fn generate() -> Result<Self, GoogleOidcError> {
        Ok(Self {
            state: random_secret()?,
            nonce: random_secret()?,
            pkce_verifier: random_secret()?,
        })
    }

    /// Reconstructs a claimed server-side attempt after durable browser-binding validation.
    ///
    /// # Errors
    ///
    /// Returns [`GoogleOidcError::Callback`] for malformed validation material.
    pub fn claimed(state: &str, nonce: &str, pkce_verifier: &str) -> Result<Self, GoogleOidcError> {
        for value in [state, nonce, pkce_verifier] {
            if value.len() != OIDC_SECRET_BYTES * 2
                || !value
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            {
                return Err(GoogleOidcError::Callback);
            }
        }
        Ok(Self {
            state: state.to_owned(),
            nonce: nonce.to_owned(),
            pkce_verifier: pkce_verifier.to_owned(),
        })
    }

    /// Returns state only for browser callback matching.
    #[must_use]
    pub fn state(&self) -> &str {
        &self.state
    }

    /// Returns nonce only for signed ID-token validation.
    #[must_use]
    pub fn nonce(&self) -> &str {
        &self.nonce
    }

    /// Returns the PKCE verifier only for token exchange.
    #[must_use]
    pub fn pkce_verifier(&self) -> &str {
        &self.pkce_verifier
    }
}

impl std::fmt::Debug for OidcAttempt {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("OidcAttempt([REDACTED])")
    }
}

/// Google OIDC provider/client configuration with discovered signing keys.
pub struct GoogleOidcClient {
    client: CoreClient<
        EndpointSet,
        openidconnect::EndpointNotSet,
        openidconnect::EndpointNotSet,
        openidconnect::EndpointNotSet,
        EndpointMaybeSet,
        EndpointMaybeSet,
    >,
    issuer: String,
    client_id: String,
}

impl GoogleOidcClient {
    /// Discovers Google's metadata and constructs a confidential web client.
    ///
    /// # Errors
    ///
    /// Returns [`GoogleOidcError`] for invalid configuration, HTTP construction,
    /// discovery, issuer mismatch, or unavailable provider metadata.
    pub async fn discover(
        client_id: &str,
        client_secret: &str,
        redirect_uri: &str,
    ) -> Result<Self, GoogleOidcError> {
        Self::discover_issuer(client_id, client_secret, redirect_uri, GOOGLE_ISSUER).await
    }

    /// Discovers a configurable issuer while retaining Google's production identity namespace.
    ///
    /// This supports deterministic acceptance providers. Production must call
    /// [`Self::discover`].
    ///
    /// # Errors
    ///
    /// Returns [`GoogleOidcError`] for invalid configuration, discovery, or
    /// exact issuer mismatch.
    pub async fn discover_issuer(
        client_id: &str,
        client_secret: &str,
        redirect_uri: &str,
        issuer_url: &str,
    ) -> Result<Self, GoogleOidcError> {
        if client_id.trim().is_empty()
            || client_secret.trim().is_empty()
            || issuer_url.trim().is_empty()
        {
            return Err(GoogleOidcError::Configuration);
        }
        let http_client = Self::provider_http_client()?;
        let issuer =
            IssuerUrl::new(issuer_url.to_owned()).map_err(|_| GoogleOidcError::Configuration)?;
        let metadata = CoreProviderMetadata::discover_async(issuer, &http_client)
            .await
            .map_err(|_| GoogleOidcError::Discovery)?;
        if metadata.issuer().as_str() != issuer_url {
            return Err(GoogleOidcError::Discovery);
        }
        let client = CoreClient::from_provider_metadata(
            metadata,
            ClientId::new(client_id.to_owned()),
            Some(ClientSecret::new(client_secret.to_owned())),
        )
        .set_redirect_uri(
            RedirectUrl::new(redirect_uri.to_owned())
                .map_err(|_| GoogleOidcError::Configuration)?,
        );
        Ok(Self {
            client,
            issuer: issuer_url.to_owned(),
            client_id: client_id.to_owned(),
        })
    }

    /// Creates the provider authorization URL with state, nonce, and S256 PKCE.
    #[must_use]
    pub fn authorization_url(&self, attempt: &OidcAttempt) -> String {
        let challenge = PkceCodeChallenge::from_code_verifier_sha256(&PkceCodeVerifier::new(
            attempt.pkce_verifier.clone(),
        ));
        let state = attempt.state.clone();
        let nonce = attempt.nonce.clone();
        let (url, _, _) = self
            .client
            .authorize_url(
                CoreAuthenticationFlow::AuthorizationCode,
                move || CsrfToken::new(state),
                move || Nonce::new(nonce),
            )
            .add_scope(Scope::new("profile".to_owned()))
            .set_pkce_challenge(challenge)
            .url();
        url.to_string()
    }

    /// Validates callback state, exchanges the code with PKCE, and verifies the
    /// signed ID token, nonce, issuer, audience, authorized party, issue time,
    /// and stable subject.
    ///
    /// # Errors
    ///
    /// Returns [`GoogleOidcError`] for state/code rejection, exchange failure,
    /// missing ID token, or invalid claims/signature.
    pub async fn exchange_callback(
        &self,
        code: &str,
        callback_state: &str,
        attempt: &OidcAttempt,
    ) -> Result<GoogleIdentity, GoogleOidcError> {
        if code.trim().is_empty() || !constant_time_eq(callback_state, attempt.state()) {
            return Err(GoogleOidcError::Callback);
        }
        let response = self
            .client
            .exchange_code(AuthorizationCode::new(code.to_owned()))
            .map_err(|_| GoogleOidcError::Callback)?
            .set_pkce_verifier(PkceCodeVerifier::new(attempt.pkce_verifier.clone()))
            .request_async(&Self::provider_http_client()?)
            .await
            .map_err(|_| GoogleOidcError::TokenExchange)?;
        let id_token = response.id_token().ok_or(GoogleOidcError::MissingIdToken)?;
        self.validate_id_token(id_token, attempt)
    }

    fn validate_id_token(
        &self,
        id_token: &openidconnect::core::CoreIdToken,
        attempt: &OidcAttempt,
    ) -> Result<GoogleIdentity, GoogleOidcError> {
        let verifier = self
            .client
            .id_token_verifier()
            .set_issue_time_verifier_fn(|issue_time| {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_err(|_| "system clock precedes Unix epoch".to_owned())?;
                let now = i64::try_from(now.as_secs())
                    .map_err(|_| "system clock is out of range".to_owned())?;
                let issue_time = issue_time.timestamp();
                if issue_time > now.saturating_add(MAX_ID_TOKEN_FUTURE_SKEW_SECS)
                    || issue_time < now.saturating_sub(MAX_ID_TOKEN_AGE_SECS)
                {
                    return Err("ID token issue time is outside bounds".to_owned());
                }
                Ok(())
            });
        let claims = id_token
            .claims(&verifier, &Nonce::new(attempt.nonce.clone()))
            .map_err(|_| GoogleOidcError::InvalidIdToken)?;
        let audiences = claims.audiences();
        let requires_azp = audiences.len() > 1;
        let azp_matches = claims
            .authorized_party()
            .is_none_or(|party| party.as_str() == self.client_id);
        if (requires_azp && claims.authorized_party().is_none()) || !azp_matches {
            return Err(GoogleOidcError::InvalidIdToken);
        }
        let subject = claims.subject().as_str();
        GoogleIdentity::verified(&self.issuer, subject).map_err(|_| GoogleOidcError::InvalidIdToken)
    }

    fn provider_http_client() -> Result<openidconnect::reqwest::Client, GoogleOidcError> {
        openidconnect::reqwest::ClientBuilder::new()
            .redirect(openidconnect::reqwest::redirect::Policy::none())
            .connect_timeout(PROVIDER_CONNECT_TIMEOUT)
            .timeout(PROVIDER_REQUEST_TIMEOUT)
            .build()
            .map_err(|_| GoogleOidcError::HttpClient)
    }
}

/// Secret-safe Google OIDC protocol/configuration failure.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum GoogleOidcError {
    /// Client, redirect, or issuer configuration is invalid.
    #[error("Google OIDC configuration is invalid")]
    Configuration,
    /// Secure provider HTTP client construction failed.
    #[error("Google OIDC HTTP client could not be constructed")]
    HttpClient,
    /// Provider discovery or exact issuer validation failed.
    #[error("Google OIDC discovery failed")]
    Discovery,
    /// Callback state or authorization code is invalid.
    #[error("Google OIDC callback is invalid")]
    Callback,
    /// Provider token exchange failed.
    #[error("Google OIDC token exchange failed")]
    TokenExchange,
    /// Provider response omitted the required ID token.
    #[error("Google OIDC response did not contain an ID token")]
    MissingIdToken,
    /// Signed ID-token verification or claims validation failed.
    #[error("Google OIDC ID token is invalid")]
    InvalidIdToken,
    /// Secure attempt generation failed.
    #[error("Google OIDC secure randomness failed")]
    Randomness,
}

fn random_secret() -> Result<String, GoogleOidcError> {
    let mut bytes = [0_u8; OIDC_SECRET_BYTES];
    getrandom::fill(&mut bytes).map_err(|_| GoogleOidcError::Randomness)?;
    let mut output = String::with_capacity(OIDC_SECRET_BYTES * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        write!(output, "{byte:02x}").expect("writing to String cannot fail");
    }
    Ok(output)
}

fn constant_time_eq(left: &str, right: &str) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.bytes()
        .zip(right.bytes())
        .fold(0_u8, |difference, (left, right)| {
            difference | (left ^ right)
        })
        == 0
}

#[cfg(test)]
mod tests {
    use futures_lite::future::block_on;

    use super::*;

    #[test]
    fn attempts_are_distinct_strict_and_redacted() {
        let first = OidcAttempt::generate().unwrap();
        let second = OidcAttempt::generate().unwrap();
        assert_ne!(first, second);
        assert_eq!(format!("{first:?}"), "OidcAttempt([REDACTED])");
        assert_eq!(
            OidcAttempt::claimed(first.state(), first.nonce(), first.pkce_verifier()).unwrap(),
            first
        );
        assert_eq!(
            OidcAttempt::claimed("bad", first.nonce(), first.pkce_verifier()),
            Err(GoogleOidcError::Callback)
        );
    }

    #[test]
    fn invalid_configuration_fails_before_network_access() {
        block_on(async {
            assert_eq!(
                GoogleOidcClient::discover(
                    "",
                    "secret",
                    "https://pwmtf.hyperchad.dev/auth/callback"
                )
                .await
                .err(),
                Some(GoogleOidcError::Configuration)
            );
            assert_eq!(
                GoogleOidcClient::discover(
                    "client",
                    "",
                    "https://pwmtf.hyperchad.dev/auth/callback"
                )
                .await
                .err(),
                Some(GoogleOidcError::Configuration)
            );
        });
    }

    #[test]
    fn callback_state_comparison_is_exact() {
        assert!(constant_time_eq("abcdef", "abcdef"));
        assert!(!constant_time_eq("abcdef", "abcdeg"));
        assert!(!constant_time_eq("abcdef", "abc"));
    }
}

//! Secure cookie policy for opaque browser sessions.

use crate::{SessionToken, TokenError};
use thiserror::Error;

/// Canonical production cookie name.
pub const SESSION_COOKIE_NAME: &str = "__Host-pwmtf_session";

/// Same-site policy emitted for the authentication cookie.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SameSite {
    /// Cookie is sent on same-site navigation and requests only.
    Lax,
    /// Cookie is sent only on strict same-site requests.
    Strict,
}

/// Validated server-owned session-cookie policy.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionCookiePolicy {
    name: String,
    path: String,
    same_site: SameSite,
    secure: bool,
    http_only: bool,
}

impl SessionCookiePolicy {
    /// Returns the fixed production policy.
    #[must_use]
    pub fn production() -> Self {
        Self {
            name: SESSION_COOKIE_NAME.to_owned(),
            path: "/".to_owned(),
            same_site: SameSite::Lax,
            secure: true,
            http_only: true,
        }
    }

    /// Creates a validated custom policy for tests or local adapters.
    ///
    /// `__Host-` cookies must remain secure, have path `/`, and omit Domain.
    /// Domain is intentionally not represented by this type.
    ///
    /// # Errors
    ///
    /// Returns [`CookieError::InvalidPolicy`] for a noncanonical name/path or
    /// a policy that weakens secure or HTTP-only custody.
    pub fn new(
        name: &str,
        path: &str,
        same_site: SameSite,
        secure: bool,
        http_only: bool,
    ) -> Result<Self, CookieError> {
        if !name.starts_with("__Host-")
            || name.len() > 128
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
            || path != "/"
            || !secure
            || !http_only
        {
            return Err(CookieError::InvalidPolicy);
        }
        Ok(Self {
            name: name.to_owned(),
            path: path.to_owned(),
            same_site,
            secure,
            http_only,
        })
    }

    /// Emits the `Set-Cookie` header for one opaque session.
    ///
    /// # Errors
    ///
    /// Returns [`CookieError::InvalidLifetime`] unless `max_age_seconds` is
    /// positive and representable by browser cookie implementations.
    pub fn set_cookie(
        &self,
        token: &SessionToken,
        max_age_seconds: u64,
    ) -> Result<String, CookieError> {
        if max_age_seconds == 0 || max_age_seconds > i32::MAX as u64 {
            return Err(CookieError::InvalidLifetime);
        }
        Ok(format!(
            "{}={}; Path={}; Max-Age={max_age_seconds}; SameSite={}; Secure; HttpOnly",
            self.name,
            token.expose(),
            self.path,
            self.same_site_label()
        ))
    }

    /// Emits a deletion header with the same security and scope attributes.
    #[must_use]
    pub fn clear_cookie(&self) -> String {
        format!(
            "{}=; Path={}; Max-Age=0; SameSite={}; Secure; HttpOnly",
            self.name,
            self.path,
            self.same_site_label()
        )
    }

    /// Parses this cookie from a bounded request `Cookie` header.
    ///
    /// # Errors
    ///
    /// Returns [`CookieError`] for oversized, duplicate, malformed, or absent
    /// session cookies and for invalid opaque token syntax.
    pub fn parse_request(&self, header: &str) -> Result<SessionToken, CookieError> {
        if header.len() > 4_096
            || header
                .chars()
                .any(|character| matches!(character, '\r' | '\n'))
        {
            return Err(CookieError::Malformed);
        }
        let mut found = None;
        for pair in header.split(';') {
            let (name, value) = pair.trim().split_once('=').ok_or(CookieError::Malformed)?;
            if name == self.name {
                if found.is_some() {
                    return Err(CookieError::Duplicate);
                }
                found = Some(SessionToken::parse(value)?);
            }
        }
        found.ok_or(CookieError::Missing)
    }

    const fn same_site_label(&self) -> &'static str {
        match self.same_site {
            SameSite::Lax => "Lax",
            SameSite::Strict => "Strict",
        }
    }
}

/// Session-cookie policy or parsing failure.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum CookieError {
    /// Policy weakens required `__Host-`, Secure, HTTP-only, or path behavior.
    #[error("session cookie policy is invalid")]
    InvalidPolicy,
    /// Cookie lifetime is zero or exceeds browser bounds.
    #[error("session cookie lifetime is invalid")]
    InvalidLifetime,
    /// Request does not carry the session cookie.
    #[error("session cookie is missing")]
    Missing,
    /// Request carries the session cookie more than once.
    #[error("session cookie is duplicated")]
    Duplicate,
    /// Request cookie header is oversized or malformed.
    #[error("cookie header is malformed")]
    Malformed,
    /// Opaque token is malformed.
    #[error(transparent)]
    Token(#[from] TokenError),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn production_cookie_is_host_only_secure_http_only_and_round_trips() {
        let policy = SessionCookiePolicy::production();
        let token = SessionToken::generate().unwrap();
        let header = policy.set_cookie(&token, 3_600).unwrap();
        assert!(header.starts_with("__Host-pwmtf_session="));
        assert!(header.contains("Path=/"));
        assert!(header.contains("SameSite=Lax"));
        assert!(header.contains("Secure"));
        assert!(header.contains("HttpOnly"));
        assert!(!header.contains("Domain="));
        assert_eq!(
            policy
                .parse_request(&format!(
                    "theme=dark; {}={}",
                    SESSION_COOKIE_NAME,
                    token.expose()
                ))
                .unwrap(),
            token
        );
    }

    #[test]
    fn duplicate_and_weakened_cookies_fail_closed() {
        let policy = SessionCookiePolicy::production();
        let token = SessionToken::generate().unwrap();
        assert_eq!(
            policy.parse_request(&format!(
                "{0}={1}; {0}={1}",
                SESSION_COOKIE_NAME,
                token.expose()
            )),
            Err(CookieError::Duplicate)
        );
        assert_eq!(
            SessionCookiePolicy::new("pwmtf", "/", SameSite::Lax, true, true),
            Err(CookieError::InvalidPolicy)
        );
        assert_eq!(
            SessionCookiePolicy::new(SESSION_COOKIE_NAME, "/x", SameSite::Lax, true, true),
            Err(CookieError::InvalidPolicy)
        );
    }
}

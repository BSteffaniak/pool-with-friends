//! Cryptographic generation and hashing for opaque session and invitation tokens.

use crate::{InvitationTokenHash, SessionTokenHash};
use sha2::{Digest as _, Sha256};
use thiserror::Error;

/// Entropy bytes in each opaque token.
pub const TOKEN_BYTES: usize = 32;
/// Hexadecimal characters in each transport-safe token.
pub const TOKEN_CHARACTERS: usize = TOKEN_BYTES * 2;

/// Raw opaque session token returned only to an authenticated client.
///
/// Debug output deliberately omits the secret value.
#[derive(Clone, Eq, PartialEq)]
pub struct SessionToken(String);

impl SessionToken {
    /// Generates a token from operating-system cryptographic randomness.
    ///
    /// # Errors
    ///
    /// Returns [`TokenError::Randomness`] when secure randomness is unavailable.
    pub fn generate() -> Result<Self, TokenError> {
        Ok(Self(generate_token()?))
    }

    /// Parses a transport token after strict canonical validation.
    ///
    /// # Errors
    ///
    /// Returns [`TokenError::Invalid`] unless the token is exactly 256 bits
    /// encoded as lowercase hexadecimal.
    pub fn parse(value: &str) -> Result<Self, TokenError> {
        validate_token(value)?;
        Ok(Self(value.to_owned()))
    }

    /// Returns the secret only at the cookie/transport boundary.
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }

    /// Derives the fixed-size hash used for persistence and lookup.
    #[must_use]
    pub fn hash(&self) -> SessionTokenHash {
        SessionTokenHash::new(hash(self.0.as_bytes()))
    }
}

impl std::fmt::Debug for SessionToken {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("SessionToken([REDACTED])")
    }
}

/// Raw opaque invitation token returned only to its creator.
///
/// Debug output deliberately omits the secret value.
#[derive(Clone, Eq, PartialEq)]
pub struct InvitationToken(String);

impl InvitationToken {
    /// Generates a token from operating-system cryptographic randomness.
    ///
    /// # Errors
    ///
    /// Returns [`TokenError::Randomness`] when secure randomness is unavailable.
    pub fn generate() -> Result<Self, TokenError> {
        Ok(Self(generate_token()?))
    }

    /// Parses a transport token after strict canonical validation.
    ///
    /// # Errors
    ///
    /// Returns [`TokenError::Invalid`] unless the token is exactly 256 bits
    /// encoded as lowercase hexadecimal.
    pub fn parse(value: &str) -> Result<Self, TokenError> {
        validate_token(value)?;
        Ok(Self(value.to_owned()))
    }

    /// Returns the secret only at the invitation-link transport boundary.
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }

    /// Derives the fixed-size hash used for persistence and lookup.
    #[must_use]
    pub fn hash(&self) -> InvitationTokenHash {
        InvitationTokenHash::new(hash(self.0.as_bytes()))
    }
}

impl std::fmt::Debug for InvitationToken {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("InvitationToken([REDACTED])")
    }
}

/// Cryptographic opaque-token failure.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum TokenError {
    /// Operating-system cryptographic randomness is unavailable.
    #[error("secure token generation failed")]
    Randomness,
    /// Transport token is noncanonical or malformed.
    #[error("opaque token is invalid")]
    Invalid,
}

fn generate_token() -> Result<String, TokenError> {
    let mut bytes = [0_u8; TOKEN_BYTES];
    getrandom::fill(&mut bytes).map_err(|_| TokenError::Randomness)?;
    Ok(hex(&bytes))
}

fn validate_token(value: &str) -> Result<(), TokenError> {
    if value.len() != TOKEN_CHARACTERS
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(TokenError::Invalid);
    }
    Ok(())
}

fn hash(value: &[u8]) -> [u8; 32] {
    Sha256::digest(value).into()
}

fn hex(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        write!(output, "{byte:02x}").expect("writing to String cannot fail");
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_tokens_are_canonical_distinct_and_hashable() {
        let first = SessionToken::generate().unwrap();
        let second = SessionToken::generate().unwrap();
        assert_eq!(first.expose().len(), TOKEN_CHARACTERS);
        assert_ne!(first, second);
        assert_ne!(first.hash(), second.hash());
        assert_eq!(SessionToken::parse(first.expose()).unwrap(), first);
    }

    #[test]
    fn raw_tokens_are_redacted_and_strictly_parsed() {
        let token = InvitationToken::generate().unwrap();
        assert_eq!(format!("{token:?}"), "InvitationToken([REDACTED])");
        assert!(!format!("{token:?}").contains(token.expose()));
        assert!(matches!(
            InvitationToken::parse(&token.expose().to_uppercase()),
            Err(TokenError::Invalid)
        ));
        assert!(matches!(
            SessionToken::parse("too-short"),
            Err(TokenError::Invalid)
        ));
    }
}

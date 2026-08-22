//! Browser WebSocket lifecycle state independent of JavaScript/Bevy authority.

use pwmtf_protocol::{CommandEnvelope, PROTOCOL_VERSION, ProtocolError, SnapshotEnvelope};
use thiserror::Error;

use crate::prediction::{PredictionError, PredictionState, Reconciliation};

/// Browser transport lifecycle.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConnectionStatus {
    /// No live socket exists.
    Disconnected,
    /// Browser is establishing a socket.
    Connecting,
    /// Protocol negotiation is pending.
    Negotiating,
    /// Socket is authenticated, authorized, and initialized by a snapshot.
    Ready,
    /// Connection failed and should use bounded retry delay.
    Backoff,
}

/// Client-owned WebSocket lifecycle and authoritative snapshot ingestion.
pub struct BrowserTransport {
    status: ConnectionStatus,
    protocol_negotiated: bool,
    retry_attempt: u8,
    prediction: Option<PredictionState>,
}

impl Default for BrowserTransport {
    fn default() -> Self {
        Self {
            status: ConnectionStatus::Disconnected,
            protocol_negotiated: false,
            retry_attempt: 0,
            prediction: None,
        }
    }
}

impl BrowserTransport {
    /// Returns current transport lifecycle.
    #[must_use]
    pub const fn status(&self) -> ConnectionStatus {
        self.status
    }

    /// Returns current canonical/predicted client state after initialization.
    #[must_use]
    pub const fn prediction(&self) -> Option<&PredictionState> {
        self.prediction.as_ref()
    }

    /// Returns whether one authoritative revision has already been ingested.
    #[must_use]
    pub fn has_revision(&self, revision: u64) -> bool {
        self.prediction.as_ref().is_some_and(|prediction| {
            prediction.authoritative_revision() == revision && !prediction.has_pending_prediction()
        })
    }

    /// Begins a new browser connection attempt.
    pub const fn connecting(&mut self) {
        self.status = ConnectionStatus::Connecting;
        self.protocol_negotiated = false;
    }

    /// Marks the browser socket open and returns the bounded negotiation offer.
    #[must_use]
    pub fn opened(&mut self) -> String {
        self.status = ConnectionStatus::Negotiating;
        PROTOCOL_VERSION.to_string()
    }

    /// Accepts only the exact supported protocol negotiation response.
    ///
    /// # Errors
    ///
    /// Returns [`TransportClientError::Negotiation`] for malformed or unsupported responses.
    pub fn negotiated(&mut self, response: &str) -> Result<(), TransportClientError> {
        if self.status != ConnectionStatus::Negotiating {
            return Err(TransportClientError::NotReady);
        }
        let version = response.parse::<u16>().map_err(|_| {
            self.disconnected();
            TransportClientError::Negotiation
        })?;
        if version != PROTOCOL_VERSION {
            self.disconnected();
            return Err(TransportClientError::Negotiation);
        }
        self.protocol_negotiated = true;
        self.retry_attempt = 0;
        Ok(())
    }

    /// Decodes an initial/reconnect snapshot or reconciles an update.
    ///
    /// # Errors
    ///
    /// Returns [`TransportClientError`] for malformed framing, checksum/version
    /// rejection, or stale authoritative data.
    pub fn receive_snapshot(
        &mut self,
        bytes: &[u8],
    ) -> Result<Option<Reconciliation>, TransportClientError> {
        if !self.protocol_negotiated
            || !matches!(
                self.status,
                ConnectionStatus::Negotiating | ConnectionStatus::Ready
            )
        {
            return Err(TransportClientError::NotReady);
        }
        let snapshot = SnapshotEnvelope::from_bytes(bytes)?;
        if self.has_revision(snapshot.revision) {
            return Ok(None);
        }
        let disposition = if let Some(prediction) = &mut self.prediction {
            Some(prediction.reconcile(&snapshot)?)
        } else {
            self.prediction = Some(PredictionState::from_snapshot(&snapshot)?);
            None
        };
        self.retry_attempt = 0;
        self.status = ConnectionStatus::Ready;
        Ok(disposition)
    }

    /// Creates and locally predicts one cue-ball placement command.
    ///
    /// # Errors
    ///
    /// Returns [`TransportClientError::NotReady`] unless initialization
    /// completed, or a prediction/domain failure for invalid local state.
    pub fn predict_cue_ball_placement(
        &mut self,
        command_id: pwmtf_protocol::CommandId,
        position: pwmtf_game_domain::Vector,
    ) -> Result<CommandEnvelope, TransportClientError> {
        if self.status != ConnectionStatus::Ready {
            return Err(TransportClientError::NotReady);
        }
        self.prediction
            .as_mut()
            .ok_or(TransportClientError::NotReady)?
            .predict_cue_ball_placement(command_id, position)
            .map_err(Into::into)
    }

    /// Creates and locally predicts one bounded shot command.
    ///
    /// # Errors
    ///
    /// Returns [`TransportClientError::NotReady`] unless initialization
    /// completed, or a prediction/domain failure for invalid local state.
    pub fn predict_shot(
        &mut self,
        command_id: pwmtf_protocol::CommandId,
        shot: pwmtf_game_domain::VersionedShotCommand,
        called_pocket: Option<pwmtf_game_domain::PocketId>,
    ) -> Result<CommandEnvelope, TransportClientError> {
        if self.status != ConnectionStatus::Ready {
            return Err(TransportClientError::NotReady);
        }
        self.prediction
            .as_mut()
            .ok_or(TransportClientError::NotReady)?
            .predict_shot(command_id, shot, called_pocket)
            .map_err(Into::into)
    }

    /// Creates and locally predicts one concession command.
    ///
    /// # Errors
    ///
    /// Returns [`TransportClientError::NotReady`] unless initialization
    /// completed, or a prediction/domain failure for invalid local state.
    pub fn predict_concession(
        &mut self,
        command_id: pwmtf_protocol::CommandId,
        player: pwmtf_game_domain::Player,
    ) -> Result<CommandEnvelope, TransportClientError> {
        if self.status != ConnectionStatus::Ready {
            return Err(TransportClientError::NotReady);
        }
        self.prediction
            .as_mut()
            .ok_or(TransportClientError::NotReady)?
            .predict_concession(command_id, player)
            .map_err(Into::into)
    }

    /// Cancels local prediction after a lifecycle loss and enters retry backoff.
    pub fn disconnected(&mut self) {
        if let Some(prediction) = &mut self.prediction {
            prediction.abandon_prediction();
        }
        self.status = ConnectionStatus::Backoff;
        self.protocol_negotiated = false;
        self.retry_attempt = self.retry_attempt.saturating_add(1).min(8);
    }

    /// Returns bounded exponential retry delay, capped at 30 seconds.
    #[must_use]
    pub fn retry_delay_ms(&self) -> u64 {
        250_u64
            .saturating_mul(1_u64 << self.retry_attempt.min(7))
            .min(30_000)
    }
}

/// Browser transport lifecycle failure.
#[derive(Debug, Error)]
pub enum TransportClientError {
    /// Protocol negotiation failed closed.
    #[error("WebSocket protocol negotiation failed")]
    Negotiation,
    /// Command was attempted before authoritative initialization.
    #[error("WebSocket is not ready")]
    NotReady,
    /// Wire frame was malformed or incompatible.
    #[error(transparent)]
    Protocol(#[from] ProtocolError),
    /// Snapshot/prediction reconciliation failed.
    #[error(transparent)]
    Prediction(#[from] PredictionError),
}

#[cfg(test)]
mod tests {
    use pwmtf_game_domain::{
        MatchState, PhysicsProfile, Player, RackSeed, RulesProfile, TableGeometry,
    };

    use super::*;

    fn snapshot(revision: u64) -> Vec<u8> {
        let state = MatchState::new(
            RulesProfile::standard(),
            PhysicsProfile::standard(),
            TableGeometry::standard(),
            RackSeed::new(42),
            Player::One,
        )
        .unwrap();
        SnapshotEnvelope::new(revision, state.checksum(), state.to_bytes())
            .unwrap()
            .to_bytes()
    }

    #[test]
    fn lifecycle_negotiates_initializes_and_backs_off() {
        let mut transport = BrowserTransport::default();
        transport.connecting();
        assert_eq!(transport.opened(), PROTOCOL_VERSION.to_string());
        assert_eq!(transport.status(), ConnectionStatus::Negotiating);
        transport.negotiated("1").unwrap();
        assert_eq!(transport.status(), ConnectionStatus::Negotiating);
        assert_eq!(transport.receive_snapshot(&snapshot(0)).unwrap(), None);
        assert_eq!(transport.status(), ConnectionStatus::Ready);
        transport.disconnected();
        assert_eq!(transport.status(), ConnectionStatus::Backoff);
        assert_eq!(transport.retry_delay_ms(), 500);
    }

    #[test]
    fn ready_transport_predicts_cue_ball_placement() {
        let mut transport = BrowserTransport::default();
        transport.connecting();
        let _ = transport.opened();
        transport.negotiated("1").unwrap();
        transport.receive_snapshot(&snapshot(0)).unwrap();
        let mut state =
            MatchState::from_bytes(&SnapshotEnvelope::from_bytes(&snapshot(0)).unwrap().snapshot)
                .unwrap();
        state.timeout_turn().unwrap();
        let advanced = SnapshotEnvelope::new(1, state.checksum(), state.to_bytes())
            .unwrap()
            .to_bytes();
        transport.receive_snapshot(&advanced).unwrap();
        let cue_position = state
            .table()
            .balls()
            .iter()
            .find(|ball| ball.id == pwmtf_game_domain::BallId::CUE)
            .unwrap()
            .position;
        let command = transport
            .predict_cue_ball_placement(pwmtf_protocol::CommandId::new([3; 16]), cue_position)
            .unwrap();
        assert_eq!(command.expected_revision, 1);
        assert!(!transport.prediction().unwrap().predicted().ball_in_hand());
    }

    #[test]
    fn duplicate_authoritative_revision_is_harmless_after_confirmation() {
        let mut transport = BrowserTransport::default();
        transport.connecting();
        let _ = transport.opened();
        transport.negotiated("1").unwrap();
        assert_eq!(transport.receive_snapshot(&snapshot(0)).unwrap(), None);
        assert!(transport.has_revision(0));
        assert_eq!(transport.receive_snapshot(&snapshot(0)).unwrap(), None);
        assert_eq!(transport.status(), ConnectionStatus::Ready);
    }

    #[test]
    fn unsupported_negotiation_and_malformed_snapshots_fail_closed() {
        let mut transport = BrowserTransport::default();
        let _ = transport.opened();
        assert!(matches!(
            transport.negotiated("2"),
            Err(TransportClientError::Negotiation)
        ));
        assert_eq!(transport.status(), ConnectionStatus::Backoff);
        assert_eq!(transport.retry_delay_ms(), 500);
        transport.connecting();
        let _ = transport.opened();
        transport.negotiated("1").unwrap();
        assert!(transport.receive_snapshot(&[0, 1]).is_err());
    }

    #[test]
    fn snapshot_before_negotiation_does_not_initialize_transport() {
        let mut transport = BrowserTransport::default();
        transport.connecting();
        let _ = transport.opened();
        assert!(matches!(
            transport.receive_snapshot(&snapshot(0)),
            Err(TransportClientError::NotReady)
        ));
    }
}

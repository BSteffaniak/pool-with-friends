//! WASM-to-browser WebSocket bridge with bounded reconnect lifecycle.

use std::cell::RefCell;

use js_sys::{ArrayBuffer, Uint8Array};
use wasm_bindgen::{JsCast as _, closure::Closure};
use web_sys::{BinaryType, CloseEvent, ErrorEvent, Event, MessageEvent, WebSocket};

use crate::transport::{BrowserTransport, ConnectionStatus};

thread_local! {
    static SOCKET: RefCell<Option<WebSocket>> = const { RefCell::new(None) };
    static TRANSPORT: RefCell<BrowserTransport> = RefCell::new(BrowserTransport::default());
}

/// Connects the browser socket to a server-authorized match subscription URL.
///
/// The URL must be same-origin `ws:`/`wss:` and include server-owned match
/// subscription identifiers. Authentication remains in the secure session cookie.
///
/// # Errors
///
/// Returns a JavaScript exception for malformed URLs or socket construction failure.
pub fn connect(url: &str) -> Result<(), wasm_bindgen::JsValue> {
    disconnect();
    TRANSPORT.with(|transport| transport.borrow_mut().connecting());
    let socket = WebSocket::new(url)?;
    socket.set_binary_type(BinaryType::Arraybuffer);

    let opened_socket = socket.clone();
    let on_open = Closure::<dyn FnMut(Event)>::new(move |_| {
        let offer = TRANSPORT.with(|transport| transport.borrow_mut().opened());
        if opened_socket.send_with_str(&offer).is_err() {
            TRANSPORT.with(|transport| transport.borrow_mut().disconnected());
            let _ = opened_socket.close();
        }
    });
    socket.set_onopen(Some(on_open.as_ref().unchecked_ref()));
    on_open.forget();

    let message_socket = socket.clone();
    let on_message = Closure::<dyn FnMut(MessageEvent)>::new(move |event: MessageEvent| {
        if let Some(text) = event.data().as_string() {
            let result = TRANSPORT.with(|transport| {
                let mut transport = transport.borrow_mut();
                if transport.status() == ConnectionStatus::Ready {
                    if text == "rejected" {
                        transport.command_rejected();
                    }
                    Ok(())
                } else {
                    transport.negotiated(&text)
                }
            });
            if result.is_err() {
                TRANSPORT.with(|transport| {
                    let mut transport = transport.borrow_mut();
                    if transport.status() != ConnectionStatus::Backoff {
                        transport.disconnected();
                    }
                });
                let _ = message_socket.close();
            }
            return;
        }
        let Ok(buffer) = event.data().dyn_into::<ArrayBuffer>() else {
            TRANSPORT.with(|transport| transport.borrow_mut().disconnected());
            let _ = message_socket.close();
            return;
        };
        let bytes = Uint8Array::new(&buffer).to_vec();
        let result = TRANSPORT.with(|transport| transport.borrow_mut().receive_snapshot(&bytes));
        if result.is_err() {
            TRANSPORT.with(|transport| transport.borrow_mut().disconnected());
            let _ = message_socket.close();
        }
    });
    socket.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
    on_message.forget();

    let on_error = Closure::<dyn FnMut(ErrorEvent)>::new(move |_| {
        TRANSPORT.with(|transport| transport.borrow_mut().disconnected());
    });
    socket.set_onerror(Some(on_error.as_ref().unchecked_ref()));
    on_error.forget();

    let on_close = Closure::<dyn FnMut(CloseEvent)>::new(move |_| {
        TRANSPORT.with(|transport| transport.borrow_mut().disconnected());
        SOCKET.with(|socket| socket.borrow_mut().take());
    });
    socket.set_onclose(Some(on_close.as_ref().unchecked_ref()));
    on_close.forget();

    SOCKET.with(|stored| *stored.borrow_mut() = Some(socket));
    Ok(())
}

/// Sends one bounded, already-authorized command frame.
///
/// # Errors
///
/// Returns a JavaScript exception unless the protocol lifecycle is ready or
/// browser socket transmission fails.
pub fn send_command(frame: &[u8]) -> Result<(), wasm_bindgen::JsValue> {
    if TRANSPORT.with(|transport| transport.borrow().status()) != ConnectionStatus::Ready {
        return Err(wasm_bindgen::JsValue::from_str("PWMTF socket is not ready"));
    }
    SOCKET.with(|socket| {
        socket
            .borrow()
            .as_ref()
            .ok_or_else(|| wasm_bindgen::JsValue::from_str("PWMTF socket is disconnected"))?
            .send_with_u8_array(frame)
    })
}

fn send_predicted_frame(frame: &[u8]) -> Result<(), wasm_bindgen::JsValue> {
    let result = send_command(frame);
    if result.is_err() {
        TRANSPORT.with(|transport| transport.borrow_mut().disconnected());
    }
    result
}

/// Predicts and sends one bounded cue-ball placement command.
///
/// # Errors
///
/// Returns a JavaScript exception unless the protocol lifecycle is ready,
/// prediction fails, or browser socket transmission fails.
pub fn predict_and_send_cue_ball_placement(
    command_id: pwmtf_protocol::CommandId,
    position: pwmtf_game_domain::Vector,
) -> Result<(), wasm_bindgen::JsValue> {
    let frame = TRANSPORT
        .with(|transport| {
            transport
                .borrow_mut()
                .predict_cue_ball_placement(command_id, position)
                .map(pwmtf_protocol::CommandEnvelope::to_bytes)
        })
        .map_err(|error| wasm_bindgen::JsValue::from_str(&error.to_string()))?;
    send_predicted_frame(&frame)
}

/// Predicts and sends one bounded shot command.
///
/// # Errors
///
/// Returns a JavaScript exception unless the protocol lifecycle is ready,
/// prediction fails, or browser socket transmission fails.
pub fn predict_and_send_shot(
    command_id: pwmtf_protocol::CommandId,
    shot: pwmtf_game_domain::VersionedShotCommand,
    called_pocket: Option<pwmtf_game_domain::PocketId>,
) -> Result<(), wasm_bindgen::JsValue> {
    let frame = TRANSPORT
        .with(|transport| {
            transport
                .borrow_mut()
                .predict_shot(command_id, shot, called_pocket)
                .map(pwmtf_protocol::CommandEnvelope::to_bytes)
        })
        .map_err(|error| wasm_bindgen::JsValue::from_str(&error.to_string()))?;
    send_predicted_frame(&frame)
}

/// Predicts and sends one explicit concession command.
///
/// # Errors
///
/// Returns a JavaScript exception unless the protocol lifecycle is ready,
/// prediction fails, or browser socket transmission fails.
pub fn predict_and_send_concession(
    command_id: pwmtf_protocol::CommandId,
    player: pwmtf_game_domain::Player,
) -> Result<(), wasm_bindgen::JsValue> {
    let frame = TRANSPORT
        .with(|transport| {
            transport
                .borrow_mut()
                .predict_concession(command_id, player)
                .map(pwmtf_protocol::CommandEnvelope::to_bytes)
        })
        .map_err(|error| wasm_bindgen::JsValue::from_str(&error.to_string()))?;
    send_predicted_frame(&frame)
}

/// Closes and forgets the current browser socket and predicted in-flight work.
pub fn disconnect() {
    SOCKET.with(|socket| {
        if let Some(socket) = socket.borrow_mut().take() {
            socket.set_onopen(None);
            socket.set_onmessage(None);
            socket.set_onerror(None);
            socket.set_onclose(None);
            let _ = socket.close();
        }
    });
    TRANSPORT.with(|transport| transport.borrow_mut().disconnected());
}

/// Sets the participant seat derived from authenticated durable membership.
pub fn set_local_player(player: pwmtf_game_domain::Player) {
    TRANSPORT.with(|transport| transport.borrow_mut().set_local_player(player));
}

/// Returns whether the local participant may submit active-player commands.
#[must_use]
pub fn accepts_active_player_command() -> bool {
    TRANSPORT.with(|transport| transport.borrow().accepts_active_player_command())
}

/// Returns and clears whether the authoritative server rejected the latest command.
pub fn take_command_rejected() -> bool {
    TRANSPORT.with(|transport| transport.borrow_mut().take_command_rejected())
}

/// Returns whether the authoritative match accepts live gameplay commands.
#[must_use]
pub fn accepts_gameplay_commands() -> bool {
    TRANSPORT.with(|transport| transport.borrow().accepts_gameplay_commands())
}

/// Returns whether authoritative state permits cue-ball placement.
#[must_use]
pub fn ball_in_hand() -> bool {
    TRANSPORT.with(|transport| {
        transport
            .borrow()
            .prediction()
            .is_some_and(|prediction| prediction.predicted().ball_in_hand())
    })
}

/// Returns the latest authoritative match presentation facts.
#[must_use]
pub fn authoritative_match_info() -> Option<(
    pwmtf_game_domain::Player,
    Option<pwmtf_game_domain::MatchOutcome>,
)> {
    TRANSPORT.with(|transport| {
        let transport = transport.borrow();
        let state = transport.authoritative_state()?;
        let outcome = match state.status() {
            pwmtf_game_domain::MatchStatus::InProgress => None,
            pwmtf_game_domain::MatchStatus::Completed(outcome) => Some(outcome),
        };
        Some((state.active_player(), outcome))
    })
}

/// Returns the authoritative revision after snapshot initialization.
#[must_use]
pub fn authoritative_revision() -> Option<u64> {
    TRANSPORT.with(|transport| {
        transport
            .borrow()
            .prediction()
            .map(crate::prediction::PredictionState::authoritative_revision)
    })
}

/// Returns the current canonical/predicted state checksum after initialization.
#[must_use]
pub fn predicted_checksum() -> Option<u64> {
    TRANSPORT.with(|transport| {
        transport
            .borrow()
            .prediction()
            .map(|prediction| prediction.predicted().checksum())
    })
}

/// Returns one predicted canonical ball's position and pocket state.
#[must_use]
pub fn predicted_ball(number: u8) -> Option<(pwmtf_game_domain::Vector, bool)> {
    TRANSPORT.with(|transport| {
        transport
            .borrow()
            .prediction()?
            .predicted()
            .table()
            .balls()
            .iter()
            .find(|ball| ball.id.number() == number)
            .map(|ball| (ball.position, ball.pocketed))
    })
}

/// Returns the current retry delay for JavaScript lifecycle scheduling.
#[must_use]
pub fn retry_delay_ms() -> u64 {
    TRANSPORT.with(|transport| transport.borrow().retry_delay_ms())
}

/// Returns whether the failed socket lifecycle should be reconnected.
#[must_use]
pub fn needs_reconnect() -> bool {
    TRANSPORT.with(|transport| transport.borrow().status() == ConnectionStatus::Backoff)
}

/// Returns whether an authoritative snapshot initialized the live socket.
#[must_use]
pub fn is_ready() -> bool {
    TRANSPORT.with(|transport| transport.borrow().status() == ConnectionStatus::Ready)
}

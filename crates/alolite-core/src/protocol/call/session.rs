//! Pure `CallSession` state machine (`INV-5`, `INV-7`, `INV-9`).
//!
//! Zero `rusqlite`, network, or audio device imports; transitions consume [`CallEvent`]
//! and emit pure [`CallEffect`] values for `CallManager` to execute.

use anyhow::{bail, Result};

use super::signaling::CallSignalCode;

/// Lifecycle state of a single 1-1 ZRTC audio call (`INV-7`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CallState {
    Idle,
    OutgoingRinging,
    IncomingRinging,
    Connecting,
    Active,
    Ending,
}

/// Direction of the current call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CallDirection {
    Outgoing,
    Incoming,
}

/// Terminal reason recorded when a call transitions to [`CallState::Ending`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CallEndReason {
    Normal,
    Cancelled,
    Rejected,
    Busy,
    RingTimeout,
    MediaTimeout,
    PeerEnded,
    NetworkError,
}

/// Pure input events driving [`CallSession::apply`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallEvent {
    StartOutgoing {
        call_id: u32,
        sess_id: String,
        peer_id: String,
        srtp_mode: u8,
    },
    IncomingInvite {
        call_id: u32,
        sess_id: String,
        peer_id: String,
        srtp_mode: u8,
    },
    PeerRingRing {
        call_id: u32,
    },
    PeerAnswered {
        call_id: u32,
        accepted: bool,
    },
    AcceptIncoming {
        call_id: u32,
    },
    RejectIncoming {
        call_id: u32,
    },
    MediaReady {
        call_id: u32,
    },
    LocalCancel,
    LocalEnd,
    PeerCancelled {
        call_id: u32,
    },
    PeerEnded {
        call_id: u32,
    },
    RingTimeout,
    MediaWatchdogTimeout,
    SetMuted {
        muted: bool,
    },
    SetHeld {
        held: bool,
    },
    TeardownComplete,
}

/// Pure side-effect descriptors emitted by [`CallSession::apply`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallEffect {
    StateChanged(CallState),
    SendSignal(CallSignalCode),
    RejectBusy {
        call_id: u32,
        sess_id: String,
        peer_id: String,
    },
    StartZrtpAndMedia,
    StopZrtpAndMedia {
        send_end_call_pkt: bool,
    },
    MuteChanged(bool),
    HoldChanged(bool),
    CallFinished {
        call_id: u32,
        reason: CallEndReason,
    },
}

/// Pure single-call 1-1 ZRTC state machine (`INV-7`).
#[derive(Debug, Clone)]
pub struct CallSession {
    state: CallState,
    call_id: Option<u32>,
    sess_id: Option<String>,
    peer_id: Option<String>,
    direction: Option<CallDirection>,
    srtp_mode: u8,
    is_muted: bool,
    is_held: bool,
    end_reason: Option<CallEndReason>,
}

impl Default for CallSession {
    fn default() -> Self {
        Self::new()
    }
}

impl CallSession {
    /// Create a new [`CallSession`] in [`CallState::Idle`].
    pub fn new() -> Self {
        Self {
            state: CallState::Idle,
            call_id: None,
            sess_id: None,
            peer_id: None,
            direction: None,
            srtp_mode: 1,
            is_muted: false,
            is_held: false,
            end_reason: None,
        }
    }

    /// Current lifecycle state.
    pub const fn state(&self) -> CallState {
        self.state
    }

    /// Active `callId`, if non-`Idle`.
    pub const fn call_id(&self) -> Option<u32> {
        self.call_id
    }

    /// Active `sessId`, if non-`Idle`.
    pub fn sess_id(&self) -> Option<&str> {
        self.sess_id.as_deref()
    }

    /// Active remote peer user ID, if non-`Idle`.
    pub fn peer_id(&self) -> Option<&str> {
        self.peer_id.as_deref()
    }

    /// Active call direction, if non-`Idle`.
    pub const fn direction(&self) -> Option<CallDirection> {
        self.direction
    }

    /// Negotiated `srtpMode` (`1` by default per `INV-8`).
    pub const fn srtp_mode(&self) -> u8 {
        self.srtp_mode
    }

    /// Whether local microphone is muted.
    pub const fn is_muted(&self) -> bool {
        self.is_muted
    }

    /// Whether the call is currently on hold.
    pub const fn is_held(&self) -> bool {
        self.is_held
    }

    /// Apply a [`CallEvent`] and return the resulting [`CallEffect`] list.
    ///
    /// # Errors
    /// Returns an error when an illegal state transition is requested (e.g. starting
    /// an outgoing call while another call is already active).
    pub fn apply(&mut self, event: CallEvent) -> Result<Vec<CallEffect>> {
        match event {
            CallEvent::StartOutgoing {
                call_id,
                sess_id,
                peer_id,
                srtp_mode,
            } => {
                if self.state != CallState::Idle {
                    bail!("cannot start outgoing call while in state {:?}", self.state);
                }
                self.state = CallState::OutgoingRinging;
                self.call_id = Some(call_id);
                self.sess_id = Some(sess_id);
                self.peer_id = Some(peer_id);
                self.direction = Some(CallDirection::Outgoing);
                self.srtp_mode = srtp_mode;
                self.is_muted = false;
                self.is_held = false;
                self.end_reason = None;
                Ok(vec![CallEffect::StateChanged(CallState::OutgoingRinging)])
            }

            CallEvent::IncomingInvite {
                call_id,
                sess_id,
                peer_id,
                srtp_mode,
            } => {
                if self.state != CallState::Idle {
                    // ADV-1: deduplicate repeated 401 delivery for the same callId.
                    if self.call_id == Some(call_id) {
                        return Ok(Vec::new());
                    }
                    // ADV-2 / INV-7: reject concurrent second call as busy, keep active call intact.
                    return Ok(vec![CallEffect::RejectBusy {
                        call_id,
                        sess_id,
                        peer_id,
                    }]);
                }
                self.state = CallState::IncomingRinging;
                self.call_id = Some(call_id);
                self.sess_id = Some(sess_id);
                self.peer_id = Some(peer_id);
                self.direction = Some(CallDirection::Incoming);
                self.srtp_mode = srtp_mode;
                self.is_muted = false;
                self.is_held = false;
                self.end_reason = None;
                Ok(vec![
                    CallEffect::StateChanged(CallState::IncomingRinging),
                    CallEffect::SendSignal(CallSignalCode::Request),
                    CallEffect::SendSignal(CallSignalCode::RingRing),
                ])
            }

            CallEvent::PeerRingRing { call_id } => {
                if self.call_id != Some(call_id) {
                    return Ok(Vec::new());
                }
                Ok(Vec::new())
            }

            CallEvent::PeerAnswered { call_id, accepted } => {
                if self.call_id != Some(call_id) || self.state != CallState::OutgoingRinging {
                    return Ok(Vec::new());
                }
                if accepted {
                    self.state = CallState::Connecting;
                    Ok(vec![
                        CallEffect::StateChanged(CallState::Connecting),
                        CallEffect::SendSignal(CallSignalCode::AnswerAck),
                        CallEffect::StartZrtpAndMedia,
                    ])
                } else {
                    self.begin_ending(CallEndReason::Rejected, false, None)
                }
            }

            CallEvent::AcceptIncoming { call_id } => {
                if self.call_id != Some(call_id) || self.state != CallState::IncomingRinging {
                    bail!("no matching incoming ringing call {call_id}");
                }
                self.state = CallState::Connecting;
                Ok(vec![
                    CallEffect::StateChanged(CallState::Connecting),
                    CallEffect::SendSignal(CallSignalCode::Answer),
                    CallEffect::StartZrtpAndMedia,
                ])
            }

            CallEvent::RejectIncoming { call_id } => {
                if self.call_id != Some(call_id) || self.state != CallState::IncomingRinging {
                    return Ok(Vec::new());
                }
                self.begin_ending(CallEndReason::Rejected, false, Some(CallSignalCode::Answer))
            }

            CallEvent::MediaReady { call_id } => {
                if self.call_id != Some(call_id) || self.state != CallState::Connecting {
                    return Ok(Vec::new());
                }
                self.state = CallState::Active;
                let mut effects = Vec::with_capacity(2);
                // Outgoing calls emit 11301 request (416) once ZRTP/media is ready;
                // incoming calls already emitted 11301 on IncomingInvite.
                if self.direction == Some(CallDirection::Outgoing) {
                    effects.push(CallEffect::SendSignal(CallSignalCode::Request));
                }
                effects.push(CallEffect::StateChanged(CallState::Active));
                Ok(effects)
            }

            CallEvent::LocalCancel => {
                if self.state != CallState::OutgoingRinging {
                    return Ok(Vec::new());
                }
                self.begin_ending(CallEndReason::Cancelled, true, Some(CallSignalCode::Cancel))
            }

            CallEvent::LocalEnd => {
                // ADV-4: idempotent teardown if already Ending or Idle.
                match self.state {
                    CallState::Idle | CallState::Ending => Ok(Vec::new()),
                    CallState::OutgoingRinging => self.begin_ending(
                        CallEndReason::Cancelled,
                        true,
                        Some(CallSignalCode::Cancel),
                    ),
                    CallState::IncomingRinging => self.begin_ending(
                        CallEndReason::Rejected,
                        false,
                        Some(CallSignalCode::Answer),
                    ),
                    CallState::Connecting | CallState::Active => self.begin_ending(
                        CallEndReason::Normal,
                        true,
                        Some(CallSignalCode::EndCall),
                    ),
                }
            }

            CallEvent::PeerCancelled { call_id } => {
                if self.call_id != Some(call_id)
                    || matches!(self.state, CallState::Idle | CallState::Ending)
                {
                    return Ok(Vec::new());
                }
                self.begin_ending(CallEndReason::Cancelled, false, None)
            }

            CallEvent::PeerEnded { call_id } => {
                // ADV-4: idempotent teardown if already Ending or Idle.
                if self.call_id != Some(call_id)
                    || matches!(self.state, CallState::Idle | CallState::Ending)
                {
                    return Ok(Vec::new());
                }
                self.begin_ending(CallEndReason::PeerEnded, true, None)
            }

            CallEvent::RingTimeout => {
                if !matches!(
                    self.state,
                    CallState::OutgoingRinging | CallState::IncomingRinging
                ) {
                    return Ok(Vec::new());
                }
                let sig =
                    (self.state == CallState::OutgoingRinging).then_some(CallSignalCode::Cancel);
                self.begin_ending(CallEndReason::RingTimeout, true, sig)
            }

            CallEvent::MediaWatchdogTimeout => {
                if !matches!(self.state, CallState::Connecting | CallState::Active) {
                    return Ok(Vec::new());
                }
                self.begin_ending(
                    CallEndReason::MediaTimeout,
                    true,
                    Some(CallSignalCode::EndCall),
                )
            }

            CallEvent::SetMuted { muted } => {
                if !matches!(self.state, CallState::Connecting | CallState::Active) {
                    return Ok(Vec::new());
                }
                self.is_muted = muted;
                Ok(vec![
                    CallEffect::MuteChanged(muted),
                    CallEffect::SendSignal(CallSignalCode::Conf),
                ])
            }

            CallEvent::SetHeld { held } => {
                if self.state != CallState::Active {
                    return Ok(Vec::new());
                }
                self.is_held = held;
                let sig = if held {
                    CallSignalCode::HoldReq
                } else {
                    CallSignalCode::UnholdReq
                };
                Ok(vec![
                    CallEffect::HoldChanged(held),
                    CallEffect::SendSignal(sig),
                ])
            }

            CallEvent::TeardownComplete => {
                if self.state != CallState::Ending {
                    return Ok(Vec::new());
                }
                let call_id = self.call_id.unwrap_or(0);
                let reason = self.end_reason.unwrap_or(CallEndReason::Normal);
                self.reset_to_idle();
                Ok(vec![
                    CallEffect::SendSignal(CallSignalCode::LogEndCall),
                    CallEffect::CallFinished { call_id, reason },
                    CallEffect::StateChanged(CallState::Idle),
                ])
            }
        }
    }

    fn begin_ending(
        &mut self,
        reason: CallEndReason,
        send_end_call_pkt: bool,
        signal: Option<CallSignalCode>,
    ) -> Result<Vec<CallEffect>> {
        self.state = CallState::Ending;
        self.end_reason = Some(reason);
        let mut effects = vec![CallEffect::StateChanged(CallState::Ending)];
        if let Some(sig) = signal {
            effects.push(CallEffect::SendSignal(sig));
        }
        effects.push(CallEffect::StopZrtpAndMedia { send_end_call_pkt });
        Ok(effects)
    }

    fn reset_to_idle(&mut self) {
        self.state = CallState::Idle;
        self.call_id = None;
        self.sess_id = None;
        self.peer_id = None;
        self.direction = None;
        self.srtp_mode = 1;
        self.is_muted = false;
        self.is_held = false;
        self.end_reason = None;
    }
}

// Note: `CallSignalCode::Conf` is used in SetMuted above; ensure it exists on CallSignalCode.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outgoing_call_lifecycle_reaches_active_and_returns_to_idle() {
        let mut s = CallSession::new();
        assert_eq!(s.state(), CallState::Idle);

        let eff = s
            .apply(CallEvent::StartOutgoing {
                call_id: 100,
                sess_id: "sess-100".to_string(),
                peer_id: "user-2".to_string(),
                srtp_mode: 1,
            })
            .unwrap();
        assert_eq!(s.state(), CallState::OutgoingRinging);
        assert!(!eff.contains(&CallEffect::StartZrtpAndMedia));

        let eff = s
            .apply(CallEvent::PeerAnswered {
                call_id: 100,
                accepted: true,
            })
            .unwrap();
        assert_eq!(s.state(), CallState::Connecting);
        assert!(eff.contains(&CallEffect::SendSignal(CallSignalCode::AnswerAck)));
        assert!(eff.contains(&CallEffect::StartZrtpAndMedia));

        let eff = s.apply(CallEvent::MediaReady { call_id: 100 }).unwrap();
        assert_eq!(s.state(), CallState::Active);
        assert_eq!(
            eff,
            vec![
                CallEffect::SendSignal(CallSignalCode::Request),
                CallEffect::StateChanged(CallState::Active),
            ]
        );

        let eff = s.apply(CallEvent::LocalEnd).unwrap();
        assert_eq!(s.state(), CallState::Ending);
        assert!(eff.contains(&CallEffect::SendSignal(CallSignalCode::EndCall)));
        assert!(eff.contains(&CallEffect::StopZrtpAndMedia {
            send_end_call_pkt: true
        }));

        let eff = s.apply(CallEvent::TeardownComplete).unwrap();
        assert_eq!(s.state(), CallState::Idle);
        assert!(eff.contains(&CallEffect::SendSignal(CallSignalCode::LogEndCall)));
        assert!(eff.contains(&CallEffect::CallFinished {
            call_id: 100,
            reason: CallEndReason::Normal
        }));
    }

    #[test]
    fn incoming_call_lifecycle_and_single_call_invariant() {
        let mut s = CallSession::new();
        let eff = s
            .apply(CallEvent::IncomingInvite {
                call_id: 200,
                sess_id: "sess-200".to_string(),
                peer_id: "user-1".to_string(),
                srtp_mode: 1,
            })
            .unwrap();
        assert_eq!(s.state(), CallState::IncomingRinging);
        assert!(eff.contains(&CallEffect::SendSignal(CallSignalCode::Request)));
        assert!(eff.contains(&CallEffect::SendSignal(CallSignalCode::RingRing)));

        // (ADV-1) Duplicate 401 invite for same callId is deduplicated.
        let dup = s
            .apply(CallEvent::IncomingInvite {
                call_id: 200,
                sess_id: "sess-200".to_string(),
                peer_id: "user-1".to_string(),
                srtp_mode: 1,
            })
            .unwrap();
        assert!(dup.is_empty());

        // (INV-7, ADV-2) Second 401 from different callId is rejected as busy while keeping call 200.
        let busy = s
            .apply(CallEvent::IncomingInvite {
                call_id: 201,
                sess_id: "sess-201".to_string(),
                peer_id: "user-3".to_string(),
                srtp_mode: 1,
            })
            .unwrap();
        assert_eq!(
            busy,
            vec![CallEffect::RejectBusy {
                call_id: 201,
                sess_id: "sess-201".to_string(),
                peer_id: "user-3".to_string(),
            }]
        );
        assert_eq!(s.call_id(), Some(200));

        // Accept -> Connecting -> Active.
        s.apply(CallEvent::AcceptIncoming { call_id: 200 }).unwrap();
        assert_eq!(s.state(), CallState::Connecting);
        s.apply(CallEvent::MediaReady { call_id: 200 }).unwrap();
        assert_eq!(s.state(), CallState::Active);

        // (ADV-4) Concurrent LocalEnd + PeerEnded executes teardown at most once.
        let first_end = s.apply(CallEvent::LocalEnd).unwrap();
        assert!(!first_end.is_empty());
        let second_end = s.apply(CallEvent::PeerEnded { call_id: 200 }).unwrap();
        assert!(second_end.is_empty());
        assert_eq!(s.state(), CallState::Ending);
    }
}

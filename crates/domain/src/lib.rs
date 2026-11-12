//! Pure domain types shared by the coordinator, worker protocol and UI contract.
//!
//! M0 only carries identity types and the session phase enumeration; the full
//! reducer (work package A1) lands in M1. Nothing here performs I/O.

pub mod chord;
pub mod session;

use std::fmt;

use serde::{Deserialize, Serialize};

/// Unguessable 128-bit session identifier, rendered as 32 lowercase hex chars.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct SessionId([u8; 16]);

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum IdError {
    #[error("session id must be 32 lowercase hex characters")]
    Malformed,
    #[error("OS random source unavailable")]
    Entropy,
}

impl SessionId {
    pub fn random() -> Result<Self, IdError> {
        let mut bytes = [0u8; 16];
        getrandom::fill(&mut bytes).map_err(|_| IdError::Entropy)?;
        Ok(Self(bytes))
    }

    pub fn from_bytes(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }
}

impl fmt::Display for SessionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for b in self.0 {
            write!(f, "{b:02x}")?;
        }
        Ok(())
    }
}

// Deliberately identical to Display: the id is not secret once issued, but the
// Debug form must stay short so it never tempts anyone to log larger state.
impl fmt::Debug for SessionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SessionId({self})")
    }
}

impl TryFrom<String> for SessionId {
    type Error = IdError;
    fn try_from(s: String) -> Result<Self, IdError> {
        s.parse()
    }
}

impl std::str::FromStr for SessionId {
    type Err = IdError;
    fn from_str(s: &str) -> Result<Self, IdError> {
        let raw = s.as_bytes();
        if raw.len() != 32 {
            return Err(IdError::Malformed);
        }
        let mut out = [0u8; 16];
        for (i, pair) in raw.chunks_exact(2).enumerate() {
            let hi = hex_val(pair[0]).ok_or(IdError::Malformed)?;
            let lo = hex_val(pair[1]).ok_or(IdError::Malformed)?;
            out[i] = (hi << 4) | lo;
        }
        Ok(Self(out))
    }
}

impl From<SessionId> for String {
    fn from(id: SessionId) -> String {
        id.to_string()
    }
}

fn hex_val(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        _ => None,
    }
}

/// Monotonic per-session generation; bumped on every restart of async work.
pub type Generation = u64;
/// Monotonic counter bumped every time the inference worker is (re)spawned.
pub type WorkerEpoch = u64;

/// Session phases from docs/04 §1. Readiness is tracked separately.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Phase {
    Idle,
    Starting,
    Listening,
    Finalizing,
    Transcribing,
    Delivering,
    DraftReady,
    Recovering,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_id_roundtrips_through_hex() {
        let id = SessionId::from_bytes([0xab; 16]);
        let s = id.to_string();
        assert_eq!(s, "ab".repeat(16));
        assert_eq!(s.parse::<SessionId>(), Ok(id));
    }

    #[test]
    fn session_id_rejects_bad_input() {
        for bad in [
            "",
            "AB".repeat(16).as_str(),
            &"0".repeat(31),
            &"g".repeat(32),
        ] {
            assert_eq!(bad.parse::<SessionId>(), Err(IdError::Malformed), "{bad:?}");
        }
    }

    #[test]
    fn random_ids_differ() {
        let a = SessionId::random();
        let b = SessionId::random();
        assert!(matches!((a, b), (Ok(a), Ok(b)) if a != b));
    }

    #[test]
    fn phase_serializes_like_the_ts_contract() {
        let json = serde_json::to_string(&Phase::DraftReady).ok();
        assert_eq!(json.as_deref(), Some("\"draftReady\""));
    }
}

//! Single-writer registry of native sessions.
//!
//! A Claude session id / Codex thread id must be driven by at most one process
//! of the app at a time: two writers fork or corrupt the session file. The
//! registry maps `(agent, native_id)` to the live chat or terminal that owns it.
//! It is shared by [`super::LiveManager`] and [`crate::pty::PtyManager`].
//! External terminals are not tracked.
use std::collections::HashMap;
use std::sync::Mutex;

use serde::Serialize;

use super::Agent;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OwnerKind {
    Live,
    Pty,
}

/// `{kind: "live" | "pty", id}`: the live id or the pty id.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct Owner {
    pub kind: OwnerKind,
    pub id: String,
}

impl Owner {
    pub fn live(id: &str) -> Self {
        Self { kind: OwnerKind::Live, id: id.to_string() }
    }
    pub fn pty(id: &str) -> Self {
        Self { kind: OwnerKind::Pty, id: id.to_string() }
    }
}

#[derive(Default)]
pub struct NativeRegistry {
    map: Mutex<HashMap<(Agent, String), Owner>>,
}

impl NativeRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn owner(&self, agent: Agent, native_id: &str) -> Option<Owner> {
        self.map.lock().unwrap().get(&(agent, native_id.to_string())).cloned()
    }

    /// Takes `native_id` for `owner`. `Err(current)` when someone else owns it
    /// (claiming again for the same owner is fine).
    pub fn claim(&self, agent: Agent, native_id: &str, owner: Owner) -> Result<(), Owner> {
        let mut map = self.map.lock().unwrap();
        let key = (agent, native_id.to_string());
        match map.get(&key) {
            Some(cur) if *cur != owner => Err(cur.clone()),
            _ => {
                map.insert(key, owner);
                Ok(())
            }
        }
    }

    /// Takes `native_id` for `owner` even if a (stale) owner holds it.
    pub fn replace(&self, agent: Agent, native_id: &str, owner: Owner) {
        self.map.lock().unwrap().insert((agent, native_id.to_string()), owner);
    }

    /// Forgets every id held by `owner`.
    pub fn release_owner(&self, owner: &Owner) {
        self.map.lock().unwrap().retain(|_, o| o != owner);
    }
}

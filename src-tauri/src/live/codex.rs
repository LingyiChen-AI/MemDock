//! Codex driver for live chat. See the module docs of `live` for the protocol.
use super::{Driver, Emit, StartOptions};

/// Launches the CLI and returns its driver; the reader threads report through `emit`.
pub fn spawn(_opts: &StartOptions, _emit: Emit) -> Result<Box<dyn Driver>, String> {
    Err("Codex 实时对话尚未实现".into())
}

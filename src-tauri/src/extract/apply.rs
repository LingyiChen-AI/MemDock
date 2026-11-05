//! Applies a reviewed suggestion to the real memory file (accept /
//! edit-then-accept / reject / accept many). This is the only code path that
//! writes memory files from suggestions.
//!
//! Safety rules: the store lock is held only for short reads/writes, never
//! across file IO; the whole apply runs under `apply_lock` (injected, shared
//! with the memory editor's write lock) so writes to the same file are
//! serialized; writes go through `write_memory` (hash check + snapshot +
//! BOM/CRLF preservation); a missing target file is only created when its
//! parent directory already exists; applying is idempotent (a block that is
//! already present is never added twice).

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use serde::Serialize;

use super::prompt::{Suggestion, SuggestionKind, SuggestionStatus};
use super::store::Store;
use crate::memory::io::{read_memory, write_memory, WriteError};
use crate::memory::snapshot::SnapshotStore;

pub struct ApplyCtx {
    pub store: Arc<Mutex<Store>>,
    pub snapshots: SnapshotStore,
    /// Held for the whole apply / reject. Wire the same lock the memory
    /// editor's saves use.
    pub apply_lock: Arc<Mutex<()>>,
}

impl ApplyCtx {
    pub fn new(store: Arc<Mutex<Store>>, snapshots: SnapshotStore, apply_lock: Arc<Mutex<()>>) -> Self {
        ApplyCtx { store, snapshots, apply_lock }
    }
}

#[derive(Serialize, Clone, PartialEq, Eq, Debug)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ApplyOutcome {
    /// File written. `saved == false`: the status could not be persisted
    /// ("written, but status save failed").
    Applied { hash: String, saved: bool },
    NeedsConfirm { reason: String },
    Stale,
    /// The change is already present in the target; nothing written and the
    /// suggestion is marked Stale.
    AlreadyApplied,
}

#[derive(Serialize, Clone, PartialEq, Eq, Debug)]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum ApplyError {
    NotFound,
    NotPending,
    Invalid { message: String },
    Io { message: String },
}

fn invalid(m: &str) -> ApplyError {
    ApplyError::Invalid { message: m.to_string() }
}

fn lf(s: &str) -> String {
    s.replace("\r\n", "\n")
}

fn lock_store(ctx: &ApplyCtx) -> MutexGuard<'_, Store> {
    ctx.store.lock().unwrap_or_else(|e| e.into_inner())
}

fn norm_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Moves a still-Pending suggestion to `status` and saves. `Ok(false)` when it
/// was no longer Pending; `Err` when the save failed (status stays set in
/// memory; callers decide whether to revert).
fn transition(ctx: &ApplyCtx, id: &str, status: SuggestionStatus) -> Result<bool, String> {
    let mut st = lock_store(ctx);
    let pending = st
        .suggestions()
        .iter()
        .find(|s| s.id == id)
        .is_some_and(|s| s.status == SuggestionStatus::Pending);
    if !pending {
        return Ok(false);
    }
    st.set_status(id, status);
    st.save().map(|_| true).map_err(|e| e.to_string())
}

/// Like `transition` but a failed save is reverted and reported as an error.
fn transition_strict(ctx: &ApplyCtx, id: &str, status: SuggestionStatus) -> Result<(), ApplyError> {
    match transition(ctx, id, status) {
        Ok(true) => Ok(()),
        Ok(false) => Err(ApplyError::NotPending),
        Err(message) => {
            lock_store(ctx).set_status(id, SuggestionStatus::Pending);
            Err(ApplyError::Io { message })
        }
    }
}

/// Appends `block` after `text`: exactly one blank line in between, and the
/// result ends with a single newline.
fn append_block(text: &str, block: &str) -> String {
    let base = text.trim_end_matches('\n');
    let block = block.trim_end_matches('\n');
    if base.is_empty() {
        format!("{block}\n")
    } else {
        format!("{base}\n\n{block}\n")
    }
}

/// Removes `old` (at byte offset `pos`) plus one following blank line.
fn remove_block(text: &str, pos: usize, old: &str) -> String {
    let end = pos + old.len();
    let rest = &text[end..];
    let skip = if old.ends_with('\n') {
        usize::from(rest.starts_with('\n'))
    } else if rest.starts_with("\n\n") {
        2
    } else if rest.starts_with('\n') {
        1
    } else {
        0
    };
    let tail = &rest[skip..];
    let mut out = String::with_capacity(text.len());
    out.push_str(&text[..pos]);
    out.push_str(tail);
    if tail.is_empty() {
        // Removed at end of file: don't leave a dangling blank line.
        while out.ends_with("\n\n") {
            out.pop();
        }
    }
    out
}

/// Position of `old` when it occurs exactly once (overlapping matches count).
fn unique_pos(text: &str, old: &str) -> Option<usize> {
    let first = text.find(old)?;
    let step = text[first..].chars().next().map_or(1, char::len_utf8);
    if text[first + step..].contains(old) {
        None
    } else {
        Some(first)
    }
}

/// Makes the replacement's trailing newline match `old`'s.
fn align_newline(old: &str, r: &str) -> String {
    if old.ends_with('\n') {
        if r.ends_with('\n') { r.to_string() } else { format!("{r}\n") }
    } else {
        r.trim_end_matches('\n').to_string()
    }
}

pub fn apply(
    ctx: &ApplyCtx,
    id: &str,
    edited_new_text: Option<String>,
    confirm_changed: bool,
) -> Result<ApplyOutcome, ApplyError> {
    apply_inner(ctx, id, edited_new_text, confirm_changed, &|| {})
}

/// `before_write` runs right before the write (test hook).
fn apply_inner(
    ctx: &ApplyCtx,
    id: &str,
    edited_new_text: Option<String>,
    confirm_changed: bool,
    before_write: &dyn Fn(),
) -> Result<ApplyOutcome, ApplyError> {
    let _guard = ctx.apply_lock.lock().unwrap_or_else(|e| e.into_inner());
    let s: Suggestion = {
        let st = lock_store(ctx);
        st.suggestions().iter().find(|s| s.id == id).cloned().ok_or(ApplyError::NotFound)?
    };
    if s.status != SuggestionStatus::Pending {
        return Err(ApplyError::NotPending);
    }
    let path: PathBuf = PathBuf::from(&s.path);
    let cur = read_memory(&path).map_err(|e| ApplyError::Io { message: e.to_string() })?;
    let exists = !cur.hash.is_empty();
    let changed = cur.hash != s.base_hash;
    let new_text = edited_new_text.or_else(|| s.new_text.clone()).map(|t| lf(&t));
    let target_changed = || Ok(ApplyOutcome::NeedsConfirm { reason: "target_changed".into() });
    let already = |ctx: &ApplyCtx| -> Result<ApplyOutcome, ApplyError> {
        transition_strict(ctx, id, SuggestionStatus::Stale)?;
        Ok(ApplyOutcome::AlreadyApplied)
    };

    let result_text = match s.kind {
        SuggestionKind::Add => {
            let block = new_text.filter(|t| !t.trim().is_empty()).ok_or_else(|| invalid("new_text required"))?;
            if !exists && !path.parent().is_some_and(Path::is_dir) {
                return Err(invalid("parent directory not found"));
            }
            if exists && norm_ws(&cur.text).contains(&norm_ws(&block)) {
                return already(ctx);
            }
            if changed && !confirm_changed {
                return target_changed();
            }
            append_block(&cur.text, &block)
        }
        SuggestionKind::Modify | SuggestionKind::Obsolete => {
            let old = s.old_text.as_deref().map(lf).filter(|t| !t.is_empty());
            let old = old.ok_or_else(|| invalid("old_text required"))?;
            let replacement = match (s.kind, new_text) {
                (SuggestionKind::Modify, Some(t)) if !t.trim().is_empty() => Some(t),
                (SuggestionKind::Modify, _) => return Err(invalid("new_text required")),
                (_, Some(t)) if !t.trim().is_empty() => Some(t),
                _ => None,
            };
            let pos = unique_pos(&cur.text, &old);
            if changed {
                if let Some(r) = &replacement {
                    let (nr, no) = (norm_ws(r), norm_ws(&old));
                    if norm_ws(&cur.text).contains(&nr) && (!cur.text.contains(&old) || nr.contains(&no)) {
                        return already(ctx);
                    }
                }
            }
            let Some(pos) = pos else {
                transition_strict(ctx, id, SuggestionStatus::Stale)?;
                return Ok(ApplyOutcome::Stale);
            };
            if changed && !confirm_changed {
                return target_changed();
            }
            match replacement {
                Some(r) => {
                    let r = align_newline(&old, &r);
                    format!("{}{}{}", &cur.text[..pos], r, &cur.text[pos + old.len()..])
                }
                None => remove_block(&cur.text, pos, &old),
            }
        }
    };

    // Still Pending right before writing (a reject may have raced in).
    let still_pending = lock_store(ctx)
        .suggestions()
        .iter()
        .find(|x| x.id == id)
        .is_some_and(|x| x.status == SuggestionStatus::Pending);
    if !still_pending {
        return Err(ApplyError::NotPending);
    }
    before_write();
    match write_memory(&path, &result_text, &cur.hash, &ctx.snapshots) {
        Ok(fc) => {
            let saved = matches!(transition(ctx, id, SuggestionStatus::Accepted), Ok(true));
            Ok(ApplyOutcome::Applied { hash: fc.hash, saved })
        }
        Err(WriteError::DiskChanged { .. }) => Ok(ApplyOutcome::NeedsConfirm { reason: "disk_changed".into() }),
        Err(WriteError::Io(e)) => Err(ApplyError::Io { message: e.to_string() }),
    }
}

pub fn reject(ctx: &ApplyCtx, id: &str) -> Result<(), ApplyError> {
    let _guard = ctx.apply_lock.lock().unwrap_or_else(|e| e.into_inner());
    if !lock_store(ctx).suggestions().iter().any(|s| s.id == id) {
        return Err(ApplyError::NotFound);
    }
    transition_strict(ctx, id, SuggestionStatus::Rejected)
}

/// Applies in order; continues after failures; `NeedsConfirm` items are not
/// applied and are reported as such.
pub fn accept_many(ctx: &ApplyCtx, ids: &[String]) -> Vec<(String, Result<ApplyOutcome, ApplyError>)> {
    ids.iter().map(|id| (id.clone(), apply(ctx, id, None, false))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Tool;
    use std::fs;

    struct Env {
        _dir: tempfile::TempDir,
        root: PathBuf,
        ctx: ApplyCtx,
    }

    fn env() -> Env {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        fs::create_dir_all(root.join("data")).unwrap();
        fs::create_dir_all(root.join("proj")).unwrap();
        let store = Store::load(&root.join("data")).unwrap();
        let ctx = ApplyCtx::new(
            Arc::new(Mutex::new(store)),
            SnapshotStore::new(root.join("snaps")),
            Arc::new(Mutex::new(())),
        );
        Env { _dir: dir, root, ctx }
    }

    fn put(e: &Env, id: &str, kind: SuggestionKind, path: &Path, old: Option<&str>, new: Option<&str>) {
        let base = read_memory(path).unwrap().hash;
        let s = Suggestion {
            id: id.into(),
            fingerprint: format!("fp-{id}"),
            kind,
            file_id: "f".into(),
            path: path.to_string_lossy().into_owned(),
            tool_of_file: Tool::Claude,
            old_text: old.map(Into::into),
            new_text: new.map(Into::into),
            reason: "r".into(),
            evidence: vec![],
            source_tool: Tool::Claude,
            created_at: 1,
            base_hash: base,
            status: SuggestionStatus::Pending,
        };
        e.ctx.store.lock().unwrap().add_suggestions(vec![s]);
    }

    fn status(e: &Env, id: &str) -> SuggestionStatus {
        e.ctx.store.lock().unwrap().suggestions().iter().find(|s| s.id == id).unwrap().status
    }

    #[test]
    fn apply_modify_replaces_unique_old_text() {
        let e = env();
        let p = e.root.join("proj/CLAUDE.md");
        fs::write(&p, "# T\n\nuse npm\n\nend\n").unwrap();
        put(&e, "a", SuggestionKind::Modify, &p, Some("use npm"), Some("use pnpm"));
        let r = apply(&e.ctx, "a", None, false).unwrap();
        assert!(matches!(r, ApplyOutcome::Applied { .. }));
        assert_eq!(fs::read_to_string(&p).unwrap(), "# T\n\nuse pnpm\n\nend\n");
        assert_eq!(status(&e, "a"), SuggestionStatus::Accepted);
        assert_eq!(e.ctx.snapshots.list(&p).unwrap().len(), 1);
    }

    #[test]
    fn apply_obsolete_removes_block() {
        let e = env();
        let p = e.root.join("proj/CLAUDE.md");
        fs::write(&p, "A\n\nold rule\n\nB\n").unwrap();
        put(&e, "a", SuggestionKind::Obsolete, &p, Some("old rule"), None);
        apply(&e.ctx, "a", None, false).unwrap();
        assert_eq!(fs::read_to_string(&p).unwrap(), "A\n\nB\n");
        // obsolete with replacement text replaces instead
        fs::write(&p, "A\n\nold rule\n\nB\n").unwrap();
        put(&e, "b", SuggestionKind::Obsolete, &p, Some("old rule"), Some("new rule"));
        apply(&e.ctx, "b", None, false).unwrap();
        assert_eq!(fs::read_to_string(&p).unwrap(), "A\n\nnew rule\n\nB\n");
        // removal at end of file leaves no dangling blank line
        fs::write(&p, "A\n\nlast\n").unwrap();
        put(&e, "c", SuggestionKind::Obsolete, &p, Some("last"), None);
        apply(&e.ctx, "c", None, false).unwrap();
        assert_eq!(fs::read_to_string(&p).unwrap(), "A\n\n".trim_end().to_string() + "\n");
    }

    #[test]
    fn apply_add_appends_with_blank_line() {
        let e = env();
        let p = e.root.join("proj/CLAUDE.md");
        fs::write(&p, "A\n").unwrap();
        put(&e, "a", SuggestionKind::Add, &p, None, Some("new line"));
        apply(&e.ctx, "a", None, false).unwrap();
        assert_eq!(fs::read_to_string(&p).unwrap(), "A\n\nnew line\n");
        fs::write(&p, "A").unwrap();
        put(&e, "b", SuggestionKind::Add, &p, None, Some("x\n\n"));
        apply(&e.ctx, "b", None, false).unwrap();
        assert_eq!(fs::read_to_string(&p).unwrap(), "A\n\nx\n");
    }

    #[test]
    fn apply_detects_changed_target() {
        let e = env();
        let p = e.root.join("proj/CLAUDE.md");
        fs::write(&p, "A\n\nuse npm\n").unwrap();
        put(&e, "a", SuggestionKind::Modify, &p, Some("use npm"), Some("use pnpm"));
        put(&e, "b", SuggestionKind::Add, &p, None, Some("added"));
        fs::write(&p, "A\n\nuse npm\n\nextra\n").unwrap();
        // unique old_text still there -> NeedsConfirm, nothing written
        let r = apply(&e.ctx, "a", None, false).unwrap();
        assert!(matches!(r, ApplyOutcome::NeedsConfirm { .. }));
        assert_eq!(fs::read_to_string(&p).unwrap(), "A\n\nuse npm\n\nextra\n");
        assert_eq!(status(&e, "a"), SuggestionStatus::Pending);
        // add with changed hash -> NeedsConfirm too
        assert!(matches!(apply(&e.ctx, "b", None, false).unwrap(), ApplyOutcome::NeedsConfirm { .. }));
        // confirm -> applied
        assert!(matches!(apply(&e.ctx, "a", None, true).unwrap(), ApplyOutcome::Applied { .. }));
        assert_eq!(fs::read_to_string(&p).unwrap(), "A\n\nuse pnpm\n\nextra\n");
        // old_text gone -> Stale
        put(&e, "c", SuggestionKind::Modify, &p, Some("extra"), Some("more"));
        fs::write(&p, "nothing here\n").unwrap();
        assert_eq!(apply(&e.ctx, "c", None, true).unwrap(), ApplyOutcome::Stale);
        assert_eq!(status(&e, "c"), SuggestionStatus::Stale);
        // old_text twice -> Stale
        put(&e, "d", SuggestionKind::Modify, &p, Some("x"), Some("y"));
        fs::write(&p, "x and x\n").unwrap();
        assert_eq!(apply(&e.ctx, "d", None, true).unwrap(), ApplyOutcome::Stale);
        assert_eq!(fs::read_to_string(&p).unwrap(), "x and x\n");
    }

    #[test]
    fn apply_two_suggestions_on_same_file_in_sequence() {
        let e = env();
        let p = e.root.join("proj/CLAUDE.md");
        fs::write(&p, "one\n\ntwo\n").unwrap();
        put(&e, "a", SuggestionKind::Modify, &p, Some("one"), Some("1"));
        put(&e, "b", SuggestionKind::Modify, &p, Some("two"), Some("2"));
        assert!(matches!(apply(&e.ctx, "a", None, false).unwrap(), ApplyOutcome::Applied { .. }));
        assert!(matches!(apply(&e.ctx, "b", None, false).unwrap(), ApplyOutcome::NeedsConfirm { .. }));
        assert!(matches!(apply(&e.ctx, "b", None, true).unwrap(), ApplyOutcome::Applied { .. }));
        assert_eq!(fs::read_to_string(&p).unwrap(), "1\n\n2\n");
    }

    #[test]
    fn apply_with_edited_text() {
        let e = env();
        let p = e.root.join("proj/CLAUDE.md");
        fs::write(&p, "use npm\n").unwrap();
        put(&e, "a", SuggestionKind::Modify, &p, Some("use npm"), Some("use pnpm"));
        apply(&e.ctx, "a", Some("use yarn".into()), false).unwrap();
        assert_eq!(fs::read_to_string(&p).unwrap(), "use yarn\n");
    }

    #[test]
    fn apply_creates_missing_file_only_if_parent_exists() {
        let e = env();
        let p = e.root.join("proj/CLAUDE.md");
        put(&e, "a", SuggestionKind::Add, &p, None, Some("hello"));
        assert!(matches!(apply(&e.ctx, "a", None, false).unwrap(), ApplyOutcome::Applied { .. }));
        assert_eq!(fs::read_to_string(&p).unwrap(), "hello\n");

        let missing = e.root.join("nodir/CLAUDE.md");
        put(&e, "b", SuggestionKind::Add, &missing, None, Some("hello"));
        let r = apply(&e.ctx, "b", None, false);
        assert!(matches!(r, Err(ApplyError::Invalid { ref message }) if message == "parent directory not found"));
        assert!(!e.root.join("nodir").exists());
        assert_eq!(status(&e, "b"), SuggestionStatus::Pending);
    }

    #[test]
    fn reject_sets_status() {
        let e = env();
        let p = e.root.join("proj/CLAUDE.md");
        put(&e, "a", SuggestionKind::Add, &p, None, Some("x"));
        reject(&e.ctx, "a").unwrap();
        assert_eq!(status(&e, "a"), SuggestionStatus::Rejected);
        assert_eq!(reject(&e.ctx, "a"), Err(ApplyError::NotPending));
        assert_eq!(reject(&e.ctx, "zz"), Err(ApplyError::NotFound));
        assert_eq!(apply(&e.ctx, "a", None, false), Err(ApplyError::NotPending));
        assert!(!p.exists());
    }

    #[test]
    fn accept_many_continues_after_failure() {
        let e = env();
        let p = e.root.join("proj/CLAUDE.md");
        let q = e.root.join("proj/AGENTS.md");
        fs::write(&p, "a\n").unwrap();
        fs::write(&q, "q\n").unwrap();
        put(&e, "ok1", SuggestionKind::Modify, &p, Some("a"), Some("b"));
        put(&e, "gone", SuggestionKind::Modify, &q, Some("missing"), Some("z"));
        put(&e, "ok2", SuggestionKind::Add, &q, None, Some("n"));
        // ok2 is based on q's hash, which is unchanged since "gone" is Stale (no write)
        let ids: Vec<String> = ["ok1", "gone", "nope", "ok2"].iter().map(|s| s.to_string()).collect();
        let r = accept_many(&e.ctx, &ids);
        assert_eq!(r.len(), 4);
        assert!(matches!(r[0].1, Ok(ApplyOutcome::Applied { .. })));
        assert_eq!(r[1].1, Ok(ApplyOutcome::Stale));
        assert_eq!(r[2].1, Err(ApplyError::NotFound));
        assert!(matches!(r[3].1, Ok(ApplyOutcome::Applied { .. })));
        assert_eq!(fs::read_to_string(&q).unwrap(), "q\n\nn\n");
    }

    #[test]
    fn apply_preserves_bom_and_crlf() {
        let e = env();
        let p = e.root.join("proj/CLAUDE.md");
        fs::write(&p, b"\xEF\xBB\xBFone\r\n\r\ntwo\r\n").unwrap();
        put(&e, "a", SuggestionKind::Modify, &p, Some("two"), Some("2\nmore"));
        put(&e, "b", SuggestionKind::Add, &p, None, Some("tail"));
        apply(&e.ctx, "a", None, false).unwrap();
        assert_eq!(fs::read(&p).unwrap(), b"\xEF\xBB\xBFone\r\n\r\n2\r\nmore\r\n");
        apply(&e.ctx, "b", None, true).unwrap();
        assert_eq!(fs::read(&p).unwrap(), b"\xEF\xBB\xBFone\r\n\r\n2\r\nmore\r\n\r\ntail\r\n");
    }

    fn disk(p: &Path) -> String {
        fs::read_to_string(p).unwrap()
    }

    #[test]
    fn status_save_failure_is_reported() {
        let e = env();
        let p = e.root.join("proj/CLAUDE.md");
        fs::write(&p, "one\n\ntwo\n").unwrap();
        put(&e, "a", SuggestionKind::Modify, &p, Some("one"), Some("1"));
        put(&e, "r", SuggestionKind::Add, &p, None, Some("zzz"));
        put(&e, "s", SuggestionKind::Modify, &p, Some("missing"), Some("m"));
        // Break the store directory so every save fails.
        fs::remove_dir_all(e.root.join("data")).unwrap();
        fs::write(e.root.join("data"), "not a dir").unwrap();
        // file written, status not persisted -> saved:false
        let r = apply(&e.ctx, "a", None, false).unwrap();
        assert!(matches!(r, ApplyOutcome::Applied { saved: false, .. }));
        assert_eq!(disk(&p), "1\n\ntwo\n");
        // reject / stale marking surface the error and revert to Pending
        assert!(matches!(reject(&e.ctx, "r"), Err(ApplyError::Io { .. })));
        assert_eq!(status(&e, "r"), SuggestionStatus::Pending);
        assert!(matches!(apply(&e.ctx, "s", None, true), Err(ApplyError::Io { .. })));
        assert_eq!(status(&e, "s"), SuggestionStatus::Pending);
    }

    #[test]
    fn add_is_idempotent() {
        let e = env();
        let p = e.root.join("proj/CLAUDE.md");
        fs::write(&p, "A\n\nnew   line\n").unwrap();
        put(&e, "a", SuggestionKind::Add, &p, None, Some("new line"));
        assert_eq!(apply(&e.ctx, "a", None, true).unwrap(), ApplyOutcome::AlreadyApplied);
        assert_eq!(disk(&p), "A\n\nnew   line\n");
        assert_eq!(status(&e, "a"), SuggestionStatus::Stale);
    }

    #[test]
    fn modify_already_applied_is_detected() {
        let e = env();
        let p = e.root.join("proj/CLAUDE.md");
        fs::write(&p, "use npm\n").unwrap();
        put(&e, "a", SuggestionKind::Modify, &p, Some("use npm"), Some("use pnpm"));
        put(&e, "b", SuggestionKind::Modify, &p, Some("use npm"), Some("use npm always"));
        fs::write(&p, "use pnpm\n\nextra\n").unwrap();
        assert_eq!(apply(&e.ctx, "a", None, true).unwrap(), ApplyOutcome::AlreadyApplied);
        // new_text contains old_text and is already there: must not double up
        fs::write(&p, "use npm always\n\nextra\n").unwrap();
        assert_eq!(apply(&e.ctx, "b", None, true).unwrap(), ApplyOutcome::AlreadyApplied);
        assert_eq!(disk(&p), "use npm always\n\nextra\n");
    }

    #[test]
    fn modify_with_blank_new_text_is_invalid() {
        let e = env();
        let p = e.root.join("proj/CLAUDE.md");
        fs::write(&p, "use npm\n").unwrap();
        put(&e, "a", SuggestionKind::Modify, &p, Some("use npm"), Some("x"));
        for t in ["", "  \n"] {
            let r = apply(&e.ctx, "a", Some(t.into()), false);
            assert!(matches!(r, Err(ApplyError::Invalid { ref message }) if message == "new_text required"));
        }
        assert_eq!(disk(&p), "use npm\n");
    }

    #[test]
    fn replacement_trailing_newline_follows_old_text() {
        let e = env();
        let p = e.root.join("proj/CLAUDE.md");
        fs::write(&p, "foo\nz\n").unwrap();
        put(&e, "a", SuggestionKind::Modify, &p, Some("foo\n"), Some("bar"));
        apply(&e.ctx, "a", None, false).unwrap();
        assert_eq!(disk(&p), "bar\nz\n");
        fs::write(&p, "foo\nz\n").unwrap();
        put(&e, "b", SuggestionKind::Modify, &p, Some("foo"), Some("bar\n"));
        apply(&e.ctx, "b", None, false).unwrap();
        assert_eq!(disk(&p), "bar\nz\n");
    }

    #[test]
    fn overlapping_matches_are_not_unique() {
        let e = env();
        let p = e.root.join("proj/CLAUDE.md");
        fs::write(&p, "aaa\n").unwrap();
        put(&e, "a", SuggestionKind::Modify, &p, Some("aa"), Some("b"));
        assert_eq!(apply(&e.ctx, "a", None, false).unwrap(), ApplyOutcome::Stale);
        assert_eq!(disk(&p), "aaa\n");
    }

    #[test]
    fn disk_change_between_read_and_write_needs_confirm() {
        let e = env();
        let p = e.root.join("proj/CLAUDE.md");
        fs::write(&p, "use npm\n").unwrap();
        put(&e, "a", SuggestionKind::Modify, &p, Some("use npm"), Some("use pnpm"));
        let r = apply_inner(&e.ctx, "a", None, false, &|| fs::write(&p, "edited elsewhere\n").unwrap()).unwrap();
        assert_eq!(r, ApplyOutcome::NeedsConfirm { reason: "disk_changed".into() });
        assert_eq!(disk(&p), "edited elsewhere\n");
        assert_eq!(status(&e, "a"), SuggestionStatus::Pending);
    }

    #[test]
    fn statuses_persist_across_store_load() {
        let e = env();
        let p = e.root.join("proj/CLAUDE.md");
        fs::write(&p, "one\n").unwrap();
        put(&e, "ok", SuggestionKind::Modify, &p, Some("one"), Some("1"));
        put(&e, "st", SuggestionKind::Modify, &p, Some("nope"), Some("2"));
        put(&e, "rj", SuggestionKind::Add, &p, None, Some("x"));
        apply(&e.ctx, "ok", None, false).unwrap();
        apply(&e.ctx, "st", None, true).unwrap();
        reject(&e.ctx, "rj").unwrap();
        let reloaded = Store::load(&e.root.join("data")).unwrap();
        let get = |id: &str| reloaded.suggestions().iter().find(|s| s.id == id).unwrap().status;
        assert_eq!(get("ok"), SuggestionStatus::Accepted);
        assert_eq!(get("st"), SuggestionStatus::Stale);
        assert_eq!(get("rj"), SuggestionStatus::Rejected);
    }

    #[test]
    fn concurrent_applies_on_same_file_are_serialized() {
        for _ in 0..10 {
            let e = env();
            let p = e.root.join("proj/CLAUDE.md");
            fs::write(&p, "one\n\ntwo\n").unwrap();
            put(&e, "a", SuggestionKind::Modify, &p, Some("one"), Some("1"));
            put(&e, "b", SuggestionKind::Modify, &p, Some("two"), Some("2"));
            std::thread::scope(|sc| {
                let ha = sc.spawn(|| apply(&e.ctx, "a", None, true));
                let hb = sc.spawn(|| apply(&e.ctx, "b", None, true));
                assert!(matches!(ha.join().unwrap().unwrap(), ApplyOutcome::Applied { .. }));
                assert!(matches!(hb.join().unwrap().unwrap(), ApplyOutcome::Applied { .. }));
            });
            assert_eq!(disk(&p), "1\n\n2\n");
        }
        // without confirmation exactly one wins, the other asks
        let e = env();
        let p = e.root.join("proj/CLAUDE.md");
        fs::write(&p, "one\n\ntwo\n").unwrap();
        put(&e, "a", SuggestionKind::Modify, &p, Some("one"), Some("1"));
        put(&e, "b", SuggestionKind::Modify, &p, Some("two"), Some("2"));
        let (ra, rb) = std::thread::scope(|sc| {
            let ha = sc.spawn(|| apply(&e.ctx, "a", None, false).unwrap());
            let hb = sc.spawn(|| apply(&e.ctx, "b", None, false).unwrap());
            (ha.join().unwrap(), hb.join().unwrap())
        });
        let applied = [&ra, &rb].iter().filter(|r| matches!(r, ApplyOutcome::Applied { .. })).count();
        let asked = [&ra, &rb].iter().filter(|r| matches!(r, ApplyOutcome::NeedsConfirm { .. })).count();
        assert_eq!((applied, asked), (1, 1));
    }

    #[test]
    fn reject_racing_apply_never_leaves_inconsistent_state() {
        for _ in 0..20 {
            let e = env();
            let p = e.root.join("proj/CLAUDE.md");
            fs::write(&p, "one\n").unwrap();
            put(&e, "a", SuggestionKind::Modify, &p, Some("one"), Some("1"));
            std::thread::scope(|sc| {
                let ha = sc.spawn(|| apply(&e.ctx, "a", None, false));
                let hr = sc.spawn(|| reject(&e.ctx, "a"));
                let _ = (ha.join().unwrap(), hr.join().unwrap());
            });
            match status(&e, "a") {
                SuggestionStatus::Accepted => assert_eq!(disk(&p), "1\n"),
                SuggestionStatus::Rejected => assert_eq!(disk(&p), "one\n"),
                other => panic!("unexpected status {other:?}"),
            }
        }
    }
}

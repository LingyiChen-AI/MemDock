# Multi-session isolation (fixes 1–5)

Base: `feature/live-chat` at b4f8d52. Backend agent: Rust only. Frontend agent: TS only. Contract below.

## Audit findings (file:line at b4f8d52)
- P0-1 No single-writer check per native session: `live/mod.rs` `start()` and `pty/mod.rs` `pty_open` never check whether the same Claude session id / Codex thread is already driven by another process. Two writers corrupt/fork the session jsonl / rollout.
- P0-2 All draft terminals share key `live:new` (`src/pages/chat/terminalStore.ts` `terminalKey`), so a new draft re-attaches another draft's terminal.
- P1-3 Events are published outside the lock (`live/mod.rs` `record()`): seq assigned under lock, `publish` after unlock; several threads emit for one session (reader thread + send/respond/stop) → out-of-order delivery; frontend drops `seq <= lastSeq` (`src/pages/chat/liveModel.ts:80`) and never detects gaps → lost events.
- P1-4 `ApprovalResolved` / `UserMessage` recorded after the stdin write (`live/mod.rs` respond/send) → a fast `TurnCompleted` can be overtaken, leaving state stuck at Running.
- P1-5 One global `sessions` lock held during driver I/O (`with_driver`), incl. writing images to disk and multi-MB stdin writes → one session blocks all.
- P2-6 `liveByNative` (SessionsPage) shows only the newest process per native id and hides older still-running ones.
- P2-7 `mergeList` (liveStore) drops convs/selection when a stale list arrives right after `startLive`.
- P2-8 Composer text/images live in component state and are lost on switch; model/permission prefs are global.
- P2-9 Scroll position not kept per conversation.
- P2-10 Event logs unbounded; every delta stored; images stored as base64 in `UserMessage`; `transcript()` clones everything.
- P2-11 Every token replaces the whole live store → the whole chat page re-renders for background sessions.

## Contract

### Backend
1. **Native-id registry** (new, shared by live + pty): key `(agent, native_id)` → owner `Live(live_id)` | `Pty(pty_id)`.
   - `live_start`: if `resume_id` is owned by a running live session → return that session's `LiveSessionInfo` (no new process; if `prompt`/`prompt_images` are given, send them to it). If owned by a PTY → error code-prefixed string `"busy_pty:<pty_id>"`.
   - `pty_open` (tool claude/codex with `resume_id`): if owned by a live session → error `"busy_live:<live_id>"`; if owned by another PTY → return that `PtyOpened` (re-attach).
   - Register when the native id becomes known (Claude: immediately via `--session-id`/resume id; Codex: on `Started`), release on exit/close.
   - New command `native_owner(agent, native_id) -> Option<Owner>` where `Owner = {kind: "live"|"pty", id: String}`.
   - Best effort against external terminals is out of scope.
2. **Per-session ordering**: `Entry` behind its own lock (`Arc<Mutex<Entry>>`, the map lock only for lookup); seq assignment + publish happen under the session lock (or through a per-session channel drained by one thread) so delivery order == seq order. No lock held during driver stdin I/O or image saving.
3. **State**: record `UserMessage` / `ApprovalResolved` *before* writing to the driver (roll back with an `Error` event if the write fails); `ApprovalResolved` no longer moves state out of Idle/Exited (only from WaitingApproval → Running).
4. **Bounded logs**: on `TurnCompleted`, drop the `AssistantDelta` / `ToolOutput` events of that turn whose message/tool has a final `AssistantMessage` / `ToolFinished` (keep seq numbers of what remains; the log may have gaps — transcript returns envelopes with their original seq). Cap the log at 5000 events / ~8 MB (drop oldest, but keep a leading `Status`/`Started` snapshot). `UserMessage.images` stored and sent as small `file:` paths? → No: keep data URIs in the live event published to the UI, but in the stored log replace each image with a temp file path string prefixed `memdock-image:` + absolute path, and add command `read_image_data_uri` (exists) for the UI to load it.
5. **Stop / idle**: keep `live_stop` / `live_close`; add an idle reaper: a live process idle (state Idle) for 30 min with no UI attached is stopped (its session stays resumable). Configurable constant.

### Frontend
1. Draft identity: drafts get `draft:<uuid>` keys (terminalStore + composer state); when the draft's live session / pty gets a native id, migrate the key (no new process).
2. Before opening a terminal for a conversation, ask `native_owner`; if a live GUI process owns it, stop it (confirm if running) then open; if `pty_open` returns `busy_live:` show a clear message. Before `startLive` with `resume_id`, if a PTY owns it, offer to switch to the terminal or close it first.
3. Gap handling in `applyLiveEvent`: if `env.seq > lastSeq + 1` keep it in a pending buffer and refetch `liveTranscript` (debounced) to fill the gap; never silently drop. Gaps produced by backend compaction are fine because transcript returns the original seqs (treat a transcript as authoritative: replace state from it).
4. Show every running process of a native id (if two exist, list both with a warning badge "同一会话有两个进程"), not only the newest.
5. `mergeList`: never drop a conv/selection that was created by `startLive` in the last 5 s or that has buffered events.
6. Per-conversation UI state map (module-level, keyed by conversation key): composer text, images, scrollTop + stuck-to-bottom flag, and composer settings (permission/model/effort/args) once the conversation has started (drafts still default from global prefs). Restore on switch.
7. Store subscriptions: `useLiveStore(selector)` with `useSyncExternalStore` + selector so components re-render only when their slice changes (sidebar badge = counts only; list = session infos only; conversation = its own conv).
8. `memdock-image:<path>` images in transcripts: load via `readImageDataUri` lazily for thumbnails.

# Command composer: backend ⇄ frontend contract

Base: branch `feature/live-chat` at 0d191af. Backend agent changes Rust only; frontend agent changes TS only (including `src/api/*.ts`), developing against this contract with mocks.

## 1. Thinking effort
- `StartOptions` (Rust `live/mod.rs`, TS `src/api/live.ts`): new optional `effort: Option<String>` / `effort?: string | null` (serde default None).
  - Claude: `--effort <level>` (both the stream-json live driver and the PTY CLI).
  - Codex: `-c model_reasoning_effort=<level>` (app-server spawn args and the PTY CLI).
- `PtyOpenOptions` (Rust `pty/mod.rs`, TS `src/api/pty.ts`): same `effort` field.
- `ModelOption` gets `efforts: Vec<String>` (supported levels; Claude `supportedEffortLevels`, Codex `supportedReasoningEfforts[].reasoningEffort` / cache `supported_reasoning_levels[].effort`) and `default_effort: Option<String>` (Codex `defaultReasoningEffort` / `default_reasoning_level`; Claude none). TS: `efforts: string[]; default_effort: string | null`.

## 2. CLI commands and skills (for the `/` menu)
- Tauri command `live_commands(agent: Agent, force: Option<bool>) -> Vec<CommandOption>`; TS `liveCommands(agent, force = false): Promise<CommandOption[]>`.
- `CommandOption { name: String /* without the leading slash */, description: String, argument_hint: String, kind: "skill" | "command" }` (serde snake_case).
  - Claude: the `commands` array of the `initialize` control response (same probe as `live_models`; fetch both in one CLI run and cache both for an hour). Entries whose description ends with "(user)" / "(project)" / "(plugin...)" or that come from skills are `skill`; built-ins are `command` — best effort, by heuristic.
  - Codex: `skills/list` on app-server (check bindings for params/response) → `skill`; plus nothing else (Codex app-server has no slash command list).

## 3. Project files (for the `@` menu)
- Tauri command `project_files(cwd: String, query: String, limit: Option<u32>) -> Vec<ProjectFile>`; TS `projectFiles(cwd, query, limit = 50)`.
- `ProjectFile { path: String /* relative to cwd, forward slashes; dirs end with "/" */, is_dir: bool }`.
- Respect `.gitignore` (use `git ls-files -co --exclude-standard` when cwd is a git repo, else a walk skipping node_modules/target/.git/dist/build, max depth 8, max 20000 entries); dirs derived from file paths. Fuzzy-ish match: all query chars as case-insensitive subsequence, rank by (basename contains query, shorter path). Cache the file list per cwd for 30 s.

## 4. Plain terminal
- `PtyOpenOptions.tool` accepts `"shell"` in addition to `"claude" | "codex"` (Rust: make `tool` an enum `PtyTool { Claude, Codex, Shell }` or similar; serde snake_case). Shell: Windows `pwsh.exe` if found else `powershell.exe` (`-NoLogo`); macOS/Linux `$SHELL -l` (fallback `/bin/zsh` / `/bin/bash`). `resume_id`, `permission`, `model`, `effort` are ignored. `display_command` shows e.g. `PowerShell`.

## 5. Pasted images for the terminal
- Tauri command `save_temp_image(data_uri: String) -> String` (absolute path of a file in `%TEMP%/memdock-images/`, same as the Codex driver's `save_image`; reuse it). TS `saveTempImage(dataUri)`.

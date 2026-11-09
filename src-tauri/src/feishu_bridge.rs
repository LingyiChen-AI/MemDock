use std::{
    collections::{HashMap, HashSet, VecDeque},
    fs,
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{
    commands::AppState,
    feishu_agent,
    feishu_transport::{self, IncomingMessage},
    fsutil,
};

mod conversation;

#[cfg(test)]
mod tests;

const MAX_TEXT: usize = 16 * 1024;
const MAX_PENDING: usize = 64;
const MAX_SEEN: usize = 4096;
const MAX_AUDIT: u64 = 1024 * 1024;
const HELP: &str = "直接说你想处理哪个项目和任务，例如「看看 cc-memory 的登录问题」。\n/projects 查看绑定项目 · /project <编号> 切换 · /status 查看当前项目\n/new 开始新会话 · /stop 停止任务\n/ls · /read · /create · /edit · /delete · /confirm";

#[derive(Clone, Copy, Default, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
enum Engine {
    #[default]
    Codex,
    Claude,
}

fn enabled() -> bool {
    true
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProjectGrant {
    path: String,
    #[serde(default = "enabled")]
    read: bool,
    #[serde(default)]
    create: bool,
    #[serde(default)]
    edit: bool,
    #[serde(default)]
    delete: bool,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Bot {
    id: String,
    name: String,
    app_id: String,
    #[serde(default)]
    registered_credentials: bool,
    #[serde(default)]
    start_with_client: bool,
    engine: Engine,
    projects: Vec<ProjectGrant>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BotInput {
    id: Option<String>,
    name: String,
    app_id: String,
    app_secret: Option<String>,
    registration_id: Option<String>,
    engine: Engine,
    projects: Vec<ProjectGrant>,
}

fn accepts_chat_message(event: &IncomingMessage) -> bool {
    !event.is_group || event.mentions_bot
}

fn linked(meta: &fs::Metadata) -> bool {
    if meta.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        return meta.file_attributes() & 0x400 != 0;
    }
    #[cfg(not(windows))]
    false
}

fn canonical_root(path: &Path) -> Result<PathBuf, String> {
    let unavailable = || "Project folder is unavailable or linked".to_string();
    if !path.is_absolute() || path.parent().is_none() {
        return Err(unavailable());
    }
    for ancestor in path.ancestors().filter(|p| p.parent().is_some()) {
        let meta = fs::symlink_metadata(ancestor).map_err(|_| unavailable())?;
        if linked(&meta) || !meta.is_dir() {
            return Err(unavailable());
        }
    }
    path.canonicalize().map_err(|_| unavailable())
}

fn allowed_segment(part: &str) -> bool {
    if part.is_empty()
        || part.starts_with('.')
        || part.ends_with(['.', ' '])
        || part.contains([':', '\\', '<', '>', '"', '|', '?', '*'])
        || part.chars().any(char::is_control)
    {
        return false;
    }
    let lower = part.to_ascii_lowercase();
    let stem = lower.split('.').next().unwrap_or("");
    !matches!(
        stem,
        "con"
            | "prn"
            | "aux"
            | "nul"
            | "conin$"
            | "conout$"
            | "credentials"
            | "secrets"
            | "auth"
            | "id_rsa"
            | "id_ed25519"
    ) && !["com", "lpt"].iter().any(|prefix| {
        stem.strip_prefix(prefix).is_some_and(|suffix| {
            suffix.chars().count() == 1
                && suffix
                    .chars()
                    .all(|c| c.is_ascii_digit() || matches!(c, '¹' | '²' | '³'))
        })
    }) && ![".pem", ".key", ".pfx", ".p12", ".kdbx"]
        .iter()
        .any(|ext| lower.ends_with(ext))
}

fn resolve(root: &Path, relative: &str, must_exist: bool) -> Result<PathBuf, String> {
    if canonical_root(root)? != root {
        return Err("Project root changed".into());
    }
    let mut result = root.to_owned();
    let mut parts = relative.split('/').peekable();
    while let Some(part) = parts.next() {
        if !allowed_segment(part)
            || !matches!(
                Path::new(part).components().next(),
                Some(Component::Normal(_))
            )
        {
            return Err("Use a safe project-relative path with / separators".into());
        }
        result.push(part);
        match fs::symlink_metadata(&result) {
            Ok(meta) => {
                if linked(&meta) || result.canonicalize().ok().as_ref() != Some(&result) {
                    return Err("Linked or aliased paths are outside the bridge scope".into());
                }
                if !meta.is_dir() && !meta.is_file() {
                    return Err("Only regular project files and folders are supported".into());
                }
                if parts.peek().is_some() && !meta.is_dir() {
                    return Err("Parent folder is unavailable".into());
                }
                if meta.is_file() && !single_link(&open_file(&result)?)? {
                    return Err("Hard-linked files are outside the bridge scope".into());
                }
            }
            Err(err)
                if err.kind() == std::io::ErrorKind::NotFound
                    && !must_exist
                    && parts.peek().is_none() => {}
            Err(_) => return Err("Project path is unavailable".into()),
        }
    }
    Ok(result)
}

fn open_file(path: &Path) -> Result<fs::File, String> {
    let meta = fs::symlink_metadata(path).map_err(|_| "File is unavailable")?;
    if linked(&meta) || !meta.is_file() {
        return Err("Only regular project files are supported".into());
    }
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // FILE_FLAG_OPEN_REPARSE_POINT keeps final-component links unopened.
        options.custom_flags(0x0020_0000);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // Avoid following a final-component symlink or blocking if the file is replaced by a FIFO.
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options.open(path).map_err(|_| "File is unavailable")?;
    let meta = file.metadata().map_err(|_| "File is unavailable")?;
    if linked(&meta) || !meta.is_file() {
        return Err("Only regular project files are supported".into());
    }
    Ok(file)
}

#[cfg(unix)]
fn single_link(file: &fs::File) -> Result<bool, String> {
    use std::os::unix::fs::MetadataExt;
    Ok(file.metadata().map_err(|_| "File is unavailable")?.nlink() == 1)
}

#[cfg(windows)]
fn single_link(file: &fs::File) -> Result<bool, String> {
    use std::{ffi::c_void, mem::MaybeUninit, os::windows::io::AsRawHandle};

    // BY_HANDLE_FILE_INFORMATION consists of thirteen DWORDs, including the
    // file timestamps and the link count in its documented ABI order.
    #[repr(C)]
    struct FileInformation {
        attributes_and_times: [u32; 7],
        volume_serial_number: u32,
        file_size: [u32; 2],
        number_of_links: u32,
        file_index: [u32; 2],
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetFileInformationByHandle(file: *mut c_void, info: *mut FileInformation) -> i32;
    }
    let mut info = MaybeUninit::uninit();
    // The OS writes the full structure on success and borrows a live handle.
    if unsafe { GetFileInformationByHandle(file.as_raw_handle(), info.as_mut_ptr()) } == 0 {
        return Err("Cannot verify file link count".into());
    }
    Ok(unsafe { info.assume_init() }.number_of_links == 1)
}

fn read_file(path: &Path) -> Result<(String, String), String> {
    let file = open_file(path)?;
    if !single_link(&file)? {
        return Err("Hard-linked files are outside the bridge scope".into());
    }
    let mut bytes = Vec::new();
    file.take(MAX_TEXT as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "File could not be read")?;
    if bytes.len() > MAX_TEXT || bytes.contains(&0) {
        return Err("Only small UTF-8 files are supported".into());
    }
    let revision = format!("{:x}", Sha256::digest(&bytes));
    let text = String::from_utf8(bytes).map_err(|_| "Only UTF-8 files are supported")?;
    Ok((text, revision))
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct Chat {
    id: String,
    user: String,
}

struct Deletion {
    chat: Chat,
    project: String,
    relative: String,
    revision: String,
    expires: Instant,
}

#[derive(Default)]
struct ChatState {
    selected: HashMap<Chat, usize>,
    histories: HashMap<(Chat, usize), conversation::History>,
    pending: HashMap<Uuid, Deletion>,
    seen: HashSet<String>,
    recent: VecDeque<String>,
    boundaries: Vec<PathBuf>,
}

impl ChatState {
    fn remember(&mut self, id: &str) -> bool {
        if id.is_empty() || !self.seen.insert(id.to_owned()) {
            return false;
        }
        self.recent.push_back(id.to_owned());
        if self.recent.len() > MAX_SEEN {
            if let Some(old) = self.recent.pop_front() {
                self.seen.remove(&old);
            }
        }
        true
    }

    fn target(&self, root: &Path, relative: &str, must_exist: bool) -> Result<PathBuf, String> {
        let result = if relative.is_empty() {
            canonical_root(root)?
        } else {
            resolve(root, relative, must_exist)?
        };
        if self
            .boundaries
            .iter()
            .any(|other| other != root && other.starts_with(root) && result.starts_with(other))
        {
            return Err("Select the nested project separately".into());
        }
        Ok(result)
    }
}

fn require(enabled: bool) -> Result<(), String> {
    if enabled {
        Ok(())
    } else {
        Err("This project permission is disabled in MemDock".into())
    }
}

fn content(text: &str) -> Result<&str, String> {
    if text.len() > MAX_TEXT || text.contains('\0') {
        return Err("Only small UTF-8 files are supported".into());
    }
    Ok(text)
}

fn respond(bot: &Bot, event: &IncomingMessage, state: &mut ChatState) -> Result<String, String> {
    respond_text(bot, event, state, &event.text)
}

fn respond_text(
    bot: &Bot,
    event: &IncomingMessage,
    state: &mut ChatState,
    text: &str,
) -> Result<String, String> {
    let chat = Chat {
        id: event.chat_id.clone(),
        user: event.sender_open_id.clone(),
    };
    let (line, body) = text.split_once('\n').unwrap_or((text, ""));
    let (command, argument) = line
        .trim()
        .split_once(char::is_whitespace)
        .map_or((line.trim(), ""), |(c, a)| (c, a.trim()));
    match command {
        "/help" => return Ok(HELP.into()),
        "/projects" => {
            if bot.projects.is_empty() {
                return Ok("No projects are enabled for this bot.".into());
            }
            return Ok(format!(
                "Projects:\n{}",
                bot.projects
                    .iter()
                    .enumerate()
                    .map(|(i, grant)| {
                        let name = Path::new(&grant.path)
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("project");
                        format!("{}. {name}", i + 1)
                    })
                    .collect::<Vec<_>>()
                    .join("\n"),
            ));
        }
        "/project" => {
            let index = argument
                .parse::<usize>()
                .ok()
                .and_then(|value| value.checked_sub(1))
                .filter(|index| *index < bot.projects.len())
                .ok_or("Choose a project number from /projects")?;
            state.selected.insert(chat.clone(), index);
            state.pending.retain(|_, deletion| deletion.chat != chat);
            return Ok(format!(
                "当前项目 · {}",
                conversation::name(&bot.projects[index])
            ));
        }
        "/status" => {
            return Ok(state
                .selected
                .get(&chat)
                .and_then(|index| bot.projects.get(*index))
                .map_or_else(
                    || "尚未选择项目。".into(),
                    |grant| format!("当前项目 · {}", conversation::name(grant)),
                ));
        }
        "/new" => {
            let index = state.selected.get(&chat).copied().ok_or("请先选择项目。")?;
            state.histories.remove(&(chat.clone(), index));
            state.pending.retain(|_, deletion| deletion.chat != chat);
            return Ok("已开启新会话。".into());
        }
        "/ls" | "/read" | "/create" | "/edit" | "/delete" | "/confirm" => {}
        _ => return Ok(HELP.into()),
    }
    let grant = state
        .selected
        .get(&chat)
        .and_then(|i| bot.projects.get(*i))
        .ok_or("Choose a project with /project <number>")?;
    let root = Path::new(&grant.path);
    if argument.is_empty() && command != "/ls" {
        return Err("A project-relative path or confirmation token is required".into());
    }
    match command {
        "/ls" => {
            require(grant.read)?;
            let folder = state.target(root, argument, true)?;
            let mut names = fs::read_dir(folder)
                .map_err(|_| "Project folder is unavailable")?
                .take(512)
                .filter_map(|entry| entry.ok())
                .filter_map(|entry| {
                    let name = entry.file_name().into_string().ok()?;
                    if !allowed_segment(&name) || linked(&fs::symlink_metadata(entry.path()).ok()?)
                    {
                        return None;
                    }
                    let relative = if argument.is_empty() {
                        name.clone()
                    } else {
                        format!("{argument}/{name}")
                    };
                    state.target(root, &relative, true).ok()?;
                    Some(name)
                })
                .take(50)
                .collect::<Vec<_>>();
            names.sort();
            if names.is_empty() {
                return Ok("No visible entries.".into());
            }
            Ok(names.join("\n"))
        }
        "/read" => {
            require(grant.read)?;
            let (text, revision) = read_file(&state.target(root, argument, true)?)?;
            if !crate::secrets::scan(&text).is_empty() {
                return Err("This file contains possible secrets and was withheld".into());
            }
            Ok(format!("SHA-256 {revision}\n{text}"))
        }
        "/create" => {
            require(grant.create)?;
            if !text.contains('\n') {
                return Err("Put the file contents on the next line".into());
            }
            let text = content(body)?;
            let file = state.target(root, argument, false)?;
            let parent = file.parent().ok_or("Parent folder is unavailable")?;
            let mut staged = tempfile::NamedTempFile::new_in(parent)
                .map_err(|_| "Could not prepare the new file")?;
            staged
                .write_all(text.as_bytes())
                .map_err(|_| "Could not prepare the new file")?;
            staged
                .as_file()
                .sync_all()
                .map_err(|_| "Could not prepare the new file")?;
            state.target(root, argument, false)?;
            staged
                .persist_noclobber(file)
                .map_err(|_| "Could not create a new file at that path")?;
            Ok("File created.".into())
        }
        "/edit" => {
            require(grant.edit)?;
            let (revision, text) = body
                .split_once('\n')
                .ok_or("Provide the SHA-256 and contents on separate lines")?;
            let text = content(text)?;
            let target = state.target(root, argument, true)?;
            let current = read_file(&target)?.1;
            if current != revision.trim() {
                return Err("File changed. Read the current revision before editing".into());
            }
            crate::memory::io::write_atomic_if(&target, text.as_bytes(), || {
                let current = state
                    .target(root, argument, true)
                    .and_then(|p| read_file(&p));
                if current.is_ok_and(|(_, hash)| hash == revision.trim()) {
                    Ok(())
                } else {
                    Err(std::io::Error::other("File changed"))
                }
            })
            .map_err(|_| "Could not save the file or it changed during editing")?;
            Ok("File updated.".into())
        }
        "/delete" => {
            require(grant.delete)?;
            let target = state.target(root, argument, true)?;
            let revision = read_file(&target)?.1;
            let now = Instant::now();
            state
                .pending
                .retain(|_, d| d.expires > now && d.chat != chat);
            if state.pending.len() >= MAX_PENDING {
                return Err("Too many pending deletion requests. Try again later".into());
            }
            let token = Uuid::new_v4();
            state.pending.insert(
                token,
                Deletion {
                    chat,
                    project: grant.path.clone(),
                    relative: argument.into(),
                    revision,
                    expires: now + Duration::from_secs(300),
                },
            );
            Ok(format!(
                "Confirm deletion of {argument} within 5 minutes with /confirm {token}"
            ))
        }
        "/confirm" => {
            require(grant.delete)?;
            let token = Uuid::parse_str(argument).map_err(|_| "Invalid or expired confirmation")?;
            let valid = state.pending.get(&token).is_some_and(|d| {
                d.chat == chat && d.project == grant.path && d.expires > Instant::now()
            });
            if !valid {
                return Err("Invalid or expired confirmation".into());
            }
            let deletion = state
                .pending
                .remove(&token)
                .ok_or("Confirmation already used")?;
            let target = state.target(root, &deletion.relative, true)?;
            if read_file(&target)?.1 != deletion.revision {
                return Err("File changed. Request deletion again".into());
            }
            trash::delete(target).map_err(|_| "Could not move the file to Trash")?;
            Ok("File moved to Trash.".into())
        }
        _ => unreachable!(),
    }
}

#[derive(Default, Serialize, Deserialize)]
struct BotStore {
    version: u32,
    bots: Vec<Bot>,
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "lowercase")]
enum Status {
    Stopped,
    Connecting,
    Connected,
    Error,
    Deleted,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BotView {
    #[serde(flatten)]
    config: Bot,
    has_secret: bool,
    status: Status,
    error: Option<String>,
}

struct Active {
    generation: Uuid,
    stop: Arc<AtomicBool>,
    close: tokio::sync::watch::Sender<bool>,
    status: Status,
    error: Option<String>,
    chat: ChatState,
    jobs: HashMap<Chat, Arc<AtomicBool>>,
}

impl Active {
    fn cancel(&self) {
        self.stop.store(true, Ordering::Release);
        for job in self.jobs.values() {
            job.store(true, Ordering::Release);
        }
        let _ = self.close.send(true);
    }
}

impl Drop for Active {
    fn drop(&mut self) {
        self.cancel();
    }
}

pub(crate) struct Bridge {
    app: Arc<AppState>,
    active: Mutex<HashMap<String, Active>>,
    audit_lock: Mutex<()>,
}

#[derive(Serialize)]
pub(crate) struct ProjectOption {
    path: String,
    name: String,
}

fn keychain(id: &str) -> Result<keyring::Entry, String> {
    if !cfg!(any(target_os = "windows", target_os = "macos")) {
        return Err("The Feishu OS credential store is unavailable on this platform".into());
    }
    keyring::Entry::new("MemDock", &format!("feishu:{id}"))
        .map_err(|_| "OS credential store is unavailable".into())
}

fn secret(id: &str) -> Result<Option<String>, String> {
    match keychain(id)?.get_password() {
        Ok(secret) if !secret.trim().is_empty() => Ok(Some(secret)),
        Ok(_) | Err(keyring::Error::NoEntry) => Ok(None),
        Err(_) => Err("Could not read the Feishu app secret from the OS credential store".into()),
    }
}

fn store_secret(id: &str, secret: Option<&str>) -> Result<(), String> {
    let entry = keychain(id)?;
    match secret {
        Some(secret) => entry.set_password(secret),
        None => match entry.delete_credential() {
            Err(keyring::Error::NoEntry) => Ok(()),
            result => result,
        },
    }
    .map_err(|_| "Could not update the Feishu app secret in the OS credential store".into())
}

fn identifier(value: &str, prefix: &str) -> bool {
    value.len() <= 128
        && value.strip_prefix(prefix).is_some_and(|suffix| {
            !suffix.is_empty()
                && suffix
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
        })
}

#[derive(Clone, Default)]
struct ReplyUpdate {
    progress: String,
    answer: Option<String>,
}

impl ReplyUpdate {
    fn render(&self, answer: Option<&str>) -> String {
        match (self.progress.is_empty(), answer) {
            (_, None) => self.progress.clone(),
            (true, Some(text)) => text.to_owned(),
            (false, Some(text)) => format!("{}\n\n{text}", self.progress),
        }
    }
}

impl Bridge {
    pub(crate) fn new(app: Arc<AppState>) -> Self {
        Self {
            app,
            active: Mutex::new(HashMap::new()),
            audit_lock: Mutex::new(()),
        }
    }

    fn path(&self) -> PathBuf {
        self.app.data_dir.join("feishu-bridge.json")
    }

    fn load(&self) -> Result<BotStore, String> {
        match fsutil::load_json_or_backup(&self.path(), 1, "Feishu bridge settings")? {
            fsutil::Loaded::Missing => Ok(BotStore {
                version: 1,
                bots: Vec::new(),
            }),
            fsutil::Loaded::Ok(store) => Ok(store),
            fsutil::Loaded::Recovered => {
                Err("Feishu bridge settings are unreadable and were backed up".into())
            }
        }
    }

    fn persist(&self, store: &BotStore) -> Result<(), String> {
        fsutil::save_json_atomic(&self.path(), store)
            .map_err(|_| "Could not save Feishu bridge settings".into())
    }

    fn active(&self) -> Result<std::sync::MutexGuard<'_, HashMap<String, Active>>, String> {
        self.active
            .lock()
            .map_err(|_| "Feishu bridge is unavailable".into())
    }

    pub(crate) fn stop_all(&self) {
        if let Ok(mut active) = self.active() {
            active.clear();
        }
    }

    fn options(&self) -> Vec<ProjectOption> {
        let mut seen = HashSet::new();
        let home = self.app.home.canonicalize().ok();
        let protected = [
            &self.app.data_dir,
            &self.app.paths.claude_home,
            &self.app.paths.codex_home,
        ]
        .into_iter()
        .filter_map(|path| path.canonicalize().ok())
        .collect::<Vec<_>>();
        self.app
            .snapshot_index()
            .projects
            .into_iter()
            .filter_map(|entry| {
                let root = canonical_root(Path::new(&entry.project.path)).ok()?;
                if home.as_ref().is_some_and(|home| home.starts_with(&root))
                    || protected
                        .iter()
                        .any(|path| path.starts_with(&root) || root.starts_with(path))
                {
                    return None;
                }
                let path = root.into_os_string().into_string().ok()?;
                seen.insert(path.clone()).then_some(ProjectOption {
                    path,
                    name: entry.project.name,
                })
            })
            .collect()
    }

    fn bot(&self, id: &str) -> Result<Bot, String> {
        self.load()?
            .bots
            .into_iter()
            .find(|bot| bot.id == id)
            .ok_or_else(|| "Feishu bot was not found".into())
    }

    fn validate_binding(&self, input: BotInput, store: &BotStore, registered: bool, deleted: bool) -> Result<(Bot, Option<String>), String> {
        let name = input.name.trim().to_owned();
        let app_id = input.app_id.trim().to_owned();
        if name.is_empty() || name.len() > 120 || name.chars().any(char::is_control) {
            return Err("Enter a valid bot name".into());
        }
        if !identifier(&app_id, "cli_") {
            return Err("Enter a valid Feishu App ID beginning with cli_".into());
        }
        let id = match input.id {
            Some(id) if store.bots.iter().any(|b| b.id == id) => id,
            Some(_) => return Err("Feishu bot was not found".into()),
            None if store.bots.len() >= 20 => return Err("Too many Feishu bots".into()),
            None => Uuid::new_v4().to_string(),
        };
        if store.bots.iter().any(|b| b.app_id == app_id && b.id != id) {
            return Err("This Feishu App ID is already registered".into());
        }
        let options = self
            .options()
            .into_iter()
            .map(|p| p.path)
            .collect::<HashSet<_>>();
        let mut projects = Vec::new();
        let mut seen = HashSet::new();
        if input.projects.len() > 64 {
            return Err("Too many projects for this Feishu bot".into());
        }
        for mut grant in input.projects {
            let root = canonical_root(Path::new(&grant.path))?;
            grant.path = root
                .into_os_string()
                .into_string()
                .map_err(|_| "Project path must be UTF-8")?;
            if !options.contains(&grant.path) {
                return Err("Select a currently registered MemDock project".into());
            }
            if !seen.insert(grant.path.clone()) {
                return Err("A project can be selected only once".into());
            }
            projects.push(grant);
        }
        let new_secret = input
            .app_secret
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty());
        if new_secret
            .as_ref()
            .is_some_and(|s| s.len() > 512 || s.chars().any(char::is_whitespace))
        {
            return Err("Enter a valid Feishu app secret".into());
        }
        if store.bots.iter().any(|b| b.id == id && b.app_id != app_id) && new_secret.is_none() {
            return Err("Enter the app secret for the new Feishu App ID".into());
        }
        let existing = store.bots.iter().find(|bot| bot.id == id);
        if deleted && existing.is_some_and(|bot| bot.app_id == app_id) {
            return Err(feishu_transport::APP_DELETED.into());
        }
        let previously_registered = existing.filter(|bot| bot.registered_credentials && !deleted);
        if let Some(previous) = previously_registered {
            if app_id != previous.app_id || new_secret.is_some() {
                return Err("Automatically registered Feishu credentials are read-only".into());
            }
        }
        let bot = Bot {
            id,
            name,
            app_id,
            registered_credentials: registered || previously_registered.is_some(),
            start_with_client: existing.is_some_and(|bot| bot.start_with_client),
            engine: input.engine,
            projects,
        };
        Ok((bot, new_secret))
    }
}

fn view(config: Bot, has_secret: bool, active: Option<&Active>) -> BotView {
    BotView {
        config,
        has_secret,
        status: active.map_or(Status::Stopped, |a| a.status),
        error: active.and_then(|a| a.error.clone()),
    }
}

type State<'a> = tauri::State<'a, Arc<Bridge>>;

#[tauri::command]
pub(crate) async fn feishu_projects(state: State<'_>) -> Result<Vec<ProjectOption>, String> {
    let bridge = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || bridge.options())
        .await
        .map_err(|_| "Feishu bridge is unavailable".into())
}

#[tauri::command]
pub(crate) async fn feishu_list(state: State<'_>) -> Result<Vec<BotView>, String> {
    let bridge = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let active = bridge.active()?;
        bridge
            .load()?
            .bots
            .into_iter()
            .map(|bot| {
                let has_secret = secret(&bot.id)?.is_some();
                let runtime = active.get(&bot.id);
                Ok(view(bot, has_secret, runtime))
            })
            .collect()
    })
    .await
    .map_err(|_| "Feishu bridge is unavailable".to_string())?
}

#[tauri::command]
pub(crate) async fn feishu_save(state: State<'_>, mut input: BotInput) -> Result<BotView, String> {
    let bridge = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut active = bridge.active()?;
        let mut store = bridge.load()?;
        let registration = input.registration_id.take();
        if let Some(id) = &registration {
            input.app_secret = Some(crate::feishu_registration::registered_secret(id, input.app_id.trim())?);
        }
        let deleted = input.id.as_ref().and_then(|id| active.get(id))
            .is_some_and(|runtime| matches!(runtime.status, Status::Deleted));
        let (bot, new_secret) = bridge.validate_binding(input, &store, registration.is_some(), deleted)?;
        let previous = secret(&bot.id)?;
        if let Some(new_secret) = &new_secret {
            store_secret(&bot.id, Some(new_secret))?;
        }
        match store.bots.iter_mut().find(|b| b.id == bot.id) {
            Some(existing) => *existing = bot.clone(),
            None => store.bots.push(bot.clone()),
        }
        if let Err(error) = bridge.persist(&store) {
            if new_secret.is_some() {
                store_secret(&bot.id, previous.as_deref()).map_err(|_| {
                    format!("{error}. The previous credential could not be restored")
                })?;
            }
            return Err(error);
        }
        active.remove(&bot.id);
        if let Some(id) = registration { crate::feishu_registration::feishu_cancel_create_app(id); }
        Ok(view(bot, new_secret.is_some() || previous.is_some(), None))
    })
    .await
    .map_err(|_| "Feishu bridge is unavailable".to_string())?
}

#[tauri::command]
pub(crate) async fn feishu_remove(state: State<'_>, id: String) -> Result<(), String> {
    let bridge = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut active = bridge.active()?;
        let mut store = bridge.load()?;
        let count = store.bots.len();
        store.bots.retain(|b| b.id != id);
        if store.bots.len() == count {
            return Err("Feishu bot was not found".into());
        }
        let previous = secret(&id)?;
        store_secret(&id, None)?;
        if let Err(error) = bridge.persist(&store) {
            store_secret(&id, previous.as_deref())
                .map_err(|_| format!("{error}. The previous credential could not be restored"))?;
            return Err(error);
        }
        active.remove(&id);
        Ok(())
    })
    .await
    .map_err(|_| "Feishu bridge is unavailable".to_string())?
}

#[tauri::command]
pub(crate) async fn feishu_set_start_with_client(
    state: State<'_>,
    id: String,
    enabled: bool,
) -> Result<(), String> {
    let bridge = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _active = bridge.active()?;
        let mut store = bridge.load()?;
        let bot = store.bots.iter_mut().find(|bot| bot.id == id)
            .ok_or("Feishu bot was not found")?;
        bot.start_with_client = enabled;
        bridge.persist(&store)
    })
    .await
    .map_err(|_| "Feishu bridge is unavailable".to_string())?
}

#[tauri::command]
pub(crate) async fn feishu_stop(state: State<'_>, id: String) -> Result<BotView, String> {
    let bridge = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut active = bridge.active()?;
        active.remove(&id);
        let bot = bridge.bot(&id)?;
        Ok(view(bot, secret(&id)?.is_some(), None))
    })
    .await
    .map_err(|_| "Feishu bridge is unavailable".to_string())?
}

struct Ask {
    engine: crate::model::Tool,
    relative: String,
    source: String,
    question: String,
}

fn prepare_ask(bot: &Bot, event: &IncomingMessage, state: &ChatState) -> Result<Ask, String> {
    let (header, question) = event
        .text
        .split_once('\n')
        .ok_or("Use /ask <file> and put the question on the next line")?;
    let relative = header
        .trim()
        .split_once(char::is_whitespace)
        .map(|(_, path)| path.trim())
        .filter(|path| !path.is_empty())
        .ok_or("A project-relative file is required")?;
    let question = question.trim();
    if question.is_empty() || question.len() > 4000 || question.contains('\0') {
        return Err("Provide a question up to 4,000 UTF-8 bytes".into());
    }
    let chat = Chat {
        id: event.chat_id.clone(),
        user: event.sender_open_id.clone(),
    };
    let grant = state
        .selected
        .get(&chat)
        .and_then(|i| bot.projects.get(*i))
        .ok_or("Choose a project with /project <number>")?;
    require(grant.read)?;
    let file = state.target(Path::new(&grant.path), relative, true)?;
    let (source, _) = read_file(&file)?;
    if !crate::secrets::scan(&source).is_empty() || !crate::secrets::scan(question).is_empty() {
        return Err("The request contains possible secrets and was withheld".into());
    }
    let engine = match bot.engine {
        Engine::Codex => crate::model::Tool::Codex,
        Engine::Claude => crate::model::Tool::Claude,
    };
    Ok(Ask {
        engine,
        relative: relative.into(),
        source,
        question: question.into(),
    })
}

impl Bridge {
    fn audit(
        &self,
        bot: &Bot,
        event: &IncomingMessage,
        project: Option<&str>,
        operation: &str,
        succeeded: bool,
    ) {
        let write = || -> std::io::Result<()> {
            let _lock = self
                .audit_lock
                .lock()
                .map_err(|_| std::io::Error::other("audit lock"))?;
            let path = self.app.data_dir.join("feishu-audit.jsonl");
            if fs::metadata(&path).is_ok_and(|meta| meta.len() >= MAX_AUDIT) {
                fs::rename(&path, self.app.data_dir.join("feishu-audit.previous.jsonl"))?;
            }
            let record = serde_json::json!({
                "timestamp": chrono::Utc::now().to_rfc3339(),
                "bot": bot.id,
                "actor": event.sender_open_id,
                "project": project,
                "operation": operation,
                "outcome": if succeeded { "ok" } else { "error" },
            });
            let mut line = serde_json::to_vec(&record).map_err(std::io::Error::other)?;
            line.push(b'\n');
            let mut audit = fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)?;
            audit.write_all(&line)
        };
        if let Err(error) = write() {
            eprintln!(
                "memdock: Feishu audit could not be written ({})",
                error.kind()
            );
        }
    }

    fn event(
        &self,
        bot: &Bot,
        generation: Uuid,
        event: &IncomingMessage,
        updates: &tokio::sync::watch::Sender<Option<ReplyUpdate>>,
    ) -> Option<String> {
        if !accepts_chat_message(event) {
            return None;
        }
        let mut active = self.active().ok()?;
        let runtime = active
            .get_mut(&bot.id)
            .filter(|a| a.generation == generation)?;
        if !runtime.chat.remember(&event.message_id) {
            return None;
        }
        let command = event.text.split_whitespace().next().unwrap_or("");
        if command == "/oversize" {
            return Some("The command exceeds the Feishu bridge size limit.".into());
        }
        let auditable = matches!(
            command,
            "/project" | "/ls" | "/read" | "/create" | "/edit" | "/delete" | "/confirm" | "/ask"
        );
        let chat = Chat {
            id: event.chat_id.clone(),
            user: event.sender_open_id.clone(),
        };
        if command == "/stop" {
            return Some("已处理停止请求。后续消息可以继续当前项目。".into());
        }
        let project = |state: &ChatState| {
            state
                .selected
                .get(&chat)
                .and_then(|index| bot.projects.get(*index))
                .map(|grant| grant.path.as_str())
        };
        let available = self
            .options()
            .into_iter()
            .map(|p| p.path)
            .collect::<HashSet<_>>();
        if bot.projects.iter().any(|p| !available.contains(&p.path)) {
            if auditable {
                self.audit(bot, event, project(&runtime.chat), command, false);
            }
            return Some(
                "A project is no longer registered or available. Review the bot in MemDock.".into(),
            );
        }
        runtime.chat.boundaries = available.into_iter().map(PathBuf::from).collect();
        if !event.text.trim_start().starts_with('/') && !event.text.trim().is_empty() {
            let current = runtime.chat.selected.get(&chat).copied();
            let cancel = Arc::new(AtomicBool::new(false));
            runtime.jobs.insert(chat.clone(), Arc::clone(&cancel));
            drop(active);
            updates.send_replace(Some(ReplyUpdate::default()));
            let result =
                self.conversation(bot, generation, event, &chat, current, &cancel, updates);
            if let Ok(mut active) = self.active() {
                if let Some(runtime) = active
                    .get_mut(&bot.id)
                    .filter(|a| a.generation == generation)
                {
                    runtime.jobs.remove(&chat);
                }
            }
            return Some(result.unwrap_or_else(|error| error));
        }
        if command == "/ask" {
            let project = project(&runtime.chat);
            let request = prepare_ask(bot, event, &runtime.chat);
            let cancel = Arc::clone(&runtime.stop);
            drop(active);
            if cancel.load(Ordering::Acquire) {
                return None;
            }
            let response = request
                .and_then(|request| {
                    feishu_agent::ask(
                        request.engine,
                        &request.relative,
                        &request.source,
                        &request.question,
                        &cancel,
                    )
                })
                .map(|text| crate::secrets::redact(&text));
            self.audit(
                bot,
                event,
                project,
                command,
                response.is_ok() && !cancel.load(Ordering::Acquire),
            );
            return Some(response.unwrap_or_else(|error| error));
        }
        let response = respond(bot, event, &mut runtime.chat);
        if auditable {
            self.audit(
                bot,
                event,
                project(&runtime.chat),
                command,
                response.is_ok(),
            );
        }
        Some(response.unwrap_or_else(|error| error))
    }

    fn conversation(
        &self,
        bot: &Bot,
        generation: Uuid,
        event: &IncomingMessage,
        chat: &Chat,
        current: Option<usize>,
        cancel: &AtomicBool,
        updates: &tokio::sync::watch::Sender<Option<ReplyUpdate>>,
    ) -> Result<String, String> {
        let message = event.text.trim();
        if message.len() > 4000 || !crate::secrets::scan(message).is_empty() {
            return Err("请使用不超过 4000 字节且不含密钥的任务描述。".into());
        }
        let tool = match bot.engine {
            Engine::Claude => crate::model::Tool::Claude,
            Engine::Codex => crate::model::Tool::Codex,
        };
        let query = |request: &str| feishu_agent::query(tool, request, cancel);
        let index = conversation::choose(bot, message, current, query)?;
        let grant = &bot.projects[index];
        let key = (chat.clone(), index);
        let mut history = {
            let mut active = self.active()?;
            let runtime = active
                .get_mut(&bot.id)
                .filter(|a| a.generation == generation)
                .ok_or("连接已停止。")?;
            if runtime.chat.selected.insert(chat.clone(), index) != Some(index) {
                runtime
                    .chat
                    .pending
                    .retain(|_, deletion| &deletion.chat != chat);
            }
            runtime
                .chat
                .histories
                .get(&key)
                .cloned()
                .unwrap_or_default()
        };
        if cancel.load(Ordering::Acquire) {
            return Err("任务已停止。".into());
        }
        let progress = std::cell::RefCell::new(Vec::new());
        let query_step = |request: &str| {
            if matches!(bot.engine, Engine::Claude) {
                let updates = updates.clone();
                let progress = progress.borrow().join("\n");
                feishu_agent::query_streaming(tool, request, cancel, Some(Box::new(move |answer| {
                    updates.send_replace(Some(ReplyUpdate { progress: progress.clone(), answer: Some(answer) }));
                })))
            } else {
                query(request)
            }
        };
        let result = conversation::turn(grant, &mut history, message, cancel, query_step, |step| {
            let mut active = self.active()?;
            let runtime = active
                .get_mut(&bot.id)
                .filter(|a| a.generation == generation)
                .ok_or("连接已停止。")?;
            if cancel.load(Ordering::Acquire) {
                return Err("任务已停止。".into());
            }
            let operation = step.action.audit_name();
            let result = conversation::execute(bot, event, &mut runtime.chat, step);
            self.audit(bot, event, Some(&grant.path), operation, result.is_ok());
            let label = match (operation, result.is_ok()) {
                ("/ls", true) => "查看了目录",
                ("/read", true) => "读取了文件",
                ("agent_search", true) => "搜索了项目",
                ("/create", true) => "创建了文件",
                ("/edit", true) => "修改了文件",
                ("/delete", true) => "提交了删除确认",
                _ => "项目操作未完成",
            };
            let mut progress = progress.borrow_mut();
            progress.push(label);
            updates.send_replace(Some(ReplyUpdate { progress: progress.join("\n"), answer: None }));
            result
        });
        if result.is_ok() {
            let mut active = self.active()?;
            let runtime = active
                .get_mut(&bot.id)
                .filter(|a| a.generation == generation)
                .ok_or("连接已停止。")?;
            runtime.chat.histories.insert(key, history);
        }
        result
    }
}

#[tauri::command]
pub(crate) async fn feishu_start(state: State<'_>, id: String) -> Result<BotView, String> {
    start_bot(state.inner().clone(), id).await
}

pub(crate) fn start_configured(bridge: Arc<Bridge>) {
    tauri::async_runtime::spawn(async move {
        let reader = Arc::clone(&bridge);
        let store = match tauri::async_runtime::spawn_blocking(move || {
            let _active = reader.active()?;
            reader.load()
        }).await {
            Ok(Ok(store)) => store,
            _ => {
                eprintln!("MemDock: could not load Feishu startup settings");
                return;
            }
        };
        for bot in store.bots.into_iter().filter(|bot| bot.start_with_client) {
            if let Err(error) = start_bot(Arc::clone(&bridge), bot.id).await {
                eprintln!("MemDock: could not auto-start a Feishu connection: {error}");
            }
        }
    });
}

async fn start_bot(bridge: Arc<Bridge>, id: String) -> Result<BotView, String> {
    let prepare = Arc::clone(&bridge);
    let (bot, client, generation, stop, stop_rx) = tauri::async_runtime::spawn_blocking(move || {
        let mut active = prepare.active()?;
        let bot = prepare.bot(&id)?;
        if bot.projects.is_empty() {
            return Err("Select at least one project before starting".to_string());
        }
        if active.get(&id).is_some_and(|runtime| matches!(runtime.status, Status::Deleted)) {
            return Err(feishu_transport::APP_DELETED.into());
        }
        if active.get(&id).is_some_and(|runtime| !matches!(runtime.status, Status::Error)) {
            return Err("The Feishu bot is already running".to_string());
        }
        let app_secret = secret(&id)?.ok_or("Save the Feishu app secret before starting")?;
        let client = feishu_transport::Client::new(feishu_transport::Config {
            app_id: bot.app_id.clone(),
            app_secret,
        });
        let generation = Uuid::new_v4();
        let stop = Arc::new(AtomicBool::new(false));
        let (close, stop_rx) = tokio::sync::watch::channel(false);
        active.insert(id, Active {
            generation,
            stop: Arc::clone(&stop),
            close,
            status: Status::Connecting,
            error: None,
            chat: ChatState::default(),
            jobs: HashMap::new(),
        });
        Ok((bot, client, generation, stop, stop_rx))
    }).await.map_err(|_| "Feishu bridge is unavailable".to_string())??;

    let client = Arc::new(client);
    let receiver_client = Arc::clone(&client);
    let receiver_bridge = Arc::clone(&bridge);
    let receiver_bot = bot.clone();
    let (events_tx, mut events_rx) = tokio::sync::mpsc::channel::<IncomingMessage>(64);
    let mut receiver_stop = stop_rx.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            let event = tokio::select! {
                biased;
                _ = receiver_stop.changed() => break,
                event = events_rx.recv() => match event {
                    Some(event) => event,
                    None => break,
                },
            };
            if stop.load(Ordering::Acquire) {
                break;
            }
            let message_id = event.message_id.clone();
            let bridge = Arc::clone(&receiver_bridge);
            let bot = receiver_bot.clone();
            let engine = bot.engine;
            let (updates, mut stream) = tokio::sync::watch::channel::<Option<ReplyUpdate>>(None);
            let publisher = Arc::clone(&receiver_client);
            let origin = message_id.clone();
            let publishing = tauri::async_runtime::spawn(async move {
                let mut card = None;
                let mut previous = String::new();
                let mut latest = ReplyUpdate::default();
                let mut failed = false;
                let mut reaction = None;
                while stream.changed().await.is_ok() {
                    let Some(update) = stream.borrow_and_update().clone() else {
                        continue;
                    };
                    if reaction.is_none() {
                        let client = Arc::clone(&publisher);
                        let origin = origin.clone();
                        reaction = Some(tauri::async_runtime::spawn(async move {
                            client.working_reaction(&origin).await
                        }));
                    }
                    latest = update;
                    let text = latest.render(latest.answer.as_deref());
                    if text.trim().is_empty() || failed || previous == text {
                        continue;
                    }
                    if card.is_none() {
                        card = publisher.start_markdown_stream(&origin).await.ok();
                    }
                    if let Some(card) = &mut card {
                        if publisher.write_markdown_stream(card, &text).await.is_ok() {
                            previous = text;
                        } else {
                            failed = true;
                        }
                    } else {
                        failed = true;
                    }
                    tokio::time::sleep(Duration::from_millis(600)).await;
                }
                (card, previous, latest, reaction)
            });
            let result = tauri::async_runtime::spawn_blocking(move || {
                bridge.event(&bot, generation, &event, &updates)
            })
            .await;
            let (card, previous, latest, reaction) = publishing.await.unwrap_or_default();
            let reply = result.ok().flatten().filter(|text| !text.trim().is_empty());
            let reply = if stop.load(Ordering::Acquire) { None } else { reply };
            // Match the upstream default Markdown mode: Claude finishes in the
            // open stream, while Codex holds its final answer for a separate post.
            let mut pending_reply = reply;
            if let Some(mut card) = card {
                let terminal = match (engine, pending_reply.as_deref()) {
                    (Engine::Claude, Some(text)) => latest.render(Some(text)),
                    (Engine::Codex, Some(_)) => previous.clone(),
                    (_, None) => String::new(),
                };
                if terminal.is_empty() {
                    let _ = receiver_client.recall_markdown_stream(&card).await;
                } else if terminal == previous
                    || receiver_client.write_markdown_stream(&mut card, &terminal).await.is_ok()
                {
                    let _ = receiver_client.close_markdown_stream(&mut card, &terminal).await;
                    if matches!(engine, Engine::Claude) {
                        pending_reply = None;
                    }
                } else {
                    let _ = receiver_client.recall_markdown_stream(&card).await;
                    pending_reply = Some(terminal);
                }
            }
            if let Some(reply) = pending_reply {
                let markdown = reaction.is_some()
                    && receiver_client.reply_markdown(&message_id, &reply).await.is_ok();
                if !markdown {
                    let _ = receiver_client.reply(&message_id, &reply).await;
                }
            }
            if let Some(reaction) = reaction {
                let client = Arc::clone(&receiver_client);
                tauri::async_runtime::spawn(async move {
                    if let Ok(Some(id)) = reaction.await {
                        client.remove_working_reaction(&message_id, &id).await;
                    }
                });
            }
        }
    });
    let bot_id = bot.id.clone();
    let transport_bot = bot.clone();
    let cancel_bridge = Arc::clone(&bridge);
    tauri::async_runtime::spawn(async move {
        client
            .run(
                stop_rx,
                move |mut event| {
                    if !accepts_chat_message(&event) {
                        return true;
                    }
                    if event.text.len() > MAX_TEXT + 1024 {
                        event.text = "/oversize".into();
                    }
                    if event.text.trim() == "/stop" {
                        let chat = Chat {
                            id: event.chat_id.clone(),
                            user: event.sender_open_id.clone(),
                        };
                        if let Ok(active) = cancel_bridge.active() {
                            if let Some(cancel) = active
                                .get(&transport_bot.id)
                                .filter(|runtime| runtime.generation == generation)
                                .and_then(|runtime| runtime.jobs.get(&chat))
                            {
                                cancel.store(true, Ordering::Release);
                            }
                        }
                    }
                    events_tx.try_send(event).is_ok()
                },
                move |status| {
                    if let Ok(mut active) = bridge.active() {
                        if let Some(runtime) = active
                            .get_mut(&bot_id)
                            .filter(|a| a.generation == generation)
                        {
                            let (status, error) = match status {
                                feishu_transport::Status::Connecting => (Status::Connecting, None),
                                feishu_transport::Status::Connected => (Status::Connected, None),
                                feishu_transport::Status::Disconnected(error) => {
                                    (Status::Error, Some(error))
                                }
                                feishu_transport::Status::AppDeleted => {
                                    runtime.cancel();
                                    (Status::Deleted, Some(feishu_transport::APP_DELETED.into()))
                                }
                            };
                            runtime.status = status;
                            runtime.error = error;
                        }
                    }
                },
            )
            .await;
    });
    Ok(BotView {
        config: bot,
        has_secret: true,
        status: Status::Connecting,
        error: None,
    })
}

#[tauri::command]
pub(crate) async fn feishu_test(state: State<'_>, id: String) -> Result<BotView, String> {
    let bridge = state.inner().clone();
    let prepare = Arc::clone(&bridge);
    let (bot, client) = tauri::async_runtime::spawn_blocking(move || {
        let _active = prepare.active()?;
        let bot = prepare.bot(&id)?;
        let app_secret = secret(&id)?.ok_or("Save the Feishu app secret before testing")?;
        let client = feishu_transport::Client::new(feishu_transport::Config {
            app_id: bot.app_id.clone(),
            app_secret,
        });
        Ok::<_, String>((bot, client))
    })
    .await
    .map_err(|_| "Feishu bridge is unavailable".to_string())??;
    client.probe().await?;
    let active = bridge.active()?;
    let runtime = active.get(&bot.id);
    Ok(view(bot, true, runtime))
}

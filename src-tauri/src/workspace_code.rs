use base64::Engine;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    fs,
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    process::Command,
    sync::Mutex,
};

#[cfg(test)]
#[path = "workspace_code_tests.rs"]
mod tests;

const MAX_TEXT: usize = 2 * 1024 * 1024;
const MAX_IMAGE: usize = 8 * 1024 * 1024;
static SAVES: Mutex<()> = Mutex::new(());
const FILE_CHANGED: &str = "The file changed on disk. Reopen it before saving.";

fn error(e: impl std::fmt::Display) -> String {
    e.to_string()
}

fn resolve(root: &Path, path: &str) -> Result<PathBuf, String> {
    let root = root.canonicalize().map_err(error)?;
    if !root.is_dir() {
        return Err("Workspace folder is unavailable".into());
    }
    let mut target = root.clone();
    for part in Path::new(path).components() {
        match part {
            Component::Normal(name)
                if !name.to_string_lossy().contains([':', '\\'])
                    && !name
                        .to_string_lossy()
                        .trim_end_matches(['.', ' '])
                        .eq_ignore_ascii_case(".git") =>
            {
                target.push(name);
                let actual = target.canonicalize().map_err(error)?;
                if actual != target {
                    return Err("Linked paths are outside the workspace editor scope".into());
                }
            }
            _ => return Err("Invalid workspace path".into()),
        }
    }
    Ok(target)
}

fn read_bytes(path: &Path, max: usize) -> Result<Vec<u8>, String> {
    let meta = fs::metadata(path).map_err(error)?;
    if !meta.is_file() || meta.len() > max as u64 {
        return Err("Unsupported file type or file too large".into());
    }
    let mut bytes = Vec::new();
    fs::File::open(path)
        .map_err(error)?
        .take(max as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(error)?;
    if bytes.len() > max {
        return Err("File is too large".into());
    }
    Ok(bytes)
}

#[derive(Serialize)]
pub struct TextFile {
    text: String,
    revision: String,
    baseline: Option<String>,
    #[serde(skip)]
    bom: bool,
    #[serde(skip)]
    crlf: bool,
    #[serde(skip)]
    mixed: bool,
}

fn read_text(path: &Path) -> Result<TextFile, String> {
    let bytes = read_bytes(path, MAX_TEXT)?;
    let revision = format!("{:x}", Sha256::digest(&bytes));
    let bom = bytes.starts_with(b"\xef\xbb\xbf");
    let source = std::str::from_utf8(&bytes[if bom { 3 } else { 0 }..])
        .map_err(|_| "Only UTF-8 text can be edited")?;
    if source.contains('\0') {
        return Err("Binary files cannot be edited".into());
    }
    let crlf = source.contains("\r\n");
    let text = source.replace("\r\n", "\n");
    let mixed = text.contains('\r')
        || (crlf && text.matches('\n').count() != source.matches("\r\n").count());
    Ok(TextFile {
        text,
        revision,
        baseline: None,
        bom,
        crlf,
        mixed,
    })
}

fn save_text(path: &Path, revision: &str, text: &str) -> Result<String, String> {
    let _guard = SAVES.lock().map_err(error)?;
    let original = read_text(path)?;
    if original.revision != revision {
        return Err(FILE_CHANGED.into());
    }
    if original.mixed {
        return Err("Saving files with mixed line endings is unsupported".into());
    }
    if text.contains(['\0', '\r']) || text.len() > MAX_TEXT {
        return Err("Invalid text or file too large".into());
    }
    let mut bytes = Vec::new();
    if original.bom {
        bytes.extend_from_slice(b"\xef\xbb\xbf");
    }
    if original.crlf {
        bytes.extend_from_slice(text.replace('\n', "\r\n").as_bytes());
    } else {
        bytes.extend_from_slice(text.as_bytes());
    }
    if bytes.len() > MAX_TEXT {
        return Err("File is too large".into());
    }
    crate::memory::io::write_atomic_if(path, &bytes, || {
        let current = read_bytes(path, MAX_TEXT).ok();
        if current.is_some_and(|current| format!("{:x}", Sha256::digest(current)) == revision) {
            Ok(())
        } else {
            Err(std::io::Error::other(FILE_CHANGED))
        }
    })
    .map_err(error)?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

fn git(cwd: &str, args: &[&str]) -> Option<Vec<u8>> {
    let mut cmd = Command::new("git");

    cmd.current_dir(cwd)
        .args(["--no-optional-locks", "-c", "core.fsmonitor=false"])
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null());
    let child = crate::proc::spawn_hidden(&mut cmd).ok()?;
    let output =
        crate::proc::wait_with_timeout(child, std::time::Duration::from_secs(8), None).ok()?;
    output.status.success().then_some(output.stdout)
}

fn baseline(cwd: &str, path: &str, base: Option<&crate::worktree::WorktreeInfo>) -> Option<String> {
    let prefix = String::from_utf8(git(cwd, &["rev-parse", "--show-prefix"])?).ok()?;
    let revision = match base {
        Some(info) => crate::worktree::diff_base(info, Path::new(cwd)).ok()?,
        None => "HEAD".into(),
    };
    let spec = format!("{revision}:{}{path}", prefix.trim_end_matches(['\n', '\r']));
    let Some(size) = git(cwd, &["cat-file", "-s", &spec]) else {
        git(cwd, &["rev-parse", "--verify", &revision])?;
        // Missing from the baseline can mean either a new file or an ignored
        // artifact. Include staged additions even when they were force-added.
        let visible_to_git = git(
            cwd,
            &[
                "--literal-pathspecs",
                "ls-files",
                "-z",
                "--cached",
                "--others",
                "--exclude-standard",
                "--",
                path,
            ],
        )?;
        return (!visible_to_git.is_empty()).then(String::new);
    };
    let size = String::from_utf8(size).ok();
    if size
        .and_then(|size| size.trim().parse::<usize>().ok())
        .is_some_and(|size| size <= MAX_TEXT)
    {
        let bytes = git(cwd, &["show", &spec])?;
        let text = String::from_utf8(bytes).ok()?;
        return (!text.contains('\0'))
            .then(|| text.trim_start_matches('\u{feff}').replace("\r\n", "\n"));
    }
    None
}

#[derive(Serialize)]
pub struct Entry {
    name: String,
    path: String,
    directory: bool,
}

fn ignored(name: &str) -> bool {
    [".git", "node_modules", ".memdock"]
        .iter()
        .any(|skip| name.eq_ignore_ascii_case(skip))
}

fn entries(root: &Path, path: &str) -> Result<Vec<Entry>, String> {
    let dir = resolve(root, path)?;
    let mut out = Vec::new();
    for item in fs::read_dir(dir).map_err(error)?.filter_map(Result::ok) {
        let Some(name) = item.file_name().into_string().ok() else {
            continue;
        };
        let Ok(kind) = item.file_type() else { continue };
        if ignored(&name) || !(kind.is_dir() || kind.is_file()) {
            continue;
        }
        let path = if path.is_empty() {
            name.clone()
        } else {
            format!("{path}/{name}")
        };
        out.push(Entry {
            name,
            path,
            directory: kind.is_dir(),
        });
    }
    out.sort_by(|a, b| {
        b.directory
            .cmp(&a.directory)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(out)
}

#[derive(Serialize)]
pub struct Match {
    path: String,
    line: Option<usize>,
    text: Option<String>,
}

fn search(root: &Path, query: &str, content: bool, exact_name: bool) -> Result<Vec<Match>, String> {
    let root = resolve(root, "")?;
    let query = if exact_name && !cfg!(windows) { query.to_owned() } else { query.to_lowercase() };
    let mut results = Vec::new();
    let mut budget = 64 * 1024 * 1024;
    for item in walkdir::WalkDir::new(&root)
        .max_depth(40)
        .into_iter()
        .filter_entry(|item| {
            item.depth() == 0 || item.file_name().to_str().is_some_and(|name| !ignored(name))
        })
        .filter_map(Result::ok)
        .take(if exact_name { usize::MAX } else { 30_000 })
    {
        if !item.file_type().is_file() {
            continue;
        }
        let Some(path) = item.path().strip_prefix(&root).ok().and_then(Path::to_str) else {
            continue;
        };
        let path = path.replace('\\', "/");
        if !content {
            let matches = if exact_name {
                let name = item.file_name().to_string_lossy();
                if cfg!(windows) { name.to_lowercase() == query } else { name == query }
            } else {
                path.to_lowercase().contains(&query)
            };
            if matches {
                results.push(Match {
                    path,
                    line: None,
                    text: None,
                });
            }
        } else {
            let Ok(meta) = item.metadata() else { continue };
            let size = meta.len() as usize;
            if size > MAX_TEXT || size > budget {
                continue;
            }
            budget -= size;
            if let Ok(file) = read_text(item.path()) {
                for (index, line) in file.text.lines().enumerate() {
                    if line.to_lowercase().contains(&query) {
                        results.push(Match {
                            path: path.clone(),
                            line: Some(index + 1),
                            text: Some(line.chars().take(240).collect()),
                        });
                    }
                    if results.len() >= 100 {
                        break;
                    }
                }
            }
        }
        if results.len() >= (if exact_name { 2 } else { 100 }) || budget == 0 {
            break;
        }
    }
    Ok(results)
}

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FileView {
    Text(TextFile),
    Image { data: String },
}

async fn background<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(error)?
}

fn create_file(root: &Path, directory: &str, name: &str, text: &str) -> Result<String, String> {
    if name.is_empty()
        || name.starts_with(' ')
        || name.ends_with(['.', ' '])
        || name.contains(['/', '\\', ':', '<', '>', '"', '|', '?', '*'])
        || name.chars().any(char::is_control)
        || ignored(name)
        || crate::commands_skills::is_windows_reserved(name)
    {
        return Err("Invalid file name".into());
    }
    if text.len() > MAX_TEXT || text.contains(['\0', '\r']) {
        return Err("Invalid text or file too large".into());
    }
    let parent = resolve(root, directory)?;
    if text.is_empty() {
        fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(parent.join(name))
            .map_err(error)?;
    } else {
        let mut file = tempfile::NamedTempFile::new_in(&parent).map_err(error)?;
        file.write_all(text.as_bytes()).map_err(error)?;
        file.as_file().sync_all().map_err(error)?;
        file.persist_noclobber(parent.join(name)).map_err(error)?;
    }
    Ok(if directory.is_empty() {
        name.to_owned()
    } else {
        format!("{directory}/{name}")
    })
}

fn trash_file(
    root: &Path,
    path: &str,
    move_to_trash: impl FnOnce(&Path) -> Result<(), String>,
) -> Result<(), String> {
    let _guard = SAVES.lock().map_err(error)?;
    let file = resolve(root, path)?;
    if !fs::symlink_metadata(&file)
        .map_err(error)?
        .file_type()
        .is_file()
    {
        return Err("Only workspace files can be moved to the trash".into());
    }
    move_to_trash(&file)
}

#[tauri::command]
pub async fn code_trash_file(cwd: String, path: String) -> Result<(), String> {
    background(move || {
        trash_file(Path::new(&cwd), &path, |file| {
            trash::delete(dunce_path(file)).map_err(error)
        })
    })
    .await
}

#[tauri::command]
pub async fn code_create_file(
    cwd: String,
    directory: String,
    name: String,
    text: Option<String>,
) -> Result<String, String> {
    background(move || {
        create_file(
            Path::new(&cwd),
            &directory,
            &name,
            text.as_deref().unwrap_or(""),
        )
    })
    .await
}

#[tauri::command]
pub async fn code_entries(cwd: String, path: String) -> Result<Vec<Entry>, String> {
    background(move || entries(Path::new(&cwd), &path)).await
}

#[tauri::command]
pub async fn code_read(
    cwd: String,
    path: String,
    base: Option<crate::worktree::WorktreeInfo>,
) -> Result<FileView, String> {
    background(move || {
        let file = resolve(Path::new(&cwd), &path)?;
        let ext = file
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let mime = match ext.as_str() {
            "jpg" | "jpeg" => Some("jpeg"),
            "png" => Some("png"),
            "gif" => Some("gif"),
            "webp" => Some("webp"),
            "ico" => Some("x-icon"),
            "bmp" => Some("bmp"),
            "avif" => Some("avif"),
            _ => None,
        };
        if let Some(mime) = mime {
            let data =
                base64::engine::general_purpose::STANDARD.encode(read_bytes(&file, MAX_IMAGE)?);
            return Ok(FileView::Image {
                data: format!("data:image/{mime};base64,{data}"),
            });
        }
        let mut text = read_text(&file)?;
        text.baseline = baseline(&cwd, &path, base.as_ref());
        Ok(FileView::Text(text))
    })
    .await
}

#[tauri::command]
pub async fn code_save(
    cwd: String,
    path: String,
    revision: String,
    text: String,
) -> Result<String, String> {
    background(move || save_text(&resolve(Path::new(&cwd), &path)?, &revision, &text)).await
}

#[tauri::command]
pub async fn code_search(cwd: String, query: String, content: bool) -> Result<Vec<Match>, String> {
    background(move || {
        if query.trim().is_empty() {
            return Ok(Vec::new());
        }
        search(Path::new(&cwd), query.trim(), content, false)
    })
    .await
}

#[tauri::command]
pub async fn code_resolve_link(cwd: String, path: String, bare_name: bool) -> Result<Vec<String>, String> {
    background(move || {
        let root = Path::new(&cwd);
        if let Ok(target) = resolve(root, &path) {
            if target.is_file() {
                return Ok(vec![path]);
            }
            if target.is_dir() {
                return Ok(vec![format!("{}/", path.trim_end_matches('/'))]);
            }
        }
        if !bare_name || path.contains(['/', '\\', ':']) || matches!(path.as_str(), "" | "." | "..") {
            return Ok(vec![]);
        }
        Ok(search(root, &path, false, true)?.into_iter()
            .filter(|hit| resolve(root, &hit.path).is_ok_and(|file| file.is_file()))
            .map(|hit| hit.path).collect())
    }).await
}

#[tauri::command]
pub async fn code_status(cwd: String) -> Result<HashMap<String, String>, String> {
    background(move || {
        let mut result = HashMap::new();
        let Some(prefix) = git(&cwd, &["rev-parse", "--show-prefix"]) else {
            return Ok(result);
        };
        let prefix = String::from_utf8_lossy(&prefix);
        let prefix = prefix.trim_end_matches(['\r', '\n']);
        let Some(output) = git(
            &cwd,
            &[
                "status",
                "--porcelain=v1",
                "-z",
                "--untracked-files=all",
                "--",
                ".",
            ],
        ) else {
            return Ok(result);
        };
        let mut records = output.split(|byte| *byte == 0);
        while let Some(record) = records.next() {
            if record.len() < 4 {
                continue;
            }
            let status = String::from_utf8_lossy(&record[..2]);
            if status.contains(['R', 'C']) {
                records.next();
            }
            let name = String::from_utf8_lossy(&record[3..]);
            if let Some(path) = name.strip_prefix(prefix) {
                let mark = if status == "??" { "U" } else { status.trim() };
                result.insert(path.into(), mark.into());
            }
        }
        Ok(result)
    })
    .await
}

#[tauri::command]
pub async fn code_open_vscode(cwd: String, path: String, line: Option<u32>) -> Result<(), String> {
    background(move || {
        let target = resolve(Path::new(&cwd), &path)?;
        launch_editor(&target, line, false)
    })
    .await
}

#[tauri::command]
pub async fn code_open_absolute(app: tauri::AppHandle, path: String, line: Option<u32>) -> Result<bool, String> {
    use tauri_plugin_opener::OpenerExt;
    background(move || {
        let path = match path.strip_prefix("~/") {
            Some(relative) => match dirs::home_dir() {
                Some(home) => home.join(relative),
                None => return Ok(false),
            },
            None => PathBuf::from(path),
        };
        let raw = path.to_string_lossy();
        if !path.is_absolute() || raw.starts_with("\\\\") || raw.starts_with("//") {
            return Ok(false);
        }
        let target = match path.canonicalize() {
            Ok(target) => target,
            _ => return Ok(false),
        };
        if target.is_dir() {
            app.opener().open_path(dunce_path(&target), None::<&str>).map_err(error)?;
        } else if target.is_file() {
            launch_editor(&target, line, true)?;
        } else {
            return Ok(false);
        }
        Ok(true)
    }).await
}

fn launch_editor(path: &Path, line: Option<u32>, allow_fallback: bool) -> Result<(), String> {
    let target = dunce_path(path);
    let mut command = Command::new(vscode_program());

    if let Some(line) = line {
        command.arg("--goto").arg(format!("{target}:{line}"));
    } else {
        command.arg("--").arg(&target);
    }
    crate::proc::spawn_hidden(&mut command)
        .or_else(|error| {
            if cfg!(windows) && allow_fallback && error.kind() == std::io::ErrorKind::NotFound {
                Command::new("notepad.exe").arg(target).spawn()
            } else {
                Err(error)
            }
        })
        .map(|mut child| {
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        })
        .map_err(|e| format!("Could not start the editor: {e}"))
}

fn vscode_program() -> PathBuf {
    #[cfg(windows)]
    {
        // The Windows CLI shim lives in <installation>/bin. Launch its executable
        // directly so workspace paths are never interpreted by a command shell.
        if let Some(path) = std::env::var_os("PATH") {
            for bin in std::env::split_paths(&path) {
                if bin.join("code.cmd").is_file() {
                    if let Some(exe) = bin
                        .parent()
                        .map(|parent| parent.join("Code.exe"))
                        .filter(|exe| exe.is_file())
                    {
                        return exe;
                    }
                }
            }
        }
        for variable in ["LOCALAPPDATA", "ProgramFiles", "ProgramFiles(x86)"] {
            if let Some(root) = std::env::var_os(variable) {
                let root = PathBuf::from(root);
                for folder in ["Programs/Microsoft VS Code", "Microsoft VS Code"] {
                    let exe = root.join(folder).join("Code.exe");
                    if exe.is_file() {
                        return exe;
                    }
                }
            }
        }
        PathBuf::from("Code.exe")
    }
    #[cfg(not(windows))]
    PathBuf::from("code")
}

fn dunce_path(path: &Path) -> String {
    let path = path.to_string_lossy();
    if let Some(unc) = path.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{unc}")
    } else {
        path.strip_prefix(r"\\?\").unwrap_or(&path).to_owned()
    }
}

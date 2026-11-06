pub mod commands;
pub mod commands_env;
pub mod commands_extract;
pub mod commands_sessions;
pub mod commands_ghimport;
pub mod commands_skill_manifest;
pub mod commands_skills;
pub mod commands_sync;
pub mod commands_usage;
pub mod convert;
pub mod env;
pub mod extract;
pub mod live;
pub mod fsutil;
pub mod ghimport;
pub mod memory;
pub mod model;
pub mod paths;
pub mod proc;
pub mod project_files;
pub mod pty;
pub mod projects;
pub mod secrets;
pub mod settings;
pub mod tray;
pub mod skills;
pub mod sync;
pub mod usage;
pub mod watcher;
pub mod window;
pub mod worktree;
#[cfg(test)]
mod e2e_tests;

use std::sync::Arc;

use tauri::{LogicalSize, Manager, WebviewWindow};
use tauri_plugin_window_state::{AppHandleExt, StateFlags};

/// Sizes and centers the main window to 80% x 90% of its monitor's work area.
fn apply_initial_size(win: &WebviewWindow) -> tauri::Result<()> {
    let monitor = match win.current_monitor()? {
        Some(m) => m,
        None => match win.primary_monitor()? {
            Some(m) => m,
            None => return Ok(()),
        },
    };
    let scale = monitor.scale_factor();
    let area = monitor.work_area();
    let (w, h) = window::initial_window_size(
        f64::from(area.size.width) / scale,
        f64::from(area.size.height) / scale,
    );
    win.set_size(LogicalSize::new(w, h))?;
    win.center()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        // VISIBLE is excluded: the window starts hidden and setup shows it once it is sized.
        .plugin(
            tauri_plugin_window_state::Builder::new()
                .with_state_flags(StateFlags::all() & !StateFlags::VISIBLE)
                .build(),
        )
        .on_window_event(|win, event| tray::handle_window_event(win, event))
        .setup(|app| {
            tray::create(app.handle())?;
            if let Some(win) = app.get_webview_window("main") {
                let state_file = app
                    .path()
                    .app_config_dir()
                    .map(|d| d.join(app.handle().filename()));
                let restored = state_file
                    .map(|f| window::has_saved_state(&f, "main"))
                    .unwrap_or(false);
                let marker = app.path().app_config_dir().ok().map(|d| d.join("window-size-version"));
                let current = marker.as_deref().is_some_and(window::size_version_applied);
                if !restored || !current {
                    if let Err(e) = apply_initial_size(&win) {
                        eprintln!("memdock: initial window sizing failed: {e}");
                    }
                    if let Some(m) = &marker {
                        let _ = m.parent().map(std::fs::create_dir_all);
                        let _ = std::fs::write(m, window::SIZE_VERSION.to_string());
                    }
                }
                // Always show, even if sizing failed.
                let _ = win.show();
            }
            let data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data_dir)?;
            let state = Arc::new(commands::AppState::new(
                paths::AgentPaths::detect(),
                data_dir,
            ));
            let cleanup_state = state.clone();
            std::thread::spawn(move || commands_skills::startup_cleanup(&cleanup_state));
            state.sync.attach(app.handle().clone());
            commands_sync::start_status_watch(&state);
            // Keychain read happens once, off the UI thread.
            commands_sync::warm_token_cache(&state);
            let handle = watcher::start(app.handle().clone(), state.clone());
            let extract = Arc::new(commands_extract::ExtractState::new(state.clone()));
            extract.attach(app.handle().clone());
            extract.start_scheduler();
            let usage = Arc::new(commands_usage::UsageState::new(state.clone()));
            let usage_handle = app.handle().clone();
            usage.start(Arc::new(move |name: &str, payload: serde_json::Value| {
                use tauri::Emitter;
                let _ = usage_handle.emit(name, payload);
            }));
            let env_state = Arc::new(commands_env::EnvState::new(state.clone()));
            let gh_import = Arc::new(commands_ghimport::GhImportState::production(state.clone()));
            app.manage(gh_import);
            // One registry: a native session is driven by one live chat or terminal.
            let native = Arc::new(live::NativeRegistry::new());
            let live_mgr = Arc::new(live::LiveManager::production(app.handle().clone(), native.clone()));
            live_mgr.spawn_reaper();
            app.manage(live_mgr);
            app.manage(Arc::new(pty::PtyManager::production(app.handle().clone(), native.clone())));
            app.manage(native);
            app.manage(usage);
            app.manage(env_state);
            app.manage(Arc::new(commands_sessions::SessionsState::new(state.clone())));
            app.manage(state);
            app.manage(handle);
            app.manage(extract);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_memories,
            commands::refresh_memories,
            commands::read_memory_file,
            commands::write_memory_file,
            commands::search_memories,
            commands::list_snapshots,
            commands::read_snapshot,
            commands::read_codex_db,
            commands::add_manual_project,
            commands::remove_manual_project,
            commands::get_settings,
            commands::update_ui_prefs,
            commands_extract::get_extract_config,
            commands_extract::set_extract_config,
            commands_extract::extract_prompt_defaults,
            commands_extract::extract_runners,
            commands_extract::reset_extract_data,
            commands_extract::start_extraction,
            commands_extract::cancel_extraction,
            commands_extract::list_extract_runs,
            commands_extract::list_suggestions,
            commands_extract::pending_summary,
            commands_extract::apply_suggestion,
            commands_extract::reject_suggestion,
            commands_extract::accept_suggestions,
            commands_extract::suggestion_context,
            commands_skills::list_skills,
            commands_skills::git_available,
            commands_skills::inspect_git_source,
            commands_skills::install_skill,
            commands_skills::check_skill_updates,
            commands_skills::update_skill,
            commands_skills::rollback_skill,
            commands_skills::list_skill_backups,
            commands_skills::delete_skill,
            commands_skills::list_deleted_skill_backups,
            commands_skills::restore_deleted_skill,
            commands_skills::reveal_backup_folder,
            commands_skills::sync_skill,
            commands_skills::register_skill_source,
            commands_skills::list_skill_files,
            commands_skills::read_skill_file,
            commands_skills::read_skill_backup_file,
            commands_skills::write_skill_file,
            commands_skill_manifest::skill_manifest_local,
            commands_skill_manifest::skill_manifest_compare,
            commands_skill_manifest::skill_manifest_export,
            commands_skill_manifest::skill_manifest_import,
            commands_sync::sync_status,
            commands_sync::login_start,
            commands_sync::login_cancel,
            commands_sync::logout,
            commands_sync::set_repo_name,
            commands_sync::ack_public_repo,
            commands_sync::open_verification_url,
            commands_sync::sync_start,
            commands_sync::sync_skipped_files,
            commands_sync::sync_finish,
            commands_sync::sync_discard,
            commands_sync::set_included_local,
            commands_sync::sync_projects,
            commands_sync::sync_history,
            commands_sync::sync_file_at,
            commands_sync::restore_from_history,
            commands_usage::usage_summary,
            commands_usage::usage_quota,
            commands_usage::usage_export_csv,
            commands_usage::set_claude_quota_enabled,
            commands_usage::set_codex_quota_enabled,
            commands_usage::usage_rescan,
            commands_env::env_detect,
            commands_env::env_install,
            commands_env::open_data_dir,
            commands_sessions::list_sessions,
            commands_sessions::session_preview,
            commands_sessions::rename_session,
            commands_sessions::convert_target_dir,
            commands_sessions::convert_session,
            commands_ghimport::gh_import_scan,
            commands_ghimport::gh_import_preview,
            commands_ghimport::gh_import_memories,
            commands_ghimport::gh_import_skill,
            tray::set_tray_labels,
            live::live_start,
            live::live_send,
            live::live_respond,
            live::live_interrupt,
            live::live_stop,
            live::live_close,
            live::live_list,
            live::live_transcript,
            live::live_attach,
            live::native_owner,
            live::live_models,
            live::live_commands,
            live::save_temp_image,
            live::read_image_data_uri,
            project_files::project_files,
            pty::pty_open,
            pty::pty_write,
            pty::pty_resize,
            pty::pty_close,
            worktree::worktree_check,
            worktree::worktree_create,
            worktree::worktree_list,
            worktree::worktree_status,
            worktree::worktree_diff,
            worktree::worktree_merge,
            worktree::worktree_remove,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Reopen { .. } = event {
                tray::show_main(app);
            }
            if let tauri::RunEvent::Exit = event {
                // Stop the scheduler and cancel a running extraction (kills the CLI).
                if let Some(es) = app.try_state::<Arc<commands_extract::ExtractState>>() {
                    es.shutdown();
                }
                if let Some(live) = app.try_state::<Arc<live::LiveManager>>() {
                    live.stop_all();
                }
                if let Some(pty) = app.try_state::<Arc<pty::PtyManager>>() {
                    pty.close_all();
                }
            }
        });
}

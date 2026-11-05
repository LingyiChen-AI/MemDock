pub mod commands;
pub mod commands_skills;
pub mod memory;
pub mod model;
pub mod paths;
pub mod projects;
pub mod settings;
pub mod skills;
pub mod watcher;
pub mod window;

use std::sync::Arc;

use tauri::{LogicalSize, Manager, WebviewWindow};
use tauri_plugin_window_state::{AppHandleExt, StateFlags};

/// Sizes and centers the main window to 70% x 75% of its monitor's work area.
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
        // VISIBLE is excluded: the window starts hidden and setup shows it once it is sized.
        .plugin(
            tauri_plugin_window_state::Builder::new()
                .with_state_flags(StateFlags::all() & !StateFlags::VISIBLE)
                .build(),
        )
        .setup(|app| {
            if let Some(win) = app.get_webview_window("main") {
                let state_file = app
                    .path()
                    .app_config_dir()
                    .map(|d| d.join(app.handle().filename()));
                let restored = state_file
                    .map(|f| window::has_saved_state(&f, "main"))
                    .unwrap_or(false);
                if !restored {
                    if let Err(e) = apply_initial_size(&win) {
                        eprintln!("memdock: initial window sizing failed: {e}");
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
            let handle = watcher::start(app.handle().clone(), state.clone());
            app.manage(state);
            app.manage(handle);
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
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

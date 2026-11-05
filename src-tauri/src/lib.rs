pub mod commands;
pub mod memory;
pub mod model;
pub mod paths;
pub mod projects;
pub mod settings;
pub mod watcher;

use std::sync::Arc;

use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data_dir)?;
            let state = Arc::new(commands::AppState::new(
                paths::AgentPaths::detect(),
                data_dir,
            ));
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
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

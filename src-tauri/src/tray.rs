//! System tray: closing the main window hides it here instead of quitting.

use tauri::menu::{IconMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, Runtime, Window, WindowEvent};

const TRAY_ID: &str = "main";
const SHOW_ID: &str = "show";
const QUIT_ID: &str = "quit";

/// Shows, un-minimizes and focuses the main window.
pub fn show_main<R: Runtime>(app: &AppHandle<R>) {
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.unminimize();
        let _ = win.show();
        let _ = win.set_focus();
    }
}

fn tray_menu<R: Runtime>(app: &AppHandle<R>, show: &str, quit: &str) -> tauri::Result<Menu<R>> {
    let open = IconMenuItem::with_id(app, SHOW_ID, show, true, app.default_window_icon().cloned(), None::<&str>)?;
    let divider = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, QUIT_ID, quit, true, None::<&str>)?;
    Menu::with_items(app, &[&open, &divider, &quit])
}

pub fn create<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    let menu = tray_menu(app, "打开主窗口", "退出 MemDock")?;
    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .tooltip("MemDock")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            SHOW_ID => show_main(app),
            QUIT_ID => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                show_main(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;
    Ok(())
}

/// Relabels the tray menu when the UI language changes.
#[tauri::command]
pub fn set_tray_labels(app: AppHandle, show: String, quit: String) -> Result<(), String> {
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        let menu = tray_menu(&app, &show, &quit).map_err(|e| e.to_string())?;
        tray.set_menu(Some(menu)).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Close button hides the window to the tray; quitting goes through the tray menu.
pub fn handle_window_event<R: Runtime>(win: &Window<R>, event: &WindowEvent) {
    if win.label() != "main" {
        return;
    }
    if let WindowEvent::CloseRequested { api, .. } = event {
        api.prevent_close();
        let _ = win.hide();
    }
}

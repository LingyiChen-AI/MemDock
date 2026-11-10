use tauri::{
    AppHandle, Emitter, Manager, PhysicalPosition, WebviewUrl, WebviewWindow, WebviewWindowBuilder,
};

const PREFIX: &str = "quick-reply-";
const WIDTH: f64 = 384.0;
const HEIGHT: f64 = 218.0;

#[tauri::command]
pub fn open_reply_conversation(popup: WebviewWindow) -> Result<(), String> {
    let live_id = popup
        .label()
        .strip_prefix(PREFIX)
        .ok_or("Not a reply notification")?;
    let app = popup.app_handle();
    app.emit_to("main", "quick-reply-open", live_id)
        .map_err(|e| e.to_string())?;
    crate::tray::show_main(app);
    Ok(())
}

#[tauri::command]
pub async fn show_reply_notification(
    app: AppHandle,
    live_id: String,
    title: String,
    body: String,
    status: String,
    placeholder: String,
    reply_label: String,
    failed_label: String,
    close_label: String,
) -> Result<(), String> {
    let id = uuid::Uuid::parse_str(&live_id).map_err(|e| e.to_string())?;
    let label = format!("{PREFIX}{id}");
    // Preserve an in-progress draft if this chat finishes another queued turn.
    if app.get_webview_window(&label).is_some() {
        return Ok(());
    }
    let cursor = app.cursor_position().map_err(|e| e.to_string())?;
    let monitor = app
        .monitor_from_point(cursor.x, cursor.y)
        .map_err(|e| e.to_string())?
        .or(app.primary_monitor().map_err(|e| e.to_string())?)
        .ok_or("No monitor available")?;
    let area = monitor.work_area();
    let scale = monitor.scale_factor();
    let px = |size: f64| (size * scale).round() as i32;
    let left = area.position.x;
    let top = area.position.y;
    let right = left + area.size.width as i32;
    let bottom = top + area.size.height as i32;
    let x = (right - px(WIDTH + 16.0)).max(left);
    let base_y = bottom - px(HEIGHT + 16.0);
    let step = px(HEIGHT + 8.0);
    let occupied = app
        .webview_windows()
        .into_iter()
        .filter_map(|(label, window)| {
            label
                .starts_with(PREFIX)
                .then(|| window.outer_position().ok())
                .flatten()
        })
        .collect::<Vec<_>>();
    let y = std::iter::successors(Some(base_y), |y| Some(y - step))
        .take_while(|&y| y >= top)
        .find(|&y| {
            !occupied
                .iter()
                .any(|p| (p.x - x).abs() < px(WIDTH) && (p.y - y).abs() < px(HEIGHT))
        })
        .unwrap_or(top);
    let payload = serde_json::json!({
        "liveId": live_id, "title": title, "body": body, "status": status,
        "placeholder": placeholder, "replyLabel": reply_label,
        "failedLabel": failed_label, "closeLabel": close_label,
    });
    let popup = WebviewWindowBuilder::new(
        &app,
        label,
        WebviewUrl::App("index.html?quick-reply".into()),
    )
    .title("MemDock")
    .inner_size(WIDTH, HEIGHT)
    .decorations(false)
    .resizable(false)
    .always_on_top(true)
    .skip_taskbar(true)
    .focused(false)
    .visible(false)
    .initialization_script(format!("window.__memdockQuickReply = {payload};"))
    .build()
    .map_err(|e| e.to_string())?;
    if let Err(error) = popup.set_position(PhysicalPosition::new(x, y)) {
        let _ = popup.destroy();
        return Err(error.to_string());
    }
    popup.show().map_err(|e| e.to_string())
}

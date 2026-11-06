//! First-launch window sizing (design system section 5).

use std::path::Path;

pub const MIN_WIDTH: f64 = 1024.0;
pub const MIN_HEIGHT: f64 = 640.0;

/// Logical size for the very first launch: 80% of the work-area width and 75% of its height,
/// never below the minimum window size, and never larger than the work area itself.
pub fn initial_window_size(work_w: f64, work_h: f64) -> (f64, f64) {
    let w = (work_w * 0.80).round().max(MIN_WIDTH).min(work_w);
    let h = (work_h * 0.75).round().max(MIN_HEIGHT).min(work_h);
    (w, h)
}

/// True when the window-state plugin's file already holds a saved entry for `label`
/// (the plugin writes it on exit, so it exists from the second launch on).
pub fn has_saved_state(state_file: &Path, label: &str) -> bool {
    std::fs::read_to_string(state_file)
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .map(|v| v.get(label).is_some_and(|e| e.get("width").and_then(|w| w.as_u64()).unwrap_or(0) > 0))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_window_is_80pct_width_75pct_height() {
        assert_eq!(initial_window_size(2560.0, 1400.0), (2048.0, 1050.0));
        assert_eq!(initial_window_size(1920.0, 1040.0), (1536.0, 780.0));
    }

    #[test]
    fn initial_window_respects_minimum() {
        assert_eq!(initial_window_size(1280.0, 720.0), (1024.0, 640.0));
        assert_eq!(initial_window_size(1366.0, 728.0), (1093.0, 640.0));
    }

    #[test]
    fn saved_state_detection() {
        let tmp = tempfile::tempdir().unwrap();
        let f = tmp.path().join(".window-state.json");
        assert!(!has_saved_state(&f, "main"));
        std::fs::write(&f, "not json").unwrap();
        assert!(!has_saved_state(&f, "main"));
        std::fs::write(&f, r#"{"main":{"width":0,"height":0}}"#).unwrap();
        assert!(!has_saved_state(&f, "main"));
        std::fs::write(&f, r#"{"main":{"width":1792,"height":1050}}"#).unwrap();
        assert!(has_saved_state(&f, "main"));
        assert!(!has_saved_state(&f, "other"));
    }

    #[test]
    fn initial_window_never_exceeds_work_area() {
        assert_eq!(initial_window_size(800.0, 500.0), (800.0, 500.0));
    }
}

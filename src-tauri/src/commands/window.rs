//! ===== 命令分组 ③：🪟 窗口管理/贴边/桌面插件框/剪贴板 =====
//! 包含：剪贴板定时清空、显示/隐藏窗口、窗口位置/大小/置顶/鼠标穿透、迷你模式、Widget 配置读写与应用

use tauri::{AppHandle, Manager, State, Window};

use crate::db::Db;
use crate::AppState;

#[cfg(windows)]
use crate::win_dock;

// ============================================================
// 📋 剪贴板：带倒计时清空（只有剪贴板内容仍是我们写的原文时才清空）
// ============================================================
#[tauri::command]
pub async fn copy_to_clipboard_with_timeout(
    app: AppHandle,
    text: String,
    timeout_secs: u64,
) -> Result<(), String> {
    use tauri_plugin_clipboard_manager::ClipboardExt;
    let clipboard = app.clipboard();
    clipboard.write_text(text.clone()).map_err(|e| e.to_string())?;

    let app_clone = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(timeout_secs)).await;
        if let Ok(current) = app_clone.clipboard().read_text() {
            if current == text {
                let _ = app_clone.clipboard().write_text(String::new());
            }
        }
    });

    Ok(())
}

// ============================================================
// 🪟 切换窗口显示/隐藏
// ============================================================
#[tauri::command]
pub fn toggle_window(window: Window) {
    if window.is_visible().unwrap_or(false) {
        let _ = window.hide();
    } else {
        let _ = window.show();
        let _ = window.set_focus();
    }
}

// ============================================================
// 🪟 显示窗口（贴边隐藏的先复位）
// ============================================================
#[tauri::command]
pub fn show_window(app: AppHandle) {
    #[cfg(windows)]
    {
        if let Some(window) = app.get_webview_window("main") {
            if win_dock::is_hidden() {
                let _ = win_dock::force_reveal(&window);
            }
        }
    }
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.set_focus();
    }
}

// ============================================================
// 🪟 隐藏窗口
// ============================================================
#[tauri::command]
pub fn hide_window(window: Window) {
    let _ = window.hide();
}

// ============================================================
// 🪟 保存 Widget 配置到 settings 表（key 会加 widget. 前缀）
// ============================================================
#[tauri::command]
pub fn save_widget_config(
    state: State<AppState>,
    key: String,
    value: String,
) -> Result<(), String> {
    let db = state.db.lock().unwrap();
    db.set_setting(&format!("widget.{}", key), &value)
        .map_err(|e| e.to_string())
}

// ============================================================
// 🪟 读取 Widget 配置
// ============================================================
#[tauri::command]
pub fn load_widget_config(state: State<AppState>, key: String) -> Result<Option<String>, String> {
    eprintln!("[IPC] load_widget_config({})", key);
    let db = state.db.lock().unwrap();
    let r = db.get_setting(&format!("widget.{}", key)).map_err(|e| e.to_string());
    eprintln!("[IPC] load_widget_config({}) -> {:?}", key, r);
    r
}

// ============================================================
// 🪟 获取当前窗口位置
// ============================================================
#[tauri::command]
pub fn get_window_position(window: Window) -> Result<(f64, f64), String> {
    let pos = window.outer_position().map_err(|e| e.to_string())?;
    Ok((pos.x as f64, pos.y as f64))
}

// ============================================================
// 🪟 获取当前窗口大小
// ============================================================
#[tauri::command]
pub fn get_window_size(window: Window) -> Result<(f64, f64), String> {
    let size = window.outer_size().map_err(|e| e.to_string())?;
    Ok((size.width as f64, size.height as f64))
}

// ============================================================
// 🪟 设置窗口位置（PhysicalPosition）
// ============================================================
#[tauri::command]
pub fn set_window_position(window: Window, x: f64, y: f64) -> Result<(), String> {
    window
        .set_position(tauri::PhysicalPosition::new(x as i32, y as i32))
        .map_err(|e| e.to_string())
}

// ============================================================
// 🪟 设置窗口大小（PhysicalSize）
// ============================================================
#[tauri::command]
pub fn set_window_size(window: Window, width: f64, height: f64) -> Result<(), String> {
    window
        .set_size(tauri::PhysicalSize::new(width as u32, height as u32))
        .map_err(|e| e.to_string())
}

// ============================================================
// 🪟 切换置顶
// ============================================================
#[tauri::command]
pub fn set_always_on_top(window: Window, on_top: bool) -> Result<(), String> {
    window.set_always_on_top(on_top).map_err(|e| e.to_string())
}

// ============================================================
// 🪟 切换鼠标穿透（透明区点击落到桌面）
// ============================================================
#[tauri::command]
pub fn set_ignore_cursor_events(window: Window, ignore: bool) -> Result<(), String> {
    window.set_ignore_cursor_events(ignore).map_err(|e| e.to_string())
}

// ============================================================
// 🪟 切换迷你模式（极简图标列 64×360 / 默认 480×720）
// ============================================================
#[tauri::command]
pub fn set_mini_mode(window: Window, mini: bool) -> Result<(), String> {
    if mini {
        window
            .set_size(tauri::PhysicalSize::new(64, 360))
            .map_err(|e| e.to_string())?;
    } else {
        window
            .set_size(tauri::PhysicalSize::new(480, 720))
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

// ============================================================
// 🪟 迷你模式 + 同步位置（保持原 X，迷你时往下偏移 140）
// ============================================================
#[tauri::command]
pub fn set_mini_mode_with_pos(
    window: Window,
    state: State<AppState>,
    mini: bool,
) -> Result<(), String> {
    let (x, y) = {
        let db = state.db.lock().unwrap();
        let x = db
            .get_setting("widget.x")
            .ok()
            .flatten()
            .and_then(|s| s.parse::<f64>().ok())
            .unwrap_or(100.0) as i32;
        let y = db
            .get_setting("widget.y")
            .ok()
            .flatten()
            .and_then(|s| s.parse::<f64>().ok())
            .unwrap_or(100.0) as i32;
        (x, y)
    };

    if mini {
        let _ = window.set_size(tauri::PhysicalSize::new(64, 360));
        let _ = window.set_position(tauri::PhysicalPosition::new(x, y + 140));
    } else {
        // 关键修复：完整模式只 setSize，不改位置（否则会"跳回最初位置"）
        let _ = window.set_size(tauri::PhysicalSize::new(480, 720));
    }
    Ok(())
}

// ============================================================
// 🪟 启动时应用已保存的 Widget 配置（位置/大小/置顶），带边界保护
// ============================================================
#[tauri::command]
pub fn apply_widget_config(window: Window, state: State<AppState>) -> Result<(), String> {
    eprintln!("[IPC] apply_widget_config ENTER");
    let db = state.db.lock().unwrap();
    if let Some(s) = db.get_setting("widget.always_on_top").map_err(|e| e.to_string())? {
        let on_top = s == "true";
        let _ = window.set_always_on_top(on_top);
    }
    // 读取主显示器尺寸，用于边界校验
    let (mon_w, mon_h) = window
        .primary_monitor()
        .ok()
        .flatten()
        .map(|m| {
            let sf = m.scale_factor();
            (
                (m.size().width as f64 / sf) as i32,
                (m.size().height as f64 / sf) as i32,
            )
        })
        .unwrap_or((1920, 1080));
    if let (Some(x), Some(y)) = (
        db.get_setting("widget.x").map_err(|e| e.to_string())?,
        db.get_setting("widget.y").map_err(|e| e.to_string())?,
    ) {
        if let (Ok(mut x), Ok(mut y)) = (x.parse::<i32>(), y.parse::<i32>()) {
            // 边界保护：把窗口拉回主屏可见区域（保留至少 200×200 可见）
            let win_w = 480i32;
            x = x.clamp(-(win_w - 200), mon_w - 200);
            y = y.clamp(0, mon_h - 200);
            let _ = window.set_position(tauri::PhysicalPosition::new(x, y));
        }
    }
    if let (Some(w), Some(h)) = (
        db.get_setting("widget.width").map_err(|e| e.to_string())?,
        db.get_setting("widget.height").map_err(|e| e.to_string())?,
    ) {
        if let (Ok(w), Ok(h)) = (w.parse::<f64>(), h.parse::<f64>()) {
            // 尺寸限制：320~1400 × 400~1100（与 tauri.conf.json 对齐）
            let max_w = (mon_w as f64).min(1400.0);
            let max_h = (mon_h as f64).min(1100.0);
            let w = w.clamp(320.0, max_w);
            let h = h.clamp(400.0, max_h);
            let _ = window.set_size(tauri::PhysicalSize::new(w as u32, h as u32));
        }
    }
    Ok(())
}

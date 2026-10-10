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
    clipboard
        .write_text(text.clone())
        .map_err(|e| e.to_string())?;

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
    // [data-gate:shared] Phase 2C-3 统一数据操作门（锁顺序: gate → master_wrap_gate → db → conn）
    let _data_gate = state.data_read()?;

    let db = state.db.lock().unwrap();
    db.set_setting(&format!("widget.{}", key), &value)
        .map_err(|e| e.to_string())
}

// ============================================================
// 🪟 读取 Widget 配置
// ============================================================
#[tauri::command]
pub fn load_widget_config(state: State<AppState>, key: String) -> Result<Option<String>, String> {
    // [data-gate:shared] Phase 2C-3 统一数据操作门（锁顺序: gate → master_wrap_gate → db → conn）
    let _data_gate = state.data_read()?;

    eprintln!("[IPC] load_widget_config({})", key);
    let db = state.db.lock().unwrap();
    let r = db
        .get_setting(&format!("widget.{}", key))
        .map_err(|e| e.to_string());
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
    window
        .set_ignore_cursor_events(ignore)
        .map_err(|e| e.to_string())
}

// ============================================================
// 🪟 切换迷你模式（极简图标列 64×360 逻辑像素 / 默认 480×720）
// R3：进入迷你前解除最小尺寸约束（64 < minWidth 320 逻辑px，否则会被钳制），
//     尺寸用逻辑像素保证不同 DPI 下迷你列视觉一致。
// ============================================================
#[tauri::command]
pub fn set_mini_mode(window: Window, mini: bool) -> Result<(), String> {
    if mini {
        let _ = window.set_min_size::<tauri::LogicalSize<f64>>(None);
        let _ = window.set_size(tauri::LogicalSize::new(64.0, 360.0));
    } else {
        let _ = window.set_min_size(Some(tauri::LogicalSize::new(320.0, 400.0)));
        let _ = window.set_size(tauri::LogicalSize::new(480.0, 720.0));
    }
    Ok(())
}

// ============================================================
// 🪟 迷你模式 + 几何隔离（R3，v0.4.1 D-Repair）
//
// 状态迁移规则（完整模式几何 = widget.x/y/width/height，迷你几何 = widget.mini_x/mini_y）：
// - 进入迷你：解除 min 约束 → 逻辑 64×360 → 位置取独立保存的迷你位置，
//   无记录时回退 完整位置下方 140px（兼容旧版行为）；【不写完整几何】
// - 退出迷你：先恢复 min 约束（320×400 逻辑px，与 tauri.conf.json 对齐）
//   → 恢复用户原有完整几何（保存过的位置+尺寸；缺省项保持当前值），【不强制默认值】
// - 前端在切换期间抑制几何保存（geometryTransition），move/resize 事件不污染配置；
//   迷你态下的移动由前端路由到 widget.mini_x/mini_y，同样不触碰完整几何快照。
// ============================================================
#[tauri::command]
pub fn set_mini_mode_with_pos(
    window: Window,
    state: State<AppState>,
    mini: bool,
) -> Result<(), String> {
    // [data-gate:shared] Phase 2C-3 统一数据操作门（锁顺序: gate → master_wrap_gate → db → conn）
    let _data_gate = state.data_read()?;

    // 统一读取设置（缺省 None，不瞎填默认位置）
    let read_setting = |key: &str| -> Option<i32> {
        let db = state.db.lock().unwrap();
        db.get_setting(key)
            .ok()
            .flatten()
            .and_then(|s| s.parse::<i32>().ok())
    };

    if mini {
        // ⓪ 放行检查修正：先把"当前完整窗口的真实几何"可靠快照进 settings。
        // 覆盖三类场景：数据库尚无几何记录的首次切换 / 用户已自定义尺寸
        //（含仅 resize 未 move，tauri://move 监听器不触发）/ 任何时点切换。
        // 必须在缩小窗口之前读取（缩完读到的就是迷你几何）。快速连续切换也
        // 安全：每次进入时读到的都是上一轮退出恢复出的完整几何，快照值收敛。
        if let (Ok(pos), Ok(size)) = (window.outer_position(), window.outer_size()) {
            let db = state.db.lock().unwrap();
            let _ = db.snapshot_full_geometry(pos.x, pos.y, size.width, size.height);
        }
        // ① 先解除最小尺寸约束，再缩小（64×360 逻辑px 低于 minWidth 320）
        let _ = window.set_min_size::<tauri::LogicalSize<f64>>(None);
        let _ = window.set_size(tauri::LogicalSize::new(64.0, 360.0));
        // ② 位置：优先独立迷你位置；无记录回退完整位置下方 140px（旧行为）
        let (mx, my) = (read_setting("widget.mini_x"), read_setting("widget.mini_y"));
        let (x, y) = match (mx, my) {
            (Some(x), Some(y)) => (x, y),
            _ => {
                let fx = read_setting("widget.x").unwrap_or(100);
                let fy = read_setting("widget.y").unwrap_or(100);
                (fx, fy + 140)
            }
        };
        let _ = window.set_position(tauri::PhysicalPosition::new(x, y));
    } else {
        // ① 先恢复 min 约束（逻辑 320×400，与 tauri.conf.json 对齐）
        let _ = window.set_min_size(Some(tauri::LogicalSize::new(320.0, 400.0)));
        // ② 恢复用户原有完整几何：有保存值才设置，缺省项保持当前值（不强制默认）
        let (x, y) = (read_setting("widget.x"), read_setting("widget.y"));
        if let (Some(x), Some(y)) = (x, y) {
            let _ = window.set_position(tauri::PhysicalPosition::new(x, y));
        }
        let (w, h) = (read_setting("widget.width"), read_setting("widget.height"));
        if let (Some(w), Some(h)) = (w, h) {
            // 尺寸下限与 min 约束一致（物理像素下限按 100% 基准，多余部分由窗口管理器兜底）
            let w = w.max(320) as f64;
            let h = h.max(400) as f64;
            let _ = window.set_size(tauri::PhysicalSize::new(w as u32, h as u32));
        }
    }
    Ok(())
}

// ============================================================
// 🪟 启动时应用已保存的 Widget 配置（位置/大小/置顶），带边界保护
// ============================================================
#[tauri::command]
pub fn apply_widget_config(window: Window, state: State<AppState>) -> Result<(), String> {
    // [data-gate:shared] Phase 2C-3 统一数据操作门（锁顺序: gate → master_wrap_gate → db → conn）
    let _data_gate = state.data_read()?;

    eprintln!("[IPC] apply_widget_config ENTER");
    let db = state.db.lock().unwrap();
    if let Some(s) = db
        .get_setting("widget.always_on_top")
        .map_err(|e| e.to_string())?
    {
        let on_top = s == "true";
        let _ = window.set_always_on_top(on_top);
    }
    // 读取主显示器尺寸，用于边界校验。
    // v0.4.1 修复：widget.x/y/width/height 保存的都是物理像素
    //（来自 outer_position/outer_size），旧实现把显示器尺寸除以
    // scale_factor 折算成逻辑像素再做 clamp，125%/150% 缩放下两套
    // 坐标系混用导致窗口位置恢复偏移。现在全程物理像素。
    let (mon_w, mon_h) = window
        .primary_monitor()
        .ok()
        .flatten()
        .map(|m| (m.size().width as i32, m.size().height as i32))
        .unwrap_or((1920, 1080));
    // 先解析保存的尺寸（缺省 480×720），供位置 clamp 使用真实窗口宽度
    let saved_size: Option<(f64, f64)> = {
        match (
            db.get_setting("widget.width").map_err(|e| e.to_string())?,
            db.get_setting("widget.height").map_err(|e| e.to_string())?,
        ) {
            (Some(w), Some(h)) => match (w.parse::<f64>(), h.parse::<f64>()) {
                (Ok(w), Ok(h)) => Some((w, h)),
                _ => None,
            },
            _ => None,
        }
    };
    let (cur_w, _cur_h) = saved_size.unwrap_or((480.0, 720.0));
    if let (Some(x), Some(y)) = (
        db.get_setting("widget.x").map_err(|e| e.to_string())?,
        db.get_setting("widget.y").map_err(|e| e.to_string())?,
    ) {
        if let (Ok(mut x), Ok(mut y)) = (x.parse::<i32>(), y.parse::<i32>()) {
            // 边界保护：把窗口拉回主屏可见区域（保留至少 200×200 可见）
            let win_w = (cur_w as i32).max(200);
            x = x.clamp(-(win_w - 200), mon_w - 200);
            y = y.clamp(0, mon_h - 200);
            let _ = window.set_position(tauri::PhysicalPosition::new(x, y));
        }
    }
    if let Some((w, h)) = saved_size {
        // 尺寸限制：320~1400 × 400~1100（与 tauri.conf.json 对齐）
        let max_w = (mon_w as f64).min(1400.0);
        let max_h = (mon_h as f64).min(1100.0);
        let w = w.clamp(320.0, max_w);
        let h = h.clamp(400.0, max_h);
        let _ = window.set_size(tauri::PhysicalSize::new(w as u32, h as u32));
    }
    Ok(())
}

// ============================================================
// 🪟 应用版本形态（v0.4.1 验收便携测试版："测试版"徽章依据）
// ============================================================
#[tauri::command]
pub fn get_app_edition() -> Result<String, String> {
    Ok(
        if crate::data_root::is_portable_test_edition() {
            "acceptance".to_string()
        } else {
            "standard".to_string()
        },
    )
}

//! Windows 贴边自动隐藏（参考 QQ、Yue Launcher、Second Desk 等优秀应用）
//!
//! 核心思路（来自 CSDN 详解）：
//! 1. 拦截 WM_WINDOWPOSCHANGING 在窗口位置变更前预判是否贴边
//! 2. 用 MonitorFromPoint + GetMonitorInfo 拿窗口所在显示器的工作区（多屏适配）
//! 3. 贴边后用 SetTimer(16ms) 驱动 SetWindowPos 动画，将窗口压缩至 6px 触角
//! 4. 鼠标重新进入触角区域（通过 WM_MOUSEMOVE + 边界检测）反向展开
//! 5. 不在前端做轮询：在 Rust 端做最可靠（不依赖 WebView 收消息）

#![cfg(windows)]

use std::sync::{Mutex, OnceLock};
use std::time::Instant;

use tauri::{PhysicalPosition, PhysicalSize, WebviewWindow};
use windows_sys::Win32::Foundation::{
    GetLastError, HWND, LPARAM, LRESULT, POINT, RECT, SetLastError, WPARAM,
};
use windows_sys::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MonitorFromPoint, MONITOR_DEFAULTTONEAREST, MONITORINFO,
};
use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallWindowProcW, GetAncestor, GetCursorPos, GetWindowRect, SetTimer, SetWindowLongPtrW,
    SetWindowPos, GA_ROOT, GWLP_WNDPROC, HWND_TOP, SWP_NOACTIVATE, SWP_NOZORDER, SWP_NOSENDCHANGING,
    WNDPROC,
};

const WM_WINDOWPOSCHANGING: u32 = 0x0046;
const WM_MOUSEMOVE: u32 = 0x0200;
const WM_MOVE: u32 = 0x0003; // 窗口位置变化后触发（OS 模态拖动期间也会触发）
const WM_EXITSIZEMOVE: u32 = 0x0232; // 拖动/缩放结束触发一次

const TICK_MS: u32 = 32; // 已废弃，保留仅为兼容（不再启动 SetTimer）
const HIDE_SLIVER_PX: i32 = 6; // 隐藏后保留 6px 触角
const REVEAL_TRIGGER_PX: i32 = 12; // 鼠标在触角 +12px 范围内即唤回
const AUTO_HIDE_DELAY_MS: u64 = 200;
const SNAP_THRESHOLD_PX: i32 = 20;
// 修复 P0-#X：节流常量
// WM_WINDOWPOSCHANGING 在拖动期间每秒触发 60+ 次，不加节流会卡死主线程
const WND_PROC_THROTTLE_MS: u128 = 80;
const MOUSE_MOVE_THROTTLE_MS: u128 = 60;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Side {
    Left,
    Right,
    Top,
    Bottom,
    None,
}

struct State {
    hwnd: HWND,
    width: i32,
    height: i32,
    side: Side,
    pre_hide_x: i32,
    pre_hide_y: i32,
    pre_hide_w: i32,
    pre_hide_h: i32,
    enabled: bool,
    hidden: bool,
    animation_target_x: i32,
    animation_target_y: i32,
    animation_target_w: i32,
    animation_target_h: i32,
    animation_from_x: i32,
    animation_from_y: i32,
    animation_from_w: i32,
    animation_from_h: i32,
    animation_start: Option<Instant>,
    animation_duration_ms: u32,
    is_animating: bool,
    pending_hide_at: Option<Instant>,
    // 修复 P0-#X：节流 + 缓存
    /// 上次处理 WM_WINDOWPOSCHANGING 的时间（用于节流）
    last_wndproc_at: Instant,
    /// 上次处理 WM_MOUSEMOVE 的时间（用于节流）
    last_mousemove_at: Instant,
    /// 缓存的工作区（避免每次 GetMonitorInfoW）
    cached_work: RECT,
    /// 缓存的 monitor 句柄（变化时说明窗口被拖到别的屏幕了）
    cached_hmonitor: isize,
}

// 修复 P0-#5：HWND（*mut c_void）默认不 Send，但 widget 是单实例
// 跨线程仅读/同步访问不会出问题，这里手动断言 Send + Sync。
// 原因：Dock 只有一个窗口，install 一次后所有线程（GUI/定时器/window proc）都引用同一个 HWND。
unsafe impl Send for State {}
unsafe impl Sync for State {}

// 全局：当前唯一的 dock state（widget 是单例）
// 修复 P0-#5：使用 OnceLock 替代 static mut，避免跨线程裸读 = UB。
// 同时语义更清晰：只装一次，后续访问通过 .get() 拿引用。
static DOCK_STATE: OnceLock<Mutex<State>> = OnceLock::new();
static ORIGINAL_WNDPROC: OnceLock<WNDPROC> = OnceLock::new();

/// 内部：根据 hwnd 重新查询 monitor 和 work area，返回 (hmonitor, work_area)
/// 修复 P0-#X：原本用 zeroed() 的 RECT 作为缓存默认值，导致 `detect_side` 第一次比较
/// `win.left <= work.left + 20` 时 work.left = 0，永远触发 Left 隐藏 bug。
/// 正确做法：在 install() 时调用本函数，填入真实的 RECT。
unsafe fn query_work_area(hwnd: HWND) -> (isize, RECT) {
    let mut rect: RECT = std::mem::zeroed();
    GetWindowRect(hwnd, &mut rect);
    let mut pt: POINT = std::mem::zeroed();
    pt.x = (rect.left + rect.right) / 2;
    pt.y = (rect.top + rect.bottom) / 2;
    let hmonitor = MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST) as isize;
    let mut info: MONITORINFO = std::mem::zeroed();
    info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
    GetMonitorInfoW(hmonitor as _, &mut info);
    let _ = GetDpiForWindow(hwnd);
    (hmonitor, info.rcWork)
}

fn detect_side(work: RECT, win: RECT) -> Side {
    let threshold = SNAP_THRESHOLD_PX;
    if win.left <= work.left + threshold {
        Side::Left
    } else if win.right >= work.right - threshold {
        Side::Right
    } else if win.top <= work.top + threshold {
        Side::Top
    } else if win.bottom >= work.bottom - threshold {
        Side::Bottom
    } else {
        Side::None
    }
}

fn compute_hidden_rect(side: Side, pre: RECT, work: RECT) -> RECT {
    let mut r = pre;
    match side {
        Side::Left => {
            r.right = work.left + HIDE_SLIVER_PX;
            r.left = work.left;
        }
        Side::Right => {
            r.left = work.right - HIDE_SLIVER_PX;
            r.right = work.right;
        }
        Side::Top => {
            r.bottom = work.top + HIDE_SLIVER_PX;
            r.top = work.top;
        }
        Side::Bottom => {
            r.top = work.bottom - HIDE_SLIVER_PX;
            r.bottom = work.bottom;
        }
        Side::None => {}
    }
    r
}

unsafe fn apply_window_pos(state: &State, rect: RECT) {
    let w = (rect.right - rect.left).max(1);
    let h = (rect.bottom - rect.top).max(1);
    SetWindowPos(
        state.hwnd,
        HWND_TOP,
        rect.left,
        rect.top,
        w,
        h,
        // 修复 P1-#14：加 SWP_NOSENDCHANGING 防止 apply_window_pos 触发的
        // SetWindowPos 再次发 WM_WINDOWPOSCHANGING，导致隐藏↔恢复死循环。
        SWP_NOACTIVATE | SWP_NOZORDER | SWP_NOSENDCHANGING,
    );
}

/// 安全访问全局 DOCK_STATE（必须先 install）
fn with_state<F, R>(f: F) -> Option<R>
where
    F: FnOnce(&mut State) -> R,
{
    if let Some(mutex) = DOCK_STATE.get() {
        if let Ok(mut s) = mutex.lock() {
            return Some(f(&mut s));
        }
    }
    None
}

/// 触发隐藏（不抢主线程时间片）
/// 用 std::thread::spawn 而非 SetTimer：
/// - SetTimer(32ms) 会让 OS 每 32ms 强制中断主线程 → 拖动期间主线程被切来切去 → "未响应"
/// - thread::spawn sleep 200ms 后只发一次 PostMessage，主线程只在消息到达时处理一次
/// 修复 P0-#X：HWND 是 *mut c_void 不 Send，转换成 usize（u64 on 64-bit Windows）
unsafe fn schedule_hide(hwnd: HWND) {
    let hwnd_addr = hwnd as usize; // u64 永远 Send + 'static
    eprintln!("[win_dock] schedule_hide 触发, hwnd={:#x}", hwnd_addr);
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(AUTO_HIDE_DELAY_MS));
        // 在 hide 之前再检查一次"是否还贴边"（防止 200ms 内用户已经拖回中间）
        let hwnd = hwnd_addr as HWND;
        let (still_at_edge, side_dbg) = with_state(|state| -> (bool, String) {
            if state.hidden { return (false, "already_hidden".to_string()); }
            let mut cur: RECT = std::mem::zeroed();
            GetWindowRect(hwnd, &mut cur);
            let s = detect_side(state.cached_work, cur);
            (s != Side::None, format!("side={:?} cur=({},{},{},{})", s, cur.left, cur.top, cur.right, cur.bottom))
        }).unwrap_or((false, "no_state".to_string()));
        eprintln!("[win_dock] 200ms 后检查: still_at_edge={} ({})", still_at_edge, side_dbg);
        if still_at_edge {
            // 用 PostMessageW 发到主线程触发隐藏
            extern "system" {
                fn PostMessageW(hwnd: HWND, msg: u32, wparam: usize, lparam: isize) -> i32;
            }
            let r = PostMessageW(hwnd, WM_DOCK_HIDE, 0, 0);
            eprintln!("[win_dock] PostMessageW(WM_DOCK_HIDE) = {}", r);
        }
    });
}

// 自定义消息：贴边 200ms 后触发隐藏
const WM_DOCK_HIDE: u32 = 0x0400 + 1; // WM_USER 范围

unsafe extern "system" fn dock_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // 先调原 WndProc
    let result = if let Some(original) = ORIGINAL_WNDPROC.get() {
        CallWindowProcW(*original, hwnd, msg, wparam, lparam)
    } else {
        0
    };

    // 调试：每次进入 WndProc 都打（确认子类化生效）
    // 只打非高频消息（避免 WM_MOUSEMOVE 等每秒几十条日志）
    if msg == WM_MOVE || msg == WM_WINDOWPOSCHANGING || msg == WM_EXITSIZEMOVE || msg == WM_DOCK_HIDE
        || msg == 0x0001 /* WM_CREATE */ || msg == 0x0002 /* WM_DESTROY */
        || msg == 0x0005 /* WM_SIZE */
    {
        eprintln!("[win_dock] WndProc 收到 msg=0x{:04x}", msg);
    }

    with_state(|state| {
        if !state.enabled {
            return;
        }

        // 1) 自定义消息：200ms 延迟到期后真正执行隐藏
        if msg == WM_DOCK_HIDE {
            eprintln!("[win_dock] 收到 WM_DOCK_HIDE，开始隐藏");
            if !state.hidden {
                let mut cur: RECT = std::mem::zeroed();
                GetWindowRect(hwnd, &mut cur);
                let side = detect_side(state.cached_work, cur);
                if side != Side::None {
                    state.side = side;
                    state.pre_hide_x = cur.left;
                    state.pre_hide_y = cur.top;
                    state.pre_hide_w = cur.right - cur.left;
                    state.pre_hide_h = cur.bottom - cur.top;
                    let hidden_rect = compute_hidden_rect(side, cur, state.cached_work);
                    apply_window_pos(state, hidden_rect); // 一次到位，无动画
                    state.hidden = true;
                    eprintln!("[win_dock] 已隐藏, side={:?} cur=({},{},{},{}) → hidden=({},{},{},{})",
                        side, cur.left, cur.top, cur.right, cur.bottom,
                        hidden_rect.left, hidden_rect.top, hidden_rect.right, hidden_rect.bottom);
                }
            }
            return;
        }

        // 2) WM_MOVE / WM_WINDOWPOSCHANGING：节流 + detect_side
        //    修复 P0-#X：取消 SetTimer 持续触发，改用 thread::spawn + PostMessage
        //    之前 SetTimer(32ms) 让 OS 每 32ms 强制中断主线程 → 拖动期间 "未响应"
        //    修复 P0-#Y：同时监听 WM_MOVE（OS 模态拖动期间也会触发 WM_MOVE）
        //    之前只监听 WM_WINDOWPOSCHANGING，但 OS 模态拖动时 WM_WINDOWPOSCHANGING
        //    不发到 WndProc（OS 内部处理）→ 拖动结束后才发 1-2 次 → schedule_hide 错过时机
        if msg == WM_MOVE || msg == WM_WINDOWPOSCHANGING {
            let now = Instant::now();
            let since_last = now.duration_since(state.last_wndproc_at).as_millis();
            if since_last < WND_PROC_THROTTLE_MS {
                eprintln!("[win_dock] WM_{} 节流丢弃 (since_last={}ms)",
                    if msg == WM_MOVE { "MOVE" } else { "WINDOWPOSCHANGING" }, since_last);
                return;
            }
            state.last_wndproc_at = now;

            let mut cur: RECT = std::mem::zeroed();
            GetWindowRect(hwnd, &mut cur);
            let side = detect_side(state.cached_work, cur);
            eprintln!("[win_dock] WM_{} cur=({},{},{},{}) side={:?} hidden={}",
                if msg == WM_MOVE { "MOVE" } else { "WINDOWPOSCHANGING" },
                cur.left, cur.top, cur.right, cur.bottom, side, state.hidden);

            if side == Side::None {
                // 离开边缘：直接恢复（无动画）
                if state.hidden {
                    state.hidden = false;
                    let restore = RECT {
                        left: state.pre_hide_x,
                        top: state.pre_hide_y,
                        right: state.pre_hide_x + state.pre_hide_w,
                        bottom: state.pre_hide_y + state.pre_hide_h,
                    };
                    apply_window_pos(state, restore);
                }
                state.pending_hide_at = None;
            } else {
                // 贴边：调度延迟隐藏（不抢主线程）
                if !state.hidden {
                    schedule_hide(hwnd);
                }
            }
        }

        // 3) WM_MOUSEMOVE：节流 + 触角检测
        if msg == WM_MOUSEMOVE && state.hidden {
            let now = Instant::now();
            if now.duration_since(state.last_mousemove_at).as_millis() < MOUSE_MOVE_THROTTLE_MS {
                return;
            }
            state.last_mousemove_at = now;

            let mut cur: RECT = std::mem::zeroed();
            GetWindowRect(hwnd, &mut cur);
            let mut cursor: POINT = std::mem::zeroed();
            GetCursorPos(&mut cursor);

            let near = match state.side {
                Side::Left => {
                    cursor.x <= cur.left + REVEAL_TRIGGER_PX
                        && cursor.y >= cur.top
                        && cursor.y <= cur.bottom
                }
                Side::Right => {
                    cursor.x >= cur.right - REVEAL_TRIGGER_PX
                        && cursor.y >= cur.top
                        && cursor.y <= cur.bottom
                }
                Side::Top => {
                    cursor.y <= cur.top + REVEAL_TRIGGER_PX
                        && cursor.x >= cur.left
                        && cursor.x <= cur.right
                }
                Side::Bottom => {
                    cursor.y >= cur.bottom - REVEAL_TRIGGER_PX
                        && cursor.x >= cur.left
                        && cursor.x <= cur.right
                }
                Side::None => false,
            };
            if near {
                state.hidden = false;
                state.pending_hide_at = None;
                let restore = RECT {
                    left: state.pre_hide_x,
                    top: state.pre_hide_y,
                    right: state.pre_hide_x + state.pre_hide_w,
                    bottom: state.pre_hide_y + state.pre_hide_h,
                };
                apply_window_pos(state, restore); // 一次到位，无动画
            }
        }
    });
    result
}

/// 安装 Win32 贴边自动隐藏到指定窗口
pub fn install(window: &WebviewWindow) -> Result<(), String> {
    unsafe {
        let webview_hwnd = window.hwnd().map_err(|e| e.to_string())?.0 as isize;
        // 修复 P0-#Z：Tauri 2 的 webview 是 child window
        // window.hwnd() 返回的是 webview 子窗口，OS 拖动消息发给**顶层父窗口**！
        // 之前子类化 webview 子窗口 → WndProc 收不到任何拖动消息 → 贴边自动隐藏永远不生效
        // 用 GetAncestor(hwnd, GA_ROOT) 拿真正的顶层父窗口来子类化
        let hwnd = GetAncestor(webview_hwnd as HWND, GA_ROOT) as HWND;
        eprintln!("[win_dock] install: webview_hwnd={:#x} → root_hwnd={:#x}", webview_hwnd, hwnd as isize);
        // 修复 P1-#10：outer_size/outer_position 返回的是物理像素（与 HWND 坐标一致），
        // 之前错误地除以 scale_factor 转成逻辑像素，导致与 GetWindowRect / SetWindowPos 不匹配，
        // 高 DPI + 多屏下窗口会偏移 1-2 倍。这里全部统一用物理像素。
        let size = window.outer_size().map_err(|e| e.to_string())?;
        let pos = window.outer_position().map_err(|e| e.to_string())?;
        let w = size.width as i32;
        let h = size.height as i32;
        let x = pos.x as i32;
        let y = pos.y as i32;

        let state = State {
            hwnd,
            width: w,
            height: h,
            side: Side::None,
            pre_hide_x: x,
            pre_hide_y: y,
            pre_hide_w: w,
            pre_hide_h: h,
            enabled: true, // 修复 P0-#X 验证完毕（无 SetTimer 后不卡），恢复启用
            hidden: false,
            animation_target_x: x,
            animation_target_y: y,
            animation_target_w: w,
            animation_target_h: h,
            animation_from_x: x,
            animation_from_y: y,
            animation_from_w: w,
            animation_from_h: h,
            animation_start: None,
            animation_duration_ms: 180,
            is_animating: false,
            pending_hide_at: None,
            // 修复 P0-#X：节流 + 缓存初始化
            // last_wndproc_at 初始化为"很久以前"，保证 install 后第一次 WM_WINDOWPOSCHANGING
            // 不会被节流（< 80ms 会被丢弃）
            last_wndproc_at: Instant::now()
                .checked_sub(std::time::Duration::from_millis(
                    WND_PROC_THROTTLE_MS as u64 + 1,
                ))
                .unwrap_or_else(Instant::now),
            last_mousemove_at: Instant::now()
                .checked_sub(std::time::Duration::from_millis(
                    MOUSE_MOVE_THROTTLE_MS as u64 + 1,
                ))
                .unwrap_or_else(Instant::now),
            // 修复 P0-#X：用真实的 work area（不是 zeroed()）。
            // 之前用 zeroed() 作为默认，detect_side 第一次比较 win.left <= work.left(0) + 20
            // 永远成立 → 窗口一启动就立刻触发 Left 隐藏倒计时（但实际还在屏幕中间！）
            cached_work: {
                let (_hm, work) = query_work_area(hwnd);
                work
            },
            cached_hmonitor: {
                let (hm, _work) = query_work_area(hwnd);
                hm
            },
        };

        // 修复 P0-#5：OnceLock.set 失败说明已装过
        DOCK_STATE
            .set(Mutex::new(state))
            .map_err(|_| "Widget dock 已经安装过了".to_string())?;

        // 修复 P0-#4：使用 SetLastError(0) + GetLastError() 判定安装是否成功，
        // 因为 SetWindowLongPtrW 的"原 WNDPROC"返回值可以是 0（DefWindowProcW 地址），
        // 不能用 `== 0` 判断失败。
        SetLastError(0);
        let original = SetWindowLongPtrW(hwnd, GWLP_WNDPROC, dock_wnd_proc as *const () as isize);
        let err = GetLastError();
        if err != 0 {
            return Err(format!("SetWindowLongPtrW 失败 (GetLastError={})", err));
        }
        // 把原始 WNDPROC 保存到 OnceLock，dock_wnd_proc 用它来转发消息
        ORIGINAL_WNDPROC
            .set(std::mem::transmute(original))
            .map_err(|_| "ORIGINAL_WNDPROC 已被设置".to_string())?;

        // 修复 P0-#X：取消 SetTimer 持续触发
        // 之前 SetTimer(32ms) 让 OS 每 32ms 强制中断主线程 → 拖动期间 "未响应"
        // 现在：动画一次到位，延迟用 thread::spawn + PostMessage（不抢主线程）
    }
    Ok(())
}

/// 启用 / 关闭贴边自动隐藏
pub fn set_enabled(enabled: bool) {
    with_state(|s| {
        s.enabled = enabled;
        if !enabled && s.hidden {
            s.hidden = false;
            s.pending_hide_at = None;
            let restore = RECT {
                left: s.pre_hide_x,
                top: s.pre_hide_y,
                right: s.pre_hide_x + s.pre_hide_w,
                bottom: s.pre_hide_y + s.pre_hide_h,
            };
            unsafe { apply_window_pos(s, restore) };
        }
    });
}

/// 当前是否处于隐藏态
pub fn is_hidden() -> bool {
    with_state(|s| s.hidden).unwrap_or(false)
}

/// 强制唤回（暴露给前端快捷键 Alt+Q 唤出时，先把窗口恢复）
pub fn force_reveal(window: &WebviewWindow) -> Result<(), String> {
    let restore_rect = with_state(|s| {
        s.hidden = false;
        s.pending_hide_at = None;
        RECT {
            left: s.pre_hide_x,
            top: s.pre_hide_y,
            right: s.pre_hide_x + s.pre_hide_w,
            bottom: s.pre_hide_y + s.pre_hide_h,
        }
    });
    if let Some(rect) = restore_rect {
        let _ = window.set_position(PhysicalPosition::new(rect.left, rect.top));
        let _ = window.set_size(PhysicalSize::new(
            (rect.right - rect.left) as u32,
            (rect.bottom - rect.top) as u32,
        ));
    }
    Ok(())
}

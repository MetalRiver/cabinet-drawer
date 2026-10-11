//! Windows 启动软件（ShellExecuteW 方案）
//!
//! 优势（vs cmd /C start）：
//! - 原生支持 .lnk 快捷方式（无需 cmd 解析）
//! - 原生支持 URL 协议（http://、steam://、ms-settings: 等）
//! - 自动从 PATH 查找可执行文件
//! - 不会弹出黑窗 cmd
//!
//! 修复 P1-#7：path 和 args 必须分开传，不能拼成字符串塞 lpFile，
//! 否则遇到 "C:\Program Files\X.exe --flag" 会被当成"文件名带空格的快捷方式"解析而失败。
//!
//! 修复 P0-#Y#URL#LAUNCH：URL 协议（http/https/steam:// 等）必须用 `explorer.exe <url>`
//! 兜底打开。ShellExecuteW 接受 URL，但实际 Windows 协议解析不稳——直接调 explorer
//! 会把 URL 交给 Windows 协议解析器（HTTP 用默认浏览器，steam:// 用 Steam 客户端）。

#![cfg(windows)]

use std::ffi::OsStr;
use std::iter::once;
use std::os::windows::ffi::OsStrExt;
use std::ptr;
use std::process::Command;

use windows_sys::Win32::UI::Shell::ShellExecuteW;
/// 判断是否是 URL 协议（http/https/steam:// 等）
pub fn is_url(s: &str) -> bool {
    let s = s.trim().to_lowercase();
    s.starts_with("http://")
        || s.starts_with("https://")
        || s.starts_with("steam://")
        || s.starts_with("com.epicgames.launcher://")
        || s.starts_with("ms-settings:")
        || s.starts_with("ms-store:")
        || s.starts_with("mailto:")
        || s.starts_with("discord:")
        || s.starts_with("spotify:")
        || s.starts_with("obsidian://")
        || s.starts_with("typora://")
        || s.starts_with("vscode://")
        || s.starts_with("jetbrains://")
}

/// 判断字符串是否看起来像裸域名（www.baidu.com / github.com/user/repo 等）
/// 用于后端二次校验 app_type，避免前端 bug 把网址存为 folder
pub fn looks_like_domain(s: &str) -> bool {
    let s = s.trim().to_lowercase();
    if s.is_empty() { return false; }
    // Windows 盘符：C:\ D:\ 不是域名
    let bytes = s.as_bytes();
    if bytes.len() >= 3 && bytes[0].is_ascii_alphabetic()
        && (bytes[1] == b':' )
        && (bytes[2] == b'\\' || bytes[2] == b'/') {
        return false;
    }
    const TLDS: &[&str] = &[
        // 中国区域后缀（长的放前面，避免被短的抢先匹配）
        ".com.cn", ".net.cn", ".org.cn", ".gov.cn", ".edu.cn", ".ac.cn",
        // 通用顶级域名 gTLD（最常用）
        ".com", ".cn", ".net", ".org", ".io", ".dev", ".cc", ".co", ".ai", ".app",
        ".top", ".xyz", ".club", ".shop", ".site", ".vip", ".tech", ".store",
        ".me", ".tv", ".fm", ".info", ".biz", ".us", ".jp", ".kr", ".ru", ".uk",
        ".de", ".fr", ".edu", ".gov", ".mil",
        // 高频率新型 TLD（之前漏的 .fun 特别补：用于 v2fun 等）
        ".fun", ".online", ".live", ".news", ".blog", ".wiki", ".video", ".cloud",
        ".work", ".link", ".win", ".space", ".website", ".press", ".today", ".run",
        ".life", ".group", ".design", ".art", ".photo", ".pics", ".pictures",
        ".show", ".watch", ".games", ".game", ".play", ".plus", ".pro", ".name",
        ".mobi", ".icu", ".xin", ".top", ".ren", ".wang", ".so", ".lu",
    ];
    for tld in TLDS {
        if let Some(idx) = s.find(tld) {
            let after = s.as_bytes().get(idx + tld.len()).copied();
            let match_after = match after {
                None => true,                          // 字符串结尾
                Some(b'/') | Some(b'?') | Some(b':') | Some(b'#') => true,
                _ => false,
            };
            if !match_after { continue; }
            // TLD 前面如果是反斜杠（\），那就是 Windows 路径，不是域名
            if idx > 0 {
                if let Some(before) = s.as_bytes().get(idx - 1) {
                    if *before == b'\\' { continue; }
                }
            }
            return true;
        }
    }
    false
}

/// 综合判断 path 是否属于 URL 类型（协议前缀 / .url 扩展名 / 裸域名）
pub fn is_url_path(path: &str) -> bool {
    if is_url(path) { return true; }
    if looks_like_domain(path) { return true; }
    let ext = std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .unwrap_or_default();
    ext == "url"
}

/// 用 ShellExecuteW 启动（自动处理 .lnk/.exe/URL）
///
/// # 参数
/// - `path`: 可执行文件路径 / .lnk 路径 / URL 协议
/// - `args`:  命令行参数（可为 ""）
///
/// # 返回
/// - `Ok(())`: 成功（返回值 > 32）
/// - `Err(msg)`: 失败，含 Windows 错误码语义
pub fn launch(path: &str, args: &str) -> Result<(), String> {
    // P0-#Y#FIX#BARE#DOMAIN：终极修复裸域名（无协议前缀）无法正确打开的问题
    let path_trim = path.trim();
    if !is_url(path_trim) && looks_like_domain(path_trim) {
        let fixed_url = format!("https://{}", path_trim);
        return launch_url_via_explorer(&fixed_url);
    }
    if is_url(path_trim) {
        return launch_url_via_explorer(path_trim);
    }
    // .url 文件也按 URL 走：读内部 URL 或者直接让 ShellExecuteW 处理
    let is_dot_url = std::path::Path::new(path_trim)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("url"))
        .unwrap_or(false);
    if is_dot_url {
        if let Some(inner_url) = crate::win_icon::parse_url_shortcut(path_trim) {
            return launch_url_via_explorer(&inner_url);
        }
    }

    let wide_path: Vec<u16> = OsStr::new(path_trim)
        .encode_wide()
        .chain(once(0))
        .collect();
    let wide_args: Vec<u16> = if args.is_empty() {
        // 空参数也必须传一个空字符串的指针，而不是 null
        vec![0u16]
    } else {
        OsStr::new(args).encode_wide().chain(once(0)).collect()
    };
    unsafe {
        // ShellExecuteW 参数语义：
        //   hwnd         = NULL（无父窗口）
        //   lpOperation  = NULL（不指定操作，如 "open" / "print"）
        //   lpFile       = path（要执行的文件 / .lnk / URL）
        //   lpParameters = args（命令行参数）← 关键：之前错误地塞进 lpFile
        //   lpDirectory  = NULL（用当前工作目录）
        //   nShowCmd     = SW_SHOWNORMAL = 1
        let result = ShellExecuteW(
            ptr::null_mut(),
            ptr::null(),
            wide_path.as_ptr(),
            wide_args.as_ptr(),
            ptr::null(),
            1,
        );
        let code = result as isize;
        if code > 32 {
            Ok(())
        } else {
            // 错误码 < 32 表示失败
            let msg = match code {
                0 => "系统内存或资源不足".to_string(),
                2 => format!("找不到文件: {}", path_trim),
                3 => format!("找不到路径: {}", path_trim),
                5 => "权限不足或文件不可执行".to_string(),
                11 => "EXE 文件无效".to_string(),
                26 => "共享错误".to_string(),
                27 => "关联不完整".to_string(),
                28 => "DLL 关联超时".to_string(),
                29 => "DLL 关联错误".to_string(),
                30 => "DLL 关联繁忙".to_string(),
                31 => "没有关联程序来处理此文件".to_string(),
                other => format!("ShellExecuteW 错误码 {}", other),
            };
            Err(msg)
        }
    }
}

/// URL 启动终极方案：ShellExecuteW + "open" verb（彻底弃用 explorer.exe！）
/// 🚨 之前用 explorer.exe <url> 有致命缺陷：一旦 URL 参数解析异常 / 空字符串，
///    explorer.exe 就直接打开「文档」文件夹（用户截图的罪魁祸首！）
///    ShellExecuteW 传 "open" verb + URL → Windows 直接按协议调用默认浏览器
fn launch_url_via_explorer(url: &str) -> Result<(), String> {
    let url_trim = url.trim();
    if url_trim.is_empty() {
        return Err("URL 是空字符串，拒绝启动（否则 explorer 会误开「文档」文件夹！）".to_string());
    }
    let wide_url: Vec<u16> = OsStr::new(url_trim)
        .encode_wide()
        .chain(once(0))
        .collect();
    let wide_verb: Vec<u16> = OsStr::new("open")
        .encode_wide()
        .chain(once(0))
        .collect();
    unsafe {
        let result = ShellExecuteW(
            ptr::null_mut(),
            wide_verb.as_ptr(),   // lpOperation = "open"（强制走「打开」动词，URL → 默认浏览器）
            wide_url.as_ptr(),    // lpFile = URL
            ptr::null(),          // lpParameters = 空（URL 不需要额外参数）
            ptr::null(),
            1,
        );
        let code = result as isize;
        if code > 32 {
            Ok(())
        } else {
            // ShellExecuteW 失败 → 兜底 explorer.exe（原错误码文案仅用于已删除的调试日志）
            match Command::new("explorer.exe").arg(url_trim).spawn() {
                Ok(_) => Ok(()),
                Err(e) => Err(format!("打开 URL 彻底失败（ShellExecuteW={}, explorer.exe 兜底也失败）: {} ({})", code, url_trim, e)),
            }
        }
    }
}

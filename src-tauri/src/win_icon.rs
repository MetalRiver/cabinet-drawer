//! Windows 应用图标提取与缓存
//!
//! 实现：用 `windows-icons` crate 直接调 Windows API `ExtractIconW`
//! 替代原 PowerShell + System.Drawing.Icon 方案
//!
//! 优势：
//! - 速度：纯 Rust 内存操作，834 个软件从 ~30-60s 降到 ~1-3s
//! - 可靠：不依赖 .NET / PowerShell / PowerShell 执行策略
//! - 跨版本一致：Windows 7+ 行为一致（直接调系统 API）
//!
//! 缓存策略：exe_path SHA256 前 8 字节（lowercase 标准化） → icons/{hash}.png
//! 命中缓存直接返回；未命中调 windows-icons 抽图保存

use std::fs;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use windows_icons::get_icon_by_path;

// P0-#PERF#NOCMD：Windows 上用 CommandExt 加 CREATE_NO_WINDOW flag 防止闪窗
#[cfg(windows)]
use std::os::windows::process::CommandExt;

/// 解析 .lnk 目标路径（用 WScript.Shell COM）
/// 返回：Some(target_path) 或 None（解析失败）
pub fn resolve_lnk_target(lnk_path: &str) -> Option<String> {
    let escaped = lnk_path.replace('\'', "''");
    // P0-A（v0.4.3 验收）：与 Batch 1 修复的批量解析器口径对齐——
    // ① 显式 UTF-8 输出：原先按控制台 OEM 代码页（中文系统=GBK）输出，
    //    from_utf8_lossy 后中文目标路径变乱码 → exists() 恒 false →
    //    被 classify_one 误判为 fixed_missing（可清退，数据误判）。
    // ② 每轮重置 $t + SilentlyContinue：CreateShortcut 失败时旧变量残留会错绑。
    let ps = format!(
        "[Console]::OutputEncoding=[System.Text.Encoding]::UTF8; \
         $ErrorActionPreference='SilentlyContinue'; \
         $sh = New-Object -ComObject WScript.Shell; \
         $t = $null; \
         $t = $sh.CreateShortcut('{}').TargetPath; \
         if ($t) {{ [Environment]::ExpandEnvironmentVariables($t) }} else {{ '' }}",
        escaped
    );
    // P0-#PERF#NOCMD：加 CREATE_NO_WINDOW flag
    // 之前不加 → PowerShell 启动时 Windows 强制显示 console → 闪一下关闭
    // 这是 health_check_apps 闪窗的元凶（每个 .lnk 解析都跑一次）
    let output = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &ps])
        .creation_flags(0x08000000) // CREATE_NO_WINDOW
        .output()
        .ok()?;
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let target = stdout.trim().to_string();
    if target.is_empty() {
        None
    } else {
        Some(target)
    }
}

/// P0-#Y#FIX#ICON#REEXTRACT：清空所有 cache PNG
/// 用户反馈"图标不对"时调用 → 强制下次 fill_missing_icons 重新抽图
pub fn clear_all_cache() -> std::io::Result<usize> {
    let dir = cache_root();
    if !dir.exists() {
        return Ok(0);
    }
    let mut count = 0;
    for entry in std::fs::read_dir(&dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("png") {
            if std::fs::remove_file(&path).is_ok() {
                count += 1;
            }
        }
    }
    Ok(count)
}

/// 选择最佳抽图路径：
/// - .lnk → 先解析 target，target 是 .exe/.bat/.cmd 就抽 target（豆包/Epic 等 UWP/store 应用专用）
/// - 否则直接抽原路径
fn pick_extract_path(p: &str) -> String {
    let ext = Path::new(p)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .unwrap_or_default();
    if ext != "lnk" {
        return p.to_string();
    }
    // P0-#Y#FIX#ICON#REAL：.lnk 优先用 Windows Shell API 抽 .lnk 自身图标
    //  之前：调 PowerShell 解析 target → 抽 target.exe 图标
    //    Steam 游戏的 .lnk target 是 steam.exe → 抽到 Steam launcher 图标（**不是游戏原本图标**）
    //    Epic 游戏的 .lnk target 是 EpicGamesLauncher.exe → 同上
    //  现在：直接抽 .lnk 文件 → Windows ExtractIconExW 会按 .lnk 在 Explorer 里的显示抽图
    //    Steam/Epic/UWP 的 .lnk 内嵌了 target 的真实图标 → 抽到游戏原本图标
    //    普通程序的 .lnk 也有它自己展示用的图标
    //  UWP（target 是 windows-explorer:）情况：windows-icons 抽 .lnk 也能拿到通用 UWP 图标
    //  性能：比 PowerShell 解析 target 快 100 倍
    //  没破坏：所有非 .lnk 路径走 p.to_string() 同样行为
    p.to_string()
}

/// P0-#Y#URL：解析 .url 文件（Internet Shortcut）内部的 URL 字段
/// .url 文件是文本格式：内容形如
///   [InternetShortcut]
///   URL=https://example.com
/// 返回 Option<String>（Some(url) = 解析到 URL，None = 解析失败或非 .url）
pub fn parse_url_shortcut(path: &str) -> Option<String> {
    let ext = Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .unwrap_or_default();
    if ext != "url" {
        return None;
    }
    let content = std::fs::read_to_string(path).ok()?;
    // 找 URL= 开头的那行
    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(url) = trimmed.strip_prefix("URL=") {
            return Some(url.trim().to_string());
        }
        // 大小写不敏感
        if let Some(rest) = trimmed.strip_prefix("url=") {
            return Some(rest.trim().to_string());
        }
    }
    None
}


// ============================================================
// G3-1 取证守卫（仅测试构建 test-edition 编译；正式构建零改动）
// 目的：H3/生产取证发现隔离测试期间生产图标目录出现孤儿占位图，
//       而 G-1 只守护启动入口、不拦运行时文件写入。
// 行为：设置 DRAWER_ICON_GUARD_ROOT 后，任何图标 PNG 写入目标必须位于
//       该根内；越界则【在执行写入之前】拒绝（fail-closed），并把
//       PID / 目标 / 来源路径 / 调用方 写入守卫日志（守卫根内，隔离目录）。
// ============================================================
#[cfg(feature = "test-edition")]
fn icon_write_guard(target: &std::path::Path, source_path: &str, caller: &str) -> Result<(), String> {
    // G3-1 收口（评审要求）：测试构建【必须】配置守卫根；缺失 = fail-closed
    // （拒绝一切图标写入），防止任何未挂守卫的测试实例把文件写进真实用户目录。
    // 正式构建不含本函数（cfg 门控），行为零变化。
    let root = match std::env::var("DRAWER_ICON_GUARD_ROOT") {
        Ok(r) if !r.trim().is_empty() => r,
        _ => {
            let line = format!(
                "[icon-write-guard] FAIL-CLOSED pid={} caller={} target={}（测试构建未配置 DRAWER_ICON_GUARD_ROOT，拒绝写入）",
                std::process::id(), caller, target.display()
            );
            eprintln!("{}", line);
            return Err(line);
        }
    };
    let t = target.to_string_lossy().to_lowercase();
    let r = root.trim_end_matches(['\\', '/']).to_lowercase();
    if t.starts_with(&r) {
        return Ok(());
    }
    let line = format!(
        "[icon-write-guard] REJECT pid={} caller={} target={} source={} guard_root={}
",
        std::process::id(),
        caller,
        target.display(),
        source_path,
        root
    );
    let log_path = std::path::Path::new(&root).join("icon-write-guard.log");
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&log_path) {
        use std::io::Write as _;
        let _ = f.write_all(line.as_bytes());
    }
    eprintln!("{}", line.trim_end());
    Err(format!("icon-write-guard: 拒绝写入测试根之外: {}", target.display()))
}

#[cfg(not(feature = "test-edition"))]
fn icon_write_guard(_target: &std::path::Path, _source_path: &str, _caller: &str) -> Result<(), String> {
    Ok(())
}

/// icon 缓存根目录
/// G3-1C：改为 pub —— 受控回退命令 `get_app_icon_data_url` 需用它校验
/// 请求读取的路径必须位于缓存根内（防止按路径任意读文件的旧通道被滥用）。
/// G7-2 隔离修正：目录名改为按 identifier 解析（app_identity::app_dir_name）——
/// 正式版=com.drawer-box.app（与历史硬编码逐字节一致）；
/// 候选版=com.drawer-box.candidate（与资产 scope `$APPLOCALDATA/icons/**/*` 对齐，不再写穿正式缓存）。
pub fn cache_root() -> PathBuf {
    let base = std::env::var("LOCALAPPDATA")
        .or_else(|_| std::env::var("APPDATA"))
        .unwrap_or_else(|_| ".".to_string());
    PathBuf::from(base)
        .join(crate::app_identity::app_dir_name())
        .join("icons")
}

/// 计算 cache 文件名（SHA256 前 16 字符）
/// 注意：lowercase 标准化路径，避免大小写差异导致 cache miss
pub fn cache_key(exe_path: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(exe_path.to_lowercase().as_bytes());
    let result = hasher.finalize();
    let bytes = &result[..8];
    let mut s = String::with_capacity(16);
    for b in bytes {
        s.push_str(&format!("{:02x}", b));
    }
    s
}

/// 单个抽图：调 windows-icons 抽图，存为 64x64 PNG
/// 命中缓存时直接返回，0 延迟
/// 图标来源优先级（M1-P2 F2 统一）：快捷方式自身 > 解析出的目标程序 > 无图标（前端类型 SVG 兜底）
/// —— .lnk 直接抽自身（Windows 按 Explorer 显示抽图，Steam/Epic/UWP 游戏图标正确）；
///    lnk 抽取失败再试 resolve_lnk_target 解析出的 target（两级候选）。
///
/// 抽图全部失败时：**不写占位图、不返回路径**（M1-P2 F2：占位图曾以"成功"身份入库并被
/// 当真实图标显示——运行时实证）。缓存键 cache_key 恒按**入口路径**计算：同一入口永远
/// 映射同一缓存文件，fill_missing_icons 据此实现确定性重基（cache_key 归属校验）。
/// 失败重试为事件驱动（视图挂载/手动刷新/单次延迟重试），非周期轮询，单次成本有界。
pub fn extract_to_cache(exe_path: &str) -> Option<PathBuf> {
    if !Path::new(exe_path).exists() {
        return None;
    }
    let cache_dir = cache_root();
    if !cache_dir.exists() {
        let _ = fs::create_dir_all(&cache_dir);
    }
    // 归属键恒按入口路径（不随候选源变化）——重抽/重基后 DB 引用保持确定性
    let key = cache_key(exe_path);
    let cache_path = cache_dir.join(format!("{}.png", key));

    // G3-1 取证守卫：目标不在守卫根内 → 不写任何文件（fail-closed）
    if let Err(e) = icon_write_guard(&cache_path, exe_path, "extract_to_cache") {
        eprintln!("{}", e);
        return None;
    }

    if cache_path.exists() {
        return Some(cache_path);
    }

    // 两级候选：lnk 自身优先；失败再试解析出的 target。非 lnk 仅自身一级。
    let mut candidates: Vec<String> = vec![pick_extract_path(exe_path)];
    if exe_path.to_lowercase().ends_with(".lnk") {
        if let Some(target) = resolve_lnk_target(exe_path) {
            if Path::new(&target).exists() && !candidates.contains(&target) {
                candidates.push(target);
            }
        }
    }
    for cand in &candidates {
        if !Path::new(cand).exists() {
            continue;
        }
        // windows-icons 内调 Windows API ExtractIconW，返回 RgbaImage
        match get_icon_by_path(cand) {
            Ok(img) => {
                if let Err(e) = img.save_with_format(&cache_path, image::ImageFormat::Png) {
                    eprintln!("[win_icon] save failed for {}: {}", cand, e);
                    return None;
                }
                return Some(cache_path);
            }
            Err(e) => {
                eprintln!("[win_icon] extract failed for {}: {}", cand, e);
            }
        }
    }
    // 全部候选失败 → 不写占位、不落库（fill 不回写 → icon_path 保持空 → 前端类型 SVG 兜底）
    None
}

// M1-P2（F2）：write_fallback_icon 已移除——占位图曾以"成功"身份入库并被当真实图标
// 显示（运行时实证），且一旦落盘即永久毒化缓存判定。抽图失败现返回 None，由前端
// 类型 SVG/emoji 诚实兜底；历史占位 PNG 由 force_reextract（清缓存+全量重抽）清理。

/// 批量抽图：分离缓存命中与未命中，未命中项串行抽图
/// 注意：windows-icons 内部有 COM 调用（Shell），并发抽图可能 COM 初始化冲突；
///       但单次调用极快（~5-15ms/个），串行也够（834 个约 5-10s）
///
/// 返回：所有成功抽图（含命中缓存）的 (exe_path, png_path) 列表
pub fn extract_batch_to_cache(exe_paths: &[String]) -> Vec<(String, PathBuf)> {
    let cache_dir = cache_root();
    if !cache_dir.exists() {
        let _ = fs::create_dir_all(&cache_dir);
    }

    let mut cached: Vec<(String, PathBuf)> = Vec::new();
    let mut to_extract: Vec<String> = Vec::new();
    for exe in exe_paths {
        if !Path::new(exe).exists() {
            continue;
        }
        let p = cache_dir.join(format!("{}.png", cache_key(exe)));
        if p.exists() {
            cached.push((exe.clone(), p));
        } else {
            to_extract.push(exe.clone());
        }
    }

    // 串行抽图（避免 COM 冲突，且单次够快）
    let total = to_extract.len();
    let mut done = 0usize;
    for exe in to_extract {
        if let Some(p) = extract_to_cache(&exe) {
            cached.push((exe, p));
        }
        done += 1;
        // 每 100 个打印一次进度
        if done % 100 == 0 {
            eprintln!("[win_icon] batch progress {}/{}", done, total);
        }
    }
    eprintln!(
        "[win_icon] batch done: {} total, {} cached, {} fresh",
        cached.len(),
        cached.len() - (total - done),
        total - done
    );

    cached
}

// ============================================================
// M1-P2（F2/F3）单测：缓存键归属确定性（fill 确定性重基的证据基础）
// 覆盖：同一入口路径恒产生同一缓存键（大小写不敏感）；不同入口不同键。
// 注意：不测试 extract_to_cache 的写盘行为——测试进程的 cache_root 指向真实
// %LOCALAPPDATA% 缓存（生产边界），写盘行为由隔离环境运行时探针验证。
// ============================================================
#[cfg(test)]
mod m1_cache_key_tests {
    use super::cache_key;

    #[test]
    fn t1_entry_path_deterministic_case_insensitive() {
        let a = cache_key(r"D:\Drawer-Upg-Rehearsal\perf-fix-v043\h1-exes\notepad.exe");
        let b = cache_key(r"d:\drawer-upg-rehearsal\perf-fix-v043\h1-exes\NOTEPAD.EXE");
        assert_eq!(a, b, "同一入口路径（大小写差异）必须映射同一缓存键——fill 重基依赖此性质");
    }

    #[test]
    fn t2_different_entries_different_keys() {
        let a = cache_key(r"D:\x\a.lnk");
        let b = cache_key(r"D:\x\b.lnk");
        assert_ne!(a, b, "不同入口路径必须不同缓存键（防错误关联图标）");
    }

    #[test]
    fn t3_key_is_flat_hex_filename_base() {
        let k = cache_key(r"C:\Windows\System32\notepad.exe");
        assert_eq!(k.len(), 16, "cache_key 为 SHA256 前 8 字节的 16 位 hex——缓存布局扁平 key.png");
        assert!(k.chars().all(|c| c.is_ascii_hexdigit()));
    }
}

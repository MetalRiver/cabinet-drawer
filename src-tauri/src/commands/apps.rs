//! ===== 命令分组 ⑦：🛡️ 软件管理（apps）+ 分类 + 图标 + 扫描 + 启动 + 健康检查 =====
//! 包含：扫描系统软件、软件 CRUD、分类 CRUD、图标提取/批量重抽、文件选择、批量导入路径、启动软件（含自动重定位）、健康检查（跨盘修复路径）

use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use tauri::{AppHandle, Manager, State};

use crate::crypto;
use crate::db::{self, Db};
use crate::AppState;

#[cfg(windows)]
use crate::win_dock;
#[cfg(windows)]
use crate::win_icon;
#[cfg(windows)]
use crate::win_launch;

#[cfg(windows)]
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
// ============================================================
// 📦 扫描系统软件时返回的条目
// ============================================================
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ScannedApp {
    pub name: String,
    pub path: String,
    pub icon_path: String,
    pub args: String,
    pub source: String,
    pub app_type: String,
    pub app_subtype: String,
    pub is_launchable: bool,
    // ---- Batch 2.1（v0.4.3）：入口身份与状态（serde default 兼容旧调用方）----
    /// 实际目标（lnk 解析出的目标路径 / .url 内的网址 / 直连 exe 即自身）
    #[serde(default)]
    pub target: String,
    /// 快捷方式自带的启动参数（身份的一部分：同 exe 不同参数 = 不同入口）
    #[serde(default)]
    pub target_args: String,
    /// 快捷方式的工作目录
    #[serde(default)]
    pub workdir: String,
    /// "identified"（已识别）| "needs_confirmation"（需要确认）
    #[serde(default)]
    pub status: String,
    /// 库中已有同身份有效条目（前端据此禁用勾选并标注「已在库中」）
    #[serde(default)]
    pub already_imported: bool,
    /// 库中存在同身份但已进回收站的条目（前端提示可恢复）
    #[serde(default)]
    pub in_trash: bool,
    /// P1-B（v0.4.3 验收）：回收站命中条目的 id（前端「恢复此软件」直接据此恢复；
    /// 无命中为 None。不自动恢复——恢复必须是用户显式动作）
    #[serde(default)]
    pub trash_id: Option<i64>,
}

/// Batch 2.1：一个 .lnk 的批量解析结果（目标 + 参数 + 工作目录）。
/// 参数与工作目录参与入口身份；目标可能为空 = 解析失败。
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct LnkResolution {
    pub target: String,
    pub args: String,
    pub workdir: String,
}

/// Batch 2.1：入口身份键。
/// 身份 = 入口类型 ∥ 规范化目标（或 URL）∥ 规范化参数。
/// 只有两层完全一致才视为"同一入口"——Chrome PWA（同 exe 不同 --app=）等
/// 不同用途入口不会被合并。
pub(crate) fn entry_identity_key(kind: &str, target_or_url: &str, args: &str) -> String {
    format!(
        "{}\u{1}{}\u{1}{}",
        kind,
        normalize_path_key(target_or_url),
        normalize_path_key(args)
    )
}

/// Batch 2.1：路径/参数规范化——小写、正斜杠统一为反斜杠、去尾部分隔符。
/// 纯函数（不做 UNC/短文件名解析，控制范围）。
pub(crate) fn normalize_path_key(s: &str) -> String {
    let mut out = s.trim().to_lowercase().replace('/', "\\");
    while out.ends_with('\\') && out.len() > 1 {
        out.pop();
    }
    out
}

/// Batch 2.1：入口分类结果（None = 不是软件候选，直接排除）。
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct EntryClassification {
    pub app_type: &'static str, // "app" | "url"
    pub app_subtype: &'static str,
    pub status: &'static str, // "identified" | "needs_confirmation"
    pub is_launchable: bool,
}

/// Batch 2.1：lnk/url 候选分类（纯函数；文件系统事实由调用方传入）。
///
/// 设计红线（对应产品决策）：
/// - **lnk 候选不做目标文件名模式过滤**——开始菜单/桌面 lnk 是人工策划入口，
///   `Discord.lnk → Update.exe --processStart` 这类真实入口不得被 "update" 字样误杀；
/// - **不做路径级剔除**——`\Windows\`、System32、WindowsApps 下有大量系统自带
///   常用软件（记事本/命令提示符/Win11 截图工具等）；
/// - 文件夹与文档目标不是软件 → 排除（用户需要时可手动添加）；
/// - 解析失败/无法确认 → 「需要确认」，可见可选，绝不静默丢弃、绝不错绑。
pub(crate) fn classify_entry(
    is_url_file: bool,
    target: &str,
    target_args: &str,
    target_exists: bool,
    target_is_dir: bool,
) -> Option<EntryClassification> {
    if is_url_file {
        return Some(EntryClassification {
            app_type: "url",
            app_subtype: "",
            status: "identified",
            is_launchable: true,
        });
    }
    if target.is_empty() {
        // 解析失败（协议层空结果）：保留为「需要确认」，绝不丢弃
        return Some(EntryClassification {
            app_type: "app",
            app_subtype: "",
            status: "needs_confirmation",
            is_launchable: true,
        });
    }
    if !target_exists || target_is_dir {
        // 指向目录（用户整理的文件夹快捷方式）或已失效目标：
        // 目录 → 非软件，排除；失效目标 → 「需要确认」（启动时可尝试自动重定位）
        if target_is_dir {
            return None;
        }
        return Some(EntryClassification {
            app_type: "app",
            app_subtype: "",
            status: "needs_confirmation",
            is_launchable: true,
        });
    }
    let ext = std::path::Path::new(target)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "exe" => Some(EntryClassification {
            app_type: "app",
            app_subtype: detect_subtype_from_path(std::path::Path::new(target), "app"),
            status: "identified",
            is_launchable: true,
        }),
        // 脚本启动器：.bat/.cmd/.vbs 经 ShellExecute 可直接运行 → 已识别；
        // .ps1 默认关联是记事本，直接启动会打开编辑器 → 「需要确认」（启动适配待后续批次）
        "bat" | "cmd" | "vbs" => Some(EntryClassification {
            app_type: "app",
            app_subtype: "script",
            status: "identified",
            is_launchable: true,
        }),
        "ps1" => Some(EntryClassification {
            app_type: "app",
            app_subtype: "script",
            status: "needs_confirmation",
            is_launchable: false,
        }),
        // 指向文档等其他文件 → 非软件候选
        _ => None,
    }
}

/// Batch 2.4：AppID 结构分类（Spike 门1 实证的五桶；不依赖单一 `!` 特征）。
/// 返回 Some(kind) = 可纳入的打包应用身份（"uwp"=PFN!App / "msix"=纯 PFN）；
/// None = 路径类/GUID/AutoGenerated 等——不纳入（路径类已由 lnk 通道覆盖，
/// GUID/AutoGenerated 无法证明可激活，按决策保留为未支持类型，不标「已识别」）。
pub(crate) fn classify_appid(app_id: &str) -> Option<&'static str> {
    let s = app_id.trim();
    if s.is_empty() {
        return None;
    }
    if s.starts_with('{') || s.starts_with("Microsoft.AutoGenerated") {
        return None; // GUID 路径 / 自动生成项：不可证明可激活
    }
    if s.contains('\\') {
        // 盘符路径或 GUID\文件：路径类（已有 lnk/exe 通道覆盖）
        return None;
    }
    if s.contains('!') {
        return Some("uwp"); // 标准 AUMID：PackageFamilyName!App
    }
    // 无 ! 无斜杠：PFName（MSIX 桌面应用，如 com.zhipuai.autoclaw2）
    if s.contains('.') {
        return Some("msix");
    }
    None
}

/// Batch 2.4 修正（按验收指令）：打包应用身份验证结果。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct UwpVerification {
    pub app_id: String,
    pub verified: bool,
    pub reason: String,
}

/// 批量验证打包应用身份（Spike 门4 方案的落地）：
/// - AUMID（PFN!App）：包已注册 + Status=Ok + **Manifest 内存在匹配的 Application Id**；
/// - 纯 PFN：包已注册 + Status=Ok（MSIX 桌面应用默认条目）；
/// - 任何一步失败 → verified=false（前端降级「需要确认」，绝不标「已识别」）。
/// 只读查询，不修改系统状态。
#[cfg(windows)]
pub(crate) fn verify_uwp_identities(app_ids: &[String]) -> Result<Vec<UwpVerification>, String> {
    // 构造 PS 数组字面量（单引号转义）
    let items: Vec<String> = app_ids
        .iter()
        .map(|id| format!("'{}'", id.replace('\'', "''")))
        .collect();
    let ps = format!(
        "[Console]::OutputEncoding=[System.Text.Encoding]::UTF8;          $ErrorActionPreference='SilentlyContinue';          $ids = @({});          $out = foreach ($id in $ids) {{            $pfn = ($id -split '!')[0];            $suffix = $null; if ($id -like '*!*') {{ $suffix = ($id -split '!')[1] }};            $pkg = Get-AppxPackage -Name $pfn -ErrorAction SilentlyContinue;            if (-not $pkg) {{ [pscustomobject]@{{ app_id=$id; verified=$false; reason='package-not-found' }}; continue }};            if ($pkg.Status -ne 'Ok') {{ [pscustomobject]@{{ app_id=$id; verified=$false; reason=('package-status-' + $pkg.Status) }}; continue }};            if ($suffix) {{              $manifest = Join-Path $pkg.InstallLocation 'AppxManifest.xml';              if (-not (Test-Path $manifest)) {{ [pscustomobject]@{{ app_id=$id; verified=$false; reason='manifest-missing' }}; continue }};              $xml = Get-Content $manifest -Raw;              if ($xml -match ('Id='' + [regex]::Escape($suffix) + ''' )) {{ [pscustomobject]@{{ app_id=$id; verified=$true; reason='' }} }}              else {{ [pscustomobject]@{{ app_id=$id; verified=$false; reason='app-id-not-in-manifest' }} }}            }} else {{              [pscustomobject]@{{ app_id=$id; verified=$true; reason='' }}            }}          }};          $out | ConvertTo-Json -Depth 3 -Compress",
        items.join(",")
    );
    let output = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &ps])
        .creation_flags(0x08000000)
        .output()
        .map_err(|e| format!("打包应用身份验证失败：{e}"))?;
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    if stdout.trim().is_empty() {
        return Err("打包应用身份验证无输出".to_string());
    }
    let json_text = if stdout.trim_start().starts_with('[') {
        stdout.clone()
    } else {
        format!("[{}]", stdout.trim())
    };
    let parsed: Vec<UwpVerification> = serde_json::from_str(&json_text)
        .map_err(|e| format!("验证结果解析失败：{e}"))?;
    Ok(parsed)
}

#[cfg(not(windows))]
pub(crate) fn verify_uwp_identities(_app_ids: &[String]) -> Result<Vec<UwpVerification>, String> {
    Err("打包应用身份验证仅支持 Windows".to_string())
}

/// Batch 2.4 修正：验证结果应用到候选——
/// verified → 「已识别」可启动；未验证/失败 → **「需要确认」且 is_launchable=false**
///（验收指令：对无法验证启动身份的候选显示「需要确认」，不标记为已可启动）。
pub(crate) fn apply_uwp_verification(
    results: &mut [ScannedApp],
    verified: &[UwpVerification],
) {
    let map: std::collections::HashMap<&str, &UwpVerification> = verified
        .iter()
        .map(|v| (v.app_id.as_str(), v))
        .collect();
    for c in results.iter_mut() {
        if c.app_type != "uwp" {
            continue;
        }
        match map.get(c.path.as_str()) {
            Some(v) if v.verified => {
                c.status = "identified".to_string();
                c.is_launchable = true;
            }
            _ => {
                c.status = "needs_confirmation".to_string();
                c.is_launchable = false;
            }
        }
    }
}

/// Batch 2.4：Get-StartApps 枚举（PS JSON）→ 打包应用 raw 候选。
/// 仅纳入 classify_appid 认可的 AUMID/PFN；身份 = uwp∥AppID（与 exe 身份互不合并）。
#[cfg(windows)]
pub(crate) fn scan_uwp_raw() -> Result<Vec<(String, String, String)>, String> {
    let output = std::process::Command::new("powershell")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "[Console]::OutputEncoding=[System.Text.Encoding]::UTF8; \
             $ErrorActionPreference='SilentlyContinue'; \
             Get-StartApps | ConvertTo-Json -Depth 3 -Compress",
        ])
        .creation_flags(0x08000000)
        .output()
        .map_err(|e| format!("打包应用枚举失败：{e}"))?;
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    if stdout.trim().is_empty() {
        return Err("Get-StartApps 无输出（PowerShell 枚举失败）".to_string());
    }
    // ConvertTo-Json 单对象时返回 object、多对象返回数组——统一包一层数组解析
    let json_text = if stdout.trim_start().starts_with('[') {
        stdout.clone()
    } else {
        format!("[{}]", stdout.trim())
    };
    let parsed: Vec<serde_json::Value> =
        serde_json::from_str(&json_text).map_err(|e| format!("Get-StartApps 输出解析失败：{e}"))?;
    let mut out: Vec<(String, String, String)> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for item in parsed {
        let name = item
            .get("Name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let app_id = item
            .get("AppID")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if classify_appid(&app_id).is_some() && seen.insert(app_id.clone()) {
            out.push((app_id, name, "Store/应用".to_string()));
        }
    }
    Ok(out)
}

#[cfg(not(windows))]
pub(crate) fn scan_uwp_raw() -> Result<Vec<(String, String, String)>, String> {
    Err("打包应用枚举仅支持 Windows".to_string())
}

/// Batch 2.2：高级扫描（Program Files 遍历）的入口状态。
/// 精确辅助名已在 walk_exe 剔除；模糊模式（helper/elevate/crash/report）
/// 不删除——降级为「需要确认」交用户判断。
pub(crate) fn advanced_status_for(name_lc: &str) -> &'static str {
    let hints = ["helper", "elevate", "crash", "report", "toolkit", "unins"];
    if hints.iter().any(|h| name_lc.contains(h)) {
        "needs_confirmation"
    } else {
        "identified"
    }
}

/// Batch 2.3：扫描取消旗标（真正终止——遍历每 256 项、解析每批边界检查）。
/// scan 命令开始时复位；`cancel_scan` 命令置位。
pub static SCAN_CANCELLED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Batch 2.4（并发审计 D2）：扫描互斥——同一时刻只允许一个扫描任务。
/// 没有它，任务 A 在飞时任务 B 启动会复位取消旗标（A 复活）并双写结果。
/// RAII guard：任何返回路径（含 ? / panic unwind）都保证释放。
pub static SCAN_RUNNING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub(crate) struct ScanGuard;

impl ScanGuard {
    /// 尝试获取唯一的扫描执行权；已被占用时返回 Err（前端提示稍后再试）。
    pub fn acquire() -> Result<Self, String> {
        use std::sync::atomic::Ordering;
        match SCAN_RUNNING.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst) {
            Ok(_) => {
                // 只有拿到执行权的任务才允许复位取消旗标——
                // 修复"任务 B 启动把任务 A 的取消信号抹掉"。
                SCAN_CANCELLED.store(false, Ordering::Relaxed);
                Ok(ScanGuard)
            }
            Err(_) => Err("已有扫描正在进行，请等待完成或取消后再试。".to_string()),
        }
    }
}

impl Drop for ScanGuard {
    fn drop(&mut self) {
        SCAN_RUNNING.store(false, std::sync::atomic::Ordering::SeqCst);
    }
}

/// Batch 2.3：扫描结果（含取消/截断状态——取消时返回已完成的部分结果）。
#[derive(Debug, Clone, serde::Serialize)]
pub struct ScanOutcome {
    pub apps: Vec<ScannedApp>,
    /// 用户取消：已终止后续遍历/解析，apps 为已完成部分
    pub cancelled: bool,
    /// 自定义目录扫描触发数量/深度保护（结果可能不完整）
    pub truncated: bool,
}

// ============================================================
// 🔍 辅助：按路径 / 扩展名 识别 app_type / subtype / 能否启动
// ============================================================
/// 根据扩展名 + 路径特征返回 app_type（"app" / "document" / "url" / "folder"）
/// P0-#Y#FIX#URL#DETECT：不仅看扩展名，还要判断 URL 协议 + 裸域名，避免网址被误判
pub fn detect_type_from_path(path: &std::path::Path) -> &'static str {
    let s = path.to_string_lossy();
    #[cfg(windows)]
    {
        // 先看是否是 URL 形式
        if win_launch::is_url_path(&s) {
            return "url";
        }
    }
    #[cfg(not(windows))]
    {
        let sl = s.to_lowercase();
        if sl.starts_with("http://")
            || sl.starts_with("https://")
            || sl.contains("://")
            || sl.ends_with(".url")
        {
            return "url";
        }
    }
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "exe" | "lnk" | "bat" | "cmd" | "msi" => "app",
        "url" => "url",
        "doc" | "docx" | "xls" | "xlsx" | "ppt" | "pptx" | "pdf" | "txt" | "md" | "rtf" | "odt"
        | "ods" | "odp" | "csv" => "document",
        _ => "app",
    }
}

/// Batch 2.1：detect_launchable 已被 classify_entry 的 is_launchable 判定取代，
/// 旧函数删除（唯一调用方是已重构的旧扫描器）。

/// 根据扩展名 / 路径 / 文件名 推断 app_subtype
pub fn detect_subtype_from_path(path: &std::path::Path, app_type: &str) -> &'static str {
    if app_type == "document" {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_lowercase())
            .unwrap_or_default();
        return match ext.as_str() {
            "doc" | "docx" | "rtf" | "odt" => "word",
            "xls" | "xlsx" | "ods" | "csv" => "excel",
            "ppt" | "pptx" | "odp" => "ppt",
            "pdf" => "pdf",
            "txt" | "md" => "text",
            "png" | "jpg" | "jpeg" | "gif" | "bmp" | "webp" | "svg" | "ico" => "image",
            _ => "other",
        };
    }
    if app_type != "app" {
        return "";
    }

    let path_lower = path.to_string_lossy().to_lowercase();
    let name_lower = path
        .file_name()
        .and_then(|s| s.to_str())
        .map(|s| s.to_lowercase())
        .unwrap_or_default();

    let game_kw = [
        "steam",
        "epic",
        "origin",
        "ubisoft",
        "uplay",
        "riot",
        "battlenet",
        "battle.net",
        "xbox",
        "playstation",
        "minecraft",
        "wegame",
        "netease",
        "lol",
        "dota",
        "csgo",
        "pubg",
    ];
    for kw in game_kw.iter() {
        if name_lower.contains(kw) || path_lower.contains(kw) {
            return "game";
        }
    }
    let office_kw = [
        "office",
        "word",
        "excel",
        "powerpoint",
        "outlook",
        "wps",
        "dingtalk",
        "feishu",
        "lark",
        "wework",
        "teams",
        "zoom",
        "notion",
    ];
    for kw in office_kw.iter() {
        if name_lower.contains(kw) || path_lower.contains(kw) {
            return "office";
        }
    }
    let dev_kw = [
        "code",
        "vscode",
        "visual studio",
        "rider",
        "pycharm",
        "intellij",
        "idea",
        "webstorm",
        "goland",
        "clion",
        "android studio",
        "xcode",
        "sublime",
        "vim",
        "neovim",
        "git",
        "github",
        "gitlab",
        "docker",
        "kubernetes",
        "kubectl",
        "postman",
        "insomnia",
        "dbeaver",
        "navicat",
        "redis",
        "mongodb",
        "mysql",
        "wsl",
        "terminal",
        "powershell",
        "cmd",
        "node",
        "npm",
        "yarn",
        "pnpm",
        "cargo",
        "rust",
        "python",
        "java",
        "jdk",
        "gradle",
        "maven",
        "electron",
        "tauri",
    ];
    for kw in dev_kw.iter() {
        if name_lower.contains(kw) || path_lower.contains(kw) {
            return "dev";
        }
    }
    let media_kw = [
        "potplayer",
        "vlc",
        "mpv",
        "kmplayer",
        "foobar",
        "spotify",
        "youtube",
        "obs",
        "shotcut",
    ];
    for kw in media_kw.iter() {
        if name_lower.contains(kw) || path_lower.contains(kw) {
            return "media";
        }
    }
    let design_kw = [
        "photoshop",
        "illustrator",
        "figma",
        "sketch",
        "indesign",
        "canva",
        "pixso",
        "mastergo",
    ];
    for kw in design_kw.iter() {
        if name_lower.contains(kw) || path_lower.contains(kw) {
            return "design";
        }
    }
    "utility"
}

// ============================================================
// 🔍 辅助：批量解析 .lnk 目标（scan_installed_software 用）
// ============================================================

/// B1（v0.4.3）：解析 PowerShell 输出 → {lnk 路径 → 目标路径}。
/// 输出协议（Batch 2.1 扩展）：每个快捷方式一行
/// `<索引>\t<目标路径>\t<启动参数>\t<工作目录>`（目标可为空 = 该项解析失败）。
/// 配对按**标记索引**而非行序——旧实现先把空行 filter 掉再按原始索引 zip，
/// 任一快捷方式解析为空，其后所有条目错位一位（图标/类型/文件夹重判/启动全错）。
/// 纯函数：无文件系统访问、无进程调用，可独立测试（见 lnk_resolve_tests.rs）。
/// 目标是否存在（exists()）由调用方过滤——本函数只负责**稳定身份对应**。
/// 字段数兼容：少于 4 个字段时缺省项为空串（旧格式 2 字段行仍可解析）。
pub(crate) fn parse_lnk_output(
    lnk_paths: &[&str],
    stdout: &str,
) -> std::collections::HashMap<String, LnkResolution> {
    use std::collections::HashMap;
    let mut out: HashMap<String, LnkResolution> = HashMap::new();
    for line in stdout.lines() {
        let line = line.trim_end();
        if line.trim().is_empty() {
            continue;
        }
        let mut parts = line.splitn(4, '\t');
        // 无索引或索引不是数字的行（PowerShell 警告等杂讯）一律忽略，不参与配对
        let Some(idx) = parts.next() else { continue };
        let Ok(i) = idx.trim().parse::<usize>() else {
            continue;
        };
        let Some(lnk) = lnk_paths.get(i) else {
            continue;
        };
        let target = parts.next().map(|s| s.trim()).unwrap_or("");
        if target.is_empty() {
            // 解析失败/空目标：该快捷方式无映射，但**绝不影响其他条目**
            continue;
        }
        let args = parts.next().map(|s| s.trim()).unwrap_or("").to_string();
        let workdir = parts.next().map(|s| s.trim()).unwrap_or("").to_string();
        out.insert(
            (*lnk).to_string(),
            LnkResolution {
                target: target.to_string(),
                args,
                workdir,
            },
        );
    }
    out
}

#[cfg(windows)]
const LNK_RESOLVE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

/// 等待子进程退出；超时则 kill 并返回 false（B1：旧实现无限等待，
/// PowerShell/COM 卡死会挂住整个扫描）。
#[cfg(windows)]
pub(crate) fn wait_child_with_timeout(
    child: &mut std::process::Child,
    timeout: std::time::Duration,
) -> bool {
    use std::sync::atomic::Ordering;
    let start = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return true,
            Ok(None) => {
                let elapsed = start.elapsed();
                // Batch 2.4（并发审计 D4）：取消时主动终止当前 PS 子进程——
                // 否则单批最长阻塞至 60s 超时，"取消"变成"取消等待"。
                // 300ms 宽限：给刚发生的正常退出留窗口。
                if SCAN_CANCELLED.load(Ordering::Relaxed)
                    && elapsed >= std::time::Duration::from_millis(300)
                {
                    let _ = child.kill();
                    let _ = child.wait();
                    eprintln!("[scan] 取消：当前解析子进程已终止");
                    return false;
                }
                if elapsed >= timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    return false;
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            Err(_) => return false,
        }
    }
}

/// Batch 2.1：.lnk 批量解析**唯一核心**（路径版）。
/// 返回原始解析结果（不做 exists 过滤——存在性判定与降级策略由调用方决定）。
/// Batch 1-B 三修复全部保留：①索引标记协议 ②UTF-8 输出 ③每轮变量重置 + 超时。
/// Batch 2.1 扩展：协议升级为 4 字段（目标/参数/工作目录），供入口身份使用。
/// Batch 2.1 实测修复：**按累计长度分批**——Windows CreateProcess 命令行上限
/// 32767 字符，140+ 条 lnk 的单条 -Command 会超限导致 PowerShell 启动即失败、
/// 整体空输出（真实机器 147 条全灭的实测根因）。单批 ≤24000 字符，批内索引
/// 从 0 重新编号，解析按本批配对。
#[cfg(windows)]
pub(crate) fn resolve_lnk_paths_raw(
    paths: &[String],
    on_batch: Option<&(dyn Fn(usize, usize) + '_)>,
) -> std::collections::HashMap<String, LnkResolution> {
    use std::collections::HashMap;
    let mut out: HashMap<String, LnkResolution> = HashMap::new();
    let lnk_paths: Vec<&str> = paths
        .iter()
        .filter(|p| std::path::Path::new(p).extension().and_then(|e| e.to_str()) == Some("lnk"))
        .map(|s| s.as_str())
        .collect();
    if lnk_paths.is_empty() {
        return out;
    }
    let mut batches: Vec<Vec<&str>> = Vec::new();
    let mut cur: Vec<&str> = Vec::new();
    let mut cur_len = 0usize;
    for p in &lnk_paths {
        cur_len += p.len() + 260; // 片段样板的保守估算
        cur.push(p);
        if cur_len > 24_000 {
            batches.push(std::mem::take(&mut cur));
            cur_len = 0;
        }
    }
    if !cur.is_empty() {
        batches.push(cur);
    }
    for (bi, batch) in batches.iter().enumerate() {
        let mut ps = String::from(
            "[Console]::OutputEncoding=[System.Text.Encoding]::UTF8; \
             $ErrorActionPreference='SilentlyContinue'; \
             $sh = New-Object -ComObject WScript.Shell; ",
        );
        for (i, lnk) in batch.iter().enumerate() {
            let escaped = lnk.replace('\'', "''");
            ps.push_str(&format!(
                "$s = $null; $t = $null; $a = $null; $w = $null; \
                 $s = $sh.CreateShortcut('{escaped}'); \
                 $t = $s.TargetPath; if ($t) {{ $t = [Environment]::ExpandEnvironmentVariables($t) }}; \
                 $a = $s.Arguments; \
                 $w = $s.WorkingDirectory; if ($w) {{ $w = [Environment]::ExpandEnvironmentVariables($w) }}; \
                 Write-Output (\"{i}`t$t`t$a`t$w\"); "
            ));
        }
        let mut child = match std::process::Command::new("powershell")
            .args(&["-NoProfile", "-NonInteractive", "-Command", &ps])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .creation_flags(0x08000000)
            .spawn()
        {
            Ok(c) => c,
            Err(e) => {
                eprintln!("[scan] lnk 目标解析进程启动失败: {e}");
                continue;
            }
        };
        // 管道必须并行排空：先 wait 后读会在输出超过管道缓冲区时死锁
        let stdout_pipe = child.stdout.take();
        let stderr_pipe = child.stderr.take();
        let t_out = std::thread::spawn(move || {
            stdout_pipe
                .map(|mut s| {
                    let mut buf = Vec::new();
                    let _ = std::io::Read::read_to_end(&mut s, &mut buf);
                    buf
                })
                .unwrap_or_default()
        });
        let t_err = std::thread::spawn(move || {
            stderr_pipe
                .map(|mut s| {
                    let mut buf = Vec::new();
                    let _ = std::io::Read::read_to_end(&mut s, &mut buf);
                    buf
                })
                .unwrap_or_default()
        });
        let completed = wait_child_with_timeout(&mut child, LNK_RESOLVE_TIMEOUT);
        let stdout_bytes = t_out.join().unwrap_or_default();
        let stderr_bytes = t_err.join().unwrap_or_default();
        if !completed {
            eprintln!(
                "[scan] lnk 目标解析超时（{}s）已终止，本批 {} 条按解析失败处理",
                LNK_RESOLVE_TIMEOUT.as_secs(),
                batch.len()
            );
            continue;
        }
        // 索引标记使解析对截断/部分失败免疫：每行可独立验证归属。
        // 批内索引从 0 编号 → 按本批 paths 配对。
        let batch_refs: Vec<&str> = batch.iter().copied().collect();
        let stdout = String::from_utf8_lossy(&stdout_bytes).to_string();
        for (k, v) in parse_lnk_output(&batch_refs, &stdout) {
            out.insert(k, v);
        }
        if !stderr_bytes.is_empty() && out.len() < batch.len() {
            eprintln!(
                "[scan] lnk 解析批 stderr（前 200 字）: {}",
                String::from_utf8_lossy(&stderr_bytes[..stderr_bytes.len().min(200)])
            );
        }
        if let Some(cb) = on_batch {
            cb(out.len(), batches.len());
        }
        // Batch 2.3：批间取消检查——PS 单批不可中途安全终止（≤数秒），
        // 取消在下一批边界生效；这是"真正终止"而非"取消等待"，
        // 已完成批次的结果保留为部分结果。
        if SCAN_CANCELLED.load(std::sync::atomic::Ordering::Relaxed) {
            eprintln!(
                "[scan] 收到取消信号，剩余 {} 批跳过",
                batches.len() - bi - 1
            );
            break;
        }
    }
    out
}

#[cfg(not(windows))]
pub(crate) fn resolve_lnk_paths_raw(
    _paths: &[String],
    _on_batch: Option<&(dyn Fn(usize, usize) + '_)>,
) -> std::collections::HashMap<String, LnkResolution> {
    std::collections::HashMap::new()
}

// ============================================================
// 🔍 辅助：常用 Windows 目录（开始菜单 / 桌面 / ProgramFiles）
// ============================================================
mod dirs_known {
    use std::path::PathBuf;
    pub fn start_menu_user() -> PathBuf {
        std::env::var("APPDATA")
            .map(|a| PathBuf::from(a).join("Microsoft\\Windows\\Start Menu"))
            .unwrap_or_else(|_| PathBuf::from("C:\\Windows\\Start Menu"))
    }
    pub fn start_menu_common() -> PathBuf {
        std::env::var("PROGRAMDATA")
            .map(|a| PathBuf::from(a).join("Microsoft\\Windows\\Start Menu"))
            .unwrap_or_else(|_| PathBuf::from("C:\\ProgramData\\Microsoft\\Windows\\Start Menu"))
    }
    pub fn desktop_user() -> PathBuf {
        std::env::var("USERPROFILE")
            .map(|p| PathBuf::from(p).join("Desktop"))
            .unwrap_or_else(|_| PathBuf::from("C:\\Users\\Public\\Desktop"))
    }
    pub fn desktop_common() -> PathBuf {
        std::env::var("PUBLIC")
            .map(|p| PathBuf::from(p).join("Desktop"))
            .unwrap_or_else(|_| PathBuf::from("C:\\Users\\Public\\Desktop"))
    }
    pub fn program_files() -> PathBuf {
        std::env::var("ProgramFiles")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("C:\\Program Files"))
    }
    pub fn program_files_x86() -> PathBuf {
        std::env::var("ProgramFiles(x86)")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("C:\\Program Files (x86)"))
    }
}

// ============================================================
// 🔍 辅助：walk_lnk_urls（默认扫描）/ walk_exe_advanced（高级扫描）
// Batch 2.1 重构说明：
//   旧 walk_mixed 的三大噪声源已移除——①所有目录收为 folder 候选
//   ②document/url 混入软件扫描 ③全局按文件名词干去重误删候选。
//   默认扫描只收 .lnk/.url 入口（人工策划的真实启动点），分类交给
//   classify_entry 纯函数；同身份跨来源去重按来源优先级保留首条。
// ============================================================
fn walk_lnk_urls(
    dir: &std::path::Path,
    source: &str,
    out: &mut Vec<(String, String, String)>, // (path, name, source)
    seen_paths: &mut std::collections::HashSet<String>,
    depth: u8,
) {
    use std::fs;
    if depth == 0 {
        return;
    }
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    let mut visited: usize = 0;
    for entry in entries.flatten() {
        // Batch 2.3：遍历级取消检查（每 256 项一次，开销可忽略）
        visited += 1;
        if visited % 256 == 0 && SCAN_CANCELLED.load(std::sync::atomic::Ordering::Relaxed) {
            return;
        }
        let path = entry.path();
        let Ok(ft) = entry.file_type() else { continue };
        if ft.is_dir() {
            walk_lnk_urls(&path, source, out, seen_paths, depth - 1);
        } else if ft.is_file() {
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| e.to_lowercase())
                .unwrap_or_default();
            if ext != "lnk" && ext != "url" {
                continue; // 只收真实启动入口：快捷方式与网址
            }
            let path_str = path.to_string_lossy().to_string();
            if seen_paths.insert(path_str.clone()) {
                let name = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("Unknown")
                    .to_string();
                out.push((path_str, name, source.to_string()));
            }
        }
    }
}

fn walk_exe_advanced(
    dir: &std::path::Path,
    source: &str,
    out: &mut Vec<(String, String, String)>,
    seen_paths: &mut std::collections::HashSet<String>,
    depth: u8,
) {
    use std::fs;
    if depth == 0 {
        return;
    }
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(ft) = entry.file_type() else { continue };
        if ft.is_dir() {
            walk_exe_advanced(&path, source, out, seen_paths, depth - 1);
        } else if ft.is_file() && path.extension().and_then(|e| e.to_str()) == Some("exe") {
            let name_lc = path
                .file_stem()
                .and_then(|s| s.to_str())
                .map(|s| s.to_lowercase())
                .unwrap_or_default();
            // 精确辅助名剔除（高级扫描专属；模糊模式不删除，降级为「需要确认」）
            let skip = matches!(
                name_lc.as_str(),
                "uninstall"
                    | "uninst"
                    | "setup"
                    | "installer"
                    | "update"
                    | "updater"
                    | "crashpad_handler"
                    | "vcredist"
            );
            if skip {
                continue;
            }
            let path_str = path.to_string_lossy().to_string();
            if seen_paths.insert(path_str.clone()) {
                let name = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("Unknown")
                    .to_string();
                out.push((path_str, name, source.to_string()));
            }
        }
    }
}

/// Batch 2.1：候选 → ScannedApp（分类 + 身份 + 图标提取）。
/// `raw` = (path, name, source)；lnk 目标批量解析一次完成。
pub(crate) fn build_candidates(raw: Vec<(String, String, String)>) -> Vec<ScannedApp> {
    build_candidates_with(raw, None)
}

/// Batch 2.3：带批次进度回调的候选构建（进度事件/取消都从这里透传）。
fn build_candidates_with(
    raw: Vec<(String, String, String)>,
    on_batch: Option<&(dyn Fn(usize, usize) + '_)>,
) -> Vec<ScannedApp> {
    let paths: Vec<String> = raw.iter().map(|(p, _, _)| p.clone()).collect();
    #[allow(unused_mut)]
    let mut resolutions: std::collections::HashMap<String, LnkResolution> =
        resolve_lnk_paths_raw(&paths, on_batch);

    // ---- 分类 + 同身份去重（来源优先级 = 传入顺序）----
    let mut results: Vec<ScannedApp> = Vec::new();
    let mut seen_identity: std::collections::HashSet<String> = std::collections::HashSet::new();
    for (path, name, source) in raw {
        let is_url_file = std::path::Path::new(&path)
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_lowercase())
            .as_deref()
            == Some("url");
        let res = resolutions.get(&path).cloned().unwrap_or_default();
        let target_exists = !res.target.is_empty() && std::path::Path::new(&res.target).exists();
        let target_is_dir = target_exists && std::path::Path::new(&res.target).is_dir();
        // Batch 2.4：打包应用候选（UWP/MSIX，source=Store/应用，path=AppID）。
        // 四验证门已过（识别/启动/去重/失效），受控接入高级扫描。
        let direct_uwp = !is_url_file && source == "Store/应用" && classify_appid(&path).is_some();
        // Batch 2.3：直接 exe 候选（高级扫描的目录遍历产物，无 lnk 包装）。
        // 目标即自身；状态按模糊辅助模式降级（helper/crash 等 → 需要确认，不删除）。
        let direct_exe = !is_url_file
            && res.target.is_empty()
            && std::path::Path::new(&path)
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| e.to_lowercase())
                .as_deref()
                == Some("exe")
            && std::path::Path::new(&path).is_file();
        let cls = if direct_uwp {
            EntryClassification {
                app_type: "uwp",
                app_subtype: "",
                status: "identified",
                is_launchable: true,
            }
        } else if direct_exe {
            let name_lc = name.to_lowercase();
            EntryClassification {
                app_type: "app",
                app_subtype: detect_subtype_from_path(std::path::Path::new(&path), "app"),
                status: advanced_status_for(&name_lc),
                is_launchable: true,
            }
        } else {
            let Some(c) = classify_entry(
                is_url_file,
                &res.target,
                &res.args,
                target_exists,
                target_is_dir,
            ) else {
                continue; // 文件夹/文档目标：非软件候选
            };
            c
        };
        // 入口身份（同身份跨来源只保留优先级最高的一条）
        let url_or_target = if direct_exe || direct_uwp {
            path.clone() // 直接 exe：目标即自身；打包应用：AppID 即目标
        } else if is_url_file && res.target.is_empty() {
            win_icon::parse_url_shortcut(&path).unwrap_or_default()
        } else {
            res.target.clone()
        };
        let identity = if cls.app_type == "url" {
            entry_identity_key("url", &url_or_target, "")
        } else if cls.app_type == "uwp" {
            entry_identity_key("uwp", &url_or_target, "")
        } else if !res.target.is_empty() {
            entry_identity_key("exe", &res.target, &res.args)
        } else {
            // 无法解析：退化为 lnk 文件自身身份（同文件重复出现仍可去重）
            entry_identity_key("lnk", &path, "")
        };
        if !seen_identity.insert(identity) {
            continue;
        }
        results.push(ScannedApp {
            name,
            path,
            icon_path: String::new(),
            args: String::new(),
            source,
            app_type: cls.app_type.to_string(),
            app_subtype: cls.app_subtype.to_string(),
            is_launchable: cls.is_launchable,
            target: url_or_target,
            target_args: res.args,
            workdir: res.workdir,
            status: cls.status.to_string(),
            already_imported: false,
            in_trash: false,
            trash_id: None,
        });
    }

    // ---- 图标提取（目标优先，失败回退入口自身）----
    #[cfg(windows)]
    {
        let mut to_extract: Vec<(String, String)> = Vec::new();
        for app in results.iter() {
            let icon_src = if !app.target.is_empty()
                && (app.app_type != "url")
                && std::path::Path::new(&app.target).is_file()
            {
                app.target.clone()
            } else {
                app.path.clone()
            };
            to_extract.push((icon_src, app.path.clone()));
        }
        let exe_paths: Vec<String> = to_extract.iter().map(|(t, _)| t.clone()).collect();
        let extracted = win_icon::extract_batch_to_cache(&exe_paths);
        let icon_map: std::collections::HashMap<String, PathBuf> = extracted.into_iter().collect();
        for (icon_src, key) in to_extract.iter() {
            if let Some(a) = results.iter_mut().find(|a| &a.path == key) {
                if let Some(cached) = icon_map.get(icon_src) {
                    a.icon_path = cached.to_string_lossy().to_string();
                }
            }
        }
    }

    results.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    results
}

/// Batch 2.1：默认扫描来源（raw 层，供命令组装进度事件）。
pub(crate) fn scan_default_raw() -> Vec<(String, String, String)> {
    let mut raw: Vec<(String, String, String)> = Vec::new();
    let mut seen_paths: std::collections::HashSet<String> = std::collections::HashSet::new();
    let sources: [(std::path::PathBuf, &str); 4] = [
        (dirs_known::start_menu_user().join("Programs"), "开始菜单"),
        (
            dirs_known::start_menu_common().join("Programs"),
            "开始菜单(所有用户)",
        ),
        (dirs_known::desktop_user(), "桌面"),
        (dirs_known::desktop_common(), "桌面(公共)"),
    ];
    for (dir, source) in sources.iter() {
        if dir.exists() {
            walk_lnk_urls(dir, source, &mut raw, &mut seen_paths, 3);
        }
    }
    raw
}

/// Batch 2.3：Program Files 遍历（raw 层）。深度 2：PF\厂商\主程序.exe。
pub(crate) fn scan_pf_raw() -> Vec<(String, String, String)> {
    let mut raw: Vec<(String, String, String)> = Vec::new();
    let mut seen_paths: std::collections::HashSet<String> = std::collections::HashSet::new();
    for (dir, source) in [
        (dirs_known::program_files(), "Program Files"),
        (dirs_known::program_files_x86(), "Program Files (x86)"),
    ] {
        if dir.exists() {
            walk_exe_advanced(&dir, source, &mut raw, &mut seen_paths, 2);
        }
    }
    raw
}

/// Batch 2.3：PATH 扫描（raw 层）。默认关闭、用户主动开启才执行；
/// 大量开发命令场景——条目上限 2000 防失控，命中模糊辅助模式降级「需要确认」。
pub(crate) fn scan_path_raw() -> Vec<(String, String, String)> {
    let mut out: Vec<(String, String, String)> = Vec::new();
    let mut seen_paths: std::collections::HashSet<String> = std::collections::HashSet::new();
    if let Ok(path_var) = std::env::var("PATH") {
        'outer: for p in std::env::split_paths(&path_var) {
            let Ok(entries) = std::fs::read_dir(&p) else {
                continue;
            };
            for entry in entries.flatten() {
                if SCAN_CANCELLED.load(std::sync::atomic::Ordering::Relaxed) {
                    break 'outer;
                }
                if out.len() >= 2000 {
                    eprintln!("[scan] PATH 候选达上限 2000，剩余跳过");
                    break 'outer;
                }
                let Ok(ft) = entry.file_type() else { continue };
                if !ft.is_file() {
                    continue;
                }
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) != Some("exe") {
                    continue;
                }
                let path_str = path.to_string_lossy().to_string();
                if seen_paths.insert(path_str.clone()) {
                    let name = path
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("Unknown")
                        .to_string();
                    out.push((path_str, name, "PATH".to_string()));
                }
            }
        }
    }
    out
}

/// Batch 2.3：自定义目录扫描（raw 层）。返回 (raw, truncated)。
/// 保护：深度 ≤4、条目 ≤20000、符号链接不跟随（entry.file_type 对 symlink
/// 返回 is_dir=false，天然防循环）、UNC 拒绝、失效/无权限目录静默跳过。
pub(crate) fn scan_custom_raw(
    dir: &std::path::Path,
) -> Result<(Vec<(String, String, String)>, bool), String> {
    use crate::data_root::is_unc_path;
    let dir = dir.to_path_buf();
    if !dir.is_absolute() {
        return Err(format!("路径必须是完整路径（含盘符）：{}", dir.display()));
    }
    if is_unc_path(&dir) {
        return Err("暂不支持网络位置（以双反斜杠开头），请选择本地磁盘目录。".to_string());
    }
    if !dir.exists() || !dir.is_dir() {
        return Err(format!("目录不存在或不是文件夹：{}", dir.display()));
    }
    let mut raw: Vec<(String, String, String)> = Vec::new();
    let mut seen_paths: std::collections::HashSet<String> = std::collections::HashSet::new();
    // 自定义目录同时收 .lnk/.url（入口）与 .exe（便携软件本体）
    let truncated = walk_custom(
        dir.as_path(),
        "自定义目录",
        &mut raw,
        &mut seen_paths,
        4,
        20_000,
    );
    Ok((raw, truncated))
}

/// 自定义目录遍历：lnk/url/exe 三类；返回是否触发数量保护。
fn walk_custom(
    dir: &std::path::Path,
    source: &str,
    out: &mut Vec<(String, String, String)>,
    seen_paths: &mut std::collections::HashSet<String>,
    depth: u8,
    cap: usize,
) -> bool {
    if depth == 0 || out.len() >= cap {
        return out.len() >= cap;
    }
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return false, // 无权限/失效：跳过该子目录，不中断整体
    };
    let mut visited: usize = 0;
    for entry in entries.flatten() {
        visited += 1;
        if visited % 256 == 0 && SCAN_CANCELLED.load(std::sync::atomic::Ordering::Relaxed) {
            return false;
        }
        if out.len() >= cap {
            return true;
        }
        let path = entry.path();
        let Ok(ft) = entry.file_type() else { continue };
        if ft.is_dir() {
            if walk_custom(&path, source, out, seen_paths, depth - 1, cap) {
                return true;
            }
        } else if ft.is_file() {
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| e.to_lowercase())
                .unwrap_or_default();
            if ext != "lnk" && ext != "url" && ext != "exe" {
                continue;
            }
            let path_str = path.to_string_lossy().to_string();
            if seen_paths.insert(path_str.clone()) {
                let name = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("Unknown")
                    .to_string();
                out.push((path_str, name, source.to_string()));
            }
        }
    }
    false
}

/// Batch 2.1：默认扫描核心（测试与复用入口）。
pub(crate) fn scan_default_core() -> Vec<ScannedApp> {
    build_candidates(scan_default_raw())
}

/// Batch 2.2：Program Files 高级扫描核心（测试与复用入口）。
pub(crate) fn scan_advanced_core() -> Vec<ScannedApp> {
    build_candidates(scan_pf_raw())
}

/// Batch 2.1：应用库现有条目的身份索引。
/// identity → [(id, deleted_at)]。lnk 行重新解析目标（同一 PS 批次），
/// 非 lnk 行按规范化存储路径。查询只读，不修改任何用户数据。
#[cfg_attr(not(windows), allow(unused_variables))]
pub(crate) fn compute_library_identities(
    db: &Db,
) -> std::collections::HashMap<String, Vec<(i64, Option<i64>)>> {
    let rows = match read_library_rows(db) {
        Ok(r) => r,
        Err(_) => return std::collections::HashMap::new(),
    };
    let mut out: std::collections::HashMap<String, Vec<(i64, Option<i64>)>> =
        std::collections::HashMap::new();
    // lnk 行批量解析一次（目标在磁盘上的快捷方式仍可解析；已删除的 lnk 文件
    // 解析为空 → 身份退化为规范化存储路径，仍可与同路径候选去重）
    #[cfg(windows)]
    let lnk_paths: Vec<String> = rows
        .iter()
        .filter(|(_, path, _, _)| {
            std::path::Path::new(path)
                .extension()
                .and_then(|e| e.to_str())
                == Some("lnk")
        })
        .map(|(_, path, _, _)| path.clone())
        .collect();
    #[cfg(windows)]
    let resolved = resolve_lnk_paths_raw(&lnk_paths, None);
    for (id, path, args, app_type) in rows {
        let ext_is_lnk = std::path::Path::new(&path)
            .extension()
            .and_then(|e| e.to_str())
            == Some("lnk");
        let identity = if app_type == "url" {
            entry_identity_key("url", &path, &args)
        } else if app_type == "uwp" {
            entry_identity_key("uwp", &path, &args)
        } else if ext_is_lnk {
            #[cfg(windows)]
            {
                match resolved.get(&path) {
                    Some(r) if !r.target.is_empty() => {
                        entry_identity_key("exe", &r.target, &r.args)
                    }
                    _ => entry_identity_key("lnk", &path, &args),
                }
            }
            #[cfg(not(windows))]
            {
                entry_identity_key("lnk", &path, &args)
            }
        } else {
            entry_identity_key("exe", &path, &args)
        };
        out.entry(identity).or_default().push((id, None)); // deleted_at 由下方统一回填
    }
    // 补 deleted_at（一次查询，避免 N+1）
    if let Ok(deleted) = read_deleted_map(db) {
        for v in out.values_mut() {
            for (id, d) in v.iter_mut() {
                *d = deleted.get(id).copied().flatten();
            }
        }
    }
    out
}

fn read_library_rows(db: &Db) -> rusqlite::Result<Vec<(i64, String, String, String)>> {
    let conn = db.conn.lock().unwrap();
    let mut stmt = conn.prepare("SELECT id, path, args, app_type FROM apps")?;
    let it = stmt.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
        ))
    })?;
    Ok(it.flatten().collect())
}

fn read_deleted_map(db: &Db) -> rusqlite::Result<std::collections::HashMap<i64, Option<i64>>> {
    let conn = db.conn.lock().unwrap();
    let mut stmt = conn.prepare("SELECT id, deleted_at FROM apps")?;
    let it = stmt.query_map([], |row| {
        Ok((row.get::<_, i64>(0)?, row.get::<_, Option<i64>>(1)?))
    })?;
    Ok(it.flatten().collect())
}

/// 候选的库比对身份键组（与 scan/导入两侧共用，保证口径一致）。
/// 双层键：主键 = 目标+参数（跨来源去重）；次键 = 入口文件自身路径
/// （库行 lnk 文件丢失、无法重解析时的兜底匹配）。
pub(crate) fn candidate_identity_keys(c: &ScannedApp) -> Vec<String> {
    let mut keys = Vec::new();
    if c.app_type == "url" {
        let url = if c.target.is_empty() {
            &c.path
        } else {
            &c.target
        };
        keys.push(entry_identity_key("url", url, ""));
    } else if c.app_type == "uwp" {
        // 打包应用身份 = AppID；与 exe/lnk 身份永不合并（门3 裁决）
        keys.push(entry_identity_key("uwp", &c.path, &c.target_args));
    } else {
        if !c.target.is_empty() {
            keys.push(entry_identity_key("exe", &c.target, &c.target_args));
        }
        keys.push(entry_identity_key("lnk", &c.path, ""));
    }
    keys
}

/// Batch 2.1：导入摘要（幂等导入的结果口径）
#[derive(Debug, Clone, serde::Serialize)]
pub struct ImportSummary {
    pub added: usize,
    pub already_exists: usize,
    pub in_trash: usize,
    pub failed: Vec<String>,
}

/// Batch 2.1：批量导入核心（幂等）。
/// 每条候选先做身份比对：库中同身份有效条目 → already_exists；
/// 同身份条目全部在回收站 → in_trash（提示恢复，不自动恢复）；
/// 否则创建。重复执行同一批导入 → 第二次全部 already_exists（幂等）。
pub(crate) fn import_candidates_core(
    db: &Db,
    candidates: &[ScannedApp],
    category_id: Option<i64>,
) -> ImportSummary {
    let lib = compute_library_identities(db);
    let mut summary = ImportSummary {
        added: 0,
        already_exists: 0,
        in_trash: 0,
        failed: Vec::new(),
    };
    for c in candidates {
        // 双层键任一命中即视为同身份：主键（目标+参数）跨来源去重，
        // 次键（入口文件路径）兜底 lnk 文件无法重解析的场景。
        let keys = candidate_identity_keys(c);
        let hits: Vec<&Vec<(i64, Option<i64>)>> = keys.iter().filter_map(|k| lib.get(k)).collect();
        if hits.iter().any(|e| e.iter().any(|(_, d)| d.is_none())) {
            summary.already_exists += 1;
            continue;
        }
        if !hits.is_empty() {
            summary.in_trash += 1;
            continue;
        }
        match db.create_app(
            &c.name,
            &c.path,
            &c.icon_path,
            &c.args,
            category_id,
            &c.app_type,
            &c.app_subtype,
        ) {
            Ok(_) => summary.added += 1,
            Err(e) => summary.failed.push(format!("{}: {}", c.name, e)),
        }
    }
    summary
}

// ============================================================
// 🛡️ 1. 扫描系统软件（默认=开始菜单+桌面入口；高级=Program Files）
// ============================================================
#[tauri::command]
/// 库比对（只读）：标记已在库 / 回收站命中（双层键任一命中即算）
fn enrich_with_library(state: &AppState, results: &mut [ScannedApp]) {
    let lib = compute_library_identities(&state.db.lock().unwrap());
    apply_library_hits(results, &lib);
}

/// 把库身份快照落到候选上（纯函数，便于回归测试）。
/// 有效条目优先于回收站条目：只要同身份存在一条未删除记录，就算「已在库」，
/// 不把候选标成「待恢复」（否则恢复会造出重复条目）。
pub(crate) fn apply_library_hits(
    results: &mut [ScannedApp],
    lib: &std::collections::HashMap<String, Vec<(i64, Option<i64>)>>,
) {
    for a in results.iter_mut() {
        let mut active = false;
        let mut trashed: Option<i64> = None;
        for key in candidate_identity_keys(a) {
            let Some(entries) = lib.get(&key) else { continue };
            for (id, deleted_at) in entries {
                match deleted_at {
                    None => active = true,
                    // 同身份可能在回收站有多条：取 id 最大的（最近收录的一条）恢复
                    Some(_) => trashed = Some(trashed.map_or(*id, |t| t.max(*id))),
                }
            }
        }
        a.already_imported = active;
        a.in_trash = !active && trashed.is_some();
        a.trash_id = if active { None } else { trashed };
    }
}

fn emit_scan_progress(app: &tauri::AppHandle, stage: &str, done: usize, total: usize) {
    use tauri::Emitter;
    let _ = app.emit(
        "scan:progress",
        serde_json::json!({ "stage": stage, "done": done, "total": total }),
    );
}

#[tauri::command]
pub fn scan_installed_software(
    state: State<AppState>,
    app: tauri::AppHandle,
    level: Option<String>,
    include_path: Option<bool>,
    include_uwp: Option<bool>,
) -> Result<ScanOutcome, String> {
    let _data_gate = state.data_read()?;
    // Batch 2.4（D2）：串行化——拿到唯一执行权才复位取消旗标
    let _scan_guard = ScanGuard::acquire()?;
    let advanced = level.as_deref() == Some("advanced");
    let mut raw = if advanced {
        let mut r = scan_pf_raw();
        emit_scan_progress(&app, "enumerate", r.len(), 0);
        // PATH 开关：默认关闭，用户主动开启才执行（决策 5/§三）
        if include_path.unwrap_or(false) {
            let path_raw = scan_path_raw();
            emit_scan_progress(&app, "enumerate-path", path_raw.len(), 0);
            r.extend(path_raw);
        }
        // Store 应用开关：默认关闭（Batch 2.4 受控接入，四门已过）
        if include_uwp.unwrap_or(false) {
            let uwp_raw = scan_uwp_raw()?;
            emit_scan_progress(&app, "enumerate-uwp", uwp_raw.len(), 0);
            r.extend(uwp_raw);
        }
        r
    } else {
        let r = scan_default_raw();
        emit_scan_progress(&app, "enumerate", r.len(), 0);
        r
    };
    let truncated = false;
    let app2 = app.clone();
    let outcome_apps = build_candidates_with(
        std::mem::take(&mut raw),
        Some(&|done: usize, total: usize| {
            emit_scan_progress(&app2, "resolve", done, total);
        }),
    );
    emit_scan_progress(&app, "icons", outcome_apps.len(), 0);
    let cancelled = SCAN_CANCELLED.load(std::sync::atomic::Ordering::Relaxed);
    let mut results = outcome_apps;
    // Batch 2.4 修正：打包应用身份验证（fail-closed——验证失败全部降级「需要确认」）
    if include_uwp.unwrap_or(false) {
        let uwp_ids: Vec<String> = results
            .iter()
            .filter(|a| a.app_type == "uwp")
            .map(|a| a.path.clone())
            .collect();
        if !uwp_ids.is_empty() {
            match verify_uwp_identities(&uwp_ids) {
                Ok(v) => apply_uwp_verification(&mut results, &v),
                Err(e) => {
                    eprintln!("[scan] 打包应用身份验证失败，全部降级需要确认: {e}");
                    for a in results.iter_mut().filter(|a| a.app_type == "uwp") {
                        a.status = "needs_confirmation".to_string();
                        a.is_launchable = false;
                    }
                }
            }
        }
    }
    enrich_with_library(&state, &mut results);
    Ok(ScanOutcome {
        apps: results,
        cancelled,
        truncated,
    })
}

/// Batch 2.3：自定义目录扫描（用户主动选择；不自动导入任何结果）
#[tauri::command]
pub fn scan_custom_dir(
    state: State<AppState>,
    app: tauri::AppHandle,
    path: String,
) -> Result<ScanOutcome, String> {
    let _data_gate = state.data_read()?;
    let _scan_guard = ScanGuard::acquire()?;
    let (raw, truncated) = scan_custom_raw(std::path::Path::new(&path))?;
    emit_scan_progress(&app, "enumerate", raw.len(), 0);
    let app2 = app.clone();
    let outcome_apps = build_candidates_with(
        raw,
        Some(&|done: usize, total: usize| {
            emit_scan_progress(&app2, "resolve", done, total);
        }),
    );
    emit_scan_progress(&app, "icons", outcome_apps.len(), 0);
    let cancelled = SCAN_CANCELLED.load(std::sync::atomic::Ordering::Relaxed);
    let mut results = outcome_apps;
    enrich_with_library(&state, &mut results);
    Ok(ScanOutcome {
        apps: results,
        cancelled,
        truncated,
    })
}

/// Batch 2.3：真正终止扫描（遍历每 256 项 / 解析每批边界生效）
#[tauri::command]
pub fn cancel_scan() {
    SCAN_CANCELLED.store(true, std::sync::atomic::Ordering::Relaxed);
}

// ============================================================
// 🛡️ 2. 列出软件（支持搜索 + 分类过滤）
// ============================================================
#[tauri::command]
pub fn list_apps(
    state: State<AppState>,
    query: Option<String>,
    category_id: Option<i64>,
) -> Result<Vec<db::AppMeta>, String> {
    // [data-gate:shared] Phase 2C-3 统一数据操作门（锁顺序: gate → master_wrap_gate → db → conn）
    let _data_gate = state.data_read()?;

    let db = state.db.lock().unwrap();
    let q = query.unwrap_or_default();
    db.list_apps(&q, category_id).map_err(|e| e.to_string())
}

// ============================================================
// 🛡️ 3. 列出软件 + 一次性转好全部图标 base64 data URL（前端零异步）
// ============================================================
#[derive(serde::Serialize)]
pub struct AppWithIcon {
    id: i64,
    name: String,
    path: String,
    icon_path: String,
    icon_data_url: Option<String>,
    args: String,
    category_id: Option<i64>,
    app_type: String,
    app_subtype: String,
    use_count: i64,
    last_used_at: i64,
    created_at: i64,
    /// Phase 2D：运行时派生可用性（来自 AppMeta.available）
    available: bool,
}

fn read_icon_as_data_url_inner(path: String) -> Option<String> {
    let bytes = std::fs::read(&path).ok()?;
    let mime = if path.to_lowercase().ends_with(".png") {
        "image/png"
    } else if path.to_lowercase().ends_with(".jpg") || path.to_lowercase().ends_with(".jpeg") {
        "image/jpeg"
    } else if path.to_lowercase().ends_with(".gif") {
        "image/gif"
    } else if path.to_lowercase().ends_with(".webp") {
        "image/webp"
    } else {
        "image/png"
    };
    let b64 = base64::Engine::encode(&BASE64, &bytes);
    Some(format!("data:{};base64,{}", mime, b64))
}

#[tauri::command]
pub fn list_apps_with_icons(
    state: State<AppState>,
    query: String,
    category_id: Option<i64>,
) -> Result<Vec<AppWithIcon>, String> {
    // [data-gate:shared] Phase 2C-3 统一数据操作门（锁顺序: gate → master_wrap_gate → db → conn）
    let _data_gate = state.data_read()?;

    let db = state.db.lock().unwrap();
    let apps = db
        .list_apps(&query, category_id)
        .map_err(|e| e.to_string())?;
    let result: Vec<AppWithIcon> = apps
        .into_iter()
        .map(|a| {
            let icon_data_url = if a.icon_path.is_empty() {
                None
            } else {
                read_icon_as_data_url_inner(a.icon_path.clone())
            };
            AppWithIcon {
                id: a.id,
                name: a.name,
                path: a.path,
                icon_path: a.icon_path,
                icon_data_url,
                args: a.args,
                category_id: a.category_id,
                app_type: a.app_type,
                app_subtype: a.app_subtype,
                use_count: a.use_count,
                last_used_at: a.last_used_at,
                created_at: a.created_at,
                available: a.available,
            }
        })
        .collect();
    Ok(result)
}

// ============================================================
// 🧭 Phase 2D：应用生命周期失效治理
// ============================================================
// 重新关联：只改 path，保留原 Drawer 元数据（名称/分类/统计/图标选择）。
// 不做自动删除，不冒充卸载器——真正卸载第三方软件永远是用户与系统的事。
#[tauri::command]
pub fn relink_app(state: State<AppState>, id: i64, new_path: String) -> Result<(), String> {
    let p = std::path::Path::new(&new_path);
    if !p.exists() {
        return Err("所选路径不存在，无法关联".to_string());
    }
    let db = state.db.lock().unwrap();
    db.update_app_path(id, &new_path).map_err(|e| e.to_string())
}

/// 打开原位置：从记录路径向上找最近的"仍存在"的祖先目录并打开。
/// 全部祖先都失效时返回明确错误，绝不假装成功。
#[tauri::command]
pub fn open_app_location(state: State<AppState>, id: i64) -> Result<(), String> {
    let (name, path, app_type) = {
        let db = state.db.lock().unwrap();
        let apps = db.list_apps("", None).map_err(|e| e.to_string())?;
        match apps.into_iter().find(|a| a.id == id) {
            Some(a) => (a.name, a.path, a.app_type),
            None => return Err("应用不存在或已移除".to_string()),
        }
    };
    if app_type == "url" {
        return Err("网址类型没有本地位置".to_string());
    }
    let p = std::path::PathBuf::from(&path);
    let mut dir: Option<std::path::PathBuf> = None;
    if p.is_dir() {
        dir = Some(p);
    } else {
        let mut cur = p.parent().map(|x| x.to_path_buf());
        while let Some(d) = cur {
            if d.is_dir() {
                dir = Some(d);
                break;
            }
            cur = d.parent().map(|x| x.to_path_buf());
        }
    }
    let dir = dir.ok_or_else(|| format!("「{}」的原位置已完全失效，请重新关联", name))?;
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        std::process::Command::new("explorer.exe")
            .arg(dir.as_os_str())
            .creation_flags(0x08000000)
            .spawn()
            .map_err(|e| format!("打开位置失败: {}", e))?;
    }
    Ok(())
}

// ============================================================
// 🛡️ 4. 查询 lock 状态 + build 时间戳诊断
// ============================================================
#[tauri::command]
pub fn get_app_status(state: State<AppState>) -> serde_json::Value {
    let unlocked = state.is_unlocked();
    serde_json::json!({
        "unlocked": unlocked,
        "build_timestamp": env!("BUILD_TIMESTAMP"),
    })
}

// ============================================================
// 🛡️ 5. 读图标 → base64 data URL
// ============================================================
#[tauri::command]
pub fn read_icon_as_data_url(path: String) -> Option<String> {
    read_icon_as_data_url_inner(path)
}

// ============================================================
// 🛡️ 6. 原生文件 / 文件夹选择对话框
// ============================================================
#[tauri::command]
pub async fn pick_path(mode: String, title: Option<String>) -> Option<String> {
    let title = title.unwrap_or_else(|| {
        if mode == "folder" {
            "选择文件夹".to_string()
        } else {
            "选择文件".to_string()
        }
    });
    let result = tauri::async_runtime::spawn_blocking(move || {
        if mode == "folder" {
            rfd::FileDialog::new()
                .set_title(&title)
                .pick_folder()
                .map(|p| p.to_string_lossy().to_string())
        } else {
            let lower_title = title.to_lowercase();
            let is_backup_picker =
                lower_title.contains("备份") || lower_title.contains("drawerbox");
            let mut dlg = rfd::FileDialog::new().set_title(&title);
            if is_backup_picker {
                dlg = dlg.add_filter("抽屉柜备份文件", &["drawerbox"]);
                dlg = dlg.add_filter("应用文件", &["exe", "lnk", "url", "bat", "cmd", "msi"]);
            } else {
                dlg = dlg.add_filter("应用文件", &["exe", "lnk", "url", "bat", "cmd", "msi"]);
                dlg = dlg.add_filter("抽屉柜备份文件", &["drawerbox"]);
            }
            dlg = dlg.add_filter("所有文件", &["*"]);
            dlg.pick_file().map(|p| p.to_string_lossy().to_string())
        }
    })
    .await
    .ok()
    .flatten();
    result
}

// ============================================================
// 🛡️ 7. 填充缺失图标（空路径 OR 文件不存在的都重抽）
// ============================================================
#[tauri::command]
pub fn fill_missing_icons(state: State<AppState>) -> Result<usize, String> {
    // [data-gate:shared] Phase 2C-3 统一数据操作门（锁顺序: gate → master_wrap_gate → db → conn）
    let _data_gate = state.data_read()?;

    let apps: Vec<db::AppMeta> = {
        let db = state.db.lock().unwrap();
        db.list_apps("", None).map_err(|e| e.to_string())?
    };
    let missing: Vec<(i64, String)> = apps
        .into_iter()
        .filter(|a| !a.path.is_empty())
        .filter(|a| a.icon_path.is_empty() || !std::path::Path::new(&a.icon_path).exists())
        .map(|a| (a.id, a.path))
        .collect();
    if missing.is_empty() {
        return Ok(0);
    }
    #[cfg(windows)]
    {
        let exe_paths: Vec<String> = missing.iter().map(|(_, p)| p.clone()).collect();
        let extracted = win_icon::extract_batch_to_cache(&exe_paths);
        let icon_map: std::collections::HashMap<String, PathBuf> = extracted.into_iter().collect();
        let mut updated = 0;
        let db = state.db.lock().unwrap();
        for (id, path) in missing {
            if let Some(cached) = icon_map.get(&path) {
                let png = cached.to_string_lossy().to_string();
                if db.update_app_icon(id, &png).is_ok() {
                    updated += 1;
                }
            }
        }
        Ok(updated)
    }
    #[cfg(not(windows))]
    {
        Ok(0)
    }
}

// ============================================================
// 🛡️ 8. 强制清空图标缓存 + 重抽
// ============================================================
#[tauri::command]
pub fn force_reextract_icons(state: State<AppState>) -> Result<usize, String> {
    // [data-gate:shared] Phase 2C-3 统一数据操作门（锁顺序: gate → master_wrap_gate → db → conn）
    let _data_gate = state.data_read()?;

    #[cfg(windows)]
    {
        let cleared = win_icon::clear_all_cache().map_err(|e| e.to_string())?;
        eprintln!("[force_reextract_icons] cleared {} cached icons", cleared);
    }
    let db = state.db.lock().unwrap();
    let count = db.clear_all_icon_paths().map_err(|e| e.to_string())?;
    eprintln!("[force_reextract_icons] cleared {} icon_paths in db", count);
    Ok(count)
}

// ============================================================
// 🛡️ 9. 重启 App（让新代码生效）
// ============================================================
#[tauri::command]
pub fn restart_app(app: AppHandle) -> Result<(), String> {
    use std::process::Command;
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let exe_str = exe.to_string_lossy().to_string();
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        Command::new(&exe_str)
            .creation_flags(0x08000000)
            .spawn()
            .map_err(|e| format!("启动新进程失败：{}", e))?;
    }
    #[cfg(not(windows))]
    {
        Command::new(&exe_str).spawn().map_err(|e| e.to_string())?;
    }
    app.exit(0);
    Ok(())
}

// ============================================================
// 🛡️ 10. 一次性回填所有 app 的 subtype（旧版本没填的）
// ============================================================
#[tauri::command]
pub fn rescan_subtypes(state: State<AppState>) -> Result<usize, String> {
    // [data-gate:shared] Phase 2C-3 统一数据操作门（锁顺序: gate → master_wrap_gate → db → conn）
    let _data_gate = state.data_read()?;

    eprintln!("[rescan_subtypes] ENTER");
    let apps: Vec<db::AppMeta> = {
        let db = state.db.lock().unwrap();
        db.list_apps("", None).map_err(|e| e.to_string())?
    };
    let mut updated = 0;
    let db = state.db.lock().unwrap();
    for a in &apps {
        if !a.app_subtype.is_empty() {
            continue;
        }
        let p = std::path::Path::new(&a.path);
        let new_sub = detect_subtype_from_path(p, &a.app_type).to_string();
        if !new_sub.is_empty() {
            if db
                .update_app(
                    a.id,
                    &a.name,
                    &a.path,
                    &a.icon_path,
                    &a.args,
                    a.category_id,
                    &new_sub,
                )
                .is_ok()
            {
                updated += 1;
            }
        }
    }
    eprintln!("[rescan_subtypes] updated {} apps", updated);
    Ok(updated)
}

// ============================================================
// 🛡️ 11. 软件分类 CRUD
// ============================================================
#[tauri::command]
pub fn list_app_categories(state: State<AppState>) -> Result<Vec<db::AppCategoryMeta>, String> {
    // [data-gate:shared] Phase 2C-3 统一数据操作门（锁顺序: gate → master_wrap_gate → db → conn）
    let _data_gate = state.data_read()?;

    let db = state.db.lock().unwrap();
    db.list_app_categories().map_err(|e| e.to_string())
}
#[tauri::command]
pub fn create_app_category(
    state: State<AppState>,
    name: String,
    icon: Option<String>,
) -> Result<i64, String> {
    // [data-gate:shared] Phase 2C-3 统一数据操作门（锁顺序: gate → master_wrap_gate → db → conn）
    let _data_gate = state.data_read()?;

    let db = state.db.lock().unwrap();
    db.create_app_category(&name, icon.as_deref().unwrap_or("📁"))
        .map_err(|e| e.to_string())
}
#[tauri::command]
pub fn update_app_category(
    state: State<AppState>,
    id: i64,
    name: String,
    icon: String,
) -> Result<(), String> {
    // [data-gate:shared] Phase 2C-3 统一数据操作门（锁顺序: gate → master_wrap_gate → db → conn）
    let _data_gate = state.data_read()?;

    let db = state.db.lock().unwrap();
    db.update_app_category(id, &name, &icon)
        .map_err(|e| e.to_string())
}
#[tauri::command]
pub fn delete_app_category(state: State<AppState>, id: i64) -> Result<(), String> {
    // [data-gate:shared] Phase 2C-3 统一数据操作门（锁顺序: gate → master_wrap_gate → db → conn）
    let _data_gate = state.data_read()?;

    let db = state.db.lock().unwrap();
    db.delete_app_category(id).map_err(|e| e.to_string())
}

// ============================================================
// 🛡️ 12. 前端日志写文件（诊断 webview 问题）
// ============================================================
#[tauri::command]
pub fn log_frontend(level: String, msg: String) {
    let log_path = std::env::var("APPDATA")
        .or_else(|_| std::env::var("LOCALAPPDATA"))
        .map(|p| {
            std::path::PathBuf::from(p)
                .join("com.drawer-box.app")
                .join("frontend.log")
        })
        .unwrap_or_else(|_| std::path::PathBuf::from("frontend.log"));
    let _ = std::fs::create_dir_all(log_path.parent().unwrap());
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let line = format!("[{}] [{}] {}\n", ts, level, msg);
    let _ = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .and_then(|mut f| std::io::Write::write_all(&mut f, line.as_bytes()));
    eprintln!("[frontend] {}", line.trim_end());
}

// ============================================================
// 🛡️ 13. 新增软件（没传图标时同步抽图）
// ============================================================
#[tauri::command]
pub fn create_app(
    state: State<AppState>,
    name: String,
    path: String,
    icon_path: Option<String>,
    args: Option<String>,
    category_id: Option<i64>,
    app_type: Option<String>,
    app_subtype: Option<String>,
) -> Result<i64, String> {
    // [data-gate:shared] Phase 2C-3 统一数据操作门（锁顺序: gate → master_wrap_gate → db → conn）
    let _data_gate = state.data_read()?;

    // P0-#DEBUG#APPTYPE：入口立刻打印所有入参（最关键）→ 同时写磁盘日志
    let db = state.db.lock().unwrap();
    let path_obj = std::path::Path::new(&path);

    // P0-#Y#FIX#URL#APPTYPE#SANITY：app_type 二次校验（双保险）
    let mut at = app_type.unwrap_or_else(|| detect_type_from_path(path_obj).to_string());

    #[cfg(windows)]
    {
        if win_launch::is_url_path(&path) {
            if at != "url" {
                eprintln!("[create_app] app_type 纠正: 路径是 URL 形式，但前端传 app_type={} → 强制为 url (path={})", at, path);
            }
            at = "url".to_string();
        }
    }
    #[cfg(not(windows))]
    {
        let pl = path.to_lowercase();
        if (pl.starts_with("http://")
            || pl.starts_with("https://")
            || pl.contains("://")
            || pl.ends_with(".url"))
            && at != "url"
        {
            eprintln!("[create_app] app_type 纠正: 路径是 URL 形式，但前端传 app_type={} → 强制为 url (path={})", at, path);
            at = "url".to_string();
        }
    }

    if at == "folder" {
        // folder 必须是真实存在的目录；如果不是就按 detect_type_from_path 重推
        if !path_obj.is_dir() {
            let corrected = detect_type_from_path(path_obj);
            eprintln!(
                "[create_app] app_type 纠正: app_type=folder 但 path 不是目录({:?}) → 改为 {}",
                path_obj.exists(),
                corrected
            );
            at = corrected.to_string();
        }
    }

    let sub = match app_subtype {
        Some(s) if !s.is_empty() => s,
        _ => detect_subtype_from_path(path_obj, &at).to_string(),
    };
    let final_icon = if icon_path.as_deref().map(|s| !s.is_empty()).unwrap_or(false) {
        icon_path.clone().unwrap()
    } else if at == "app" {
        #[cfg(windows)]
        {
            win_icon::extract_to_cache(&path)
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default()
        }
        #[cfg(not(windows))]
        {
            String::new()
        }
    } else {
        String::new()
    };
    db.create_app(
        &name,
        &path,
        &final_icon,
        args.as_deref().unwrap_or(""),
        category_id,
        &at,
        &sub,
    )
    .map_err(|e| e.to_string())
}

// ============================================================
// 🛡️ 14. 批量导入路径（拖入或批量选择）
// ============================================================
#[derive(serde::Serialize)]
pub struct ImportResult {
    new_ids: Vec<i64>,
    skipped: Vec<String>,
    errors: Vec<String>,
}

#[cfg(windows)]
fn resolve_lnk_targets_batch_paths(paths: &[String]) -> std::collections::HashMap<String, String> {
    // Batch 2.1：统一走 Batch 1-B 修复后的解析核心。
    // 旧实现是第二处旧协议实例（无索引标记 + OEM 编码 + 索引 zip）——
    // 拖拽/批量导入路径时同样存在错绑风险，现已消除。
    resolve_lnk_paths_raw(paths, None)
        .into_iter()
        .map(|(k, v)| (k, v.target))
        .collect()
}
#[cfg(not(windows))]
fn resolve_lnk_targets_batch_paths(_paths: &[String]) -> std::collections::HashMap<String, String> {
    std::collections::HashMap::new()
}

#[tauri::command]
pub fn import_paths(
    state: State<AppState>,
    paths: Vec<String>,
    category_id: Option<i64>,
) -> Result<ImportResult, String> {
    // [data-gate:shared] Phase 2C-3 统一数据操作门（锁顺序: gate → master_wrap_gate → db → conn）
    let _data_gate = state.data_read()?;

    use std::path::Path;
    let mut new_ids: Vec<i64> = Vec::new();
    let mut skipped: Vec<String> = Vec::new();
    let mut errors: Vec<String> = Vec::new();

    let lnk_targets = resolve_lnk_targets_batch_paths(&paths);

    let db = state.db.lock().unwrap();
    for path_str in &paths {
        let p = Path::new(path_str);
        if !p.exists() {
            errors.push(format!("文件不存在: {}", path_str));
            continue;
        }
        let ext = p
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_lowercase())
            .unwrap_or_default();
        let (db_path, db_name) = if ext == "lnk" {
            if let Some(target) = lnk_targets.get(path_str) {
                if !target.is_empty() && Path::new(target).exists() {
                    let t_ext = Path::new(target)
                        .extension()
                        .and_then(|e| e.to_str())
                        .map(|e| e.to_lowercase())
                        .unwrap_or_default();
                    if matches!(t_ext.as_str(), "exe" | "bat" | "cmd" | "msi") {
                        let target_name = Path::new(target)
                            .file_name()
                            .and_then(|s| s.to_str())
                            .unwrap_or("Unknown")
                            .to_string();
                        (target.clone(), target_name)
                    } else {
                        (
                            path_str.clone(),
                            p.file_name()
                                .and_then(|s| s.to_str())
                                .unwrap_or("Unknown")
                                .to_string(),
                        )
                    }
                } else {
                    (
                        path_str.clone(),
                        p.file_name()
                            .and_then(|s| s.to_str())
                            .unwrap_or("Unknown")
                            .to_string(),
                    )
                }
            } else {
                (
                    path_str.clone(),
                    p.file_name()
                        .and_then(|s| s.to_str())
                        .unwrap_or("Unknown")
                        .to_string(),
                )
            }
        } else {
            (
                path_str.clone(),
                p.file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("Unknown")
                    .to_string(),
            )
        };
        if let Ok(Some(_)) = db.find_app_id_by_path(&db_path) {
            skipped.push(db_path);
            continue;
        }
        if let Some(stem) = Path::new(&db_path).file_stem().and_then(|s| s.to_str()) {
            if let Ok(Some(_)) = db.find_app_id_by_name_stem(stem) {
                skipped.push(db_path);
                continue;
            }
        }
        let name = db_name;
        let app_type = if p.is_dir() {
            "folder"
        } else if ext == "lnk" {
            if let Some(target) = lnk_targets.get(path_str) {
                if !target.is_empty() && Path::new(target).is_dir() {
                    "folder"
                } else {
                    "app"
                }
            } else {
                "app"
            }
        } else {
            detect_type_from_path(p)
        };
        let app_subtype = detect_subtype_from_path(p, app_type).to_string();
        match db.create_app(&name, &db_path, "", "", category_id, app_type, &app_subtype) {
            Ok(id) => new_ids.push(id),
            Err(e) => errors.push(format!("{}: {}", db_path, e)),
        }
    }
    Ok(ImportResult {
        new_ids,
        skipped,
        errors,
    })
}

// ============================================================
// 🛡️ 14b. 批量导入扫描候选（Batch 2.1：幂等 + 身份去重 + 回收站提示）
// ============================================================
#[tauri::command]
pub fn import_scanned_apps(
    state: State<AppState>,
    candidates: Vec<ScannedApp>,
    category_id: Option<i64>,
) -> Result<ImportSummary, String> {
    // [data-gate:shared] Phase 2C-3 统一数据操作门（锁顺序: gate → master_wrap_gate → db → conn）
    let _data_gate = state.data_read()?;
    let db = state.db.lock().unwrap();
    Ok(import_candidates_core(&db, &candidates, category_id))
}

// ============================================================
// 🛡️ 15. 修改软件（含 app_type）
// ============================================================
#[tauri::command]
pub fn update_app(
    state: State<AppState>,
    id: i64,
    name: String,
    path: String,
    icon_path: Option<String>,
    args: Option<String>,
    category_id: Option<i64>,
    app_subtype: Option<String>,
    app_type: Option<String>,
) -> Result<(), String> {
    // [data-gate:shared] Phase 2C-3 统一数据操作门（锁顺序: gate → master_wrap_gate → db → conn）
    let _data_gate = state.data_read()?;

    let db = state.db.lock().unwrap();
    // 🛡️ patch 语义（P0-#Y#FIX#PATCH）：Option 字段统一 None = 保持原值、Some(v) = 显式更新。
    // 旧实现把"未提供"（unwrap_or("")/String::new()）当成"主动清空"，导致"编辑名称/路径"表单
    // 顺带清掉 icon_path/app_subtype；右键"修改一级分类"(changeType) 同样会清掉 app_subtype。
    // 注意：None ≠ 清空。UI 目前没有主动清空 icon/category/subtype 的入口；
    // 未来若需支持，必须新增显式信号（专用命令或哨兵值），不得复用 None。
    db.update_app_full(
        id,
        &name,
        &path,
        icon_path.as_deref(),
        args.as_deref(),
        category_id,
        app_subtype.as_deref(),
        app_type.as_deref(),
    )
    .map_err(|e| e.to_string())
}

// ============================================================
// 🛡️ 16. 删除软件（软删除 → 回收站）
// ============================================================
#[tauri::command]
pub fn delete_app(state: State<AppState>, id: i64) -> Result<(), String> {
    // [data-gate:shared] Phase 2C-3 统一数据操作门（锁顺序: gate → master_wrap_gate → db → conn）
    let _data_gate = state.data_read()?;

    let db = state.db.lock().unwrap();
    db.soft_delete("apps", id).map_err(|e| e.to_string())?;
    Ok(())
}

// ============================================================
// 🛡️ 17. 记录使用次数
// ============================================================
#[tauri::command]
pub fn record_app_usage(state: State<AppState>, id: i64) -> Result<(), String> {
    // [data-gate:shared] Phase 2C-3 统一数据操作门（锁顺序: gate → master_wrap_gate → db → conn）
    let _data_gate = state.data_read()?;

    let db = state.db.lock().unwrap();
    db.record_app_usage(id).map_err(|e| e.to_string())
}

// ============================================================
// 🛡️ 18. 启动软件（ShellExecuteW + 失败时自动重定位）
// ============================================================
fn try_open_in_steam(_game_name: &str) -> Result<(), String> {
    if let Some(steam_path) = try_relocate_app("Steam", "C:\\") {
        eprintln!("[launch_app] 找到 Steam: {}", steam_path);
        let _ = std::process::Command::new(&steam_path).spawn();
        return Ok(());
    }
    Err("Steam 未安装".to_string())
}

#[cfg(windows)]
async fn launch_with_path(
    path: String,
    args: String,
    app_type: String,
    _app_name: String,
) -> Result<(), String> {
    // P0-#Y#ULTIMATE：终极防线！永远不允许 URL 形式的 path 被当成 folder 打开！
    //   规则优先级：
    //   1) path 含 "://" / looks_like_domain = 是 URL → 100% 强制走 URL 分支（直接返回，不看 app_type！）
    //   2) 只有 app_type=="folder" && path 真实是目录 → 才调 explorer 打开文件夹
    //   3) 其他 → win_launch::launch（ShellExecuteW 自动按扩展名/协议打开）
    let ext = std::path::Path::new(&path)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .unwrap_or_default();
    let definitely_url = win_launch::is_url_path(&path);
    // 🔴 终极防线：path 本身包含协议分隔符（即使 is_url_path 漏判）→ 直接当 URL
    let contains_protocol_sep = path.contains("://");
    let is_dot_url_file = ext == "url";

    if is_dot_url_file {
        if let Some(url) = win_icon::parse_url_shortcut(&path) {
            eprintln!("[launch_with_path] .url 内部 URL: {}", url);
        }
    }

    // 🛡️ 第一道 + 终极防线：只要是 URL 形式或含 :// → 永远强制 URL 打开，不看 app_type！
    if definitely_url || contains_protocol_sep {
        return match win_launch::launch(&path, &args) {
            Ok(()) => Ok(()),
            Err(e) => {
                if is_dot_url_file {
                    if let Some(url) = win_icon::parse_url_shortcut(&path) {
                        let _ = std::process::Command::new("explorer.exe").arg(&url).spawn();
                        return Ok(());
                    }
                }
                Err(e)
            }
        };
    }

    // 🛡️ 第二道防线：只有 app_type=folder 且 path 真实存在且是目录 → 才 explorer 打开文件夹
    if app_type == "folder" {
        let p = std::path::Path::new(&path);
        if p.is_dir() {
            match std::process::Command::new("explorer.exe")
                .arg(&path)
                .spawn()
            {
                Ok(_) => {
                    return Ok(());
                }
                Err(e) => {
                    return Err(format!("打开文件夹失败: {} ({})", path, e));
                }
            }
        } else {
            // 再检查一次是不是 URL 形式（裸域名）
            if win_launch::looks_like_domain(&path) {
                return win_launch::launch(&path, &args);
            }
            // 兜底：ShellExecuteW 打开 whatever 这个 path
            return match win_launch::launch(&path, &args) {
                Ok(()) => Ok(()),
                Err(e) => Err(format!(
                    "app_type=folder 但 path 不是目录，兜底打开也失败: path={} → {}",
                    path, e
                )),
            };
        }
    }
    // 普通 app/document：直接 win_launch::launch（ShellExecuteW 会根据扩展名关联打开）
    match win_launch::launch(&path, &args) {
        Ok(()) => Ok(()),
        Err(e) => Err(e),
    }
}
#[cfg(not(windows))]
async fn launch_with_path(
    path: String,
    _args: String,
    _app_type: String,
    _app_name: String,
) -> Result<(), String> {
    // 非 Windows 走系统默认打开
    let _ = std::process::Command::new("xdg-open").arg(&path).spawn();
    Ok(())
}

/// P0-A：哪些入口允许"按名重定位"。
/// 只有直接文件入口（exe 等）可以；快捷方式 / 网址入口一律不重定位。
pub(crate) fn is_relocatable_entry(ext: &str) -> bool {
    ext != "lnk" && ext != "url"
}

fn try_relocate_app(app_name: &str, _old_path: &str) -> Option<String> {
    use std::path::PathBuf;
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(appdata) = std::env::var("APPDATA") {
        candidates.push(PathBuf::from(appdata).join("Microsoft\\Windows\\Start Menu"));
    }
    if let Ok(progdata) = std::env::var("PROGRAMDATA") {
        candidates.push(PathBuf::from(progdata).join("Microsoft\\Windows\\Start Menu"));
    }
    if let Ok(public_dir) = std::env::var("PUBLIC") {
        candidates.push(PathBuf::from(public_dir).join("Desktop"));
    }
    if let Ok(userprofile) = std::env::var("USERPROFILE") {
        candidates.push(PathBuf::from(userprofile).join("Desktop"));
    }
    candidates.push(PathBuf::from("C:\\Program Files"));
    candidates.push(PathBuf::from("C:\\Program Files (x86)"));
    if let Ok(pf) = std::env::var("ProgramFiles") {
        candidates.push(PathBuf::from(pf));
    }
    if let Ok(pf86) = std::env::var("ProgramFiles(x86)") {
        candidates.push(PathBuf::from(pf86));
    }
    if let Ok(localappdata) = std::env::var("LOCALAPPDATA") {
        candidates.push(PathBuf::from(localappdata).join("Programs"));
    }
    for drive in ["D", "E", "F", "G", "H"] {
        candidates.push(PathBuf::from(format!("{}:\\Program Files", drive)));
        candidates.push(PathBuf::from(format!("{}:\\Program Files (x86)", drive)));
        candidates.push(PathBuf::from(format!("{}:\\Steam", drive)));
        candidates.push(PathBuf::from(format!("{}:\\SteamLibrary", drive)));
        candidates.push(PathBuf::from(format!("{}:\\Epic Games", drive)));
        candidates.push(PathBuf::from(format!("{}:\\Games", drive)));
    }
    let name_lower = app_name.to_lowercase();
    for dir in &candidates {
        if !dir.exists() {
            continue;
        }
        if let Some(found) = scan_dir_for_name(dir, &name_lower, 5, 30) {
            eprintln!(
                "[try_relocate_app] 在 {} 找到 {}: {}",
                dir.display(),
                app_name,
                found
            );
            return Some(found);
        }
    }
    eprintln!("[try_relocate_app] 跨盘扫描未找到: {}", app_name);
    None
}

fn scan_dir_for_name(
    dir: &std::path::Path,
    name_lower: &str,
    max_depth: usize,
    max_results: usize,
) -> Option<String> {
    if max_depth == 0 {
        return None;
    }
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return None,
    };
    let mut found: Vec<String> = Vec::new();
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_dir() {
            if let Some(sub) = scan_dir_for_name(&p, name_lower, max_depth - 1, max_results) {
                found.push(sub);
                if found.len() >= max_results {
                    break;
                }
            }
        } else if let Some(fname) = p.file_name().and_then(|s| s.to_str()) {
            let fname_lower = fname.to_lowercase();
            if let Some(stem) = std::path::Path::new(&fname_lower)
                .file_stem()
                .and_then(|s| s.to_str())
            {
                if stem == name_lower || fname_lower.starts_with(name_lower) {
                    let ext = std::path::Path::new(&fname_lower)
                        .extension()
                        .and_then(|s| s.to_str())
                        .unwrap_or("");
                    if matches!(ext, "exe" | "lnk" | "bat" | "cmd") {
                        if let Some(s) = p.to_str() {
                            found.push(s.to_string());
                            if found.len() >= max_results {
                                break;
                            }
                        }
                    }
                }
            }
        }
    }
    found.sort_by_key(|p| {
        let ext = std::path::Path::new(p)
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_lowercase())
            .unwrap_or_default();
        match ext.as_str() {
            "exe" => 0,
            "bat" | "cmd" => 1,
            "lnk" => 2,
            _ => 3,
        }
    });
    found.into_iter().next()
}

#[tauri::command]
pub async fn launch_app(state: State<'_, AppState>, id: i64) -> Result<String, String> {
    launch_app_inner(&state, id).await
}

/// C1（v0.4.3）：核心逻辑独立于 tauri::State，供单元测试直接构造 AppState 验证
/// "失败不计数 / 受理成功才计数"契约（见 launch_count_tests.rs）。
pub(crate) async fn launch_app_inner(state: &AppState, id: i64) -> Result<String, String> {
    // [data-gate:shared] Phase 2C-3（同步段持锁，不跨 await）
    let _data_gate = state.data_read()?;
    let (path, args, app_type, app_name): (String, String, String, String) = {
        let db = state.db.lock().unwrap();
        let conn = db.conn.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT path, args, app_type, name FROM apps WHERE id=?1")
            .map_err(|e| e.to_string())?;
        stmt.query_row(rusqlite::params![id], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })
        .map_err(|e| e.to_string())?
    };
    // gate 不跨 await：同步段结束即释放
    drop(_data_gate);
    // C1（v0.4.3）：使用次数不再在此处预记——旧实现无论启动成败都 +1，
    // 文件缺失/重定位后仍失败/ShellExecute 报错全部被计为"已使用"。
    // 现改为：仅在启动请求被系统成功受理（各 Ok 出口）后计数，
    // 见下方 count_launch_success 调用点。
    // 注意口径：计数含义 = "启动请求受理成功"，不代表目标进程已成功运行
    // （ShellExecuteW 返回 ≠ 应用跑起来），该语义与常用排序用途匹配。

    // C1（v0.4.3）计数助手：仅在启动请求被系统成功受理后调用。
    // 数据门不重取——与函数内 update_app_path 的后置写库先例一致（单条 UPDATE，
    // 与迁移互斥由迁移侧独占闸保证）。
    fn count_launch_success(state: &AppState, id: i64) {
        let _ = state.db.lock().unwrap().record_app_usage(id);
    }
    // Batch 2.4 修正：打包应用（UWP/MSIX）——explorer 托管 shell:AppsFolder 激活
    //（Spike 门2 实证：AUMID 激活成功且有进程实证；伪造 AUMID 可被同法捕获失败）。
    // 启动口径三分（验收指令）：①请求已派发（spawn 成功）→ ②系统接受请求
    // → ③实际进程激活。explorer 托管即时返回且退出码不携带激活结果，
    // 本分支只能证明到 ①——计数口径 = 已派发，返回串如实标注 dispatched。
    if app_type == "uwp" {
        return match std::process::Command::new("explorer.exe")
            .arg(format!("shell:AppsFolder\\{}", path))
            .spawn()
        {
            Ok(_) => {
                count_launch_success(state, id);
                Ok("ok:uwp-dispatched".to_string())
            }
            Err(e) => Err(format!("打包应用启动失败: {} ({})", path, e)),
        };
    }

    #[cfg(not(windows))]
    let (path_exists, is_url_path_flag) = (std::path::Path::new(&path).exists(), false);
    #[cfg(windows)]
    // P0-#Y#FIX#ISURL#PATH：改用 is_url_path（协议前缀 + 裸域名 + .url 文件）
    //   之前只用 is_url → mp.weixin.qq.com / v2fun.fun 这种裸域名判不到 → 直接 Err 找不到文件
    let (path_exists, is_url_path_flag) = (
        std::path::Path::new(&path).exists(),
        win_launch::is_url_path(&path),
    );

    if !path_exists && !is_url_path_flag {
        eprintln!(
            "[launch_app] 路径不存在: {} (id={}, name={})",
            path, id, app_name
        );
        if let Some(new_path) = try_relocate_app(&app_name, &path) {
            eprintln!("[launch_app] 自动重定位: {} → {}", path, new_path);
            {
                let db = state.db.lock().unwrap();
                let _ = db.update_app_path(id, &new_path);
            }
            return match launch_with_path(new_path.clone(), args, app_type, app_name).await {
                Ok(()) => {
                    count_launch_success(state, id);
                    Ok(format!("ok:auto-relocated:{} → {}", path, new_path))
                }
                Err(e) => Err(format!(
                    "relocated-but-still-fail:{} → {}: {}",
                    path, new_path, e
                )),
            };
        }
        if app_type == "url" || path.to_lowercase().ends_with(".url") {
            if try_open_in_steam(&app_name).is_ok() {
                count_launch_success(state, id);
                return Ok("ok:steam-fallback".to_string());
            }
        }
        return Err(format!("找不到文件: {}", path));
    }

    // P0-#Y#FIX#URL#FORCE：如果路径本身是 URL 形式（但文件系统不存在，比如裸域名 www.baidu.com）
    //   强制走 URL 启动，不依赖 app_type 字段（历史数据有 bug 可能导致 app_type=folder/app）
    //   如果传进来的 app_type 不是 url，这里不报错，直接强制用 url 类型启动
    if !path_exists && is_url_path_flag {
        eprintln!(
            "[launch_app] 文件不存在但判定为 URL 形式，强制启动（app_type={}，path={}）",
            app_type, path
        );
        return match launch_with_path(
            path.clone(),
            args.clone(),
            "url".to_string(),
            app_name.clone(),
        )
        .await
        {
            Ok(()) => {
                count_launch_success(state, id);
                Ok(format!("ok:forced-url-launch:{}", path))
            }
            Err(e) => Err(format!("url-launch-fail:{} → {}", path, e)),
        };
    }

    match launch_with_path(
        path.clone(),
        args.clone(),
        app_type.clone(),
        app_name.clone(),
    )
    .await
    {
        Ok(()) => {
            count_launch_success(state, id);
            Ok("ok".to_string())
        }
        Err(e) => {
            eprintln!(
                "[launch_app] 启动失败 (path 存在但 ShellExecuteW 报错): {} → {}",
                path, e
            );
            if let Some(new_path) = try_relocate_app(&app_name, &path) {
                if new_path != path && std::path::Path::new(&new_path).exists() {
                    eprintln!("[launch_app] 运行搬迁恢复: {} → {}", path, new_path);
                    {
                        let db = state.db.lock().unwrap();
                        let _ = db.update_app_path(id, &new_path);
                    }
                    return match launch_with_path(new_path.clone(), args, app_type, app_name).await
                    {
                        Ok(()) => {
                            count_launch_success(state, id);
                            Ok(format!("ok:auto-relocated:{} → {}", path, new_path))
                        }
                        Err(e2) => Err(format!(
                            "relocated-but-still-fail:{} → {}: {}",
                            path, new_path, e2
                        )),
                    };
                }
            }
            Err(e)
        }
    }
}

// ============================================================
// 🛡️ 19. 启用/关闭贴边自动隐藏（诊断：暂时禁用）
// ============================================================
#[tauri::command]
pub fn set_auto_hide_enabled(_enabled: bool) {}

// ============================================================
// 🛡️ 20. 健康检查（跨盘重定位路径，spawn_blocking 不阻塞主线程）
// ============================================================
fn health_check_apps_sync(state: &tauri::State<'_, AppState>) -> Result<usize, String> {
    use std::path::Path;
    let apps: Vec<(i64, String, String, String)> = {
        let db = state.db.lock().unwrap();
        let conn = db.conn.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT id, name, path, app_type FROM apps WHERE deleted_at IS NULL")
            .map_err(|e| e.to_string())?;
        let v: Vec<(i64, String, String, String)> = stmt
            .query_map([], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })
            .map_err(|e| e.to_string())?
            .filter_map(|r| r.ok())
            .collect();
        drop(stmt);
        drop(conn);
        drop(db);
        v
    };
    let mut fixed = 0usize;
    for (id, name, path, app_type) in apps {
        let path_p = Path::new(&path);
        let ext = path_p
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_lowercase())
            .unwrap_or_default();
        let path_exists = path_p.exists();
        if app_type == "folder" {
            continue;
        }
        #[cfg(windows)]
        {
            // P0-A（v0.4.3 验收）：**不再**把 .lnk 静默改写为解析目标。
            // 原行为会把入口身份从 .lnk 换成 target exe —— Steam 游戏（→ steam.exe）、
            // Electron 转发（→ Update.exe --processStart）、Chrome PWA（--app= 丢失）、
            // 安装器转发入口都会被改写为错误程序，且不可逆。
            // 图标提取另有 pick_extract_path 处理 .lnk，无需改写路径。
            // 检测失败/无法解析的条目不写库（严格区分失效与检测失败）。
        }
        if ext == "url" && path_exists {
            continue;
        }
        if !path_exists {
            eprintln!(
                "[health_check] app {} (id={}) path 失效: {}",
                name, id, path
            );
            // P0-A：只对「直接文件入口」做按名重定位；.lnk / .url 不参与——
            // 快捷方式入口的身份是"入口文件本身 + 目标"，按名称另找一个程序会
            // 把 Steam / Electron / PWA 入口错绑到别的可执行文件，且不可逆。
            if is_relocatable_entry(&ext) {
                if let Some(new_path) = try_relocate_app(&name, &path) {
                    if new_path != path {
                        eprintln!("[health_check]   → 重定位: {}", new_path);
                        let db2 = state.db.lock().unwrap();
                        let _ = db2.update_app_path(id, &new_path);
                        fixed += 1;
                        continue;
                    }
                }
            }
            // P0-A：.lnk 的按名重定位同样移除——按文件名匹配可能错绑到同名异程序；
            // 失效 .lnk 交由用户在失效列表中显式「重新关联」（既有入口）。
            eprintln!("[health_check]   ✗ 重定位失败: {}", name);
        }
    }
    Ok(fixed)
}

#[tauri::command]
pub async fn health_check_apps(app: AppHandle) -> Result<usize, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state: tauri::State<AppState> = app.state();
        // [data-gate:shared] Phase 2C-3（spawn_blocking 内同步持有）
        let _data_gate = state.data_read()?;
        health_check_apps_sync(&state)
    })
    .await
    .map_err(|e| format!("健康检查线程失败: {}", e))?
}

// ============================================================
// 🛡️ 20b. 失效记录集中治理（v0.4.1 软件失效治理）
// ============================================================

use serde::Serialize;

/// 单条失效记录：kind 由 db::classify_path_health 给出，
/// 只有 kind == "fixed_missing" 允许进入批量清退候选；其余类别仅展示（保护）。
#[derive(Debug, Clone, Serialize)]
pub struct InvalidAppInfo {
    pub id: i64,
    pub name: String,
    pub path: String,
    pub app_type: String,
    pub kind: &'static str,
    /// .lnk 已解析出的目标路径（自检/目标双检查的展示依据；可能为空）
    pub lnk_target: Option<String>,
}

/// 盘符健康探测：GetDriveTypeW + UNC 判定（Windows 专用；非测试路径）
#[cfg(windows)]
fn probe_drive(path: &str) -> (u32, bool) {
    use windows_sys::Win32::Storage::FileSystem::GetDriveTypeW;
    let is_net = crate::db::is_unc_path(path);
    // 取盘符根（"C:\"）；UNC 取 \server\share
    let root: String = if is_net {
        let rest = path.trim_start_matches(|c| c == '\\' || c == '/');
        format!(
            "\\\\{}",
            rest.split(|c| c == '\\' || c == '/')
                .take(2)
                .collect::<Vec<_>>()
                .join("\\")
        )
    } else {
        let mut chars = path.chars();
        match (chars.next(), chars.next()) {
            (Some(d), Some(':')) => format!("{}:\\", d),
            _ => return (0, is_net),
        }
    };
    let wide: Vec<u16> = root.encode_utf16().chain(std::iter::once(0)).collect();
    let dt = unsafe { GetDriveTypeW(wide.as_ptr()) };
    (dt, is_net)
}

#[cfg(not(windows))]
fn probe_drive(_path: &str) -> (u32, bool) {
    (0, false)
}

/// 解析出的目标路径是否可信（可作为判定依据）：
/// - 不能含 U+FFFD 替换符（编码失败的典型痕迹）；
/// - 必须是带盘符的绝对路径（或 UNC）——协议名 / CLSID / 相对路径都不算。
pub(crate) fn is_plausible_target(target: &str) -> bool {
    if target.is_empty() || target.contains('\u{fffd}') {
        return false;
    }
    let bytes = target.as_bytes();
    if bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic() {
        return true;
    }
    target.starts_with("\\\\")
}

fn target_opt(target: &str) -> Option<String> {
    if target.is_empty() {
        None
    } else {
        Some(target.to_string())
    }
}

/// 单条记录分类（纯编排，分类语义在 db::classify_path_health；OS 探测在此层）
pub(crate) fn classify_one(path: &str, app_type: &str) -> (&'static str, Option<String>) {
    if app_type == "url" {
        // URL（含 steam:// epic:// 等协议）恒可用语义与 Phase 2D 一致
        return ("ok", None);
    }
    let exists = std::path::Path::new(path).exists();
    if app_type == "folder" {
        let (dt, net) = probe_drive(path);
        return (crate::db::classify_path_health(dt, exists, net), None);
    }
    let ext = std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .unwrap_or_default();
    if ext == "lnk" && exists {
        // .lnk 双检查：快捷方式自身存在 + 目标程序存在
        let target = crate::win_icon::resolve_lnk_target(path).unwrap_or_default();
        if target.is_empty() || !is_plausible_target(&target) {
            // 无法解析 / 路径含乱码替换符 / 非绝对盘符路径 →
            // 只能证明"检测失败"，不能证明"目标已被删除" → 保护，绝不建议清退
            return ("uncertain", target_opt(&target));
        }
        if !std::path::Path::new(&target).exists() {
            // P0-A（v0.4.3 验收）：.lnk 目标缺失一律归「需要确认」。
            // 可能是卸载（真实失效），也可能是解析/权限/入口类型（UWP、协议、
            // 安装器转发）等检测失败——两者在此层无法可靠区分，按验收要求
            // "未确定失效原因时统一归为需要确认，不得推荐清退"。
            return ("uncertain", Some(target));
        }
        return ("ok", Some(target));
    }
    if ext == "url" && exists {
        return ("ok", None);
    }
    let (dt, net) = probe_drive(path);
    (crate::db::classify_path_health(dt, exists, net), None)
}

/// 列出全部失效记录（含保护分类）。url 恒可用不进入列表。
#[tauri::command]
pub async fn list_invalid_apps(app: AppHandle) -> Result<Vec<InvalidAppInfo>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state: tauri::State<AppState> = app.state();
        let _data_gate = state.data_read()?;
        let apps: Vec<(i64, String, String, String)> = {
            let db = state.db.lock().unwrap();
            let conn = db.conn.lock().unwrap();
            let mut stmt = conn
                .prepare("SELECT id, name, path, app_type FROM apps WHERE deleted_at IS NULL")
                .map_err(|e| e.to_string())?;
            let v = stmt
                .query_map([], |row| {
                    Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
                })
                .map_err(|e| e.to_string())?
                .filter_map(|r| r.ok())
                .collect();
            v
        };
        let mut out = Vec::new();
        for (id, name, path, app_type) in apps {
            let (kind, lnk_target) = classify_one(&path, &app_type);
            if kind != "ok" {
                out.push(InvalidAppInfo {
                    id,
                    name,
                    path,
                    app_type,
                    kind,
                    lnk_target,
                });
            }
        }
        Ok(out)
    })
    .await
    .map_err(|e| format!("失效检查线程失败: {}", e))?
}

/// 批量移入回收站（软删除）：只删除抽屉柜收录记录，不卸载任何 Windows 程序。
/// 幂等：已在回收站的 id 记为 skipped；返回成功/跳过/逐条错误（部分失败可见）。
#[derive(Debug, Serialize)]
pub struct BatchTrashResult {
    pub moved: usize,
    pub skipped: usize,
    pub errors: Vec<(i64, String)>,
}

#[tauri::command]
pub fn batch_move_apps_to_trash(
    state: State<AppState>,
    ids: Vec<i64>,
) -> Result<BatchTrashResult, String> {
    let _data_gate = state.data_read()?;
    let mut out = BatchTrashResult {
        moved: 0,
        skipped: 0,
        errors: Vec::new(),
    };
    let db = state.db.lock().unwrap();
    for id in ids {
        match db.soft_delete("apps", id) {
            Ok(n) if n > 0 => out.moved += 1,
            Ok(_) => out.skipped += 1, // 不存在或已在回收站（幂等）
            Err(e) => out.errors.push((id, e.to_string())),
        }
    }
    Ok(out)
}

// ============================================================
// 🛡️ 20c. 外链打开（v0.4.1 任务 B）：系统默认浏览器，错误可见
// ============================================================
#[tauri::command]
pub fn open_external_url(url: String) -> Result<(), String> {
    // 防御：只放行 http(s)，其余一律拒绝（ftp/file/自定义协议不走本命令）
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err(format!("已阻止打开非 http(s) 链接：{}", url));
    }
    ::open::that_detached(&url).map_err(|e| format!("打开系统浏览器失败：{}", e))
}

// ============================================================
// 🛡️ 20d. 验收测试数据播种 / 重置（v0.4.1 任务 A）
// 仅便携测试版可用（正式版调用直接报错）；数据全部落在便携测试数据根内。
// ============================================================
const FIXTURE_NAME_PREFIX: &str = "[TEST]";

#[derive(Debug, Serialize)]
pub struct SeedFixturesResult {
    pub cleared_previous: usize,
    pub inserted: usize,
    pub fixture_dir: String,
}

fn seed_one(db: &crate::db::Db, name: &str, path: &str, app_type: &str) -> Result<i64, String> {
    db.create_app(
        &format!("{} {}", FIXTURE_NAME_PREFIX, name),
        path,
        "",
        "",
        None,
        app_type,
        "other",
    )
    .map_err(|e| e.to_string())
}

fn stale_fixture_ids(db: &crate::db::Db) -> Vec<i64> {
    let conn = db.conn.lock().unwrap();
    let mut stmt = conn
        .prepare("SELECT id FROM apps WHERE deleted_at IS NULL AND name LIKE ?1")
        .expect("查询播种记录失败");
    let pat = format!("{}%", FIXTURE_NAME_PREFIX);
    let v = stmt
        .query_map([pat], |row| row.get(0))
        .expect("读取播种记录失败")
        .filter_map(|r| r.ok())
        .collect();
    v
}

#[cfg(feature = "test-edition")]
#[tauri::command]
pub async fn seed_test_fixtures(app: AppHandle) -> Result<SeedFixturesResult, String> {
    if !crate::data_root::is_portable_test_edition() {
        return Err("测试数据播种仅支持便携测试版（正式版禁止注入模拟数据）".to_string());
    }
    tauri::async_runtime::spawn_blocking(move || {
        let state: tauri::State<AppState> = app.state();
        let _data_gate = state.data_read()?;
        let data_root = crate::data_root::effective_config_root(&app)?;
        let fixture_dir = data_root.join("fixtures");
        std::fs::create_dir_all(&fixture_dir)
            .map_err(|e| format!("创建 fixtures 目录失败: {}", e))?;
        // 正常记录用例：真实存在的 fixture 文件（其余为不存在路径，绝不触碰用户真实软件）
        let normal_exe = fixture_dir.join("normal-app.exe");
        std::fs::write(&normal_exe, b"MZ").map_err(|e| format!("写 normal fixture 失败: {}", e))?;
        // .lnk 目标异常用例：写入非法内容（非真实快捷方式）→ 目标解析失败 → uncertain（保护）
        let broken_lnk = fixture_dir.join("broken.lnk");
        std::fs::write(&broken_lnk, b"NOT-A-REAL-SHORTCUT")
            .map_err(|e| format!("写 lnk fixture 失败: {}", e))?;

        let db = state.db.lock().unwrap();
        // 1) 清理上一轮播种（软删除，回收站可恢复；幂等）
        let stale = stale_fixture_ids(&db);
        let cleared = stale.len();
        for id in stale {
            let _ = db.soft_delete("apps", id);
        }
        // 2) 播种六类覆盖：正常 / 固定盘缺失（唯一清退候选）/ 网络位置（保护）/
        //    网址 / Steam 协议 / 快捷方式目标异常（保护）
        let normal = seed_one(
            &db,
            "正常软件（固定盘存在）",
            normal_exe.to_str().unwrap_or(""),
            "app",
        )?;
        let missing = seed_one(
            &db,
            "固定盘路径缺失（可清退）",
            r"C:\drawer-test-fixture\missing-fixed.exe",
            "app",
        )?;
        let network = seed_one(
            &db,
            "网络位置失效（保护）",
            r"\drawer-test-server\share\app.exe",
            "app",
        )?;
        let url = seed_one(&db, "网址（恒可用）", "https://example.com", "url")?;
        let steam = seed_one(&db, "Steam 协议（恒可用）", "steam://rungame/7", "url")?;
        let lnk = seed_one(
            &db,
            "快捷方式目标异常（保护）",
            broken_lnk.to_str().unwrap_or(""),
            "app",
        )?;
        let _ = (normal, missing, network, url, steam, lnk);
        Ok(SeedFixturesResult {
            cleared_previous: cleared,
            inserted: 6,
            fixture_dir: fixture_dir.to_string_lossy().to_string(),
        })
    })
    .await
    .map_err(|e| format!("播种线程失败: {}", e))?
}

#[cfg(not(feature = "test-edition"))]
#[tauri::command]
pub async fn seed_test_fixtures(_app: AppHandle) -> Result<SeedFixturesResult, String> {
    Err("正式构建不包含测试数据播种能力（该能力仅存在于验收便携测试版）".to_string())
}

#[cfg(feature = "test-edition")]
#[tauri::command]
pub async fn clear_test_fixtures(app: AppHandle) -> Result<usize, String> {
    if !crate::data_root::is_portable_test_edition() {
        return Err("测试数据重置仅支持便携测试版".to_string());
    }
    tauri::async_runtime::spawn_blocking(move || {
        let state: tauri::State<AppState> = app.state();
        let _data_gate = state.data_read()?;
        let db = state.db.lock().unwrap();
        let stale = stale_fixture_ids(&db);
        let n = stale.len();
        for id in stale {
            let _ = db.soft_delete("apps", id);
        }
        Ok(n)
    })
    .await
    .map_err(|e| format!("重置线程失败: {}", e))?
}

#[cfg(not(feature = "test-edition"))]
#[tauri::command]
pub async fn clear_test_fixtures(_app: AppHandle) -> Result<usize, String> {
    Err("正式构建不包含测试数据重置能力（该能力仅存在于验收便携测试版）".to_string())
}

// ============================================================
// 🛡️ 21. 当前数据存储目录
// ============================================================
#[tauri::command]
pub fn get_data_dir(state: State<AppState>) -> Result<String, String> {
    // [data-gate:shared] Phase 2C-3 统一数据操作门（锁顺序: gate → master_wrap_gate → db → conn）
    let _data_gate = state.data_read()?;

    eprintln!("[IPC] get_data_dir ENTER");
    let db = state.db.lock().unwrap();
    let conn = db.conn.lock().unwrap();
    let mut stmt = conn
        .prepare("PRAGMA database_list")
        .map_err(|e| e.to_string())?;
    let mut rows = stmt.query([]).map_err(|e| e.to_string())?;
    if let Some(row) = rows.next().map_err(|e| e.to_string())? {
        let file: String = row.get(2).map_err(|e| e.to_string())?;
        if let Some(parent) = std::path::Path::new(&file).parent() {
            return Ok(parent.to_string_lossy().to_string());
        }
    }
    Ok(String::from("(未知)"))
}

/// 在资源管理器中打开当前数据目录（U1：数据页「打开」按钮）。
/// 只打开应用自己的数据目录，不接受前端传入任意路径。
#[tauri::command]
pub fn open_data_dir(state: State<AppState>) -> Result<(), String> {
    let _data_gate = state.data_read()?;
    let dir = {
        let db = state.db.lock().unwrap();
        let conn = db.conn.lock().unwrap();
        let mut stmt = conn
            .prepare("PRAGMA database_list")
            .map_err(|e| e.to_string())?;
        let mut rows = stmt.query([]).map_err(|e| e.to_string())?;
        let mut found = None;
        if let Some(row) = rows.next().map_err(|e| e.to_string())? {
            let file: String = row.get(2).map_err(|e| e.to_string())?;
            found = std::path::Path::new(&file)
                .parent()
                .map(|p| p.to_string_lossy().to_string());
        }
        found.ok_or_else(|| "数据目录不可用".to_string())?
    };
    std::process::Command::new("explorer.exe")
        .arg(&dir)
        .spawn()
        .map_err(|e| format!("打开数据目录失败：{e}"))?;
    Ok(())
}

// ============================================================
// 🛡️ 23. 启动时一次性全表修正 apps 脏数据（app_type 与 path 不一致问题）
// ============================================================
/// **启动时自动修所有历史脏数据：**
/// - 规则 1：path 是 URL 形式（is_url_path 或含 ://）但 app_type != "url" → 强制 UPDATE app_type=url
/// - 规则 2：app_type="folder" 但 path 不是真实目录 → 用 detect_type_from_path 重新推断并 UPDATE
/// - 规则 3：app_type="document" 但扩展名不在文档列表 → 重新推断
/// **每次启动必执行一次，保证 DB 与实际语义永远一致**
pub fn sanitize_db_on_startup(state: &tauri::State<'_, AppState>) -> Result<usize, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    let conn = db.conn.lock().map_err(|e| e.to_string())?;

    // 先读出所有未删除条目
    let mut stmt = conn
        .prepare("SELECT id, name, path, app_type FROM apps WHERE deleted_at IS NULL")
        .map_err(|e| e.to_string())?;
    let rows: Vec<(i64, String, String, String)> = stmt
        .query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })
        .map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect();
    drop(stmt);

    let mut fixed_count = 0usize;
    for (id, _name, path, old_app_type) in rows {
        let path_obj = std::path::Path::new(&path);
        #[cfg(windows)]
        let path_is_url = win_launch::is_url_path(&path) || path.contains("://");
        #[cfg(not(windows))]
        let path_is_url = {
            let pl = path.to_lowercase();
            pl.starts_with("http://")
                || pl.starts_with("https://")
                || pl.contains("://")
                || pl.ends_with(".url")
        };
        let mut new_app_type: Option<String> = None;

        // 规则 1：URL 形式但 app_type != url → 强制 url
        if path_is_url && old_app_type != "url" {
            new_app_type = Some("url".to_string());
        }
        // 规则 2：app_type=folder 但 path 不是真实目录 → 重新推断
        else if old_app_type == "folder" && !path_obj.is_dir() {
            let corrected = detect_type_from_path(path_obj);
            new_app_type = Some(corrected.to_string());
        }

        if let Some(new_at) = new_app_type {
            match conn.execute(
                "UPDATE apps SET app_type=?1 WHERE id=?2",
                rusqlite::params![new_at, id],
            ) {
                Ok(_) => {
                    fixed_count += 1;
                }
                Err(_) => { /* 写失败按未修正处理，不中断启动 */ }
            }
        }
    }

    Ok(fixed_count)
}

// ============================================================
// 🛡️ 22. 贴边隐藏相关（暂时留空，诊断用）
// ============================================================
#[tauri::command]
pub fn is_dock_hidden() -> bool {
    false
}
#[tauri::command]
pub fn force_dock_reveal(_app: AppHandle) -> Result<(), String> {
    Ok(())
}

// 让 crypto / BASE64 import 不 warning（有的文件直接用到，有的没用到）
#[allow(unused_imports)]
fn _unused() {
    let _ = crypto::generate_salt;
    let _ = BASE64.encode(b"");
}

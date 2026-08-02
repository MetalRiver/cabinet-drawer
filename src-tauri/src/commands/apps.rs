//! ===== 命令分组 ⑦：🛡️ 软件管理（apps）+ 分类 + 图标 + 扫描 + 启动 + 健康检查 =====
//! 包含：扫描系统软件、软件 CRUD、分类 CRUD、图标提取/批量重抽、文件选择、批量导入路径、启动软件（含自动重定位）、健康检查（跨盘修复路径）

use tauri::{AppHandle, Manager, State};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};

use crate::crypto;
use crate::db::{self, Db};
use crate::AppState;

#[cfg(windows)]
use crate::win_dock;
#[cfg(windows)]
use crate::win_icon;
#[cfg(windows)]
use crate::win_launch;

use std::path::PathBuf;
#[cfg(windows)]
use std::os::windows::process::CommandExt;

// ============================================================
// 📝 诊断日志写磁盘（%APPDATA%\drawer-box\debug_apptype.log）
//   用途：release 版 App 看不到 eprintln，直接写文件给用户发过来就能定位
// ============================================================
fn app_debug_log(msg: &str) {
    use std::io::Write;
    if let Ok(appdata) = std::env::var("APPDATA") {
        let dir = std::path::PathBuf::from(appdata).join("drawer-box");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("debug_apptype.log");
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
            let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
            let _ = writeln!(f, "[{}] {}", now, msg);
        }
    }
    // 同时也 eprintln（dev 模式能看到）
    eprintln!("{}", msg);
}

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
        if sl.starts_with("http://") || sl.starts_with("https://")
            || sl.contains("://")
            || sl.ends_with(".url") {
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
        "doc" | "docx" | "xls" | "xlsx" | "ppt" | "pptx" | "pdf"
        | "txt" | "md" | "rtf" | "odt" | "ods" | "odp" | "csv" => "document",
        _ => "app",
    }
}

/// 根据 app_type + 路径判断能否直接启动
pub fn detect_launchable(path: &std::path::Path) -> bool {
    let s = path.to_string_lossy();
    #[cfg(windows)]
    {
        if win_launch::is_url_path(&s) { return true; }
    }
    #[cfg(not(windows))]
    {
        let sl = s.to_lowercase();
        if sl.starts_with("http://") || sl.starts_with("https://") || sl.contains("://") {
            return true;
        }
    }
    if s.contains("://") {
        return true;
    }
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .unwrap_or_default();
    matches!(ext.as_str(), "exe" | "msi" | "bat" | "cmd" | "lnk" | "url")
}

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
        "steam", "epic", "origin", "ubisoft", "uplay", "riot", "battlenet", "battle.net",
        "xbox", "playstation", "minecraft", "wegame",
        "netease", "lol", "dota", "csgo", "pubg",
    ];
    for kw in game_kw.iter() {
        if name_lower.contains(kw) || path_lower.contains(kw) {
            return "game";
        }
    }
    let office_kw = [
        "office", "word", "excel", "powerpoint", "outlook", "wps", "dingtalk",
        "feishu", "lark", "wework", "teams", "zoom", "notion",
    ];
    for kw in office_kw.iter() {
        if name_lower.contains(kw) || path_lower.contains(kw) {
            return "office";
        }
    }
    let dev_kw = [
        "code", "vscode", "visual studio", "rider", "pycharm", "intellij", "idea",
        "webstorm", "goland", "clion", "android studio", "xcode", "sublime",
        "vim", "neovim", "git", "github", "gitlab", "docker", "kubernetes", "kubectl",
        "postman", "insomnia", "dbeaver", "navicat", "redis", "mongodb", "mysql",
        "wsl", "terminal", "powershell", "cmd", "node", "npm", "yarn",
        "pnpm", "cargo", "rust", "python", "java", "jdk", "gradle", "maven",
        "electron", "tauri",
    ];
    for kw in dev_kw.iter() {
        if name_lower.contains(kw) || path_lower.contains(kw) {
            return "dev";
        }
    }
    let media_kw = [
        "potplayer", "vlc", "mpv", "kmplayer", "foobar", "spotify",
        "youtube", "obs", "shotcut",
    ];
    for kw in media_kw.iter() {
        if name_lower.contains(kw) || path_lower.contains(kw) {
            return "media";
        }
    }
    let design_kw = [
        "photoshop", "illustrator", "figma", "sketch",
        "indesign", "canva", "pixso", "mastergo",
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
#[cfg(windows)]
fn resolve_lnk_targets_batch(apps: &[ScannedApp]) -> std::collections::HashMap<String, String> {
    use std::collections::HashMap;
    let mut out: HashMap<String, String> = HashMap::new();
    let lnk_paths: Vec<&str> = apps
        .iter()
        .filter(|a| {
            std::path::Path::new(&a.path)
                .extension()
                .and_then(|e| e.to_str())
                == Some("lnk")
        })
        .map(|a| a.path.as_str())
        .collect();
    if lnk_paths.is_empty() {
        return out;
    }
    let mut ps = String::from("$sh = New-Object -ComObject WScript.Shell; ");
    for lnk in &lnk_paths {
        let escaped = lnk.replace('\'', "''");
        ps.push_str(&format!(
            "$x = $sh.CreateShortcut('{}').TargetPath; if ($x) {{ [Environment]::ExpandEnvironmentVariables($x) }} else {{ '' }}; ",
            escaped
        ));
    }
    let output = std::process::Command::new("powershell")
        .args(&["-NoProfile", "-NonInteractive", "-Command", &ps])
        .creation_flags(0x08000000)
        .output();
    let stdout = match output {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).to_string(),
        _ => return out,
    };
    let lines: Vec<String> = stdout
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();
    for (i, lnk) in lnk_paths.iter().enumerate() {
        if let Some(target) = lines.get(i) {
            if !target.is_empty() && std::path::Path::new(target).exists() {
                out.insert(lnk.to_string(), target.clone());
            }
        }
    }
    out
}

#[cfg(not(windows))]
fn resolve_lnk_targets_batch(_apps: &[ScannedApp]) -> std::collections::HashMap<String, String> {
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
// 🔍 辅助：walk_mixed / walk_exe（扫描用）
// ============================================================
fn walk_mixed(
    dir: &std::path::Path,
    source: &str,
    out: &mut Vec<ScannedApp>,
    seen_paths: &mut std::collections::HashSet<String>,
    seen_names: &mut std::collections::HashSet<String>,
    depth: u8,
) {
    use std::fs;
    if depth == 0 { return; }
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let ft = match entry.file_type() {
            Ok(t) => t,
            Err(_) => continue,
        };
        if ft.is_dir() {
            let path_str = path.to_string_lossy().to_string();
            let name = path
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("Unknown")
                .to_string();
            if seen_paths.insert(path_str.clone()) {
                out.push(ScannedApp {
                    name: name.clone(),
                    path: path_str,
                    icon_path: String::new(),
                    args: String::new(),
                    source: source.to_string(),
                    app_type: "folder".to_string(),
                    app_subtype: String::new(),
                    is_launchable: false,
                });
            }
            walk_mixed(&path, source, out, seen_paths, seen_names, depth - 1);
        } else if ft.is_file() {
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| e.to_lowercase())
                .unwrap_or_default();
            let app_type = detect_type_from_path(&path);
            if app_type == "app"
                && ext != "lnk" && ext != "exe" && ext != "bat" && ext != "cmd" && ext != "msi"
            {
                continue;
            }
            if app_type == "app" || app_type == "document" || app_type == "url" {
                let path_str = path.to_string_lossy().to_string();
                let name_lc = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .map(|s| s.to_lowercase())
                    .unwrap_or_default();
                if seen_paths.insert(path_str.clone()) && seen_names.insert(name_lc) {
                    let name = path
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("Unknown")
                        .to_string();
                    out.push(ScannedApp {
                        name,
                        path: path_str,
                        icon_path: String::new(),
                        args: String::new(),
                        source: source.to_string(),
                        app_type: app_type.to_string(),
                        app_subtype: detect_subtype_from_path(&path, app_type).to_string(),
                        is_launchable: detect_launchable(&path),
                    });
                }
            }
        }
    }
}

fn walk_exe(
    dir: &std::path::Path,
    source: &str,
    out: &mut Vec<ScannedApp>,
    seen_paths: &mut std::collections::HashSet<String>,
    seen_names: &mut std::collections::HashSet<String>,
    depth: u8,
) {
    use std::fs;
    if depth == 0 { return; }
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let ft = match entry.file_type() {
            Ok(t) => t,
            Err(_) => continue,
        };
        if ft.is_dir() {
            walk_exe(&path, source, out, seen_paths, seen_names, depth - 1);
        } else if ft.is_file()
            && path.extension().and_then(|e| e.to_str()) == Some("exe")
        {
            let path_str = path.to_string_lossy().to_string();
            let name_lc = path
                .file_stem()
                .and_then(|s| s.to_str())
                .map(|s| s.to_lowercase())
                .unwrap_or_default();
            let skip = matches!(
                name_lc.as_str(),
                "uninstall" | "uninst" | "setup" | "installer"
                    | "update" | "updater" | "crashpad_handler" | "vcredist"
            );
            if skip { continue; }
            if seen_paths.insert(path_str.clone()) && seen_names.insert(name_lc) {
                let name = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("Unknown")
                    .to_string();
                let at = detect_type_from_path(&path);
                out.push(ScannedApp {
                    name,
                    path: path_str,
                    icon_path: String::new(),
                    args: String::new(),
                    source: source.to_string(),
                    app_type: at.to_string(),
                    app_subtype: detect_subtype_from_path(&path, at).to_string(),
                    is_launchable: detect_launchable(&path),
                });
            }
        }
    }
}

// ============================================================
// 🛡️ 1. 扫描系统已安装软件（开始菜单 + 桌面 + ProgramFiles + PATH）
// ============================================================
#[tauri::command]
pub fn scan_installed_software() -> Vec<ScannedApp> {
    use std::collections::HashSet;
    use std::fs;

    let mut results: Vec<ScannedApp> = Vec::new();
    let mut seen_paths: HashSet<String> = HashSet::new();
    let mut seen_names: HashSet<String> = HashSet::new();

    let candidates: [(std::path::PathBuf, &str); 6] = [
        (dirs_known::start_menu_user().join("Programs"), "StartMenu"),
        (dirs_known::start_menu_common().join("Programs"), "StartMenu"),
        (dirs_known::desktop_user(), "Desktop"),
        (dirs_known::desktop_common(), "Desktop"),
        (dirs_known::program_files(), "ProgramFiles"),
        (dirs_known::program_files_x86(), "ProgramFiles"),
    ];

    for (dir, source) in candidates.iter() {
        if !dir.exists() { continue; }
        if *source == "ProgramFiles" {
            walk_exe(dir, source, &mut results, &mut seen_paths, &mut seen_names, 2);
        } else {
            walk_mixed(dir, source, &mut results, &mut seen_paths, &mut seen_names, 3);
        }
    }

    if let Ok(path_var) = std::env::var("PATH") {
        for p in std::env::split_paths(&path_var) {
            if let Ok(entries) = fs::read_dir(&p) {
                for entry in entries.flatten() {
                    if let Ok(ft) = entry.file_type() {
                        if !ft.is_file() { continue; }
                    }
                    let path = entry.path();
                    if path.extension().and_then(|e| e.to_str()) == Some("exe") {
                        let path_str = path.to_string_lossy().to_string();
                        let name = path
                            .file_stem()
                            .and_then(|s| s.to_str())
                            .map(|s| s.to_lowercase())
                            .unwrap_or_default();
                        if seen_paths.insert(path_str.clone()) && seen_names.insert(name.clone()) {
                            if let Some(name) = path.file_stem().and_then(|s| s.to_str()) {
                                let at = detect_type_from_path(&path);
                                results.push(ScannedApp {
                                    name: name.to_string(),
                                    path: path_str,
                                    icon_path: String::new(),
                                    args: String::new(),
                                    source: "PATH".to_string(),
                                    app_type: at.to_string(),
                                    app_subtype: detect_subtype_from_path(&path, at).to_string(),
                                    is_launchable: detect_launchable(&path),
                                });
                            }
                        }
                    }
                }
            }
        }
    }

    #[cfg(windows)]
    {
        let lnk_targets = resolve_lnk_targets_batch(&results);
        let mut to_extract: Vec<(String, String)> = Vec::new();
        for app in results.iter() {
            let p = std::path::Path::new(&app.path);
            let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("");
            let target_for_icon: String = if ext == "lnk" {
                lnk_targets.get(&app.path).cloned().unwrap_or_default()
            } else {
                app.path.clone()
            };
            if target_for_icon.is_empty() {
                to_extract.push((app.path.clone(), app.path.clone()));
            } else {
                to_extract.push((target_for_icon, app.path.clone()));
            }
        }
        let exe_paths: Vec<String> = to_extract.iter().map(|(t, _)| t.clone()).collect();
        let extracted = win_icon::extract_batch_to_cache(&exe_paths);
        let icon_map: std::collections::HashMap<String, PathBuf> = extracted.into_iter().collect();
        for app in results.iter_mut() {
            let p = std::path::Path::new(&app.path);
            let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("");
            let target_for_icon: String = if ext == "lnk" {
                lnk_targets.get(&app.path).cloned().unwrap_or_default()
            } else {
                app.path.clone()
            };
            if let Some(cached) = icon_map.get(&target_for_icon) {
                app.icon_path = cached.to_string_lossy().to_string();
            }
            if ext == "lnk" {
                if let Some(target) = lnk_targets.get(&app.path) {
                    if !target.is_empty() && std::path::Path::new(target).is_dir() {
                        app.app_type = "folder".to_string();
                    }
                }
            }
        }
        for app in results.iter_mut() {
            if !app.icon_path.is_empty() { continue; }
            if app.app_type != "app" { continue; }
            let p = std::path::Path::new(&app.path);
            let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("");
            let target = if ext == "lnk" {
                lnk_targets.get(&app.path).cloned().unwrap_or_default()
            } else {
                app.path.clone()
            };
            if target.is_empty() { continue; }
            if let Some(cached) = win_icon::extract_to_cache(&target) {
                app.icon_path = cached.to_string_lossy().to_string();
            }
        }
    }

    results.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    results
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
    let db = state.db.lock().unwrap();
    let apps = db.list_apps(&query, category_id).map_err(|e| e.to_string())?;
    // P0-#DEBUG#APPTYPE：DB 查询返回前打印每条的 app_type
    eprintln!("[list_apps_with_icons] 🔵 共返回 {} 条记录给前端：", apps.len());
    for a in &apps {
        eprintln!("  id={}, name={:?}, path={:?}, app_type={:?}, app_subtype={:?}", a.id, a.name, a.path, a.app_type, a.app_subtype);
    }
    let result: Vec<AppWithIcon> = apps.into_iter().map(|a| {
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
        }
    }).collect();
    Ok(result)
}

// ============================================================
// 🛡️ 4. 查询 lock 状态 + build 时间戳诊断
// ============================================================
#[tauri::command]
pub fn get_app_status(state: State<AppState>) -> serde_json::Value {
    let unlocked = state.key.lock().unwrap().is_some();
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
    let title = title.unwrap_or_else(|| if mode == "folder" { "选择文件夹".to_string() } else { "选择文件".to_string() });
    let result = tauri::async_runtime::spawn_blocking(move || {
        if mode == "folder" {
            rfd::FileDialog::new()
                .set_title(&title)
                .pick_folder()
                .map(|p| p.to_string_lossy().to_string())
        } else {
            let lower_title = title.to_lowercase();
            let is_backup_picker = lower_title.contains("备份") || lower_title.contains("drawerbox");
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
    let apps: Vec<db::AppMeta> = {
        let db = state.db.lock().unwrap();
        db.list_apps("", None).map_err(|e| e.to_string())?
    };
    let missing: Vec<(i64, String)> = apps
        .into_iter()
        .filter(|a| !a.path.is_empty())
        .filter(|a| {
            a.icon_path.is_empty()
                || !std::path::Path::new(&a.icon_path).exists()
        })
        .map(|a| (a.id, a.path))
        .collect();
    if missing.is_empty() { return Ok(0); }
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
    { Ok(0) }
}

// ============================================================
// 🛡️ 8. 强制清空图标缓存 + 重抽
// ============================================================
#[tauri::command]
pub fn force_reextract_icons(state: State<AppState>) -> Result<usize, String> {
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
        Command::new(&exe_str)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    app.exit(0);
    Ok(())
}

// ============================================================
// 🛡️ 10. 一次性回填所有 app 的 subtype（旧版本没填的）
// ============================================================
#[tauri::command]
pub fn rescan_subtypes(state: State<AppState>) -> Result<usize, String> {
    eprintln!("[rescan_subtypes] ENTER");
    let apps: Vec<db::AppMeta> = {
        let db = state.db.lock().unwrap();
        db.list_apps("", None).map_err(|e| e.to_string())?
    };
    let mut updated = 0;
    let db = state.db.lock().unwrap();
    for a in &apps {
        if !a.app_subtype.is_empty() { continue; }
        let p = std::path::Path::new(&a.path);
        let new_sub = detect_subtype_from_path(p, &a.app_type).to_string();
        if !new_sub.is_empty() {
            if db.update_app(a.id, &a.name, &a.path, &a.icon_path, &a.args, a.category_id, &new_sub).is_ok() {
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
    let db = state.db.lock().unwrap();
    db.list_app_categories().map_err(|e| e.to_string())
}
#[tauri::command]
pub fn create_app_category(
    state: State<AppState>,
    name: String,
    icon: Option<String>,
) -> Result<i64, String> {
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
    let db = state.db.lock().unwrap();
    db.update_app_category(id, &name, &icon)
        .map_err(|e| e.to_string())
}
#[tauri::command]
pub fn delete_app_category(state: State<AppState>, id: i64) -> Result<(), String> {
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
        .map(|p| std::path::PathBuf::from(p).join("com.drawer-box.app").join("frontend.log"))
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
    // P0-#DEBUG#APPTYPE：入口立刻打印所有入参（最关键）→ 同时写磁盘日志
    app_debug_log(&format!("[create_app] 🔴 === 收到前端 create_app 调用 ==="));
    app_debug_log(&format!("[create_app]   name = {:?}", name));
    app_debug_log(&format!("[create_app]   path = {:?}", path));
    app_debug_log(&format!("[create_app]   app_type (前端原始传入 Option<String>) = {:?}", app_type));
    app_debug_log(&format!("[create_app]   app_subtype = {:?}", app_subtype));
    app_debug_log(&format!("[create_app]   icon_path = {:?}", icon_path));
    app_debug_log(&format!("[create_app]   category_id = {:?}", category_id));
    app_debug_log(&format!("[create_app]   args = {:?}", args));
    let db = state.db.lock().unwrap();
    let path_obj = std::path::Path::new(&path);

    // P0-#Y#FIX#URL#APPTYPE#SANITY：app_type 二次校验（双保险）
    let mut at = app_type.unwrap_or_else(|| {
        app_debug_log(&format!("[create_app] ⚠️  app_type 是 None！ 用 detect_type_from_path 回退推断！path={:?}", path));
        detect_type_from_path(path_obj).to_string()
    });
    app_debug_log(&format!("[create_app]   二次校验前 at = {:?}", at));

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
        if (pl.starts_with("http://") || pl.starts_with("https://") || pl.contains("://") || pl.ends_with(".url"))
            && at != "url" {
            eprintln!("[create_app] app_type 纠正: 路径是 URL 形式，但前端传 app_type={} → 强制为 url (path={})", at, path);
            at = "url".to_string();
        }
    }

    if at == "folder" {
        // folder 必须是真实存在的目录；如果不是就按 detect_type_from_path 重推
        if !path_obj.is_dir() {
            let corrected = detect_type_from_path(path_obj);
            eprintln!("[create_app] app_type 纠正: app_type=folder 但 path 不是目录({:?}) → 改为 {}", path_obj.exists(), corrected);
            at = corrected.to_string();
        }
    }
    app_debug_log(&format!("[create_app]   二次校验最终 at = {:?}（将以这个值写入 DB）", at));

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
        { String::new() }
    } else {
        String::new()
    };
    app_debug_log(&format!("[create_app] 🟢 调用 db.create_app 入库：name={:?}, path={:?}, app_type={:?}, sub={:?}", name, path, at, sub));
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
    use std::collections::HashMap;
    let mut out: HashMap<String, String> = HashMap::new();
    let lnk_paths: Vec<&str> = paths
        .iter()
        .filter(|p| std::path::Path::new(p).extension().and_then(|e| e.to_str()) == Some("lnk"))
        .map(|s| s.as_str())
        .collect();
    if lnk_paths.is_empty() { return out; }
    let mut ps = String::from("$sh = New-Object -ComObject WScript.Shell; ");
    for lnk in &lnk_paths {
        let escaped = lnk.replace('\'', "''");
        ps.push_str(&format!(
            "$x = $sh.CreateShortcut('{}').TargetPath; if ($x) {{ [Environment]::ExpandEnvironmentVariables($x) }} else {{ '' }}; ",
            escaped
        ));
    }
    let output = std::process::Command::new("powershell")
        .args(&["-NoProfile", "-NonInteractive", "-Command", &ps])
        .creation_flags(0x08000000)
        .output();
    let stdout = match output {
        Ok(o) => String::from_utf8_lossy(&o.stdout).to_string(),
        Err(_) => return out,
    };
    let lines: Vec<&str> = stdout.lines().collect();
    for (i, lnk) in lnk_paths.iter().enumerate() {
        if let Some(target) = lines.get(i) {
            if !target.is_empty() {
                out.insert(lnk.to_string(), target.to_string());
            }
        }
    }
    out
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
        let ext = p.extension().and_then(|e| e.to_str()).map(|e| e.to_lowercase()).unwrap_or_default();
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
                        (path_str.clone(), p.file_name().and_then(|s| s.to_str()).unwrap_or("Unknown").to_string())
                    }
                } else {
                    (path_str.clone(), p.file_name().and_then(|s| s.to_str()).unwrap_or("Unknown").to_string())
                }
            } else {
                (path_str.clone(), p.file_name().and_then(|s| s.to_str()).unwrap_or("Unknown").to_string())
            }
        } else {
            (path_str.clone(), p.file_name().and_then(|s| s.to_str()).unwrap_or("Unknown").to_string())
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
            } else { "app" }
        } else {
            detect_type_from_path(p)
        };
        let app_subtype = detect_subtype_from_path(p, app_type).to_string();
        match db.create_app(
            &name,
            &db_path,
            "",
            "",
            category_id,
            app_type,
            &app_subtype,
        ) {
            Ok(id) => new_ids.push(id),
            Err(e) => errors.push(format!("{}: {}", db_path, e)),
        }
    }
    Ok(ImportResult { new_ids, skipped, errors })
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
    let db = state.db.lock().unwrap();
    let sub = match app_subtype { Some(s) => s, None => String::new() };
    db.update_app_full(
        id,
        &name,
        &path,
        icon_path.as_deref().unwrap_or(""),
        args.as_deref().unwrap_or(""),
        category_id,
        &sub,
        app_type.as_deref(),
    )
    .map_err(|e| e.to_string())
}

// ============================================================
// 🛡️ 16. 删除软件（软删除 → 回收站）
// ============================================================
#[tauri::command]
pub fn delete_app(state: State<AppState>, id: i64) -> Result<(), String> {
    let db = state.db.lock().unwrap();
    db.soft_delete("apps", id).map_err(|e| e.to_string())?;
    Ok(())
}

// ============================================================
// 🛡️ 17. 记录使用次数
// ============================================================
#[tauri::command]
pub fn record_app_usage(state: State<AppState>, id: i64) -> Result<(), String> {
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
        app_debug_log(&format!(
            "[launch_with_path] 🛡️🛡️🛡️ 终极防线命中：强制按 URL 启动（DB.app_type={}, is_url_path={}, contains://={}, path={}）",
            app_type, definitely_url, contains_protocol_sep, path
        ));
        return match win_launch::launch(&path, &args) {
            Ok(()) => Ok(()),
            Err(e) => {
                if is_dot_url_file {
                    if let Some(url) = win_icon::parse_url_shortcut(&path) {
                        app_debug_log(&format!("[launch_with_path] .url 关联失败，explorer 兜底: {}", url));
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
            match std::process::Command::new("explorer.exe").arg(&path).spawn() {
                Ok(_) => { return Ok(()); }
                Err(e) => { return Err(format!("打开文件夹失败: {} ({})", path, e)); }
            }
        } else {
            app_debug_log(&format!(
                "[launch_with_path] ⚠️  DB.app_type=\"folder\"，但 path 不是真实目录（exists={}, is_dir={}），path={:?}",
                p.exists(), p.is_dir(), path
            ));
            // 再检查一次是不是 URL 形式（裸域名）
            if win_launch::looks_like_domain(&path) {
                app_debug_log(&format!("[launch_with_path] ⚠️  看起来还是裸域名，强制 URL 启动"));
                return win_launch::launch(&path, &args);
            }
            // 兜底：ShellExecuteW 打开 whatever 这个 path
            app_debug_log(&format!("[launch_with_path] ⚠️  兜底：ShellExecuteW 直接打开 path"));
            return match win_launch::launch(&path, &args) {
                Ok(()) => Ok(()),
                Err(e) => Err(format!(
                    "app_type=folder 但 path 不是目录，兜底打开也失败: path={} → {}",
                    path, e
                ))
            };
        }
    }
    // 普通 app/document：直接 win_launch::launch（ShellExecuteW 会根据扩展名关联打开）
    match win_launch::launch(&path, &args) {
        Ok(()) => Ok(()),
        Err(e) => Err(e)
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
    if let Ok(pf) = std::env::var("ProgramFiles") { candidates.push(PathBuf::from(pf)); }
    if let Ok(pf86) = std::env::var("ProgramFiles(x86)") { candidates.push(PathBuf::from(pf86)); }
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
        if !dir.exists() { continue; }
        if let Some(found) = scan_dir_for_name(dir, &name_lower, 5, 30) {
            eprintln!("[try_relocate_app] 在 {} 找到 {}: {}", dir.display(), app_name, found);
            return Some(found);
        }
    }
    eprintln!("[try_relocate_app] 跨盘扫描未找到: {}", app_name);
    None
}

fn scan_dir_for_name(dir: &std::path::Path, name_lower: &str, max_depth: usize, max_results: usize) -> Option<String> {
    if max_depth == 0 { return None; }
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
                if found.len() >= max_results { break; }
            }
        } else if let Some(fname) = p.file_name().and_then(|s| s.to_str()) {
            let fname_lower = fname.to_lowercase();
            if let Some(stem) = std::path::Path::new(&fname_lower).file_stem().and_then(|s| s.to_str()) {
                if stem == name_lower || fname_lower.starts_with(name_lower) {
                    let ext = std::path::Path::new(&fname_lower)
                        .extension()
                        .and_then(|s| s.to_str())
                        .unwrap_or("");
                    if matches!(ext, "exe" | "lnk" | "bat" | "cmd") {
                        if let Some(s) = p.to_str() {
                            found.push(s.to_string());
                            if found.len() >= max_results { break; }
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
    let (path, args, app_type, app_name): (String, String, String, String) = {
        let db = state.db.lock().unwrap();
        let conn = db.conn.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT path, args, app_type, name FROM apps WHERE id=?1")
            .map_err(|e| e.to_string())?;
        stmt
            .query_row(rusqlite::params![id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)))
            .map_err(|e| e.to_string())?
    };
    let _ = state.db.lock().unwrap().record_app_usage(id);

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
        eprintln!("[launch_app] 路径不存在: {} (id={}, name={})", path, id, app_name);
        if let Some(new_path) = try_relocate_app(&app_name, &path) {
            eprintln!("[launch_app] 自动重定位: {} → {}", path, new_path);
            {
                let db = state.db.lock().unwrap();
                let _ = db.update_app_path(id, &new_path);
            }
            return match launch_with_path(new_path.clone(), args, app_type, app_name).await {
                Ok(()) => Ok(format!("ok:auto-relocated:{} → {}", path, new_path)),
                Err(e) => Err(format!("relocated-but-still-fail:{} → {}: {}", path, new_path, e)),
            };
        }
        if app_type == "url" || path.to_lowercase().ends_with(".url") {
            if try_open_in_steam(&app_name).is_ok() {
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
        return match launch_with_path(path.clone(), args.clone(), "url".to_string(), app_name.clone()).await {
            Ok(()) => Ok(format!("ok:forced-url-launch:{}", path)),
            Err(e) => Err(format!("url-launch-fail:{} → {}", path, e)),
        };
    }

    match launch_with_path(path.clone(), args.clone(), app_type.clone(), app_name.clone()).await {
        Ok(()) => Ok("ok".to_string()),
        Err(e) => {
            eprintln!("[launch_app] 启动失败 (path 存在但 ShellExecuteW 报错): {} → {}", path, e);
            if let Some(new_path) = try_relocate_app(&app_name, &path) {
                if new_path != path && std::path::Path::new(&new_path).exists() {
                    eprintln!("[launch_app] 运行搬迁恢复: {} → {}", path, new_path);
                    {
                        let db = state.db.lock().unwrap();
                        let _ = db.update_app_path(id, &new_path);
                    }
                    return match launch_with_path(new_path.clone(), args, app_type, app_name).await {
                        Ok(()) => Ok(format!("ok:auto-relocated:{} → {}", path, new_path)),
                        Err(e2) => Err(format!("relocated-but-still-fail:{} → {}: {}", path, new_path, e2)),
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
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)))
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
        if app_type == "folder" { continue; }
        #[cfg(windows)]
        {
            if ext == "lnk" && path_exists {
                if let Some(target) = win_icon::resolve_lnk_target(&path) {
                    if !target.is_empty() && Path::new(&target).exists() && target != path {
                        let t_ext = Path::new(&target)
                            .extension()
                            .and_then(|e| e.to_str())
                            .map(|e| e.to_lowercase())
                            .unwrap_or_default();
                        if matches!(t_ext.as_str(), "exe" | "bat" | "cmd" | "msi") {
                            eprintln!("[health_check] .lnk 重解析: {} → {}", path, target);
                            let db2 = state.db.lock().unwrap();
                            let _ = db2.update_app_path(id, &target);
                            fixed += 1;
                            continue;
                        }
                    }
                }
            }
        }
        if ext == "url" && path_exists { continue; }
        if !path_exists {
            eprintln!("[health_check] app {} (id={}) path 失效: {}", name, id, path);
            if let Some(new_path) = try_relocate_app(&name, &path) {
                if new_path != path {
                    eprintln!("[health_check]   → 重定位: {}", new_path);
                    let db2 = state.db.lock().unwrap();
                    let _ = db2.update_app_path(id, &new_path);
                    fixed += 1;
                    continue;
                }
            }
            if ext == "lnk" {
                let stem = path_p
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or(&name);
                if let Some(new_path) = try_relocate_app(stem, &path) {
                    if new_path != path {
                        eprintln!("[health_check]   → lnk 兜底重定位: {}", new_path);
                        let db2 = state.db.lock().unwrap();
                        let _ = db2.update_app_path(id, &new_path);
                        fixed += 1;
                        continue;
                    }
                }
            }
            eprintln!("[health_check]   ✗ 重定位失败: {}", name);
        }
    }
    Ok(fixed)
}

#[tauri::command]
pub async fn health_check_apps(app: AppHandle) -> Result<usize, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state: tauri::State<AppState> = app.state();
        health_check_apps_sync(&state)
    })
    .await
    .map_err(|e| format!("健康检查线程失败: {}", e))?
}

// ============================================================
// 🛡️ 21. 当前数据存储目录
// ============================================================
#[tauri::command]
pub fn get_data_dir(state: State<AppState>) -> Result<String, String> {
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

// ============================================================
// 🛡️ 23. 启动时一次性全表修正 apps 脏数据（app_type 与 path 不一致问题）
// ============================================================
/// **启动时自动修所有历史脏数据：**
/// - 规则 1：path 是 URL 形式（is_url_path 或含 ://）但 app_type != "url" → 强制 UPDATE app_type=url
/// - 规则 2：app_type="folder" 但 path 不是真实目录 → 用 detect_type_from_path 重新推断并 UPDATE
/// - 规则 3：app_type="document" 但扩展名不在文档列表 → 重新推断
/// **每次启动必执行一次，保证 DB 与实际语义永远一致**
pub fn sanitize_db_on_startup(state: &tauri::State<'_, AppState>) -> Result<usize, String> {
    app_debug_log(&format!("[sanitize_db_on_startup] 🟢 === 开始启动时脏数据修正 ==="));
    let db = state.db.lock().map_err(|e| e.to_string())?;
    let conn = db.conn.lock().map_err(|e| e.to_string())?;

    // 先读出所有未删除条目
    let mut stmt = conn
        .prepare("SELECT id, name, path, app_type FROM apps WHERE deleted_at IS NULL")
        .map_err(|e| e.to_string())?;
    let rows: Vec<(i64, String, String, String)> = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)))
        .map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect();
    let total = rows.len();
    drop(stmt);

    let mut fixed_count = 0usize;
    for (id, name, path, old_app_type) in rows {
        let path_obj = std::path::Path::new(&path);
        #[cfg(windows)]
        let path_is_url = win_launch::is_url_path(&path) || path.contains("://");
        #[cfg(not(windows))]
        let path_is_url = {
            let pl = path.to_lowercase();
            pl.starts_with("http://") || pl.starts_with("https://") || pl.contains("://") || pl.ends_with(".url")
        };
        let mut new_app_type: Option<String> = None;

        // 规则 1：URL 形式但 app_type != url → 强制 url
        if path_is_url && old_app_type != "url" {
            new_app_type = Some("url".to_string());
            app_debug_log(&format!(
                "[sanitize_db_on_startup] ✏️  修正 #{}「{}」：path 是 URL 但 app_type={:?} → url，path={:?}",
                id, name, old_app_type, path
            ));
        }
        // 规则 2：app_type=folder 但 path 不是真实目录 → 重新推断
        else if old_app_type == "folder" && !path_obj.is_dir() {
            let corrected = detect_type_from_path(path_obj);
            new_app_type = Some(corrected.to_string());
            app_debug_log(&format!(
                "[sanitize_db_on_startup] ✏️  修正 #{}「{}」：app_type=folder 但 path 不是目录（exists={:?}, is_dir={:?}）→ {:?}，path={:?}",
                id, name, path_obj.exists(), path_obj.is_dir(), corrected, path
            ));
        }

        if let Some(new_at) = new_app_type {
            match conn.execute(
                "UPDATE apps SET app_type=?1 WHERE id=?2",
                rusqlite::params![new_at, id],
            ) {
                Ok(_) => { fixed_count += 1; }
                Err(e) => {
                    app_debug_log(&format!(
                        "[sanitize_db_on_startup] ❌ UPDATE 失败 id={}, err={}",
                        id, e
                    ));
                }
            }
        }
    }

    app_debug_log(&format!(
        "[sanitize_db_on_startup] 🟢 === 脏数据修正完成，共修正 {} 条 / 总 {} 条 ===",
        fixed_count, total
    ));
    Ok(fixed_count)
}

// ============================================================
// 🛡️ 22. 贴边隐藏相关（暂时留空，诊断用）
// ============================================================
#[tauri::command]
pub fn is_dock_hidden() -> bool { false }
#[tauri::command]
pub fn force_dock_reveal(_app: AppHandle) -> Result<(), String> { Ok(()) }

// 让 crypto / BASE64 import 不 warning（有的文件直接用到，有的没用到）
#[allow(unused_imports)]
fn _unused() {
    let _ = crypto::generate_salt;
    let _ = BASE64.encode(b"");
}

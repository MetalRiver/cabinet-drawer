//! v0.4.3 Batch 2.1：入口身份 / 分类矩阵 / 导入幂等 回归。
//!
//! 钉死三件事：
//! 1. 身份键 = 类型 ∥ 规范化目标 ∥ 规范化参数（Chrome PWA 等不同参数入口不合并）；
//! 2. 分类矩阵：exe/脚本/协议/网址/目录/文档/解析失败/失效目标 八类各归其位，
//!    F1（Discord→Update.exe）、F2（System32）、F3（WindowsApps）不误过滤；
//! 3. 批量导入幂等：重复导入 = already_exists；不同参数并存；回收站命中提示恢复。

use crate::commands::apps::{
    advanced_status_for, classify_entry, entry_identity_key, import_candidates_core,
    normalize_path_key, ScannedApp,
};
use crate::db::Db;
use std::fs;
use std::path::PathBuf;

// ============ 身份键 ============

#[test]
fn t_si01_规范化_大小写斜杠尾分隔符() {
    assert_eq!(
        normalize_path_key("C:\\Apps\\Tool.EXE\\"),
        "c:\\apps\\tool.exe"
    );
    assert_eq!(normalize_path_key("c:/apps/tool.exe"), "c:\\apps\\tool.exe");
    assert_eq!(normalize_path_key("  C:\\A\\B  "), "c:\\a\\b");
}

#[test]
fn t_si02_同目标不同参数_身份不同() {
    let a = entry_identity_key("exe", "C:\\Chrome\\chrome.exe", "");
    let b = entry_identity_key("exe", "c:\\chrome\\chrome.exe", "--app=https://x.y");
    assert_ne!(a, b, "Chrome PWA（不同参数）不得与浏览器本体合并");
}

#[test]
fn t_si03_同目标同参数_不同lnk文件_身份相同() {
    let a = entry_identity_key("exe", "C:\\Apps\\Tool.exe", "");
    let b = entry_identity_key("exe", "c:\\apps\\tool.exe", "");
    assert_eq!(a, b, "开始菜单与桌面快捷方式指向同一目标 → 同一入口");
}

#[test]
fn t_si04_类型参与身份_url与exe不混() {
    assert_ne!(
        entry_identity_key("url", "https://x.y", ""),
        entry_identity_key("exe", "https://x.y", "")
    );
}

// ============ 分类矩阵（F1-F7 fixture）============

fn c(
    is_url: bool,
    target: &str,
    args: &str,
    exists: bool,
    is_dir: bool,
) -> Option<(&'static str, &'static str, &'static str, bool)> {
    classify_entry(is_url, target, args, exists, is_dir)
        .map(|e| (e.app_type, e.app_subtype, e.status, e.is_launchable))
}

#[test]
fn t_si05_F1_更新器目标入口不误杀() {
    // Discord.lnk → Update.exe --processStart Discord.exe：lnk 候选不做文件名过滤
    let r = c(
        false,
        "C:\\Users\\u\\AppData\\Discord\\Update.exe",
        "--processStart Discord.exe",
        true,
        false,
    );
    assert_eq!(
        r,
        Some(("app", "utility", "identified", true)),
        "F1 回归：Update.exe 目标被误杀"
    );
}

#[test]
fn t_si06_F2_系统目录目标不剔除() {
    // 命令提示符 lnk → System32\cmd.exe：不做路径级剔除。
    // subtype 沿用既有关键词推断（cmd 命中 dev），本测试只锁"不剔除+已识别"。
    let r = c(false, "C:\\Windows\\System32\\cmd.exe", "", true, false);
    assert_eq!(r, Some(("app", "dev", "identified", true)));
}

#[test]
fn t_si07_脚本启动器分类() {
    assert_eq!(
        c(false, "C:\\tool\\start.bat", "", true, false),
        Some(("app", "script", "identified", true))
    );
    assert_eq!(
        c(false, "C:\\tool\\run.cmd", "", true, false),
        Some(("app", "script", "identified", true))
    );
    assert_eq!(
        c(false, "C:\\tool\\setup.vbs", "", true, false),
        Some(("app", "script", "identified", true))
    );
    // .ps1 默认关联是记事本 → 需要确认（启动适配待后续批次）
    assert_eq!(
        c(false, "C:\\tool\\run.ps1", "", true, false),
        Some(("app", "script", "needs_confirmation", false))
    );
}

#[test]
fn t_si08_目录与文档目标排除() {
    assert_eq!(
        c(false, "C:\\SomeFolder", "", true, true),
        None,
        "文件夹快捷方式不是软件"
    );
    assert_eq!(
        c(false, "C:\\docs\\readme.pdf", "", true, false),
        None,
        "文档不是软件"
    );
    assert_eq!(
        c(false, "C:\\docs\\报告.docx", "", true, false),
        None,
        "中文文档名同样排除"
    );
}

#[test]
fn t_si09_解析失败与失效目标_需要确认不丢弃() {
    // 解析失败（空目标）→ 需要确认（可见可选，绝不错绑）
    assert_eq!(
        c(false, "", "", false, false),
        Some(("app", "", "needs_confirmation", true))
    );
    // 目标已失效（卸载/搬迁）→ 需要确认（启动时可尝试自动重定位）
    assert_eq!(
        c(false, "C:\\gone\\app.exe", "", false, false),
        Some(("app", "", "needs_confirmation", true))
    );
}

#[test]
fn t_si10_url文件恒为已识别网址类型() {
    assert_eq!(
        c(true, "https://example.com", "", true, false),
        Some(("url", "", "identified", true))
    );
    assert_eq!(
        c(true, "", "", false, false),
        Some(("url", "", "identified", true)),
        "解析不出网址的 .url 仍是网址入口"
    );
}

#[test]
fn t_si11_高级扫描模糊模式降级不删除() {
    assert_eq!(advanced_status_for("elevate_util"), "needs_confirmation");
    assert_eq!(advanced_status_for("crash_reporter"), "needs_confirmation");
    assert_eq!(advanced_status_for("firefox"), "identified");
}

// ============ 导入幂等（临时库）============

fn temp_db(label: &str) -> (PathBuf, Db) {
    let dir = std::env::temp_dir().join(format!(
        "drawer-scanident-{}-{}",
        label,
        uuid::Uuid::new_v4()
    ));
    fs::create_dir_all(&dir).expect("建临时目录失败");
    let path = dir.join("drawer-v2.db");
    let db = Db::open(&path).expect("建测试库失败");
    (dir, db)
}

fn cand(name: &str, path: &str, target: &str, target_args: &str, app_type: &str) -> ScannedApp {
    ScannedApp {
        name: name.to_string(),
        path: path.to_string(),
        icon_path: String::new(),
        args: String::new(),
        source: "开始菜单".to_string(),
        app_type: app_type.to_string(),
        app_subtype: String::new(),
        is_launchable: true,
        target: target.to_string(),
        target_args: target_args.to_string(),
        workdir: String::new(),
        status: "identified".to_string(),
        already_imported: false,
        in_trash: false,
        trash_id: None,
    }
}

fn app_count(db: &Db) -> i64 {
    let conn = db.conn.lock().unwrap();
    conn.query_row(
        "SELECT COUNT(*) FROM apps WHERE deleted_at IS NULL",
        [],
        |r| r.get(0),
    )
    .unwrap()
}

#[test]
fn t_si12_导入幂等_重复导入不再新增() {
    let (dir, db) = temp_db("imp1");
    let c1 = vec![cand(
        "工具A",
        "C:\\lnk\\a.lnk",
        "C:\\Apps\\toolA.exe",
        "",
        "app",
    )];
    let s1 = import_candidates_core(&db, &c1, None);
    assert_eq!(s1.added, 1);
    assert_eq!(s1.already_exists, 0);
    // 同一批再导一遍 → 全部已存在（幂等），库行数不变
    let s2 = import_candidates_core(&db, &c1, None);
    assert_eq!(s2.added, 0);
    assert_eq!(s2.already_exists, 1);
    assert_eq!(app_count(&db), 1);
    // 同目标不同参数（PWA）→ 独立新条目，不误合并
    let c2 = vec![cand(
        "工具A·配置B",
        "C:\\lnk\\a2.lnk",
        "C:\\Apps\\toolA.exe",
        "--profile b",
        "app",
    )];
    let s3 = import_candidates_core(&db, &c2, None);
    assert_eq!(s3.added, 1, "不同参数入口不得误合并");
    assert_eq!(app_count(&db), 2);
    drop(dir);
}

#[test]
fn t_si13_回收站命中_提示恢复不自动恢复() {
    let (dir, db) = temp_db("imp2");
    // 先建一条，再软删（进回收站）
    let id = db
        .create_app("工具B", "C:\\lnk\\b.lnk", "", "", None, "app", "")
        .unwrap();
    db.soft_delete("apps", id).unwrap();
    assert_eq!(app_count(&db), 0);
    // 同身份（同 lnk 路径）再导入 → in_trash 提示，不新增、不自动恢复
    let c1 = vec![cand(
        "工具B",
        "C:\\lnk\\b.lnk",
        "C:\\Apps\\toolB.exe",
        "",
        "app",
    )];
    let s = import_candidates_core(&db, &c1, None);
    assert_eq!(s.in_trash, 1, "回收站同身份应提示恢复");
    assert_eq!(s.added, 0, "不得绕过回收站直接重建");
    assert_eq!(app_count(&db), 0);
    drop(dir);
}

#[test]
fn t_si14_不同目标_不同条目() {
    let (dir, db) = temp_db("imp3");
    let c1 = vec![
        cand("软件甲", "C:\\lnk\\x.lnk", "C:\\Apps\\x.exe", "", "app"),
        cand(
            "软件乙",
            "C:\\lnk\\y.lnk",
            "D:\\ portable\\y.exe",
            "",
            "app",
        ),
    ];
    let s = import_candidates_core(&db, &c1, None);
    assert_eq!(s.added, 2);
    drop(dir);
}

// ============ Batch 2.3：自定义目录 / PATH / 取消 / 便携去重 ============

#[test]
fn t_si15_自定义目录_便携exe直接识别_辅助名降级() {
    // 构造隔离目录：portable.exe（正常）+ helper.exe（模糊辅助名）+ 文档 + 子目录
    let root = std::env::temp_dir().join(format!("drawer-custom-scan-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(root.join("sub")).unwrap();
    fs::write(root.join("MyPortableTool.exe"), b"MZ").unwrap();
    fs::write(root.join("crash_reporter.exe"), b"MZ").unwrap();
    fs::write(root.join("readme.txt"), b"x").unwrap();
    fs::write(root.join("sub").join("inner.exe"), b"MZ").unwrap();
    let (raw, truncated) =
        crate::commands::apps::scan_custom_raw(&root).expect("自定义目录扫描失败");
    assert!(!truncated);
    assert_eq!(raw.len(), 3, "只收 exe（lnk/url 无）；文档不收");
    let out = crate::commands::apps::build_candidates(raw);
    assert_eq!(out.len(), 3);
    let normal = out.iter().find(|a| a.name == "MyPortableTool").unwrap();
    assert_eq!(normal.status, "identified", "便携 exe 直接识别");
    assert_eq!(normal.source, "自定义目录");
    assert_eq!(
        normal.target.to_lowercase(),
        root.join("MyPortableTool.exe")
            .to_string_lossy()
            .to_lowercase()
    );
    let helper = out.iter().find(|a| a.name == "crash_reporter").unwrap();
    assert_eq!(helper.status, "needs_confirmation", "模糊辅助名降级不删除");
    // 子目录内的 exe 也在（深度内）
    assert!(out.iter().any(|a| a.name == "inner"));
    fs::remove_dir_all(&root).ok();
}

#[test]
fn t_si16_自定义目录_非法输入_明确报错不扫描() {
    // UNC（双反斜杠开头）
    let unc = "\\\\server\\share";
    let r = crate::commands::apps::scan_custom_raw(std::path::Path::new(unc));
    assert!(r.is_err());
    // 不存在
    let missing = "C:\\definitely\\not\\here-7f3d";
    let r = crate::commands::apps::scan_custom_raw(std::path::Path::new(missing));
    assert!(r.is_err());
    // 相对路径
    let r = crate::commands::apps::scan_custom_raw(std::path::Path::new("relative\\path"));
    assert!(r.is_err());
}

#[test]
fn t_si17_便携exe与桌面快捷方式_同身份不重复() {
    // 场景：用户先从桌面快捷方式导入，之后在便携目录扫描到同一个 exe。
    // 用【真实 .lnk 文件】走完整 PS 解析链路（库行身份 = 解析出的目标）。
    let root = std::env::temp_dir().join(format!("drawer-si17-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let exe_path_raw = root.join("portable-tool.exe");
    fs::write(&exe_path_raw, b"MZ").unwrap();
    // 统一为长路径：env::temp_dir 可能返回短名（ADMINI~1），而 WScript.Shell
    // TargetPath 返回长路径——身份键必须基于同一形态（normalize 不解析短名，
    // 这是已登记的规范化边界）。
    let exe_path = {
        let c = exe_path_raw
            .canonicalize()
            .unwrap()
            .to_string_lossy()
            .to_string();
        c.strip_prefix(r"\\?\").map(|s| s.to_string()).unwrap_or(c)
    };
    let lnk_path = root.join("tool.lnk");
    // 用 PowerShell 创建真实快捷方式（与生产解析同一 COM 组件）
    let ps = format!(
        "$s = (New-Object -ComObject WScript.Shell).CreateShortcut('{}'); $s.TargetPath = '{}'; $s.Save();",
        lnk_path.to_string_lossy().replace('\'', "''"),
        exe_path.replace('\'', "''"),
    );
    let st = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &ps])
        .status()
        .expect("创建 lnk 失败");
    assert!(st.success(), "真实 lnk 创建失败");

    let exe_c = cand("便携版", exe_path.as_str(), exe_path.as_str(), "", "app");
    let lnk_c = cand(
        "桌面快捷方式",
        lnk_path.to_string_lossy().as_ref(),
        exe_path.as_str(),
        "",
        "app",
    );
    let k1 = crate::commands::apps::candidate_identity_keys(&exe_c);
    let k2 = crate::commands::apps::candidate_identity_keys(&lnk_c);
    assert!(
        k1.iter().any(|k| k2.contains(k)),
        "便携软件与指向它的桌面快捷方式必须判为同身份：{:?} vs {:?}",
        k1,
        k2
    );
    // 导入侧：先导 lnk 再导 exe → already_exists（库行经真实 lnk 重解析对上目标）
    let (dir, db) = temp_db("imp4");
    let s1 = import_candidates_core(&db, &[lnk_c], None);
    assert_eq!(s1.added, 1);
    let s2 = import_candidates_core(&db, &[exe_c], None);
    assert_eq!(s2.already_exists, 1, "便携/快捷方式重复导入不得重复建");
    assert_eq!(app_count(&db), 1);
    drop(dir);
    fs::remove_dir_all(&root).ok();
}

#[test]
fn t_si18_取消旗标_遍历与解析真正终止() {
    use crate::commands::apps::SCAN_CANCELLED;
    use std::sync::atomic::Ordering;
    // 置位后：PATH 枚举立即为空（真正终止，非取消等待）
    SCAN_CANCELLED.store(true, Ordering::Relaxed);
    let raw = crate::commands::apps::scan_path_raw();
    assert!(raw.is_empty(), "取消后 PATH 枚举不得产出条目");
    // 复位后恢复正常
    SCAN_CANCELLED.store(false, Ordering::Relaxed);
    let _ = crate::commands::apps::scan_path_raw();
}

#[test]
fn t_si19_PATH状态_辅助模式降级_正常识别() {
    // PATH 候选是直接 exe：build_candidates 走 direct-exe 分支
    let root = std::env::temp_dir().join(format!("drawer-path-scan-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("devtool.exe"), b"MZ").unwrap();
    fs::write(root.join("crashpad_handler.exe"), b"MZ").unwrap();
    let mut raw = Vec::new();
    let mut seen = std::collections::HashSet::new();
    // 直接复用 walk_custom 的直接 exe 语义：手工构造 raw（模拟 PATH 枚举结果）
    for name in ["devtool.exe", "crashpad_handler.exe"] {
        let p = root.join(name).to_string_lossy().to_string();
        if seen.insert(p.clone()) {
            raw.push((
                p,
                name.trim_end_matches(".exe").to_string(),
                "PATH".to_string(),
            ));
        }
    }
    let out = crate::commands::apps::build_candidates(raw);
    assert_eq!(out.len(), 2);
    let dev = out.iter().find(|a| a.name == "devtool").unwrap();
    assert_eq!(dev.status, "identified");
    assert_eq!(dev.source, "PATH");
    let crash = out.iter().find(|a| a.name == "crashpad_handler").unwrap();
    assert_eq!(crash.status, "needs_confirmation");
    fs::remove_dir_all(&root).ok();
}

// ============ 真实机器样本基线（#[ignore]，显式运行）============
// 运行：cargo test t_real -- --ignored --nocapture
// 作用：可重复的耗时/数量基线 + 默认扫描噪声源头消除的机器级断言。

#[test]
#[ignore = "真实机器性能基线：显式运行 cargo test t_real -- --ignored --nocapture"]
fn t_real_四类扫描性能基线() {
    use crate::commands::apps::{scan_custom_raw, scan_default_raw, scan_path_raw, scan_pf_raw};
    // 自定义目录样本：使用开始菜单 Programs（真实、中等规模、可复现）
    let custom_dir = dirs_root();

    fn run5<F: FnMut() -> Vec<(String, String, String)>>(mut f: F, label: &str) {
        let mut times: Vec<f64> = Vec::new();
        let mut counts: Vec<usize> = Vec::new();
        let mut errors = 0usize;
        for _ in 0..5 {
            let t0 = std::time::Instant::now();
            let raw = f();
            let out = crate::commands::apps::build_candidates(raw);
            times.push(t0.elapsed().as_secs_f64() * 1000.0);
            counts.push(out.len());
            let _ = &mut errors;
        }
        times.sort_by(|a, b| a.partial_cmp(b).unwrap());
        counts.sort_unstable();
        let p50 = times[2];
        let p90 = times[(times.len() as f64 * 0.9).ceil() as usize - 1];
        println!(
            "{:<14} 5次耗时ms={:?} p50={:.0} p90={:.0}  候选数={:?}（中位 {}）",
            label, times, p50, p90, counts, counts[2]
        );
    }

    println!("=== 四类扫描性能基线（各 5 次，含目标解析与图标提取）===");
    run5(|| scan_default_raw(), "默认扫描");
    run5(|| scan_pf_raw(), "ProgramFiles");
    run5(|| scan_path_raw(), "PATH");
    let cd = custom_dir.clone();
    run5(
        move || scan_custom_raw(std::path::Path::new(&cd)).unwrap().0,
        "自定义目录",
    );
    println!("（门槛待本基线多次采集后再定；取消/进度见 scan:progress 事件）");
}

fn dirs_root() -> String {
    std::env::var("APPDATA").unwrap() + r"\Microsoft\Windows\Start Menu\Programs"
}

#[test]
#[ignore = "诊断：Rust 侧 PS 解析链路隔离"]
fn t_diag_lnk_resolve_rust_vs_plain() {
    // 取 3 个真实 lnk
    let mut lnks: Vec<String> = Vec::new();
    let root = std::env::var("APPDATA").unwrap() + "\\Microsoft\\Windows\\Start Menu\\Programs";
    fn collect(dir: &std::path::Path, out: &mut Vec<String>, depth: u8) {
        if depth == 0 {
            return;
        }
        for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() {
                collect(&p, out, depth - 1);
            } else if p.extension().and_then(|x| x.to_str()) == Some("lnk") {
                out.push(p.to_string_lossy().to_string());
            }
            if out.len() >= 3 {
                return;
            }
        }
    }
    collect(std::path::Path::new(&root), &mut lnks, 5);
    println!("样本: {:?}", lnks);
    let mut ps = String::from(
        "[Console]::OutputEncoding=[System.Text.Encoding]::UTF8; \
         $ErrorActionPreference='SilentlyContinue'; \
         $sh = New-Object -ComObject WScript.Shell; ",
    );
    for (i, lnk) in lnks.iter().enumerate() {
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
    println!("--- 生成的 PS（前 600 字）---");
    println!("{}", &ps[..ps.len().min(600)]);
    // 方式 1：简单 output()
    let o1 = std::process::Command::new("powershell")
        .args(&["-NoProfile", "-NonInteractive", "-Command", &ps])
        .output();
    match o1 {
        Ok(o) => {
            println!(
                "方式1 output(): status={:?} stdout={}B",
                o.status,
                o.stdout.len()
            );
            println!(
                "  stdout 前 300: {}",
                String::from_utf8_lossy(&o.stdout[..o.stdout.len().min(300)])
            );
            if !o.stderr.is_empty() {
                println!(
                    "  stderr 前 300: {}",
                    String::from_utf8_lossy(&o.stderr[..o.stderr.len().min(300)])
                );
            }
        }
        Err(e) => println!("方式1 启动失败: {e}"),
    }
    // 方式 2：走仓库核心（spawn+线程排空+CREATE_NO_WINDOW）
    let m = crate::commands::apps::resolve_lnk_paths_raw(&lnks, None);
    println!(
        "方式2 resolve_lnk_paths_raw(): 命中 {}/{}",
        m.len(),
        lnks.len()
    );
    for (k, v) in m.iter().take(3) {
        println!(
            "  {:?} -> {:?}",
            std::path::Path::new(k).file_name(),
            v.target
        );
    }
}

#[test]
#[ignore = "诊断 t_si17"]
fn t_diag_si17_keys() {
    let root = std::env::temp_dir().join(format!("drawer-diag17-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let exe_path = root.join("portable-tool.exe");
    fs::write(&exe_path, b"MZ").unwrap();
    let lnk_path = root.join("tool.lnk");
    let ps = format!(
        "$s = (New-Object -ComObject WScript.Shell).CreateShortcut('{}'); $s.TargetPath = '{}'; $s.Save();",
        lnk_path.to_string_lossy().replace('\'', "''"),
        exe_path.to_string_lossy().replace('\'', "''"),
    );
    std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &ps])
        .status()
        .unwrap();
    let resolved = crate::commands::apps::resolve_lnk_paths_raw(
        &[lnk_path.to_string_lossy().to_string()],
        None,
    );
    println!("resolved = {:?}", resolved);
    let exe_c = cand(
        "便携版",
        exe_path.to_string_lossy().as_ref(),
        exe_path.to_string_lossy().as_ref(),
        "",
        "app",
    );
    println!(
        "exe keys  = {:?}",
        crate::commands::apps::candidate_identity_keys(&exe_c)
    );
    // 库行身份（模拟 compute 的单行逻辑）
    let r = resolved.get(lnk_path.to_string_lossy().as_ref()).unwrap();
    let lib_key = crate::commands::apps::entry_identity_key("exe", &r.target, &r.args);
    println!("lib key   = {}", lib_key);
    fs::remove_dir_all(&root).ok();
}

// ============ Batch 2.4：AppID 结构分类（Spike 门1 实证案例）============

#[test]
fn t_uw01_AppID五桶分类_不得仅凭叹号判断() {
    use crate::commands::apps::classify_appid;
    // 标准 AUMID（本机真机样本）
    assert_eq!(
        classify_appid("OpenAI.Codex_2p2nqsd0c76g0!App"),
        Some("uwp")
    );
    // 纯 PFN（MSIX 桌面应用，无 !——门1 实证：不得漏判）
    assert_eq!(classify_appid("com.zhipuai.autoclaw2"), Some("msix"));
    assert_eq!(classify_appid("Tencent.CodeBuddyCN"), Some("msix"));
    // 完整路径（lnk/exe 通道已覆盖，不纳入）
    assert_eq!(classify_appid("D:\\综合性工具\\Bettbox\\Bettbox.exe"), None);
    // GUID 路径（不可证明可激活，未支持类型）
    assert_eq!(
        classify_appid("{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}\\appverif.exe"),
        None
    );
    // AutoGenerated（Spike 实证的第 5 桶，未支持类型）
    assert_eq!(
        classify_appid("Microsoft.AutoGenerated.{96690F80-0B7F-841E-E0DB-CC38AC5C839F}"),
        None
    );
    assert_eq!(classify_appid(""), None);
}

// ============ Batch 2.4：扫描并发与取消回归 ============

#[test]
fn t_sc01_扫描互斥_同时仅一个任务_释放后可再获取() {
    use crate::commands::apps::ScanGuard;
    // 获取成功
    let g1 = ScanGuard::acquire();
    assert!(g1.is_ok(), "空闲时应可获取");
    // 第二个任务必须被拒（D2：防止 B 复位 A 的取消旗标 / 双写结果）
    let g2 = ScanGuard::acquire();
    assert!(g2.is_err(), "并发第二任务必须被拒绝");
    drop(g1);
    // 释放后可再获取（取消后允许重新扫描）
    let g3 = ScanGuard::acquire();
    assert!(g3.is_ok(), "释放后必须可重新获取");
    drop(g3);
}

#[test]
fn t_sc02_取消即杀_PS子进程_不等到超时() {
    use crate::commands::apps::{wait_child_with_timeout, SCAN_CANCELLED};
    use std::sync::atomic::Ordering;
    use std::time::{Duration, Instant};
    // 启动一个 30s 的 PS 进程模拟卡住的解析批
    let mut child = std::process::Command::new("powershell")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "Start-Sleep -Seconds 30",
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("spawn");
    // 100ms 后置取消旗标（模拟用户在解析中点取消）
    let flag = std::thread::spawn(|| {
        std::thread::sleep(Duration::from_millis(100));
        SCAN_CANCELLED.store(true, Ordering::Relaxed);
    });
    let t0 = Instant::now();
    let completed = wait_child_with_timeout(&mut child, Duration::from_secs(60));
    let elapsed = t0.elapsed();
    flag.join().unwrap();
    SCAN_CANCELLED.store(false, Ordering::Relaxed);
    assert!(!completed, "被取消的批必须返回 false（非自然完成）");
    assert!(
        elapsed < Duration::from_secs(5),
        "取消必须即刻杀掉 PS 子进程（D4），实际耗时 {:?}",
        elapsed
    );
    assert!(child.try_wait().unwrap().is_some(), "子进程必须已退出");
}

#[test]
#[ignore = "诊断：PATH 15.4s 离群分阶段定位"]
fn t_diag_path_phase() {
    use crate::commands::apps::{build_candidates, scan_path_raw};
    use std::time::Instant;
    for round in 1..=3 {
        let n;
        let t0 = Instant::now();
        {
            let raw = scan_path_raw();
            n = raw.len();
            let t_enum = t0.elapsed();
            let t1 = Instant::now();
            let out = build_candidates(raw);
            let t_icons = t1.elapsed();
            println!(
                "第{}轮: 枚举={:?} ({}条)  分类+图标={:?} ({}条)",
                round,
                t_enum,
                n,
                t_icons,
                out.len()
            );
        }
    }
}

// ============ Batch 2.4 修正：打包应用身份验证（fail-closed）============

#[test]
fn t_uw02_打包应用验证_通过才识别_未验证降级() {
    use crate::commands::apps::{apply_uwp_verification, ScannedApp, UwpVerification};
    let mut results = vec![
        ScannedApp {
            name: "计算器".into(),
            path: "Microsoft.WindowsCalculator_8wekyb3d8bbwe!App".into(),
            icon_path: String::new(),
            args: String::new(),
            source: "Store/应用".into(),
            app_type: "uwp".into(),
            app_subtype: String::new(),
            is_launchable: true,
            target: "Microsoft.WindowsCalculator_8wekyb3d8bbwe!App".into(),
            target_args: String::new(),
            workdir: String::new(),
            status: "identified".into(),
            already_imported: false,
            in_trash: false,
            trash_id: None,
        },
        ScannedApp {
            name: "消失的包".into(),
            path: "Gone.Package_deadbeef!App".into(),
            icon_path: String::new(),
            args: String::new(),
            source: "Store/应用".into(),
            app_type: "uwp".into(),
            app_subtype: String::new(),
            is_launchable: true,
            target: "Gone.Package_deadbeef!App".into(),
            target_args: String::new(),
            workdir: String::new(),
            status: "identified".into(),
            already_imported: false,
            in_trash: false,
            trash_id: None,
        },
        ScannedApp {
            name: "普通软件".into(),
            path: r"C:\\lnk\\x.lnk".into(),
            icon_path: String::new(),
            args: String::new(),
            source: "开始菜单".into(),
            app_type: "app".into(),
            app_subtype: String::new(),
            is_launchable: true,
            target: r"C:\\Apps\\x.exe".into(),
            target_args: String::new(),
            workdir: String::new(),
            status: "identified".into(),
            already_imported: false,
            in_trash: false,
            trash_id: None,
        },
    ];
    let verified = vec![
        UwpVerification {
            app_id: "Microsoft.WindowsCalculator_8wekyb3d8bbwe!App".into(),
            verified: true,
            reason: String::new(),
        },
        UwpVerification {
            app_id: "Gone.Package_deadbeef!App".into(),
            verified: false,
            reason: "package-not-found".into(),
        },
    ];
    apply_uwp_verification(&mut results, &verified);
    assert_eq!(results[0].status, "identified", "验证通过的打包应用保持已识别");
    assert!(results[0].is_launchable);
    assert_eq!(results[1].status, "needs_confirmation", "未验证身份必须降级需要确认");
    assert!(!results[1].is_launchable, "未验证不得标记为可启动");
    // 非 uwp 条目不受影响
    assert_eq!(results[2].status, "identified");
    assert!(results[2].is_launchable);
}

#[test]
fn t_uw03_验证失败_全量fail_closed() {
    // verify_uwp_identities 整体失败（PS 无输出等）→ 命令层把全部 uwp 降级；
    // 这里验证 apply 在空验证表下的行为等价于全量降级
    use crate::commands::apps::{apply_uwp_verification, ScannedApp};
    let mut results = vec![ScannedApp {
        name: "任意打包应用".into(),
        path: "Some.Package_abc!App".into(),
        icon_path: String::new(),
        args: String::new(),
        source: "Store/应用".into(),
        app_type: "uwp".into(),
        app_subtype: String::new(),
        is_launchable: true,
        target: "Some.Package_abc!App".into(),
        target_args: String::new(),
        workdir: String::new(),
        status: "identified".into(),
        already_imported: false,
        in_trash: false,
        trash_id: None,
    }];
    apply_uwp_verification(&mut results, &[]);
    assert_eq!(results[0].status, "needs_confirmation", "空验证表 = 全量降级（fail-closed）");
    assert!(!results[0].is_launchable);
}

#[test]
fn t_uw04_名称相同_跨来源身份不同_不合并() {
    // 验收指令：不得以名称相同为依据跨来源自动合并
    use crate::commands::apps::{candidate_identity_keys, ScannedApp};
    let mk = |app_type: &str, path: &str, target: &str| ScannedApp {
        name: "同名软件".into(),
        path: path.into(),
        icon_path: String::new(),
        args: String::new(),
        source: "test".into(),
        app_type: app_type.into(),
        app_subtype: String::new(),
        is_launchable: true,
        target: target.into(),
        target_args: String::new(),
        workdir: String::new(),
        status: "identified".into(),
        already_imported: false,
        in_trash: false,
        trash_id: None,
    };
    let uwp = mk("uwp", "Same.Name_pkg!App", "Same.Name_pkg!App");
    let exe = mk("app", r"C:\\lnk\\s.lnk", r"C:\\Apps\\Same.Name.exe");
    let ku = candidate_identity_keys(&uwp);
    let ke = candidate_identity_keys(&exe);
    assert!(
        !ku.iter().any(|k| ke.contains(k)),
        "名称相同的 uwp 与 exe 入口身份不得相交：{:?} vs {:?}",
        ku,
        ke
    );
}

// ============ P1-B（v0.4.3 验收）：回收站命中 → 标记待恢复 + 带出 trash_id ============

#[test]
fn t_si_p1b_回收站命中_标记待恢复并带出id() {
    use crate::commands::apps::{
        apply_library_hits, candidate_identity_keys, entry_identity_key,
    };
    use std::collections::HashMap;

    // 候选：一个直连 exe 入口（主键 = exe 目标 + 参数）
    let mut c = cand("被删软件", r"C:\Apps\Gone\Gone.exe", r"C:\Apps\Gone\Gone.exe", "", "app");
    let main_key = candidate_identity_keys(&c)[0].clone();
    let mut lib: HashMap<String, Vec<(i64, Option<i64>)>> = HashMap::new();

    // ① 只有回收站条目 → in_trash + trash_id
    lib.insert(main_key.clone(), vec![(42, Some(1_700_000_000))]);
    apply_library_hits(std::slice::from_mut(&mut c), &lib);
    assert!(c.in_trash, "回收站命中必须标记 in_trash");
    assert_eq!(c.trash_id, Some(42), "必须带出回收站条目 id 供显式恢复");
    assert!(!c.already_imported, "不能同时算已在库");

    // ② 同身份多条回收站记录 → 取 id 最大的（最近收录的那条）
    c.in_trash = false;
    c.trash_id = None;
    lib.insert(main_key.clone(), vec![(42, Some(1)), (77, Some(2))]);
    apply_library_hits(std::slice::from_mut(&mut c), &lib);
    assert_eq!(c.trash_id, Some(77));

    // ③ 同身份存在有效条目 + 回收站条目 → 视为已在库，不提示恢复（避免恢复后重复）
    c.in_trash = false;
    c.trash_id = Some(42);
    lib.insert(main_key.clone(), vec![(42, Some(1)), (88, None)]);
    apply_library_hits(std::slice::from_mut(&mut c), &lib);
    assert!(c.already_imported, "有效条目优先");
    assert!(!c.in_trash, "有效存在时不得提示恢复");
    assert_eq!(c.trash_id, None, "已在库时不应暴露 trash_id");

    // ④ 无命中 → 两个标记都为 false
    let mut fresh = cand("新软件", r"C:\Apps\New\New.exe", r"C:\Apps\New\New.exe", "", "app");
    let empty: HashMap<String, Vec<(i64, Option<i64>)>> = HashMap::new();
    apply_library_hits(std::slice::from_mut(&mut fresh), &empty);
    assert!(!fresh.already_imported && !fresh.in_trash && fresh.trash_id.is_none());

    // ⑤ 双层键兜底：只有 lnk 次键命中，也要能识别回收站条目
    let mut lnk = cand("快捷方式", r"C:\Users\u\Desktop\x.lnk", "", "", "app");
    let lnk_key = entry_identity_key("lnk", r"C:\Users\u\Desktop\x.lnk", "");
    let mut lib2: HashMap<String, Vec<(i64, Option<i64>)>> = HashMap::new();
    lib2.insert(lnk_key, vec![(9, Some(5))]);
    apply_library_hits(std::slice::from_mut(&mut lnk), &lib2);
    assert_eq!(lnk.trash_id, Some(9));
    assert!(lnk.in_trash);
}

#[test]
fn t_si_p1b_回收站恢复端到端_分类与使用记录保留_再次扫描不重复() {
    use crate::commands::apps::{
        apply_library_hits, compute_library_identities,
    };
    use crate::db::Db;

    let dir = std::env::temp_dir().join(format!("drawer-p1b-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let db = Db::open(&dir.join("p1b.db")).expect("open db");
    let cat = db.create_app_category("工作", "💼").unwrap();
    let exe = r"C:\Apps\Restore Me\restore.exe";
    let id = db
        .create_app("待恢复软件", exe, "", "", Some(cat), "app", "")
        .unwrap();
    db.record_app_usage(id).unwrap();
    db.record_app_usage(id).unwrap();
    db.soft_delete("apps", id).unwrap();

    // ① 扫描：同身份命中回收站 → 待恢复 + trash_id
    let mut c = cand("待恢复软件", exe, exe, "", "app");
    let lib = compute_library_identities(&db);
    apply_library_hits(std::slice::from_mut(&mut c), &lib);
    assert!(c.in_trash && !c.already_imported);
    assert_eq!(c.trash_id, Some(id));

    // ② 用户显式恢复：复用同一行（不新建），分类/使用次数保留
    assert_eq!(db.restore("apps", c.trash_id.unwrap()).unwrap(), 1);
    let listed = db.list_apps("", None).unwrap();
    assert_eq!(listed.len(), 1, "恢复必须复用原记录，不得产生重复条目");
    assert_eq!(listed[0].id, id);
    assert_eq!(listed[0].category_id, Some(cat), "分类必须保留");
    assert_eq!(listed[0].use_count, 2, "使用次数必须保留");

    // ③ 再次扫描：转为「已在库」，不再提示恢复、不再计入回收站
    let mut c2 = cand("待恢复软件", exe, exe, "", "app");
    let lib2 = compute_library_identities(&db);
    apply_library_hits(std::slice::from_mut(&mut c2), &lib2);
    assert!(c2.already_imported);
    assert!(!c2.in_trash);
    assert_eq!(c2.trash_id, None);

    // ④ 导入同一候选不得新增（幂等）
    let summary = crate::commands::apps::import_candidates_core(&db, &[c2.clone()], None);
    assert_eq!(summary.added, 0, "已在库的同身份候选不得重复入库");
    assert_eq!(summary.already_exists, 1);

    drop(db);
    std::fs::remove_dir_all(&dir).ok();
}

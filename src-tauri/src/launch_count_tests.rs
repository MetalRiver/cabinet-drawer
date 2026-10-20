//! v0.4.3 Batch 1-C：软件启动计数口径回归。
//! 旧缺陷：前端在调用 launch 前先 record_app_usage，无论启动成败都 +1
//! （文件缺失 / 重定位后仍失败 / ShellExecuteW 报错全部被计为"已使用"）。
//! 新契约：仅 launch_app_inner 的 Ok 出口（启动请求被系统受理）才计数。
//!
//! 口径说明（按指令要求区分）：本测试钉死"失败不计数"；"受理成功 → 计数"
//! 的正向路径需要真实进程启动，属 Windows 实机验收项，不在单测中伪造。

use crate::commands::apps::launch_app_inner;
use crate::db::Db;
use crate::{AppState, StartupMode};
use std::fs;
use std::path::PathBuf;

fn temp_db(label: &str) -> (PathBuf, Db) {
    let dir = std::env::temp_dir().join(format!(
        "drawer-launchcount-{}-{}",
        label,
        uuid::Uuid::new_v4()
    ));
    fs::create_dir_all(&dir).expect("建临时目录失败");
    let path = dir.join("drawer-v2.db");
    let db = Db::open(&path).expect("建测试库失败");
    (dir, db)
}

fn create_app_row(db: &Db, name: &str, path: &str) -> i64 {
    db.create_app(name, path, "", "", None, "app", "utility")
        .expect("create_app 失败")
}

fn use_count(db: &Db, id: i64) -> i64 {
    let conn = db.conn.lock().unwrap();
    conn.query_row(
        "SELECT use_count FROM apps WHERE id=?1",
        rusqlite::params![id],
        |r| r.get(0),
    )
    .unwrap()
}

#[test]
fn t_lc01_路径失效_启动失败不计数() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    rt.block_on(async {
        let (dir, db) = temp_db("lc01");
        let db_path = dir.join("drawer-v2.db");
        // 名字用乱串，确保 try_relocate_app 在开始菜单/桌面扫描中不可能撞名
        let id = create_app_row(
            &db,
            "zz-launchcount-missing-7f3d",
            "C:\\definitely\\not\\exist\\zz-7f3d.exe",
        );
        let state = AppState::new(db, db_path, StartupMode::ExistingV2);
        let r = launch_app_inner(&state, id).await;
        assert!(r.is_err(), "不存在的路径必须报错，实际 {:?}", r);
        assert_eq!(
            use_count(&state.db.lock().unwrap(), id),
            0,
            "启动失败不得计入使用次数（旧实现此处为 1）"
        );
    });
}

#[test]
fn t_lc02_路径存在但非可执行_启动失败不计数() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    rt.block_on(async {
        let (dir, db) = temp_db("lc02");
        let db_path = dir.join("drawer-v2.db");
        // 伪造 .exe（内容非 PE）→ ShellExecuteW 以 ERROR_BAD_EXE_FORMAT 失败。
        // 注：不能用无关联扩展名（.zzz）——实测 ShellExecuteW 对无关联文件
        // 弹"选择打开方式"并**返回成功码**，恰好实证"受理成功 ≠ 应用运行"
        // 的口径边界，此类场景计入成功是当前口径的已知边界（见交付报告）。
        let fixture = dir.join("zz-launchcount-badexe-7f3d.exe");
        fs::write(&fixture, b"definitely not a PE image").expect("写 fixture 失败");
        let id = create_app_row(
            &db,
            "zz-launchcount-badexe-7f3d",
            fixture.to_string_lossy().as_ref(),
        );
        let state = AppState::new(db, db_path, StartupMode::ExistingV2);
        let r = launch_app_inner(&state, id).await;
        assert!(r.is_err(), "损坏 exe 必须报错，实际 {:?}", r);
        assert_eq!(
            use_count(&state.db.lock().unwrap(), id),
            0,
            "ShellExecuteW 失败不得计入使用次数"
        );
    });
}

#[test]
fn t_lc03_重定位后仍失败_不计数() {
    // relocate 分支（relocated-but-still-fail）同样不得计数：
    // 乱串名保证 try_relocate_app 无命中，走"找不到文件"失败出口（与 lc01 同出口）
    let rt = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    rt.block_on(async {
        let (dir, db) = temp_db("lc03");
        let db_path = dir.join("drawer-v2.db");
        let id = create_app_row(
            &db,
            "zz-launchcount-reloc-7f3d",
            "D:\\drawer-test-nonexistent\\zz-7f3d.exe",
        );
        let state = AppState::new(db, db_path, StartupMode::ExistingV2);
        let r = launch_app_inner(&state, id).await;
        assert!(r.is_err());
        assert_eq!(use_count(&state.db.lock().unwrap(), id), 0);
    });
}

#[test]
fn t_lc04_record_app_usage_基线语义_计数只增_调用方负责时机() {
    // 底层原语回归：record_app_usage 本身无条件 +1 —— 时机正确性由
    // launch_app_inner 的 Ok 出口调用保证（代码审查 + 实机验收覆盖）
    let (dir, db) = temp_db("lc04");
    let id = create_app_row(&db, "baseline", "C:\\x\\y.exe");
    assert_eq!(use_count(&db, id), 0);
    db.record_app_usage(id).unwrap();
    assert_eq!(use_count(&db, id), 1);
    drop(dir);
}

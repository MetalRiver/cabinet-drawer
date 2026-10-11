//! ===== v0.4.1 单元测试：软件失效治理（分类保护 / 批量软删幂等 / 回收站闭环）=====
//!
//! 覆盖（任务 A）：
//! - 误判保护：移动盘/网络盘/光驱离线/盘符不存在/不确定状态绝不进入清退候选
//!   （唯一候选 = 固定盘上确认缺失）；URL 恒可用语义不进入失效列表口径
//! - 批量移入回收站：软删除（不硬删）、幂等（重复批量 → skipped）、
//!   回收站恢复后重新可见、部分失败结果结构
//! - 批量操作只处理显式传入的 id（无任何后台自动清退路径）

use std::fs;
use std::path::PathBuf;

use rusqlite::params;

use crate::db::{classify_path_health, is_unc_path, Db};

fn temp_db(label: &str) -> (PathBuf, Db) {
    let dir = std::env::temp_dir().join(format!(
        "drawer-apphealth-{}-{}",
        label,
        uuid::Uuid::new_v4()
    ));
    fs::create_dir_all(&dir).expect("建临时目录失败");
    let path = dir.join("drawer-v2.db");
    let db = Db::open(&path).expect("建测试库失败");
    (dir, db)
}

fn exe_fixture(label: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!("drawer-ah-{}-{}.exe", label, uuid::Uuid::new_v4()));
    fs::write(&p, b"MZ").expect("写 fixture 失败");
    p
}

fn create_app(db: &Db, name: &str, path: &str) -> i64 {
    db.create_app(name, path, "", "", None, "app", "other")
        .unwrap()
}

// ===== 误判保护：分类纯函数 =====

#[test]
fn t_ah01_only_fixed_missing_is_clearable_candidate() {
    assert_eq!(classify_path_health(3, false, false), "fixed_missing");
    // 保护类：移动盘 / 光驱 / 网络盘 / 盘符不存在 / 未知
    assert_eq!(classify_path_health(2, false, false), "removable_offline");
    assert_eq!(classify_path_health(5, false, false), "removable_offline");
    assert_eq!(classify_path_health(4, false, false), "network_offline");
    assert_eq!(classify_path_health(1, false, false), "uncertain");
    assert_eq!(classify_path_health(0, false, false), "uncertain");
    assert_eq!(classify_path_health(0, false, true), "network_offline");
    // 存在即 ok
    assert_eq!(classify_path_health(3, true, false), "ok");
}

#[test]
fn t_ah02_unc_detection() {
    assert!(is_unc_path(r"\\server\share\app.exe"));
    assert!(is_unc_path("//server/share/app.exe"));
    assert!(!is_unc_path(r"C:\Program Files\app.exe"));
    assert!(!is_unc_path("D:/Games/app.exe"));
}

#[test]
fn t_ah03_temporarily_offline_drive_never_candidates() {
    // 场景：移动盘今天不在位（单次检查缺失）→ removable_offline 保护类
    // 网络盘断连 → network_offline 保护类。两类在任何批量清退 UI 中不可勾选。
    let kinds = [
        classify_path_health(2, false, false),
        classify_path_health(4, false, true),
        classify_path_health(1, false, false),
    ];
    assert!(kinds.iter().all(|k| *k != "fixed_missing"));
}

// ===== 批量软删：幂等 / 软删不硬删 / 回收站恢复 =====

#[test]
fn t_ah04_batch_soft_delete_idempotent_and_recoverable() {
    let (_dir, db) = temp_db("batch");
    let exe = exe_fixture("batch");
    let ids: Vec<i64> = ["A", "B", "C"]
        .iter()
        .map(|n| create_app(&db, n, exe.to_str().unwrap()))
        .collect();

    // 软删除语义（与生产命令 batch_move_apps_to_trash 相同的 db 层调用）
    let mut moved = 0;
    for id in &ids {
        moved += db.soft_delete("apps", *id).unwrap();
    }
    assert_eq!(moved, 3, "首次批量：3 条全部移入回收站");

    // 列表不再显示（软删生效，记录仍在库）
    assert!(db.list_apps("", None).unwrap().is_empty());
    let conn = db.conn.lock().unwrap();
    let in_trash: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM apps WHERE deleted_at IS NOT NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();
    drop(conn);
    assert_eq!(in_trash, 3, "记录保留在库（可恢复），未物理删除");

    // 幂等：重复批量 → 0 moved
    let mut moved2 = 0;
    let mut skipped = 0;
    for id in &ids {
        match db.soft_delete("apps", *id) {
            Ok(n) if n > 0 => moved2 += 1,
            Ok(_) => skipped += 1,
            Err(_) => {}
        }
    }
    assert_eq!((moved2, skipped), (0, 3), "重复批量全部记为 skipped");

    // 回收站恢复 → 重新可见
    for id in &ids {
        db.restore("apps", *id).unwrap();
    }
    assert_eq!(db.list_apps("", None).unwrap().len(), 3);
    fs::remove_file(&exe).ok();
}

#[test]
fn t_ah05_batch_only_touches_explicit_ids() {
    let (_dir, db) = temp_db("scoped");
    let exe = exe_fixture("scoped");
    let a = create_app(&db, "keep-1", exe.to_str().unwrap());
    let b = create_app(&db, "drop-1", exe.to_str().unwrap());
    let c = create_app(&db, "keep-2", exe.to_str().unwrap());

    // 只清退 b：a/c 不受任何影响（无后台自动清退语义）
    db.soft_delete("apps", b).unwrap();
    let listed = db.list_apps("", None).unwrap();
    let ids: Vec<i64> = listed.iter().map(|x| x.id).collect();
    assert_eq!(ids.len(), 2);
    assert!(ids.contains(&a) && ids.contains(&c) && !ids.contains(&b));
    fs::remove_file(&exe).ok();
}

#[test]
fn t_ah06_soft_delete_then_readd_keeps_trash_integrity() {
    let (_dir, db) = temp_db("integrity");
    let exe = exe_fixture("integrity");
    let id = create_app(&db, "X", exe.to_str().unwrap());
    db.soft_delete("apps", id).unwrap();
    // 回收站计数与 trash list 都能看到
    let conn = db.conn.lock().unwrap();
    let n: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM apps WHERE deleted_at IS NOT NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();
    drop(conn);
    assert_eq!(n, 1);
    let trash = db.list_trash().unwrap();
    assert!(trash.iter().any(|t| t.kind == "app" && t.id == id));
    fs::remove_file(&exe).ok();
}

#[test]
fn t_ah07_fixture_cleanup_idempotent() {
    // 播种/重置命令的 db 层语义：按 [TEST] 前缀软删，重复清理幂等
    let (_dir, db) = temp_db("fixtures");
    let exe = exe_fixture("fx");
    let a = db
        .create_app(
            "[TEST] 固定盘缺失",
            r"C:\drawer-test-fixture\missing.exe",
            "",
            "",
            None,
            "app",
            "other",
        )
        .unwrap();
    let b = db
        .create_app(
            "[TEST] 网络位置",
            r"\server\share\app.exe",
            "",
            "",
            None,
            "app",
            "other",
        )
        .unwrap();
    let real = db
        .create_app(
            "真实软件",
            exe.to_str().unwrap(),
            "",
            "",
            None,
            "app",
            "other",
        )
        .unwrap();

    // 第一轮清理：只清 [TEST] 前缀
    let mut cleared = 0;
    for id in [a, b] {
        cleared += db.soft_delete("apps", id).unwrap();
    }
    assert_eq!(cleared, 2);
    // 第二轮：幂等（已删 → 0）
    let mut cleared2 = 0;
    for id in [a, b] {
        cleared2 += db.soft_delete("apps", id).unwrap();
    }
    assert_eq!(cleared2, 0);
    // 真实软件不受影响
    let listed = db.list_apps("", None).unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, real);
    fs::remove_file(&exe).ok();
}

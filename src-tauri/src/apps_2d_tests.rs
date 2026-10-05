//! ===== Phase 2D 单元测试：应用生命周期失效治理 =====
//!
//! 覆盖（v0.4.0 Release Train §2 对应可自动化子集）：
//! - AppMeta.available 运行时派生：存在=可用 / 不存在=不可用 / url 恒可用 / folder 按目录
//! - fs 检查失败（路径非法字符）不得 panic、不得影响 DB 加载（fail safe → 不可用）
//! - 路径失效 ≠ 删除：记录仍在 DB，available=false 仅是状态
//! - relink（update_app_path）后 available 恢复 true，且其他元数据不动
//! - delete_app（软删除）后 list_apps 不再返回

use std::fs;
use std::path::PathBuf;

use crate::db::{app_path_available, Db};

fn temp_db(label: &str) -> (PathBuf, Db) {
    let dir = std::env::temp_dir().join(format!("drawer-2d-{}-{}", label, uuid::Uuid::new_v4()));
    fs::create_dir_all(&dir).expect("建临时目录失败");
    let path = dir.join("drawer-v2.db");
    let db = Db::open(&path).expect("建测试库失败");
    (dir, db)
}

fn exe_fixture(label: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!(
        "drawer-2d-exe-{}-{}.exe",
        label,
        uuid::Uuid::new_v4()
    ));
    fs::write(&p, b"MZ").expect("写 fixture 失败");
    p
}

#[test]
fn t2d01_available_true_when_path_exists() {
    let exe = exe_fixture("ok");
    assert!(app_path_available("app", exe.to_str().unwrap()));
    fs::remove_file(&exe).ok();
}

#[test]
fn t2d02_available_false_when_path_missing() {
    let missing = std::env::temp_dir().join(format!("drawer-2d-gone-{}.exe", uuid::Uuid::new_v4()));
    assert!(!app_path_available("app", missing.to_str().unwrap()));
}

#[test]
fn t2d03_url_always_available() {
    assert!(app_path_available("url", "https://example.com"));
    assert!(app_path_available("url", ""));
}

#[test]
fn t2d04_folder_needs_existing_dir() {
    let dir = std::env::temp_dir().join(format!("drawer-2d-dir-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&dir).unwrap();
    assert!(app_path_available("folder", dir.to_str().unwrap()));
    fs::remove_dir(&dir).ok();
    assert!(!app_path_available("folder", dir.to_str().unwrap()));
}

#[test]
fn t2d05_invalid_path_fails_safe_unavailable_not_panic() {
    // Windows 非法路径字符：exists() 返回 None→false，绝不 panic
    assert!(!app_path_available("app", "C:\0bad<?|*path"));
    assert!(!app_path_available("app", ""));
}

#[test]
fn t2d06_list_apps_derives_availability_without_deleting() {
    let (_dir, db) = temp_db("list");
    let live = exe_fixture("live");
    let gone = std::env::temp_dir().join(format!("drawer-2d-gone2-{}.exe", uuid::Uuid::new_v4()));
    let id_live = db
        .create_app(
            "Live App",
            live.to_str().unwrap(),
            "",
            "",
            None,
            "app",
            "other",
        )
        .unwrap();
    let id_gone = db
        .create_app(
            "Gone App",
            gone.to_str().unwrap(),
            "",
            "",
            None,
            "app",
            "other",
        )
        .unwrap();

    let rows = db.list_apps("", None).unwrap();
    assert_eq!(rows.len(), 2, "路径失效≠删除：两条记录都必须还在");
    let live_row = rows.iter().find(|r| r.id == id_live).unwrap();
    let gone_row = rows.iter().find(|r| r.id == id_gone).unwrap();
    assert!(live_row.available, "存在的 exe → available=true");
    assert!(
        !gone_row.available,
        "失效 exe → available=false，但记录保留"
    );
    fs::remove_file(&live).ok();
}

#[test]
fn t2d07_relink_restores_availability_keeps_metadata() {
    let (_dir, db) = temp_db("relink");
    let old = exe_fixture("old");
    let id = db
        .create_app(
            "Relink App",
            old.to_str().unwrap(),
            "",
            "",
            None,
            "app",
            "other",
        )
        .unwrap();
    fs::remove_file(&old).ok();
    assert!(
        !db.list_apps("", None)
            .unwrap()
            .iter()
            .find(|r| r.id == id)
            .unwrap()
            .available
    );

    let new = exe_fixture("new");
    db.update_app_path(id, new.to_str().unwrap()).unwrap();
    let row = db
        .list_apps("", None)
        .unwrap()
        .into_iter()
        .find(|r| r.id == id)
        .unwrap();
    assert!(row.available, "relink 后恢复可用");
    assert_eq!(
        row.name, "Relink App",
        "relink 保留原 Drawer 元数据（名称）"
    );
    assert_eq!(
        row.created_at,
        db.list_apps("", None).unwrap()[0].created_at
    );
    fs::remove_file(&new).ok();
}

#[test]
fn t2d08_soft_deleted_app_not_listed() {
    let (_dir, db) = temp_db("softdel");
    let exe = exe_fixture("del");
    let id = db
        .create_app(
            "To Remove",
            exe.to_str().unwrap(),
            "",
            "",
            None,
            "app",
            "other",
        )
        .unwrap();
    db.delete_app(id).unwrap();
    let rows = db.list_apps("", None).unwrap();
    assert!(
        rows.iter().all(|r| r.id != id),
        "从抽屉柜移除后不再出现在列表"
    );
    fs::remove_file(&exe).ok();
}

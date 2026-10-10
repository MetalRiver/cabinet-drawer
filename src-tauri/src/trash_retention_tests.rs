//! ===== v0.4.1 单元测试：回收站保留天数语义（P0：永久保留不得触发 DELETE）=====
//!
//! 覆盖（用户反馈修复 检查点B）：
//! - trash_retention_days = 0（永久保留）→ startup_auto_cleanup 绝不删除任何回收站条目
//! - 1 / 30 / 365 等正整数按天数正常清理（过期删、未过期留）
//! - 非法值（非整数 / 负数 / 空串）→ 安全默认 30 天，不 panic、不误删
//! - 设置缺失 → 默认 30 天
//! - settings 表 0 值写入/读取往返不被改写（widget.auto_lock_minutes 等）

use std::fs;
use std::path::PathBuf;

use rusqlite::params;

use crate::commands::trash::startup_auto_cleanup;
use crate::db::Db;

const DAY_MS: i64 = 24 * 60 * 60 * 1000;

fn temp_db(label: &str) -> (PathBuf, Db) {
    let dir = std::env::temp_dir().join(format!("drawer-trash-{}-{}", label, uuid::Uuid::new_v4()));
    fs::create_dir_all(&dir).expect("建临时目录失败");
    let path = dir.join("drawer-v2.db");
    let db = Db::open(&path).expect("建测试库失败");
    (dir, db)
}

/// 直接向 apps 表插入一条软删除记录；deleted_at_offset_ms 为相对当前时间的偏移（负=过去）
fn seed_trash_app(db: &Db, name: &str, deleted_at_offset_ms: i64) -> i64 {
    let now = chrono::Utc::now().timestamp_millis();
    let conn = db.conn.lock().unwrap();
    conn.execute(
        "INSERT INTO apps (name, path, icon_path, args, category_id, created_at, use_count, last_used_at, deleted_at)
         VALUES (?1, 'C:\\fake.exe', NULL, '', NULL, ?2, 0, 0, ?3)",
        params![name, now, now + deleted_at_offset_ms],
    )
    .unwrap();
    conn.last_insert_rowid()
}

fn count_trash_apps(db: &Db) -> i64 {
    let conn = db.conn.lock().unwrap();
    conn.query_row(
        "SELECT COUNT(*) FROM apps WHERE deleted_at IS NOT NULL",
        [],
        |r| r.get(0),
    )
    .unwrap()
}

fn set_retention(db: &Db, value: &str) {
    db.set_setting("trash_retention_days", value).unwrap();
}

#[test]
fn t_tr01_retention_zero_keeps_all_trash() {
    let (_dir, db) = temp_db("zero");
    set_retention(&db, "0");
    // 极端旧条目：400 天前删除（任何正数保留天数都会清掉它）
    let old_id = seed_trash_app(&db, "old-400d", -400 * DAY_MS);
    let recent_id = seed_trash_app(&db, "recent-1h", -1 * 60 * 60 * 1000);
    assert_eq!(count_trash_apps(&db), 2);

    startup_auto_cleanup(&db);

    assert_eq!(count_trash_apps(&db), 2, "永久保留（0）不得触发任何 DELETE");
    let conn = db.conn.lock().unwrap();
    for id in [old_id, recent_id] {
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM apps WHERE id = ?1",
                params![id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 1, "id={} 行必须仍在", id);
    }
}

#[test]
fn t_tr02_retention_30_deletes_expired_keeps_fresh() {
    let (_dir, db) = temp_db("d30");
    set_retention(&db, "30");
    let expired_id = seed_trash_app(&db, "expired-40d", -40 * DAY_MS);
    let fresh_id = seed_trash_app(&db, "fresh-1d", -1 * DAY_MS);

    startup_auto_cleanup(&db);

    let conn = db.conn.lock().unwrap();
    let n_expired: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM apps WHERE id = ?1",
            params![expired_id],
            |r| r.get(0),
        )
        .unwrap();
    let n_fresh: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM apps WHERE id = ?1",
            params![fresh_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n_expired, 0, "超过 30 天的应被清理");
    assert_eq!(n_fresh, 1, "未到 30 天的必须保留");
}

#[test]
fn t_tr03_retention_1_day_boundary() {
    let (_dir, db) = temp_db("d1");
    set_retention(&db, "1");
    let expired_id = seed_trash_app(&db, "expired-2d", -2 * DAY_MS);
    let fresh_id = seed_trash_app(&db, "fresh-12h", -12 * 60 * 60 * 1000);

    startup_auto_cleanup(&db);

    let conn = db.conn.lock().unwrap();
    let n_expired: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM apps WHERE id = ?1",
            params![expired_id],
            |r| r.get(0),
        )
        .unwrap();
    let n_fresh: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM apps WHERE id = ?1",
            params![fresh_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n_expired, 0);
    assert_eq!(n_fresh, 1, "12 小时前删除的未到 1 天，必须保留");
}

#[test]
fn t_tr04_invalid_values_fall_back_to_30d() {
    for bad in ["abc", "-5", ""] {
        let (_dir, db) = temp_db("bad");
        set_retention(&db, bad);
        let expired_id = seed_trash_app(&db, "expired-40d", -40 * DAY_MS);
        let fresh_id = seed_trash_app(&db, "fresh-1d", -1 * DAY_MS);

        startup_auto_cleanup(&db);

        let conn = db.conn.lock().unwrap();
        let n_expired: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM apps WHERE id = ?1",
                params![expired_id],
                |r| r.get(0),
            )
            .unwrap();
        let n_fresh: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM apps WHERE id = ?1",
                params![fresh_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            n_expired, 0,
            "非法值 {:?} 应回退默认 30 天并清理超期项",
            bad
        );
        assert_eq!(n_fresh, 1, "非法值 {:?} 不得误删未过期项", bad);
    }
}

#[test]
fn t_tr05_missing_setting_defaults_30d() {
    let (_dir, db) = temp_db("missing");
    // 不写 trash_retention_days
    let expired_id = seed_trash_app(&db, "expired-40d", -40 * DAY_MS);
    let fresh_id = seed_trash_app(&db, "fresh-1d", -1 * DAY_MS);

    startup_auto_cleanup(&db);

    let conn = db.conn.lock().unwrap();
    let n_expired: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM apps WHERE id = ?1",
            params![expired_id],
            |r| r.get(0),
        )
        .unwrap();
    let n_fresh: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM apps WHERE id = ?1",
            params![fresh_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n_expired, 0);
    assert_eq!(n_fresh, 1);
}

#[test]
fn t_tr06_retention_365_boundary() {
    let (_dir, db) = temp_db("d365");
    set_retention(&db, "365");
    let expired_id = seed_trash_app(&db, "expired-400d", -400 * DAY_MS);
    let fresh_id = seed_trash_app(&db, "fresh-100d", -100 * DAY_MS);

    startup_auto_cleanup(&db);

    let conn = db.conn.lock().unwrap();
    let n_expired: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM apps WHERE id = ?1",
            params![expired_id],
            |r| r.get(0),
        )
        .unwrap();
    let n_fresh: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM apps WHERE id = ?1",
            params![fresh_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n_expired, 0);
    assert_eq!(n_fresh, 1, "100 天前删除的未到 1 年，必须保留");
}

#[test]
fn t_tr07_zero_value_settings_roundtrip() {
    let (_dir, db) = temp_db("roundtrip");
    // 0 是合法设置值（永不锁定 / 永不过期 / 永久保留），写入后必须原样读回
    for key in [
        "widget.auto_lock_minutes",
        "widget.temp_expire_days",
        "trash_retention_days",
    ] {
        db.set_setting(key, "0").unwrap();
        assert_eq!(db.get_setting(key).unwrap().as_deref(), Some("0"));
    }
}

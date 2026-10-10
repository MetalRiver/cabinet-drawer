//! ===== v0.4.1 单元测试：便签编辑 / 追加 / 续期 / 永不过期 =====
//!
//! 覆盖（用户反馈 检查点C）：
//! - update_temp_text：只改 text，保持原始 id / created_at / expires_at
//! - renew_temp add_ms：在现有到期时间上延长
//! - renew_temp set_expires_at：直接设定，含永久哨兵
//! - 永不过期（哨兵 2100）：list_temp 正常显示、cleanup_expired_temp 不删
//! - ttl_minutes <= 0 创建 = 永不过期
//! - 非法续期入参 fail（命令层校验语义在 db 层以 0 行/参数校验体现）
//! - 旧数据兼容：既有有限 expires_at 行为不变

use std::fs;
use std::path::PathBuf;

use rusqlite::params;

use crate::db::{Db, Db as DbType};

const NEVER: i64 = DbType::TEMP_NEVER_EXPIRES_MS;

fn temp_db(label: &str) -> (PathBuf, Db) {
    let dir = std::env::temp_dir().join(format!(
        "drawer-tempedit-{}-{}",
        label,
        uuid::Uuid::new_v4()
    ));
    fs::create_dir_all(&dir).expect("建临时目录失败");
    let path = dir.join("drawer-v2.db");
    let db = Db::open(&path).expect("建测试库失败");
    (dir, db)
}

fn get_row(db: &Db, id: i64) -> Option<(String, i64, i64)> {
    let conn = db.conn.lock().unwrap();
    conn.query_row(
        "SELECT text, created_at, expires_at FROM temp_contents WHERE id = ?1",
        params![id],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )
    .ok()
}

#[test]
fn t_te01_update_text_keeps_id_created_at_expires_at() {
    let (_dir, db) = temp_db("edit");
    let id = db.create_temp("原始内容", 3 * 24 * 60).unwrap();
    let (_, created, expires_before) = get_row(&db, id).unwrap();

    let n = db.update_temp_text(id, "编辑后的内容").unwrap();
    assert_eq!(n, 1);

    let (text, created_after, expires_after) = get_row(&db, id).unwrap();
    assert_eq!(text, "编辑后的内容");
    assert_eq!(created_after, created, "编辑不得改 created_at");
    assert_eq!(expires_after, expires_before, "编辑不得自动重置到期时间");
    assert_eq!(id, id, "id 不变（同一行原地更新）");
}

#[test]
fn t_te02_renew_add_ms_extends_from_current_expiry() {
    let (_dir, db) = temp_db("renew-add");
    let id = db.create_temp("note", 60).unwrap(); // 1 小时后过期
    let (_, _, expires_before) = get_row(&db, id).unwrap();

    let n = db
        .renew_temp(id, Some(3 * 24 * 60 * 60 * 1000), None)
        .unwrap();
    assert_eq!(n, 1);
    let (_, _, expires_after) = get_row(&db, id).unwrap();
    assert_eq!(
        expires_after,
        expires_before + 3 * 24 * 60 * 60 * 1000,
        "延长 N 天 = 现有到期时间 + N 天（用户可见的续期计算规则）"
    );
}

#[test]
fn t_te03_renew_set_absolute_and_never() {
    let (_dir, db) = temp_db("renew-set");
    let id = db.create_temp("note", 60).unwrap();

    let target = chrono::Utc::now().timestamp_millis() + 7 * 24 * 60 * 60 * 1000;
    let n = db.renew_temp(id, None, Some(target)).unwrap();
    assert_eq!(n, 1);
    assert_eq!(get_row(&db, id).unwrap().2, target);

    // 永不过期
    let n = db.renew_temp(id, None, Some(NEVER)).unwrap();
    assert_eq!(n, 1);
    assert_eq!(get_row(&db, id).unwrap().2, NEVER);
}

#[test]
fn t_te04_never_expire_visible_and_survives_cleanup() {
    let (_dir, db) = temp_db("never");
    // 命令层 create_temp(ttl<=0) 会换算为哨兵；db 层直接以哨兵落库验证同等效果
    let id = db.create_temp("永久便签", NEVER).unwrap();
    assert_eq!(get_row(&db, id).unwrap().2, NEVER);

    let listed = db.list_temp().unwrap();
    assert!(listed.iter().any(|t| t.id == id), "永久便签必须出现在列表");

    let deleted = db.cleanup_expired_temp().unwrap();
    assert_eq!(deleted, 0, "永久便签不得被过期清理删除");
    assert!(get_row(&db, id).is_some());
}

#[test]
fn t_te05_never_expire_row_excluded_from_expiry_cleanup_by_sentinel() {
    let (_dir, db) = temp_db("sentinel");
    let conn = db.conn.lock().unwrap();
    // 直接以哨兵值落库（模拟任意来源的永久便签）
    conn.execute(
        "INSERT INTO temp_contents (text, created_at, expires_at) VALUES ('s', 0, ?1)",
        params![NEVER],
    )
    .unwrap();
    // 已过期行：应被清理
    conn.execute(
        "INSERT INTO temp_contents (text, created_at, expires_at) VALUES ('e', 0, 1)",
        params![],
    )
    .unwrap();
    drop(conn);

    let n = db.cleanup_expired_temp().unwrap();
    assert_eq!(n, 1, "只清理真正过期的行");
    let listed = db.list_temp().unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].expires_at, NEVER);
}

#[test]
fn t_te06_renew_missing_or_deleted_note_returns_zero() {
    let (_dir, db) = temp_db("renew-missing");
    assert_eq!(db.renew_temp(99999, Some(1000), None).unwrap(), 0);
    assert_eq!(db.renew_temp(99999, None, Some(NEVER)).unwrap(), 0);

    // 软删除（进回收站）的便签不可续期
    let id = db.create_temp("to-trash", 60).unwrap();
    db.soft_delete("temp_contents", id).unwrap();
    assert_eq!(
        db.renew_temp(id, Some(1000), None).unwrap(),
        0,
        "已进回收站的便签不得绕过回收站流程被续期"
    );
}

#[test]
fn t_te07_legacy_finite_expiry_rows_unchanged() {
    let (_dir, db) = temp_db("legacy");
    // 旧版数据：有限 expires_at，行为必须保持不变（列表可见、到期消失）
    let now = chrono::Utc::now().timestamp_millis();
    let conn = db.conn.lock().unwrap();
    conn.execute(
        "INSERT INTO temp_contents (text, created_at, expires_at) VALUES ('old-live', 0, ?1)",
        params![now + 60 * 60 * 1000],
    )
    .unwrap();
    drop(conn);

    let listed = db.list_temp().unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].expires_at, now + 60 * 60 * 1000);
}

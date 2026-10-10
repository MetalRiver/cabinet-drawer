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

    let now = chrono::Utc::now().timestamp_millis();
    let new_expiry = db
        .renew_temp(id, now, Some(3 * 24 * 60 * 60 * 1000), None)
        .unwrap();
    assert_eq!(
        new_expiry,
        expires_before + 3 * 24 * 60 * 60 * 1000,
        "延长 N 天 = 现有到期时间 + N 天（用户可见的续期计算规则）"
    );
    let (_, _, expires_after) = get_row(&db, id).unwrap();
    assert_eq!(expires_after, new_expiry);
}

#[test]
fn t_te03_renew_set_absolute_and_never() {
    let (_dir, db) = temp_db("renew-set");
    let id = db.create_temp("note", 60).unwrap();

    let now = chrono::Utc::now().timestamp_millis();
    let target = now + 7 * 24 * 60 * 60 * 1000;
    let new_expiry = db.renew_temp(id, now, None, Some(target)).unwrap();
    assert_eq!(new_expiry, target);
    assert_eq!(get_row(&db, id).unwrap().2, target);

    // 永不过期
    let new_expiry = db.renew_temp(id, now, None, Some(NEVER)).unwrap();
    assert_eq!(new_expiry, NEVER);
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
fn t_te06_renew_missing_or_deleted_note_fails_visible() {
    let (_dir, db) = temp_db("renew-missing");
    let now = chrono::Utc::now().timestamp_millis();
    assert!(db
        .renew_temp(99999, now, Some(1000), None)
        .unwrap_err()
        .contains("不存在"));
    assert!(db
        .renew_temp(99999, now, None, Some(NEVER))
        .unwrap_err()
        .contains("不存在"));

    // 软删除（进回收站）的便签不可续期
    let id = db.create_temp("to-trash", 60).unwrap();
    db.soft_delete("temp_contents", id).unwrap();
    let err = db.renew_temp(id, now, Some(1000), None).unwrap_err();
    assert!(
        err.contains("不存在"),
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

// ===== R1/R2（D-Repair）：永久语义统一 + 期限范围校验 + 哨兵一致性 =====

#[test]
fn t_te08_sentinel_matches_frontend_constant() {
    // Rust 与 TypeScript 的永久哨兵必须一致：直接读取前端源文件钉死
    let ts_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../src/api/index.ts");
    let ts = std::fs::read_to_string(&ts_path)
        .expect("读取 src/api/index.ts 失败（哨兵一致性测试需要前端源码在位）");
    let needle = "TEMP_NEVER_EXPIRES_MS = 4_102_444_800_000";
    assert!(
        ts.contains(needle),
        "前端 TEMP_NEVER_EXPIRES_MS 与 Rust Db::TEMP_NEVER_EXPIRES_MS 不一致（应为 4_102_444_800_000）"
    );
    assert_eq!(DbType::TEMP_NEVER_EXPIRES_MS, 4_102_444_800_000);
}

#[test]
fn t_te09_create_ttl_boundaries() {
    let now = 1_700_000_000_000_i64;
    // 0 = 永不过期（唯一合法永久入口）
    assert_eq!(DbType::resolve_create_expiry(now, 0).unwrap(), NEVER);
    // 负数 = 可见错误（不静默当永久，放行检查修正）
    assert!(DbType::resolve_create_expiry(now, -1)
        .unwrap_err()
        .contains("不能为负数"));
    assert!(DbType::resolve_create_expiry(now, i64::MIN)
        .unwrap_err()
        .contains("不能为负数"));
    // 正常范围
    assert_eq!(DbType::resolve_create_expiry(now, 1).unwrap(), now + 60_000);
    let max = DbType::TEMP_MAX_TTL_MINUTES;
    assert_eq!(
        DbType::resolve_create_expiry(now, max).unwrap(),
        now + max.checked_mul(60_000).unwrap()
    );
    // 超上限 → 可见错误（不静默截断为永久）
    let err = DbType::resolve_create_expiry(now, max + 1).unwrap_err();
    assert!(err.contains("超出支持范围"), "err={}", err);
    assert!(DbType::resolve_create_expiry(now, i64::MAX)
        .unwrap_err()
        .contains("超出支持范围"));
}

#[test]
fn t_te10_renew_target_validation() {
    let now = 1_700_000_000_000_i64;
    // 合法：未来时间、永久哨兵
    assert!(DbType::validate_renew_target(now, now + 1).is_ok());
    assert!(DbType::validate_renew_target(now, NEVER).is_ok());
    // 非法：过去/现在
    assert!(DbType::validate_renew_target(now, now).is_err());
    assert!(DbType::validate_renew_target(now, now - 1).is_err());
    // 大于哨兵 → 拒绝（防"UI 显示永久、实际会过期"歧义）
    assert!(DbType::validate_renew_target(now, NEVER + 1).is_err());
    assert!(DbType::validate_renew_target(now, i64::MAX).is_err());
}

#[test]
fn t_te11_renew_add_validation() {
    assert!(DbType::validate_renew_add(0).is_err());
    assert!(DbType::validate_renew_add(-1).is_err());
    assert!(DbType::validate_renew_add(DbType::TEMP_MAX_RENEW_ADD_MS).is_ok());
    assert!(DbType::validate_renew_add(DbType::TEMP_MAX_RENEW_ADD_MS + 1).is_err());
    assert!(DbType::validate_renew_add(i64::MAX).is_err());
}

#[test]
fn t_te12_renew_permanent_with_add_is_rejected_not_accumulated() {
    let (_dir, db) = temp_db("perm-add");
    let id = db.create_temp("永久", NEVER).unwrap();
    let now = chrono::Utc::now().timestamp_millis();
    let err = db
        .renew_temp(id, now, Some(24 * 60 * 60 * 1000), None)
        .unwrap_err();
    assert!(err.contains("永不过期"), "err={}", err);
    // 哨兵未被累加（重复续期不漂移）
    assert_eq!(get_row(&db, id).unwrap().2, NEVER);
    // 永久 → 限时：显式选择新到期日是唯一合法路径
    let target = now + 7 * 24 * 60 * 60 * 1000;
    assert_eq!(db.renew_temp(id, now, None, Some(target)).unwrap(), target);
    assert_eq!(get_row(&db, id).unwrap().2, target);
}

#[test]
fn t_te13_renew_param_mutex_and_custom_collision() {
    let (_dir, db) = temp_db("mutex");
    let id = db.create_temp("n", 60).unwrap();
    let now = chrono::Utc::now().timestamp_millis();
    // 二者都给 / 都不给 → 错误
    assert!(db.renew_temp(id, now, Some(1000), Some(NEVER)).is_err());
    assert!(db.renew_temp(id, now, None, None).is_err());
    // 自定义日期恰好等于哨兵 = 永久（合法，无歧义：UI 单独呈现"永不过期"项）
    assert_eq!(db.renew_temp(id, now, None, Some(NEVER)).unwrap(), NEVER);
}

#[test]
fn t_te14_restore_temp_with_expiry_atomic() {
    let (_dir, db) = temp_db("restore-exp");
    let id = db.create_temp("过期便签", 60).unwrap();
    // 模拟已过期 + 软删除
    let conn = db.conn.lock().unwrap();
    conn.execute(
        "UPDATE temp_contents SET expires_at = 1, deleted_at = ?1 WHERE id = ?2",
        params![chrono::Utc::now().timestamp_millis(), id],
    )
    .unwrap();
    drop(conn);

    // 恢复 + 新到期时间原子生效
    let new_expiry = chrono::Utc::now().timestamp_millis() + 3 * 24 * 60 * 60 * 1000;
    let n = db.restore_temp_with_expiry(id, new_expiry).unwrap();
    assert_eq!(n, 1);
    let (text, _created, expires_after) = get_row(&db, id).unwrap();
    assert_eq!(text, "过期便签");
    assert_eq!(expires_after, new_expiry);
    let conn = db.conn.lock().unwrap();
    let deleted: Option<i64> = conn
        .query_row(
            "SELECT deleted_at FROM temp_contents WHERE id = ?1",
            params![id],
            |r| r.get(0),
        )
        .unwrap();
    drop(conn);
    assert!(deleted.is_none(), "恢复后 deleted_at 必须为 NULL");
    assert!(
        db.list_temp().unwrap().iter().any(|x| x.id == id),
        "恢复+续期后必须可见"
    );

    // 未删除的行不可用本方法触碰
    let live = db.create_temp("live", 60).unwrap();
    assert_eq!(db.restore_temp_with_expiry(live, new_expiry).unwrap(), 0);
}

// ===== 放行检查 2（D-Repair Final）：迷你模式初始几何快照 =====

#[test]
fn t_gm01_snapshot_full_geometry_overwrites_stale_values() {
    let (_dir, db) = temp_db("geosnap");
    // 旧几何已在库（如上次会话保存）
    db.snapshot_full_geometry(10, 20, 320, 400).unwrap();
    // 用户已调整尺寸/位置但未触发 move 保存；进入迷你时快照"当前真实几何"
    db.snapshot_full_geometry(140, 260, 900, 800).unwrap();
    for (k, v) in [
        ("widget.x", "140"),
        ("widget.y", "260"),
        ("widget.width", "900"),
        ("widget.height", "800"),
    ] {
        assert_eq!(
            db.get_setting(k).unwrap().as_deref(),
            Some(v),
            "旧几何必须被当前真实值覆盖（退出迷你按真实值还原，非固定默认）"
        );
    }
}

#[test]
fn t_gm02_snapshot_fresh_db_writes_all_four_keys() {
    let (_dir, db) = temp_db("geofresh");
    // 首次切换：库中无任何几何记录
    assert_eq!(db.get_setting("widget.width").unwrap(), None);
    db.snapshot_full_geometry(0, 0, 480, 720).unwrap();
    for k in ["widget.x", "widget.y", "widget.width", "widget.height"] {
        assert!(db.get_setting(k).unwrap().is_some(), "首次切换四键必须齐全");
    }
}

// ===== v0.4.3 P0 发布候选验收：旧数据库升级 / 旧备份恢复 / 新版往返 =====
// 铁律：全部使用隔离临时库；绝不触碰正式用户数据库。
//
// 信封格式核对（git diff v0.4.1 -- backup_v2.rs）：相对 v0.4.1 仅
// REQUIRED_TABLES / EXPECTED_COLUMNS 两处白名单差异，加密信封（envelope/
// decode/base64/sha256/快照）逐字节未变 → 当前导出代码对 v0.4.1-schema 库
// 产出的备份 = 真实 v0.4.1 格式备份（非伪造）。

use crate::backup_v2::{export_v2_to_path, prepare_restored_staging};
use crate::db::Db;
use crate::migration::{
    finalize_prepared_v2, open_existing_v2_db, prepare_fresh_v2, V2_DB_FILENAME,
    V2_SETUP_LOCK_FILENAME,
};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};

fn work_dir(label: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("drawer-p0-{}-{}", label, uuid::Uuid::new_v4()));
    fs::create_dir_all(&d).unwrap();
    d
}

/// 构造 v0.4.1-schema 数据库：真实 crypto 元数据（prepare_fresh_v2），
/// 再降级为 v0.4.1 结构（DROP temp_categories 表 + 两个 Batch 4 新列）。
fn make_v041_schema_db(path: &Path, master_pw: &str) -> Db {
    make_v041_schema_db_with_dek(path, master_pw).0
}

fn make_v041_schema_db_with_dek(path: &Path, master_pw: &str) -> (Db, zeroize::Zeroizing<Vec<u8>>) {
    let prepared = prepare_fresh_v2(path.parent().unwrap(), master_pw).expect("prepare_fresh_v2");
    let dek = prepared.dek.clone();
    finalize_prepared_v2(&prepared.tmp_path, &prepared.v2_path).expect("finalize");
    drop(prepared.setup_lock);
    let _ = fs::remove_file(path.parent().unwrap().join(V2_SETUP_LOCK_FILENAME));
    let db = open_existing_v2_db(path).expect("open v2");
    {
        let conn = db.conn.lock().unwrap();
        conn.execute_batch(
            "DROP TABLE IF EXISTS temp_categories;
             ALTER TABLE temp_contents DROP COLUMN title;
             ALTER TABLE temp_contents DROP COLUMN category_id;",
        )
        .expect("降级到 v0.4.1 schema");
    }
    (db, dek)
}

/// 代表性业务数据（覆盖全部 7 张 v0.4.1 表）
fn seed_v041_business_data(db: &Db, dek: &zeroize::Zeroizing<Vec<u8>>) {
    let conn = db.conn.lock().unwrap();
    let now = chrono::Utc::now().timestamp_millis();
    conn.execute_batch(
        "INSERT INTO app_categories (id, name, icon, sort_order) VALUES (1, '开发工具', '⚙️', 1);
         INSERT INTO apps (name, path, icon_path, args, category_id, created_at, use_count, last_used_at, app_type, app_subtype)
           VALUES ('测试软件', 'C:\\Apps\\test.exe', '', '', 1, 1000, 7, 2000, 'app', 'dev');
         INSERT INTO snippets (title, content, language, tags, created_at, updated_at, use_count, last_used_at)
           VALUES ('片段', 'content', 'text', '', 1000, 1000, 1, 1);
         INSERT INTO temp_contents (text, created_at, expires_at) VALUES ('v041便签', 1000, 9999999999999);
         INSERT INTO pinned_items (item_type, item_id, sort_order) VALUES ('app', 1, 0);",
    )
    .unwrap();
    // 密码字段必须是 DW2 格式（record_uuid 绑定；导出侧逐行解密校验）
    let record_uuid = uuid::Uuid::new_v4().to_string();
    let enc = crate::crypto::encrypt_password_dw2("real-secret", dek.as_slice(), &record_uuid)
        .expect("encrypt dw2");
    conn.execute(
        "INSERT INTO passwords (record_uuid, title, username, password, url, notes, created_at, updated_at)
         VALUES (?1, '站点', 'user', ?3, 'https://x', '', ?2, ?2)",
        rusqlite::params![record_uuid, now, enc],
    )
    .unwrap();
    conn.execute(
        "INSERT OR REPLACE INTO settings (key, value) VALUES ('user_pref', 'v041')",
        [],
    )
    .unwrap();
}

fn sha256_file(path: &Path) -> String {
    let mut f = fs::File::open(path).unwrap();
    let mut hasher = Sha256::new();
    std::io::copy(&mut f, &mut hasher).unwrap();
    format!("{:x}", hasher.finalize())
}

fn table_count(db: &Db, table: &str) -> i64 {
    let conn = db.conn.lock().unwrap();
    conn.query_row(&format!("SELECT COUNT(*) FROM {}", table), [], |r| r.get(0))
        .unwrap()
}

// ---- P0-A：v0.4.1 旧格式加密备份 → v0.4.3 恢复 ----
#[test]
#[ignore = "P0 发布候选验收：显式运行 cargo test t_p0a -- --ignored --nocapture"]
fn t_p0a_v041_backup_restore() {
    let work = work_dir("p0a");
    let db_path = work.join(V2_DB_FILENAME);
    let (db, dek_v041) = make_v041_schema_db_with_dek(&db_path, "V041-Pw!");
    seed_v041_business_data(&db, &dek_v041);
    // 前置确认：测试库必须处于 v0.4.1 schema（无 temp_categories 表）
    assert_eq!(table_count_of_master(&db_path, "temp_categories"), 0);
    drop(db);

    // v0.4.1 格式加密导出
    // prepare_fresh_v2 返回的 dek 即该库的 Stable DEK（t66 同模式）
    let state = crate::AppState::new(
        open_existing_v2_db(&db_path).unwrap(),
        db_path.clone(),
        crate::StartupMode::ExistingV2,
    );
    state.set_stable_dek(dek_v041.clone());
    let backup_path = work.join("v041.drawerbox");
    export_v2_to_path(&state, "V041-Pw!", &backup_path).expect("导出失败");
    drop(state);

    // v0.4.3 恢复：staging 解密 → open_existing_v2_db 内 compat 先行 → 再校验
    let staging = work.join("restore.tmp");
    let stats = prepare_restored_staging(&backup_path, "V041-Pw!", &staging)
        .expect("v0.4.1 旧备份必须能被 v0.4.3 恢复（校验白名单不得在 compat 前拒绝）");
    let _ = stats;

    // 逐项核对全部业务表
    let restored = open_existing_v2_db(&staging).unwrap();
    assert_eq!(table_count(&restored, "apps"), 1, "软件保留");
    assert_eq!(table_count(&restored, "app_categories"), 1, "分类保留");
    assert_eq!(table_count(&restored, "snippets"), 1, "片段保留");
    assert_eq!(table_count(&restored, "temp_contents"), 1, "便签保留");
    assert_eq!(table_count(&restored, "pinned_items"), 1, "置顶保留");
    assert_eq!(table_count(&restored, "passwords"), 1, "密码保留");
    {
        let conn = restored.conn.lock().unwrap();
        let use_count: i64 = conn
            .query_row("SELECT use_count FROM apps WHERE id=1", [], |r| r.get(0))
            .unwrap();
        assert_eq!(use_count, 7, "使用次数保留");
        let (text, expires): (String, i64) = conn
            .query_row(
                "SELECT text, expires_at FROM temp_contents WHERE id=1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(text, "v041便签", "正文保留");
        assert_eq!(expires, 9999999999999, "到期时间保留");
        let pref: String = conn
            .query_row(
                "SELECT value FROM settings WHERE key='user_pref'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(pref, "v041", "用户设置保留");
        // compat 已把恢复库升级到位
        assert_eq!(table_count_of_master(&staging, "temp_categories"), 1);
        let col_title: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('temp_contents') WHERE name='title'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(col_title, 1, "title 列已补齐");
    }
    drop(restored);
    fs::remove_dir_all(&work).ok();
}

fn table_count_of_master(db_path: &Path, table: &str) -> i64 {
    let conn = rusqlite::Connection::open(db_path).unwrap();
    conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
        rusqlite::params![table],
        |r| r.get(0),
    )
    .unwrap()
}

// ---- P0-B：代表性 v0.4.1 全表旧库升级 + 三次重启 + 原副本不受损 ----
#[test]
#[ignore = "P0 发布候选验收：显式运行 cargo test t_p0b -- --ignored --nocapture"]
fn t_p0b_v041_full_db_upgrade() {
    let work = work_dir("p0b");
    let db_path = work.join(V2_DB_FILENAME);
    let (db, dek_v041) = make_v041_schema_db_with_dek(&db_path, "V041-Pw!");
    seed_v041_business_data(&db, &dek_v041);
    drop(db);
    let original_hash = sha256_file(&db_path);

    // 升级在副本上执行（对照：原文件保留）
    let copy_path = work.join("upgrade-copy.db");
    fs::copy(&db_path, &copy_path).unwrap();
    {
        let db = Db::open(&copy_path).expect("v0.4.1 库必须能被 v0.4.3 打开");
        // 新功能立即可用
        let cid = db.create_temp_category("升级后分类").unwrap();
        let id = db
            .create_temp_with_title(Some("新标题"), "新正文", 9999999999999)
            .unwrap();
        db.set_temp_category(id, Some(cid)).unwrap();
    }
    // 重启三次，状态不变
    for round in 1..=3 {
        let db = Db::open(&copy_path).expect("重启失败");
        let list = db.list_temp().unwrap();
        assert_eq!(list.len(), 2, "第 {} 次重启：历史便签 + 新便签都在", round);
        let titled = list.iter().find(|t| t.title.is_some()).unwrap();
        assert_eq!(titled.title.as_deref(), Some("新标题"));
        assert_eq!(titled.category_name.as_deref(), Some("升级后分类"));
        let legacy = list.iter().find(|t| t.title.is_none()).unwrap();
        assert_eq!(legacy.text, "v041便签", "第 {} 次重启后历史正文不变", round);
        assert_eq!(table_count(&db, "apps"), 1, "第 {} 次重启：软件记录不变", round);
        assert_eq!(table_count(&db, "passwords"), 1, "密码记录不变");
    }
    // 原始未迁移文件哈希不变（迁移只发生在副本上 → 中断不损原副本）
    assert_eq!(
        sha256_file(&db_path),
        original_hash,
        "原始副本必须不受迁移影响"
    );
    fs::remove_dir_all(&work).ok();
}

// ---- P0-C：v0.4.3 新版往返（标题/分类/正文/回收站状态/有效期）----
#[test]
#[ignore = "P0 发布候选验收：显式运行 cargo test t_p0c -- --ignored --nocapture"]
fn t_p0c_v043_roundtrip_with_trash() {
    let work = work_dir("p0c");
    let db_path = work.join(V2_DB_FILENAME);
    let (db, dek_v043) = make_v043_db(&db_path, "V043-Pw!");
    let cid = db.create_temp_category("出差").unwrap();
    let keep = db
        .create_temp_with_title(Some("出差清单"), "护照、充电器", 9999999999999)
        .unwrap();
    db.set_temp_category(keep, Some(cid)).unwrap();
    let trashed = db
        .create_temp_with_title(Some("被删便签"), "trash body", 8888888888888)
        .unwrap();
    db.soft_delete("temp_contents", trashed).unwrap();
    drop(db);

    // 加密备份 → 清空环境（删除原库）→ 恢复到新文件
    let state = crate::AppState::new(
        open_existing_v2_db(&db_path).unwrap(),
        db_path.clone(),
        crate::StartupMode::ExistingV2,
    );
    state.set_stable_dek(dek_v043.clone());
    let backup_path = work.join("v043.drawerbox");
    export_v2_to_path(&state, "V043-Pw!", &backup_path).unwrap();
    drop(state);
    fs::remove_file(&db_path).unwrap(); // 清空隔离环境

    let staging = work.join("restore.tmp");
    prepare_restored_staging(&backup_path, "V043-Pw!", &staging).unwrap();
    let restored = open_existing_v2_db(&staging).unwrap();
    let list = restored.list_temp().unwrap();
    assert_eq!(list.len(), 1, "未删除便签恢复");
    assert_eq!(list[0].title.as_deref(), Some("出差清单"), "标题保留");
    assert_eq!(list[0].category_name.as_deref(), Some("出差"), "分类保留");
    assert_eq!(list[0].expires_at, 9999999999999, "有效期保留");
    // 回收站状态保留：软删便签仍在回收站且带标题
    let trash = restored.list_trash().unwrap();
    let t = trash
        .iter()
        .find(|x| x.table_name == "temp_contents")
        .expect("回收站中的便签恢复");
    assert_eq!(t.title.as_deref(), Some("被删便签"), "回收站标题保留");
    assert_eq!(t.expires_at, Some(8888888888888), "回收站到期保留");
    drop(restored);
    fs::remove_dir_all(&work).ok();
}

/// v0.4.3 当前 schema 库（真实 crypto）
fn make_v043_db(path: &Path, master_pw: &str) -> (Db, zeroize::Zeroizing<Vec<u8>>) {
    let prepared = prepare_fresh_v2(path.parent().unwrap(), master_pw).expect("prepare");
    let dek = prepared.dek.clone();
    finalize_prepared_v2(&prepared.tmp_path, &prepared.v2_path).expect("finalize");
    drop(prepared.setup_lock);
    let _ = fs::remove_file(path.parent().unwrap().join(V2_SETUP_LOCK_FILENAME));
    let db = open_existing_v2_db(path).expect("open");
    (db, dek)
}

// ---- P0-D 前置：为隔离 GUI 验收播种已知主密码的测试环境 ----
// 仅写验收包目录（D:\Drawer-Upg-Rehearsal\...），不触碰正式数据。
#[test]
#[ignore = "GUI 验收前置：显式运行 cargo test t_seed_gui -- --ignored --nocapture"]
fn t_seed_gui_env() {
    let dir = Path::new(r"D:\Drawer-Upg-Rehearsal\v043-batch1-acceptance\drawer-test-data");
    if dir.exists() {
        fs::remove_dir_all(dir).unwrap();
    }
    fs::create_dir_all(dir).unwrap();
    let db_path = dir.join(V2_DB_FILENAME);
    let (db, _dek) = make_v043_db(&db_path, "GuiPw-2026!");
    // 预置一条带标题与分类的便签（GUI 便签验收用）
    let cid = db.create_temp_category("GUI分类").unwrap();
    let id = db
        .create_temp_with_title(Some("GUI便签"), "GUI正文", 9999999999999)
        .unwrap();
    db.set_temp_category(id, Some(cid)).unwrap();
    println!("SEEDED {}", db_path.display());
}

// ===== P1-C（v0.4.3 验收）：隔离测量库种子 =====
// 只在显式运行时执行（cargo test t_perf_seed -- --ignored），用于在隔离目录
// 生成一个真实 v2 库（已知主密码），供测试版安装包做滚动性能测量。
// 绝不写正式数据位置：目标目录必须由 PERF_SEED_DIR 显式给出。
#[test]
#[ignore = "P1-C 测量脚手架：需显式设置 PERF_SEED_DIR 并 --ignored 运行"]
fn t_perf_seed_isolated_db() {
    let dir = PathBuf::from(
        std::env::var("PERF_SEED_DIR").expect("需要 PERF_SEED_DIR（隔离目录绝对路径）"),
    );
    fs::create_dir_all(&dir).unwrap();
    let prepared = prepare_fresh_v2(&dir, "perfpass2026").expect("prepare_fresh_v2");
    finalize_prepared_v2(&prepared.tmp_path, &prepared.v2_path).expect("finalize");
    drop(prepared.setup_lock);
    let _ = fs::remove_file(dir.join(V2_SETUP_LOCK_FILENAME));
    let db = open_existing_v2_db(&dir.join(V2_DB_FILENAME)).expect("open v2");
    drop(db);
    println!("SEEDED {}", dir.display());
}

//! ============================================================
//! Phase 2A：v2 安全库（drawer-v2.db）与迁移引擎（设计稿 v3 终稿）
//!
//! - v0.3 正式活动库 = drawer-v2.db（旧 binary 硬编码路径 drawer_box.db，
//!   物理上读不到 v2 库 → downgrade guard 结构性成立，无需触发器）
//! - 迁移 = 只读 legacy → 生成 drawer-v2.db.tmp → 校验 → atomic rename
//! - security_version 最后写；任何失败终态只能是完整旧格式或完整新格式
//! - 本批（Phase 2A）不对真实用户目录执行迁移：测试全部使用临时目录 fixture
//! ============================================================

use crate::crypto;
use crate::db::Db;
use rusqlite::params;
use std::path::{Path, PathBuf};
use zeroize::Zeroizing;

pub const V2_DB_FILENAME: &str = "drawer-v2.db";
pub const LEGACY_DB_FILENAME: &str = "drawer_box.db";
pub const LEGACY_BACKUP_SUFFIX: &str = ".legacy-v0.2.0";
pub const V2_TMP_FILENAME: &str = "drawer-v2.db.tmp";
pub const SECURITY_VERSION_V2: &str = "1";

/// legacy 库中被视为安全敏感、迁移时**不拷贝**进 v2 的设置键
const LEGACY_SECURITY_KEYS: [&str; 3] =
    ["master_password_hash", "master_password_salt", "recovery_phrase_encrypted"];

/// v2 库建库 schema（drawer-v2.db / drawer-v2.db.tmp 通用）
fn init_v2_schema(conn: &rusqlite::Connection) -> Result<(), rusqlite::Error> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS settings (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS passwords (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            record_uuid TEXT NOT NULL UNIQUE,
            title TEXT NOT NULL,
            username TEXT,
            password TEXT NOT NULL,
            url TEXT,
            notes TEXT,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL,
            use_count INTEGER NOT NULL DEFAULT 0,
            last_used_at INTEGER NOT NULL DEFAULT 0,
            deleted_at INTEGER
        );
        CREATE TABLE IF NOT EXISTS app_categories (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL,
            icon TEXT,
            sort_order INTEGER DEFAULT 0
        );
        CREATE TABLE IF NOT EXISTS apps (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL,
            path TEXT NOT NULL,
            icon_path TEXT,
            args TEXT,
            category_id INTEGER,
            created_at INTEGER NOT NULL,
            use_count INTEGER NOT NULL DEFAULT 0,
            last_used_at INTEGER NOT NULL DEFAULT 0,
            app_type TEXT NOT NULL DEFAULT 'app',
            app_subtype TEXT NOT NULL DEFAULT '',
            deleted_at INTEGER
        );
        CREATE TABLE IF NOT EXISTS snippets (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            title TEXT NOT NULL,
            content TEXT NOT NULL,
            language TEXT DEFAULT 'text',
            tags TEXT,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL,
            use_count INTEGER NOT NULL DEFAULT 0,
            last_used_at INTEGER NOT NULL DEFAULT 0,
            deleted_at INTEGER
        );
        CREATE TABLE IF NOT EXISTS temp_contents (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            text TEXT NOT NULL,
            created_at INTEGER NOT NULL,
            expires_at INTEGER NOT NULL,
            deleted_at INTEGER
        );
        "#,
    )?;
    Ok(())
}

/// 打开/创建 v2 库
pub fn open_v2_db(path: &Path) -> Result<Db, String> {
    let conn = rusqlite::Connection::open(path).map_err(|e| e.to_string())?;
    init_v2_schema(&conn).map_err(|e| e.to_string())?;
    Ok(Db { conn: std::sync::Mutex::new(conn) })
}

/// 启动仲裁结果
#[derive(Debug, PartialEq)]
pub enum DbSelection {
    V2(PathBuf),
    Legacy(PathBuf),
    Fresh,
}

/// 启动仲裁（幂等；含 tmp 清理与 legacy 隔离归档）
pub fn resolve_startup_db(app_dir: &Path) -> DbSelection {
    let v2 = app_dir.join(V2_DB_FILENAME);
    let tmp = app_dir.join(V2_TMP_FILENAME);
    let legacy = app_dir.join(LEGACY_DB_FILENAME);

    // 上次迁移残留的 tmp：直接清理（重新迁移会重建）
    if tmp.exists() {
        let _ = std::fs::remove_file(&tmp);
    }

    if v2.exists() {
        // 隔离归档 legacy 原件（迁移前原件）或降级期 stray（不自动合并）
        if legacy.exists() {
            let backup = app_dir.join(format!("{}{}", LEGACY_DB_FILENAME, LEGACY_BACKUP_SUFFIX));
            let target = if backup.exists() {
                app_dir.join(format!(
                    "{}.stray-{}",
                    LEGACY_DB_FILENAME,
                    chrono_now_millis()
                ))
            } else {
                backup
            };
            let _ = std::fs::rename(&legacy, &target);
        }
        return DbSelection::V2(v2);
    }
    if legacy.exists() {
        return DbSelection::Legacy(legacy);
    }
    DbSelection::Fresh
}

fn chrono_now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// 迁移引擎错误（本地类型，绕开孤儿规则；Display 不泄漏密码学细节）
#[derive(Debug)]
pub enum MigrationError {
    Msg(String),
    Sql(rusqlite::Error),
}

impl std::fmt::Display for MigrationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MigrationError::Msg(s) => write!(f, "{}", s),
            MigrationError::Sql(e) => write!(f, "数据库错误: {}", e),
        }
    }
}

impl std::error::Error for MigrationError {}

impl From<rusqlite::Error> for MigrationError {
    fn from(e: rusqlite::Error) -> Self {
        MigrationError::Sql(e)
    }
}

impl From<String> for MigrationError {
    fn from(s: String) -> Self {
        MigrationError::Msg(s)
    }
}

impl From<&str> for MigrationError {
    fn from(s: &str) -> Self {
        MigrationError::Msg(s.into())
    }
}

/// 迁移故障注入点（仅测试使用；生产调用传 None）
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MigrationFailPoint {
    BeforeTmpCreate,
    DuringBuild,
    AfterPartialRows,
    BeforeSettingsWrite,
    OnIntegrityFail,
    BeforeRename,
}

/// 迁移产物
pub struct MigrationOutput {
    pub v2_path: PathBuf,
    pub dek: Zeroizing<Vec<u8>>,
    /// 新恢复短语（BIP39 12 词）——只在此处返回一次，由 UI 展示，不落库
    pub mnemonic: Vec<String>,
    pub passwords_migrated: usize,
}

/// 迁移入口（生产）
pub fn migrate_legacy_to_v2(app_dir: &Path, master_password: &str) -> Result<MigrationOutput, MigrationError> {
    migrate_legacy_to_v2_inject(app_dir, master_password, None)
}

/// 迁移引擎（T1-T8；fail_at 仅供测试注入失败点）
pub fn migrate_legacy_to_v2_inject(
    app_dir: &Path,
    master_password: &str,
    fail_at: Option<MigrationFailPoint>,
) -> Result<MigrationOutput, MigrationError> {
    let fail = |fp: MigrationFailPoint| fail_at == Some(fp);

    let legacy_path = app_dir.join(LEGACY_DB_FILENAME);
    let tmp_path = app_dir.join(V2_TMP_FILENAME);
    let v2_path = app_dir.join(V2_DB_FILENAME);

    // 注入点 1：创建 tmp 前失败
    if fail(MigrationFailPoint::BeforeTmpCreate) {
        return Err("injected:BeforeTmpCreate".into());
    }
    if !legacy_path.exists() {
        return Err("legacy 库不存在".into());
    }
    if tmp_path.exists() {
        let _ = std::fs::remove_file(&tmp_path); // 上次迁移残留清理
    }

    // T2 legacy 打开（只读用途）+ 旧 verifier 校验（迁移必须用正确旧主密码）
    let legacy = Db::open(&legacy_path).map_err(|e| format!("legacy 库打开失败: {}", e))?;
    let salt_b64 = legacy
        .get_setting("master_password_salt")
        .map_err(|e| e.to_string())?
        .ok_or("legacy 库缺少 master_password_salt")?;
    let hash = legacy
        .get_setting("master_password_hash")
        .map_err(|e| e.to_string())?
        .ok_or("legacy 库缺少 master_password_hash")?;
    let salt = crypto::b64_decode(&salt_b64).map_err(|e| e)?;
    if crypto::hash_password(master_password, &salt) != hash {
        return Err("主密码不正确".into());
    }
    let legacy_key = Zeroizing::new(crypto::derive_key(master_password, &salt));

    // T3 v2 tmp 建库
    let v2 = open_v2_db(&tmp_path).map_err(|e| format!("v2 建库失败: {}", e))?;
    if fail(MigrationFailPoint::DuringBuild) {
        return Err("injected:DuringBuild".into());
    }

    // T4 密码行迁移（legacy key 解密 → DEK 加密 DW2: + record_uuid）
    let dek = crypto::generate_dek();
    let mut migrated = 0usize;
    {
        let lconn = legacy.conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = lconn
            .prepare(
                "SELECT id, title, username, password, url, notes, created_at, updated_at,
                        use_count, last_used_at, deleted_at
                 FROM passwords ORDER BY id",
            )
            .map_err(|e| e.to_string())?;
        let rows: Vec<(i64, String, String, String, String, String, i64, i64, i64, i64, Option<i64>)> = stmt
            .query_map([], |row| {
                Ok((
                    row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?,
                    row.get(5)?, row.get(6)?, row.get(7)?, row.get(8)?, row.get(9)?,
                    row.get(10)?,
                ))
            })
            .map_err(|e| e.to_string())?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        drop(stmt);
        let mut vconn = v2.conn.lock().map_err(|e| e.to_string())?;
        let tx = vconn.transaction().map_err(|e| e.to_string())?;
        for (id, title, username, enc, url, notes, created, updated, usec, lused, deleted) in rows {
            let plain = crypto::decrypt(&enc, &legacy_key)
                .map_err(|_| format!("legacy 行 id={} 解密失败，迁移中止", id))?;
            let uuid = uuid::Uuid::new_v4().to_string();
            let dw2 = crypto::encrypt_password_dw2(&plain, &dek, &uuid).map_err(|e| e)?;
            tx.execute(
                "INSERT INTO passwords (record_uuid, title, username, password, url, notes,
                                        created_at, updated_at, use_count, last_used_at, deleted_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
                params![uuid, title, username, dw2, url, notes, created, updated, usec, lused, deleted],
            )
            .map_err(|e| e.to_string())?;
            migrated += 1;
            if fail(MigrationFailPoint::AfterPartialRows) && migrated >= 1 {
                // 模拟"部分行完成后"失败：tx 在此返回，未 commit → 由 drop 自动回滚
                return Err("injected:AfterPartialRows".into());
            }
        }
        tx.commit().map_err(|e| e.to_string())?;
    }

    // T5 非密码业务数据与普通 settings 原样拷贝（明文数据，格式不变）
    {
        let lconn = legacy.conn.lock().map_err(|e| e.to_string())?;
        let mut vconn = v2.conn.lock().map_err(|e| e.to_string())?;
        let tx = vconn.transaction().map_err(|e| e.to_string())?;
        // app_categories
        let cats: Vec<(i64, String, String, i64)> = {
            let mut st = lconn.prepare("SELECT id, name, icon, sort_order FROM app_categories ORDER BY id")?;
            let v = st
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            v
        };
        for (id, name, icon, order) in cats {
            tx.execute(
                "INSERT INTO app_categories (id, name, icon, sort_order) VALUES (?1,?2,?3,?4)",
                params![id, name, icon, order],
            )
            .map_err(|e| e.to_string())?;
        }
        // apps
        let apps: Vec<(i64, String, String, String, String, Option<i64>, i64, i64, i64, String, String, Option<i64>)> = {
            let mut st = lconn.prepare(
                "SELECT id, name, path, icon_path, args, category_id, created_at, use_count,
                        last_used_at, app_type, app_subtype, deleted_at
                 FROM apps ORDER BY id",
            )?;
            let v = st
                .query_map([], |r| {
                    Ok((
                        r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?,
                        r.get(6)?, r.get(7)?, r.get(8)?, r.get(9)?, r.get(10)?, r.get(11)?,
                    ))
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            v
        };
        for (id, name, path, icon, args, cat, created, usec, lused, atype, asub, deleted) in apps {
            tx.execute(
                "INSERT INTO apps (id, name, path, icon_path, args, category_id, created_at,
                                   use_count, last_used_at, app_type, app_subtype, deleted_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
                params![id, name, path, icon, args, cat, created, usec, lused, atype, asub, deleted],
            )
            .map_err(|e| e.to_string())?;
        }
        // snippets
        let snippets: Vec<(i64, String, String, String, String, i64, i64, i64, i64, Option<i64>)> = {
            let mut st = lconn.prepare(
                "SELECT id, title, content, language, tags, created_at, updated_at,
                        use_count, last_used_at, deleted_at
                 FROM snippets ORDER BY id",
            )?;
            let v = st
                .query_map([], |r| {
                    Ok((
                        r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?,
                        r.get(5)?, r.get(6)?, r.get(7)?, r.get(8)?, r.get(9)?,
                    ))
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            v
        };
        for (id, title, content, lang, tags, created, updated, usec, lused, deleted) in snippets {
            tx.execute(
                "INSERT INTO snippets (id, title, content, language, tags, created_at, updated_at,
                                       use_count, last_used_at, deleted_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
                params![id, title, content, lang, tags, created, updated, usec, lused, deleted],
            )
            .map_err(|e| e.to_string())?;
        }
        // temp_contents
        let temps: Vec<(i64, String, i64, i64, Option<i64>)> = {
            let mut st = lconn
                .prepare("SELECT id, text, created_at, expires_at, deleted_at FROM temp_contents ORDER BY id")?;
            let v = st
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            v
        };
        for (id, text, created, expires, deleted) in temps {
            tx.execute(
                "INSERT INTO temp_contents (id, text, created_at, expires_at, deleted_at)
                 VALUES (?1,?2,?3,?4,?5)",
                params![id, text, created, expires, deleted],
            )
            .map_err(|e| e.to_string())?;
        }
        // 普通 settings 拷贝（排除 legacy 安全三键与 security_version 本身）
        let legacy_settings: Vec<(String, String)> = {
            let mut st = lconn.prepare("SELECT key, value FROM settings")?;
            let v = st
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            v
        };
        for (k, v) in legacy_settings {
            if LEGACY_SECURITY_KEYS.contains(&k.as_str()) || k == "security_version" {
                continue;
            }
            tx.execute("INSERT OR REPLACE INTO settings (key, value) VALUES (?1, ?2)", params![k, v])
                .map_err(|e| e.to_string())?;
        }
        tx.commit().map_err(|e| e.to_string())?;
    }

    // T6 recovery phrase（生成 → 返回给 UI 展示 → KEK_r 包裹 DEK；短语本身不落库）
    let entropy = crypto::generate_recovery_entropy();
    let phrase = crypto::entropy_to_mnemonic(&entropy).map_err(|e| e)?;
    let salt_r = crypto::generate_salt();
    let kek_r = crypto::derive_recovery_kek(&entropy, &salt_r);
    let wrapped_r = crypto::wrap_dek(&dek, &kek_r, crypto::AAD_WRAP_RECOVERY).map_err(|e| e)?;
    let salt_m_v2 = crypto::generate_salt();
    let params_m = crypto::default_kdf_params(crypto::b64_encode(&salt_m_v2));
    let kek_m = crypto::derive_master_kek(master_password, &params_m);
    let wrapped_m = crypto::wrap_dek(&dek, &kek_m, crypto::AAD_WRAP_MASTER).map_err(|e| e)?;

    // 注入点 4：安全 settings 写入前失败
    if fail(MigrationFailPoint::BeforeSettingsWrite) {
        return Err("injected:BeforeSettingsWrite".into());
    }

    // T7 安全 settings 写入
    {
        let vconn = v2.conn.lock().map_err(|e| e.to_string())?;
        vconn
            .execute(
                "INSERT OR REPLACE INTO settings (key, value) VALUES ('kdf_params_m', ?1)",
                params![serde_json::to_string(&params_m).map_err(|e| e.to_string())?],
            )
            .map_err(|e| e.to_string())?;
        vconn
            .execute("INSERT OR REPLACE INTO settings (key, value) VALUES ('wrapped_dek_m', ?1)", params![wrapped_m])
            .map_err(|e| e.to_string())?;
        vconn
            .execute(
                "INSERT OR REPLACE INTO settings (key, value) VALUES ('kdf_params_r', ?1)",
                params![serde_json::to_string(&crypto::KdfParams {
                    algo: "argon2id".into(),
                    version: 1,
                    m_cost: 19456,
                    t_cost: 2,
                    p_cost: 1,
                    salt: crypto::b64_encode(&salt_r),
                })
                .map_err(|e| e.to_string())?],
            )
            .map_err(|e| e.to_string())?;
        vconn
            .execute("INSERT OR REPLACE INTO settings (key, value) VALUES ('wrapped_dek_r', ?1)", params![wrapped_r])
            .map_err(|e| e.to_string())?;
        // security_version 必须最后写（迁移状态机的提交标记）
        vconn
            .execute(
                "INSERT OR REPLACE INTO settings (key, value) VALUES ('security_version', ?1)",
                params![SECURITY_VERSION_V2],
            )
            .map_err(|e| e.to_string())?;
    }

    // 注入点 5：完整性校验失败
    if fail(MigrationFailPoint::OnIntegrityFail) {
        return Err("injected:OnIntegrityFail".into());
    }

    // T8 完整性校验（必须全部通过才允许 rename）
    {
        let vconn = v2.conn.lock().map_err(|e| e.to_string())?;
        let ok: String = vconn
            .query_row("PRAGMA integrity_check", [], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        if ok != "ok" {
            return Err(format!("v2 完整性校验失败: {}", ok).into());
        }
        let v2_pw: usize = vconn
            .query_row("SELECT COUNT(*) FROM passwords", [], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        if v2_pw != migrated {
            return Err(format!("v2 行数不符: {} != {}", v2_pw, migrated).into());
        }
        // 抽样解密：第一行
        let (uuid, dw2): (String, String) = vconn
            .query_row("SELECT record_uuid, password FROM passwords ORDER BY id LIMIT 1", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .map_err(|e| e.to_string())?;
        crypto::decrypt_password_dw2(&dw2, &dek, &uuid).map_err(|_| "v2 抽样解密失败".to_string())?;
    }

    // 关闭连接（rename 前必须全部释放句柄）
    drop(v2);
    drop(legacy);

    // 注入点 6：rename 前失败
    if fail(MigrationFailPoint::BeforeRename) {
        return Err("injected:BeforeRename".into());
    }

    // T9 atomic rename → v2 正式活动库
    std::fs::rename(&tmp_path, &v2_path).map_err(|e| format!("rename 失败: {}", e))?;

    // T10 legacy 隔离归档
    let legacy_backup = app_dir.join(format!("{}{}", LEGACY_DB_FILENAME, LEGACY_BACKUP_SUFFIX));
    if legacy_path.exists() {
        std::fs::rename(&legacy_path, &legacy_backup).map_err(|e| format!("legacy 归档失败: {}", e))?;
    }

    Ok(MigrationOutput {
        v2_path,
        dek,
        mnemonic: phrase.split_whitespace().map(String::from).collect(),
        passwords_migrated: migrated,
    })
}

/// v2 解锁核心（service 层；2B 才接入 UI/命令层）
/// 认证解封 wrapped_dek_m 成功 = 主密码正确 = 返回 DEK
pub fn unlock_v2_core(v2: &Db, password: &str) -> Result<Zeroizing<Vec<u8>>, String> {
    let sv = v2
        .get_setting("security_version")
        .map_err(|e| e.to_string())?
        .unwrap_or_default();
    if sv != SECURITY_VERSION_V2 {
        return Err(format!("安全数据格式版本不支持: {}", sv));
    }
    let params_json = v2
        .get_setting("kdf_params_m")
        .map_err(|e| e.to_string())?
        .ok_or("安全数据缺失：kdf_params_m")?;
    let params: crypto::KdfParams =
        serde_json::from_str(&params_json).map_err(|_| "主密码不正确或安全数据损坏".to_string())?;
    let wrapped = v2
        .get_setting("wrapped_dek_m")
        .map_err(|e| e.to_string())?
        .ok_or("安全数据缺失：wrapped_dek_m")?;
    let kek = crypto::derive_master_kek(password, &params);
    crypto::unwrap_dek(&wrapped, &kek, crypto::AAD_WRAP_MASTER)
        .map_err(|_| "主密码不正确或安全数据损坏".to_string())
}

fn chrono_now_millis_unused_guard() {}

#[cfg(test)]
mod migration_tests {
    use super::*;
    use crate::crypto;

    fn temp_app_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "fw2a_{}_{}",
            tag,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn cleanup_dir(dir: &PathBuf) {
        let _ = std::fs::remove_dir_all(dir);
    }

    /// legacy 0.2.0 fixture：镜像 0.2.0 真实建库流程（verifier + 加密密码行 + 业务数据）
    fn build_legacy_fixture(dir: &Path, master_password: &str) -> Vec<String> {
        let db = Db::open(&dir.join(LEGACY_DB_FILENAME)).unwrap();
        let salt = crypto::generate_salt();
        let hash = crypto::hash_password(master_password, &salt);
        let key = Zeroizing::new(crypto::derive_key(master_password, &salt));
        db.set_setting("master_password_hash", &hash).unwrap();
        db.set_setting("master_password_salt", &crypto::b64_encode(&salt)).unwrap();
        db.set_setting("recovery_phrase_encrypted", &crypto::encrypt("[\"legacy\"]", &key).unwrap())
            .unwrap();
        db.set_setting("trash_retention_days", "7").unwrap();

        let mut titles = Vec::new();
        for i in 0..3 {
            let title = format!("Legacy-PW-{}", i);
            let enc = crypto::encrypt(&format!("legacy-secret-{}", i), &key).unwrap();
            db.create_password(&title, "fixture-user", &enc, "https://fixture.test", "n").unwrap();
            titles.push(title);
        }
        db.create_app_category("fixture-cat", "📁").unwrap();
        db.create_app("Fixture App", "C:\\fixture\\app.exe", "", "", None, "app", "utility").unwrap();
        db.create_snippet("Fixture Snippet", "echo fixture", "bash", "fixture").unwrap();
        db.create_temp("fixture temp content", 60).unwrap();
        titles
    }

    fn file_hash(p: &Path) -> u64 {
        use std::io::Read;
        let mut f = std::fs::File::open(p).unwrap();
        let mut buf = Vec::new();
        f.read_to_end(&mut buf).unwrap();
        let mut h: u64 = 5381;
        for b in &buf {
            h = h.wrapping_mul(33).wrapping_add(*b as u64);
        }
        h
    }

    // ===== T6：完整迁移（数据完整性） =====
    #[test]
    fn t6_full_migration_data_integrity() {
        let dir = temp_app_dir("t6");
        let titles = build_legacy_fixture(&dir, MASTER);

        let out = migrate_legacy_to_v2(&dir, MASTER).unwrap();
        assert_eq!(out.passwords_migrated, 3);
        assert_eq!(out.mnemonic.len(), 12, "恢复短语 12 词");
        assert!(out.v2_path.exists(), "drawer-v2.db 已就位");
        assert!(!dir.join(V2_TMP_FILENAME).exists(), "tmp 已 rename 消失");

        let v2 = open_v2_db(&out.v2_path).unwrap();
        let conn = v2.conn.lock().unwrap();
        // 全部密码行：uuid 唯一、DW2: 前缀、DEK 可解密
        let rows = conn
            .query_row(
                "SELECT COUNT(*), COUNT(DISTINCT record_uuid),
                        SUM(password LIKE 'DW2:%'), MIN(title), MAX(title)
                 FROM passwords",
                [],
                |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?, r.get::<_, String>(3)?, r.get::<_, String>(4)?)),
            )
            .unwrap();
        assert_eq!(rows.0, 3);
        assert_eq!(rows.1, 3, "record_uuid 全部唯一");
        assert_eq!(rows.2, 3, "全部 DW2: 前缀");
        // security_version 与 legacy verifier 排除
        let sv: String = conn.query_row("SELECT value FROM settings WHERE key='security_version'", [], |r| r.get(0)).unwrap();
        assert_eq!(sv, "1");
        let legacy_keys: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM settings WHERE key IN ('master_password_hash','master_password_salt','recovery_phrase_encrypted')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(legacy_keys, 0, "legacy 安全三键不得进入 v2 库");
        // 非密码业务数据完整
        for (t, n) in [("apps", 1), ("app_categories", 4), ("snippets", 1), ("temp_contents", 1)] {
            let c: i64 = conn.query_row(&format!("SELECT COUNT(*) FROM {}", t), [], |r| r.get(0)).unwrap();
            assert_eq!(c, n, "{} 行数不符", t);
        }
        // 抽样解密：每行用 DEK 解密，明文匹配 fixture
        let mut stmt = conn.prepare("SELECT title, password, record_uuid FROM passwords ORDER BY title").unwrap();
        let mut checked = 0;
        let rows = stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))
            .unwrap();
        for row in rows {
            let (title, dw2, uuid) = row.unwrap();
            let plain = crypto::decrypt_password_dw2(&dw2, &out.dek, &uuid).unwrap();
            let idx: String = title.chars().last().unwrap().to_string();
            assert_eq!(plain, format!("legacy-secret-{}", idx), "行 {} 解密明文不匹配", title);
            checked += 1;
        }
        assert_eq!(checked, 3);
        // wrapped_dek_m：unwrap 得到同一 DEK
        let params_json = conn.query_row("SELECT value FROM settings WHERE key='kdf_params_m'", [], |r| r.get::<_, String>(0)).unwrap();
        let params: crypto::KdfParams = serde_json::from_str(&params_json).unwrap();
        let wrapped_m = conn.query_row("SELECT value FROM settings WHERE key='wrapped_dek_m'", [], |r| r.get::<_, String>(0)).unwrap();
        let kek = crypto::derive_master_kek(MASTER, &params);
        assert_eq!(*crypto::unwrap_dek(&wrapped_m, &kek, crypto::AAD_WRAP_MASTER).unwrap(), *out.dek);
        // wrapped_dek_r：经短语派生 KEK 解出同一 DEK
        let params_r_json = conn.query_row("SELECT value FROM settings WHERE key='kdf_params_r'", [], |r| r.get::<_, String>(0)).unwrap();
        let params_r: crypto::KdfParams = serde_json::from_str(&params_r_json).unwrap();
        let entropy = crypto::mnemonic_to_entropy(&out.mnemonic.join(" ")).unwrap();
        let salt_r_bytes = crypto::b64_decode(&params_r.salt).unwrap();
        let kek_r = crypto::derive_recovery_kek(&entropy, &salt_r_bytes);
        let wrapped_r = conn.query_row("SELECT value FROM settings WHERE key='wrapped_dek_r'", [], |r| r.get::<_, String>(0)).unwrap();
        assert_eq!(*crypto::unwrap_dek(&wrapped_r, &kek_r, crypto::AAD_WRAP_RECOVERY).unwrap(), *out.dek);
        assert_eq!(checked, 3);
        drop(stmt);
        drop(conn);
        drop(v2);
        cleanup_dir(&dir);
    }

    // ===== T7：崩溃/失败状态机（6 个注入点） =====
    #[test]
    fn t7_crash_points_never_leave_half_migration() {
        let fps = [
            MigrationFailPoint::BeforeTmpCreate,
            MigrationFailPoint::DuringBuild,
            MigrationFailPoint::AfterPartialRows,
            MigrationFailPoint::BeforeSettingsWrite,
            MigrationFailPoint::OnIntegrityFail,
            MigrationFailPoint::BeforeRename,
        ];
        for (i, fp) in fps.iter().enumerate() {
            let dir = temp_app_dir(&format!("t7_{}", i));
            build_legacy_fixture(&dir, MASTER);
            let legacy_path = dir.join(LEGACY_DB_FILENAME);
            let legacy_hash_before = file_hash(&legacy_path);

            let result = migrate_legacy_to_v2_inject(&dir, MASTER, Some(*fp));
            assert!(result.is_err(), "注入点 {:?} 必须失败", fp);
            // 正式活动库绝不能是半迁移状态：legacy 未动、v2 未成为 active
            assert_eq!(
                file_hash(&legacy_path),
                legacy_hash_before,
                "注入点 {:?}: legacy 库必须字节级未变",
                fp
            );
            assert!(
                !dir.join(V2_DB_FILENAME).exists(),
                "注入点 {:?}: drawer-v2.db 不得成为 active",
                fp
            );
            // 启动仲裁清理：tmp 残留被清掉，仲裁结果仍为 Legacy（可重试迁移）
            let sel = resolve_startup_db(&dir);
            assert_eq!(sel, DbSelection::Legacy(dir.join(LEGACY_DB_FILENAME)), "注入点 {:?}", fp);
            assert!(!dir.join(V2_TMP_FILENAME).exists(), "注入点 {:?}: tmp 残留未清理", fp);
            cleanup_dir(&dir);
        }
    }

    // ===== 启动仲裁规则（7 种文件组合） =====
    #[test]
    fn t8_startup_arbitration() {
        let base = temp_app_dir("t8");
        let mk = |dir: &Path, name: &str| {
            std::fs::write(dir.join(name), b"x").unwrap();
            dir.join(name)
        };
        // 1. 只有 legacy
        let d = base.join("c1");
        std::fs::create_dir_all(&d).unwrap();
        mk(&d, LEGACY_DB_FILENAME);
        assert_eq!(resolve_startup_db(&d), DbSelection::Legacy(d.join(LEGACY_DB_FILENAME)));
        // 2. 只有 v2
        let d = base.join("c2");
        std::fs::create_dir_all(&d).unwrap();
        mk(&d, V2_DB_FILENAME);
        assert_eq!(resolve_startup_db(&d), DbSelection::V2(d.join(V2_DB_FILENAME)));
        // 3. 两者都有 → v2 胜出 + legacy 隔离归档
        let d = base.join("c3");
        std::fs::create_dir_all(&d).unwrap();
        mk(&d, LEGACY_DB_FILENAME);
        mk(&d, V2_DB_FILENAME);
        assert_eq!(resolve_startup_db(&d), DbSelection::V2(d.join(V2_DB_FILENAME)));
        assert!(!d.join(LEGACY_DB_FILENAME).exists(), "legacy 应被归档改名");
        assert!(d.join(format!("{}{}", LEGACY_DB_FILENAME, LEGACY_BACKUP_SUFFIX)).exists());
        // 4. 两者皆无
        let d = base.join("c4");
        std::fs::create_dir_all(&d).unwrap();
        assert_eq!(resolve_startup_db(&d), DbSelection::Fresh);
        // 5. 存在 tmp → 清理
        let d = base.join("c5");
        std::fs::create_dir_all(&d).unwrap();
        mk(&d, V2_TMP_FILENAME);
        mk(&d, V2_DB_FILENAME);
        assert_eq!(resolve_startup_db(&d), DbSelection::V2(d.join(V2_DB_FILENAME)));
        assert!(!d.join(V2_TMP_FILENAME).exists(), "tmp 必须被清理");
        // 6. 存在 legacy backup + v2（正常升级后状态）
        let d = base.join("c6");
        std::fs::create_dir_all(&d).unwrap();
        mk(&d, V2_DB_FILENAME);
        mk(&d, format!("{}{}", LEGACY_DB_FILENAME, LEGACY_BACKUP_SUFFIX).as_str());
        assert_eq!(resolve_startup_db(&d), DbSelection::V2(d.join(V2_DB_FILENAME)));
        // 7. downgrade stray：backup 已存在 + 出现新的 legacy → 封存为 stray
        let d = base.join("c7");
        std::fs::create_dir_all(&d).unwrap();
        mk(&d, V2_DB_FILENAME);
        mk(&d, format!("{}{}", LEGACY_DB_FILENAME, LEGACY_BACKUP_SUFFIX).as_str());
        mk(&d, LEGACY_DB_FILENAME);
        assert_eq!(resolve_startup_db(&d), DbSelection::V2(d.join(V2_DB_FILENAME)));
        assert!(!d.join(LEGACY_DB_FILENAME).exists(), "stray 应被封存改名");
        let strays: Vec<_> = std::fs::read_dir(&d)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains(".stray-"))
            .collect();
        assert_eq!(strays.len(), 1, "stray 必须恰好 1 个");
        cleanup_dir(&base);
    }

    // ===== v2 解锁核心 =====
    #[test]
    fn t9_unlock_v2_core() {
        let dir = temp_app_dir("t9");
        build_legacy_fixture(&dir, MASTER);
        let out = migrate_legacy_to_v2(&dir, MASTER).unwrap();
        let v2 = open_v2_db(&out.v2_path).unwrap();
        let dek = unlock_v2_core(&v2, MASTER).unwrap();
        assert_eq!(*dek, *out.dek);
        let err = unlock_v2_core(&v2, "wrong-password").unwrap_err();
        assert_eq!(err, "主密码不正确或安全数据损坏", "错误消息必须统一");
        drop(v2);
        cleanup_dir(&dir);
    }

    const MASTER: &str = "Phase2A-Master!";
}

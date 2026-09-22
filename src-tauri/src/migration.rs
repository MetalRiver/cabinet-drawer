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
use rusqlite::{Connection, OpenFlags};
use std::path::{Path, PathBuf};
use zeroize::Zeroizing;

pub const V2_DB_FILENAME: &str = "drawer-v2.db";
pub const LEGACY_DB_FILENAME: &str = "drawer_box.db";
pub const LEGACY_BACKUP_SUFFIX: &str = ".legacy-v0.2.0";
pub const V2_TMP_FILENAME: &str = "drawer-v2.db.tmp";
pub const V2_SETUP_LOCK_FILENAME: &str = "drawer-v2.db.setup.lock";
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
        CREATE TABLE IF NOT EXISTS pinned_items (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            item_type TEXT NOT NULL,
            item_id INTEGER NOT NULL,
            sort_order INTEGER DEFAULT 0,
            UNIQUE(item_type, item_id)
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DbSelection {
    V2(PathBuf),
    Legacy(PathBuf),
    FreshV2(PathBuf),
    Blocked(&'static str),
}

/// FreshV2 启动阶段的占位库。只存在于内存，初始化完成前不会创建正式文件。
pub fn open_v2_db_in_memory() -> Result<Db, String> {
    let conn = rusqlite::Connection::open_in_memory().map_err(|e| e.to_string())?;
    init_v2_schema(&conn).map_err(|e| e.to_string())?;
    Ok(Db { conn: std::sync::Mutex::new(conn) })
}

/// 启动仲裁。只观察文件系统，绝不在启动时迁移、删除或重命名用户数据。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartupArbitration {
    pub selection: DbSelection,
    pub migration_required: bool,
    pub writes_allowed: bool,
    pub tmp_present: bool,
    pub legacy_backup_present: bool,
    pub stray_legacy_present: bool,
}

pub fn resolve_startup_db(app_dir: &Path, allow_fresh_v2: bool) -> StartupArbitration {
    let v2 = app_dir.join(V2_DB_FILENAME);
    let tmp = app_dir.join(V2_TMP_FILENAME);
    let legacy = app_dir.join(LEGACY_DB_FILENAME);
    let backup = app_dir.join(format!("{}{}", LEGACY_DB_FILENAME, LEGACY_BACKUP_SUFFIX));
    let stray_legacy_present = std::fs::read_dir(app_dir).ok().into_iter().flatten().filter_map(|entry| entry.ok()).any(|entry| entry.file_name().to_string_lossy().starts_with(&format!("{}.stray-", LEGACY_DB_FILENAME)));

    if tmp.exists() {
        return StartupArbitration { selection: DbSelection::Blocked("检测到未完成的 v2 迁移临时库"), migration_required: legacy.exists(), writes_allowed: false, tmp_present: true, legacy_backup_present: backup.exists(), stray_legacy_present };
    }

    if v2.exists() {
        return StartupArbitration { selection: DbSelection::V2(v2), migration_required: false, writes_allowed: true, tmp_present: tmp.exists(), legacy_backup_present: backup.exists(), stray_legacy_present };
    }
    if legacy.exists() {
        if backup.exists() {
            return StartupArbitration { selection: DbSelection::Blocked("legacy 与备份并存但 v2 不存在"), migration_required: true, writes_allowed: false, tmp_present: false, legacy_backup_present: true, stray_legacy_present };
        }
        return StartupArbitration { selection: DbSelection::Legacy(legacy), migration_required: true, writes_allowed: true, tmp_present: tmp.exists(), legacy_backup_present: backup.exists(), stray_legacy_present };
    }
    if backup.exists() || stray_legacy_present {
        return StartupArbitration { selection: DbSelection::Blocked("发现遗留数据库痕迹但正式数据库缺失"), migration_required: false, writes_allowed: false, tmp_present: false, legacy_backup_present: backup.exists(), stray_legacy_present };
    }
    if allow_fresh_v2 {
        StartupArbitration { selection: DbSelection::FreshV2(v2), migration_required: false, writes_allowed: true, tmp_present: false, legacy_backup_present: false, stray_legacy_present: false }
    } else {
        StartupArbitration { selection: DbSelection::Blocked("Phase 2A 不允许正常启动自动创建 v2 数据库"), migration_required: false, writes_allowed: false, tmp_present: false, legacy_backup_present: false, stray_legacy_present: false }
    }
}

/// 打开已经完成迁移的 v2 库。不开启 CREATE，也不补 schema；异常状态直接拒绝。
pub fn open_existing_v2_db(path: &Path) -> Result<Db, String> {
    let conn = rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE)
        .map_err(|_| "v2 数据库无法安全打开".to_string())?;
    let db = Db { conn: std::sync::Mutex::new(conn) };
    let version = db.get_setting("security_version").map_err(|_| "v2 安全元数据无效".to_string())?;
    if version.as_deref() != Some(SECURITY_VERSION_V2) {
        return Err("v2 安全元数据无效".to_string());
    }
    for key in ["kdf_params_m", "wrapped_dek_m", "kdf_params_r", "wrapped_dek_r"] {
        if db.get_setting(key).map_err(|_| "v2 安全元数据无效".to_string())?.filter(|v| !v.is_empty()).is_none() {
            return Err("v2 安全元数据无效".to_string());
        }
    }
    {
        let conn = db.conn.lock().map_err(|_| "v2 数据库无法安全打开".to_string())?;
        let integrity: String = conn.query_row("PRAGMA integrity_check", [], |row| row.get(0))
            .map_err(|_| "v2 数据库完整性校验失败".to_string())?;
        let uuid_columns: i64 = conn.query_row(
            "SELECT COUNT(*) FROM pragma_table_info('passwords') WHERE name='record_uuid'",
            [],
            |row| row.get(0),
        ).map_err(|_| "v2 数据库 schema 无效".to_string())?;
        if integrity != "ok" || uuid_columns != 1 {
            return Err("v2 数据库完整性或 schema 无效".to_string());
        }
    }
    Ok(db)
}

/// 新用户初始化故障注入点。仅由测试调用，生产入口始终传 None。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InitializationFailPoint {
    AfterSchema,
    AfterMasterWrap,
    BeforeRecoveryWrap,
    DuringMetadataWrite,
    BeforeRename,
}

/// 已完整校验、尚未正式生效的新用户初始化产物。
/// 恢复短语只通过此返回值交给一次性 UI 展示，不持久化。
pub struct PreparedInitialization {
    pub tmp_path: PathBuf,
    pub v2_path: PathBuf,
    pub dek: Zeroizing<Vec<u8>>,
    pub mnemonic: Vec<String>,
    pub setup_lock: std::fs::File,
    pub setup_lock_path: PathBuf,
}

pub fn prepare_fresh_v2(
    app_dir: &Path,
    master_password: &str,
) -> Result<PreparedInitialization, String> {
    prepare_fresh_v2_inject(app_dir, master_password, None)
}

/// FreshV2 准备阶段：只写同目录 tmp；正式库必须由确认阶段单独发布。
pub fn prepare_fresh_v2_inject(
    app_dir: &Path,
    master_password: &str,
    fail_at: Option<InitializationFailPoint>,
) -> Result<PreparedInitialization, String> {
    let v2_path = app_dir.join(V2_DB_FILENAME);
    let tmp_path = app_dir.join(V2_TMP_FILENAME);
    let setup_lock_path = app_dir.join(V2_SETUP_LOCK_FILENAME);
    let fail = |point| fail_at == Some(point);

    if resolve_startup_db(app_dir, true).selection != DbSelection::FreshV2(v2_path.clone()) {
        return Err("当前数据目录不允许新用户初始化".to_string());
    }

    let mut owns_setup_lock = false;
    let mut owns_tmp = false;
    let result = (|| -> Result<PreparedInitialization, String> {
        let mut lock_options = std::fs::OpenOptions::new();
        lock_options.write(true).create_new(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            lock_options.share_mode(0);
        }
        let setup_lock = lock_options
            .open(&setup_lock_path)
            .map_err(|_| "安全数据库初始化正在进行或存在未完成状态".to_string())?;
        owns_setup_lock = true;
        // create_new 是跨线程/进程的第二道门禁；已有 tmp 时绝不复用或覆盖。
        let reservation = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp_path)
            .map_err(|_| "安全数据库初始化正在进行或存在未完成状态".to_string())?;
        owns_tmp = true;
        drop(reservation);
        let v2 = open_v2_db(&tmp_path).map_err(|_| "无法创建安全数据库".to_string())?;
        if fail(InitializationFailPoint::AfterSchema) {
            return Err("injected:AfterSchema".to_string());
        }

        let dek = crypto::generate_dek();
        let salt_m = crypto::generate_salt();
        let params_m = crypto::default_kdf_params(crypto::b64_encode(&salt_m));
        let kek_m = crypto::derive_master_kek(master_password, &params_m);
        let wrapped_m = crypto::wrap_dek(&dek, &kek_m, crypto::AAD_WRAP_MASTER)
            .map_err(|_| "无法建立主密码保护".to_string())?;
        if fail(InitializationFailPoint::AfterMasterWrap) {
            return Err("injected:AfterMasterWrap".to_string());
        }

        let entropy = crypto::generate_recovery_entropy();
        let phrase = Zeroizing::new(
            crypto::entropy_to_mnemonic(&entropy)
                .map_err(|_| "无法生成恢复短语".to_string())?,
        );
        let salt_r = crypto::generate_salt();
        let params_r = crypto::KdfParams {
            algo: "hkdf-sha256".into(),
            version: 1,
            m_cost: 0,
            t_cost: 0,
            p_cost: 0,
            salt: crypto::b64_encode(&salt_r),
        };
        if fail(InitializationFailPoint::BeforeRecoveryWrap) {
            return Err("injected:BeforeRecoveryWrap".to_string());
        }
        let wrapped_r = crypto::wrap_dek_recovery(&dek, &entropy, &salt_r)
            .map_err(|_| "无法建立恢复保护".to_string())?;

        {
            let mut conn = v2.conn.lock().map_err(|_| "安全数据库不可用".to_string())?;
            let tx = conn.transaction().map_err(|_| "无法写入安全元数据".to_string())?;
            tx.execute(
                "INSERT INTO settings (key, value) VALUES ('kdf_params_m', ?1)",
                params![serde_json::to_string(&params_m).map_err(|_| "安全元数据无效".to_string())?],
            )
            .map_err(|_| "无法写入安全元数据".to_string())?;
            tx.execute(
                "INSERT INTO settings (key, value) VALUES ('wrapped_dek_m', ?1)",
                params![wrapped_m],
            )
            .map_err(|_| "无法写入安全元数据".to_string())?;
            if fail(InitializationFailPoint::DuringMetadataWrite) {
                return Err("injected:DuringMetadataWrite".to_string());
            }
            tx.execute(
                "INSERT INTO settings (key, value) VALUES ('kdf_params_r', ?1)",
                params![serde_json::to_string(&params_r).map_err(|_| "安全元数据无效".to_string())?],
            )
            .map_err(|_| "无法写入安全元数据".to_string())?;
            tx.execute(
                "INSERT INTO settings (key, value) VALUES ('wrapped_dek_r', ?1)",
                params![wrapped_r],
            )
            .map_err(|_| "无法写入安全元数据".to_string())?;
            // 提交标记必须最后写，并与其余安全元数据处于同一事务。
            tx.execute(
                "INSERT INTO settings (key, value) VALUES ('security_version', ?1)",
                params![SECURITY_VERSION_V2],
            )
            .map_err(|_| "无法写入安全元数据".to_string())?;
            tx.commit().map_err(|_| "无法提交安全元数据".to_string())?;
        }
        drop(v2);

        // 正式就位前同时验证 schema/integrity、Master 与 Recovery 两条解封路径。
        let verified = open_existing_v2_db(&tmp_path)?;
        let master_dek = unlock_v2_core(&verified, master_password)?;
        let recovery_dek = crypto::unwrap_dek_recovery(&wrapped_r, &entropy, &salt_r)
            .map_err(|_| "安全数据校验失败".to_string())?;
        if master_dek.as_slice() != dek.as_slice() || recovery_dek.as_slice() != dek.as_slice() {
            return Err("安全数据校验失败".to_string());
        }
        drop(verified);

        if fail(InitializationFailPoint::BeforeRename) {
            return Err("injected:BeforeRename".to_string());
        }
        Ok(PreparedInitialization {
            tmp_path: tmp_path.clone(),
            v2_path,
            dek,
            mnemonic: phrase.split_whitespace().map(String::from).collect(),
            setup_lock,
            setup_lock_path: setup_lock_path.clone(),
        })
    })();

    if result.is_err() && owns_tmp && tmp_path.exists() {
        let _ = std::fs::remove_file(&tmp_path);
    }
    if result.is_err() && owns_setup_lock && setup_lock_path.exists() {
        let _ = std::fs::remove_file(&setup_lock_path);
    }
    result
}

/// 用户确认已保存 Recovery Phrase 后，才把已校验的 tmp 原子发布为正式 v2。
pub fn finalize_prepared_v2(tmp_path: &Path, v2_path: &Path) -> Result<(), String> {
    if v2_path.exists() || !tmp_path.exists() {
        return Err("当前状态不允许完成安全数据库初始化".to_string());
    }
    let verified = open_existing_v2_db(tmp_path)?;
    drop(verified);
    // Windows 同目录 rename：目标存在时失败，不覆盖现有正式库。
    std::fs::rename(tmp_path, v2_path)
        .map_err(|_| "无法完成安全数据库初始化".to_string())
}

/// 仅清理“无 legacy、无正式 v2、无备份/stray”的中断新用户 setup tmp。
/// 迁移 tmp 或任何含真实数据痕迹的目录一律不碰。
pub fn discard_abandoned_fresh_initialization(app_dir: &Path) -> Result<bool, String> {
    let v2_path = app_dir.join(V2_DB_FILENAME);
    let legacy_path = app_dir.join(LEGACY_DB_FILENAME);
    let backup_path = app_dir.join(format!("{}{}", LEGACY_DB_FILENAME, LEGACY_BACKUP_SUFFIX));
    let arbitration = resolve_startup_db(app_dir, true);
    if v2_path.exists()
        || legacy_path.exists()
        || backup_path.exists()
        || arbitration.stray_legacy_present
        || !arbitration.tmp_present
    {
        return Ok(false);
    }

    let setup_lock_path = app_dir.join(V2_SETUP_LOCK_FILENAME);
    if setup_lock_path.exists() {
        // 活动初始化持有 Windows 独占句柄时删除会失败，此时必须 fail closed。
        std::fs::remove_file(&setup_lock_path)
            .map_err(|_| "检测到仍在进行的新用户初始化".to_string())?;
    }

    let mut removed = false;
    for path in [
        app_dir.join(V2_TMP_FILENAME),
        app_dir.join(format!("{}-wal", V2_TMP_FILENAME)),
        app_dir.join(format!("{}-shm", V2_TMP_FILENAME)),
        app_dir.join(format!("{}-journal", V2_TMP_FILENAME)),
    ] {
        if path.exists() {
            std::fs::remove_file(&path)
                .map_err(|_| "无法清理未完成的新用户初始化".to_string())?;
            removed = true;
        }
    }
    Ok(removed)
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
    DuringSettingsWrite,
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

/// 两阶段 legacy 升级的 Prepare 产物：完整校验、尚未正式生效的 pending v2 tmp。
/// 恢复短语只经此返回一次给 UI；正式 v2 与 legacy 均未被触碰。
pub struct PreparedLegacyMigration {
    pub tmp_path: PathBuf,
    pub v2_path: PathBuf,
    pub legacy_path: PathBuf,
    pub dek: Zeroizing<Vec<u8>>,
    pub mnemonic: Vec<String>,
    pub passwords_migrated: usize,
    /// legacy 源库 canonical 指纹；confirm 前必须复验一致才允许激活。
    pub source_fingerprint: [u8; 32],
}

/// 迁移入口（生产）
pub fn migrate_legacy_to_v2(app_dir: &Path, master_password: &str) -> Result<MigrationOutput, MigrationError> {
    migrate_legacy_to_v2_inject(app_dir, master_password, None)
}

/// legacy 源库 canonical 指纹：schema + 全部用户表全行，同一只读事务内的 consistent snapshot。
/// 不依赖 mtime / 文件字节（WAL checkpoint 等无关重排不影响结果）。
pub fn legacy_db_fingerprint(path: &Path) -> Result<[u8; 32], String> {
    use rusqlite::types::ValueRef;
    use sha2::{Digest, Sha256};
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|_| "无法只读打开 legacy 库".to_string())?;
    conn.pragma_update(None, "query_only", true)
        .map_err(|_| "无法启用只读保护".to_string())?;
    let mut digest = Sha256::new();
    let hash_value = |digest: &mut Sha256, value: ValueRef<'_>| {
        match value {
            ValueRef::Null => digest.update([0]),
            ValueRef::Integer(value) => {
                digest.update([1]);
                digest.update(value.to_le_bytes());
            }
            ValueRef::Real(value) => {
                digest.update([2]);
                digest.update(value.to_bits().to_le_bytes());
            }
            ValueRef::Text(value) => {
                digest.update([3]);
                digest.update((value.len() as u64).to_le_bytes());
                digest.update(value);
            }
            ValueRef::Blob(value) => {
                digest.update([4]);
                digest.update((value.len() as u64).to_le_bytes());
                digest.update(value);
            }
        };
    };
    // 单只读事务 = consistent snapshot；prepare/confirm 两次调用读到同一逻辑状态才可能同指纹。
    conn.execute_batch("BEGIN")
        .map_err(|_| "无法建立一致读事务".to_string())?;
    let fingerprint = (|| -> Result<[u8; 32], String> {
        {
            let mut stmt = conn
                .prepare(
                    "SELECT type,name,tbl_name,sql FROM sqlite_master
                     WHERE name NOT LIKE 'sqlite_%' ORDER BY type,name",
                )
                .map_err(|_| "无法读取 legacy schema".to_string())?;
            let mut rows = stmt.query([]).map_err(|_| "无法读取 legacy schema".to_string())?;
            while let Some(row) = rows.next().map_err(|_| "无法读取 legacy schema".to_string())? {
                digest.update([0xfa]);
                for column in 0..4 {
                    let value = row.get_ref(column).map_err(|_| "无法读取 legacy schema".to_string())?;
                    hash_value(&mut digest, value);
                }
            }
        }
        let mut names = conn
            .prepare(
                "SELECT name FROM sqlite_master
                 WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
            )
            .map_err(|_| "无法读取 legacy 表清单".to_string())?;
        let names = names
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|_| "无法读取 legacy 表清单".to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| "无法读取 legacy 表清单".to_string())?;
        for name in names {
            let quoted = format!("\"{}\"", name.replace('"', "\"\""));
            let mut stmt = conn
                .prepare(&format!("SELECT * FROM {quoted} ORDER BY rowid"))
                .map_err(|_| "无法读取 legacy 数据行".to_string())?;
            let columns = stmt.column_count();
            let mut rows = stmt.query([]).map_err(|_| "无法读取 legacy 数据行".to_string())?;
            while let Some(row) = rows.next().map_err(|_| "无法读取 legacy 数据行".to_string())? {
                digest.update([0xfe]);
                for column in 0..columns {
                    let value = row
                        .get_ref(column)
                        .map_err(|_| "无法读取 legacy 数据行".to_string())?;
                    hash_value(&mut digest, value);
                }
            }
        }
        Ok(digest.finalize().into())
    })();
    conn.execute_batch("COMMIT")
        .map_err(|_| "无法结束一致读事务".to_string())?;
    fingerprint
}

/// 迁移引擎（T1-T8；fail_at 仅供测试注入失败点）
pub fn migrate_legacy_to_v2_inject(
    app_dir: &Path,
    master_password: &str,
    fail_at: Option<MigrationFailPoint>,
) -> Result<MigrationOutput, MigrationError> {
    let fail = |fp: MigrationFailPoint| fail_at == Some(fp);

    let legacy_path = app_dir.join(LEGACY_DB_FILENAME);
    let v2_path = app_dir.join(V2_DB_FILENAME);

    // 注入点 1：创建 tmp 前失败
    if fail(MigrationFailPoint::BeforeTmpCreate) {
        return Err("injected:BeforeTmpCreate".into());
    }
    if !legacy_path.exists() {
        return Err("legacy 库不存在".into());
    }
    if v2_path.exists() {
        return Err("v2 库已存在，拒绝覆盖".into());
    }
    if app_dir.join(V2_TMP_FILENAME).exists() {
        return Err("检测到未完成的 v2 迁移临时库".into());
    }

    let built = build_migrated_v2_tmp(app_dir, master_password, fail_at)?;

    // 注入点 6：rename 前失败
    if fail(MigrationFailPoint::BeforeRename) {
        return Err("injected:BeforeRename".into());
    }

    // T9 atomic rename → v2 正式活动库
    std::fs::rename(&built.tmp_path, &v2_path).map_err(|e| format!("rename 失败: {}", e))?;

    // T10 legacy 隔离归档
    archive_legacy_source(app_dir)?;

    Ok(MigrationOutput {
        v2_path,
        dek: built.dek,
        mnemonic: built.mnemonic,
        passwords_migrated: built.passwords_migrated,
    })
}

/// legacy 源库隔离归档（迁移成功后调用；归档失败属于可见错误，调用方决定语义）。
pub fn archive_legacy_source(app_dir: &Path) -> Result<(), MigrationError> {
    let legacy_path = app_dir.join(LEGACY_DB_FILENAME);
    let legacy_backup = app_dir.join(format!("{}{}", LEGACY_DB_FILENAME, LEGACY_BACKUP_SUFFIX));
    if legacy_path.exists() {
        std::fs::rename(&legacy_path, &legacy_backup)
            .map_err(|e| format!("legacy 归档失败: {}", e))?;
    }
    Ok(())
}

/// Prepare 阶段核心（T2-T8）：验证旧主密码 → 严格只读 legacy → 构建 pending v2 tmp。
/// 绝不创建正式 v2、绝不改写 legacy、绝不归档 legacy。
/// 失败时可能留下未完成 tmp，由调用方负责清理。
pub fn build_migrated_v2_tmp(
    app_dir: &Path,
    master_password: &str,
    fail_at: Option<MigrationFailPoint>,
) -> Result<PreparedLegacyMigration, MigrationError> {
    let fail = |fp: MigrationFailPoint| fail_at == Some(fp);

    let legacy_path = app_dir.join(LEGACY_DB_FILENAME);
    let tmp_path = app_dir.join(V2_TMP_FILENAME);
    let v2_path = app_dir.join(V2_DB_FILENAME);

    if !legacy_path.exists() {
        return Err("legacy 库不存在".into());
    }
    if v2_path.exists() {
        return Err("v2 库已存在，拒绝覆盖".into());
    }
    if tmp_path.exists() {
        return Err("检测到未完成的 v2 迁移临时库".into());
    }

    // T2 legacy 打开（只读用途）+ 旧 verifier 校验（迁移必须用正确旧主密码）
    let legacy = Db::open_read_only(&legacy_path)
        .map_err(|e| format!("legacy 库打开失败: {}", e))?;
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
            let plain = Zeroizing::new(
                crypto::decrypt(&enc, &legacy_key)
                    .map_err(|_| format!("legacy 行 id={} 解密失败，迁移中止", id))?,
            );
            let uuid = uuid::Uuid::new_v4().to_string();
            let dw2 = crypto::encrypt_password_dw2(&plain, &dek, &uuid).map_err(|e| e)?;
            tx.execute(
                "INSERT INTO passwords (id, record_uuid, title, username, password, url, notes,
                                        created_at, updated_at, use_count, last_used_at, deleted_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
                params![id, uuid, title, username, dw2, url, notes, created, updated, usec, lused, deleted],
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
        // pinned_items 的 item_id 依赖各业务表原始 id，因此 password id 也必须保留。
        let pinned: Vec<(i64, String, i64, i64)> = {
            let mut st = lconn.prepare(
                "SELECT id, item_type, item_id, sort_order FROM pinned_items ORDER BY id",
            )?;
            let rows = st
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            rows
        };
        for (id, item_type, item_id, sort_order) in pinned {
            tx.execute(
                "INSERT INTO pinned_items (id, item_type, item_id, sort_order)
                 VALUES (?1,?2,?3,?4)",
                params![id, item_type, item_id, sort_order],
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
        let mut vconn = v2.conn.lock().map_err(|e| e.to_string())?;
        let tx = vconn.transaction().map_err(|e| e.to_string())?;
        tx.execute(
                "INSERT OR REPLACE INTO settings (key, value) VALUES ('kdf_params_m', ?1)",
                params![serde_json::to_string(&params_m).map_err(|e| e.to_string())?],
            )
            .map_err(|e| e.to_string())?;
        tx.execute("INSERT OR REPLACE INTO settings (key, value) VALUES ('wrapped_dek_m', ?1)", params![wrapped_m])
            .map_err(|e| e.to_string())?;
        if fail(MigrationFailPoint::DuringSettingsWrite) {
            return Err("injected:DuringSettingsWrite".into());
        }
        tx.execute(
                "INSERT OR REPLACE INTO settings (key, value) VALUES ('kdf_params_r', ?1)",
                params![serde_json::to_string(&crypto::KdfParams {
                    algo: "hkdf-sha256".into(),
                    version: 1,
                    m_cost: 0,
                    t_cost: 0,
                    p_cost: 0,
                    salt: crypto::b64_encode(&salt_r),
                })
                .map_err(|e| e.to_string())?],
            )
            .map_err(|e| e.to_string())?;
        tx.execute("INSERT OR REPLACE INTO settings (key, value) VALUES ('wrapped_dek_r', ?1)", params![wrapped_r])
            .map_err(|e| e.to_string())?;
        // security_version 必须最后写（迁移状态机的提交标记）
        tx.execute(
                "INSERT OR REPLACE INTO settings (key, value) VALUES ('security_version', ?1)",
                params![SECURITY_VERSION_V2],
            )
            .map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
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

    // 关闭连接（指纹计算与调用方激活前，必须全部释放句柄）
    drop(v2);
    drop(legacy);

    // Prepare 完成前建立 legacy 源库一致快照指纹；confirm 激活前必须复验一致。
    let source_fingerprint = legacy_db_fingerprint(&legacy_path)?;

    Ok(PreparedLegacyMigration {
        tmp_path,
        v2_path,
        legacy_path,
        dek,
        mnemonic: phrase.split_whitespace().map(String::from).collect(),
        passwords_migrated: migrated,
        source_fingerprint,
    })
}

/// Confirm 阶段的持久化提交点：把已校验的 pending v2 tmp 原子发布为正式 v2。
/// 此函数返回后，升级即已持久化完成——即使进程立刻崩溃，下次启动也能通过
/// 正常 Master unlock 进入 v2（不依赖任何丢失的内存 pending 状态）。
/// legacy 隔离归档由调用方在替换 AppState 连接后单独执行（Windows 下旧连接
/// 未释放时 rename 会失败）。
pub fn activate_migrated_v2(app_dir: &Path) -> Result<Db, String> {
    let tmp_path = app_dir.join(V2_TMP_FILENAME);
    let v2_path = app_dir.join(V2_DB_FILENAME);
    if v2_path.exists() {
        return Err("v2 库已存在，拒绝覆盖".to_string());
    }
    if !tmp_path.exists() {
        return Err("升级临时库不存在".to_string());
    }
    let verified = open_existing_v2_db(&tmp_path)?;
    drop(verified);
    // Windows 同目录 rename：目标存在时失败，不覆盖现有正式库。
    std::fs::rename(&tmp_path, &v2_path).map_err(|_| "无法完成安全升级".to_string())?;
    open_existing_v2_db(&v2_path)
}

/// 仅清理「legacy 存在、无正式 v2、无备份」的中断升级 orphan tmp。
/// 任何活动升级持有的 setup lock 带独占句柄时删除会失败 → fail closed。
/// 目标：orphan tmp 永远不会让用户失去再次发起升级的能力。
pub fn discard_abandoned_migration_tmp(app_dir: &Path) -> Result<bool, String> {
    let v2_path = app_dir.join(V2_DB_FILENAME);
    let legacy_path = app_dir.join(LEGACY_DB_FILENAME);
    let backup_path = app_dir.join(format!("{}{}", LEGACY_DB_FILENAME, LEGACY_BACKUP_SUFFIX));
    let tmp_path = app_dir.join(V2_TMP_FILENAME);
    if !tmp_path.exists() || v2_path.exists() || !legacy_path.exists() || backup_path.exists() {
        return Ok(false);
    }
    let setup_lock_path = app_dir.join(V2_SETUP_LOCK_FILENAME);
    if setup_lock_path.exists() {
        std::fs::remove_file(&setup_lock_path)
            .map_err(|_| "检测到仍在进行的安全升级".to_string())?;
    }
    let mut removed = false;
    for path in [
        tmp_path,
        app_dir.join(format!("{}-wal", V2_TMP_FILENAME)),
        app_dir.join(format!("{}-shm", V2_TMP_FILENAME)),
        app_dir.join(format!("{}-journal", V2_TMP_FILENAME)),
    ] {
        if path.exists() {
            std::fs::remove_file(&path).map_err(|_| "无法清理未完成的安全升级".to_string())?;
            removed = true;
        }
    }
    Ok(removed)
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
    if params.algo != "argon2id"
        || params.version != 1
        || params.m_cost != 19456
        || params.t_cost != 2
        || params.p_cost != 1
    {
        return Err("主密码不正确或安全数据损坏".to_string());
    }
    let wrapped = v2
        .get_setting("wrapped_dek_m")
        .map_err(|e| e.to_string())?
        .ok_or("安全数据缺失：wrapped_dek_m")?;
    let kek = crypto::derive_master_kek(password, &params);
    crypto::unwrap_dek(&wrapped, &kek, crypto::AAD_WRAP_MASTER)
        .map_err(|_| "主密码不正确或安全数据损坏".to_string())
}

/// Recovery Phrase 认证核心：仅解封现有 Stable DEK，不修改数据库或 AppState。
/// 所有词数、词表、checksum、metadata 与 AEAD 认证失败统一为同一用户级错误。
pub fn recover_v2_core(v2: &Db, recovery_phrase: &str) -> Result<Zeroizing<Vec<u8>>, String> {
    let invalid = || "恢复短语无效".to_string();
    if v2
        .get_setting("security_version")
        .map_err(|_| invalid())?
        .as_deref()
        != Some(SECURITY_VERSION_V2)
    {
        return Err(invalid());
    }

    let params_json = v2
        .get_setting("kdf_params_r")
        .map_err(|_| invalid())?
        .ok_or_else(invalid)?;
    let params: crypto::KdfParams = serde_json::from_str(&params_json).map_err(|_| invalid())?;
    if params.algo != "hkdf-sha256"
        || params.version != 1
        || params.m_cost != 0
        || params.t_cost != 0
        || params.p_cost != 0
    {
        return Err(invalid());
    }
    let salt = Zeroizing::new(crypto::b64_decode(&params.salt).map_err(|_| invalid())?);
    if salt.len() < 8 {
        return Err(invalid());
    }
    let wrapped = v2
        .get_setting("wrapped_dek_r")
        .map_err(|_| invalid())?
        .filter(|value| !value.is_empty())
        .ok_or_else(invalid)?;
    let entropy = crypto::mnemonic_to_entropy(recovery_phrase).map_err(|_| invalid())?;
    crypto::unwrap_dek_recovery(&wrapped, &entropy, &salt).map_err(|_| invalid())
}

/// Recovery rotation 的内存准备结果。mnemonic 仅用于一次性返回 UI，不写数据库。
pub struct PreparedRecoveryRotation {
    pub params_r_json: String,
    pub wrapped_dek_r: String,
    pub mnemonic: Zeroizing<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryRotationFailPoint {
    AfterNewSalt,
    AfterRecoveryKek,
    AfterWrappedDek,
    DuringMetadataWrite,
    BeforeCommit,
}

pub fn prepare_recovery_rotation(
    dek: &[u8],
) -> Result<PreparedRecoveryRotation, String> {
    prepare_recovery_rotation_inject(dek, None)
}

pub fn prepare_recovery_rotation_inject(
    dek: &[u8],
    fail_at: Option<RecoveryRotationFailPoint>,
) -> Result<PreparedRecoveryRotation, String> {
    let fail = |point| fail_at == Some(point);
    let entropy = crypto::generate_recovery_entropy();
    let mnemonic = Zeroizing::new(
        crypto::entropy_to_mnemonic(&entropy)
            .map_err(|_| "无法生成新的恢复短语".to_string())?,
    );
    let salt_r = Zeroizing::new(crypto::generate_salt());
    if fail(RecoveryRotationFailPoint::AfterNewSalt) {
        return Err("injected:AfterNewSalt".to_string());
    }
    let params_r = crypto::KdfParams {
        algo: "hkdf-sha256".into(),
        version: 1,
        m_cost: 0,
        t_cost: 0,
        p_cost: 0,
        salt: crypto::b64_encode(&salt_r),
    };
    let kek_r = crypto::derive_recovery_kek(&entropy, &salt_r);
    if fail(RecoveryRotationFailPoint::AfterRecoveryKek) {
        return Err("injected:AfterRecoveryKek".to_string());
    }
    let wrapped_dek_r = crypto::wrap_dek(dek, &kek_r, crypto::AAD_WRAP_RECOVERY)
        .map_err(|_| "无法建立新的恢复保护".to_string())?;
    if fail(RecoveryRotationFailPoint::AfterWrappedDek) {
        return Err("injected:AfterWrappedDek".to_string());
    }
    let params_r_json = serde_json::to_string(&params_r)
        .map_err(|_| "无法建立新的恢复保护".to_string())?;
    Ok(PreparedRecoveryRotation {
        params_r_json,
        wrapped_dek_r,
        mnemonic,
    })
}

/// 原子提交新的 Recovery metadata。调用方必须先完成 pending token 与三词确认校验。
pub fn commit_recovery_rotation(
    v2: &Db,
    params_r_json: &str,
    wrapped_dek_r: &str,
) -> Result<(), String> {
    commit_recovery_rotation_inject(v2, params_r_json, wrapped_dek_r, None)
}

pub fn commit_recovery_rotation_inject(
    v2: &Db,
    params_r_json: &str,
    wrapped_dek_r: &str,
    fail_at: Option<RecoveryRotationFailPoint>,
) -> Result<(), String> {
    let fail = |point| fail_at == Some(point);
    let mut conn = v2
        .conn
        .lock()
        .map_err(|_| "安全数据库不可用".to_string())?;
    let tx = conn
        .transaction()
        .map_err(|_| "无法更新恢复保护".to_string())?;
    let params_updated = tx
        .execute(
            "UPDATE settings SET value = ?1 WHERE key = 'kdf_params_r'",
            params![params_r_json],
        )
        .map_err(|_| "无法更新恢复保护".to_string())?;
    if params_updated != 1 {
        return Err("无法更新恢复保护".to_string());
    }
    if fail(RecoveryRotationFailPoint::DuringMetadataWrite) {
        return Err("injected:DuringMetadataWrite".to_string());
    }
    let wrap_updated = tx
        .execute(
            "UPDATE settings SET value = ?1 WHERE key = 'wrapped_dek_r'",
            params![wrapped_dek_r],
        )
        .map_err(|_| "无法更新恢复保护".to_string())?;
    if wrap_updated != 1 {
        return Err("无法更新恢复保护".to_string());
    }
    if fail(RecoveryRotationFailPoint::BeforeCommit) {
        return Err("injected:BeforeCommit".to_string());
    }
    tx.commit()
        .map_err(|_| "无法更新恢复保护".to_string())?;
    Ok(())
}

/// 主密码重包故障注入点。生产入口始终传 None。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MasterRewrapFailPoint {
    AfterNewSalt,
    AfterMasterKek,
    AfterWrappedDek,
    DuringMetadataWrite,
    BeforeCommit,
}

/// Recovery 认证成功后，用新主密码重新包裹同一个 Stable DEK。
/// 只原子更新 master KDF metadata 与 wrapped_dek_m；Recovery metadata 与密码行均不触碰。
pub fn recover_v2_with_phrase(
    v2: &Db,
    recovery_phrase: &str,
    new_master_password: &str,
) -> Result<Zeroizing<Vec<u8>>, String> {
    recover_v2_with_phrase_inject(v2, recovery_phrase, new_master_password, None)
}

pub fn recover_v2_with_phrase_inject(
    v2: &Db,
    recovery_phrase: &str,
    new_master_password: &str,
    fail_at: Option<MasterRewrapFailPoint>,
) -> Result<Zeroizing<Vec<u8>>, String> {
    if new_master_password.len() < 6 {
        return Err("新主密码长度至少 6 位".to_string());
    }
    let dek = recover_v2_core(v2, recovery_phrase)?;
    rewrap_master_dek_inject(v2, dek.as_slice(), new_master_password, fail_at)?;
    Ok(dek)
}

/// 已知旧主密码时的 v2 普通改密。旧 wrap 必须真实解封，且结果必须等于当前活动 DEK。
pub fn change_v2_master_password(
    v2: &Db,
    current_password: &str,
    new_master_password: &str,
    active_dek: &[u8],
) -> Result<(), String> {
    change_v2_master_password_inject(
        v2,
        current_password,
        new_master_password,
        active_dek,
        None,
    )
}

pub fn change_v2_master_password_inject(
    v2: &Db,
    current_password: &str,
    new_master_password: &str,
    active_dek: &[u8],
    fail_at: Option<MasterRewrapFailPoint>,
) -> Result<(), String> {
    if new_master_password.len() < 6 {
        return Err("新主密码长度至少 6 位".to_string());
    }
    if new_master_password == current_password {
        return Err("新主密码必须与当前主密码不同".to_string());
    }
    let verified_dek = unlock_v2_core(v2, current_password)
        .map_err(|_| "当前主密码错误".to_string())?;
    if verified_dek.as_slice() != active_dek {
        return Err("活动密钥与安全数据库不一致".to_string());
    }
    rewrap_master_dek_inject(v2, verified_dek.as_slice(), new_master_password, fail_at)
}

/// 共用的 master-wrap 原子更新。调用方必须先完成 Master 或 Recovery 身份验证。
fn rewrap_master_dek_inject(
    v2: &Db,
    dek: &[u8],
    new_master_password: &str,
    fail_at: Option<MasterRewrapFailPoint>,
) -> Result<(), String> {
    let fail = |point| fail_at == Some(point);
    let salt_m = Zeroizing::new(crypto::generate_salt());
    if fail(MasterRewrapFailPoint::AfterNewSalt) {
        return Err("injected:AfterNewSalt".to_string());
    }
    let params_m = crypto::default_kdf_params(crypto::b64_encode(&salt_m));
    let kek_m = crypto::derive_master_kek(new_master_password, &params_m);
    if fail(MasterRewrapFailPoint::AfterMasterKek) {
        return Err("injected:AfterMasterKek".to_string());
    }
    let wrapped_m = crypto::wrap_dek(dek, &kek_m, crypto::AAD_WRAP_MASTER)
        .map_err(|_| "无法设置新主密码".to_string())?;
    if fail(MasterRewrapFailPoint::AfterWrappedDek) {
        return Err("injected:AfterWrappedDek".to_string());
    }
    let params_json = serde_json::to_string(&params_m)
        .map_err(|_| "无法设置新主密码".to_string())?;

    let mut conn = v2
        .conn
        .lock()
        .map_err(|_| "安全数据库不可用".to_string())?;
    let tx = conn
        .transaction()
        .map_err(|_| "无法设置新主密码".to_string())?;
    let params_updated = tx
        .execute(
            "UPDATE settings SET value = ?1 WHERE key = 'kdf_params_m'",
            params![params_json],
        )
        .map_err(|_| "无法设置新主密码".to_string())?;
    if params_updated != 1 {
        return Err("无法设置新主密码".to_string());
    }
    if fail(MasterRewrapFailPoint::DuringMetadataWrite) {
        return Err("injected:DuringMetadataWrite".to_string());
    }
    let wrap_updated = tx
        .execute(
            "UPDATE settings SET value = ?1 WHERE key = 'wrapped_dek_m'",
            params![wrapped_m],
        )
        .map_err(|_| "无法设置新主密码".to_string())?;
    if wrap_updated != 1 {
        return Err("无法设置新主密码".to_string());
    }
    if fail(MasterRewrapFailPoint::BeforeCommit) {
        return Err("injected:BeforeCommit".to_string());
    }
    tx.commit()
        .map_err(|_| "无法设置新主密码".to_string())?;
    Ok(())
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
        // 制造真实常见的自增 id 缺口，确保迁移不会重排 password id。
        db.conn
            .lock()
            .unwrap()
            .execute("DELETE FROM passwords WHERE id=2", [])
            .unwrap();
        titles.retain(|title| title != "Legacy-PW-1");
        let replacement_title = "Legacy-PW-3".to_string();
        let replacement = crypto::encrypt("legacy-secret-3", &key).unwrap();
        db.create_password(
            &replacement_title,
            "fixture-user",
            &replacement,
            "https://fixture.test",
            "n",
        )
        .unwrap();
        titles.push(replacement_title);
        db.conn
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO pinned_items (item_type,item_id,sort_order) VALUES ('password',1,7)",
                [],
            )
            .unwrap();
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
        let ids: Vec<i64> = conn
            .prepare("SELECT id FROM passwords ORDER BY id")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(ids, vec![1, 3, 4], "password id 缺口必须原样保留");
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
        for (t, n) in [
            ("apps", 1),
            ("app_categories", 4),
            ("snippets", 1),
            ("temp_contents", 1),
            ("pinned_items", 1),
        ] {
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
        assert_eq!(
            *recover_v2_core(&v2, &out.mnemonic.join(" ")).unwrap(),
            *out.dek,
            "迁移产物必须通过正式 Recovery 解锁路径",
        );
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
            MigrationFailPoint::DuringSettingsWrite,
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
            // 启动仲裁只观察：tmp 保留给后续明确的恢复流程，仍识别为 Legacy。
            let sel = resolve_startup_db(&dir, false);
            if dir.join(V2_TMP_FILENAME).exists() {
                assert!(matches!(sel.selection, DbSelection::Blocked(_)), "注入点 {:?}: tmp 状态必须 fail closed", fp);
                assert!(!sel.writes_allowed);
            } else {
                assert_eq!(sel.selection, DbSelection::Legacy(dir.join(LEGACY_DB_FILENAME)), "注入点 {:?}", fp);
            }
            assert_eq!(sel.tmp_present, dir.join(V2_TMP_FILENAME).exists(), "注入点 {:?}: tmp 状态必须如实报告", fp);
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
        let a = resolve_startup_db(&d, false); assert_eq!(a.selection, DbSelection::Legacy(d.join(LEGACY_DB_FILENAME))); assert!(a.migration_required); assert!(!d.join(V2_DB_FILENAME).exists());
        // 2. 只有 v2
        let d = base.join("c2");
        std::fs::create_dir_all(&d).unwrap();
        mk(&d, V2_DB_FILENAME);
        assert_eq!(resolve_startup_db(&d, false).selection, DbSelection::V2(d.join(V2_DB_FILENAME)));
        // 3. 两者都有 → v2 胜出，legacy 保留且绝不被启动路径改写
        let d = base.join("c3");
        std::fs::create_dir_all(&d).unwrap();
        mk(&d, LEGACY_DB_FILENAME);
        mk(&d, V2_DB_FILENAME);
        assert_eq!(resolve_startup_db(&d, false).selection, DbSelection::V2(d.join(V2_DB_FILENAME)));
        assert!(d.join(LEGACY_DB_FILENAME).exists(), "legacy 必须保留");
        // 4. 两者皆无
        let d = base.join("c4");
        std::fs::create_dir_all(&d).unwrap();
        assert!(matches!(resolve_startup_db(&d, false).selection, DbSelection::Blocked(_)));
        assert_eq!(resolve_startup_db(&d, true).selection, DbSelection::FreshV2(d.join(V2_DB_FILENAME)));
        assert!(!d.join(V2_DB_FILENAME).exists(), "仲裁本身不得创建数据库");
        // 5. 存在 tmp → 不可作为 active DB，也不在启动时删除
        let d = base.join("c5");
        std::fs::create_dir_all(&d).unwrap();
        mk(&d, V2_TMP_FILENAME);
        mk(&d, V2_DB_FILENAME);
        let a = resolve_startup_db(&d, false); assert!(matches!(a.selection, DbSelection::Blocked(_))); assert!(a.tmp_present); assert!(!a.writes_allowed);
        assert!(d.join(V2_TMP_FILENAME).exists(), "tmp 必须保留供明确恢复流程处理");
        // 6. 存在 legacy backup + v2（正常升级后状态）
        let d = base.join("c6");
        std::fs::create_dir_all(&d).unwrap();
        mk(&d, V2_DB_FILENAME);
        mk(&d, format!("{}{}", LEGACY_DB_FILENAME, LEGACY_BACKUP_SUFFIX).as_str());
        assert_eq!(resolve_startup_db(&d, false).selection, DbSelection::V2(d.join(V2_DB_FILENAME)));
        // 7. stray 被显式报告，仲裁不重命名任何文件
        let d = base.join("c7");
        std::fs::create_dir_all(&d).unwrap();
        mk(&d, V2_DB_FILENAME);
        mk(&d, format!("{}{}", LEGACY_DB_FILENAME, LEGACY_BACKUP_SUFFIX).as_str());
        mk(&d, LEGACY_DB_FILENAME);
        mk(&d, format!("{}.stray-fixture", LEGACY_DB_FILENAME).as_str());
        let a = resolve_startup_db(&d, false);
        assert_eq!(a.selection, DbSelection::V2(d.join(V2_DB_FILENAME)));
        assert!(d.join(LEGACY_DB_FILENAME).exists(), "legacy 不得被启动仲裁改写");
        assert!(a.stray_legacy_present);

        // 8. backup + legacy 但无 v2：状态矛盾，禁止猜测 active DB。
        let d = base.join("c8");
        std::fs::create_dir_all(&d).unwrap();
        mk(&d, LEGACY_DB_FILENAME);
        mk(&d, format!("{}{}", LEGACY_DB_FILENAME, LEGACY_BACKUP_SUFFIX).as_str());
        let a = resolve_startup_db(&d, false);
        assert!(matches!(a.selection, DbSelection::Blocked(_)));
        assert!(!a.writes_allowed);

        // 9. 仅 tmp：不得把 tmp 当正式库，也不得顺手创建空 v2。
        let d = base.join("c9");
        std::fs::create_dir_all(&d).unwrap();
        mk(&d, V2_TMP_FILENAME);
        let a = resolve_startup_db(&d, true);
        assert!(matches!(a.selection, DbSelection::Blocked(_)));
        assert!(!d.join(V2_DB_FILENAME).exists());
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

    #[test]
    fn i1_i7_v2_unlock_stores_expected_stable_dek() {
        let dir = temp_app_dir("i1-i7");
        build_legacy_fixture(&dir, MASTER);
        let output = migrate_legacy_to_v2(&dir, MASTER).unwrap();
        let expected = output.dek.to_vec();
        let state = crate::AppState {
            db: std::sync::Mutex::new(open_existing_v2_db(&output.v2_path).unwrap()),
            db_path: output.v2_path.clone(),
            startup_mode: std::sync::Mutex::new(crate::StartupMode::ExistingV2),
            pending_v2: std::sync::Mutex::new(None),
            pending_recovery_rotation: std::sync::Mutex::new(None),
            pending_legacy_migration: std::sync::Mutex::new(None),
            master_wrap_gate: std::sync::Mutex::new(()),
            key: std::sync::Mutex::new(None),
        };
        let hash_before = file_hash(&output.v2_path);
        assert!(state.unlock_v2_and_store("wrong-password").is_err());
        assert!(state.stable_dek().is_err(), "错误密码不得污染 AppState");
        assert_eq!(file_hash(&output.v2_path), hash_before, "错误密码不得修改 v2 DB");
        state.unlock_v2_and_store(MASTER).unwrap();
        assert_eq!(state.stable_dek().unwrap().to_vec(), expected);
        assert!(state.legacy_key().is_err());
        state.clear_key();
        drop(state);
        cleanup_dir(&dir);
    }

    fn fresh_v2_state(dir: &Path) -> crate::AppState {
        crate::AppState {
            db: std::sync::Mutex::new(open_v2_db_in_memory().unwrap()),
            db_path: dir.join(V2_DB_FILENAME),
            startup_mode: std::sync::Mutex::new(crate::StartupMode::FreshV2),
            pending_v2: std::sync::Mutex::new(None),
            pending_recovery_rotation: std::sync::Mutex::new(None),
            pending_legacy_migration: std::sync::Mutex::new(None),
            master_wrap_gate: std::sync::Mutex::new(()),
            key: std::sync::Mutex::new(None),
        }
    }

    // ===== Phase 2B.1：新用户初始化 + Recovery 首次建立 =====
    #[test]
    fn n1_fresh_initialization_writes_complete_metadata_and_stores_dek() {
        let dir = temp_app_dir("n1");
        let state = fresh_v2_state(&dir);

        let words = state.prepare_v2_initialization(MASTER).unwrap();
        assert_eq!(words.len(), 12);
        assert!(!dir.join(V2_DB_FILENAME).exists());
        assert!(dir.join(V2_TMP_FILENAME).exists());
        assert!(state.stable_dek().is_err(), "确认恢复词前不得激活 DEK");
        state.finalize_v2_initialization().unwrap();
        assert!(dir.join(V2_DB_FILENAME).exists());
        assert!(!dir.join(V2_TMP_FILENAME).exists());

        let db = open_existing_v2_db(&dir.join(V2_DB_FILENAME)).unwrap();
        for key in [
            "kdf_params_m",
            "wrapped_dek_m",
            "kdf_params_r",
            "wrapped_dek_r",
            "security_version",
        ] {
            assert!(db.get_setting(key).unwrap().filter(|v| !v.is_empty()).is_some());
        }
        assert!(db.get_setting("recovery_phrase").unwrap().is_none());
        assert!(db.get_setting("recovery_phrase_encrypted").unwrap().is_none());

        let unlocked = unlock_v2_core(&db, MASTER).unwrap();
        assert_eq!(state.stable_dek().unwrap().as_slice(), unlocked.as_slice());
        cleanup_dir(&dir);
    }

    #[test]
    fn n2_returns_valid_bip39_words_but_never_persists_phrase() {
        let dir = temp_app_dir("n2");
        let state = fresh_v2_state(&dir);
        let words = state.prepare_v2_initialization(MASTER).unwrap();
        let phrase = words.join(" ");

        assert_eq!(words.len(), 12);
        assert_eq!(crypto::mnemonic_to_entropy(&phrase).unwrap().len(), 16);
        let bytes = std::fs::read(dir.join(V2_TMP_FILENAME)).unwrap();
        assert!(
            !bytes.windows(phrase.len()).any(|window| window == phrase.as_bytes()),
            "恢复短语不得以明文写入数据库文件"
        );
        let db = open_existing_v2_db(&dir.join(V2_TMP_FILENAME)).unwrap();
        let settings: String = db
            .conn
            .lock()
            .unwrap()
            .query_row("SELECT COALESCE(GROUP_CONCAT(value, ''), '') FROM settings", [], |r| r.get(0))
            .unwrap();
        assert!(!settings.contains(&phrase));
        drop(db);
        state.finalize_v2_initialization().unwrap();
        cleanup_dir(&dir);
    }

    #[test]
    fn n3_master_unlock_recovers_same_dek_and_wrong_password_does_not_pollute_state() {
        let dir = temp_app_dir("n3");
        let state = fresh_v2_state(&dir);
        state.prepare_v2_initialization(MASTER).unwrap();
        state.finalize_v2_initialization().unwrap();
        let expected = state.stable_dek().unwrap().to_vec();
        state.clear_key();

        assert!(state.unlock_v2_and_store("wrong-password").is_err());
        assert!(!state.is_unlocked());
        state.unlock_v2_and_store(MASTER).unwrap();
        assert_eq!(state.stable_dek().unwrap().to_vec(), expected);
        cleanup_dir(&dir);
    }

    #[test]
    fn n4_recovery_words_unwrap_the_same_stable_dek() {
        let dir = temp_app_dir("n4");
        let state = fresh_v2_state(&dir);
        let words = state.prepare_v2_initialization(MASTER).unwrap();
        state.finalize_v2_initialization().unwrap();
        let expected = state.stable_dek().unwrap().to_vec();
        let db = open_existing_v2_db(&dir.join(V2_DB_FILENAME)).unwrap();
        let params: crypto::KdfParams = serde_json::from_str(
            &db.get_setting("kdf_params_r").unwrap().unwrap(),
        )
        .unwrap();
        let wrapped = db.get_setting("wrapped_dek_r").unwrap().unwrap();
        let entropy = crypto::mnemonic_to_entropy(&words.join(" ")).unwrap();
        let salt = crypto::b64_decode(&params.salt).unwrap();
        let recovered = crypto::unwrap_dek_recovery(&wrapped, &entropy, &salt).unwrap();
        assert_eq!(recovered.to_vec(), expected);
        cleanup_dir(&dir);
    }

    #[test]
    fn n5_duplicate_initialization_fails_closed_without_overwrite() {
        let dir = temp_app_dir("n5");
        let state = fresh_v2_state(&dir);
        state.prepare_v2_initialization(MASTER).unwrap();
        state.finalize_v2_initialization().unwrap();
        let v2_path = dir.join(V2_DB_FILENAME);
        let hash_before = file_hash(&v2_path);
        let dek_before = state.stable_dek().unwrap().to_vec();

        assert!(state.prepare_v2_initialization("Another-Master!").is_err());
        assert!(prepare_fresh_v2(&dir, "Another-Master!").is_err());
        assert_eq!(file_hash(&v2_path), hash_before);
        assert_eq!(state.stable_dek().unwrap().to_vec(), dek_before);
        assert!(!dir.join(V2_TMP_FILENAME).exists());
        cleanup_dir(&dir);
    }

    #[test]
    fn n6_every_initialization_failure_point_leaves_no_formal_or_tmp_database() {
        let fail_points = [
            InitializationFailPoint::AfterSchema,
            InitializationFailPoint::AfterMasterWrap,
            InitializationFailPoint::BeforeRecoveryWrap,
            InitializationFailPoint::DuringMetadataWrite,
            InitializationFailPoint::BeforeRename,
        ];
        for (index, fail_point) in fail_points.iter().enumerate() {
            let dir = temp_app_dir(&format!("n6_{}", index));
            let result = prepare_fresh_v2_inject(&dir, MASTER, Some(*fail_point));
            assert!(result.is_err(), "注入点 {:?} 必须失败", fail_point);
            assert!(!dir.join(V2_DB_FILENAME).exists());
            assert!(!dir.join(V2_TMP_FILENAME).exists());
            assert!(!dir.join(format!("{}-wal", V2_TMP_FILENAME)).exists());
            assert!(!dir.join(format!("{}-shm", V2_TMP_FILENAME)).exists());
            assert!(!dir.join(format!("{}-journal", V2_TMP_FILENAME)).exists());
            assert!(!dir.join(V2_SETUP_LOCK_FILENAME).exists());
            cleanup_dir(&dir);
        }
    }

    #[test]
    fn n7_closing_before_recovery_confirmation_discards_only_setup_tmp_and_restarts_fresh() {
        let dir = temp_app_dir("n7");
        let state = fresh_v2_state(&dir);
        let abandoned_words = state.prepare_v2_initialization(MASTER).unwrap();
        assert!(!dir.join(V2_DB_FILENAME).exists());
        assert!(dir.join(V2_TMP_FILENAME).exists());
        assert!(dir.join(V2_SETUP_LOCK_FILENAME).exists());
        assert!(state.stable_dek().is_err());
        assert!(discard_abandoned_fresh_initialization(&dir).is_err());
        assert!(dir.join(V2_TMP_FILENAME).exists());
        drop(state);

        assert!(discard_abandoned_fresh_initialization(&dir).unwrap());
        assert!(!dir.join(V2_TMP_FILENAME).exists());
        assert!(!dir.join(V2_SETUP_LOCK_FILENAME).exists());
        assert!(!dir.join(V2_DB_FILENAME).exists());

        let restarted = fresh_v2_state(&dir);
        let replacement_words = restarted.prepare_v2_initialization(MASTER).unwrap();
        assert_ne!(replacement_words, abandoned_words);
        restarted.finalize_v2_initialization().unwrap();
        assert!(dir.join(V2_DB_FILENAME).exists());
        assert!(!dir.join(V2_SETUP_LOCK_FILENAME).exists());

        let protected = dir.join("legacy-protected");
        std::fs::create_dir_all(&protected).unwrap();
        std::fs::write(protected.join(LEGACY_DB_FILENAME), b"legacy-marker").unwrap();
        std::fs::write(protected.join(V2_TMP_FILENAME), b"migration-marker").unwrap();
        assert!(!discard_abandoned_fresh_initialization(&protected).unwrap());
        assert!(protected.join(LEGACY_DB_FILENAME).exists());
        assert!(protected.join(V2_TMP_FILENAME).exists());
        cleanup_dir(&dir);
    }

    #[test]
    fn n8_concurrent_initialization_allows_exactly_one_pending_keyset() {
        use std::sync::{Arc, Barrier};

        let dir = temp_app_dir("n8");
        let state = Arc::new(fresh_v2_state(&dir));
        let barrier = Arc::new(Barrier::new(2));
        let mut handles = Vec::new();
        for _ in 0..2 {
            let state = Arc::clone(&state);
            let barrier = Arc::clone(&barrier);
            handles.push(std::thread::spawn(move || {
                barrier.wait();
                state.prepare_v2_initialization(MASTER)
            }));
        }
        let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
        assert_eq!(results.iter().filter(|r| r.is_err()).count(), 1);
        assert!(!dir.join(V2_DB_FILENAME).exists());
        assert!(dir.join(V2_TMP_FILENAME).exists());

        state.finalize_v2_initialization().unwrap();
        assert!(dir.join(V2_DB_FILENAME).exists());
        assert!(!dir.join(V2_TMP_FILENAME).exists());
        drop(state);
        cleanup_dir(&dir);
    }

    // ===== Phase 2B.2：Recovery Phrase 恢复 + 主密码重包 =====
    fn completed_v2_state(
        tag: &str,
    ) -> (PathBuf, crate::AppState, Vec<String>, Vec<u8>) {
        let dir = temp_app_dir(tag);
        let state = fresh_v2_state(&dir);
        let words = state.prepare_v2_initialization(MASTER).unwrap();
        state.finalize_v2_initialization().unwrap();
        let dek = state.stable_dek().unwrap().to_vec();
        state.clear_key();
        (dir, state, words, dek)
    }

    #[test]
    fn r1_correct_recovery_phrase_unwraps_expected_stable_dek() {
        let (dir, state, words, expected) = completed_v2_state("r1");
        state.verify_v2_recovery_phrase(&words.join(" ")).unwrap();
        let db = state.db.lock().unwrap();
        let recovered = recover_v2_core(&db, &words.join(" ")).unwrap();
        assert_eq!(recovered.to_vec(), expected);
        assert!(!state.is_unlocked(), "验证短语不得污染 AppState");
        drop(db);
        drop(state);
        cleanup_dir(&dir);
    }

    #[test]
    fn r2_wrong_recovery_phrases_fail_without_db_or_state_changes() {
        let (dir, state, words, _) = completed_v2_state("r2");
        let v2_path = dir.join(V2_DB_FILENAME);
        let hash_before = file_hash(&v2_path);

        let mut reordered = words.clone();
        reordered.swap(0, 1);
        let unknown_word = format!("notaword {}", words[1..].join(" "));
        let checksum_invalid = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon".to_string();
        assert!(crypto::mnemonic_to_entropy(&checksum_invalid).is_err());
        let other_entropy = crypto::generate_recovery_entropy();
        let other_valid = crypto::entropy_to_mnemonic(&other_entropy).unwrap();

        for invalid in [unknown_word, reordered.join(" "), checksum_invalid, other_valid] {
            assert!(state.verify_v2_recovery_phrase(&invalid).is_err());
            assert_eq!(file_hash(&v2_path), hash_before, "认证失败不得修改 DB");
            assert!(!state.is_unlocked(), "认证失败不得污染 AppState");
        }
        drop(state);
        cleanup_dir(&dir);

        let legacy_dir = temp_app_dir("r2_legacy");
        build_legacy_fixture(&legacy_dir, MASTER);
        let legacy_path = legacy_dir.join(LEGACY_DB_FILENAME);
        let legacy_state = crate::AppState {
            db: std::sync::Mutex::new(Db::open(&legacy_path).unwrap()),
            db_path: legacy_path,
            startup_mode: std::sync::Mutex::new(crate::StartupMode::Legacy),
            pending_v2: std::sync::Mutex::new(None),
            pending_recovery_rotation: std::sync::Mutex::new(None),
            pending_legacy_migration: std::sync::Mutex::new(None),
            master_wrap_gate: std::sync::Mutex::new(()),
            key: std::sync::Mutex::new(None),
        };
        assert!(legacy_state.verify_v2_recovery_phrase(&words.join(" ")).is_err());
        assert!(legacy_state
            .recover_v2_with_phrase_and_store(&words.join(" "), "New-Master-Password!")
            .is_err());
        assert!(!legacy_dir.join(V2_DB_FILENAME).exists());
        drop(legacy_state);
        cleanup_dir(&legacy_dir);
    }

    #[test]
    fn r3_recovery_rewraps_same_dek_for_new_master_only() {
        let (dir, state, words, expected) = completed_v2_state("r3");
        let metadata_before = {
            let db = state.db.lock().unwrap();
            (
                db.get_setting("kdf_params_r").unwrap(),
                db.get_setting("wrapped_dek_r").unwrap(),
                db.get_setting("security_version").unwrap(),
                db.get_setting("wrapped_dek_m").unwrap(),
            )
        };
        state
            .recover_v2_with_phrase_and_store(&words.join(" "), "New-Master-Password!")
            .unwrap();
        assert_eq!(state.stable_dek().unwrap().to_vec(), expected);
        let db = state.db.lock().unwrap();
        assert!(unlock_v2_core(&db, MASTER).is_err(), "旧主密码必须失效");
        assert_eq!(
            unlock_v2_core(&db, "New-Master-Password!").unwrap().to_vec(),
            expected
        );
        assert_eq!(db.get_setting("kdf_params_r").unwrap(), metadata_before.0);
        assert_eq!(db.get_setting("wrapped_dek_r").unwrap(), metadata_before.1);
        assert_eq!(db.get_setting("security_version").unwrap(), metadata_before.2);
        assert_ne!(db.get_setting("wrapped_dek_m").unwrap(), metadata_before.3);
        drop(db);
        drop(state);
        cleanup_dir(&dir);

        // 模拟 COMMIT 成功、AppState 尚未安装 DEK 即进程中断：磁盘上的新主密码仍可解锁。
        let (crash_dir, crash_state, crash_words, crash_expected) =
            completed_v2_state("r3_post_commit_crash");
        let db = crash_state.db.lock().unwrap();
        let recovered = recover_v2_with_phrase(
            &db,
            &crash_words.join(" "),
            "Crash-Window-New-Master!",
        )
        .unwrap();
        assert_eq!(recovered.to_vec(), crash_expected);
        drop(recovered);
        assert!(!crash_state.is_unlocked());
        assert_eq!(
            unlock_v2_core(&db, "Crash-Window-New-Master!")
                .unwrap()
                .to_vec(),
            crash_expected
        );
        drop(db);
        drop(crash_state);
        cleanup_dir(&crash_dir);
    }

    #[test]
    fn r4_recovery_never_changes_password_ciphertext_or_record_uuid() {
        let (dir, state, words, expected) = completed_v2_state("r4");
        let uuids = [
            "4cbb8a2e-ec33-44d1-b38e-9fb2a9e88055",
            "1d8cc118-52ca-41a9-9e45-40322eb94973",
            "0cc8d60f-aae0-4742-a20f-74ccac6591b0",
        ];
        let mut ids = Vec::new();
        for (index, record_uuid) in uuids.iter().enumerate() {
            let ciphertext = crypto::encrypt_password_dw2(
                &format!("fixture-secret-{index}"),
                &expected,
                record_uuid,
            )
            .unwrap();
            ids.push(
                state
                    .db
                    .lock()
                    .unwrap()
                    .create_password_v2(record_uuid, "fixture", "user", &ciphertext, "", "")
                    .unwrap(),
            );
        }
        let before: Vec<_> = ids
            .iter()
            .map(|id| state.db.lock().unwrap().get_password_v2_snapshot(*id).unwrap().unwrap())
            .collect();

        state
            .recover_v2_with_phrase_and_store(&words.join(" "), "New-Master-Password!")
            .unwrap();
        let after: Vec<_> = ids
            .iter()
            .map(|id| state.db.lock().unwrap().get_password_v2_snapshot(*id).unwrap().unwrap())
            .collect();
        for (before, after) in before.iter().zip(after.iter()) {
            assert_eq!(after.record_uuid, before.record_uuid);
            assert_eq!(after.encrypted_password, before.encrypted_password);
        }
        drop(state);
        cleanup_dir(&dir);
    }

    #[test]
    fn r5_original_recovery_phrase_remains_valid_after_master_reset() {
        let (dir, state, words, expected) = completed_v2_state("r5");
        state
            .recover_v2_with_phrase_and_store(&words.join(" "), "New-Master-Password!")
            .unwrap();
        let db = state.db.lock().unwrap();
        assert_eq!(recover_v2_core(&db, &words.join(" ")).unwrap().to_vec(), expected);
        drop(db);
        drop(state);
        cleanup_dir(&dir);
    }

    #[test]
    fn r6_every_master_rewrap_failure_rolls_back_atomically() {
        let fail_points = [
            MasterRewrapFailPoint::AfterNewSalt,
            MasterRewrapFailPoint::AfterMasterKek,
            MasterRewrapFailPoint::AfterWrappedDek,
            MasterRewrapFailPoint::DuringMetadataWrite,
            MasterRewrapFailPoint::BeforeCommit,
        ];
        for (index, fail_point) in fail_points.into_iter().enumerate() {
            let (dir, state, words, expected) = completed_v2_state(&format!("r6_{index}"));
            let db = state.db.lock().unwrap();
            assert!(recover_v2_with_phrase_inject(
                &db,
                &words.join(" "),
                "New-Master-Password!",
                Some(fail_point),
            )
            .is_err());
            assert_eq!(unlock_v2_core(&db, MASTER).unwrap().to_vec(), expected);
            assert!(unlock_v2_core(&db, "New-Master-Password!").is_err());
            drop(db);
            assert!(!state.is_unlocked());
            drop(state);
            cleanup_dir(&dir);
        }
    }

    #[test]
    fn r7_concurrent_recovery_completion_allows_exactly_one_commit() {
        use std::sync::{Arc, Barrier};

        let (dir, state, words, expected) = completed_v2_state("r7");
        let state = Arc::new(state);
        let phrase = words.join(" ");
        let barrier = Arc::new(Barrier::new(2));
        let mut handles = Vec::new();
        for _ in 0..2 {
            let state = Arc::clone(&state);
            let phrase = phrase.clone();
            let barrier = Arc::clone(&barrier);
            handles.push(std::thread::spawn(move || {
                barrier.wait();
                state.recover_v2_with_phrase_and_store(&phrase, "New-Master-Password!")
            }));
        }
        let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
        assert_eq!(results.iter().filter(|r| r.is_err()).count(), 1);
        assert_eq!(state.stable_dek().unwrap().to_vec(), expected);
        let db = state.db.lock().unwrap();
        assert_eq!(
            unlock_v2_core(&db, "New-Master-Password!").unwrap().to_vec(),
            expected
        );
        drop(db);
        drop(state);
        cleanup_dir(&dir);
    }

    // ===== Phase 2B.3a：已知旧主密码 → 普通修改主密码 =====
    fn security_metadata_snapshot(
        db: &Db,
    ) -> (Option<String>, Option<String>, Option<String>, Option<String>, Option<String>) {
        (
            db.get_setting("kdf_params_m").unwrap(),
            db.get_setting("wrapped_dek_m").unwrap(),
            db.get_setting("kdf_params_r").unwrap(),
            db.get_setting("wrapped_dek_r").unwrap(),
            db.get_setting("security_version").unwrap(),
        )
    }

    #[test]
    fn m1_correct_old_master_enables_new_master_and_invalidates_old() {
        let (dir, state, _, expected) = completed_v2_state("m1");
        state.unlock_v2_and_store(MASTER).unwrap();
        state
            .change_v2_master_password(MASTER, "M1-New-Master!")
            .unwrap();
        let db = state.db.lock().unwrap();
        assert!(unlock_v2_core(&db, MASTER).is_err());
        assert_eq!(
            unlock_v2_core(&db, "M1-New-Master!").unwrap().to_vec(),
            expected
        );
        drop(db);
        drop(state);
        cleanup_dir(&dir);
    }

    #[test]
    fn m2_wrong_old_master_changes_neither_db_nor_app_state() {
        let (dir, state, _, expected) = completed_v2_state("m2");
        state.unlock_v2_and_store(MASTER).unwrap();
        let path = dir.join(V2_DB_FILENAME);
        let hash_before = file_hash(&path);
        let metadata_before = security_metadata_snapshot(&state.db.lock().unwrap());

        assert!(state
            .change_v2_master_password("wrong-old-master", "M2-New-Master!")
            .is_err());
        assert_eq!(file_hash(&path), hash_before);
        assert_eq!(security_metadata_snapshot(&state.db.lock().unwrap()), metadata_before);
        assert_eq!(state.stable_dek().unwrap().to_vec(), expected);
        drop(state);
        cleanup_dir(&dir);
    }

    #[test]
    fn m3_stable_dek_and_active_app_state_key_are_identical_after_change() {
        let (dir, state, _, expected) = completed_v2_state("m3");
        state.unlock_v2_and_store(MASTER).unwrap();
        let active_before = state.stable_dek().unwrap().to_vec();
        state
            .change_v2_master_password(MASTER, "M3-New-Master!")
            .unwrap();
        assert_eq!(active_before, expected);
        assert_eq!(state.stable_dek().unwrap().to_vec(), active_before);
        let db = state.db.lock().unwrap();
        assert_eq!(
            unlock_v2_core(&db, "M3-New-Master!").unwrap().to_vec(),
            expected
        );
        drop(db);
        drop(state);
        cleanup_dir(&dir);
    }

    #[test]
    fn m4_password_ciphertext_and_record_uuid_remain_byte_for_byte_unchanged() {
        let (dir, state, _, expected) = completed_v2_state("m4");
        state.unlock_v2_and_store(MASTER).unwrap();
        let uuids = [
            "3cf44ab9-7780-4d7d-b96c-378894036d43",
            "0799a190-c46d-466c-a1cc-d54c2ec0f81e",
            "968920cf-28ce-435e-b5fc-558d37f8ba74",
        ];
        let mut ids = Vec::new();
        for (index, record_uuid) in uuids.iter().enumerate() {
            let ciphertext = crypto::encrypt_password_dw2(
                &format!("m4-secret-{index}"),
                &expected,
                record_uuid,
            )
            .unwrap();
            ids.push(
                state
                    .db
                    .lock()
                    .unwrap()
                    .create_password_v2(record_uuid, "fixture", "user", &ciphertext, "", "")
                    .unwrap(),
            );
        }
        let before: Vec<_> = ids
            .iter()
            .map(|id| state.db.lock().unwrap().get_password_v2_snapshot(*id).unwrap().unwrap())
            .collect();
        state
            .change_v2_master_password(MASTER, "M4-New-Master!")
            .unwrap();
        let after: Vec<_> = ids
            .iter()
            .map(|id| state.db.lock().unwrap().get_password_v2_snapshot(*id).unwrap().unwrap())
            .collect();
        for (before, after) in before.iter().zip(after.iter()) {
            assert_eq!(after.record_uuid, before.record_uuid);
            assert_eq!(after.encrypted_password, before.encrypted_password);
        }
        drop(state);
        cleanup_dir(&dir);
    }

    #[test]
    fn m5_recovery_wrap_and_metadata_remain_byte_for_byte_unchanged() {
        let (dir, state, words, expected) = completed_v2_state("m5");
        state.unlock_v2_and_store(MASTER).unwrap();
        let before = security_metadata_snapshot(&state.db.lock().unwrap());
        state
            .change_v2_master_password(MASTER, "M5-New-Master!")
            .unwrap();
        let db = state.db.lock().unwrap();
        let after = security_metadata_snapshot(&db);
        assert_ne!(after.0, before.0, "master KDF metadata 必须更新");
        assert_ne!(after.1, before.1, "master wrap 必须更新");
        assert_eq!(after.2, before.2, "Recovery KDF metadata 不得变化");
        assert_eq!(after.3, before.3, "Recovery wrap 不得变化");
        assert_eq!(after.4, before.4, "security_version 不得变化");
        assert_eq!(recover_v2_core(&db, &words.join(" ")).unwrap().to_vec(), expected);
        drop(db);
        drop(state);
        cleanup_dir(&dir);
    }

    #[test]
    fn m6_every_master_change_failure_point_keeps_old_master_valid() {
        let fail_points = [
            MasterRewrapFailPoint::AfterNewSalt,
            MasterRewrapFailPoint::AfterMasterKek,
            MasterRewrapFailPoint::AfterWrappedDek,
            MasterRewrapFailPoint::DuringMetadataWrite,
            MasterRewrapFailPoint::BeforeCommit,
        ];
        for (index, fail_point) in fail_points.into_iter().enumerate() {
            let (dir, state, _, expected) = completed_v2_state(&format!("m6_{index}"));
            state.unlock_v2_and_store(MASTER).unwrap();
            let active = state.stable_dek().unwrap();
            let db = state.db.lock().unwrap();
            let before = security_metadata_snapshot(&db);
            assert!(change_v2_master_password_inject(
                &db,
                MASTER,
                "M6-New-Master!",
                active.as_slice(),
                Some(fail_point),
            )
            .is_err());
            assert_eq!(security_metadata_snapshot(&db), before);
            assert_eq!(unlock_v2_core(&db, MASTER).unwrap().to_vec(), expected);
            assert!(unlock_v2_core(&db, "M6-New-Master!").is_err());
            drop(db);
            drop(active);
            drop(state);
            cleanup_dir(&dir);
        }
    }

    #[test]
    fn m7_concurrent_master_changes_allow_exactly_one_complete_commit() {
        use std::sync::{Arc, Barrier};

        let (dir, state, _, expected) = completed_v2_state("m7");
        state.unlock_v2_and_store(MASTER).unwrap();
        let state = Arc::new(state);
        let barrier = Arc::new(Barrier::new(2));
        let new_passwords = ["M7-New-Master-A!", "M7-New-Master-B!"];
        let mut handles = Vec::new();
        for new_password in new_passwords {
            let state = Arc::clone(&state);
            let barrier = Arc::clone(&barrier);
            handles.push(std::thread::spawn(move || {
                barrier.wait();
                state.change_v2_master_password(MASTER, new_password)
            }));
        }
        let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        assert_eq!(results.iter().filter(|result| result.is_err()).count(), 1);
        assert_eq!(state.stable_dek().unwrap().to_vec(), expected);
        let db = state.db.lock().unwrap();
        let valid_new: Vec<_> = new_passwords
            .iter()
            .filter_map(|password| unlock_v2_core(&db, password).ok())
            .collect();
        assert_eq!(valid_new.len(), 1, "最终 metadata 必须完整属于一个请求");
        assert_eq!(valid_new[0].to_vec(), expected);
        assert!(unlock_v2_core(&db, MASTER).is_err());
        drop(db);
        drop(state);
        cleanup_dir(&dir);
    }

    #[test]
    fn m8_same_password_and_legacy_api_calls_fail_closed_without_writes() {
        let (dir, state, _, expected) = completed_v2_state("m8");
        state.unlock_v2_and_store(MASTER).unwrap();
        let before = security_metadata_snapshot(&state.db.lock().unwrap());
        assert_eq!(
            state.change_v2_master_password(MASTER, MASTER).unwrap_err(),
            "新主密码必须与当前主密码不同"
        );
        assert_eq!(security_metadata_snapshot(&state.db.lock().unwrap()), before);
        assert_eq!(state.stable_dek().unwrap().to_vec(), expected);
        drop(state);
        cleanup_dir(&dir);

        let legacy_dir = temp_app_dir("m8_legacy");
        build_legacy_fixture(&legacy_dir, MASTER);
        let legacy_path = legacy_dir.join(LEGACY_DB_FILENAME);
        let legacy_state = crate::AppState {
            db: std::sync::Mutex::new(Db::open(&legacy_path).unwrap()),
            db_path: legacy_path,
            startup_mode: std::sync::Mutex::new(crate::StartupMode::Legacy),
            pending_v2: std::sync::Mutex::new(None),
            pending_recovery_rotation: std::sync::Mutex::new(None),
            pending_legacy_migration: std::sync::Mutex::new(None),
            master_wrap_gate: std::sync::Mutex::new(()),
            key: std::sync::Mutex::new(None),
        };
        assert!(legacy_state
            .change_v2_master_password(MASTER, "M8-New-Master!")
            .is_err());
        assert!(!legacy_dir.join(V2_DB_FILENAME).exists());
        drop(legacy_state);
        cleanup_dir(&legacy_dir);
    }

    #[test]
    fn m9_master_change_and_recovery_reset_cannot_concurrently_write_master_metadata() {
        use std::sync::{Arc, Barrier};

        let (dir, state, words, expected) = completed_v2_state("m9");
        state.unlock_v2_and_store(MASTER).unwrap();
        let state = Arc::new(state);
        let recovery_before = {
            let db = state.db.lock().unwrap();
            (
                db.get_setting("kdf_params_r").unwrap(),
                db.get_setting("wrapped_dek_r").unwrap(),
            )
        };
        let barrier = Arc::new(Barrier::new(2));

        let change_handle = {
            let state = Arc::clone(&state);
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                state.change_v2_master_password(MASTER, "M9-Normal-New-Master!")
            })
        };
        let recovery_handle = {
            let state = Arc::clone(&state);
            let barrier = Arc::clone(&barrier);
            let phrase = words.join(" ");
            std::thread::spawn(move || {
                barrier.wait();
                state.recover_v2_with_phrase_and_store(&phrase, "M9-Recovery-New-Master!")
            })
        };

        assert!(change_handle.join().unwrap().is_ok());
        assert!(recovery_handle.join().unwrap().is_err());
        assert_eq!(state.stable_dek().unwrap().to_vec(), expected);
        let db = state.db.lock().unwrap();
        assert_eq!(
            unlock_v2_core(&db, "M9-Normal-New-Master!").unwrap().to_vec(),
            expected
        );
        assert!(unlock_v2_core(&db, MASTER).is_err());
        assert!(unlock_v2_core(&db, "M9-Recovery-New-Master!").is_err());
        assert_eq!(db.get_setting("kdf_params_r").unwrap(), recovery_before.0);
        assert_eq!(db.get_setting("wrapped_dek_r").unwrap(), recovery_before.1);
        drop(db);
        drop(state);
        cleanup_dir(&dir);
    }

    // ===== Phase 2B.3b：Recovery Phrase 两阶段轮换 =====
    fn rotation_answers(prepared: &crate::RecoveryRotationPreparation) -> Vec<String> {
        prepared
            .confirmation_indexes
            .iter()
            .map(|index| prepared.recovery_words[*index].clone())
            .collect()
    }

    fn rotation_phrase(prepared: &crate::RecoveryRotationPreparation) -> String {
        prepared.recovery_words.join(" ")
    }

    #[test]
    fn rr1_confirmed_rotation_invalidates_old_phrase_and_preserves_stable_dek() {
        let (dir, state, old_words, expected) = completed_v2_state("rr1");
        state.unlock_v2_and_store(MASTER).unwrap();
        let prepared = state.prepare_v2_recovery_rotation().unwrap();
        let new_phrase = rotation_phrase(&prepared);
        let answers = rotation_answers(&prepared);
        state
            .confirm_v2_recovery_rotation(&prepared.rotation_token, &answers)
            .unwrap();

        assert_eq!(state.stable_dek().unwrap().to_vec(), expected);
        let db = state.db.lock().unwrap();
        assert!(recover_v2_core(&db, &old_words.join(" ")).is_err());
        assert_eq!(recover_v2_core(&db, &new_phrase).unwrap().to_vec(), expected);
        drop(db);
        drop(state);
        cleanup_dir(&dir);
    }

    #[test]
    fn rr2_unconfirmed_rotation_is_discarded_without_db_changes() {
        let (dir, state, old_words, expected) = completed_v2_state("rr2");
        state.unlock_v2_and_store(MASTER).unwrap();
        let v2_path = dir.join(V2_DB_FILENAME);
        let hash_before = file_hash(&v2_path);
        let before = security_metadata_snapshot(&state.db.lock().unwrap());
        let prepared = state.prepare_v2_recovery_rotation().unwrap();
        let new_phrase = rotation_phrase(&prepared);

        state.clear_key();
        assert!(state
            .confirm_v2_recovery_rotation(
                &prepared.rotation_token,
                &rotation_answers(&prepared),
            )
            .is_err());
        state.unlock_v2_and_store(MASTER).unwrap();
        assert_eq!(file_hash(&v2_path), hash_before, "pending 阶段不得写正式 DB");
        let db = state.db.lock().unwrap();
        assert_eq!(security_metadata_snapshot(&db), before);
        assert_eq!(recover_v2_core(&db, &old_words.join(" ")).unwrap().to_vec(), expected);
        assert!(recover_v2_core(&db, &new_phrase).is_err());
        drop(db);
        drop(state);
        cleanup_dir(&dir);
    }

    #[test]
    fn rr3_wrong_confirmation_keeps_old_recovery_and_pending_rotation() {
        let (dir, state, old_words, expected) = completed_v2_state("rr3");
        state.unlock_v2_and_store(MASTER).unwrap();
        let before = security_metadata_snapshot(&state.db.lock().unwrap());
        let prepared = state.prepare_v2_recovery_rotation().unwrap();
        let wrong = vec!["wrong".to_string(); 3];
        assert_eq!(
            state
                .confirm_v2_recovery_rotation(&prepared.rotation_token, &wrong)
                .unwrap_err(),
            "恢复词确认不匹配"
        );
        {
            let db = state.db.lock().unwrap();
            assert_eq!(security_metadata_snapshot(&db), before);
            assert_eq!(recover_v2_core(&db, &old_words.join(" ")).unwrap().to_vec(), expected);
        }
        state
            .confirm_v2_recovery_rotation(
                &prepared.rotation_token,
                &rotation_answers(&prepared),
            )
            .unwrap();
        drop(state);
        cleanup_dir(&dir);
    }

    #[test]
    fn rr4_master_metadata_and_master_unlock_are_unchanged() {
        let (dir, state, _, expected) = completed_v2_state("rr4");
        state.unlock_v2_and_store(MASTER).unwrap();
        let before = security_metadata_snapshot(&state.db.lock().unwrap());
        let prepared = state.prepare_v2_recovery_rotation().unwrap();
        state
            .confirm_v2_recovery_rotation(
                &prepared.rotation_token,
                &rotation_answers(&prepared),
            )
            .unwrap();
        let db = state.db.lock().unwrap();
        let after = security_metadata_snapshot(&db);
        assert_eq!(after.0, before.0, "master KDF metadata 不得变化");
        assert_eq!(after.1, before.1, "master wrap 不得变化");
        assert_eq!(after.4, before.4, "security_version 不得变化");
        assert_eq!(unlock_v2_core(&db, MASTER).unwrap().to_vec(), expected);
        drop(db);
        drop(state);
        cleanup_dir(&dir);
    }

    #[test]
    fn rr5_password_ciphertext_and_record_uuid_are_unchanged() {
        let (dir, state, _, expected) = completed_v2_state("rr5");
        state.unlock_v2_and_store(MASTER).unwrap();
        let uuids = [
            "4cb62838-60c0-434b-8c84-95375c15a729",
            "ecaf35d0-5e34-433f-8bdb-2b83cd085c58",
            "604a77f3-74bb-4b75-bb78-931726664ad8",
        ];
        let mut ids = Vec::new();
        for (index, record_uuid) in uuids.iter().enumerate() {
            let ciphertext = crypto::encrypt_password_dw2(
                &format!("rr5-secret-{index}"),
                &expected,
                record_uuid,
            )
            .unwrap();
            ids.push(
                state
                    .db
                    .lock()
                    .unwrap()
                    .create_password_v2(record_uuid, "fixture", "user", &ciphertext, "", "")
                    .unwrap(),
            );
        }
        let before: Vec<_> = ids
            .iter()
            .map(|id| state.db.lock().unwrap().get_password_v2_snapshot(*id).unwrap().unwrap())
            .collect();
        let prepared = state.prepare_v2_recovery_rotation().unwrap();
        state
            .confirm_v2_recovery_rotation(
                &prepared.rotation_token,
                &rotation_answers(&prepared),
            )
            .unwrap();
        let after: Vec<_> = ids
            .iter()
            .map(|id| state.db.lock().unwrap().get_password_v2_snapshot(*id).unwrap().unwrap())
            .collect();
        for (before, after) in before.iter().zip(after.iter()) {
            assert_eq!(after.record_uuid, before.record_uuid);
            assert_eq!(after.encrypted_password, before.encrypted_password);
        }
        drop(state);
        cleanup_dir(&dir);
    }

    #[test]
    fn rr6_all_preparation_and_transaction_failures_keep_old_recovery_valid() {
        let prepare_failures = [
            RecoveryRotationFailPoint::AfterNewSalt,
            RecoveryRotationFailPoint::AfterRecoveryKek,
            RecoveryRotationFailPoint::AfterWrappedDek,
        ];
        for (index, fail_point) in prepare_failures.into_iter().enumerate() {
            let (dir, state, old_words, expected) = completed_v2_state(&format!("rr6_p_{index}"));
            state.unlock_v2_and_store(MASTER).unwrap();
            let before = security_metadata_snapshot(&state.db.lock().unwrap());
            assert!(state.prepare_v2_recovery_rotation_inject(Some(fail_point)).is_err());
            let db = state.db.lock().unwrap();
            assert_eq!(security_metadata_snapshot(&db), before);
            assert_eq!(recover_v2_core(&db, &old_words.join(" ")).unwrap().to_vec(), expected);
            drop(db);
            drop(state);
            cleanup_dir(&dir);
        }

        let transaction_failures = [
            RecoveryRotationFailPoint::DuringMetadataWrite,
            RecoveryRotationFailPoint::BeforeCommit,
        ];
        for (index, fail_point) in transaction_failures.into_iter().enumerate() {
            let (dir, state, old_words, expected) = completed_v2_state(&format!("rr6_t_{index}"));
            state.unlock_v2_and_store(MASTER).unwrap();
            let before = security_metadata_snapshot(&state.db.lock().unwrap());
            let prepared = state.prepare_v2_recovery_rotation().unwrap();
            let new_phrase = rotation_phrase(&prepared);
            assert!(state
                .confirm_v2_recovery_rotation_inject(
                    &prepared.rotation_token,
                    &rotation_answers(&prepared),
                    Some(fail_point),
                )
                .is_err());
            let db = state.db.lock().unwrap();
            assert_eq!(security_metadata_snapshot(&db), before);
            assert_eq!(recover_v2_core(&db, &old_words.join(" ")).unwrap().to_vec(), expected);
            assert!(recover_v2_core(&db, &new_phrase).is_err());
            drop(db);
            state.cancel_v2_recovery_rotation(&prepared.rotation_token).unwrap();
            drop(state);
            cleanup_dir(&dir);
        }
    }

    #[test]
    fn rr7_concurrent_prepare_allows_one_pending_rotation_and_legacy_is_rejected() {
        use std::sync::{Arc, Barrier};

        let (dir, state, _, _) = completed_v2_state("rr7");
        state.unlock_v2_and_store(MASTER).unwrap();
        let state = Arc::new(state);
        let barrier = Arc::new(Barrier::new(2));
        let mut handles = Vec::new();
        for _ in 0..2 {
            let state = Arc::clone(&state);
            let barrier = Arc::clone(&barrier);
            handles.push(std::thread::spawn(move || {
                barrier.wait();
                state.prepare_v2_recovery_rotation()
            }));
        }
        let results: Vec<_> = handles.into_iter().map(|handle| handle.join().unwrap()).collect();
        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        assert_eq!(results.iter().filter(|result| result.is_err()).count(), 1);
        let prepared = results.into_iter().find_map(Result::ok).unwrap();
        state.cancel_v2_recovery_rotation(&prepared.rotation_token).unwrap();
        assert!(state
            .confirm_v2_recovery_rotation(
                &prepared.rotation_token,
                &rotation_answers(&prepared),
            )
            .is_err());
        drop(state);
        cleanup_dir(&dir);

        let legacy_dir = temp_app_dir("rr7_legacy");
        build_legacy_fixture(&legacy_dir, MASTER);
        let legacy_path = legacy_dir.join(LEGACY_DB_FILENAME);
        let legacy_state = crate::AppState {
            db: std::sync::Mutex::new(Db::open(&legacy_path).unwrap()),
            db_path: legacy_path,
            startup_mode: std::sync::Mutex::new(crate::StartupMode::Legacy),
            pending_v2: std::sync::Mutex::new(None),
            pending_recovery_rotation: std::sync::Mutex::new(None),
            pending_legacy_migration: std::sync::Mutex::new(None),
            master_wrap_gate: std::sync::Mutex::new(()),
            key: std::sync::Mutex::new(None),
        };
        assert!(legacy_state.prepare_v2_recovery_rotation().is_err());
        assert!(!legacy_dir.join(V2_DB_FILENAME).exists());
        drop(legacy_state);
        cleanup_dir(&legacy_dir);
    }

    #[test]
    fn rr8_duplicate_confirm_commits_exactly_once() {
        let (dir, state, old_words, expected) = completed_v2_state("rr8");
        state.unlock_v2_and_store(MASTER).unwrap();
        let prepared = state.prepare_v2_recovery_rotation().unwrap();
        let new_phrase = rotation_phrase(&prepared);
        let answers = rotation_answers(&prepared);
        state
            .confirm_v2_recovery_rotation(&prepared.rotation_token, &answers)
            .unwrap();
        assert!(state
            .confirm_v2_recovery_rotation(&prepared.rotation_token, &answers)
            .is_err());
        let db = state.db.lock().unwrap();
        assert!(recover_v2_core(&db, &old_words.join(" ")).is_err());
        assert_eq!(recover_v2_core(&db, &new_phrase).unwrap().to_vec(), expected);
        drop(db);
        drop(state);
        cleanup_dir(&dir);
    }

    #[test]
    fn rr9_rotation_confirm_and_master_change_are_concurrent_safe() {
        use std::sync::{Arc, Barrier};

        let (dir, state, old_words, expected) = completed_v2_state("rr9");
        state.unlock_v2_and_store(MASTER).unwrap();
        let prepared = state.prepare_v2_recovery_rotation().unwrap();
        let new_phrase = rotation_phrase(&prepared);
        let token = prepared.rotation_token.clone();
        let answers = rotation_answers(&prepared);
        let state = Arc::new(state);
        let barrier = Arc::new(Barrier::new(2));

        let rotate_handle = {
            let state = Arc::clone(&state);
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                state.confirm_v2_recovery_rotation(&token, &answers)
            })
        };
        let master_handle = {
            let state = Arc::clone(&state);
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                state.change_v2_master_password(MASTER, "RR9-New-Master!")
            })
        };

        assert!(rotate_handle.join().unwrap().is_ok());
        assert!(master_handle.join().unwrap().is_ok());
        assert_eq!(state.stable_dek().unwrap().to_vec(), expected);
        let db = state.db.lock().unwrap();
        assert!(unlock_v2_core(&db, MASTER).is_err());
        assert_eq!(
            unlock_v2_core(&db, "RR9-New-Master!").unwrap().to_vec(),
            expected
        );
        assert!(recover_v2_core(&db, &old_words.join(" ")).is_err());
        assert_eq!(recover_v2_core(&db, &new_phrase).unwrap().to_vec(), expected);
        drop(db);
        drop(state);
        cleanup_dir(&dir);
    }

    #[test]
    fn rr10_consecutive_rotations_invalidate_each_previous_phrase() {
        let (dir, state, words_a, expected) = completed_v2_state("rr10");
        state.unlock_v2_and_store(MASTER).unwrap();

        let prepared_b = state.prepare_v2_recovery_rotation().unwrap();
        let phrase_b = rotation_phrase(&prepared_b);
        let answers_b = rotation_answers(&prepared_b);
        state
            .confirm_v2_recovery_rotation(&prepared_b.rotation_token, &answers_b)
            .unwrap();

        let prepared_c = state.prepare_v2_recovery_rotation().unwrap();
        let phrase_c = rotation_phrase(&prepared_c);
        let answers_c = rotation_answers(&prepared_c);
        assert!(state
            .confirm_v2_recovery_rotation(&prepared_b.rotation_token, &answers_b)
            .is_err());
        state
            .confirm_v2_recovery_rotation(&prepared_c.rotation_token, &answers_c)
            .unwrap();

        assert_eq!(state.stable_dek().unwrap().to_vec(), expected);
        let db = state.db.lock().unwrap();
        assert!(recover_v2_core(&db, &words_a.join(" ")).is_err());
        assert!(recover_v2_core(&db, &phrase_b).is_err());
        assert_eq!(recover_v2_core(&db, &phrase_c).unwrap().to_vec(), expected);
        drop(db);
        drop(state);
        cleanup_dir(&dir);
    }

    #[test]
    fn rr11_lock_clears_pending_and_allows_a_fresh_rotation_after_unlock() {
        let (dir, state, old_words, expected) = completed_v2_state("rr11");
        state.unlock_v2_and_store(MASTER).unwrap();
        let before = security_metadata_snapshot(&state.db.lock().unwrap());
        let abandoned = state.prepare_v2_recovery_rotation().unwrap();
        let abandoned_phrase = rotation_phrase(&abandoned);
        let abandoned_answers = rotation_answers(&abandoned);

        state.clear_key();
        assert!(state
            .confirm_v2_recovery_rotation(
                &abandoned.rotation_token,
                &abandoned_answers,
            )
            .is_err());
        state.unlock_v2_and_store(MASTER).unwrap();
        {
            let db = state.db.lock().unwrap();
            assert_eq!(security_metadata_snapshot(&db), before);
            assert_eq!(recover_v2_core(&db, &old_words.join(" ")).unwrap().to_vec(), expected);
            assert!(recover_v2_core(&db, &abandoned_phrase).is_err());
        }

        let replacement = state.prepare_v2_recovery_rotation().unwrap();
        assert_ne!(replacement.rotation_token, abandoned.rotation_token);
        assert_ne!(rotation_phrase(&replacement), abandoned_phrase);
        state
            .cancel_v2_recovery_rotation(&replacement.rotation_token)
            .unwrap();
        drop(state);
        cleanup_dir(&dir);
    }

    const MASTER: &str = "Phase2A-Master!";
}

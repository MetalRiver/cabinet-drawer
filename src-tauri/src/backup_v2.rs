//! Stable DEK v2 离线备份/恢复。
//!
//! v2 备份从活动 SQLite 连接生成一致性快照，快照整体由独立的
//! DRAWERBOX2 认证容器保护。密码行始终保持 record_uuid + DW2 密文，
//! 不经过明文导出或逐行重加密。

use crate::{backup, crypto, migration, AppState, SecurityModel, StartupMode};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use rusqlite::{backup::Backup, Connection};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;
use zeroize::Zeroizing;

const BACKUP_VERSION: u32 = 2;
const SECURITY_MODEL: &str = "stable-dek-v2";
const MAX_BACKUP_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const REQUIRED_TABLES: [&str; 7] = [
    "settings",
    "passwords",
    "app_categories",
    "apps",
    "snippets",
    "temp_contents",
    "pinned_items",
];

const EXPECTED_COLUMNS: [(&str, &[&str]); 7] = [
    ("settings", &["key", "value"]),
    (
        "passwords",
        &[
            "id", "record_uuid", "title", "username", "password", "url", "notes",
            "created_at", "updated_at", "use_count", "last_used_at", "deleted_at",
        ],
    ),
    ("app_categories", &["id", "name", "icon", "sort_order"]),
    (
        "apps",
        &[
            "id", "name", "path", "icon_path", "args", "category_id", "created_at",
            "use_count", "last_used_at", "app_type", "app_subtype", "deleted_at",
        ],
    ),
    (
        "snippets",
        &[
            "id", "title", "content", "language", "tags", "created_at", "updated_at",
            "use_count", "last_used_at", "deleted_at",
        ],
    ),
    ("temp_contents", &["id", "text", "created_at", "expires_at", "deleted_at"]),
    ("pinned_items", &["id", "item_type", "item_id", "sort_order"]),
];

#[derive(Debug, Clone, Serialize, Deserialize)]
struct V2BackupEnvelope {
    backup_version: u32,
    security_model: String,
    generated_at: i64,
    database_size: u64,
    database_sha256: String,
    table_counts: BTreeMap<String, u64>,
    database_b64: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct V2BackupStats {
    pub settings: usize,
    pub categories: usize,
    pub apps: usize,
    pub passwords: usize,
    pub snippets: usize,
    pub temps: usize,
    pub pinned: usize,
    pub trash_passwords: usize,
    pub database_bytes: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum V2RestoreFailPoint {
    AfterDecrypt,
    AfterTempWrite,
    AfterSecurityValidation,
    AfterPasswordVerification,
    BeforeActivation,
}

struct TempDbGuard {
    path: PathBuf,
}

impl TempDbGuard {
    fn new(path: PathBuf) -> Self {
        Self { path }
    }
}

impl Drop for TempDbGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
        let _ = std::fs::remove_file(PathBuf::from(format!("{}-wal", self.path.display())));
        let _ = std::fs::remove_file(PathBuf::from(format!("{}-shm", self.path.display())));
    }
}

fn ensure_existing_v2(state: &AppState) -> Result<(), String> {
    if state.security_model() != SecurityModel::StableDekV2
        || *state
            .startup_mode
            .lock()
            .map_err(|_| "安全状态不可用".to_string())?
            != StartupMode::ExistingV2
    {
        return Err("当前密码库不支持 v2 备份恢复".to_string());
    }
    Ok(())
}

fn unique_temp_path(dir: &Path, label: &str) -> PathBuf {
    dir.join(format!(".drawerbox-{}-{}.tmp", label, uuid::Uuid::new_v4()))
}

fn validate_export_target(state: &AppState, output_path: &Path) -> Result<(), String> {
    let db_parent = state
        .db_path
        .parent()
        .and_then(|path| std::fs::canonicalize(path).ok())
        .ok_or_else(|| "数据库路径无效".to_string())?;
    let output_parent = output_path
        .parent()
        .and_then(|path| std::fs::canonicalize(path).ok());
    if output_parent.as_ref() != Some(&db_parent) {
        return Ok(());
    }
    let db_name = state
        .db_path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "数据库路径无效".to_string())?;
    let output_name = output_path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "备份路径无效".to_string())?;
    let protected = [
        db_name.to_string(),
        format!("{db_name}-wal"),
        format!("{db_name}-shm"),
        migration::V2_TMP_FILENAME.to_string(),
        migration::V2_SETUP_LOCK_FILENAME.to_string(),
    ];
    if protected.iter().any(|name| name.eq_ignore_ascii_case(output_name)) {
        return Err("备份目标不能覆盖活动数据库或安全临时文件".to_string());
    }
    Ok(())
}

fn snapshot_database(db: &crate::db::Db, path: &Path) -> Result<(), String> {
    let source = db
        .conn
        .lock()
        .map_err(|_| "数据库快照不可用".to_string())?;
    let mut destination = Connection::open(path).map_err(|_| "无法创建数据库快照".to_string())?;
    let backup = Backup::new(&source, &mut destination)
        .map_err(|_| "无法创建数据库快照".to_string())?;
    backup
        .run_to_completion(32, Duration::from_millis(5), None)
        .map_err(|_| "无法创建数据库快照".to_string())?;
    drop(backup);
    destination
        .execute_batch("PRAGMA optimize")
        .map_err(|_| "无法完成数据库快照".to_string())?;
    Ok(())
}

fn quoted_identifier(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

fn table_counts(db: &crate::db::Db) -> Result<BTreeMap<String, u64>, String> {
    let conn = db.conn.lock().map_err(|_| "数据库校验不可用".to_string())?;
    let mut stmt = conn
        .prepare(
            "SELECT name FROM sqlite_master
             WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
        )
        .map_err(|_| "数据库 schema 无效".to_string())?;
    let names = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|_| "数据库 schema 无效".to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| "数据库 schema 无效".to_string())?;
    let mut out = BTreeMap::new();
    for name in names {
        let sql = format!("SELECT COUNT(*) FROM {}", quoted_identifier(&name));
        let count: i64 = conn
            .query_row(&sql, [], |row| row.get(0))
            .map_err(|_| "数据库行数校验失败".to_string())?;
        if count < 0 {
            return Err("数据库行数校验失败".to_string());
        }
        out.insert(name, count as u64);
    }
    Ok(out)
}

fn validate_schema(db: &crate::db::Db) -> Result<(), String> {
    let conn = db.conn.lock().map_err(|_| "数据库 schema 校验不可用".to_string())?;
    let mut stmt = conn
        .prepare(
            "SELECT type, name FROM sqlite_master
             WHERE type IN ('table','view','trigger') AND name NOT LIKE 'sqlite_%'
             ORDER BY type, name",
        )
        .map_err(|_| "数据库 schema 无效".to_string())?;
    let objects = stmt
        .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))
        .map_err(|_| "数据库 schema 无效".to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| "数据库 schema 无效".to_string())?;
    let expected_tables: std::collections::BTreeSet<&str> = REQUIRED_TABLES.into_iter().collect();
    let actual_tables: std::collections::BTreeSet<&str> = objects
        .iter()
        .filter_map(|(kind, name)| (kind == "table").then_some(name.as_str()))
        .collect();
    if objects.iter().any(|(kind, _)| kind != "table") || actual_tables != expected_tables {
        return Err("v2 备份包含未知数据库对象".to_string());
    }
    for (table, expected) in EXPECTED_COLUMNS {
        let pragma = format!("PRAGMA table_info({})", quoted_identifier(table));
        let mut columns_stmt = conn
            .prepare(&pragma)
            .map_err(|_| "数据库 schema 无效".to_string())?;
        let columns = columns_stmt
            .query_map([], |row| row.get::<_, String>(1))
            .map_err(|_| "数据库 schema 无效".to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| "数据库 schema 无效".to_string())?;
        if columns != expected {
            return Err("v2 备份数据库列结构无效".to_string());
        }
    }
    Ok(())
}

fn decode_salt(params: &crypto::KdfParams) -> Result<Vec<u8>, String> {
    crypto::b64_decode(&params.salt)
        .ok()
        .filter(|salt| salt.len() >= 8)
        .ok_or_else(|| "v2 安全元数据无效".to_string())
}

fn validate_wrapped_blob(value: &str) -> Result<(), String> {
    let encoded = value
        .strip_prefix(crypto::WRAP_DEK_PREFIX)
        .ok_or_else(|| "v2 安全元数据无效".to_string())?;
    let raw = BASE64
        .decode(encoded)
        .map_err(|_| "v2 安全元数据无效".to_string())?;
    if raw.len() < 12 + 16 + 1 {
        return Err("v2 安全元数据无效".to_string());
    }
    Ok(())
}

fn validate_security_metadata(db: &crate::db::Db) -> Result<(), String> {
    if db
        .get_setting("security_version")
        .map_err(|_| "v2 安全元数据无效".to_string())?
        .as_deref()
        != Some(migration::SECURITY_VERSION_V2)
    {
        return Err("v2 安全元数据无效".to_string());
    }
    let params_m: crypto::KdfParams = serde_json::from_str(
        &db.get_setting("kdf_params_m")
            .map_err(|_| "v2 安全元数据无效".to_string())?
            .ok_or_else(|| "v2 安全元数据无效".to_string())?,
    )
    .map_err(|_| "v2 安全元数据无效".to_string())?;
    let params_r: crypto::KdfParams = serde_json::from_str(
        &db.get_setting("kdf_params_r")
            .map_err(|_| "v2 安全元数据无效".to_string())?
            .ok_or_else(|| "v2 安全元数据无效".to_string())?,
    )
    .map_err(|_| "v2 安全元数据无效".to_string())?;
    if params_m.algo != "argon2id"
        || params_m.version != 1
        || params_m.m_cost == 0
        || params_m.t_cost == 0
        || params_m.p_cost == 0
        || params_r.algo != "hkdf-sha256"
        || params_r.version != 1
    {
        return Err("v2 安全元数据无效".to_string());
    }
    let _ = decode_salt(&params_m)?;
    let _ = decode_salt(&params_r)?;
    validate_wrapped_blob(
        &db.get_setting("wrapped_dek_m")
            .map_err(|_| "v2 安全元数据无效".to_string())?
            .ok_or_else(|| "v2 安全元数据无效".to_string())?,
    )?;
    validate_wrapped_blob(
        &db.get_setting("wrapped_dek_r")
            .map_err(|_| "v2 安全元数据无效".to_string())?
            .ok_or_else(|| "v2 安全元数据无效".to_string())?,
    )?;
    for legacy_key in [
        "master_password_hash",
        "master_password_salt",
        "recovery_phrase_encrypted",
    ] {
        if db
            .get_setting(legacy_key)
            .map_err(|_| "v2 安全元数据无效".to_string())?
            .is_some()
        {
            return Err("v2 安全模型混入 legacy metadata".to_string());
        }
    }
    Ok(())
}

fn validate_password_rows(db: &crate::db::Db, dek: &[u8]) -> Result<(), String> {
    let conn = db.conn.lock().map_err(|_| "密码数据校验不可用".to_string())?;
    let mut stmt = conn
        .prepare("SELECT record_uuid, password FROM passwords ORDER BY id")
        .map_err(|_| "密码数据 schema 无效".to_string())?;
    let rows = stmt
        .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))
        .map_err(|_| "密码数据校验失败".to_string())?;
    for row in rows {
        let (record_uuid, ciphertext) = row.map_err(|_| "密码数据校验失败".to_string())?;
        let parsed = uuid::Uuid::parse_str(&record_uuid)
            .map_err(|_| "密码记录 UUID 无效".to_string())?;
        if parsed.get_version_num() != 4 || !ciphertext.starts_with(crypto::PASSWORD_PREFIX_DW2) {
            return Err("密码记录安全格式无效".to_string());
        }
        let _plaintext = Zeroizing::new(
            crypto::decrypt_password_dw2(&ciphertext, dek, &record_uuid)
                .map_err(|_| "密码数据校验失败".to_string())?,
        );
    }
    Ok(())
}

fn validate_snapshot(
    db: &crate::db::Db,
    envelope_counts: &BTreeMap<String, u64>,
) -> Result<(), String> {
    validate_schema(db)?;
    let actual = table_counts(db)?;
    for required in REQUIRED_TABLES {
        if !actual.contains_key(required) {
            return Err("v2 备份缺少必要业务表".to_string());
        }
    }
    if &actual != envelope_counts {
        return Err("v2 备份行数校验失败".to_string());
    }
    validate_security_metadata(db)
}

fn stats_from_counts(
    counts: &BTreeMap<String, u64>,
    database_bytes: usize,
    trash_passwords: usize,
) -> V2BackupStats {
    let get = |name: &str| counts.get(name).copied().unwrap_or(0) as usize;
    V2BackupStats {
        settings: get("settings"),
        categories: get("app_categories"),
        apps: get("apps"),
        passwords: get("passwords"),
        snippets: get("snippets"),
        temps: get("temp_contents"),
        pinned: get("pinned_items"),
        trash_passwords,
        database_bytes,
    }
}

fn trash_password_count(db: &crate::db::Db) -> Result<usize, String> {
    let conn = db.conn.lock().map_err(|_| "数据库校验不可用".to_string())?;
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM passwords WHERE deleted_at IS NOT NULL",
            [],
            |row| row.get(0),
        )
        .map_err(|_| "数据库行数校验失败".to_string())?;
    Ok(count.max(0) as usize)
}

pub fn export_v2_to_path(
    state: &AppState,
    master_password: &str,
    output_path: &Path,
) -> Result<V2BackupStats, String> {
    ensure_existing_v2(state)?;
    let _gate = state.lock_master_wrap_gate()?;
    let active_dek = state.stable_dek()?;
    validate_export_target(state, output_path)?;
    let parent = output_path
        .parent()
        .ok_or_else(|| "备份路径无效".to_string())?;
    std::fs::create_dir_all(parent).map_err(|_| "无法创建备份目录".to_string())?;
    let snapshot_path = unique_temp_path(parent, "snapshot");
    let snapshot_guard = TempDbGuard::new(snapshot_path.clone());

    {
        let db = state.db.lock().map_err(|_| "数据库不可用".to_string())?;
        let verified_dek = migration::unlock_v2_core(&db, master_password)
            .map_err(|_| "主密码错误".to_string())?;
        if verified_dek.as_slice() != active_dek.as_slice() {
            return Err("当前安全状态不一致".to_string());
        }
        snapshot_database(&db, &snapshot_path)?;
    }

    let snapshot = migration::open_existing_v2_db(&snapshot_path)
        .map_err(|_| "数据库快照校验失败".to_string())?;
    let counts = table_counts(&snapshot)?;
    validate_snapshot(&snapshot, &counts)?;
    validate_password_rows(&snapshot, active_dek.as_slice())?;
    let trash = trash_password_count(&snapshot)?;
    drop(snapshot);

    let database = std::fs::read(&snapshot_path)
        .map_err(|_| "无法读取数据库快照".to_string())?;
    let envelope = V2BackupEnvelope {
        backup_version: BACKUP_VERSION,
        security_model: SECURITY_MODEL.to_string(),
        generated_at: chrono::Utc::now().timestamp_millis(),
        database_size: database.len() as u64,
        database_sha256: crypto::b64_encode(&Sha256::digest(&database)),
        table_counts: counts.clone(),
        database_b64: BASE64.encode(&database),
    };
    let payload = serde_json::to_vec(&envelope)
        .map_err(|_| "无法生成 v2 备份".to_string())?;
    let encrypted = backup::encrypt_backup_v2(&payload, master_password)?;
    std::fs::write(output_path, encrypted).map_err(|_| "无法写入备份文件".to_string())?;
    drop(snapshot_guard);
    Ok(stats_from_counts(&counts, database.len(), trash))
}

fn decode_envelope(bytes: &[u8], master_password: &str) -> Result<V2BackupEnvelope, String> {
    match backup::classify_backup(bytes) {
        backup::BackupKind::StableDekV2 => {}
        backup::BackupKind::LegacyV1 => {
            return Err("v2 密码库不能导入 legacy 备份；请先在 legacy 版本恢复后再迁移".to_string())
        }
        backup::BackupKind::Unknown => return Err("未知或不受支持的备份格式".to_string()),
    }
    let payload = backup::decrypt_backup_v2(bytes, master_password)?;
    let envelope: V2BackupEnvelope = serde_json::from_slice(&payload)
        .map_err(|_| "v2 备份清单损坏".to_string())?;
    if envelope.backup_version != BACKUP_VERSION {
        return Err("不支持的未来备份版本".to_string());
    }
    if envelope.security_model != SECURITY_MODEL {
        return Err("备份安全模型不匹配".to_string());
    }
    Ok(envelope)
}

pub fn restore_v2_from_path(
    state: &AppState,
    backup_path: &Path,
    master_password: &str,
) -> Result<V2BackupStats, String> {
    restore_v2_from_path_inject(state, backup_path, master_password, None)
}

pub fn restore_v2_from_path_inject(
    state: &AppState,
    backup_path: &Path,
    master_password: &str,
    fail_at: Option<V2RestoreFailPoint>,
) -> Result<V2BackupStats, String> {
    ensure_existing_v2(state)?;
    let _gate = state.lock_master_wrap_gate()?;
    let fail = |point| fail_at == Some(point);
    let metadata = std::fs::metadata(backup_path).map_err(|_| "备份文件不存在".to_string())?;
    if metadata.len() > MAX_BACKUP_BYTES {
        return Err("备份文件过大".to_string());
    }
    let backup_bytes = std::fs::read(backup_path).map_err(|_| "无法读取备份文件".to_string())?;
    let envelope = decode_envelope(&backup_bytes, master_password)?;
    if fail(V2RestoreFailPoint::AfterDecrypt) {
        return Err("injected:AfterDecrypt".to_string());
    }
    let database = BASE64
        .decode(&envelope.database_b64)
        .map_err(|_| "v2 数据库快照损坏".to_string())?;
    if database.len() as u64 != envelope.database_size
        || crypto::b64_encode(&Sha256::digest(&database)) != envelope.database_sha256
    {
        return Err("v2 数据库快照校验失败".to_string());
    }
    let app_dir = state
        .db_path
        .parent()
        .ok_or_else(|| "数据库路径无效".to_string())?;
    let temp_path = unique_temp_path(app_dir, "restore");
    let temp_guard = TempDbGuard::new(temp_path.clone());
    let mut temp_file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp_path)
        .map_err(|_| "无法创建恢复临时库".to_string())?;
    temp_file
        .write_all(&database)
        .and_then(|_| temp_file.sync_all())
        .map_err(|_| "无法写入恢复临时库".to_string())?;
    drop(temp_file);
    if fail(V2RestoreFailPoint::AfterTempWrite) {
        return Err("injected:AfterTempWrite".to_string());
    }

    let restored = migration::open_existing_v2_db(&temp_path)
        .map_err(|_| "恢复数据库完整性或 schema 无效".to_string())?;
    validate_snapshot(&restored, &envelope.table_counts)?;
    if fail(V2RestoreFailPoint::AfterSecurityValidation) {
        return Err("injected:AfterSecurityValidation".to_string());
    }
    let restored_dek = migration::unlock_v2_core(&restored, master_password)
        .map_err(|_| "主密码错误，或备份安全数据损坏".to_string())?;
    validate_password_rows(&restored, restored_dek.as_slice())?;
    if fail(V2RestoreFailPoint::AfterPasswordVerification) {
        return Err("injected:AfterPasswordVerification".to_string());
    }
    let trash = trash_password_count(&restored)?;
    let stats = stats_from_counts(&envelope.table_counts, database.len(), trash);
    if fail(V2RestoreFailPoint::BeforeActivation) {
        return Err("injected:BeforeActivation".to_string());
    }

    // SQLite Online Backup API 在 destination 上使用单一写事务：失败或进程中断由
    // SQLite journal/WAL 回滚，只有完整复制成功后才对正式 DB 可见。
    state.activate_validated_v2_restore(&restored, restored_dek)?;
    drop(restored);
    drop(temp_guard);
    Ok(stats)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ActiveKey;
    use rusqlite::params;
    use std::sync::Mutex;

    const MASTER: &str = "Phase2C-Master!";
    const DEST_MASTER: &str = "Phase2C-Destination!";

    fn test_dir(label: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "drawer_phase2c_{}_{}_{}",
            label,
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    fn cleanup_dir(path: &Path) {
        let _ = std::fs::remove_dir_all(path);
    }

    fn create_state(label: &str, master: &str) -> (PathBuf, AppState, Vec<String>, Vec<u8>) {
        let dir = test_dir(label);
        let prepared = migration::prepare_fresh_v2(&dir, master).unwrap();
        let words = prepared.mnemonic.clone();
        let expected_dek = prepared.dek.to_vec();
        migration::finalize_prepared_v2(&prepared.tmp_path, &prepared.v2_path).unwrap();
        drop(prepared.setup_lock);
        let _ = std::fs::remove_file(prepared.setup_lock_path);
        let db_path = dir.join(migration::V2_DB_FILENAME);
        let state = AppState {
            db: Mutex::new(migration::open_existing_v2_db(&db_path).unwrap()),
            db_path,
            startup_mode: Mutex::new(StartupMode::ExistingV2),
            pending_v2: Mutex::new(None),
            pending_recovery_rotation: Mutex::new(None),
            pending_legacy_migration: std::sync::Mutex::new(None),
            master_wrap_gate: Mutex::new(()),
            key: Mutex::new(Some(ActiveKey::StableDek(Zeroizing::new(expected_dek.clone())))),
        };
        (dir, state, words, expected_dek)
    }

    fn populate_fixture(state: &AppState, dek: &[u8]) -> Vec<(String, String)> {
        let records = [
            ("936cc5af-b8c4-4659-a10f-cea1005c8348", "alpha-secret", None),
            ("94bb1e2c-4e07-43e2-b9fb-77019a4be579", "trash-secret", Some(12345i64)),
        ];
        let db = state.db.lock().unwrap();
        let mut snapshots = Vec::new();
        for (index, (record_uuid, plaintext, deleted_at)) in records.iter().enumerate() {
            let ciphertext = crypto::encrypt_password_dw2(plaintext, dek, record_uuid).unwrap();
            let id = db
                .create_password_v2(
                    record_uuid,
                    &format!("fixture-{index}"),
                    "fixture-user",
                    &ciphertext,
                    "https://example.invalid",
                    "notes",
                )
                .unwrap();
            if let Some(deleted_at) = deleted_at {
                db.conn
                    .lock()
                    .unwrap()
                    .execute(
                        "UPDATE passwords SET deleted_at=?1, use_count=7, last_used_at=8 WHERE id=?2",
                        params![deleted_at, id],
                    )
                    .unwrap();
            }
            snapshots.push((record_uuid.to_string(), ciphertext));
        }
        let conn = db.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO pinned_items (item_type,item_id,sort_order) VALUES ('password',1,3)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO app_categories (name,icon,sort_order) VALUES ('Phase2C','x',99)",
            [],
        )
        .unwrap();
        let category_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO apps (name,path,category_id,created_at,use_count,last_used_at,app_type,app_subtype,deleted_at)
             VALUES ('Fixture App','C:/fixture.exe',?1,1,2,3,'app','utility',NULL)",
            params![category_id],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO snippets (title,content,language,tags,created_at,updated_at,use_count,last_used_at,deleted_at)
             VALUES ('Fixture Snippet','body','text','tag',1,2,3,4,NULL)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO temp_contents (text,created_at,expires_at,deleted_at) VALUES ('temp',1,999999,NULL)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO settings (key,value) VALUES ('ui.phase2c','enabled')",
            [],
        )
        .unwrap();
        snapshots
    }

    fn file_hash(path: &Path) -> Vec<u8> {
        Sha256::digest(std::fs::read(path).unwrap()).to_vec()
    }

    fn password_rows(state: &AppState) -> Vec<(String, String)> {
        let db = state.db.lock().unwrap();
        let conn = db.conn.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT record_uuid,password FROM passwords ORDER BY id")
            .unwrap();
        stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    }

    fn security_snapshot(state: &AppState) -> Vec<(String, String)> {
        let db = state.db.lock().unwrap();
        [
            "security_version",
            "kdf_params_m",
            "wrapped_dek_m",
            "kdf_params_r",
            "wrapped_dek_r",
        ]
        .iter()
        .map(|key| ((*key).to_string(), db.get_setting(key).unwrap().unwrap()))
        .collect()
    }

    fn make_backup(label: &str) -> (PathBuf, AppState, Vec<String>, Vec<u8>, PathBuf) {
        let (dir, state, words, dek) = create_state(label, MASTER);
        populate_fixture(&state, &dek);
        let backup_path = dir.join("fixture.drawerbox");
        export_v2_to_path(&state, MASTER, &backup_path).unwrap();
        (dir, state, words, dek, backup_path)
    }

    fn contains(haystack: &[u8], needle: &[u8]) -> bool {
        !needle.is_empty() && haystack.windows(needle.len()).any(|window| window == needle)
    }

    fn mutate_database_in_backup(
        backup_path: &Path,
        output: &Path,
        sql: &str,
    ) {
        let bytes = std::fs::read(backup_path).unwrap();
        let payload = backup::decrypt_backup_v2(&bytes, MASTER).unwrap();
        let mut envelope: V2BackupEnvelope = serde_json::from_slice(&payload).unwrap();
        let database = BASE64.decode(&envelope.database_b64).unwrap();
        let temp = output.with_extension("db.tmp");
        std::fs::write(&temp, database).unwrap();
        {
            let conn = Connection::open(&temp).unwrap();
            conn.execute_batch(sql).unwrap();
        }
        let mutated = std::fs::read(&temp).unwrap();
        let db = migration::open_existing_v2_db(&temp).ok();
        if let Some(db) = db.as_ref() {
            envelope.table_counts = table_counts(db).unwrap();
        }
        drop(db);
        envelope.database_size = mutated.len() as u64;
        envelope.database_sha256 = crypto::b64_encode(&Sha256::digest(&mutated));
        envelope.database_b64 = BASE64.encode(mutated);
        let payload = serde_json::to_vec(&envelope).unwrap();
        std::fs::write(output, backup::encrypt_backup_v2(&payload, MASTER).unwrap()).unwrap();
        let _ = std::fs::remove_file(temp);
    }

    #[test]
    fn b1_v2_backup_contains_complete_ciphertext_snapshot_without_secrets() {
        let (dir, state, words, dek, backup_path) = make_backup("b1");
        let bytes = std::fs::read(&backup_path).unwrap();
        assert_eq!(backup::classify_backup(&bytes), backup::BackupKind::StableDekV2);
        let payload = backup::decrypt_backup_v2(&bytes, MASTER).unwrap();
        let envelope: V2BackupEnvelope = serde_json::from_slice(&payload).unwrap();
        assert_eq!(envelope.backup_version, 2);
        assert_eq!(envelope.security_model, SECURITY_MODEL);
        for table in REQUIRED_TABLES {
            assert!(envelope.table_counts.contains_key(table));
        }
        let database = BASE64.decode(envelope.database_b64).unwrap();
        assert!(!contains(&database, MASTER.as_bytes()));
        assert!(!contains(&database, words.join(" ").as_bytes()));
        assert!(!contains(&database, &dek));
        assert!(!contains(&database, b"alpha-secret"));
        drop(state);
        cleanup_dir(&dir);
    }

    #[test]
    fn b2_backup_does_not_modify_source_database() {
        let (dir, state, _, dek) = create_state("b2", MASTER);
        populate_fixture(&state, &dek);
        let before = file_hash(&state.db_path);
        export_v2_to_path(&state, MASTER, &dir.join("b2.drawerbox")).unwrap();
        assert_eq!(file_hash(&state.db_path), before);
        assert!(export_v2_to_path(&state, MASTER, &state.db_path).is_err());
        assert_eq!(file_hash(&state.db_path), before);
        drop(state);
        cleanup_dir(&dir);
    }

    #[test]
    fn b3_normal_restore_unlocks_master_recovery_and_all_business_data() {
        let (source_dir, source, words, expected, backup_path) = make_backup("b3_source");
        let source_counts = table_counts(&source.db.lock().unwrap()).unwrap();
        let (dest_dir, dest, _, _) = create_state("b3_dest", DEST_MASTER);
        let stats = restore_v2_from_path(&dest, &backup_path, MASTER).unwrap();
        assert_eq!(stats.passwords, 2);
        assert_eq!(stats.pinned, 1);
        assert_eq!(table_counts(&dest.db.lock().unwrap()).unwrap(), source_counts);
        let db = dest.db.lock().unwrap();
        assert_eq!(migration::unlock_v2_core(&db, MASTER).unwrap().to_vec(), expected);
        assert_eq!(migration::recover_v2_core(&db, &words.join(" ")).unwrap().to_vec(), expected);
        drop(db);
        assert_eq!(dest.stable_dek().unwrap().to_vec(), expected);
        drop(source);
        drop(dest);
        cleanup_dir(&source_dir);
        cleanup_dir(&dest_dir);
    }

    #[test]
    fn b4_password_rows_are_byte_for_byte_identical() {
        let (source_dir, source, _, _, backup_path) = make_backup("b4_source");
        let before = password_rows(&source);
        let (dest_dir, dest, _, _) = create_state("b4_dest", DEST_MASTER);
        restore_v2_from_path(&dest, &backup_path, MASTER).unwrap();
        assert_eq!(password_rows(&dest), before);
        drop(source);
        drop(dest);
        cleanup_dir(&source_dir);
        cleanup_dir(&dest_dir);
    }

    #[test]
    fn b5_recovery_metadata_and_phrase_remain_valid() {
        let (source_dir, source, words, expected, backup_path) = make_backup("b5_source");
        let before = security_snapshot(&source);
        let (dest_dir, dest, _, _) = create_state("b5_dest", DEST_MASTER);
        restore_v2_from_path(&dest, &backup_path, MASTER).unwrap();
        assert_eq!(security_snapshot(&dest), before);
        let db = dest.db.lock().unwrap();
        assert_eq!(migration::recover_v2_core(&db, &words.join(" ")).unwrap().to_vec(), expected);
        drop(db);
        drop(source);
        drop(dest);
        cleanup_dir(&source_dir);
        cleanup_dir(&dest_dir);
    }

    #[test]
    fn b6_wrong_master_fails_without_changing_destination() {
        let (source_dir, source, _, _, backup_path) = make_backup("b6_source");
        let (dest_dir, dest, _, _) = create_state("b6_dest", DEST_MASTER);
        let before = file_hash(&dest.db_path);
        assert!(restore_v2_from_path(&dest, &backup_path, "wrong-password").is_err());
        assert_eq!(file_hash(&dest.db_path), before);
        assert!(migration::unlock_v2_core(&dest.db.lock().unwrap(), DEST_MASTER).is_ok());
        drop(source);
        drop(dest);
        cleanup_dir(&source_dir);
        cleanup_dir(&dest_dir);
    }

    #[test]
    fn b7_header_body_and_inner_security_tampering_are_rejected() {
        let (source_dir, source, _, _, backup_path) = make_backup("b7_source");
        let (dest_dir, dest, _, _) = create_state("b7_dest", DEST_MASTER);
        let baseline = file_hash(&dest.db_path);
        let original = std::fs::read(&backup_path).unwrap();
        for (index, label) in [(0usize, "magic"), (54usize, "version"), (backup::HEADER_SIZE, "body")] {
            let path = source_dir.join(format!("tampered-{label}.drawerbox"));
            let mut bytes = original.clone();
            bytes[index] ^= 1;
            std::fs::write(&path, bytes).unwrap();
            assert!(restore_v2_from_path(&dest, &path, MASTER).is_err());
        }
        for (name, sql) in [
            ("master", "UPDATE settings SET value='DWK1:AAAA' WHERE key='wrapped_dek_m'"),
            ("password", "UPDATE passwords SET password='DW2:AAAA' WHERE id=(SELECT MIN(id) FROM passwords)"),
            ("metadata", "UPDATE settings SET value='not-json' WHERE key='kdf_params_r'"),
            (
                "schema",
                "CREATE TRIGGER unexpected_trigger AFTER INSERT ON settings BEGIN DELETE FROM settings WHERE key='never'; END",
            ),
        ] {
            let path = source_dir.join(format!("inner-{name}.drawerbox"));
            mutate_database_in_backup(&backup_path, &path, sql);
            assert!(restore_v2_from_path(&dest, &path, MASTER).is_err());
        }
        assert_eq!(file_hash(&dest.db_path), baseline);
        drop(source);
        drop(dest);
        cleanup_dir(&source_dir);
        cleanup_dir(&dest_dir);
    }

    #[test]
    fn b8_truncated_or_corrupt_backup_is_rejected() {
        let (source_dir, source, _, _, backup_path) = make_backup("b8_source");
        let (dest_dir, dest, _, _) = create_state("b8_dest", DEST_MASTER);
        let baseline = file_hash(&dest.db_path);
        let bytes = std::fs::read(&backup_path).unwrap();
        for length in [0usize, 20, backup::HEADER_SIZE + 5, bytes.len() / 2] {
            let path = source_dir.join(format!("truncated-{length}.drawerbox"));
            std::fs::write(&path, &bytes[..length.min(bytes.len())]).unwrap();
            assert!(restore_v2_from_path(&dest, &path, MASTER).is_err());
        }
        assert_eq!(file_hash(&dest.db_path), baseline);
        drop(source);
        drop(dest);
        cleanup_dir(&source_dir);
        cleanup_dir(&dest_dir);
    }

    #[test]
    fn b9_unknown_future_backup_version_fails_closed() {
        let (source_dir, source, _, _, backup_path) = make_backup("b9_source");
        let bytes = std::fs::read(&backup_path).unwrap();
        let payload = backup::decrypt_backup_v2(&bytes, MASTER).unwrap();
        let mut envelope: V2BackupEnvelope = serde_json::from_slice(&payload).unwrap();
        envelope.backup_version = 99;
        let future = source_dir.join("future.drawerbox");
        std::fs::write(
            &future,
            backup::encrypt_backup_v2(&serde_json::to_vec(&envelope).unwrap(), MASTER).unwrap(),
        )
        .unwrap();
        let (dest_dir, dest, _, _) = create_state("b9_dest", DEST_MASTER);
        let baseline = file_hash(&dest.db_path);
        assert!(restore_v2_from_path(&dest, &future, MASTER).is_err());
        assert_eq!(file_hash(&dest.db_path), baseline);
        drop(source);
        drop(dest);
        cleanup_dir(&source_dir);
        cleanup_dir(&dest_dir);
    }

    #[test]
    fn b10_legacy_backup_is_explicitly_rejected_by_v2_restore() {
        let dir = test_dir("b10");
        let legacy_path = dir.join("legacy.drawerbox");
        std::fs::write(&legacy_path, backup::encrypt_backup(b"{}", MASTER).unwrap()).unwrap();
        let (dest_dir, dest, _, _) = create_state("b10_dest", DEST_MASTER);
        let baseline = file_hash(&dest.db_path);
        let error = restore_v2_from_path(&dest, &legacy_path, MASTER).unwrap_err();
        assert!(error.contains("legacy"));
        assert_eq!(file_hash(&dest.db_path), baseline);
        drop(dest);
        cleanup_dir(&dir);
        cleanup_dir(&dest_dir);
    }

    #[test]
    fn b11_cross_path_restore_has_no_source_path_or_device_dependency() {
        let (source_dir, source, _, expected, backup_path) = make_backup("b11_source_a");
        let copied_dir = test_dir("b11_transfer");
        let copied_backup = copied_dir.join("from-machine-a.drawerbox");
        std::fs::copy(&backup_path, &copied_backup).unwrap();
        let (dest_dir, dest, _, _) = create_state("b11_machine_b", DEST_MASTER);
        assert_ne!(source.db_path.parent(), dest.db_path.parent());
        restore_v2_from_path(&dest, &copied_backup, MASTER).unwrap();
        assert_eq!(dest.stable_dek().unwrap().to_vec(), expected);
        drop(source);
        drop(dest);
        cleanup_dir(&source_dir);
        cleanup_dir(&copied_dir);
        cleanup_dir(&dest_dir);
    }

    #[test]
    fn b12_all_pre_activation_failures_preserve_formal_database() {
        let points = [
            V2RestoreFailPoint::AfterDecrypt,
            V2RestoreFailPoint::AfterTempWrite,
            V2RestoreFailPoint::AfterSecurityValidation,
            V2RestoreFailPoint::AfterPasswordVerification,
            V2RestoreFailPoint::BeforeActivation,
        ];
        for (index, point) in points.into_iter().enumerate() {
            let (source_dir, source, _, _, backup_path) = make_backup(&format!("b12_source_{index}"));
            let (dest_dir, dest, _, expected_dest) = create_state(&format!("b12_dest_{index}"), DEST_MASTER);
            let baseline = file_hash(&dest.db_path);
            assert!(restore_v2_from_path_inject(&dest, &backup_path, MASTER, Some(point)).is_err());
            assert_eq!(file_hash(&dest.db_path), baseline);
            assert_eq!(
                migration::unlock_v2_core(&dest.db.lock().unwrap(), DEST_MASTER)
                    .unwrap()
                    .to_vec(),
                expected_dest
            );
            assert_eq!(dest.stable_dek().unwrap().to_vec(), expected_dest);
            drop(source);
            drop(dest);
            cleanup_dir(&source_dir);
            cleanup_dir(&dest_dir);
        }
    }
}

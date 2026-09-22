//! Phase 2D 真实 legacy snapshot 隔离验收。
//!
//! 该模块只在 `cfg(test)` 下编译。它不实现另一套迁移，而是用不可变 snapshot
//! 驱动 production migration / backup / restore API，并只输出计数与不可逆摘要。

use crate::backup_v2;
use crate::crypto;
use crate::db::Db;
use crate::migration::{self, MigrationFailPoint};
use crate::{ActiveKey, AppState, SecurityModel, StartupMode};
use rusqlite::backup::Backup;
use rusqlite::types::ValueRef;
use rusqlite::{Connection, OpenFlags};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashSet};
use std::fmt::Write as _;
use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;
use zeroize::{Zeroize, Zeroizing};

fn fail(message: &str) -> Result<(), String> {
    Err(message.to_string())
}

fn require(condition: bool, message: &str) -> Result<(), String> {
    if condition {
        Ok(())
    } else {
        fail(message)
    }
}

fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

fn file_sha256(path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|_| "无法读取验收文件".to_string())?;
    Ok(hex(&Sha256::digest(bytes)))
}

fn readonly_connection(path: &Path) -> Result<Connection, String> {
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|_| "无法只读打开验收数据库".to_string())?;
    conn.pragma_update(None, "query_only", true)
        .map_err(|_| "无法启用只读保护".to_string())?;
    Ok(conn)
}

fn online_snapshot(source: &Path, destination: &Path) -> Result<(), String> {
    if destination.exists() {
        return Err("验收目标已存在".to_string());
    }
    let source = readonly_connection(source)?;
    let mut destination =
        Connection::open(destination).map_err(|_| "无法创建验收数据库副本".to_string())?;
    let backup = Backup::new(&source, &mut destination)
        .map_err(|_| "无法建立 SQLite Online Backup".to_string())?;
    backup
        .run_to_completion(32, Duration::from_millis(5), None)
        .map_err(|_| "SQLite Online Backup 失败".to_string())?;
    drop(backup);
    Ok(())
}

fn table_counts(conn: &Connection) -> Result<BTreeMap<String, i64>, String> {
    let mut names = conn
        .prepare(
            "SELECT name FROM sqlite_master
             WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
        )
        .map_err(|_| "无法读取表清单".to_string())?;
    let names = names
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|_| "无法读取表清单".to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| "无法读取表清单".to_string())?;
    let mut counts = BTreeMap::new();
    for name in names {
        let quoted = format!("\"{}\"", name.replace('"', "\"\""));
        let count = conn
            .query_row(&format!("SELECT COUNT(*) FROM {quoted}"), [], |row| {
                row.get(0)
            })
            .map_err(|_| "无法读取表计数".to_string())?;
        counts.insert(name, count);
    }
    Ok(counts)
}

fn canonical_query_sha256(conn: &Connection, sql: &str) -> Result<String, String> {
    let mut stmt = conn
        .prepare(sql)
        .map_err(|_| "无法建立数据指纹".to_string())?;
    let columns = stmt.column_count();
    let mut rows = stmt.query([]).map_err(|_| "无法建立数据指纹".to_string())?;
    let mut digest = Sha256::new();
    while let Some(row) = rows.next().map_err(|_| "无法建立数据指纹".to_string())? {
        digest.update([0xfe]);
        for column in 0..columns {
            match row
                .get_ref(column)
                .map_err(|_| "无法建立数据指纹".to_string())?
            {
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
            }
        }
    }
    Ok(hex(&digest.finalize()))
}

fn schema_sha256(conn: &Connection) -> Result<String, String> {
    canonical_query_sha256(
        conn,
        "SELECT type,name,tbl_name,sql FROM sqlite_master
         WHERE name NOT LIKE 'sqlite_%' ORDER BY type,name",
    )
}

fn ordinary_settings_sql() -> &'static str {
    "SELECT key,value FROM settings
     WHERE key NOT IN (
       'master_password_hash','master_password_salt','recovery_phrase_encrypted',
       'security_version','kdf_params_m','wrapped_dek_m','kdf_params_r','wrapped_dek_r'
     ) ORDER BY key"
}

fn compare_business_data(legacy: &Connection, v2: &Connection) -> Result<(), String> {
    let comparisons = [
        (
            "password metadata",
            "SELECT id,title,username,url,notes,created_at,updated_at,use_count,last_used_at,deleted_at FROM passwords ORDER BY id",
        ),
        (
            "apps",
            "SELECT id,name,path,icon_path,args,category_id,created_at,use_count,last_used_at,app_type,app_subtype,deleted_at FROM apps ORDER BY id",
        ),
        (
            "app_categories",
            "SELECT id,name,icon,sort_order FROM app_categories ORDER BY id",
        ),
        (
            "snippets",
            "SELECT id,title,content,language,tags,created_at,updated_at,use_count,last_used_at,deleted_at FROM snippets ORDER BY id",
        ),
        (
            "temp_contents",
            "SELECT id,text,created_at,expires_at,deleted_at FROM temp_contents ORDER BY id",
        ),
        (
            "pinned_items",
            "SELECT id,item_type,item_id,sort_order FROM pinned_items ORDER BY id",
        ),
    ];
    for (label, sql) in comparisons {
        require(
            canonical_query_sha256(legacy, sql)? == canonical_query_sha256(v2, sql)?,
            &format!("{label} 迁移前后不一致"),
        )?;
    }
    require(
        canonical_query_sha256(legacy, ordinary_settings_sql())?
            == canonical_query_sha256(v2, ordinary_settings_sql())?,
        "普通 settings 迁移前后不一致",
    )
}

fn legacy_key(legacy: &Db, master_password: &str) -> Result<Zeroizing<Vec<u8>>, String> {
    let salt = legacy
        .get_setting("master_password_salt")
        .map_err(|_| "legacy 安全元数据无效".to_string())?
        .ok_or_else(|| "legacy 安全元数据无效".to_string())?;
    let verifier = legacy
        .get_setting("master_password_hash")
        .map_err(|_| "legacy 安全元数据无效".to_string())?
        .ok_or_else(|| "legacy 安全元数据无效".to_string())?;
    let salt =
        Zeroizing::new(crypto::b64_decode(&salt).map_err(|_| "legacy 安全元数据无效".to_string())?);
    require(
        crypto::hash_password(master_password, salt.as_slice()) == verifier,
        "主密码验证失败",
    )?;
    Ok(Zeroizing::new(crypto::derive_key(
        master_password,
        salt.as_slice(),
    )))
}

fn verify_passwords(
    legacy: &Db,
    v2: &Db,
    master_password: &str,
    stable_dek: &[u8],
) -> Result<usize, String> {
    let key = legacy_key(legacy, master_password)?;
    let legacy_conn = legacy
        .conn
        .lock()
        .map_err(|_| "legacy 数据库不可用".to_string())?;
    let mut stmt = legacy_conn
        .prepare("SELECT id,password FROM passwords ORDER BY id")
        .map_err(|_| "无法读取 legacy 密码行".to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|_| "无法读取 legacy 密码行".to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| "无法读取 legacy 密码行".to_string())?;
    drop(stmt);
    drop(legacy_conn);

    let v2_conn = v2.conn.lock().map_err(|_| "v2 数据库不可用".to_string())?;
    let mut seen = HashSet::new();
    let mut checked = 0usize;
    let mut legacy_cipher_digest = Sha256::new();
    for (id, legacy_ciphertext) in rows {
        legacy_cipher_digest.update((legacy_ciphertext.len() as u64).to_le_bytes());
        legacy_cipher_digest.update(legacy_ciphertext.as_bytes());
        let legacy_plain = Zeroizing::new(
            crypto::decrypt(&legacy_ciphertext, key.as_slice())
                .map_err(|_| "legacy 密码解密失败".to_string())?,
        );
        let (record_uuid, ciphertext): (String, String) = v2_conn
            .query_row(
                "SELECT record_uuid,password FROM passwords WHERE id=?1",
                [id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(|_| "迁移后密码记录缺失".to_string())?;
        require(seen.insert(record_uuid.clone()), "record_uuid 不唯一")?;
        require(
            uuid::Uuid::parse_str(&record_uuid)
                .map(|value| value.get_version_num() == 4)
                .unwrap_or(false),
            "record_uuid 不是 UUID v4",
        )?;
        require(
            ciphertext.starts_with(crypto::PASSWORD_PREFIX_DW2),
            "迁移后密码不是 DW2 格式",
        )?;
        let v2_plain = Zeroizing::new(
            crypto::decrypt_password_dw2(&ciphertext, stable_dek, &record_uuid)
                .map_err(|_| "迁移后密码解密失败".to_string())?,
        );
        require(
            legacy_plain.as_bytes() == v2_plain.as_bytes(),
            "密码明文等价校验失败",
        )?;
        checked += 1;
    }
    let _legacy_ciphertext_manifest = legacy_cipher_digest.finalize();
    Ok(checked)
}

fn existing_state(path: &Path, dek: &[u8]) -> Result<AppState, String> {
    Ok(AppState {
        db: Mutex::new(migration::open_existing_v2_db(path)?),
        db_path: path.to_path_buf(),
        startup_mode: Mutex::new(StartupMode::ExistingV2),
        pending_v2: Mutex::new(None),
        pending_recovery_rotation: Mutex::new(None),
        pending_legacy_migration: std::sync::Mutex::new(None),
        master_wrap_gate: Mutex::new(()),
        key: Mutex::new(Some(ActiveKey::StableDek(Zeroizing::new(dek.to_vec())))),
    })
}

/// 与 production Legacy 启动一致的 AppState（db = legacy RW 连接）。
/// 驱动 production prepare → confirm 升级路径时使用。
fn legacy_production_state(path: &Path) -> Result<AppState, String> {
    Ok(AppState {
        db: Mutex::new(Db::open(path).map_err(|_| "无法打开 legacy 输入".to_string())?),
        db_path: path.to_path_buf(),
        startup_mode: Mutex::new(StartupMode::Legacy),
        pending_v2: Mutex::new(None),
        pending_recovery_rotation: Mutex::new(None),
        pending_legacy_migration: std::sync::Mutex::new(None),
        master_wrap_gate: Mutex::new(()),
        key: Mutex::new(None),
    })
}

fn exact_v2_hashes(conn: &Connection) -> Result<BTreeMap<&'static str, String>, String> {
    let queries = [
        ("settings", "SELECT * FROM settings ORDER BY key"),
        ("passwords", "SELECT * FROM passwords ORDER BY id"),
        ("app_categories", "SELECT * FROM app_categories ORDER BY id"),
        ("apps", "SELECT * FROM apps ORDER BY id"),
        ("snippets", "SELECT * FROM snippets ORDER BY id"),
        ("temp_contents", "SELECT * FROM temp_contents ORDER BY id"),
        ("pinned_items", "SELECT * FROM pinned_items ORDER BY id"),
    ];
    let mut hashes = BTreeMap::new();
    for (name, query) in queries {
        hashes.insert(name, canonical_query_sha256(conn, query)?);
    }
    Ok(hashes)
}

fn read_master_password() -> Result<Zeroizing<String>, String> {
    let mut password = Zeroizing::new(String::new());
    std::io::stdin()
        .lock()
        .read_line(&mut password)
        .map_err(|_| "无法从安全 stdin 读取主密码".to_string())?;
    while password.ends_with('\n') || password.ends_with('\r') {
        password.pop();
    }
    require(!password.is_empty(), "未收到主密码")?;
    Ok(password)
}

fn run_rehearsal_with_password(workspace: PathBuf, master_password: &str) -> Result<(), String> {
    let workspace = workspace
        .canonicalize()
        .map_err(|_| "Phase2D workspace 不存在".to_string())?;
    require(
        workspace
            .file_name()
            .and_then(|name| name.to_str())
            .map(|name| name.starts_with("drawer-phase2d-"))
            .unwrap_or(false),
        "拒绝在非 Phase2D 隔离目录运行",
    )?;
    let snapshot = workspace.join("legacy-real-snapshot.db");
    require(snapshot.is_file(), "一致性 snapshot 不存在")?;
    let snapshot_hash = file_sha256(&snapshot)?;
    let baseline = readonly_connection(&snapshot)?;
    let quick: String = baseline
        .query_row("PRAGMA quick_check", [], |row| row.get(0))
        .map_err(|_| "snapshot quick_check 失败".to_string())?;
    let integrity: String = baseline
        .query_row("PRAGMA integrity_check", [], |row| row.get(0))
        .map_err(|_| "snapshot integrity_check 失败".to_string())?;
    require(
        quick == "ok" && integrity == "ok",
        "snapshot 完整性校验失败",
    )?;
    let baseline_counts = table_counts(&baseline)?;
    let schema_hash = schema_sha256(&baseline)?;
    let password_metadata_hash = canonical_query_sha256(
        &baseline,
        "SELECT id,title,username,url,notes,created_at,updated_at,use_count,last_used_at,deleted_at FROM passwords ORDER BY id",
    )?;
    drop(baseline);

    let migration_dir = workspace.join("migration-run");
    std::fs::create_dir(&migration_dir).map_err(|_| "migration-run 已存在".to_string())?;
    let migration_input = migration_dir.join(migration::LEGACY_DB_FILENAME);
    online_snapshot(&snapshot, &migration_input)?;
    let migration_input_hash = file_sha256(&migration_input)?;
    let v2_path = migration_dir.join(migration::V2_DB_FILENAME);
    let archived = migration_dir.join(format!(
        "{}{}",
        migration::LEGACY_DB_FILENAME,
        migration::LEGACY_BACKUP_SUFFIX
    ));

    // ============================================================
    // production prepare → confirm 路径（0.3.0 正式产品语义）：
    // (a) abort-before-confirm；(b) 错误确认词 fail closed；(c) 正常 confirm
    // ============================================================
    let state = legacy_production_state(&migration_input)?;
    {
        // (a) prepare 后取消：无任何磁盘痕迹，legacy 原样
        let prepared = state.prepare_legacy_migration(master_password)?;
        state.cancel_legacy_migration(&prepared.migration_token)?;
        require(
            !migration_dir.join(migration::V2_TMP_FILENAME).exists(),
            "取消后 tmp 未清理",
        )?;
        require(
            !migration_dir.join(migration::V2_SETUP_LOCK_FILENAME).exists(),
            "取消后 setup lock 未清理",
        )?;
        require(!v2_path.exists(), "取消后不得存在正式 v2")?;
        require(
            file_sha256(&migration_input)? == migration_input_hash,
            "取消改写了 legacy 输入",
        )?;
    }
    let prepared = state.prepare_legacy_migration(master_password)?;
    let recovery_phrase = Zeroizing::new(prepared.recovery_words.join(" "));
    {
        // (b) 错误确认词：拒绝后 pending 保留、可重试
        let wrong: Vec<String> = prepared
            .confirmation_indexes
            .iter()
            .map(|_| "wrong-rehearsal-word".to_string())
            .collect();
        require(
            state
                .confirm_legacy_migration(&prepared.migration_token, &wrong)
                .is_err(),
            "错误确认词必须被拒绝",
        )?;
        require(!v2_path.exists(), "错误确认词不得激活 v2")?;
    }
    {
        // (c) 正确确认词：production confirm 激活
        let answers: Vec<String> = prepared
            .confirmation_indexes
            .iter()
            .map(|index| prepared.recovery_words[*index].clone())
            .collect();
        state.confirm_legacy_migration(&prepared.migration_token, &answers)?;
    }
    require(
        state.security_model() == crate::SecurityModel::StableDekV2,
        "confirm 后安全模型未切换",
    )?;
    require(state.is_unlocked(), "confirm 后 AppState 未安装 Stable DEK")?;
    let active_dek = state.stable_dek()?;

    require(archived.is_file(), "legacy 隔离归档不存在")?;
    require(
        file_sha256(&archived)? == migration_input_hash,
        "legacy 迁移输入被改写",
    )?;
    require(
        !migration_dir.join(migration::V2_TMP_FILENAME).exists(),
        "迁移 tmp 未消失",
    )?;
    require(
        !migration_dir.join(migration::V2_SETUP_LOCK_FILENAME).exists(),
        "setup lock 未清理",
    )?;

    // (d) source changed fail closed（独立目录，不污染主迁移）
    {
        let sc_dir = workspace.join("source-change-run");
        std::fs::create_dir(&sc_dir).map_err(|_| "source-change-run 已存在".to_string())?;
        let sc_input = sc_dir.join(migration::LEGACY_DB_FILENAME);
        online_snapshot(&snapshot, &sc_input)?;
        let sc_state = legacy_production_state(&sc_input)?;
        let sc_prepared = sc_state.prepare_legacy_migration(master_password)?;
        let sc_answers: Vec<String> = sc_prepared
            .confirmation_indexes
            .iter()
            .map(|index| sc_prepared.recovery_words[*index].clone())
            .collect();
        // 模拟用户在保存恢复短语期间数据发生变化
        let writer = Connection::open(&sc_input).map_err(|_| "无法打开变化注入连接".to_string())?;
        writer
            .execute(
                "INSERT INTO passwords (title, username, password, url, notes, created_at, updated_at)
                 VALUES ('rehearsal-late-entry', 'u', 'x', '', '', 1, 1)",
                [],
            )
            .map_err(|_| "无法注入源库变化".to_string())?;
        drop(writer);
        require(
            sc_state
                .confirm_legacy_migration(&sc_prepared.migration_token, &sc_answers)
                .is_err(),
            "source changed 必须 fail closed",
        )?;
        require(!sc_dir.join(migration::V2_DB_FILENAME).exists(), "stale v2 被激活")?;
        require(!sc_dir.join(migration::V2_TMP_FILENAME).exists(), "stale tmp 未清理")?;
        require(!sc_state.legacy_migration_status().active, "pending 未清理")?;
    }

    let legacy =
        Db::open_read_only(&archived).map_err(|_| "无法只读打开 legacy 归档".to_string())?;
    let v2 = migration::open_existing_v2_db(&v2_path)?;
    {
        let legacy_conn = legacy
            .conn
            .lock()
            .map_err(|_| "legacy 数据库不可用".to_string())?;
        let v2_conn = v2.conn.lock().map_err(|_| "v2 数据库不可用".to_string())?;
        compare_business_data(&legacy_conn, &v2_conn)?;
    }
    let checked = verify_passwords(&legacy, &v2, master_password, active_dek.as_slice())?;
    require(
        checked == *baseline_counts.get("passwords").unwrap_or(&-1) as usize,
        "密码等价校验数量与基线不一致",
    )?;

    let master_dek = migration::unlock_v2_core(&v2, master_password)?;
    require(
        master_dek.as_slice() == active_dek.as_slice(),
        "Master unlock DEK 不一致",
    )?;
    let recovery_dek = migration::recover_v2_core(&v2, recovery_phrase.as_str())?;
    require(
        recovery_dek.as_slice() == active_dek.as_slice(),
        "Recovery unlock DEK 不一致",
    )?;
    {
        let conn = v2.conn.lock().map_err(|_| "v2 数据库不可用".to_string())?;
        let legacy_security: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM settings WHERE key IN ('master_password_hash','master_password_salt','recovery_phrase_encrypted','recovery_phrase')",
                [],
                |row| row.get(0),
            )
            .map_err(|_| "无法验证 v2 安全元数据".to_string())?;
        let persisted_phrase: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM settings WHERE value=?1",
                [recovery_phrase.as_str()],
                |row| row.get(0),
            )
            .map_err(|_| "无法验证 Recovery Phrase 持久化状态".to_string())?;
        require(
            legacy_security == 0 && persisted_phrase == 0,
            "v2 混入 legacy 或 Recovery Phrase 数据",
        )?;
    }

    let source_v2_hash_before_backup = file_sha256(&v2_path)?;
    // production post-confirm 状态本身就是导出源（不再另建旁路 AppState）
    let source_state = &state;
    let backup_path = workspace.join("migration-rehearsal.drawerbox");
    let backup_stats = backup_v2::export_v2_to_path(&source_state, master_password, &backup_path)?;
    require(
        file_sha256(&v2_path)? == source_v2_hash_before_backup,
        "导出改写了 v2 源库",
    )?;
    require(backup_stats.passwords == checked, "备份密码计数不一致")?;

    let restore_dir = workspace.join("restore-target");
    std::fs::create_dir(&restore_dir).map_err(|_| "restore-target 已存在".to_string())?;
    let prepared = migration::prepare_fresh_v2(&restore_dir, master_password)?;
    let target_dek = prepared.dek.to_vec();
    migration::finalize_prepared_v2(&prepared.tmp_path, &prepared.v2_path)?;
    drop(prepared.setup_lock);
    let _ = std::fs::remove_file(prepared.setup_lock_path);
    let mut discarded_target_words = prepared.mnemonic;
    discarded_target_words.zeroize();
    let restore_path = restore_dir.join(migration::V2_DB_FILENAME);
    let restore_state = existing_state(&restore_path, &target_dek)?;
    let restored = backup_v2::restore_v2_from_path(&restore_state, &backup_path, master_password)?;
    require(restored.passwords == checked, "恢复后密码计数不一致")?;
    require(
        restore_state.stable_dek()?.as_slice() == active_dek.as_slice(),
        "恢复后 AppState DEK 不一致",
    )?;
    {
        let restored_db = restore_state
            .db
            .lock()
            .map_err(|_| "恢复数据库不可用".to_string())?;
        require(
            migration::unlock_v2_core(&restored_db, master_password)?.as_slice()
                == active_dek.as_slice(),
            "恢复后 Master unlock 失败",
        )?;
        require(
            migration::recover_v2_core(&restored_db, recovery_phrase.as_str())?.as_slice()
                == active_dek.as_slice(),
            "恢复后 Recovery unlock 失败",
        )?;
        let source_db = source_state
            .db
            .lock()
            .map_err(|_| "源 v2 数据库不可用".to_string())?;
        let source_conn = source_db
            .conn
            .lock()
            .map_err(|_| "源 v2 数据库不可用".to_string())?;
        let restored_conn = restored_db
            .conn
            .lock()
            .map_err(|_| "恢复数据库不可用".to_string())?;
        require(
            exact_v2_hashes(&source_conn)? == exact_v2_hashes(&restored_conn)?,
            "v2 backup/restore 业务数据不一致",
        )?;
    }

    let failure_points = [
        ("password-row", MigrationFailPoint::AfterPartialRows),
        ("security-metadata", MigrationFailPoint::DuringSettingsWrite),
        ("tmp-validation", MigrationFailPoint::OnIntegrityFail),
        ("before-activation", MigrationFailPoint::BeforeRename),
    ];
    for (label, point) in failure_points {
        let failure_dir = workspace.join(format!("failure-{label}"));
        std::fs::create_dir(&failure_dir).map_err(|_| "故障注入目录已存在".to_string())?;
        let failure_legacy = failure_dir.join(migration::LEGACY_DB_FILENAME);
        online_snapshot(&snapshot, &failure_legacy)?;
        let failure_hash = file_sha256(&failure_legacy)?;
        require(
            migration::migrate_legacy_to_v2_inject(&failure_dir, master_password, Some(point))
                .is_err(),
            "故障注入未按预期失败",
        )?;
        require(failure_legacy.is_file(), "故障注入丢失 legacy 输入")?;
        require(
            file_sha256(&failure_legacy)? == failure_hash,
            "故障注入改写了 legacy 输入",
        )?;
        require(
            !failure_dir.join(migration::V2_DB_FILENAME).exists(),
            "故障注入留下 active v2",
        )?;
    }

    require(
        file_sha256(&snapshot)? == snapshot_hash,
        "不可变 snapshot 被修改",
    )?;
    let arbitration = migration::resolve_startup_db(&migration_dir, false);
    require(
        matches!(arbitration.selection, migration::DbSelection::V2(ref path) if path == &v2_path),
        "迁移后启动仲裁未选择 v2",
    )?;
    require(
        !migration_dir.join(migration::LEGACY_DB_FILENAME).exists(),
        "0.2 固定 legacy 路径仍存在",
    )?;

    let final_v2 = readonly_connection(&v2_path)?;
    let final_counts = table_counts(&final_v2)?;
    println!("PHASE2D_RESULT=PASS");
    println!("PRODUCTION_PREPARE_CONFIRM=PASS");
    println!("ABORT_BEFORE_CONFIRM=PASS");
    println!("WRONG_WORDS_FAIL_CLOSED=PASS");
    println!("SOURCE_CHANGED_FAIL_CLOSED=PASS");
    println!("WORKSPACE={}", workspace.display());
    println!("SNAPSHOT_SHA256={snapshot_hash}");
    println!("SCHEMA_SHA256={schema_hash}");
    println!("PASSWORD_METADATA_SHA256={password_metadata_hash}");
    println!("LEGACY_COUNTS={baseline_counts:?}");
    println!("V2_COUNTS={final_counts:?}");
    println!("PASSWORD_PLAINTEXT_EQUALITY={checked}/{checked}");
    println!("MASTER_UNLOCK=PASS");
    println!("RECOVERY_UNLOCK=PASS");
    println!("BACKUP_RESTORE=PASS");
    println!("FAILURE_REHEARSAL=4/4");
    println!("DOWNGRADE_ISOLATION=PASS");
    Ok(())
}

fn build_fixture_snapshot(path: &Path, master_password: &str) {
    let db = Db::open(path).unwrap();
    let salt = crypto::generate_salt();
    let key = Zeroizing::new(crypto::derive_key(master_password, &salt));
    db.set_setting("master_password_salt", &crypto::b64_encode(&salt))
        .unwrap();
    db.set_setting(
        "master_password_hash",
        &crypto::hash_password(master_password, &salt),
    )
    .unwrap();
    db.set_setting(
        "recovery_phrase_encrypted",
        &crypto::encrypt("[\"legacy-fixture\"]", key.as_slice()).unwrap(),
    )
    .unwrap();
    db.set_setting("fixture_business_setting", "preserve-me")
        .unwrap();
    for index in 0..3 {
        let ciphertext =
            crypto::encrypt(&format!("fixture-secret-{index}"), key.as_slice()).unwrap();
        db.create_password(
            &format!("fixture-title-{index}"),
            "fixture-user",
            &ciphertext,
            "https://fixture.invalid",
            "fixture-notes",
        )
        .unwrap();
    }
    db.conn
        .lock()
        .unwrap()
        .execute("DELETE FROM passwords WHERE id=2", [])
        .unwrap();
    let replacement = crypto::encrypt("fixture-secret-3", key.as_slice()).unwrap();
    db.create_password(
        "fixture-title-3",
        "fixture-user",
        &replacement,
        "https://fixture.invalid",
        "fixture-notes",
    )
    .unwrap();
    db.create_app_category("fixture-category", "x").unwrap();
    db.create_app(
        "fixture-app",
        "C:\\fixture.exe",
        "",
        "",
        None,
        "app",
        "utility",
    )
    .unwrap();
    db.create_snippet("fixture-snippet", "fixture-body", "text", "fixture")
        .unwrap();
    db.create_temp("fixture-temp", 60).unwrap();
    db.conn
        .lock()
        .unwrap()
        .execute(
            "INSERT INTO pinned_items (item_type,item_id,sort_order) VALUES ('password',3,1)",
            [],
        )
        .unwrap();
}

#[test]
fn phase2d_fixture_drives_production_migration_backup_restore() {
    const MASTER: &str = "Phase2D-Fixture-Master!";
    let workspace = std::env::temp_dir().join(format!(
        "drawer-phase2d-fixture-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir(&workspace).unwrap();
    build_fixture_snapshot(&workspace.join("legacy-real-snapshot.db"), MASTER);
    run_rehearsal_with_password(workspace.clone(), MASTER).unwrap();
    std::fs::remove_dir_all(workspace).unwrap();
}

#[test]
#[ignore = "requires an isolated real snapshot and secure local stdin"]
fn phase2d_real_snapshot_rehearsal() {
    let workspace = PathBuf::from(
        std::env::var("DRAWER_PHASE2D_WORKSPACE")
            .unwrap_or_else(|_| panic!("Phase2D rehearsal failed: 缺少 Phase2D workspace")),
    );
    let master_password =
        read_master_password().unwrap_or_else(|error| panic!("Phase2D rehearsal failed: {error}"));
    run_rehearsal_with_password(workspace, master_password.as_str())
        .unwrap_or_else(|error| panic!("Phase2D rehearsal failed: {error}"));
}

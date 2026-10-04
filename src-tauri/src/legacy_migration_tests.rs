//! ============================================================
//! LM1–LM14：两阶段 legacy 安全升级 production-level 测试。
//!
//! 全部走正式 production 路径（AppState::prepare/confirm/cancel_legacy_migration
//! 与 migration::activate_migrated_v2），不调用任何 test-only 旁路。
//! 目录命名 fw2lm_*，结束后全部清理。
//! ============================================================

#[cfg(test)]
mod legacy_migration_tests {
    use crate::commands::auth::ensure_not_legacy_startup;
    use crate::crypto;
    use crate::db::Db;
    use crate::migration::{self, LEGACY_DB_FILENAME};
    use crate::{AppState, SecurityModel, StartupMode};
    use rusqlite::types::ValueRef;
    use rusqlite::OpenFlags;
    use sha2::{Digest, Sha256};
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;
    use zeroize::Zeroizing;

    const MASTER: &str = "FW2LM-Master!";
    const WRONG_MASTER: &str = "FW2LM-Wrong!";

    fn temp_app_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "fw2lm_{}_{}",
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

    fn file_sha256(path: &Path) -> [u8; 32] {
        let bytes = std::fs::read(path).unwrap();
        Sha256::digest(&bytes).into()
    }

    /// 构造与 production 启动一致的 legacy-only AppState
    /// （Legacy 启动时 AppState.db = legacy 库 RW 连接，db_path 指向 legacy 文件）。
    fn legacy_state(dir: &Path) -> AppState {
        let legacy_path = dir.join(LEGACY_DB_FILENAME);
        AppState {
            db: Mutex::new(Db::open(&legacy_path).unwrap()),
            db_path: legacy_path,
            startup_mode: Mutex::new(StartupMode::Legacy),
            pending_v2: Mutex::new(None),
            pending_recovery_rotation: Mutex::new(None),
            pending_legacy_migration: Mutex::new(None),
            master_wrap_gate: Mutex::new(()),
            data_op_gate: std::sync::RwLock::new(()),
            migration_freeze: std::sync::atomic::AtomicBool::new(false),
            key: Mutex::new(None),
        }
    }

    /// 丰富版 legacy fixture：密码（含回收站行 + 使用计数）+ apps/categories/
    /// snippets/temp/pinned + 普通 settings。返回 legacy key。
    fn build_rich_legacy_fixture(dir: &Path, master_password: &str) -> Zeroizing<Vec<u8>> {
        let db = Db::open(&dir.join(LEGACY_DB_FILENAME)).unwrap();
        let salt = crypto::generate_salt();
        let hash = crypto::hash_password(master_password, &salt);
        let key = Zeroizing::new(crypto::derive_key(master_password, &salt));
        db.set_setting("master_password_hash", &hash).unwrap();
        db.set_setting("master_password_salt", &crypto::b64_encode(&salt))
            .unwrap();
        db.set_setting(
            "recovery_phrase_encrypted",
            &crypto::encrypt("[\"legacy\"]", &key).unwrap(),
        )
        .unwrap();
        db.set_setting("trash_retention_days", "7").unwrap();
        db.set_setting("fixture_business_setting", "preserve-me").unwrap();

        for i in 0..6 {
            let enc = crypto::encrypt(&format!("legacy-secret-{i}"), &key).unwrap();
            db.create_password(
                &format!("Legacy-PW-{i}"),
                "fixture-user",
                &enc,
                "https://fixture.test",
                "notes",
            )
            .unwrap();
        }
        // 回收站行 + 使用计数（usage state 必须原样迁移）
        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "UPDATE passwords SET deleted_at=12345, use_count=9, last_used_at=987 WHERE id=2",
                [],
            )
            .unwrap();
            conn.execute(
                "UPDATE passwords SET use_count=5, last_used_at=555 WHERE id=1",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO pinned_items (item_type,item_id,sort_order) VALUES ('password',1,7),('app',1,3)",
                [],
            )
            .unwrap();
        }
        let cat = db.create_app_category("fixture-cat", "📁").unwrap();
        db.create_app("Fixture App", "C:\\fixture\\app.exe", "", "", Some(cat), "app", "utility")
            .unwrap();
        db.create_snippet("Fixture Snippet", "echo fixture", "bash", "fixture").unwrap();
        db.create_temp("fixture temp content", 60).unwrap();
        key
    }

    fn confirm_answers(prepared: &crate::LegacyMigrationPreparation) -> Vec<String> {
        prepared
            .confirmation_indexes
            .iter()
            .map(|i| prepared.recovery_words[*i].clone())
            .collect()
    }

    fn canonical_query_sha256(conn: &rusqlite::Connection, sql: &str) -> String {
        let mut stmt = conn.prepare(sql).unwrap();
        let columns = stmt.column_count();
        let mut rows = stmt.query([]).unwrap();
        let mut digest = Sha256::new();
        while let Some(row) = rows.next().unwrap() {
            digest.update([0xfe]);
            for column in 0..columns {
                match row.get_ref(column).unwrap() {
                    ValueRef::Null => digest.update([0]),
                    ValueRef::Integer(v) => {
                        digest.update([1]);
                        digest.update(v.to_le_bytes());
                    }
                    ValueRef::Real(v) => {
                        digest.update([2]);
                        digest.update(v.to_bits().to_le_bytes());
                    }
                    ValueRef::Text(v) => {
                        digest.update([3]);
                        digest.update((v.len() as u64).to_le_bytes());
                        digest.update(v);
                    }
                    ValueRef::Blob(v) => {
                        digest.update([4]);
                        digest.update((v.len() as u64).to_le_bytes());
                        digest.update(v);
                    }
                }
            }
        }
        hex(&digest.finalize())
    }

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    fn readonly_conn(path: &Path) -> rusqlite::Connection {
        let conn =
            rusqlite::Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        conn.pragma_update(None, "query_only", true).unwrap();
        conn
    }

    /// 业务数据 + 普通 settings 全量 canonical 对比（legacy 归档 vs 已激活 v2）。
    fn assert_business_data_identical(legacy_backup: &Path, state: &AppState) {
        let queries = [
            ("password metadata", "SELECT id,title,username,url,notes,created_at,updated_at,use_count,last_used_at,deleted_at FROM passwords ORDER BY id"),
            ("apps", "SELECT id,name,path,icon_path,args,category_id,created_at,use_count,last_used_at,app_type,app_subtype,deleted_at FROM apps ORDER BY id"),
            ("app_categories", "SELECT id,name,icon,sort_order FROM app_categories ORDER BY id"),
            ("snippets", "SELECT id,title,content,language,tags,created_at,updated_at,use_count,last_used_at,deleted_at FROM snippets ORDER BY id"),
            ("temp_contents", "SELECT id,text,created_at,expires_at,deleted_at FROM temp_contents ORDER BY id"),
            ("pinned_items", "SELECT id,item_type,item_id,sort_order FROM pinned_items ORDER BY id"),
            ("ordinary settings", "SELECT key,value FROM settings WHERE key NOT IN ('master_password_hash','master_password_salt','recovery_phrase_encrypted','security_version','kdf_params_m','wrapped_dek_m','kdf_params_r','wrapped_dek_r') ORDER BY key"),
        ];
        let legacy_conn = readonly_conn(legacy_backup);
        let v2_conn = state.db.lock().unwrap();
        let v2_guard = v2_conn.conn.lock().unwrap();
        for (label, sql) in queries {
            assert_eq!(
                canonical_query_sha256(&legacy_conn, sql),
                canonical_query_sha256(&v2_guard, sql),
                "{label} 迁移前后不一致"
            );
        }
    }

    /// 每行 legacy 密文（legacy key）与 v2 DW2（DEK）明文等价校验；返回 12/12 风格计数。
    fn verify_password_plaintext_equality(
        legacy_backup: &Path,
        state: &AppState,
        legacy_key: &Zeroizing<Vec<u8>>,
        dek: &[u8],
    ) -> usize {
        let legacy_conn = readonly_conn(legacy_backup);
        let mut stmt = legacy_conn
            .prepare("SELECT id,password FROM passwords ORDER BY id")
            .unwrap();
        let rows: Vec<(i64, String)> = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        drop(stmt);
        let v2_conn = state.db.lock().unwrap();
        let v2_guard = v2_conn.conn.lock().unwrap();
        let mut seen = std::collections::HashSet::new();
        let mut checked = 0;
        for (id, legacy_cipher) in rows {
            let legacy_plain = Zeroizing::new(crypto::decrypt(&legacy_cipher, legacy_key).unwrap());
            let (record_uuid, dw2): (String, String) = v2_guard
                .query_row(
                    "SELECT record_uuid,password FROM passwords WHERE id=?1",
                    [id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .unwrap();
            assert!(seen.insert(record_uuid.clone()), "record_uuid 不唯一");
            assert!(dw2.starts_with(crypto::PASSWORD_PREFIX_DW2), "不是 DW2 格式");
            let v2_plain =
                Zeroizing::new(crypto::decrypt_password_dw2(&dw2, dek, &record_uuid).unwrap());
            assert_eq!(
                legacy_plain.as_str(),
                v2_plain.as_str(),
                "id={id} 密码明文不等价"
            );
            checked += 1;
        }
        checked
    }

    /// LM14 的磁盘扫描：全目录逐文件字节扫描，短语与 entropy 均不得出现。
    fn assert_phrase_never_persisted(dir: &Path, phrase: &str, entropy: &[u8]) {
        let phrase_lower = phrase.as_bytes();
        fn walk(dir: &Path, found: &mut Vec<PathBuf>) {
            for entry in std::fs::read_dir(dir).unwrap() {
                let entry = entry.unwrap();
                let path = entry.path();
                if path.is_dir() {
                    walk(&path, found);
                } else {
                    found.push(path);
                }
            }
        }
        let mut files = Vec::new();
        walk(dir, &mut files);
        assert!(!files.is_empty(), "扫描目录为空");
        for file in files {
            let bytes = std::fs::read(&file).unwrap_or_default();
            assert!(
                !contains(bytes.as_slice(), phrase_lower),
                "恢复短语字节出现在磁盘文件 {:?}",
                file
            );
            assert!(
                !contains(bytes.as_slice(), entropy),
                "恢复短语 entropy 字节出现在磁盘文件 {:?}",
                file
            );
        }
    }

    fn contains(haystack: &[u8], needle: &[u8]) -> bool {
        if needle.is_empty() || haystack.len() < needle.len() {
            return false;
        }
        haystack.windows(needle.len()).any(|w| w == needle)
    }

    // ===== LM1 正常 prepare =====
    #[test]
    fn lm1_prepare_creates_pending_without_touching_legacy_or_formal_v2() {
        let dir = temp_app_dir("lm1");
        build_rich_legacy_fixture(&dir, MASTER);
        let legacy_hash = file_sha256(&dir.join(LEGACY_DB_FILENAME));

        let state = legacy_state(&dir);
        let prepared = state.prepare_legacy_migration(MASTER).unwrap();

        assert!(!prepared.migration_token.is_empty());
        assert_eq!(prepared.recovery_words.len(), 12, "恢复短语 12 词");
        assert_eq!(prepared.confirmation_indexes.len(), 3);
        let mut sorted = prepared.confirmation_indexes.clone();
        sorted.sort();
        assert_eq!(sorted, prepared.confirmation_indexes, "确认位置有序");
        assert!(sorted.iter().all(|i| (0..12).contains(i)));
        assert!(sorted.iter().collect::<std::collections::HashSet<_>>().len() == 3, "位置不重复");

        assert!(dir.join(migration::V2_TMP_FILENAME).is_file(), "pending tmp 已创建");
        assert!(!dir.join(migration::V2_DB_FILENAME).exists(), "正式 v2 不得存在");
        assert_eq!(
            file_sha256(&dir.join(LEGACY_DB_FILENAME)),
            legacy_hash,
            "legacy 库字节级不变"
        );
        let status = state.legacy_migration_status();
        assert!(status.active);
        assert_eq!(status.migration_token.as_deref(), Some(prepared.migration_token.as_str()));

        state.pending_legacy_migration.lock().unwrap().take();
        cleanup_dir(&dir);
    }

    // ===== LM2 错主密码 =====
    #[test]
    fn lm2_wrong_master_password_fails_closed_without_dirty_state() {
        let dir = temp_app_dir("lm2");
        build_rich_legacy_fixture(&dir, MASTER);
        let legacy_hash = file_sha256(&dir.join(LEGACY_DB_FILENAME));

        let state = legacy_state(&dir);
        let result = state.prepare_legacy_migration(WRONG_MASTER);
        assert!(result.is_err(), "错主密码必须被拒绝");
        assert!(!dir.join(migration::V2_DB_FILENAME).exists(), "无正式 v2");
        assert!(!dir.join(migration::V2_TMP_FILENAME).exists(), "无脏 tmp");
        assert!(!dir.join(migration::V2_SETUP_LOCK_FILENAME).exists(), "无脏 lock");
        assert!(!state.legacy_migration_status().active, "无 pending");
        assert_eq!(file_sha256(&dir.join(LEGACY_DB_FILENAME)), legacy_hash);
        cleanup_dir(&dir);
    }

    // ===== LM3 Prepare 后 exit/abort：orphan 清理 + 可重新升级 =====
    #[test]
    fn lm3_exit_after_prepare_leaves_orphan_cleanup_and_retry_possible() {
        let dir = temp_app_dir("lm3");
        build_rich_legacy_fixture(&dir, MASTER);

        // 第一“次会话”：prepare 后直接丢弃 AppState（模拟用户关闭应用）
        {
            let state = legacy_state(&dir);
            state.prepare_legacy_migration(MASTER).unwrap();
            assert!(dir.join(migration::V2_TMP_FILENAME).is_file());
            // state 在此 drop：内存 pending 与 lock 句柄全部消失，磁盘留下 orphan
        }

        // 下次启动：startup 仲裁前清理 orphan tmp/lock（lib.rs setup 的正式调用）
        let removed = migration::discard_abandoned_migration_tmp(&dir).unwrap();
        assert!(removed, "orphan tmp 必须被识别并清理");
        assert!(!dir.join(migration::V2_TMP_FILENAME).exists());
        assert!(!dir.join(migration::V2_SETUP_LOCK_FILENAME).exists());
        assert!(!dir.join(migration::V2_DB_FILENAME).exists(), "正式 v2 不存在");

        // 用户可以再次完整升级
        let state = legacy_state(&dir);
        let prepared = state.prepare_legacy_migration(MASTER).unwrap();
        let answers = confirm_answers(&prepared);
        state.confirm_legacy_migration(&prepared.migration_token, &answers).unwrap();
        assert!(dir.join(migration::V2_DB_FILENAME).is_file());
        cleanup_dir(&dir);
    }

    // ===== LM4 三词错误 =====
    #[test]
    fn lm4_wrong_confirmation_words_rejected_without_activation() {
        let dir = temp_app_dir("lm4");
        build_rich_legacy_fixture(&dir, MASTER);
        let legacy_hash = file_sha256(&dir.join(LEGACY_DB_FILENAME));

        let state = legacy_state(&dir);
        let prepared = state.prepare_legacy_migration(MASTER).unwrap();
        let wrong: Vec<String> = prepared
            .confirmation_indexes
            .iter()
            .map(|_| "not-the-right-word".to_string())
            .collect();
        let result = state.confirm_legacy_migration(&prepared.migration_token, &wrong);
        assert!(result.is_err(), "错误确认词必须拒绝");
        assert!(!dir.join(migration::V2_DB_FILENAME).exists(), "正式 v2 不得存在");
        assert!(dir.join(migration::V2_TMP_FILENAME).is_file(), "pending 保留可重试");
        assert!(state.legacy_migration_status().active, "pending 仍在");
        assert_eq!(file_sha256(&dir.join(LEGACY_DB_FILENAME)), legacy_hash);

        // 正确词仍可完成（重试路径有效）
        let answers = confirm_answers(&prepared);
        state.confirm_legacy_migration(&prepared.migration_token, &answers).unwrap();
        assert!(dir.join(migration::V2_DB_FILENAME).is_file());
        cleanup_dir(&dir);
    }

    // ===== LM5 正常 confirm（完整验收） =====
    #[test]
    fn lm5_confirm_activates_v2_with_full_validation() {
        let dir = temp_app_dir("lm5");
        let legacy_key = build_rich_legacy_fixture(&dir, MASTER);
        let legacy_hash = file_sha256(&dir.join(LEGACY_DB_FILENAME));

        let state = legacy_state(&dir);
        let prepared = state.prepare_legacy_migration(MASTER).unwrap();
        let phrase: String = prepared.recovery_words.join(" ");
        let answers = confirm_answers(&prepared);

        state.confirm_legacy_migration(&prepared.migration_token, &answers).unwrap();

        // 激活与模型切换
        assert!(dir.join(migration::V2_DB_FILENAME).is_file(), "正式 v2 已就位");
        assert!(!dir.join(migration::V2_TMP_FILENAME).exists(), "tmp 已消失");
        assert!(!dir.join(migration::V2_SETUP_LOCK_FILENAME).exists(), "lock 已清理");
        let backup = dir.join(format!("{LEGACY_DB_FILENAME}{}", migration::LEGACY_BACKUP_SUFFIX));
        assert!(backup.is_file(), "legacy 隔离归档存在");
        assert!(!dir.join(LEGACY_DB_FILENAME).exists(), "legacy 原位已移除");
        assert_eq!(file_sha256(&backup), legacy_hash, "legacy 归档字节级等于原始库");
        assert_eq!(state.security_model(), SecurityModel::StableDekV2);
        assert!(state.is_unlocked(), "confirm 后 AppState 安装 Stable DEK");

        // 业务数据完整
        assert_business_data_identical(&backup, &state);

        // DEK 一致性：AppState 内的 DEK 与 Master / Recovery 两条解封路径一致
        let active_dek = state.stable_dek().unwrap();
        {
            let db = state.db.lock().unwrap();
            let master_dek = migration::unlock_v2_core(&db, MASTER).unwrap();
            assert_eq!(master_dek.as_slice(), active_dek.as_slice(), "Master unlock DEK 一致");
            let recovery_dek = migration::recover_v2_core(&db, &phrase).unwrap();
            assert_eq!(recovery_dek.as_slice(), active_dek.as_slice(), "Recovery unlock DEK 一致");
        }

        // 12/12（全部行）密码明文等价
        let checked = verify_password_plaintext_equality(&backup, &state, &legacy_key, active_dek.as_slice());
        assert_eq!(checked, 6, "密码明文等价 6/6");

        // 重新锁定后 Master unlock 仍有效
        state.clear_key();
        assert!(!state.is_unlocked());
        state.unlock_v2_and_store(MASTER).unwrap();
        assert!(state.is_unlocked());
        cleanup_dir(&dir);
    }

    // ===== LM6 Cancel =====
    #[test]
    fn lm6_cancel_clears_pending_token_and_tmp_without_touching_legacy() {
        let dir = temp_app_dir("lm6");
        build_rich_legacy_fixture(&dir, MASTER);
        let legacy_hash = file_sha256(&dir.join(LEGACY_DB_FILENAME));

        let state = legacy_state(&dir);
        let prepared = state.prepare_legacy_migration(MASTER).unwrap();
        state.cancel_legacy_migration(&prepared.migration_token).unwrap();

        assert!(!state.legacy_migration_status().active, "pending 已清");
        assert!(!dir.join(migration::V2_TMP_FILENAME).exists(), "tmp 已删除");
        assert!(!dir.join(migration::V2_SETUP_LOCK_FILENAME).exists(), "lock 已删除");
        assert!(!dir.join(migration::V2_DB_FILENAME).exists(), "正式 v2 不存在");
        assert_eq!(file_sha256(&dir.join(LEGACY_DB_FILENAME)), legacy_hash, "legacy 不变");

        // token 失效：旧 token 再 confirm 拒绝
        let answers = confirm_answers(&prepared);
        assert!(state
            .confirm_legacy_migration(&prepared.migration_token, &answers)
            .is_err());
        assert!(!dir.join(migration::V2_DB_FILENAME).exists());
        cleanup_dir(&dir);
    }

    // ===== LM7 Token replay =====
    #[test]
    fn lm7_confirmed_token_is_permanently_invalidated() {
        let dir = temp_app_dir("lm7");
        build_rich_legacy_fixture(&dir, MASTER);

        let state = legacy_state(&dir);
        let prepared = state.prepare_legacy_migration(MASTER).unwrap();
        let answers = confirm_answers(&prepared);
        state.confirm_legacy_migration(&prepared.migration_token, &answers).unwrap();
        assert_eq!(state.security_model(), SecurityModel::StableDekV2);

        // 重复 confirm：拒绝（pending 已清、模型已切换）
        let replay = state.confirm_legacy_migration(&prepared.migration_token, &answers);
        assert!(replay.is_err(), "成功后旧 token 必须永久失效");
        assert_eq!(state.security_model(), SecurityModel::StableDekV2, "状态不被回退");
        assert!(dir.join(migration::V2_DB_FILENAME).is_file());

        // cancel 同样拒绝旧 token
        assert!(state.cancel_legacy_migration(&prepared.migration_token).is_err());
        cleanup_dir(&dir);
    }

    // ===== LM8 Concurrent Prepare =====
    #[test]
    fn lm8_only_one_pending_migration_allowed() {
        let dir = temp_app_dir("lm8");
        build_rich_legacy_fixture(&dir, MASTER);

        let state = legacy_state(&dir);
        let first = state.prepare_legacy_migration(MASTER).unwrap();
        assert!(state.legacy_migration_status().active);

        // 同一进程：第二个 pending 被拒
        let second = state.prepare_legacy_migration(MASTER);
        assert!(second.is_err(), "只允许一个 pending migration");
        assert!(state.legacy_migration_status().active);
        assert_eq!(
            state.legacy_migration_status().migration_token.as_deref(),
            Some(first.migration_token.as_str()),
            "原 pending 保持不变"
        );

        // 跨进程：setup lock 独占句柄仍被持有，create_new 必须失败
        let mut lock_options = std::fs::OpenOptions::new();
        lock_options.write(true).create_new(true);
        let cross = lock_options.open(dir.join(migration::V2_SETUP_LOCK_FILENAME));
        #[cfg(windows)]
        assert!(cross.is_err(), "活动升级期间跨进程 setup lock 必须不可获得");
        let _ = cross;

        state.cancel_legacy_migration(&first.migration_token).unwrap();
        cleanup_dir(&dir);
    }

    // ===== LM9 Crash before activation =====
    #[test]
    fn lm9_crash_before_activation_keeps_legacy_intact_and_no_formal_v2() {
        let dir = temp_app_dir("lm9");
        build_rich_legacy_fixture(&dir, MASTER);
        let legacy_hash = file_sha256(&dir.join(LEGACY_DB_FILENAME));

        {
            let state = legacy_state(&dir);
            state.prepare_legacy_migration(MASTER).unwrap();
            // 进程在此崩溃：无任何清理代码执行
        }
        assert!(dir.join(migration::V2_TMP_FILENAME).is_file(), "orphan tmp 存在");
        assert!(!dir.join(migration::V2_DB_FILENAME).exists(), "无正式 v2");
        assert_eq!(file_sha256(&dir.join(LEGACY_DB_FILENAME)), legacy_hash, "legacy 完整");

        // 下次启动：orphan 清理后回到可升级状态
        assert!(migration::discard_abandoned_migration_tmp(&dir).unwrap());
        assert!(!dir.join(migration::V2_TMP_FILENAME).exists());
        assert_eq!(file_sha256(&dir.join(LEGACY_DB_FILENAME)), legacy_hash);
        cleanup_dir(&dir);
    }

    // ===== LM10 Crash after activation before AppState install =====
    #[test]
    fn lm10_crash_after_activation_is_fully_recoverable_via_master_unlock() {
        let dir = temp_app_dir("lm10");
        let legacy_key = build_rich_legacy_fixture(&dir, MASTER);

        let prepared = {
            let state = legacy_state(&dir);
            let prepared = state.prepare_legacy_migration(MASTER).unwrap();
            // 模拟 confirm 在持久化提交（rename tmp→v2）后、AppState 安装前崩溃：
            // 直接调用 production 提交点，不安装任何运行态。
            migration::activate_migrated_v2(&dir).unwrap();
            prepared
        };
        let phrase: String = prepared.recovery_words.join(" ");

        // ===== 下次启动 =====
        let arbitration = migration::resolve_startup_db(&dir, true);
        assert!(
            matches!(arbitration.selection, migration::DbSelection::V2(_)),
            "启动仲裁必须选择 v2"
        );
        let v2_path = dir.join(migration::V2_DB_FILENAME);
        let db = migration::open_existing_v2_db(&v2_path).unwrap();
        // security metadata 完整
        for key in ["kdf_params_m", "wrapped_dek_m", "kdf_params_r", "wrapped_dek_r", "security_version"] {
            assert!(
                db.get_setting(key).unwrap().filter(|v| !v.is_empty()).is_some(),
                "{key} 完整"
            );
        }
        // 正常 Master unlock（不依赖丢失的 pending / Recovery Phrase）
        let dek = migration::unlock_v2_core(&db, MASTER).unwrap();
        // Recovery 解封同一 DEK
        assert_eq!(
            migration::recover_v2_core(&db, &phrase).unwrap().as_slice(),
            dek.as_slice()
        );
        drop(db);

        // 以 v2 状态构建 AppState（production ExistingV2 路径）后完整可用
        let state = AppState {
            db: Mutex::new(migration::open_existing_v2_db(&v2_path).unwrap()),
            db_path: v2_path,
            startup_mode: Mutex::new(StartupMode::ExistingV2),
            pending_v2: Mutex::new(None),
            pending_recovery_rotation: Mutex::new(None),
            pending_legacy_migration: Mutex::new(None),
            master_wrap_gate: Mutex::new(()),
            data_op_gate: std::sync::RwLock::new(()),
            migration_freeze: std::sync::atomic::AtomicBool::new(false),
            key: Mutex::new(None),
        };
        assert_eq!(state.security_model(), SecurityModel::StableDekV2);
        state.unlock_v2_and_store(MASTER).unwrap();
        let active = state.stable_dek().unwrap();
        assert_eq!(active.as_slice(), dek.as_slice(), "运行态 DEK 与解封 DEK 一致");

        // 业务数据可用：用 legacy key 对比明文
        let backup = dir.join(format!("{LEGACY_DB_FILENAME}{}", migration::LEGACY_BACKUP_SUFFIX));
        // LM10 场景 legacy 尚未归档（崩溃点在归档前），legacy 仍在原位
        let legacy_path = dir.join(LEGACY_DB_FILENAME);
        let checked = verify_password_plaintext_equality(&legacy_path, &state, &legacy_key, active.as_slice());
        assert_eq!(checked, 6);
        let _ = backup;
        cleanup_dir(&dir);
    }

    // ===== LM11 Source changed after prepare =====
    #[test]
    fn lm11_source_change_fail_closed_and_remigration_possible() {
        let dir = temp_app_dir("lm11");
        let legacy_key = build_rich_legacy_fixture(&dir, MASTER);

        let state = legacy_state(&dir);
        let prepared = state.prepare_legacy_migration(MASTER).unwrap();
        let answers = confirm_answers(&prepared);

        // 用户在保存短语期间数据发生了变化（新密码 + setting 变更）
        {
            let writer = rusqlite::Connection::open(dir.join(LEGACY_DB_FILENAME)).unwrap();
            let enc = crypto::encrypt("late-secret", legacy_key.as_slice()).unwrap();
            writer
                .execute(
                    "INSERT INTO passwords (title, username, password, url, notes, created_at, updated_at)
                     VALUES ('Late-Entry', 'u', ?1, '', '', 1, 1)",
                    [enc],
                )
                .unwrap();
            writer
                .execute("UPDATE settings SET value='8' WHERE key='trash_retention_days'", [])
                .unwrap();
        }

        let result = state.confirm_legacy_migration(&prepared.migration_token, &answers);
        assert!(result.is_err(), "source changed 必须 fail closed");
        assert!(!dir.join(migration::V2_DB_FILENAME).exists(), "stale v2 绝不激活");
        assert!(!dir.join(migration::V2_TMP_FILENAME).exists(), "stale tmp 已清理");
        assert!(!state.legacy_migration_status().active, "pending 已清");
        assert!(dir.join(LEGACY_DB_FILENAME).is_file(), "legacy 原样保留");

        // 用户重新发起 migration：基于新指纹完整走通
        let prepared2 = state.prepare_legacy_migration(MASTER).unwrap();
        let answers2 = confirm_answers(&prepared2);
        state.confirm_legacy_migration(&prepared2.migration_token, &answers2).unwrap();
        assert!(dir.join(migration::V2_DB_FILENAME).is_file());
        // 新数据在 v2 中
        {
            let db = state.db.lock().unwrap();
            let conn = db.conn.lock().unwrap();
            let count: i64 = conn
                .query_row("SELECT COUNT(*) FROM passwords", [], |r| r.get(0))
                .unwrap();
            assert_eq!(count, 7, "重新迁移包含变化后的数据");
        }
        cleanup_dir(&dir);
    }

    // ===== LM12 Legacy unlock bypass =====
    #[test]
    fn lm12_legacy_unlock_bypass_is_sealed() {
        let dir = temp_app_dir("lm12");
        build_rich_legacy_fixture(&dir, MASTER);

        let state = legacy_state(&dir);
        assert_eq!(state.security_model(), SecurityModel::Legacy);

        // 普通 unlock（正确主密码）必须 fail closed：不装 key、不进入工作态
        let result = state.unlock_app_core(MASTER);
        assert!(result.is_err(), "legacy-only 启动态普通 unlock 必须被拒绝");
        assert!(!state.is_unlocked(), "不得安装 ActiveKey::Legacy");

        // 变更类 legacy IPC 的硬闸
        assert!(ensure_not_legacy_startup(&state).is_err(), "改主密码闸");
        assert!(state.require_legacy_model().is_ok(), "require_legacy_model 本身只查模型");

        // setup lock 未被 bypass 触碰，升级流程不受影响
        let prepared = state.prepare_legacy_migration(MASTER).unwrap();
        let answers = confirm_answers(&prepared);
        state.confirm_legacy_migration(&prepared.migration_token, &answers).unwrap();

        // 升级完成后普通 unlock 恢复（v2 路径）
        let state_v2 = &state;
        state_v2.clear_key();
        state_v2.unlock_app_core(MASTER).unwrap();
        assert!(state_v2.is_unlocked());
        cleanup_dir(&dir);
    }

    // ===== LM13 完整业务数据 =====
    #[test]
    fn lm13_full_business_data_survives_two_phase_migration() {
        let dir = temp_app_dir("lm13");
        let legacy_key = build_rich_legacy_fixture(&dir, MASTER);

        let state = legacy_state(&dir);
        let prepared = state.prepare_legacy_migration(MASTER).unwrap();
        let answers = confirm_answers(&prepared);
        state.confirm_legacy_migration(&prepared.migration_token, &answers).unwrap();

        let backup = dir.join(format!("{LEGACY_DB_FILENAME}{}", migration::LEGACY_BACKUP_SUFFIX));
        assert_business_data_identical(&backup, &state);

        let active = state.stable_dek().unwrap();
        // LM13 专项：record_uuid 唯一、DW2、回收站、usage state、pinned_items
        {
            let conn_guard = state.db.lock().unwrap();
            let conn = conn_guard.conn.lock().unwrap();
            let (total, unique, dw2_count, trash_count): (i64, i64, i64, i64) = conn
                .query_row(
                    "SELECT COUNT(*), COUNT(DISTINCT record_uuid),
                            SUM(password LIKE 'DW2:%'),
                            SUM(deleted_at IS NOT NULL)
                     FROM passwords",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
                )
                .unwrap();
            assert_eq!((total, unique), (6, 6), "record_uuid 全部唯一");
            assert_eq!(dw2_count, 6, "全部 DW2 格式");
            assert_eq!(trash_count, 1, "回收站行原样迁移");
            let (use_count, last_used, deleted_at): (i64, i64, Option<i64>) = conn
                .query_row(
                    "SELECT use_count, last_used_at, deleted_at FROM passwords WHERE id=2",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .unwrap();
            assert_eq!((use_count, last_used, deleted_at), (9, 987, Some(12345)), "usage state 保留");
            let pinned: i64 = conn
                .query_row("SELECT COUNT(*) FROM pinned_items", [], |r| r.get(0))
                .unwrap();
            assert_eq!(pinned, 2, "pinned_items 保留");
        }
        let checked = verify_password_plaintext_equality(&backup, &state, &legacy_key, active.as_slice());
        assert_eq!(checked, 6, "12/12 风格全量明文等价 6/6");
        cleanup_dir(&dir);
    }

    // ===== LM14 Recovery Phrase 零持久化 =====
    #[test]
    fn lm14_recovery_phrase_is_never_persisted_anywhere() {
        let dir = temp_app_dir("lm14");
        build_rich_legacy_fixture(&dir, MASTER);

        let state = legacy_state(&dir);
        let prepared = state.prepare_legacy_migration(MASTER).unwrap();
        let phrase: String = prepared.recovery_words.join(" ");
        let entropy = crypto::mnemonic_to_entropy(&phrase).unwrap();

        // Prepare 后（pending tmp 在磁盘上）：phrase / entropy 不落盘
        assert_phrase_never_persisted(&dir, &phrase, entropy.as_slice());

        let answers = confirm_answers(&prepared);
        state.confirm_legacy_migration(&prepared.migration_token, &answers).unwrap();

        // Confirm 后（正式 v2 + 归档）：phrase / entropy 不落盘
        assert_phrase_never_persisted(&dir, &phrase, entropy.as_slice());

        // v2 settings 层面：无任何 legacy 恢复短语键、无任何值包含完整短语
        {
            let conn_guard = state.db.lock().unwrap();
            let conn = conn_guard.conn.lock().unwrap();
            let legacy_keys: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM settings WHERE key LIKE '%recovery_phrase%'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(legacy_keys, 0, "v2 不得包含 recovery_phrase 键");
            let phrase_rows: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM settings WHERE value=?1",
                    [&phrase],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(phrase_rows, 0, "settings 值不得包含短语");
        }

        // 清理内存 pending 后 phrase 仅存在于测试变量（随测试结束丢弃）
        state.clear_key();
        cleanup_dir(&dir);
    }
}

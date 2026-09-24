//! ============================================================
//! V1–V12：v2 密码查看验证 + 主密码修改 正向回归测试。
//!
//! 背景：0.3.0 首次发布发现 v2 模式下查看/复制密码必败——
//! verify_password_for_pw_view 是 legacy-only（v2 恒 false），只有
//! fail-closed 测试没有正向测试。本模块全部走 production 语义
//! （verify_password_for_pw_view_core / has_second_password_core /
//! AppState::change_v2_master_password），fixture/tempdir，不触碰正式数据。
//! ============================================================

#[cfg(test)]
mod v2_verification_tests {
    use crate::commands::auth::{
        has_second_password_core, verify_legacy_password_for_view,
        verify_password_for_pw_view_core,
    };
    use crate::crypto;
    use crate::db::Db;
    use crate::migration::LEGACY_DB_FILENAME;
    use crate::{AppState, SecurityModel, StartupMode};
    use sha2::{Digest, Sha256};
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;
    use zeroize::Zeroizing;

    const MASTER: &str = "V2V-Master!";
    const NEW_MASTER: &str = "V2V-NewMaster!";
    const WRONG: &str = "V2V-Wrong!";

    fn temp_app_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "v2v_{}_{}",
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
        Sha256::digest(std::fs::read(path).unwrap()).into()
    }

    /// legacy fixture（3 条密码）→ production 两阶段升级 → 已激活、已解锁的 v2 AppState
    fn upgraded_v2_state(tag: &str) -> (PathBuf, AppState, Zeroizing<Vec<u8>>) {
        let dir = temp_app_dir(tag);
        let legacy_key = {
            let db = Db::open(&dir.join(LEGACY_DB_FILENAME)).unwrap();
            let salt = crypto::generate_salt();
            let key = Zeroizing::new(crypto::derive_key(MASTER, &salt));
            db.set_setting("master_password_hash", &crypto::hash_password(MASTER, &salt))
                .unwrap();
            db.set_setting("master_password_salt", &crypto::b64_encode(&salt))
                .unwrap();
            for i in 0..3 {
                let enc = crypto::encrypt(&format!("v2v-secret-{i}"), &key).unwrap();
                db.create_password(&format!("V2V-PW-{i}"), "user", &enc, "https://v2v.test", "")
                    .unwrap();
            }
            key
        };
        let state = legacy_state(&dir);
        let prepared = state.prepare_legacy_migration(MASTER).unwrap();
        let answers: Vec<String> = prepared
            .confirmation_indexes
            .iter()
            .map(|i| prepared.recovery_words[*i].clone())
            .collect();
        state
            .confirm_legacy_migration(&prepared.migration_token, &answers)
            .unwrap();
        assert_eq!(state.security_model(), SecurityModel::StableDekV2);
        assert!(state.is_unlocked());
        (dir, state, legacy_key)
    }

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
            key: Mutex::new(None),
        }
    }

    fn security_settings_snapshot(state: &AppState) -> Vec<(String, String)> {
        let db = state.db.lock().unwrap();
        let conn = db.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT key, value FROM settings WHERE key IN
                 ('security_version','kdf_params_m','wrapped_dek_m','kdf_params_r','wrapped_dek_r')
                 ORDER BY key",
            )
            .unwrap();
        let rows = stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        rows
    }

    fn passwords_snapshot(state: &AppState) -> Vec<(i64, String, String)> {
        let db = state.db.lock().unwrap();
        let conn = db.conn.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT id, record_uuid, password FROM passwords ORDER BY id")
            .unwrap();
        let rows = stmt
            .query_map([], |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?))
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        rows
    }

    // ===== V1：v2 + 正确主密码 → true =====
    #[test]
    fn v1_v2_correct_master_password_verifies_true() {
        let (dir, state, _) = upgraded_v2_state("v1");
        assert!(verify_password_for_pw_view_core(&state, MASTER));
        cleanup_dir(&dir);
    }

    // ===== V2：v2 + 错误主密码 → false =====
    #[test]
    fn v2_v2_wrong_master_password_verifies_false() {
        let (dir, state, _) = upgraded_v2_state("v2wrong");
        assert!(!verify_password_for_pw_view_core(&state, WRONG));
        assert!(!verify_password_for_pw_view_core(&state, ""));
        cleanup_dir(&dir);
    }

    // ===== V3：验证行为零 DB / security metadata 副作用 =====
    #[test]
    fn v3_verification_has_no_db_or_security_side_effects() {
        let (dir, state, _) = upgraded_v2_state("v3");
        let v2_path = dir.join(crate::migration::V2_DB_FILENAME);
        let before_file = file_sha256(&v2_path);
        let before_meta = security_settings_snapshot(&state);

        assert!(verify_password_for_pw_view_core(&state, MASTER));
        assert!(!verify_password_for_pw_view_core(&state, WRONG));

        assert_eq!(file_sha256(&v2_path), before_file, "验证不得改写 v2 库文件");
        assert_eq!(security_settings_snapshot(&state), before_meta, "security metadata 不得变化");
        cleanup_dir(&dir);
    }

    // ===== V4：验证后 Stable DEK 不变化 =====
    #[test]
    fn v4_stable_dek_unchanged_after_verification() {
        let (dir, state, _) = upgraded_v2_state("v4");
        let before = state.stable_dek().unwrap();
        assert!(verify_password_for_pw_view_core(&state, MASTER));
        assert!(!verify_password_for_pw_view_core(&state, WRONG));
        assert_eq!(
            state.stable_dek().unwrap().as_slice(),
            before.as_slice(),
            "AppState 活动 DEK 不得被验证行为改变"
        );
        cleanup_dir(&dir);
    }

    // ===== V5：真实 DW2 行 reveal（经活动 DEK 解密）正常 =====
    #[test]
    fn v5_dw2_row_reveals_with_active_dek() {
        let (dir, state, _) = upgraded_v2_state("v5");
        let dek = state.stable_dek().unwrap();
        {
            let db = state.db.lock().unwrap();
            let conn = db.conn.lock().unwrap();
            let mut stmt = conn
                .prepare("SELECT id, record_uuid, password FROM passwords ORDER BY id")
                .unwrap();
            let rows = stmt
                .query_map([], |r| {
                    Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?))
                })
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap();
            assert_eq!(rows.len(), 3);
            for (id, record_uuid, dw2) in rows {
                let plain = crypto::decrypt_password_dw2(&dw2, dek.as_slice(), &record_uuid)
                    .unwrap_or_else(|e| panic!("reveal 语义失败 id={id}: {e}"));
                assert_eq!(
                    plain,
                    format!("v2v-secret-{}", id - 1),
                    "id={id} reveal 明文与 fixture 不符"
                );
            }
        }
        cleanup_dir(&dir);
    }

    // ===== V6：reveal / copy 共用同一验证入口（production 语义单点） =====
    #[test]
    fn v6_reveal_and_copy_share_single_verification_entrypoint() {
        let (dir, state, _) = upgraded_v2_state("v6");
        // 前端 reveal 与 copy 两个动作都经由 submitVerify → verifyPasswordForPwView IPC
        // → verify_password_for_pw_view_core。此处以同一入口连续验证两个动作场景。
        assert!(verify_password_for_pw_view_core(&state, MASTER), "reveal 场景验证");
        assert!(verify_password_for_pw_view_core(&state, MASTER), "copy 场景验证");
        cleanup_dir(&dir);
    }

    // ===== V7：legacy 0.2.0 验证语义不回归（独立函数级） =====
    #[test]
    fn v7_legacy_verification_semantics_no_regression() {
        let dir = temp_app_dir("v7");
        let second = "V2V-Second!";
        {
            let db = Db::open(&dir.join(LEGACY_DB_FILENAME)).unwrap();
            let salt = crypto::generate_salt();
            db.set_setting("master_password_hash", &crypto::hash_password(MASTER, &salt))
                .unwrap();
            db.set_setting("master_password_salt", &crypto::b64_encode(&salt))
                .unwrap();
            // 未启用独立二次密码：主密码通过
            assert!(verify_legacy_password_for_view(&db, MASTER));
            assert!(!verify_legacy_password_for_view(&db, WRONG));

            // 启用独立二次密码：只认它，主密码无效（0.2.0 原语义）
            let s2 = crypto::generate_salt();
            db.set_setting("pw2nd_hash", &crypto::hash_password(second, &s2)).unwrap();
            db.set_setting("pw2nd_salt", &crypto::b64_encode(&s2)).unwrap();
            assert!(verify_legacy_password_for_view(&db, second));
            assert!(!verify_legacy_password_for_view(&db, MASTER));
            assert!(!verify_legacy_password_for_view(&db, WRONG));
        }
        cleanup_dir(&dir);
    }

    // ===== V8：v2 修改主密码 old → new 成功 =====
    #[test]
    fn v8_v2_master_change_old_to_new_succeeds() {
        let (dir, state, _) = upgraded_v2_state("v8");
        state.change_v2_master_password(MASTER, NEW_MASTER).unwrap();
        // 新密码可正常解锁
        state.clear_key();
        state.unlock_v2_and_store(NEW_MASTER).unwrap();
        assert!(state.is_unlocked());
        cleanup_dir(&dir);
    }

    // ===== V9：改主密码后 old 验证 false / new 验证 true =====
    #[test]
    fn v9_after_change_old_fails_and_new_succeeds_for_view_verification() {
        let (dir, state, _) = upgraded_v2_state("v9");
        assert!(verify_password_for_pw_view_core(&state, MASTER));
        state.change_v2_master_password(MASTER, NEW_MASTER).unwrap();
        assert!(!verify_password_for_pw_view_core(&state, MASTER), "旧主密码必须失效");
        assert!(verify_password_for_pw_view_core(&state, NEW_MASTER), "新主密码必须通过");
        assert!(!verify_password_for_pw_view_core(&state, WRONG));
        cleanup_dir(&dir);
    }

    // ===== V10：改主密码后 DW2 密文 / record_uuid 不变化 =====
    #[test]
    fn v10_after_change_dw2_ciphertext_and_uuid_unchanged() {
        let (dir, state, _) = upgraded_v2_state("v10");
        let before = passwords_snapshot(&state);
        state.change_v2_master_password(MASTER, NEW_MASTER).unwrap();
        assert_eq!(passwords_snapshot(&state), before, "DW2 密文与 record_uuid 必须逐字节不变");
        cleanup_dir(&dir);
    }

    // ===== V11：改主密码后 Recovery metadata 不变化 =====
    #[test]
    fn v11_after_change_recovery_metadata_unchanged() {
        let (dir, state, _) = upgraded_v2_state("v11");
        let before = security_settings_snapshot(&state);
        state.change_v2_master_password(MASTER, NEW_MASTER).unwrap();
        let after = security_settings_snapshot(&state);
        let recovery_before: Vec<_> = before.iter().filter(|(k, _)| k.ends_with("_r")).collect();
        let recovery_after: Vec<_> = after.iter().filter(|(k, _)| k.ends_with("_r")).collect();
        assert!(!recovery_before.is_empty());
        assert_eq!(recovery_before, recovery_after, "kdf_params_r / wrapped_dek_r 不得变化");
        // master wrap 必须已更新（否则新密码解不开）
        assert_ne!(
            before.iter().find(|(k, _)| k == "wrapped_dek_m").unwrap().1,
            after.iter().find(|(k, _)| k == "wrapped_dek_m").unwrap().1,
            "wrapped_dek_m 应已重包"
        );
        cleanup_dir(&dir);
    }

    // ===== V12：v2 无 legacy「独立二次验证密码」（后端语义 + legacy 入口拒绝） =====
    #[test]
    fn v12_v2_has_no_legacy_second_password() {
        let (dir, state, _) = upgraded_v2_state("v12");
        assert!(!has_second_password_core(&state), "v2 不得声称存在独立二次密码");
        // legacy 变更入口在 v2 模型下被正确拒绝（前端同时隐藏该设置区）
        assert!(state.require_legacy_model().is_err());
        cleanup_dir(&dir);
    }
}

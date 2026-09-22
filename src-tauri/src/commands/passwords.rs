//! ===== 命令分组 ②：🔑 密码管理 CRUD =====
//! 包含：增删改查密码、解密单条、生成随机密码、强度检测、使用计数

use tauri::State;
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::crypto;
use crate::db;
use crate::{AppState, SecurityModel};

// ============================================================
// 🔑 列出所有密码元信息（不含解密后的密码）
// ============================================================
#[tauri::command]
pub fn list_passwords(state: State<AppState>) -> Result<Vec<db::PasswordMeta>, String> {
    list_passwords_inner(&state)
}

fn list_passwords_inner(state: &AppState) -> Result<Vec<db::PasswordMeta>, String> {
    match state.security_model {
        SecurityModel::Legacy => {
            let _key = state.legacy_key()?;
            let db = state.db.lock().unwrap();
            db.list_passwords().map_err(|e| e.to_string())
        }
        SecurityModel::StableDekV2 => {
            let _dek = state.stable_dek()?;
            let db = state.db.lock().unwrap();
            db.list_passwords_v2()
                .map_err(|_| "密码数据读取失败".to_string())
        }
    }
}

// ============================================================
// 🔑 新增一条密码
// ============================================================
#[tauri::command]
pub fn create_password(
    state: State<AppState>,
    title: String,
    username: String,
    password: String,
    url: String,
    notes: String,
) -> Result<i64, String> {
    create_password_inner(&state, title, username, password, url, notes)
}

fn create_password_inner(
    state: &AppState,
    title: String,
    username: String,
    password: String,
    url: String,
    notes: String,
) -> Result<i64, String> {
    let password = Zeroizing::new(password);
    match state.security_model {
        SecurityModel::Legacy => {
            let key = state.legacy_key()?;
            let encrypted = crypto::encrypt(&password, &key).map_err(|e| e.to_string())?;
            let db = state.db.lock().unwrap();
            db.create_password(&title, &username, &encrypted, &url, &notes)
                .map_err(|e| e.to_string())
        }
        SecurityModel::StableDekV2 => {
            let dek = state.stable_dek()?;
            let record_uuid = Uuid::new_v4().to_string();
            let encrypted = crypto::encrypt_password_dw2(&password, &dek, &record_uuid)
                .map_err(|_| "密码加密失败".to_string())?;
            let db = state.db.lock().unwrap();
            db.create_password_v2(
                &record_uuid,
                &title,
                &username,
                &encrypted,
                &url,
                &notes,
            )
            .map_err(|_| "密码保存失败".to_string())
        }
    }
}

// ============================================================
// 🔑 获取单条密码的明文（解密后返回前端）
// 不写任何日志：明文/密文/密钥材料一律不落盘（P0-A，2026-09-19）
// ============================================================
#[tauri::command]
pub fn get_password_decrypted(state: State<AppState>, id: i64) -> Result<String, String> {
    get_password_decrypted_inner(&state, id)
}

fn get_password_decrypted_inner(state: &AppState, id: i64) -> Result<String, String> {
    match state.security_model {
        SecurityModel::Legacy => {
            let key = state.legacy_key()?;
            let db = state.db.lock().unwrap();
            let (_, _, encrypted, _, _) = db
                .get_password_encrypted(id)
                .map_err(|e| e.to_string())?
                .ok_or_else(|| "密码不存在".to_string())?;
            drop(db);
            crypto::decrypt(&encrypted, &key)
                .map_err(|_| "解密失败：密文损坏或与当前主密码不匹配".to_string())
        }
        SecurityModel::StableDekV2 => {
            let dek = state.stable_dek()?;
            let db = state.db.lock().unwrap();
            let record = db
                .get_password_v2(id)
                .map_err(|_| "密码数据读取失败".to_string())?
                .ok_or_else(|| "密码不存在".to_string())?;
            drop(db);
            crypto::decrypt_password_dw2(
                &record.encrypted_password,
                &dek,
                &record.record_uuid,
            )
            .map_err(|_| "密码解密失败".to_string())
        }
    }
}

// ============================================================
// 🔑 累加密码使用次数
// ============================================================
#[tauri::command]
pub fn bump_password_use_count(state: State<AppState>, id: i64) -> Result<i64, String> {
    bump_password_use_count_inner(&state, id)
}

fn bump_password_use_count_inner(state: &AppState, id: i64) -> Result<i64, String> {
    match state.security_model {
        SecurityModel::Legacy => {
            let db = state.db.lock().unwrap();
            db.bump_password_use_count(id).map_err(|e| e.to_string())
        }
        SecurityModel::StableDekV2 => {
            let _dek = state.stable_dek()?;
            let db = state.db.lock().unwrap();
            db.bump_password_use_count_v2(id)
                .map_err(|_| "密码使用统计更新失败".to_string())
        }
    }
}

// ============================================================
// 🔑 修改一条密码
// ============================================================
/// 修改一条密码。
/// 契约（P0-B，与 update_app 的 patch 思想一致但独立实现）：
/// - `password = None`  → 本次不修改密码，DB 中原密码密文字节级保持不变；
/// - `password = Some(p)` → 明确更新密码，用当前密钥重新加密。
/// 锁定状态下两条路径都拒绝（与其它密码命令的权限模型一致）。
#[tauri::command]
pub fn update_password(
    state: State<AppState>,
    id: i64,
    title: String,
    username: String,
    password: Option<String>,
    url: String,
    notes: String,
) -> Result<(), String> {
    update_password_inner(&state, id, title, username, password, url, notes)
}

fn update_password_inner(
    state: &AppState,
    id: i64,
    title: String,
    username: String,
    password: Option<String>,
    url: String,
    notes: String,
) -> Result<(), String> {
    match state.security_model {
        SecurityModel::Legacy => {
            let encrypted: Option<String> = match password {
                Some(pw) => {
                    let password = Zeroizing::new(pw);
                    let key = state.legacy_key()?;
                    Some(crypto::encrypt(&password, &key).map_err(|e| e.to_string())?)
                }
                None => {
                    let _key = state.legacy_key()?;
                    None
                }
            };
            let db = state.db.lock().unwrap();
            match encrypted {
                Some(enc) => db
                    .update_password(id, &title, &username, &enc, &url, &notes)
                    .map_err(|e| e.to_string()),
                None => db
                    .update_password_metadata(id, &title, &username, &url, &notes)
                    .map_err(|e| e.to_string()),
            }
        }
        SecurityModel::StableDekV2 => {
            let dek = state.stable_dek()?;
            let db = state.db.lock().unwrap();
            match password {
                Some(pw) => {
                    let record = db
                        .get_password_v2(id)
                        .map_err(|_| "密码数据读取失败".to_string())?
                        .ok_or_else(|| "密码不存在".to_string())?;
                    let password = Zeroizing::new(pw);
                    let encrypted = crypto::encrypt_password_dw2(
                        &password,
                        &dek,
                        &record.record_uuid,
                    )
                    .map_err(|_| "密码加密失败".to_string())?;
                    db.update_password_v2(
                        id,
                        &title,
                        &username,
                        &encrypted,
                        &url,
                        &notes,
                    )
                    .map_err(|_| "密码更新失败".to_string())
                }
                None => db
                    .update_password_metadata_v2(id, &title, &username, &url, &notes)
                    .map_err(|_| "密码更新失败".to_string()),
            }
        }
    }
}

// ============================================================
// 🔑 删除一条密码（软删除 → 进回收站）
// ============================================================
#[tauri::command]
pub fn delete_password(state: State<AppState>, id: i64) -> Result<(), String> {
    delete_password_inner(&state, id)
}

fn delete_password_inner(state: &AppState, id: i64) -> Result<(), String> {
    match state.security_model {
        SecurityModel::Legacy => {
            let db = state.db.lock().unwrap();
            db.soft_delete("passwords", id).map_err(|e| e.to_string())?;
        }
        SecurityModel::StableDekV2 => {
            let _dek = state.stable_dek()?;
            let db = state.db.lock().unwrap();
            db.soft_delete("passwords", id)
                .map_err(|_| "密码删除失败".to_string())?;
        }
    }
    Ok(())
}

// ============================================================
// 🔑 生成随机密码（纯路由调用，无 DB 操作）
// ============================================================
#[tauri::command]
pub fn generate_random_password(
    length: usize,
    use_upper: bool,
    use_lower: bool,
    use_digits: bool,
    use_symbols: bool,
) -> String {
    crypto::generate_password(length, use_upper, use_lower, use_digits, use_symbols)
}

// ============================================================
// 🔑 密码强度检测（0~100）
// ============================================================
#[tauri::command]
pub fn password_strength(password: String) -> u8 {
    crypto::password_strength(&password)
}

#[cfg(test)]
mod phase2a2_tests {
    use super::*;
    use crate::{migration, SecurityModel};
    use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
    use std::path::PathBuf;
    use std::sync::Mutex;
    use uuid::Uuid;
    use zeroize::Zeroizing;

    const V2_DEK: [u8; 32] = [0x2a; 32];
    const LEGACY_MASTER: &str = "phase2a2-legacy-master";

    fn temp_dir(label: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "drawer_phase2a2_{}_{}_{}",
            label,
            std::process::id(),
            Uuid::new_v4()
        ));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    fn v2_state(label: &str) -> (AppState, PathBuf) {
        let dir = temp_dir(label);
        let path = dir.join("fixture-v2.db");
        let db = migration::open_v2_db(&path).unwrap();
        db.set_setting("security_version", migration::SECURITY_VERSION_V2).unwrap();
        db.set_setting("wrapped_dek_m", "fixture-master-wrap").unwrap();
        db.set_setting("wrapped_dek_r", "fixture-recovery-wrap").unwrap();
        let state = AppState {
            db: Mutex::new(db),
            db_path: path,
            security_model: SecurityModel::StableDekV2,
            startup_mode: Mutex::new(crate::StartupMode::ExistingV2),
            pending_v2: Mutex::new(None),
            pending_recovery_rotation: Mutex::new(None),
            master_wrap_gate: Mutex::new(()),
            key: Mutex::new(None),
        };
        state.set_stable_dek(Zeroizing::new(V2_DEK.to_vec()));
        (state, dir)
    }

    fn legacy_state(label: &str) -> (AppState, PathBuf) {
        let dir = temp_dir(label);
        let path = dir.join("fixture-legacy.db");
        let db = db::Db::open(&path).unwrap();
        let salt = crypto::generate_salt();
        let key = crypto::derive_key(LEGACY_MASTER, &salt);
        let state = AppState {
            db: Mutex::new(db),
            db_path: path,
            security_model: SecurityModel::Legacy,
            startup_mode: Mutex::new(crate::StartupMode::Legacy),
            pending_v2: Mutex::new(None),
            pending_recovery_rotation: Mutex::new(None),
            master_wrap_gate: Mutex::new(()),
            key: Mutex::new(None),
        };
        state.set_legacy_key(Zeroizing::new(key.to_vec()));
        (state, dir)
    }

    fn cleanup(state: AppState, dir: PathBuf) {
        drop(state);
        std::fs::remove_dir_all(dir).unwrap();
    }

    fn create_fixture_password(state: &AppState, title: &str, password: &str) -> i64 {
        create_password_inner(
            state,
            title.to_string(),
            "fixture-user".to_string(),
            password.to_string(),
            "https://fixture.invalid".to_string(),
            "fixture-note".to_string(),
        )
        .unwrap()
    }

    fn dw2_nonce(ciphertext: &str) -> Vec<u8> {
        let encoded = ciphertext
            .strip_prefix(crypto::PASSWORD_PREFIX_DW2)
            .unwrap();
        BASE64.decode(encoded).unwrap()[..12].to_vec()
    }

    #[test]
    fn p1_v2_create_generates_unique_uuid_and_dw2_ciphertext() {
        let (state, dir) = v2_state("p1");
        let first_id = create_fixture_password(&state, "first", "plain-secret-one");
        let second_id = create_fixture_password(&state, "second", "plain-secret-two");
        assert_eq!(list_passwords_inner(&state).unwrap().len(), 2);
        let db = state.db.lock().unwrap();
        let first = db.get_password_v2(first_id).unwrap().unwrap();
        let second = db.get_password_v2(second_id).unwrap().unwrap();

        assert_eq!(Uuid::parse_str(&first.record_uuid).unwrap().get_version_num(), 4);
        assert!(!first.record_uuid.is_empty());
        assert_ne!(first.record_uuid, second.record_uuid);
        assert!(first.encrypted_password.starts_with(crypto::PASSWORD_PREFIX_DW2));
        assert!(!first.encrypted_password.contains("plain-secret-one"));
        assert_eq!(
            crypto::decrypt_password_dw2(
                &first.encrypted_password,
                &V2_DEK,
                &first.record_uuid
            )
            .unwrap(),
            "plain-secret-one"
        );
        assert!(db
            .create_password_v2(
                &first.record_uuid,
                "duplicate",
                "fixture-user",
                &first.encrypted_password,
                "",
                "",
            )
            .is_err());
        assert_eq!(db.list_passwords_v2().unwrap().len(), 2);
        drop(db);
        cleanup(state, dir);
    }

    #[test]
    fn p2_v2_read_rejects_wrong_dek_and_swapped_uuid() {
        let (state, dir) = v2_state("p2");
        let first_id = create_fixture_password(&state, "first", "secret-a");
        let second_id = create_fixture_password(&state, "second", "secret-b");
        assert_eq!(get_password_decrypted_inner(&state, first_id).unwrap(), "secret-a");

        state.set_stable_dek(Zeroizing::new(vec![0x7b; 32]));
        assert_eq!(
            get_password_decrypted_inner(&state, first_id).unwrap_err(),
            "密码解密失败"
        );
        state.set_stable_dek(Zeroizing::new(V2_DEK.to_vec()));

        let db = state.db.lock().unwrap();
        let first = db.get_password_v2(first_id).unwrap().unwrap();
        let second = db.get_password_v2(second_id).unwrap().unwrap();
        assert!(crypto::decrypt_password_dw2(
            &first.encrypted_password,
            &V2_DEK,
            &second.record_uuid
        )
        .is_err());
        drop(db);
        cleanup(state, dir);
    }

    #[test]
    fn p3_v2_metadata_patch_preserves_ciphertext_uuid_and_dek() {
        let (state, dir) = v2_state("p3");
        let id = create_fixture_password(&state, "before", "secret");
        let before = state
            .db
            .lock()
            .unwrap()
            .get_password_v2_snapshot(id)
            .unwrap()
            .unwrap();
        let dek_before = state.stable_dek().unwrap().to_vec();

        update_password_inner(
            &state,
            id,
            "after".to_string(),
            "new-user".to_string(),
            None,
            "https://after.invalid".to_string(),
            "new-note".to_string(),
        )
        .unwrap();

        let after = state
            .db
            .lock()
            .unwrap()
            .get_password_v2_snapshot(id)
            .unwrap()
            .unwrap();
        assert_eq!(after.encrypted_password.as_bytes(), before.encrypted_password.as_bytes());
        assert_eq!(after.record_uuid, before.record_uuid);
        assert_eq!(state.stable_dek().unwrap().to_vec(), dek_before);
        assert_eq!(after.use_count, before.use_count);
        assert_eq!(after.last_used_at, before.last_used_at);
        assert_eq!(after.title, "after");
        assert_eq!(after.username, "new-user");
        assert_eq!(after.url, "https://after.invalid");
        assert_eq!(after.notes, "new-note");
        cleanup(state, dir);
    }

    #[test]
    fn p4_v2_explicit_password_update_preserves_uuid_and_rotates_nonce() {
        let (state, dir) = v2_state("p4");
        let id = create_fixture_password(&state, "entry", "old-secret");
        let before = state
            .db
            .lock()
            .unwrap()
            .get_password_v2_snapshot(id)
            .unwrap()
            .unwrap();

        update_password_inner(
            &state,
            id,
            "entry".to_string(),
            "fixture-user".to_string(),
            Some("new-secret".to_string()),
            "https://fixture.invalid".to_string(),
            "fixture-note".to_string(),
        )
        .unwrap();

        let after = state
            .db
            .lock()
            .unwrap()
            .get_password_v2_snapshot(id)
            .unwrap()
            .unwrap();
        assert_eq!(after.record_uuid, before.record_uuid);
        assert_ne!(after.encrypted_password, before.encrypted_password);
        assert_ne!(
            dw2_nonce(&after.encrypted_password),
            dw2_nonce(&before.encrypted_password)
        );
        assert_eq!(get_password_decrypted_inner(&state, id).unwrap(), "new-secret");
        assert_ne!(get_password_decrypted_inner(&state, id).unwrap(), "old-secret");
        assert_eq!(state.stable_dek().unwrap().to_vec(), V2_DEK);
        cleanup(state, dir);
    }

    #[test]
    fn p5_v2_trash_restore_preserves_ciphertext_uuid_and_wraps() {
        let (state, dir) = v2_state("p5");
        let id = create_fixture_password(&state, "entry", "secret");
        let before = state
            .db
            .lock()
            .unwrap()
            .get_password_v2_snapshot(id)
            .unwrap()
            .unwrap();
        let wraps_before = {
            let db = state.db.lock().unwrap();
            (
                db.get_setting("wrapped_dek_m").unwrap(),
                db.get_setting("wrapped_dek_r").unwrap(),
            )
        };

        delete_password_inner(&state, id).unwrap();
        {
            let db = state.db.lock().unwrap();
            let trashed = db.get_password_v2_snapshot(id).unwrap().unwrap();
            assert!(trashed.deleted_at.is_some());
            assert_eq!(trashed.encrypted_password, before.encrypted_password);
            assert_eq!(trashed.record_uuid, before.record_uuid);
            db.restore("passwords", id).unwrap();
        }

        let db = state.db.lock().unwrap();
        let restored = db.get_password_v2_snapshot(id).unwrap().unwrap();
        assert!(restored.deleted_at.is_none());
        assert_eq!(restored.encrypted_password, before.encrypted_password);
        assert_eq!(restored.record_uuid, before.record_uuid);
        assert_eq!(db.get_setting("wrapped_dek_m").unwrap(), wraps_before.0);
        assert_eq!(db.get_setting("wrapped_dek_r").unwrap(), wraps_before.1);
        drop(db);
        cleanup(state, dir);
    }

    #[test]
    fn p6_v2_permanent_delete_only_removes_target_row() {
        let (state, dir) = v2_state("p6");
        let target_id = create_fixture_password(&state, "target", "target-secret");
        let keep_id = create_fixture_password(&state, "keep", "keep-secret");
        let keep_before = state
            .db
            .lock()
            .unwrap()
            .get_password_v2_snapshot(keep_id)
            .unwrap()
            .unwrap();
        let dek_before = state.stable_dek().unwrap().to_vec();
        let settings_before = {
            let db = state.db.lock().unwrap();
            (
                db.get_setting("security_version").unwrap(),
                db.get_setting("wrapped_dek_m").unwrap(),
                db.get_setting("wrapped_dek_r").unwrap(),
            )
        };

        state.db.lock().unwrap().hard_delete("passwords", target_id).unwrap();

        let db = state.db.lock().unwrap();
        assert!(db.get_password_v2(target_id).unwrap().is_none());
        let keep_after = db.get_password_v2_snapshot(keep_id).unwrap().unwrap();
        assert_eq!(keep_after.record_uuid, keep_before.record_uuid);
        assert_eq!(keep_after.encrypted_password, keep_before.encrypted_password);
        assert_eq!(db.get_setting("security_version").unwrap(), settings_before.0);
        assert_eq!(db.get_setting("wrapped_dek_m").unwrap(), settings_before.1);
        assert_eq!(db.get_setting("wrapped_dek_r").unwrap(), settings_before.2);
        drop(db);
        assert_eq!(state.stable_dek().unwrap().to_vec(), dek_before);
        cleanup(state, dir);
    }

    #[test]
    fn p7_v2_use_count_only_updates_statistics() {
        let (state, dir) = v2_state("p7");
        let id = create_fixture_password(&state, "entry", "secret");
        let before = state
            .db
            .lock()
            .unwrap()
            .get_password_v2_snapshot(id)
            .unwrap()
            .unwrap();

        assert_eq!(bump_password_use_count_inner(&state, id).unwrap(), 1);

        let after = state
            .db
            .lock()
            .unwrap()
            .get_password_v2_snapshot(id)
            .unwrap()
            .unwrap();
        assert_eq!(after.use_count, before.use_count + 1);
        assert!(after.last_used_at >= before.last_used_at);
        assert_eq!(after.encrypted_password, before.encrypted_password);
        assert_eq!(after.record_uuid, before.record_uuid);
        cleanup(state, dir);
    }

    #[test]
    fn p8_legacy_crud_and_patch_semantics_remain_unchanged() {
        let (state, dir) = legacy_state("p8");
        let id = create_fixture_password(&state, "legacy", "old-secret");
        let before = state.db.lock().unwrap().get_password_encrypted(id).unwrap().unwrap();
        assert_eq!(get_password_decrypted_inner(&state, id).unwrap(), "old-secret");
        assert_eq!(list_passwords_inner(&state).unwrap().len(), 1);

        update_password_inner(
            &state,
            id,
            "legacy-renamed".to_string(),
            "legacy-user".to_string(),
            None,
            "https://legacy.invalid".to_string(),
            "legacy-note".to_string(),
        )
        .unwrap();
        let patched = state.db.lock().unwrap().get_password_encrypted(id).unwrap().unwrap();
        assert_eq!(patched.2, before.2);

        update_password_inner(
            &state,
            id,
            "legacy-renamed".to_string(),
            "legacy-user".to_string(),
            Some("new-secret".to_string()),
            "https://legacy.invalid".to_string(),
            "legacy-note".to_string(),
        )
        .unwrap();
        assert_eq!(get_password_decrypted_inner(&state, id).unwrap(), "new-secret");
        assert_eq!(bump_password_use_count_inner(&state, id).unwrap(), 1);
        delete_password_inner(&state, id).unwrap();
        assert!(list_passwords_inner(&state).unwrap().is_empty());
        cleanup(state, dir);
    }
}

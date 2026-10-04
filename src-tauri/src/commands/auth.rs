//! ===== 命令分组 ①：🔒 安全/锁定/主密码/二次验证密码 =====
//! 包含：首次启动判断、设置主密码、解锁/锁定、改主密码（全量重加密）、改二次验证密码、密码区二次校验

use tauri::State;
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use sha2::{Digest, Sha256};

use crate::crypto;
use crate::{
    AppState, LegacyMigrationPreparation, LegacyMigrationStatus, RecoveryRotationPreparation,
    SecurityModel, StartupMode,
};
use zeroize::Zeroizing;

#[derive(serde::Serialize)]
pub struct SecurityStatus { security_model: &'static str, migration_required: bool, write_allowed: bool }
#[tauri::command]
pub fn get_security_status(state: State<AppState>) -> SecurityStatus {
    match state.security_model() {
        SecurityModel::Legacy => SecurityStatus { security_model: "legacy_security_model", migration_required: true, write_allowed: true },
        SecurityModel::StableDekV2 => SecurityStatus { security_model: "stable_dek_v2", migration_required: false, write_allowed: true },
    }
}

// ============================================================
// 🔒 两阶段 legacy 安全升级（0.3.0 起唯一 legacy 出口）
// legacy-only 启动态下禁止普通 unlock；必须走 prepare → 保存短语 →
// 三词确认 → confirm 完成升级。
// ============================================================

/// legacy-only 启动态硬闸：任何绕过升级流程直接使用/改写 legacy 库的
/// production IPC 一律 fail closed。
pub(crate) fn ensure_not_legacy_startup(state: &AppState) -> Result<(), String> {
    if *state.startup_mode.lock().map_err(|_| "安全状态不可用".to_string())? == StartupMode::Legacy {
        return Err("数据库需要先完成安全升级后再使用".to_string());
    }
    Ok(())
}

#[tauri::command]
pub fn get_legacy_migration_status(state: State<AppState>) -> LegacyMigrationStatus {
    state.legacy_migration_status()
}

#[tauri::command]
pub fn prepare_legacy_migration(
    state: State<AppState>,
    master_password: String,
) -> Result<LegacyMigrationPreparation, String> {
    let master_password = Zeroizing::new(master_password);
    state.prepare_legacy_migration(master_password.as_str())
}

#[tauri::command]
pub fn confirm_legacy_migration(
    state: State<AppState>,
    migration_token: String,
    confirmation_words: Vec<String>,
) -> Result<(), String> {
    let confirmation_words = Zeroizing::new(confirmation_words);
    state.confirm_legacy_migration(&migration_token, confirmation_words.as_slice())
}

#[tauri::command]
pub fn cancel_legacy_migration(
    state: State<AppState>,
    migration_token: String,
) -> Result<(), String> {
    state.cancel_legacy_migration(&migration_token)
}

// ============================================================
// 🔒 首次启动判断
// ============================================================
#[tauri::command]
pub fn is_first_run(state: State<AppState>) -> bool {
    state
        .startup_mode
        .lock()
        .map(|mode| matches!(*mode, StartupMode::FreshV2 | StartupMode::PendingV2))
        .unwrap_or(false)
}

// ============================================================
// 🔒 v2 新用户初始化（恢复短语仅返回一次，不写数据库/日志）
// ============================================================
#[tauri::command]
pub fn initialize_v2_security(
    state: State<AppState>,
    master_password: String,
) -> Result<Vec<String>, String> {
    let master_password = Zeroizing::new(master_password);
    if master_password.len() < 6 {
        return Err("主密码长度至少 6 位".to_string());
    }
    state
        .prepare_v2_initialization(master_password.as_str())
        .map_err(|_| "安全数据库初始化失败，请重试".to_string())
}

#[tauri::command]
pub fn finalize_v2_security(
    app: tauri::AppHandle,
    state: State<AppState>,
    init_ctx: State<crate::data_root::InitContext>,
) -> Result<(), String> {
    use tauri::Manager;
    // Phase 2C-2：自定义 Data Root 时，finalize 前先落 recovery_confirmed
    // （此刻起崩溃都可由启动收尾安全接管；恢复词从未写入任何持久化位置）
    let ctx = init_ctx.get();
    let config_root = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("无法获取配置目录: {}", e))?;
    if let Some(c) = &ctx {
        crate::data_root::save_state(
            &config_root,
            &crate::data_root::state_pending("init", &c.op_id, "recovery_confirmed", &c.target),
        )
        .map_err(|_| "安全数据库初始化失败，请重试".to_string())?;
    }
    state
        .finalize_v2_initialization()
        .map_err(|_| "安全数据库初始化失败，请重试".to_string())?;
    if let Some(c) = &ctx {
        // 正式库已 rename 就位 → 先记 activated（崩溃后启动收尾可补写 active_root），
        // 再提交 canonical Data Root。任一步失败都不影响已就位的正式库。
        let _ = crate::data_root::save_state(
            &config_root,
            &crate::data_root::state_pending("init", &c.op_id, "activated", &c.target),
        );
        crate::data_root::save_state(
            &config_root,
            &crate::data_root::state_active_external(&c.target),
        )
        .map_err(|_| "数据位置提交失败，请重启抽屉柜".to_string())?;
        init_ctx.set(None);
    }
    Ok(())
}

// ============================================================
// 🔒 v2 Recovery：验证短语 → 原子重包同一个 Stable DEK
// ============================================================
#[tauri::command]
pub fn verify_v2_recovery_phrase(
    state: State<AppState>,
    recovery_phrase: String,
) -> Result<(), String> {
    let recovery_phrase = Zeroizing::new(recovery_phrase);
    state.verify_v2_recovery_phrase(recovery_phrase.as_str())
}

#[tauri::command]
pub fn recover_v2_with_phrase(
    state: State<AppState>,
    recovery_phrase: String,
    new_master_password: String,
) -> Result<(), String> {
    let recovery_phrase = Zeroizing::new(recovery_phrase);
    let new_master_password = Zeroizing::new(new_master_password);
    state.recover_v2_with_phrase_and_store(
        recovery_phrase.as_str(),
        new_master_password.as_str(),
    )
}

// ============================================================
// 🔒 v2 普通修改主密码：验证旧 wrap → 原子重包同一个 Stable DEK
// ============================================================
#[tauri::command]
pub fn change_v2_master_password(
    state: State<AppState>,
    current_password: String,
    new_password: String,
) -> Result<(), String> {
    let current_password = Zeroizing::new(current_password);
    let new_password = Zeroizing::new(new_password);
    state.change_v2_master_password(current_password.as_str(), new_password.as_str())
}

// ============================================================
// 🔒 v2 Recovery Phrase 两阶段轮换
// ============================================================
#[tauri::command]
pub fn prepare_v2_recovery_rotation(
    state: State<AppState>,
) -> Result<RecoveryRotationPreparation, String> {
    state.prepare_v2_recovery_rotation()
}

#[tauri::command]
pub fn confirm_v2_recovery_rotation(
    state: State<AppState>,
    rotation_token: String,
    confirmation_words: Vec<String>,
) -> Result<(), String> {
    let confirmation_words = Zeroizing::new(confirmation_words);
    state.confirm_v2_recovery_rotation(&rotation_token, confirmation_words.as_slice())
}

#[tauri::command]
pub fn cancel_v2_recovery_rotation(
    state: State<AppState>,
    rotation_token: String,
) -> Result<(), String> {
    state.cancel_v2_recovery_rotation(&rotation_token)
}

// ============================================================
// 🔒 设置主密码（首次启动向导）
// ============================================================
#[tauri::command]
pub fn setup_master_password(
    state: State<AppState>,
    master_password: String,
    recovery_phrase: Vec<String>,
) -> Result<(), String> {
    state.require_legacy_model()?;
    ensure_not_legacy_startup(&state)?;
    if master_password.len() < 6 {
        return Err("主密码长度至少 6 位".to_string());
    }
    if recovery_phrase.len() != 12 {
        return Err("恢复短语必须是 12 个单词".to_string());
    }

    let salt = crypto::generate_salt();
    let key = Zeroizing::new(crypto::derive_key(&master_password, &salt));
    let hash = crypto::hash_password(&master_password, &salt);
    let salt_b64 = BASE64.encode(&salt);
    let recovery_json = serde_json::to_string(&recovery_phrase).map_err(|e| e.to_string())?;
    let recovery_encrypted = crypto::encrypt(&recovery_json, &key).map_err(|e| e.to_string())?;

    let db = state.db.lock().unwrap();
    db.set_setting("master_password_hash", &hash).map_err(|e| e.to_string())?;
    db.set_setting("master_password_salt", &salt_b64).map_err(|e| e.to_string())?;
    db.set_setting("recovery_phrase_encrypted", &recovery_encrypted).map_err(|e| e.to_string())?;
    Ok(())
}

// ============================================================
// 🔒 解锁 App（v2 Stable DEK）
// ============================================================
// 硬安全边界（0.3.0）：legacy 安全模型下普通 unlock 一律 fail closed，
// 绝不允许 ActiveKey::Legacy → 进入正常工作模式。
// legacy 用户唯一出路 = prepare_legacy_migration → 保存恢复短语 →
// 三词确认 → confirm_legacy_migration。
// 生产语义唯一实现 = AppState::unlock_app_core（可测）。
#[tauri::command]
pub fn unlock_app(state: State<AppState>, master_password: String) -> Result<Vec<String>, String> {
    state.unlock_app_core(&master_password)
}

// ============================================================
// 🔒 修改主密码（全量重加密密码字段 + 恢复短语）
// ============================================================
#[derive(serde::Serialize)]
pub struct ReencryptStats {
    passwords_reencrypted: usize,
    passwords_failed: usize,
    recovery_phrase_reencrypted: bool,
}

#[tauri::command]
pub fn change_master_password(
    state: State<AppState>,
    current_password: String,
    new_password: String,
) -> Result<ReencryptStats, String> {
    state.require_legacy_model()?;
    ensure_not_legacy_startup(&state)?;
    if new_password.len() < 6 {
        return Err("新主密码长度至少 6 位".to_string());
    }

    let db = state.db.lock().unwrap();

    // 1. 读旧 hash + salt
    let old_hash = db
        .get_setting("master_password_hash")
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "未设置主密码".to_string())?;
    let old_salt_b64 = db
        .get_setting("master_password_salt")
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "未设置主密码".to_string())?;
    let old_salt = BASE64.decode(&old_salt_b64).map_err(|e| e.to_string())?;

    // 2. 验证旧密码
    if crypto::hash_password(&current_password, &old_salt) != old_hash {
        return Err("当前主密码错误".to_string());
    }
    let old_key = Zeroizing::new(crypto::derive_key(&current_password, &old_salt));

    // 3. 读所有密码 + 恢复短语
    let passwords = db
        .list_passwords_full()
        .map_err(|e| e.to_string())?;
    let recovery_enc = db
        .get_setting("recovery_phrase_encrypted")
        .map_err(|e| e.to_string())?
        .unwrap_or_default();
    let recovery_json = if !recovery_enc.is_empty() {
        crypto::decrypt(&recovery_enc, &old_key).unwrap_or_else(|_| "[]".to_string())
    } else {
        "[]".to_string()
    };

    // 4. 派生新 key
    let new_salt = crypto::generate_salt();
    let new_key = Zeroizing::new(crypto::derive_key(&new_password, &new_salt));
    let new_hash = crypto::hash_password(&new_password, &new_salt);
    let new_salt_b64 = BASE64.encode(&new_salt);

    // 5. 用新 key 重加密恢复短语
    let recovery_encrypted_new =
        crypto::encrypt(&recovery_json, &new_key).map_err(|e| e.to_string())?;

    // 6. 重加密所有 password 字段
    let mut reencrypted_passwords = 0usize;
    let mut failed_passwords = 0usize;
    let mut trash_skipped_warning = 0usize; // ⚠️ 统计回收站的密码数（之前跳过它们会导致还原后解密失败！现在要重加密！）
    for pw in &passwords {
        // 🚨 旧逻辑BUG：跳过了 deleted_at（回收站里的密码）！
        // 场景：删了密码A到回收站 → 改主密码（正常密码被重加密）→ 从回收站还原密码A
        // → 密码A的密文还是 OLD_KEY 加密的，新 key 解不开！= 解密失败！
        // 现在修复：回收站里的密码也必须重加密！
        // if pw.deleted_at.is_some() { continue; }  ← 这行是BUG，删掉！
        if pw.deleted_at.is_some() {
            trash_skipped_warning += 1;
        }
        // 解密旧值
        let plaintext = match crypto::decrypt(&pw.encrypted_password, &old_key) {
            Ok(p) => p,
            Err(_) => {
                // 旧数据已无法解密（极少数情况：可能本来就是空或损坏）
                failed_passwords += 1;
                continue;
            }
        };
        // 用新 key 重加密
        let encrypted_new =
            crypto::encrypt(&plaintext, &new_key).map_err(|e| e.to_string())?;
        db.update_password_encrypted(pw.id, &encrypted_new)
            .map_err(|e| e.to_string())?;
        reencrypted_passwords += 1;
    }

    // 7. 保存新 hash / salt / 重加密后的恢复短语
    db.set_setting("master_password_hash", &new_hash)
        .map_err(|e| e.to_string())?;
    db.set_setting("master_password_salt", &new_salt_b64)
        .map_err(|e| e.to_string())?;
    db.set_setting("recovery_phrase_encrypted", &recovery_encrypted_new)
        .map_err(|e| e.to_string())?;
    drop(db);

    // 8. 写入新 key 到内存（保持解锁状态，不需重新输入）
    state.set_legacy_key(new_key);

    Ok(ReencryptStats {
        passwords_reencrypted: reencrypted_passwords,
        passwords_failed: failed_passwords,
        recovery_phrase_reencrypted: !recovery_enc.is_empty(),
    })
}

// ============================================================
// 🔒 锁定 App（清空内存密钥）
// ============================================================
#[tauri::command]
pub fn lock_app(state: State<AppState>) {
    state.clear_key();
}

// ============================================================
// 🔒 是否启用了独立的「密码区二次验证密码」
// v2 语义：StableDekV2 下不存在 legacy「独立二次验证密码」概念，恒 false
// （前端据此隐藏 legacy 设置区；查看/复制统一用主密码做 v2 验证）。
// ============================================================
#[tauri::command]
pub fn has_second_password(state: State<AppState>) -> bool {
    has_second_password_core(&state)
}

pub(crate) fn has_second_password_core(state: &AppState) -> bool {
    if state.security_model() != SecurityModel::Legacy { return false; }
    let legacy_startup = matches!(state.startup_mode.lock().as_deref(), Ok(StartupMode::Legacy));
    if legacy_startup { return false; }
    let db = match state.db.lock() {
        Ok(g) => g,
        Err(_) => return false,
    };
    matches!(db.get_setting("pw2nd_hash"), Ok(Some(v)) if !v.is_empty())
}

// ============================================================
// 🔒 修改/启用/清空 密码区二次验证密码
// ============================================================
#[tauri::command]
pub fn change_second_password(
    state: State<AppState>,
    old_verify_input: String,
    new_second_password_opt: Option<String>,
) -> Result<(), String> {
    state.require_legacy_model()?;
    ensure_not_legacy_startup(&state)?;
    let db = state.db.lock().map_err(|e| e.to_string())?;
    let has_2nd = matches!(db.get_setting("pw2nd_hash"), Ok(Some(v)) if !v.is_empty());

    // 1) 身份校验（旧密码）
    if has_2nd {
        // 已启用独立密码 → 只接受旧二次验证密码（不接受主密码）
        let old_hash = db.get_setting("pw2nd_hash")
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "二次验证密码缺失".to_string())?;
        let old_salt_b64 = db.get_setting("pw2nd_salt")
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "二次验证密码 salt 缺失".to_string())?;
        let old_salt = BASE64.decode(&old_salt_b64).map_err(|e| e.to_string())?;
        if crypto::hash_password(&old_verify_input, &old_salt) != old_hash {
            return Err("旧二次验证密码错误".to_string());
        }
    } else {
        // 未启用 → 接受主密码作为校验（防陌生人离开时顺手开独立密码锁）
        let mp_hash = db.get_setting("master_password_hash")
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "未设置主密码".to_string())?;
        let mp_salt_b64 = db.get_setting("master_password_salt")
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "未设置主密码".to_string())?;
        let mp_salt = BASE64.decode(&mp_salt_b64).map_err(|e| e.to_string())?;
        if crypto::hash_password(&old_verify_input, &mp_salt) != mp_hash {
            return Err("主密码错误".to_string());
        }
    }

    // 2) 执行：清空 or 设置新的
    let new_pw_norm = new_second_password_opt
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    if new_pw_norm.is_empty() {
        // 清空：回退到跟随主密码
        db.set_setting("pw2nd_hash", "").map_err(|e| e.to_string())?;
        db.set_setting("pw2nd_salt", "").map_err(|e| e.to_string())?;
        return Ok(());
    }
    if new_pw_norm.len() < 6 {
        return Err("二次验证密码长度至少 6 位".to_string());
    }
    // 安全：新独立密码必须 ≠ 主密码（否则两次验证形同虚设）
    let mp_hash = db.get_setting("master_password_hash")
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "未设置主密码".to_string())?;
    let mp_salt_b64 = db.get_setting("master_password_salt")
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "未设置主密码".to_string())?;
    let mp_salt = BASE64.decode(&mp_salt_b64).map_err(|e| e.to_string())?;
    if crypto::hash_password(&new_pw_norm, &mp_salt) == mp_hash {
        return Err("二次验证密码不能与主密码相同（否则双重验证形同虚设）".to_string());
    }

    // 3) 生成独立 salt + hash
    let new_salt = crypto::generate_salt();
    let new_hash = crypto::hash_password(&new_pw_norm, &new_salt);
    let new_salt_b64 = BASE64.encode(&new_salt);
    db.set_setting("pw2nd_hash", &new_hash).map_err(|e| e.to_string())?;
    db.set_setting("pw2nd_salt", &new_salt_b64).map_err(|e| e.to_string())?;
    Ok(())
}

// ============================================================
// 🔒 密码区二次验证入口（单条目查看/复制密码前的身份验证）
// Legacy：独立二次密码优先，未启用则回退主密码 verifier（0.2.0 行为不变）。
// StableDekV2：当前主密码 → Master KEK → unwrap wrapped_dek_m → 与活动
// Stable DEK 一致性判断。复用 unlock_v2_core（与解锁/改主密码同一验证实现，
// 不新增密码学代码）；纯验证，不触碰 DB / security metadata / AppState key。
// ============================================================
#[tauri::command]
pub fn verify_password_for_pw_view(state: State<AppState>, input_password: String) -> bool {
    let input_password = Zeroizing::new(input_password);
    verify_password_for_pw_view_core(&state, input_password.as_str())
}

/// 单条目二次验证的 production 语义（verify_password_for_pw_view 与
/// verify_master_password 的唯一实现，可测）。
pub(crate) fn verify_password_for_pw_view_core(state: &AppState, input_password: &str) -> bool {
    match state.security_model() {
        SecurityModel::StableDekV2 => verify_v2_master_password(state, input_password),
        SecurityModel::Legacy => {
            // legacy-only 启动态 fail closed（0.3.0 硬边界：升级前不开放任何 legacy 工作态）
            let legacy_startup = matches!(state.startup_mode.lock().as_deref(), Ok(StartupMode::Legacy));
            if legacy_startup { return false; }
            let db = match state.db.lock() {
                Ok(g) => g,
                Err(_) => return false,
            };
            verify_legacy_password_for_view(&db, input_password)
        }
    }
}

/// legacy 0.2.0 验证语义原样保留（独立二次密码优先，未启用回退主密码 verifier）。
/// 0.3.0 中 legacy 工作态已被封死，此路径仅为语义完整性保留。
pub(crate) fn verify_legacy_password_for_view(db: &crate::db::Db, input_password: &str) -> bool {
    // A) 独立二次验证密码已启用 → 只认它，主密码无效
    if let Ok(Some(h2)) = db.get_setting("pw2nd_hash") {
        if !h2.is_empty() {
            if let Ok(Some(s2_b64)) = db.get_setting("pw2nd_salt") {
                if let Ok(s2) = BASE64.decode(&s2_b64) {
                    return crypto::hash_password(input_password, &s2) == h2;
                }
            }
            return false;
        }
    }

    // B) 未启用独立密码 → 回退到主密码（默认行为，向后兼容）
    let hash = match db.get_setting("master_password_hash") {
        Ok(Some(v)) => v,
        _ => return false,
    };
    let salt_b64 = match db.get_setting("master_password_salt") {
        Ok(Some(v)) => v,
        _ => return false,
    };
    let salt = match BASE64.decode(&salt_b64) {
        Ok(v) => v,
        Err(_) => return false,
    };
    crypto::hash_password(input_password, &salt) == hash
}

/// 定长摘要的 constant-time 比较（XOR fold）。
/// 两侧均为 32 字节 SHA-256 摘要，长度分支不泄露秘密信息。
fn constant_time_digest_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len()
        && a.iter()
            .zip(b.iter())
            .fold(0u8, |acc, (x, y)| acc | (x ^ y))
            == 0
}

/// v2 主密码纯验证：正确 → true，错误/未解锁/状态异常 → false。
/// 无任何持久化副作用（不写 DB、不 rewrap、不改 AppState key）。
fn verify_v2_master_password(state: &AppState, input_password: &str) -> bool {
    let Ok(active_dek) = state.stable_dek() else { return false; };
    let db = match state.db.lock() {
        Ok(g) => g,
        Err(_) => return false,
    };
    match crate::migration::unlock_v2_core(&db, input_password) {
        Ok(candidate_dek) => constant_time_digest_eq(
            &Sha256::digest(candidate_dek.as_slice()),
            &Sha256::digest(active_dek.as_slice()),
        ),
        Err(_) => false,
    }
}

// ============================================================
// 🚨 紧急救援：密码全部解密失败时救回
// 场景：启用独立密码后/改主密码后/导入后 发现所有密码都「解密失败」
// 原理：让用户输入「当初加密这些密码时用的主密码」，用旧密钥解密，再用当前密钥重加密
// ============================================================
#[derive(serde::Serialize)]
pub struct RescueStats {
    pub tried: usize,
    pub rescued: usize,
    pub failed: usize,
}

#[tauri::command]
pub fn rescue_passwords_with_master(
    state: State<AppState>,
    old_master_password: String,
) -> Result<RescueStats, String> {
    state.require_legacy_model()?;
    ensure_not_legacy_startup(&state)?;
    if old_master_password.len() < 6 {
        return Err("主密码长度至少 6 位".to_string());
    }
    let db = state.db.lock().map_err(|e| e.to_string())?;

    // 1. 读 master_password_salt（这个是当初派生密钥用的 salt，不会变！）
    let mp_salt_b64 = db
        .get_setting("master_password_salt")
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "未设置主密码 salt（异常）".to_string())?;
    let mp_salt = BASE64.decode(&mp_salt_b64).map_err(|e| e.to_string())?;

    // 2. 派生 OLD_KEY（用户记得的「当初加密时的主密码」+ 不变的 salt）
    let old_key = Zeroizing::new(crypto::derive_key(&old_master_password, &mp_salt));

    // 3. 当前 legacy key（v2 Stable DEK 无法进入此分支）
    let new_key = state.legacy_key().map_err(|_| "应用未解锁，请先解锁主界面再救援".to_string())?;

    // 4. 读所有密码密文
    let passwords = db.list_passwords_full().map_err(|e| e.to_string())?;

    let mut tried = 0usize;
    let mut rescued = 0usize;
    let mut failed = 0usize;
    let mut trash_rescued = 0usize; // 🔍 统计救回的回收站密码数

    for pw in &passwords {
        // 🚨 旧逻辑BUG：跳过了回收站里的密码！
        // 场景：删了密码A → 改主密码 → 救援 → 从回收站还原密码A → 解密失败！
        // 修复：回收站里的密码也必须救回！
        let is_in_trash = pw.deleted_at.is_some();
        tried += 1;
        // 4a. 用 OLD_KEY 解密（失败就跳过，记失败数）
        let plaintext = match crypto::decrypt(&pw.encrypted_password, &old_key) {
            Ok(p) => p,
            Err(_) => {
                // 也可能之前就是用 new_key 加密的（救不救都没事），尝试下
                match crypto::decrypt(&pw.encrypted_password, &new_key) {
                    Ok(_) => { continue; } // 本来就能解，不算救援也不算失败
                    Err(_) => { failed += 1; continue; }
                }
            }
        };
        // 4b. 用 NEW_KEY 重加密写回
        let encrypted_new = crypto::encrypt(&plaintext, &new_key).map_err(|e| e.to_string())?;
        db.update_password_encrypted(pw.id, &encrypted_new).map_err(|e| e.to_string())?;
        rescued += 1;
    }

    Ok(RescueStats { tried, rescued, failed })
}

// ============================================================
// 🔒 兼容旧版前端（verify_master_password 是旧接口，直接转新接口）
// ============================================================
#[tauri::command]
pub fn verify_master_password(state: State<AppState>, master_password: String) -> bool {
    verify_password_for_pw_view(state, master_password)
}

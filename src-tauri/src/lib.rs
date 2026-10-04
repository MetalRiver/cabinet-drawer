use tauri::{
    image::Image,
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Emitter, Manager, State, WindowEvent,
};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};
use tauri_plugin_clipboard_manager::ClipboardExt;

// P0-#PERF#NOCMD：Windows 上用 CommandExt 加 CREATE_NO_WINDOW flag 防止闪窗
#[cfg(windows)]
use std::os::windows::process::CommandExt;

mod db;
mod data_root;
pub mod migration;
#[cfg(test)]
mod phase2d;
#[cfg(test)]
mod legacy_migration_tests;
#[cfg(test)]
mod v2_verification_tests;
mod crypto;
mod backup;
mod backup_v2;
#[cfg(windows)]
mod win_dock;
#[cfg(windows)]
mod win_launch;
#[cfg(windows)]
mod win_icon;
mod commands;
// 把 commands 模块下所有命令 pub use 到 crate 根，generate_handler!() 不用改
use commands::*;

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use zeroize::Zeroizing;
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use rand::Rng;
use sha2::{Digest, Sha256};

use db::Db;

// ===== 应用状态 =====

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecurityModel { Legacy, StableDekV2 }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartupMode { Legacy, ExistingV2, FreshV2, PendingV2 }

enum ActiveKey { Legacy(Zeroizing<Vec<u8>>), StableDek(Zeroizing<Vec<u8>>) }

struct PendingV2Initialization {
    tmp_path: PathBuf,
    v2_path: PathBuf,
    dek: Zeroizing<Vec<u8>>,
    setup_lock: std::fs::File,
    setup_lock_path: PathBuf,
}

struct PendingRecoveryRotation {
    token: String,
    params_r_json: String,
    wrapped_dek_r: String,
    confirmation_indexes: Vec<usize>,
    confirmation_hashes: Vec<[u8; 32]>,
    confirmation_key: Zeroizing<Vec<u8>>,
    dek_fingerprint: [u8; 32],
}

/// 两阶段 legacy 升级的内存 pending 状态。只保存 confirm 所需最小状态；
/// 主密码 / 恢复短语 / KEK 一律不在此结构内，也不得持久化。
struct PendingLegacyMigration {
    token: String,
    tmp_path: PathBuf,
    v2_path: PathBuf,
    legacy_path: PathBuf,
    dek: Zeroizing<Vec<u8>>,
    /// prepare 时的 legacy 源库指纹；confirm 激活前复验，不一致 fail closed。
    source_fingerprint: [u8; 32],
    confirmation_indexes: Vec<usize>,
    confirmation_hashes: Vec<[u8; 32]>,
    confirmation_key: Zeroizing<Vec<u8>>,
    /// Windows 独占 setup lock（share_mode 0），防跨进程并发升级。
    migration_lock: Option<std::fs::File>,
    migration_lock_path: PathBuf,
}

impl PendingLegacyMigration {
    /// 丢弃 pending 并清理本次升级的全部磁盘痕迹（tmp + sidecars + lock）。
    /// legacy 与正式 v2 永不触碰。
    fn discard_files(mut self) {
        if let Some(lock) = self.migration_lock.take() {
            drop(lock);
        }
        let _ = std::fs::remove_file(&self.migration_lock_path);
        let _ = std::fs::remove_file(&self.tmp_path);
        let parent = self.tmp_path.parent().map(Path::to_path_buf).unwrap_or_default();
        if let Some(name) = self.tmp_path.file_name() {
            for suffix in ["-wal", "-shm", "-journal"] {
                let mut sidecar = name.to_os_string();
                sidecar.push(suffix);
                let _ = std::fs::remove_file(parent.join(sidecar));
            }
        }
    }
}

#[derive(serde::Serialize)]
pub struct RecoveryRotationPreparation {
    rotation_token: String,
    recovery_words: Vec<String>,
    confirmation_indexes: Vec<usize>,
}

#[derive(serde::Serialize)]
pub struct LegacyMigrationPreparation {
    migration_token: String,
    recovery_words: Vec<String>,
    confirmation_indexes: Vec<usize>,
}

#[derive(serde::Serialize)]
pub struct LegacyMigrationStatus {
    pub active: bool,
    pub migration_token: Option<String>,
}

pub struct AppState {
    pub db: Mutex<Db>,
    pub db_path: PathBuf,
    pub startup_mode: Mutex<StartupMode>,
    pending_v2: Mutex<Option<PendingV2Initialization>>,
    pending_recovery_rotation: Mutex<Option<PendingRecoveryRotation>>,
    pending_legacy_migration: Mutex<Option<PendingLegacyMigration>>,
    /// 串行化所有会改写 v2 master wrap 的操作（Recovery / 普通改主密码）。
    /// 安全边界不能只依赖前端按钮 disabled。
    master_wrap_gate: Mutex<()>,
    /// 唯一活动秘密：legacy key 或 v2 Stable DEK，不能混用。
    key: Mutex<Option<ActiveKey>>,
}

impl AppState {
    pub fn is_unlocked(&self) -> bool { self.key.lock().unwrap().is_some() }
    /// 安全模型唯一事实源 = startup_mode（Legacy ⇔ legacy 模型；其余均为 v2）。
    /// legacy 升级 confirm 把 startup_mode 切到 ExistingV2，即完成模型切换。
    pub fn security_model(&self) -> SecurityModel {
        match *self.startup_mode.lock().unwrap() {
            StartupMode::Legacy => SecurityModel::Legacy,
            _ => SecurityModel::StableDekV2,
        }
    }
    pub fn clear_key(&self) {
        *self.key.lock().unwrap() = None;
        *self.pending_recovery_rotation.lock().unwrap() = None;
    }
    pub fn set_legacy_key(&self, key: Zeroizing<Vec<u8>>) { *self.key.lock().unwrap() = Some(ActiveKey::Legacy(key)); }
    pub fn set_stable_dek(&self, dek: Zeroizing<Vec<u8>>) { *self.key.lock().unwrap() = Some(ActiveKey::StableDek(dek)); }
    pub(crate) fn lock_master_wrap_gate(&self) -> Result<std::sync::MutexGuard<'_, ()>, String> {
        self.master_wrap_gate
            .lock()
            .map_err(|_| "安全状态不可用".to_string())
    }
    pub(crate) fn activate_validated_v2_restore(
        &self,
        source: &Db,
        dek: Zeroizing<Vec<u8>>,
    ) -> Result<(), String> {
        // 所有运行态锁先取得，再启动 SQLite 原子 backup transaction；因此不存在
        // “正式 DB 已切换但 AppState 因锁中毒无法安装 DEK”的失败窗口。
        let db = self.db.lock().map_err(|_| "安全状态不可用".to_string())?;
        let mut key = self.key.lock().map_err(|_| "安全状态不可用".to_string())?;
        let mut pending = self
            .pending_recovery_rotation
            .lock()
            .map_err(|_| "安全状态不可用".to_string())?;
        let source_conn = source
            .conn
            .lock()
            .map_err(|_| "恢复临时数据库不可用".to_string())?;
        let mut destination_conn = db
            .conn
            .lock()
            .map_err(|_| "正式数据库不可用".to_string())?;
        let backup = rusqlite::backup::Backup::new(&source_conn, &mut destination_conn)
            .map_err(|_| "无法切换恢复数据库".to_string())?;
        backup
            .run_to_completion(32, std::time::Duration::from_millis(5), None)
            .map_err(|_| "无法切换恢复数据库".to_string())?;
        drop(backup);
        *key = Some(ActiveKey::StableDek(dek));
        *pending = None;
        Ok(())
    }
    pub fn legacy_key(&self) -> Result<Zeroizing<Vec<u8>>, String> { match &*self.key.lock().unwrap() { Some(ActiveKey::Legacy(key)) => Ok(Zeroizing::new(key.to_vec())), _ => Err("应用已锁定或当前数据库需要 v2 密码记录实现".into()) } }
    pub fn stable_dek(&self) -> Result<Zeroizing<Vec<u8>>, String> { match &*self.key.lock().unwrap() { Some(ActiveKey::StableDek(dek)) => Ok(Zeroizing::new(dek.to_vec())), _ => Err("应用尚未以 v2 Stable DEK 解锁".into()) } }
    pub fn require_legacy_model(&self) -> Result<(), String> { if self.security_model() == SecurityModel::Legacy { Ok(()) } else { Err("该操作尚未接入 v2 安全格式".into()) } }
    pub fn unlock_v2_and_store(&self, password: &str) -> Result<(), String> { let db = self.db.lock().map_err(|_| "安全状态不可用".to_string())?; let dek = migration::unlock_v2_core(&db, password).map_err(|_| "主密码不正确或安全数据损坏".to_string())?; drop(db); self.set_stable_dek(dek); Ok(()) }
    /// 普通解锁的 production 语义（unlock_app IPC 的唯一实现）。
    /// 硬安全边界：legacy 安全模型下永远 fail closed——legacy-only 启动态
    /// 不允许 ActiveKey::Legacy → 进入正常工作模式，唯一出路是两阶段升级。
    pub fn unlock_app_core(&self, master_password: &str) -> Result<Vec<String>, String> {
        if self.security_model() == SecurityModel::StableDekV2 {
            self.unlock_v2_and_store(master_password)?;
            return Ok(Vec::new());
        }
        Err("数据库需要先完成安全升级后再使用".to_string())
    }
    pub fn verify_v2_recovery_phrase(&self, recovery_phrase: &str) -> Result<(), String> {
        let _gate = self.master_wrap_gate.lock().map_err(|_| "安全状态不可用".to_string())?;
        if self.security_model() != SecurityModel::StableDekV2
            || *self.startup_mode.lock().map_err(|_| "安全状态不可用".to_string())?
                != StartupMode::ExistingV2
        {
            return Err("当前密码库不支持恢复短语".to_string());
        }
        if self.key.lock().map_err(|_| "安全状态不可用".to_string())?.is_some() {
            return Err("当前状态不允许恢复主密码".to_string());
        }
        let db = self.db.lock().map_err(|_| "安全状态不可用".to_string())?;
        migration::recover_v2_core(&db, recovery_phrase)
            .map(|_| ())
            .map_err(|_| "恢复短语无效".to_string())
    }
    pub fn recover_v2_with_phrase_and_store(
        &self,
        recovery_phrase: &str,
        new_master_password: &str,
    ) -> Result<(), String> {
        let _gate = self.master_wrap_gate.lock().map_err(|_| "安全状态不可用".to_string())?;
        if self.security_model() != SecurityModel::StableDekV2
            || *self.startup_mode.lock().map_err(|_| "安全状态不可用".to_string())?
                != StartupMode::ExistingV2
        {
            return Err("当前密码库不支持恢复短语".to_string());
        }
        let db = self.db.lock().map_err(|_| "安全状态不可用".to_string())?;
        let mut key_slot = self.key.lock().map_err(|_| "安全状态不可用".to_string())?;
        if key_slot.is_some() {
            return Err("当前状态不允许恢复主密码".to_string());
        }
        let dek = migration::recover_v2_with_phrase(
            &db,
            recovery_phrase,
            new_master_password,
        )
        .map_err(|error| {
            if error == "新主密码长度至少 6 位" || error == "恢复短语无效" {
                error
            } else {
                "无法设置新主密码，请重试".to_string()
            }
        })?;
        // 只有 master metadata 事务 commit 成功后，才把同一个 Stable DEK 安装进运行态。
        *key_slot = Some(ActiveKey::StableDek(dek));
        Ok(())
    }
    pub fn change_v2_master_password(
        &self,
        current_password: &str,
        new_password: &str,
    ) -> Result<(), String> {
        let _gate = self.master_wrap_gate.lock().map_err(|_| "安全状态不可用".to_string())?;
        if self.security_model() != SecurityModel::StableDekV2
            || *self.startup_mode.lock().map_err(|_| "安全状态不可用".to_string())?
                != StartupMode::ExistingV2
        {
            return Err("当前密码库不支持此操作".to_string());
        }
        let active_dek = self.stable_dek()?;
        let db = self.db.lock().map_err(|_| "安全状态不可用".to_string())?;
        migration::change_v2_master_password(
            &db,
            current_password,
            new_password,
            active_dek.as_slice(),
        )
        .map_err(|error| match error.as_str() {
            "新主密码长度至少 6 位" | "新主密码必须与当前主密码不同" | "当前主密码错误" => error,
            _ => "无法修改主密码，请重试".to_string(),
        })
    }
    fn recovery_confirmation_hash(key: &[u8], index: usize, word: &str) -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update(key);
        hasher.update(index.to_le_bytes());
        hasher.update(word.trim().to_lowercase().as_bytes());
        hasher.finalize().into()
    }
    pub fn prepare_v2_recovery_rotation(&self) -> Result<RecoveryRotationPreparation, String> {
        self.prepare_v2_recovery_rotation_inject(None)
    }
    pub fn prepare_v2_recovery_rotation_inject(
        &self,
        fail_at: Option<migration::RecoveryRotationFailPoint>,
    ) -> Result<RecoveryRotationPreparation, String> {
        let _gate = self.master_wrap_gate.lock().map_err(|_| "安全状态不可用".to_string())?;
        if self.security_model() != SecurityModel::StableDekV2
            || *self.startup_mode.lock().map_err(|_| "安全状态不可用".to_string())?
                != StartupMode::ExistingV2
        {
            return Err("当前密码库不支持更换恢复短语".to_string());
        }
        let dek = self.stable_dek()?;
        let mut pending = self
            .pending_recovery_rotation
            .lock()
            .map_err(|_| "安全状态不可用".to_string())?;
        if pending.is_some() {
            return Err("已有待确认的恢复短语".to_string());
        }
        let prepared = migration::prepare_recovery_rotation_inject(dek.as_slice(), fail_at)
            .map_err(|_| "无法生成新的恢复短语，请重试".to_string())?;
        let words: Vec<String> = prepared
            .mnemonic
            .split_whitespace()
            .map(String::from)
            .collect();
        if words.len() != 12 {
            return Err("无法生成新的恢复短语，请重试".to_string());
        }
        let mut indexes = Vec::with_capacity(3);
        let mut rng = rand::thread_rng();
        while indexes.len() < 3 {
            let index = rng.gen_range(0..12);
            if !indexes.contains(&index) {
                indexes.push(index);
            }
        }
        indexes.sort_unstable();
        let confirmation_key = Zeroizing::new(crypto::generate_salt());
        let confirmation_hashes = indexes
            .iter()
            .map(|index| Self::recovery_confirmation_hash(&confirmation_key, *index, &words[*index]))
            .collect();
        let token = uuid::Uuid::new_v4().to_string();
        let dek_fingerprint = Sha256::digest(dek.as_slice()).into();
        *pending = Some(PendingRecoveryRotation {
            token: token.clone(),
            params_r_json: prepared.params_r_json,
            wrapped_dek_r: prepared.wrapped_dek_r,
            confirmation_indexes: indexes.clone(),
            confirmation_hashes,
            confirmation_key,
            dek_fingerprint,
        });
        Ok(RecoveryRotationPreparation {
            rotation_token: token,
            recovery_words: words,
            confirmation_indexes: indexes,
        })
    }
    pub fn confirm_v2_recovery_rotation(
        &self,
        rotation_token: &str,
        confirmation_words: &[String],
    ) -> Result<(), String> {
        self.confirm_v2_recovery_rotation_inject(rotation_token, confirmation_words, None)
    }
    pub fn confirm_v2_recovery_rotation_inject(
        &self,
        rotation_token: &str,
        confirmation_words: &[String],
        fail_at: Option<migration::RecoveryRotationFailPoint>,
    ) -> Result<(), String> {
        let _gate = self.master_wrap_gate.lock().map_err(|_| "安全状态不可用".to_string())?;
        if self.security_model() != SecurityModel::StableDekV2
            || *self.startup_mode.lock().map_err(|_| "安全状态不可用".to_string())?
                != StartupMode::ExistingV2
        {
            return Err("当前密码库不支持更换恢复短语".to_string());
        }
        let dek = self.stable_dek()?;
        let mut pending = self
            .pending_recovery_rotation
            .lock()
            .map_err(|_| "安全状态不可用".to_string())?;
        let prepared = pending
            .as_ref()
            .ok_or_else(|| "恢复短语轮换状态无效".to_string())?;
        if prepared.token != rotation_token
            || confirmation_words.len() != prepared.confirmation_indexes.len()
            || Sha256::digest(dek.as_slice()).as_slice() != prepared.dek_fingerprint
        {
            return Err("恢复短语轮换状态无效".to_string());
        }
        let matches = confirmation_words.iter().enumerate().all(|(input_index, word)| {
            Self::recovery_confirmation_hash(
                &prepared.confirmation_key,
                prepared.confirmation_indexes[input_index],
                word,
            ) == prepared.confirmation_hashes[input_index]
        });
        if !matches {
            return Err("恢复词确认不匹配".to_string());
        }
        let db = self.db.lock().map_err(|_| "安全状态不可用".to_string())?;
        migration::commit_recovery_rotation_inject(
            &db,
            &prepared.params_r_json,
            &prepared.wrapped_dek_r,
            fail_at,
        )
        .map_err(|_| "无法更换恢复短语，请重试".to_string())?;
        drop(db);
        pending.take();
        Ok(())
    }
    pub fn cancel_v2_recovery_rotation(&self, rotation_token: &str) -> Result<(), String> {
        let _gate = self.master_wrap_gate.lock().map_err(|_| "安全状态不可用".to_string())?;
        if self.security_model() != SecurityModel::StableDekV2 {
            return Err("当前密码库不支持更换恢复短语".to_string());
        }
        let mut pending = self
            .pending_recovery_rotation
            .lock()
            .map_err(|_| "安全状态不可用".to_string())?;
        match pending.as_ref() {
            Some(prepared) if prepared.token == rotation_token => {
                pending.take();
                Ok(())
            }
            _ => Err("恢复短语轮换状态无效".to_string()),
        }
    }
    // ============================================================
    // 🔒 两阶段 legacy 安全升级（Prepare → Confirm；0.3.0 唯一 legacy 出口）
    // ============================================================
    pub fn legacy_migration_status(&self) -> LegacyMigrationStatus {
        match self.pending_legacy_migration.lock() {
            Ok(pending) => LegacyMigrationStatus {
                active: pending.is_some(),
                migration_token: pending.as_ref().map(|p| p.token.clone()),
            },
            Err(_) => LegacyMigrationStatus { active: false, migration_token: None },
        }
    }

    pub fn prepare_legacy_migration(
        &self,
        master_password: &str,
    ) -> Result<LegacyMigrationPreparation, String> {
        let _gate = self
            .master_wrap_gate
            .lock()
            .map_err(|_| "安全状态不可用".to_string())?;
        if self.security_model() != SecurityModel::Legacy
            || *self
                .startup_mode
                .lock()
                .map_err(|_| "安全状态不可用".to_string())?
                != StartupMode::Legacy
        {
            return Err("当前状态不需要安全升级".to_string());
        }
        let app_dir = self
            .db_path
            .parent()
            .ok_or_else(|| "安全数据库路径无效".to_string())?
            .to_path_buf();
        // 文件系统层二次确认：确实处于 legacy-only 启动态（防运行态与磁盘状态漂移）。
        match migration::resolve_startup_db(&app_dir, false).selection {
            migration::DbSelection::Legacy(_) => {}
            _ => return Err("当前状态不需要安全升级".to_string()),
        }
        {
            let fresh = self
                .pending_v2
                .lock()
                .map_err(|_| "安全状态不可用".to_string())?;
            if fresh.is_some() {
                return Err("已有进行中的安全初始化".to_string());
            }
        }
        let mut pending_slot = self
            .pending_legacy_migration
            .lock()
            .map_err(|_| "安全状态不可用".to_string())?;
        if pending_slot.is_some() {
            return Err("已有进行中的安全升级".to_string());
        }
        let tmp_path = app_dir.join(migration::V2_TMP_FILENAME);
        if tmp_path.exists() {
            return Err("存在未完成的升级临时文件，请重启应用后重试".to_string());
        }
        let setup_lock_path = app_dir.join(migration::V2_SETUP_LOCK_FILENAME);
        let mut owns_lock = false;
        let result = (|| -> Result<(PendingLegacyMigration, LegacyMigrationPreparation), String> {
            let mut lock_options = std::fs::OpenOptions::new();
            lock_options.write(true).create_new(true);
            #[cfg(windows)]
            {
                use std::os::windows::fs::OpenOptionsExt;
                lock_options.share_mode(0);
            }
            let migration_lock = lock_options
                .open(&setup_lock_path)
                .map_err(|_| "安全升级正在进行或存在未完成状态".to_string())?;
            owns_lock = true;
            // 迁移核心（Phase 2D 已验收）在这里被正式产品路径复用；
            // legacy 严格只读，正式 v2 与归档均不受影响。
            let built = migration::build_migrated_v2_tmp(&app_dir, master_password, None)
                .map_err(|error| match error {
                    migration::MigrationError::Msg(message) => message,
                    migration::MigrationError::Sql(_) => "原始数据库无法读取，升级中止".to_string(),
                })?;
            if built.mnemonic.len() != 12 {
                return Err("无法生成恢复短语，请重试".to_string());
            }
            let mut indexes = Vec::with_capacity(3);
            let mut rng = rand::thread_rng();
            while indexes.len() < 3 {
                let index = rng.gen_range(0..12);
                if !indexes.contains(&index) {
                    indexes.push(index);
                }
            }
            indexes.sort_unstable();
            let confirmation_key = Zeroizing::new(crypto::generate_salt());
            let confirmation_hashes = indexes
                .iter()
                .map(|index| {
                    Self::recovery_confirmation_hash(
                        &confirmation_key,
                        *index,
                        &built.mnemonic[*index],
                    )
                })
                .collect();
            let token = uuid::Uuid::new_v4().to_string();
            let preparation = LegacyMigrationPreparation {
                migration_token: token.clone(),
                recovery_words: built.mnemonic.clone(),
                confirmation_indexes: indexes.clone(),
            };
            let pending = PendingLegacyMigration {
                token,
                tmp_path: built.tmp_path,
                v2_path: built.v2_path,
                legacy_path: built.legacy_path,
                dek: built.dek,
                source_fingerprint: built.source_fingerprint,
                confirmation_indexes: indexes,
                confirmation_hashes,
                confirmation_key,
                migration_lock: Some(migration_lock),
                migration_lock_path: setup_lock_path.clone(),
            };
            Ok((pending, preparation))
        })();
        match result {
            Ok((pending, preparation)) => {
                *pending_slot = Some(pending);
                Ok(preparation)
            }
            Err(error) => {
                // 失败必须无脏状态：lock + tmp 全部清理，legacy 与正式 v2 不受影响。
                if owns_lock {
                    let _ = std::fs::remove_file(&setup_lock_path);
                    let _ = std::fs::remove_file(&tmp_path);
                    if let Some(name) = tmp_path.file_name() {
                        for suffix in ["-wal", "-shm", "-journal"] {
                            let mut sidecar = name.to_os_string();
                            sidecar.push(suffix);
                            let _ = std::fs::remove_file(app_dir.join(sidecar));
                        }
                    }
                }
                Err(error)
            }
        }
    }

    pub fn confirm_legacy_migration(
        &self,
        migration_token: &str,
        confirmation_words: &[String],
    ) -> Result<(), String> {
        let _gate = self
            .master_wrap_gate
            .lock()
            .map_err(|_| "安全状态不可用".to_string())?;
        if self.security_model() != SecurityModel::Legacy
            || *self
                .startup_mode
                .lock()
                .map_err(|_| "安全状态不可用".to_string())?
                != StartupMode::Legacy
        {
            return Err("当前状态不需要安全升级".to_string());
        }
        let mut pending_slot = self
            .pending_legacy_migration
            .lock()
            .map_err(|_| "安全状态不可用".to_string())?;
        {
            let pending = pending_slot
                .as_ref()
                .ok_or_else(|| "安全升级状态无效".to_string())?;
            if pending.token != migration_token
                || confirmation_words.len() != pending.confirmation_indexes.len()
            {
                return Err("安全升级状态无效".to_string());
            }
            let matches = confirmation_words.iter().enumerate().all(|(input_index, word)| {
                Self::recovery_confirmation_hash(
                    &pending.confirmation_key,
                    pending.confirmation_indexes[input_index],
                    word,
                ) == pending.confirmation_hashes[input_index]
            });
            if !matches {
                return Err("恢复词确认不匹配".to_string());
            }
        }
        // Source-change 保护：prepare → confirm 期间 legacy 源库必须逻辑不变，
        // 且不依赖 mtime —— 用 prepare 时的一致快照指纹复验。
        let (fingerprint, source_changed) = {
            let pending = pending_slot
                .as_ref()
                .ok_or_else(|| "安全升级状态无效".to_string())?;
            let fingerprint = migration::legacy_db_fingerprint(&pending.legacy_path)
                .map_err(|_| "无法读取原始数据库，升级已取消".to_string())?;
            (fingerprint, fingerprint != pending.source_fingerprint)
        };
        if source_changed {
            // fail closed：基于 stale snapshot 的 v2 绝不激活；
            // 清干净 pending 与 tmp，用户可直接重新发起升级。
            let stale = pending_slot.take();
            drop(pending_slot);
            if let Some(stale) = stale {
                stale.discard_files();
            }
            let _ = fingerprint;
            return Err("升级期间原始数据发生变化，请重新开始升级".to_string());
        }
        // 激活前最终校验 pending tmp。
        {
            let pending = pending_slot
                .as_ref()
                .ok_or_else(|| "安全升级状态无效".to_string())?;
            let verified = migration::open_existing_v2_db(&pending.tmp_path)
                .map_err(|_| "安全升级数据校验失败，请重新开始升级".to_string())?;
            drop(verified);
        }
        // 在持久化提交（rename）前先取得所有运行态锁，避免
        // 「正式库已切换但 AppState 因锁问题半更新」的失败窗口。
        let mut startup_mode = self
            .startup_mode
            .lock()
            .map_err(|_| "安全状态不可用".to_string())?;
        let mut db_slot = self.db.lock().map_err(|_| "安全状态不可用".to_string())?;
        let mut key_slot = self.key.lock().map_err(|_| "安全状态不可用".to_string())?;
        let pending = pending_slot
            .as_ref()
            .ok_or_else(|| "安全升级状态无效".to_string())?;
        let app_dir = pending
            .legacy_path
            .parent()
            .ok_or_else(|| "安全数据库路径无效".to_string())?
            .to_path_buf();
        let activated_db = migration::activate_migrated_v2(&app_dir)
            .map_err(|_| "无法完成安全升级，请重试".to_string())?;
        // ===== 持久化提交点已过：正式 drawer-v2.db 已就位。=====
        // 此后任何失败都不回滚——即便进程立刻崩溃，下次启动仲裁选择 v2，
        // 用同一主密码 Master unlock 即可进入（不依赖内存 pending）。
        *db_slot = activated_db;
        *key_slot = Some(ActiveKey::StableDek(pending.dek.clone()));
        *startup_mode = StartupMode::ExistingV2;
        drop(db_slot);
        drop(key_slot);
        drop(startup_mode);
        let done = pending_slot
            .take()
            .ok_or_else(|| "安全升级状态无效".to_string())?;
        drop(pending_slot);
        // legacy 隔离归档：旧 legacy 连接已随 db_slot 替换释放后才 rename；
        // 归档失败不影响已完成的 v2（下次启动仲裁仍选择 v2）。
        if let Err(error) = migration::archive_legacy_source(&app_dir) {
            eprintln!("[legacy-migration] legacy 归档失败（不影响已完成的 v2）: {}", error);
        }
        // token 永久失效 + 释放/删除 setup lock（tmp 已 rename 走，仅清理 lock）。
        done.discard_files();
        Ok(())
    }

    pub fn cancel_legacy_migration(&self, migration_token: &str) -> Result<(), String> {
        let _gate = self
            .master_wrap_gate
            .lock()
            .map_err(|_| "安全状态不可用".to_string())?;
        let mut pending_slot = self
            .pending_legacy_migration
            .lock()
            .map_err(|_| "安全状态不可用".to_string())?;
        match pending_slot.as_ref() {
            Some(pending) if pending.token == migration_token => {
                let stale = pending_slot.take();
                drop(pending_slot);
                if let Some(stale) = stale {
                    stale.discard_files();
                }
                Ok(())
            }
            _ => Err("安全升级状态无效".to_string()),
        }
    }

    pub fn prepare_v2_initialization(&self, password: &str) -> Result<Vec<String>, String> {
        let mut startup_mode = self.startup_mode.lock().map_err(|_| "安全状态不可用".to_string())?;
        // 注意：此处已持有 startup_mode 锁，不得再调 security_model()（会二次加锁死锁）。
        // FreshV2 ⇔ StableDekV2 是恒等映射，只查 startup_mode 即可。
        if *startup_mode != StartupMode::FreshV2 {
            return Err("当前状态不允许初始化安全数据库".to_string());
        }
        let app_dir = self.db_path.parent().ok_or_else(|| "安全数据库路径无效".to_string())?;
        let output = migration::prepare_fresh_v2(app_dir, password)
            .map_err(|_| "安全数据库初始化失败".to_string())?;
        let words = output.mnemonic;
        let mut pending = self.pending_v2.lock().map_err(|_| "安全状态不可用".to_string())?;
        *pending = Some(PendingV2Initialization {
            tmp_path: output.tmp_path,
            v2_path: output.v2_path,
            dek: output.dek,
            setup_lock: output.setup_lock,
            setup_lock_path: output.setup_lock_path,
        });
        *startup_mode = StartupMode::PendingV2;
        Ok(words)
    }
    pub fn finalize_v2_initialization(&self) -> Result<(), String> {
        let mut startup_mode = self.startup_mode.lock().map_err(|_| "安全状态不可用".to_string())?;
        if *startup_mode == StartupMode::ExistingV2 && self.is_unlocked() {
            return Ok(());
        }
        if *startup_mode != StartupMode::PendingV2 {
            return Err("当前状态不允许完成安全数据库初始化".to_string());
        }
        let mut pending = self.pending_v2.lock().map_err(|_| "安全状态不可用".to_string())?;
        let prepared = pending.as_ref().ok_or_else(|| "安全初始化状态缺失".to_string())?;
        // 在正式 rename 前先取得所有运行态锁，避免 rename 后因锁中毒留下半更新 AppState。
        let mut db_slot = self.db.lock().map_err(|_| "安全状态不可用".to_string())?;
        let mut key_slot = self.key.lock().map_err(|_| "安全状态不可用".to_string())?;
        migration::finalize_prepared_v2(&prepared.tmp_path, &prepared.v2_path)
            .map_err(|_| "安全数据库初始化失败".to_string())?;
        let completed_db = migration::open_existing_v2_db(&prepared.v2_path)
            .map_err(|_| "安全数据库初始化失败".to_string())?;
        let prepared = pending.take().ok_or_else(|| "安全初始化状态缺失".to_string())?;
        *db_slot = completed_db;
        *key_slot = Some(ActiveKey::StableDek(prepared.dek));
        *startup_mode = StartupMode::ExistingV2;
        drop(prepared.setup_lock);
        let _ = std::fs::remove_file(prepared.setup_lock_path);
        Ok(())
    }
}

// ===== 应用入口 =====

/// Phase 2C-1：Data Root Blocked 模式。
/// 状态文件/guard 异常时：DB 永不打开、托盘与快捷键不创建，
/// 只挂 BlockedState 供前端 `get_data_root_block` 读取并渲染阻断页。
fn enter_blocked_mode(app: &tauri::App, reason: data_root::BlockedReason, config_root: PathBuf) {
    eprintln!("[data-root] 启动阻断: {:?}", reason);
    app.manage(data_root::BlockedState { reason, config_root });
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.show();
        let _ = win.set_focus();
    }
}

pub fn run() {
    tauri::Builder::default()
        // 单实例必须是第一个注册的插件：第二实例在插件 init 阶段就把参数转发给
        // 主实例并退出进程，绝不进入 setup（不重复开 DB / 建托盘 / 注册快捷键）。
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(win) = app.get_webview_window("main") {
                #[cfg(windows)]
                {
                    if win_dock::is_hidden() {
                        let _ = win_dock::force_reveal(&win);
                    }
                }
                let _ = win.unminimize();
                let _ = win.show();
                let _ = win.set_focus();
            }
        }))
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_clipboard_manager::init())
        .setup(|app| {
            // 初始化数据库（或在启动前先执行 factory_reset，100% 避开 Windows SQLite 文件锁）
            // Phase 2C-1：config_root（= app_data_dir，state/guard 的家）与 effective_root
            // （真正的 Data Root，DB 的家）从此分离。默认两者相同 → 老用户零行为漂移。
            let config_root = app
                .path()
                .app_data_dir()
                .expect("failed to get app data dir");
            std::fs::create_dir_all(&config_root).expect("failed to create app data dir");
            // Data Root 解析必须在任何 SQLite 打开之前完成（2C-0 设计硬约束）。
            let effective_root = match data_root::resolve_data_root(&config_root) {
                data_root::Resolution::UseDefault(root) => root,
                data_root::Resolution::UseExternal(root) => root,
                // 2C-1 没有 pending 恢复流程 → fail closed（恢复流程属 2C-3）
                data_root::Resolution::Pending(op) => {
                    enter_blocked_mode(
                        app,
                        data_root::BlockedReason::PendingOperationNeedsRecovery(format!(
                            "{} / {} / {}",
                            op.op_type, op.op_id, op.phase
                        )),
                        config_root.clone(),
                    );
                    return Ok(());
                }
                data_root::Resolution::Blocked(reason) => {
                    enter_blocked_mode(app, reason, config_root.clone());
                    return Ok(());
                }
            };
            migration::discard_abandoned_fresh_initialization(&effective_root)
                .map_err(|reason| std::io::Error::new(std::io::ErrorKind::Other, reason))?;
            // 两阶段 legacy 升级的 orphan tmp 清理：legacy 存在且无正式 v2 时，
            // 上次升级中断留下的 tmp/lock 安全移除，保证用户永远可以重新升级。
            migration::discard_abandoned_migration_tmp(&effective_root)
                .map_err(|reason| std::io::Error::new(std::io::ErrorKind::Other, reason))?;
            let arbitration = migration::resolve_startup_db(&effective_root, true);
            let (db_path, security_model, startup_mode) = match arbitration.selection {
                migration::DbSelection::V2(path) => (path, SecurityModel::StableDekV2, StartupMode::ExistingV2),
                migration::DbSelection::Legacy(path) => (path, SecurityModel::Legacy, StartupMode::Legacy),
                migration::DbSelection::FreshV2(path) => (path, SecurityModel::StableDekV2, StartupMode::FreshV2),
                migration::DbSelection::Blocked(reason) => return Err(std::io::Error::new(std::io::ErrorKind::Other, reason).into()),
            };

            // ============================================================
            // 🔴 B3-factory_reset 核心修复：启动前先删（100% 无句柄）
            // ============================================================
            // 如果检测到数据目录下有 .factory_reset_pending 标记文件，说明上一次请求了重置
            // → 在 SQLite 打开之前（绝对零句柄）物理删掉 db + WAL/SHM + 图标缓存
            // → Windows os error 32 文件锁问题彻底解决
            let pending_flag = config_root.join(".factory_reset_pending");
            if pending_flag.exists() && security_model == SecurityModel::Legacy {
                use std::fs;
                eprintln!("[factory_reset] 检测到 .factory_reset_pending → 在 SQLite 打开前执行物理删除");
                let db_file_name = db_path.file_name().unwrap().to_string_lossy().to_string();
                let db_wal = effective_root.join(format!("{}-wal", db_file_name));
                let db_shm = effective_root.join(format!("{}-shm", db_file_name));
                let icons_dir = effective_root.join("icons");
                let icon_cache_dir = effective_root.join("icon_cache");
                let mut n = 0usize;
                for p in [&db_path, &db_wal, &db_shm] {
                    if p.exists() { match fs::remove_file(p) { Ok(()) => n += 1, Err(e) => eprintln!("[factory_reset] 删 {:?} 失败: {}", p, e) } }
                }
                for d in [&icons_dir, &icon_cache_dir] {
                    if d.exists() { match fs::remove_dir_all(d) { Ok(()) => n += 1, Err(e) => eprintln!("[factory_reset] 删目录 {:?} 失败: {}", d, e) } }
                }
                // 最后删掉标记文件，避免下次启动再删一次
                let _ = fs::remove_file(&pending_flag);
                eprintln!("[factory_reset] 启动前清理完成，共删除 {} 项 → 进入首次设置向导", n);
            } else if pending_flag.exists() {
                return Err(std::io::Error::new(std::io::ErrorKind::Other, "v2 模式拒绝执行 legacy factory reset pending").into());
            }

            let db = match startup_mode {
                StartupMode::Legacy => Db::open(&db_path),
                StartupMode::ExistingV2 => migration::open_existing_v2_db(&db_path).map_err(|_| rusqlite::Error::InvalidQuery),
                StartupMode::FreshV2 => migration::open_v2_db_in_memory().map_err(|_| rusqlite::Error::InvalidQuery),
                StartupMode::PendingV2 => unreachable!("PendingV2 只存在于当前进程内存"),
            }.expect("failed to open database");

            app.manage(AppState {
                db: Mutex::new(db),
                db_path,
                startup_mode: Mutex::new(startup_mode),
                pending_v2: Mutex::new(None),
                pending_recovery_rotation: Mutex::new(None),
                pending_legacy_migration: Mutex::new(None),
                master_wrap_gate: Mutex::new(()),
                key: Mutex::new(None),
            });

            // 🔴 P0-#Y#SANITIZE#DB：启动后立刻执行 apps 表脏数据全表修正
            //   修正所有 app_type 与 path 实际语义不一致的历史记录（比如 URL 被写成 folder）
            let _sanitize_result = {
                let state = app.state::<AppState>();
                match apps::sanitize_db_on_startup(&state) {
                    Ok(n) if n > 0 => eprintln!("[setup] 🟢 DB 脏数据修正：共修正 {} 条 apps.app_type 不一致记录", n),
                    Ok(_) => eprintln!("[setup] 🟢 DB 脏数据检查：所有 apps.app_type 一致，无需修正"),
                    Err(e) => eprintln!("[setup] ⚠️  DB 脏数据修正失败（非致命，继续）：{}", e),
                }
            };

            // 调试：图标加载失败由前端通过 log_frontend command 写到 frontend.log

            // 系统托盘
            let show_item = MenuItem::with_id(app, "show", "显示窗口", true, None::<&str>)?;
            let hide_item = MenuItem::with_id(app, "hide", "隐藏窗口", true, None::<&str>)?;
            let lock_item = MenuItem::with_id(app, "lock", "锁定", true, None::<&str>)?;
            let quit_item = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show_item, &hide_item, &lock_item, &quit_item])?;

            // ===== 🎨 托盘图标：用图1（白底彩色主图标）=====
            // 用 include_bytes! 编译进二进制，开发/打包都能找到，不依赖运行时路径
            let tray_png = include_bytes!("../icons/icon.png");
            let tray_img = image::load_from_memory(tray_png).expect("托盘图标 icon.png 解码失败");
            let tray_rgba8 = tray_img.into_rgba8();
            let tray_icon = Image::new(
                tray_rgba8.as_raw(), // Image::new 需要 &[u8]（切片引用，不是 Vec）
                tray_rgba8.width(),
                tray_rgba8.height(),
            );
            eprintln!("[setup] ✅ 托盘图标加载成功（彩色主图标，尺寸 {}x{}）", tray_rgba8.width(), tray_rgba8.height());

            let _tray = TrayIconBuilder::new()
                .icon(tray_icon)
                .menu(&menu)
                .tooltip("抽屉柜 Drawer - Alt+Q 唤起")
                .on_menu_event(|app, event| {
                    if let Some(window) = app.get_webview_window("main") {
                        match event.id.as_ref() {
                            "show" => {
                                #[cfg(windows)]
                                {
                                    if win_dock::is_hidden() {
                                        let _ = win_dock::force_reveal(&window);
                                    }
                                }
                                let _ = window.show();
                                let _ = window.set_focus();
                            }
                            "hide" => {
                                let _ = window.hide();
                            }
                            "lock" => {
                                if let Some(state) = app.try_state::<AppState>() {
                                    state.clear_key();
                                }
                                let _ = window.emit("app:lock", ());
                                let _ = window.hide();
                            }
                            "quit" => app.exit(0),
                            _ => {}
                        }
                    }
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        let app = tray.app_handle();
                        if let Some(window) = app.get_webview_window("main") {
                            if window.is_visible().unwrap_or(false) {
                                let _ = window.hide();
                            } else {
                                #[cfg(windows)]
                                {
                                    if win_dock::is_hidden() {
                                        let _ = win_dock::force_reveal(&window);
                                    }
                                }
                                let _ = window.show();
                                let _ = window.set_focus();
                            }
                        }
                    }
                })
                .build(app)?;

            // 关闭按钮：隐藏到托盘（不退出进程）。
            // H-09（2026-10 定向体检发现）：原实现只 hide 不 prevent_close，
            // Tauri 2 下窗口仍被销毁 → 进程直接退出，「隐藏到托盘」从未生效
            // （v0.3.2 release 实测复现）。
            let window = app.get_webview_window("main").unwrap();
            // ===== 安装 Win32 贴边自动隐藏 =====
            // 修复 P0-#Z：彻底禁用 win_dock.rs
            // 原因（已确认是元凶）：
            //   1. Tauri 2 启动后 widget 窗口位置是 (1541, 14)，Top 边缘阈值 20px → 立即触发 schedule_hide
            //   2. 200ms 后 SetWindowPos 把窗口隐藏到屏幕顶部触角（用户看不到）
            //   3. 用户看不到窗口 → 系统判定 "未响应"（死循环）
            //   4. 之前 user 反馈"先停掉"也建议关掉这个
            // 后续：v2 重写为"前端 tauri://move + Rust 端 GetWindowRect 兜底"无侵入实现
            #[cfg(windows)]
            {
                // 暂时禁用：let _ = win_dock::install(&window);
                eprintln!("[win_dock] 暂时禁用（已确认是未响应元凶）");
            }
            let window_clone = window.clone();
            window.on_window_event(move |event| {
                if let WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    let _ = window_clone.hide();
                }
            });

            // P0-#T：启动时自动清理过期 trash（根据 settings 中的 trash_retention_days，默认 30 天）
            let state_ref = app.state::<AppState>();
            let db_guard = state_ref.db.lock().unwrap();
            let retention_days: i64 = match db_guard.get_setting("trash_retention_days") {
                Ok(Some(v)) => v.parse::<i64>().unwrap_or(30).max(1),
                _ => 30,
            };
            let retention_ms: i64 = retention_days * 24 * 60 * 60 * 1000;
            eprintln!("[trash] 保留天数: {} 天 ({} ms)", retention_days, retention_ms);
            match db_guard.cleanup_expired_trash(retention_ms) {
                Ok(results) => {
                    let total: usize = results.iter().map(|(_, n)| n).sum();
                    if total > 0 {
                        eprintln!("[trash] 自动清理 {} 项过期内容", total);
                        for (table, n) in results {
                            eprintln!("[trash]   {} 清理 {} 项", table, n);
                        }
                    }
                }
                Err(e) => eprintln!("[trash] 自动清理失败: {}", e),
            }
            drop(db_guard);

            // 全局快捷键
            let shortcut_plugin = app.global_shortcut();
            let app_handle_q = app.handle().clone();
            let app_handle_l = app.handle().clone();

            for attempt in 0..5 {
                match shortcut_plugin.on_shortcut("Alt+Q", {
                    let h = app_handle_q.clone();
                    move |_app, _shortcut, event| {
                        // 修复 P0：tauri-plugin-global-shortcut 的 on_shortcut 会同时触发
                        // Pressed + Released 两次事件，代码之前没看 state 导致 toggle 两次
                        // （按下出现、松开消失）。只响应 Pressed。
                        if event.state() != ShortcutState::Pressed {
                            return;
                        }
                        if let Some(win) = h.get_webview_window("main") {
                            if win.is_visible().unwrap_or(false) {
                                let _ = win.hide();
                            } else {
                                // 关键修复：Alt+Q 弹窗直接 setSize(480, 720) + show + focus
                                // 之前流程：setSize(380, 560) + emit "force-expand" → 前端 forceExpand
                                //         → setMiniModeWithPos(false) → setSize(480, 720)
                                //   步骤多，emit 失败 / event listener 没注册好 → Alt+Q 完全不灵
                                // 现在：一次性 setSize(480, 720) + show，**不依赖前端事件**
                                // 位置由 OS 保留（hide 不会改 position）
                                let _ = win.set_size(tauri::PhysicalSize::new(480, 720));
                                let _ = win.show();
                                let _ = win.set_focus();
                                // 仍 emit 一次让前端知道 miniMode 状态可清
                                let _ = h.emit("widget:force-expand", ());
                            }
                        }
                    }
                }) {
                    Ok(_) => break,
                    Err(e) => {
                        // 修复 P1-#17：快捷键注册失败要打日志（不能静默）
                        eprintln!("[shortcut] Alt+Q 注册失败 (attempt {}): {}", attempt + 1, e);
                        std::thread::sleep(std::time::Duration::from_millis(500));
                    }
                }
            }

            // 修复 P1-#17：Alt+L 注册失败也要打日志
            if let Err(e) = shortcut_plugin.on_shortcut("Alt+L", move |_app, _shortcut, event| {
                // 同样只响应 Pressed
                if event.state() != ShortcutState::Pressed {
                    return;
                }
                if let Some(win) = app_handle_l.get_webview_window("main") {
                    if let Some(state) = _app.try_state::<AppState>() {
                        state.clear_key();
                    }
                    let _ = win.emit("app:lock", ());
                    let _ = win.hide();
                }
            }) {
                eprintln!("[shortcut] Alt+L 注册失败: {}", e);
            }

            // P0-#ALT+M：Alt+M 切换 完整 / 迷你模式
            let app_handle_m = app.handle().clone();
            if let Err(e) = shortcut_plugin.on_shortcut("Alt+M", move |_app, _shortcut, event| {
                if event.state() != ShortcutState::Pressed {
                    return;
                }
                let _ = app_handle_m.emit("widget:toggle-mini", ());
            }) {
                eprintln!("[shortcut] Alt+M 注册失败: {}", e);
            }

            // ===== 🔴 中危2 FIX：任务栏双图标（幽灵图标）根因&修复 =====
            // 根因：tauri.conf.json visible=true 先让窗口显示 → Windows 立刻在任务栏创建图标 A
            //      → skipTaskbar=true 才生效（但图标 A 已经创建了不会消失！→ 幽灵图标 A + 真实图标 并存）
            // 修复：visible=false 启动（先不让窗口显示）→ 所有初始化完成、skipTaskbar 已经绑定好之后 → 手动 show()
            //      → Windows 直接跳过创建任务栏按钮，完美解决
            if let Some(win) = app.get_webview_window("main") {
                let _ = win.show();
                let _ = win.set_focus();
            }
            eprintln!("[setup] ✅ 全部初始化完成 → 手动 show() 主窗口（skipTaskbar 已绑定，杜绝任务栏双图标）");

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            is_first_run,
            get_security_status,
            initialize_v2_security,
            finalize_v2_security,
            verify_v2_recovery_phrase,
            recover_v2_with_phrase,
            change_v2_master_password,
            prepare_v2_recovery_rotation,
            confirm_v2_recovery_rotation,
            cancel_v2_recovery_rotation,
            // 两阶段 legacy 安全升级（0.3.0 唯一 legacy 出口）
            prepare_legacy_migration,
            confirm_legacy_migration,
            cancel_legacy_migration,
            get_legacy_migration_status,
            setup_master_password,
            unlock_app,
            lock_app,
            // P0-#PW#2ND：独立二次验证密码（默认跟随主密码，可单独设）
            has_second_password,
            change_second_password,
            // P0-#PW#PER#ITEM#VERIFY：每条目二次验证（含独立密码分叉逻辑）
            verify_password_for_pw_view,
            verify_master_password, // 兼容旧版前端，内部直接路由到 verify_password_for_pw_view
            // 🚨 紧急救援：密码全部解密失败时，用旧主密码解密重加密救回
            rescue_passwords_with_master,
            // P1-#SETTINGS#PW#CHANGE#FULL：主密码修改 + 全量重加密
            change_master_password,
            // 通用：图标 data URL 转换（绕过 asset protocol 加载失败）
            read_icon_as_data_url,
            // P0-#ICON#NUCLEAR#ALL#IN#ONE：一次拿所有 app + 所有 icon
            list_apps_with_icons,
            // P0-#LOCK#POLLING#FALLBACK：查询 lock 状态 + build 时间戳
            get_app_status,
            // 通用：前端日志通道
            log_frontend,
            list_passwords,
            create_password,
            get_password_decrypted,
            update_password,
            delete_password,
            // P1-#PW#USE#PERSIST：密码使用次数累加
            bump_password_use_count,
            generate_random_password,
            password_strength,
            copy_to_clipboard_with_timeout,
            toggle_window,
            show_window,
            hide_window,
            // P0-#LOCK#POLLING#FALLBACK：查询 lock 状态 + build 时间戳
            get_app_status,  // 这里注册的是下方带 State 参数的版本（含 lock 状态 + build_timestamp）
            save_widget_config,
            load_widget_config,
            get_window_position,
            get_window_size,
            set_window_position,
            set_window_size,
            set_always_on_top,
            set_ignore_cursor_events,
            set_mini_mode,
            set_mini_mode_with_pos,
            apply_widget_config,
            // 贴边自动隐藏（Win32 钩子）
            set_auto_hide_enabled,
            is_dock_hidden,
            force_dock_reveal,
            // 软件区
            scan_installed_software,
            list_apps,
            fill_missing_icons,
            // P0-#Y#FIX#ICON#REEXTRACT/RESTART/VERSION：用户反馈"图标不对"时用
            force_reextract_icons,
            restart_app,
            get_app_version,
            // P0-#Y#FIX#MIGRATE/TRASH/SETTING：设置界面用
            migrate_data,
            hard_purge_trash,
            get_setting,
            set_setting,
            rescan_subtypes,
            list_app_categories,
            create_app_category,
            update_app_category,
            delete_app_category,
            create_app,
            update_app,
            delete_app,
            record_app_usage,
            launch_app,
            import_paths,
            // 片段区
            list_snippets,
            create_snippet,
            update_snippet,
            delete_snippet,
            get_snippet_content,
            record_snippet_usage,
            // 临时区
            create_temp,
            list_temp,
            delete_temp,
            cleanup_expired_temp,
            // 回收站（P0-#T）
            list_trash,
            restore_from_trash,
            permanent_delete,
            empty_trash,
            cleanup_trash,
            trash_count,
            // 启动健康检查 + 数据目录
            health_check_apps,
            get_data_dir,
            // Phase 2C-1：Data Root Blocked 模式查询（前端阻断页数据源）
            data_root::get_data_root_block,
            // P0-#Y#FIX#PICK：原生文件 / 文件夹选择对话框
            pick_path,
            // B1：导入/导出加密备份
            export_encrypted_backup,
            import_encrypted_backup,
            // B3：重置所有数据（工厂还原）
            factory_reset,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

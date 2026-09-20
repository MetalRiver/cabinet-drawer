//! ===== 命令分组 ⑧：🛠️ 设置 + 🗄️ 加密备份 + 🔥 恢复 + 🧹 数据目录迁移 =====
//! 说明：
//! - 所有命令签名、实现 100% 与 lib.rs 原版一致
//! - 避免臆想：所有 struct / 字段名 / 方法名 完全按 lib.rs 原版抄

use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use std::path::{Path, PathBuf};
use tauri::{AppHandle, State};

use crate::backup;
use crate::crypto;
use crate::db::{self, Db};
use crate::AppState;
use zeroize::Zeroizing;

// ============================================================
// 🔐 验证主密码（100% 照 lib.rs unlock_app 前半段，零臆想！）
// ============================================================
fn verify_master_password_inner(
    state: &State<'_, AppState>,
    master_password: &str,
) -> Result<Zeroizing<Vec<u8>>, String> {
    state.require_legacy_model()?;
    if let Ok(k) = state.legacy_key() { return Ok(k); }
    let db = state.db.lock().unwrap();
    // 注意：key 是 master_password_hash / master_password_salt（不是 master_salt！不是 hex！是 BASE64！）
    let hash = db
        .get_setting("master_password_hash")
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "未设置主密码".to_string())?;
    let salt_b64 = db
        .get_setting("master_password_salt")
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "未设置主密码".to_string())?;
    drop(db);
    let salt = BASE64.decode(&salt_b64).map_err(|e| e.to_string())?;
    // 注意：用 crypto::hash_password()，不是 hash_sha256()
    if crypto::hash_password(master_password, &salt) != hash {
        return Err("主密码错误".to_string());
    }
    let key = Zeroizing::new(crypto::derive_key(master_password, &salt));
    state.set_legacy_key(Zeroizing::new(key.to_vec()));
    Ok(key)
}

// ============================================================
// 🔥 1. 工厂重置（清库 + 重启）100% 原版抄
// ============================================================
#[tauri::command]
pub fn factory_reset(state: State<AppState>, app: AppHandle) -> Result<(), String> {
    state.require_legacy_model()?;
    let _ = state;
    let app_clone = app.clone();
    use tauri::Manager;
    let config_dir = app_clone
        .path()
        .app_data_dir()
        .map_err(|e| format!("无法获取配置目录: {}", e))?;
    let pending = config_dir.join(".factory_reset_pending");
    if let Some(parent) = pending.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    std::fs::write(&pending, "reset\n").map_err(|e| format!("写 pending 失败: {}", e))?;
    let exe = std::env::current_exe().map_err(|e| format!("获取 exe 路径失败: {}", e))?;
    use std::process::Command;
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        Command::new(&exe)
            .args(&["--factory-reset-confirm"])
            .creation_flags(0x08000000)
            .spawn()
            .map_err(|e| format!("启动新进程失败: {}", e))?;
    }
    #[cfg(not(windows))]
    {
        Command::new(&exe)
            .args(&["--factory-reset-confirm"])
            .spawn()
            .map_err(|e| format!("启动新进程失败: {}", e))?;
    }
    app.exit(0);
    Ok(())
}

// ============================================================
// 🗄️ 2. 导出加密备份（弹窗选保存位置，返回实际路径+统计）
// ============================================================
#[derive(serde::Serialize)]
pub struct ExportResult {
    pub path: String,
    pub passwords_decrypted_ok: usize,
    pub passwords_decrypted_failed: usize,
    pub trash_passwords: usize,
}

#[tauri::command]
pub fn export_encrypted_backup(
    state: State<AppState>,
    master_password: String,
) -> Result<ExportResult, String> {
    state.require_legacy_model()?;
    // 0. 验证主密码（导出必须知道密码才能加密）
    let _key = verify_master_password_inner(&state, &master_password)?;

    // 1. 弹窗选保存路径
    let now = chrono::Local::now();
    let default_name = format!(
        "drawerbox-backup-{}.drawerbox",
        now.format("%Y%m%d-%H%M%S")
    );
    let save_path = rfd::FileDialog::new()
        .set_title("保存加密备份到...")
        .set_file_name(&default_name)
        .add_filter("抽屉柜备份文件", &["drawerbox"])
        .set_directory(
            dirs::home_dir()
                .or_else(dirs::download_dir)
                .unwrap_or_else(|| PathBuf::from(".")),
        )
        .save_file()
        .ok_or_else(|| "用户取消".to_string())?;

    // 2. 导出数据
    let mut data = {
        let db = state.db.lock().unwrap();
        let mut d = db.export_all_data().map_err(|e| e.to_string())?;
        // ✨ 存一份当前的 master_password_salt 进备份（兜底用）
        // 场景：工厂重置后导入旧格式密文备份 → 用备份里存的旧salt + 主密码 派生出旧key解密重加密
        // 安全性：备份文件本身 AES-256-GCM 加密，存 salt 不影响安全
        if let Ok(Some(salt_b64)) = db.get_setting("master_password_salt") {
            d.export_master_password_salt_b64 = Some(salt_b64);
        }
        d
    };

    // 2b. ✨ 关键修复：先解密所有密码成明文存到 password_plaintext！
    // 🔴 【风险2修复】不再跳过回收站密码！回收站的也必须解密成明文，否则换电脑导入后还原解密失败！
    {
        let key = state.legacy_key().map_err(|_| "应用未锁定状态异常（缺少主密钥）".to_string())?;
        let mut decrypted_ok = 0usize;
        let mut decrypted_failed = 0usize;
        let mut trash_count = 0usize;
        for pw in data.passwords.iter_mut() {
            let is_trash = pw.deleted_at.is_some();
            if is_trash { trash_count += 1; }
            // 先用当前 key 解密 encrypted_password（数据库里的密文）
            match crypto::decrypt(&pw.encrypted_password, &key) {
                Ok(plaintext) => {
                    pw.password_plaintext = Some(plaintext);
                    pw.encrypted_password = String::new(); // 清掉密文，避免混淆（导入时优先用明文）
                    decrypted_ok += 1;
                }
                Err(_) => {
                    // 解密失败：保留原密文（给旧格式兼容/同电脑恢复用），标记明文为空
                    pw.password_plaintext = None;
                    decrypted_failed += 1;
                }
            }
        }
        eprintln!(
            "[export_encrypted_backup] 密码导出预处理：明文解密成功 {} 条，失败 {} 条，其中回收站密码 {} 条",
            decrypted_ok, decrypted_failed, trash_count
        );

        // 4. 写文件前先把统计存起来（后面要返回）
        let result_path = save_path.to_string_lossy().to_string();
        let result_ok = decrypted_ok;
        let result_failed = decrypted_failed;
        let result_trash = trash_count;

        let data_json = serde_json::to_vec(&data).map_err(|e| format!("序列化失败: {}", e))?;

        // 3. 加密
        let ciphertext = backup::encrypt_backup(&data_json, &master_password)?;

        // 4. 写文件
        if let Some(parent) = save_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        std::fs::write(&save_path, &ciphertext).map_err(|e| format!("写入文件失败: {}", e))?;

        eprintln!("[export_encrypted_backup] 已写入: {}", result_path);
        Ok(ExportResult {
            path: result_path,
            passwords_decrypted_ok: result_ok,
            passwords_decrypted_failed: result_failed,
            trash_passwords: result_trash,
        })
    }
}

// ============================================================
// 🗄️ 3. 导入加密备份（弹窗选文件 + 按冲突策略恢复 + 清 key 强制重解锁）100% 原版抄
// ============================================================
#[tauri::command]
pub fn import_encrypted_backup(
    state: State<AppState>,
    file_path: String,
    master_password: String,
    policy: db::ImportConflictPolicy,
) -> Result<db::ImportStats, String> {
    // v2 备份格式尚未设计完成：必须在读取/解密任何备份内容前关闭。
    state.require_legacy_model()?;
    let p = Path::new(&file_path);
    let exists = p.exists();
    let size = if exists {
        p.metadata().map(|m| m.len()).unwrap_or(0)
    } else {
        0
    };
    eprintln!(
        "[import_encrypted_backup] file={} exists={} size={} policy={:?}",
        file_path, exists, size, policy
    );

    if !exists {
        return Err(format!("文件不存在: {}", file_path));
    }

    // 1. 校验文件头（快速判断是不是备份文件）
    let header_magic_yes = {
        match std::fs::File::open(&file_path) {
            Ok(mut f) => {
                use std::io::Read;
                let mut buf = vec![0u8; 16];
                let read = f.read(&mut buf).unwrap_or(0);
                if read < backup::MAGIC.len() {
                    false
                } else {
                    let ok = &buf[0..backup::MAGIC.len()] == backup::MAGIC;
                    eprintln!(
                        "[import_encrypted_backup] 读取 header {} 字节，MAGIC(前10B={:?}) 匹配={}",
                        read, &buf[0..backup::MAGIC.len()], ok
                    );
                    ok
                }
            }
            Err(e) => {
                eprintln!("[import_encrypted_backup] 打开文件失败: {}", e);
                return Err(format!("打开文件失败: {}", e));
            }
        }
    };
    if !header_magic_yes {
        return Err(
            "这不是抽屉柜的备份文件（缺少 MAGIC 头）。请选择 .drawerbox 文件。".to_string(),
        );
    }

    // 2. 读整个文件
    let bytes =
        std::fs::read(&file_path).map_err(|e| format!("读取文件失败: {}", e))?;
    eprintln!(
        "[import_encrypted_backup] 读取 {} 字节成功",
        bytes.len()
    );

    // 3. 解密
    let json_bytes = backup::decrypt_backup(&bytes, &master_password)?;
    eprintln!(
        "[import_encrypted_backup] 解密成功！JSON 共 {} 字节",
        json_bytes.len()
    );

    // 先转成 Value 以兼容旧版结构。不得记录解密后的备份正文。
    let json_value: serde_json::Value = match serde_json::from_slice(&json_bytes) {
        Ok(v) => v,
        Err(e) => return Err(format!("备份 JSON 语法错误: {}", e)),
    };
    if let Some(obj) = json_value.as_object() {
        let keys: Vec<&String> = obj.keys().collect();
        eprintln!(
            "[import_encrypted_backup] JSON 所有顶级字段 ({:?}) 及类型：",
            keys
        );
        for (k, v) in obj {
            let t = match v {
                serde_json::Value::Null => "Null",
                serde_json::Value::Bool(_) => "Bool",
                serde_json::Value::Number(_) => "Number",
                serde_json::Value::String(_) => "String",
                serde_json::Value::Array(a) => &format!("Array(len={})", a.len())[..],
                serde_json::Value::Object(o) => &format!("Object(keys={:?})", o.keys().collect::<Vec<_>>())[..],
            };
            eprintln!("  · {:?} → {}", k, t);
        }
    } else {
        eprintln!(
            "[import_encrypted_backup] ⚠️ JSON 顶级不是对象！类型={:?}",
            json_value
        );
    }

    // 4. 解析（先转 Value，兼容旧版结构，避免字段名对不上报错）
    let mut data: db::BackupFullData =
        serde_json::from_value(json_value).map_err(|e| format!("备份数据损坏: {}", e))?;

    // 4b. ✨ 关键修复：密码预处理（3 级优先级兼容所有情况）
    // 优先级 1：新格式 password_plaintext 明文 → 用新电脑 key 加密
    // 优先级 2：V2 新备份有 export_master_password_salt_b64 → 旧salt+备份密码派生出旧key解密 → 新key重加密
    // 优先级 3：只有旧格式 encrypted_password 密文 → 直接保留（仅同电脑同salt同主密码可用，否则需救援功能）
    {
        let key = state.legacy_key().map_err(|_| "应用未锁定状态异常（缺少主密钥）".to_string())?;

        // 提前计算优先级 2 需要的旧 key（如果备份有存旧salt）
        let old_key_from_backup_salt: Option<Zeroizing<Vec<u8>>> = match data.export_master_password_salt_b64.as_ref() {
            Some(old_salt_b64) => {
                use base64::{Engine as _, engine::general_purpose::STANDARD as B64};
                match B64.decode(old_salt_b64) {
                    Ok(old_salt) if old_salt.len() >= 8 => {
                        // 备份密码 = 导出时用户设的主密码（常规操作习惯）
                        Some(Zeroizing::new(crypto::derive_key(&master_password, &old_salt)))
                    }
                    _ => None,
                }
            }
            None => None,
        };
        if old_key_from_backup_salt.is_some() {
            eprintln!("[import_encrypted_backup] ✨ 检测到备份内有旧 master_password_salt，启用「旧key解密重加密」模式");
        }

        let mut reencrypted_plain = 0usize;
        let mut reencrypted_backup_salt = 0usize;
        let mut used_old_cipher = 0usize;
        let mut no_password = 0usize;
        let mut trash_count = 0usize;
        for pw in data.passwords.iter_mut() {
            // 🔴 【风险3修复】不再跳过回收站密码！回收站的也必须重加密，否则换电脑导入后还原解密失败！
            if pw.deleted_at.is_some() { trash_count += 1; }
            // ========== 优先级 1：新格式明文 ==========
            if let Some(plain) = pw.password_plaintext.as_ref() {
                if !plain.is_empty() {
                    let encrypted_new = crypto::encrypt(plain, &key).map_err(|e| e.to_string())?;
                    pw.encrypted_password = encrypted_new;
                    reencrypted_plain += 1;
                    continue;
                }
            }
            // ========== 优先级 2：备份有旧salt → 旧salt+备份密码派生旧key解密 ==========
            if !pw.encrypted_password.is_empty() {
                if let Some(old_key) = old_key_from_backup_salt.as_ref() {
                    if let Ok(plaintext) = crypto::decrypt(&pw.encrypted_password, old_key) {
                        // 旧key解密成功 → 用新key重加密
                        let encrypted_new = crypto::encrypt(&plaintext, &key).map_err(|e| e.to_string())?;
                        pw.encrypted_password = encrypted_new;
                        reencrypted_backup_salt += 1;
                        continue;
                    }
                }
            }
            // ========== 优先级 3：旧格式密文（兼容） ==========
            if !pw.encrypted_password.is_empty() {
                // ⚠️ 既没有明文，备份也没旧salt → 只有密文
                // 这个密文是旧电脑用「旧主密码 + 旧salt」派生的密钥加密的
                // 只有「同一台电脑 & 主密码没变 & salt 没变」才能解开！
                // 换电脑/工厂重置（salt变了）→ 这里不解密，保留密文导入，
                // 用户看到解密失败后，需用救援功能（rescue_passwords_with_master）输入旧主密码救回
                used_old_cipher += 1;
                continue;
            }
            no_password += 1;
        }
        eprintln!(
            "[import_encrypted_backup] 密码导入预处理：新格式(明文)={} 条，备份旧salt解起重加密={} 条，旧格式(保留密文)={} 条，空密码={} 条，其中回收站密码 {} 条",
            reencrypted_plain, reencrypted_backup_salt, used_old_cipher, no_password, trash_count
        );
    }

    // 5. 导入（事务内，失败自动回滚）
    let stats = {
        let db = state.db.lock().unwrap();
        db.import_all_data(&data, policy.clone()).map_err(|e| e.to_string())?
    };

    // 注意：ImportStats 的真实字段名（100% 按 db.rs 1057-1072 行）
    eprintln!(
        "[import_encrypted_backup] 完成: {:?} | settings={} | cats_inserted={} cats_conflict={} | apps_inserted={} skipped={} overwritten={} | pws_inserted={} skipped={} overwritten={} | snippets_inserted={} skipped={} overwritten={} | temps_inserted={} skipped={} overwritten={}",
        policy,
        stats.settings,
        stats.categories_inserted,
        stats.categories_conflict,
        stats.apps_inserted,
        stats.apps_skipped,
        stats.apps_overwritten,
        stats.passwords_inserted,
        stats.passwords_skipped,
        stats.passwords_overwritten,
        stats.snippets_inserted,
        stats.snippets_skipped,
        stats.snippets_overwritten,
        stats.temps_inserted,
        stats.temps_skipped,
        stats.temps_overwritten,
    );

    // ✅【重要】不再强制锁屏！！
    // 导入时已经跳过了主密码相关的 settings（master_hash/salt/恢复短语/二次验证），
    // 本地的主密钥完全没变，不需要清 key 让用户重新解锁！
    // 用户能直接看到导入结果！
    /* （旧逻辑已删除）
    // 6. 清空主密码缓存（备份里带了另一个 master_salt / hash）
    state.clear_key();
    */

    Ok(stats)
}

// ============================================================
// 🔢 4. 软件版本号
// ============================================================
#[tauri::command]
pub fn get_app_version(app: AppHandle) -> String {
    app.package_info().version.to_string()
}

// ============================================================
// 🧹 5. 数据目录搬迁（target_path 参数！不是 v1→v2 迁移！100% 原版抄）
// ============================================================
#[derive(serde::Serialize)]
pub struct MigrateResult {
    pub files_copied: usize,
    pub bytes_copied: u64,
    pub source_dir: String,
    pub target_dir: String,
}

#[tauri::command]
pub fn migrate_data(state: State<AppState>, target_path: String) -> Result<MigrateResult, String> {
    state.require_legacy_model()?;
    use std::fs;
    let source = state.db_path.parent().ok_or("无法获取源目录")?.to_path_buf();
    let target = PathBuf::from(&target_path);
    if !source.exists() {
        return Err(format!("源目录不存在：{}", source.display()));
    }
    if target.exists() {
        eprintln!("[migrate] 目标目录已存在：{}（会合并覆盖）", target.display());
    }
    fs::create_dir_all(&target).map_err(|e| format!("创建目标失败：{}", e))?;
    let mut files_copied = 0usize;
    let mut bytes_copied = 0u64;
    fn copy_dir(
        src: &PathBuf,
        dst: &PathBuf,
        count: &mut usize,
        bytes: &mut u64,
    ) -> std::io::Result<()> {
        if !dst.exists() {
            fs::create_dir_all(dst)?;
        }
        for entry in fs::read_dir(src)? {
            let entry = entry?;
            let ft = entry.file_type()?;
            let src_p = entry.path();
            let dst_p = dst.join(entry.file_name());
            if ft.is_dir() {
                copy_dir(&src_p, &dst_p, count, bytes)?;
            } else {
                let n = fs::copy(&src_p, &dst_p)?;
                *count += 1;
                *bytes += n;
            }
        }
        Ok(())
    }
    copy_dir(&source, &target, &mut files_copied, &mut bytes_copied)
        .map_err(|e| format!("复制文件失败：{}", e))?;
    Ok(MigrateResult {
        files_copied,
        bytes_copied,
        source_dir: source.to_string_lossy().to_string(),
        target_dir: target.to_string_lossy().to_string(),
    })
}

// ============================================================
// 🗑️ 6. 永久彻底清空回收站（硬删除！不用 db 方法，直接 SQL，100% 原版抄）
// ============================================================
#[tauri::command]
pub fn hard_purge_trash(state: State<AppState>) -> Result<usize, String> {
    state.require_legacy_model()?;
    let db = state.db.lock().unwrap();
    let mut total = 0;
    for table in ["apps", "passwords", "snippets", "temp_contents"] {
        let sql = format!("DELETE FROM {} WHERE deleted_at IS NOT NULL", table);
        let n = db
            .conn
            .lock()
            .unwrap()
            .execute(&sql, [])
            .map_err(|e| format!("清空 {} 失败：{}", table, e))?;
        total += n;
    }
    eprintln!("[hard_purge_trash] 共清空 {} 项", total);
    Ok(total)
}

// ============================================================
// ⚙️ 7. 设置 CRUD（注意 map_err！rusqlite::Error → String）100% 原版抄
// ============================================================
#[tauri::command]
pub fn get_setting(state: State<AppState>, key: String) -> Result<Option<String>, String> {
    state.require_legacy_model()?;
    let db = state.db.lock().unwrap();
    db.get_setting(&key).map_err(|e| e.to_string())
}
#[tauri::command]
pub fn set_setting(state: State<AppState>, key: String, value: String) -> Result<(), String> {
    state.require_legacy_model()?;
    let db = state.db.lock().unwrap();
    db.set_setting(&key, &value).map_err(|e| e.to_string())
}

// 让 imports 不 warning（有的用了有的没用到）
#[allow(unused_imports)]
fn _unused() {
    let _ = BASE64.encode(b"");
    let _ = crypto::generate_salt;
}

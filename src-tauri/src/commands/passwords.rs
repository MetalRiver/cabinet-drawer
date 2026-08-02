//! ===== 命令分组 ②：🔑 密码管理 CRUD =====
//! 包含：增删改查密码、解密单条、生成随机密码、强度检测、使用计数

use tauri::State;

use crate::crypto;
use crate::db::{self, Db};
use crate::AppState;

// ============================================================
// 🔑 列出所有密码元信息（不含解密后的密码）
// ============================================================
#[tauri::command]
pub fn list_passwords(state: State<AppState>) -> Result<Vec<db::PasswordMeta>, String> {
    if state.key.lock().unwrap().is_none() {
        return Err("应用已锁定".to_string());
    }
    let db = state.db.lock().unwrap();
    db.list_passwords().map_err(|e| e.to_string())
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
    let key_guard = state.key.lock().unwrap();
    let key = key_guard.as_ref().ok_or_else(|| "应用已锁定".to_string())?;
    let encrypted = crypto::encrypt(&password, key).map_err(|e| e.to_string())?;
    drop(key_guard);

    let db = state.db.lock().unwrap();
    db.create_password(&title, &username, &encrypted, &url, &notes)
        .map_err(|e| e.to_string())
}

// ============================================================
// 🔑 获取单条密码的明文（解密后返回前端）
// ============================================================
#[tauri::command]
pub fn get_password_decrypted(state: State<AppState>, id: i64) -> Result<String, String> {
    use std::io::Write;

    let key_guard = state.key.lock().unwrap();
    let key = key_guard.as_ref().ok_or_else(|| "应用已锁定".to_string())?;
    let db = state.db.lock().unwrap();
    let (title, username, encrypted, url, notes) = db.get_password_encrypted(id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "密码不存在".to_string())?;

    // ✅ 超级诊断：写日志文件到 %TEMP%\drawer-box-debug.log
    let log_path = std::env::temp_dir().join("drawer-box-debug.log");
    let _ = std::fs::OpenOptions::new().create(true).append(true).open(&log_path)
        .and_then(|mut f| writeln!(f, "
[DEBUG decrypt id={}]
  标题: {:?} | 用户名: {:?} | URL: {:?}
  state.key 长度: {} 字节 | key 前8字节(Hex): {}",
            id, title, username, url,
            key.len(),
            &key.iter().take(8).map(|b| format!("{:02X}", b)).collect::<String>()
        ));

    let enc_bytes = encrypted.as_bytes();
    let enc_len = enc_bytes.len();
    let enc_head_hex: String = enc_bytes.iter().take(24.min(enc_len))
        .map(|b| format!("{:02X}", b)).collect();
    let enc_tail_hex: String = if enc_len > 24 {
        enc_bytes.iter().skip(enc_len.saturating_sub(16)).take(16)
            .map(|b| format!("{:02X}", b)).collect()
    } else { String::new() };
    let enc_b64 = {
        use base64::{Engine as _, engine::general_purpose::STANDARD};
        STANDARD.encode(enc_bytes)
    };

    let _ = std::fs::OpenOptions::new().create(true).append(true).open(&log_path)
        .and_then(|mut f| writeln!(f, "
  密文长度: {} 字节
  密文前24字节(Hex): {}
  密文后16字节(Hex): {}
  密文前100字符(Base64): {}
",
            enc_len,
            if enc_head_hex.is_empty() { "(空)" } else { &enc_head_hex },
            if enc_tail_hex.is_empty() { "(空)" } else { &enc_tail_hex },
            &enc_b64.chars().take(100).collect::<String>()
        ));

    // ✅ 终极手动解密诊断！不调 crypto::decrypt！自己一步一步来！100% 看清哪里错了！
    {
        use base64::{Engine as _, engine::general_purpose::STANDARD as B64};
        use aes_gcm::{Aes256Gcm, KeyInit, Nonce, aead::Aead};
        use std::io::Write;

        // 先把日志文件打开，存到 log_file 变量里！后面所有分支都能用！
        let mut log_file = std::fs::OpenOptions::new()
            .create(true).append(true).open(&log_path).ok();
        macro_rules! write_log {
            ($($t:tt)*) => {
                if let Some(ref mut f) = log_file { let _ = writeln!(f, $($t)*); }
            };
        }

        write_log!("
  ──────── 终极解密诊断开始 ────────
  1. encrypted 是 Base64 字符串，先解码...");

        let combined = match B64.decode(&encrypted) {
            Ok(c) => {
                write_log!("  ✅ Base64 解码成功！长度: {} 字节", c.len());
                c
            },
            Err(e) => {
                write_log!("  ❌ Base64 解码失败: {}", e);
                let err_msg = format!(
"解密失败！终极诊断：
❌ Base64 解码就失败了！说明密文根本不是 Base64 编码的！
错误原因：{}
密文内容: {}
", e, encrypted);
                return Err(err_msg);
            }
        };

        // 拆分 nonce(12B) + ciphertext+tag
        write_log!("
  2. 拆分 nonce(前12B) 和 ciphertext+tag(剩余)
     nonce 长度应=12, 总长度应 >= 12 + 16(tag) = 28
     实际 combined_len={}, combined_len >= 28 ? {}",
            combined.len(),
            combined.len() >= 28
        );

        if combined.len() < 28 {
            write_log!("  ❌ combined 长度不够！不是合法密文！");
            let err_msg = format!(
"解密失败！终极诊断：
❌ 密文长度非法！Base64解码后只有 {} 字节！
至少需要 28 字节（12字节nonce + 至少16字节tag）
", combined.len());
            return Err(err_msg);
        }
        let (nonce_bytes, ciphertext_with_tag) = combined.split_at(12);
        let nonce_hex: String = nonce_bytes.iter().map(|b| format!("{:02X}", b)).collect();
        let ct_head_hex: String = ciphertext_with_tag.iter().take(16).map(|b| format!("{:02X}", b)).collect();
        let ct_tail_hex: String = ciphertext_with_tag.iter().rev().take(16).collect::<Vec<_>>().iter().rev().map(|b| format!("{:02X}", b)).collect();
        write_log!("
  3. nonce(12B): {}
     ciphertext+tag 长度: {} 字节 (前16B: {}... 后16B: ...{})
  4. 用 state.key 构造 AES-256-GCM，尝试解密...
",
            nonce_hex,
            ciphertext_with_tag.len(),
            ct_head_hex,
            ct_tail_hex
        );

        let cipher = match Aes256Gcm::new_from_slice(key) {
            Ok(c) => c,
            Err(e) => {
                write_log!("  ❌ 构造 cipher 失败: {}", e);
                return Err(format!("构造 cipher 失败: key_len={}, err={}", key.len(), e));
            }
        };
        let nonce = Nonce::from_slice(nonce_bytes);
        match cipher.decrypt(nonce, ciphertext_with_tag) {
            Ok(plaintext_bytes) => {
                match String::from_utf8(plaintext_bytes) {
                    Ok(plaintext) => {
                        write_log!("  ✅✅✅ 终极解密成功！明文长度 {} 字节！前50字符: {}",
                            plaintext.len(),
                            &plaintext.chars().take(50).collect::<String>()
                        );
                        return Ok(plaintext);
                    },
                    Err(e) => {
                        write_log!("  ❌ 明文不是 UTF-8: {}", e);
                        return Err(format!("解密成功但明文不是 UTF-8: {}", e));
                    }
                }
            },
            Err(e) => {
                write_log!("
  ❌❌❌ AEAD 解密失败！原因只有 2 个：
     🔑 要么是【密钥不对】（加密这个密文的 key 和现在 state.key 不是同一个！）
     🔒 要么是【密文被篡改/截断】（nonce 或 ciphertext 不对！）

  终极对比：
  - state.key 前8字节: {}
  - nonce(12B): {}
  - ciphertext+tag 长度: {}
  - 完整 encrypted(Base64): {}
",
                    &key.iter().take(8).map(|b| format!("{:02X}", b)).collect::<String>(),
                    nonce_hex,
                    ciphertext_with_tag.len(),
                    encrypted
                );
                return Err(format!(
"解密失败！终极诊断结果：
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
❌ 最终原因：aead::Error（AEAD认证失败！）

👉 可能性 1（99%概率）：【密钥不对！】
   加密这个密文的主密钥，和你现在解锁用的主密钥不是同一个！
   → state.key 前8字节(HEX): {}

👉 可能性 2（1%概率）：【密文损坏了！】
   nonce 或密文不对！
   → nonce(12B, HEX): {}
   → ciphertext+tag 长度: {} 字节
   → 完整密文(Base64): {}
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
",
                    &key.iter().take(8).map(|b| format!("{:02X}", b)).collect::<String>(),
                    nonce_hex,
                    ciphertext_with_tag.len(),
                    encrypted
                ));
            }
        }
    }
}

// ============================================================
// 🔑 累加密码使用次数
// ============================================================
#[tauri::command]
pub fn bump_password_use_count(state: State<AppState>, id: i64) -> Result<i64, String> {
    let db = state.db.lock().unwrap();
    db.bump_password_use_count(id).map_err(|e| e.to_string())
}

// ============================================================
// 🔑 修改一条密码
// ============================================================
#[tauri::command]
pub fn update_password(
    state: State<AppState>,
    id: i64,
    title: String,
    username: String,
    password: String,
    url: String,
    notes: String,
) -> Result<(), String> {
    let key_guard = state.key.lock().unwrap();
    let key = key_guard.as_ref().ok_or_else(|| "应用已锁定".to_string())?;
    let encrypted = crypto::encrypt(&password, key).map_err(|e| e.to_string())?;
    drop(key_guard);

    let db = state.db.lock().unwrap();
    db.update_password(id, &title, &username, &encrypted, &url, &notes)
        .map_err(|e| e.to_string())
}

// ============================================================
// 🔑 删除一条密码（软删除 → 进回收站）
// ============================================================
#[tauri::command]
pub fn delete_password(state: State<AppState>, id: i64) -> Result<(), String> {
    let db = state.db.lock().unwrap();
    db.soft_delete("passwords", id).map_err(|e| e.to_string())?;
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

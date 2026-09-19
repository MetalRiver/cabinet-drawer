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
// 不写任何日志：明文/密文/密钥材料一律不落盘（P0-A，2026-09-19）
// ============================================================
#[tauri::command]
pub fn get_password_decrypted(state: State<AppState>, id: i64) -> Result<String, String> {
    let key_guard = state.key.lock().unwrap();
    let key = key_guard.as_ref().ok_or_else(|| "应用已锁定".to_string())?;
    let db = state.db.lock().unwrap();
    let (_, _, encrypted, _, _) = db
        .get_password_encrypted(id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "密码不存在".to_string())?;
    drop(db);
    // 解密失败时只返回固定文案，不携带密文/密钥/nonce 等任何材料
    crypto::decrypt(&encrypted, key)
        .map_err(|_| "解密失败：密文损坏或与当前主密码不匹配".to_string())
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
    // 1) 锁定检查 + 需要时加密（只持 key 锁，避免 db→key 交叉加锁顺序）
    let encrypted: Option<String> = match password {
        Some(pw) => {
            let key_guard = state.key.lock().unwrap();
            let key = key_guard.as_ref().ok_or_else(|| "应用已锁定".to_string())?;
            Some(crypto::encrypt(&pw, key).map_err(|e| e.to_string())?)
        }
        None => {
            if state.key.lock().unwrap().is_none() {
                return Err("应用已锁定".to_string());
            }
            None
        }
    };

    // 2) 写库（只持 db 锁）
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

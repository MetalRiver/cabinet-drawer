//! ===== 命令分组 ⑤：📝 便签（temp_contents）CRUD =====
//! 包含：增删改查便签、过期清理

use tauri::State;

use crate::db::{self, Db};
use crate::AppState;

// ============================================================
// 📝 新增便签（带 TTL 分钟数）
// ============================================================
#[tauri::command]
pub fn create_temp(state: State<AppState>, text: String, ttl_minutes: i64) -> Result<i64, String> {
    let now = chrono::Utc::now().timestamp_millis();
    let expires_at = now + ttl_minutes * 60 * 1000;
    let db = state.db.lock().unwrap();
    db.create_temp(&text, expires_at).map_err(|e| e.to_string())
}

// ============================================================
// 📝 列出所有便签（未过期 + 未软删除的）
// ============================================================
#[tauri::command]
pub fn list_temp(state: State<AppState>) -> Result<Vec<db::TempMeta>, String> {
    let db = state.db.lock().unwrap();
    db.list_temp().map_err(|e| e.to_string())
}

// ============================================================
// 📝 删除便签（软删除 → 进回收站）
// ============================================================
#[tauri::command]
pub fn delete_temp(state: State<AppState>, id: i64) -> Result<(), String> {
    let db = state.db.lock().unwrap();
    db.soft_delete("temp_contents", id).map_err(|e| e.to_string())?;
    Ok(())
}

// ============================================================
// 📝 清理过期便签（硬删除，不进回收站）
// ============================================================
#[tauri::command]
pub fn cleanup_expired_temp(state: State<AppState>) -> Result<usize, String> {
    let db = state.db.lock().unwrap();
    db.cleanup_expired_temp().map_err(|e| e.to_string())
}

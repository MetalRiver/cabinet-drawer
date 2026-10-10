//! ===== 命令分组 ⑤：📝 便签（temp_contents）CRUD =====
//! 包含：增删改查便签、过期清理

use tauri::State;

use crate::db::{self, Db};
use crate::AppState;

// ============================================================
// 📝 新增便签（带 TTL 分钟数；ttl_minutes <= 0 = 永不过期，v0.4.1）
// ============================================================
#[tauri::command]
pub fn create_temp(state: State<AppState>, text: String, ttl_minutes: i64) -> Result<i64, String> {
    // [data-gate:shared] Phase 2C-3 统一数据操作门（锁顺序: gate → master_wrap_gate → db → conn）
    let _data_gate = state.data_read()?;

    let now = chrono::Utc::now().timestamp_millis();
    let expires_at = if ttl_minutes <= 0 {
        Db::TEMP_NEVER_EXPIRES_MS
    } else {
        now + ttl_minutes.saturating_mul(60 * 1000)
    };
    let db = state.db.lock().unwrap();
    db.create_temp(&text, expires_at).map_err(|e| e.to_string())
}

// ============================================================
// 📝 编辑便签内容（只改 text，不动到期时间 —— v0.4.1 用户反馈）
// ============================================================
#[tauri::command]
pub fn update_temp_text(state: State<AppState>, id: i64, text: String) -> Result<(), String> {
    // [data-gate:shared] Phase 2C-3 统一数据操作门（锁顺序: gate → master_wrap_gate → db → conn）
    let _data_gate = state.data_read()?;

    if text.trim().is_empty() {
        return Err("便签内容不能为空".to_string());
    }
    let db = state.db.lock().unwrap();
    let n = db.update_temp_text(id, &text).map_err(|e| e.to_string())?;
    if n == 0 {
        return Err("便签不存在或已删除".to_string());
    }
    Ok(())
}

// ============================================================
// 📝 续期便签（v0.4.1 用户反馈）
// - add_ms：在现有到期时间上延长（已过期不可见的便签不可续期，走回收站恢复流程）
// - set_expires_at：直接设定绝对到期时间；Db::TEMP_NEVER_EXPIRES_MS = 永不过期
// - 二者必须恰好给一个；返回续期后的新到期时间（供 UI 展示确认）
// ============================================================
#[tauri::command]
pub fn renew_temp(
    state: State<AppState>,
    id: i64,
    add_ms: Option<i64>,
    set_expires_at: Option<i64>,
) -> Result<i64, String> {
    // [data-gate:shared] Phase 2C-3 统一数据操作门（锁顺序: gate → master_wrap_gate → db → conn）
    let _data_gate = state.data_read()?;

    if add_ms.is_some() == set_expires_at.is_some() {
        return Err("add_ms 与 set_expires_at 必须二选一".to_string());
    }
    if let Some(ms) = add_ms {
        if ms <= 0 {
            return Err("延长时长必须大于 0".to_string());
        }
    }
    if let Some(t) = set_expires_at {
        if t != Db::TEMP_NEVER_EXPIRES_MS && t <= chrono::Utc::now().timestamp_millis() {
            return Err("目标到期时间必须晚于当前时间".to_string());
        }
    }
    let db = state.db.lock().unwrap();
    let n = db
        .renew_temp(id, add_ms, set_expires_at)
        .map_err(|e| e.to_string())?;
    if n == 0 {
        return Err("便签不存在或已删除".to_string());
    }
    let conn = db.conn.lock().unwrap();
    let expires_at: i64 = conn
        .query_row(
            "SELECT expires_at FROM temp_contents WHERE id = ?1",
            [id],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    Ok(expires_at)
}

// ============================================================
// 📝 列出所有便签（未过期 + 未软删除的）
// ============================================================
#[tauri::command]
pub fn list_temp(state: State<AppState>) -> Result<Vec<db::TempMeta>, String> {
    // [data-gate:shared] Phase 2C-3 统一数据操作门（锁顺序: gate → master_wrap_gate → db → conn）
    let _data_gate = state.data_read()?;

    let db = state.db.lock().unwrap();
    db.list_temp().map_err(|e| e.to_string())
}

// ============================================================
// 📝 删除便签（软删除 → 进回收站）
// ============================================================
#[tauri::command]
pub fn delete_temp(state: State<AppState>, id: i64) -> Result<(), String> {
    // [data-gate:shared] Phase 2C-3 统一数据操作门（锁顺序: gate → master_wrap_gate → db → conn）
    let _data_gate = state.data_read()?;

    let db = state.db.lock().unwrap();
    db.soft_delete("temp_contents", id).map_err(|e| e.to_string())?;
    Ok(())
}

// ============================================================
// 📝 清理过期便签（硬删除，不进回收站）
// ============================================================
#[tauri::command]
pub fn cleanup_expired_temp(state: State<AppState>) -> Result<usize, String> {
    // [data-gate:shared] Phase 2C-3 统一数据操作门（锁顺序: gate → master_wrap_gate → db → conn）
    let _data_gate = state.data_read()?;

    let db = state.db.lock().unwrap();
    db.cleanup_expired_temp().map_err(|e| e.to_string())
}

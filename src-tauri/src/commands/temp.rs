//! ===== 命令分组 ⑤：📝 便签（temp_contents）CRUD =====
//! 包含：增删改查便签、过期清理

use tauri::State;

use crate::db::{self, Db};
use crate::AppState;

// ============================================================
// 📝 新增便签（带 TTL 分钟数；ttl_minutes == 0 = 永不过期，负数拒绝，v0.4.1）
// 范围校验：负数 / 超上限返回可见错误（不静默截断为永久，R2+放行检查）
// ============================================================
#[tauri::command]
pub fn create_temp(state: State<AppState>, text: String, ttl_minutes: i64) -> Result<i64, String> {
    // [data-gate:shared] Phase 2C-3 统一数据操作门（锁顺序: gate → master_wrap_gate → db → conn）
    let _data_gate = state.data_read()?;

    let now = chrono::Utc::now().timestamp_millis();
    let expires_at = Db::resolve_create_expiry(now, ttl_minutes)?;
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
// 📝 续期便签（v0.4.1 用户反馈；R1 语义收紧）
// - add_ms：在现有到期时间上延长；对永久便签返回明确错误（不累加哨兵）
// - set_expires_at：直接设定绝对到期时间；Db::TEMP_NEVER_EXPIRES_MS = 永不过期；
//   大于哨兵的日期拒绝（防"UI 显示永久、实际会过期"歧义）
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

    let now = chrono::Utc::now().timestamp_millis();
    let db = state.db.lock().unwrap();
    db.renew_temp(id, now, add_ms, set_expires_at)
}

// ============================================================
// 📝 从回收站恢复过期便签并同步设定新到期时间（R6：消除虚假成功）
// 直接恢复已过期便签后列表不可见 → 前端必须走本命令把恢复与新有效期原子落库；
// expires_at 合法性同续期（未来时间或永久哨兵）。
// ============================================================
#[tauri::command]
pub fn restore_temp_expired(
    state: State<AppState>,
    id: i64,
    expires_at: i64,
) -> Result<(), String> {
    // [data-gate:shared] Phase 2C-3 统一数据操作门（锁顺序: gate → master_wrap_gate → db → conn）
    let _data_gate = state.data_read()?;

    Db::validate_renew_target(chrono::Utc::now().timestamp_millis(), expires_at)?;
    let db = state.db.lock().unwrap();
    let n = db
        .restore_temp_with_expiry(id, expires_at)
        .map_err(|e| e.to_string())?;
    if n == 0 {
        return Err("便签不存在或不在回收站".to_string());
    }
    Ok(())
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
    db.soft_delete("temp_contents", id)
        .map_err(|e| e.to_string())?;
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

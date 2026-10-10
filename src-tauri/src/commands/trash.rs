//! ===== 命令分组 ⑥：🗑️ 回收站（trash）CRUD =====
//! 包含：列回收站、恢复、硬删除、清空、自动清理过期、计数

use tauri::State;

use crate::db::{self, Db};
use crate::AppState;

// ============================================================
// 🗑️ 列出回收站所有条目（apps/passwords/snippets/temp_contents 四张表软删除的）
// ============================================================
#[tauri::command]
pub fn list_trash(state: State<AppState>) -> Result<Vec<db::TrashItem>, String> {
    // [data-gate:shared] Phase 2C-3 统一数据操作门（锁顺序: gate → master_wrap_gate → db → conn）
    let _data_gate = state.data_read()?;

    let db = state.db.lock().unwrap();
    db.list_trash().map_err(|e| e.to_string())
}

// ============================================================
// 🗑️ 从回收站恢复（把 deleted_at 置 NULL）
// ============================================================
#[tauri::command]
pub fn restore_from_trash(state: State<AppState>, table: String, id: i64) -> Result<usize, String> {
    // [data-gate:shared] Phase 2C-3 统一数据操作门（锁顺序: gate → master_wrap_gate → db → conn）
    let _data_gate = state.data_read()?;

    let db = state.db.lock().unwrap();
    db.restore(&table, id).map_err(|e| e.to_string())
}

// ============================================================
// 🗑️ 硬删除（物理从数据库删掉，不可恢复）
// ============================================================
#[tauri::command]
pub fn permanent_delete(state: State<AppState>, table: String, id: i64) -> Result<usize, String> {
    // [data-gate:shared] Phase 2C-3 统一数据操作门（锁顺序: gate → master_wrap_gate → db → conn）
    let _data_gate = state.data_read()?;

    let db = state.db.lock().unwrap();
    db.hard_delete(&table, id).map_err(|e| e.to_string())
}

// ============================================================
// 🗑️ 清空指定表的回收站（或 "all" 清全部）
// ============================================================
#[tauri::command]
pub fn empty_trash(state: State<AppState>, table: String) -> Result<usize, String> {
    // [data-gate:shared] Phase 2C-3 统一数据操作门（锁顺序: gate → master_wrap_gate → db → conn）
    let _data_gate = state.data_read()?;

    let db = state.db.lock().unwrap();
    db.empty_trash(&table).map_err(|e| e.to_string())
}

// ============================================================
// 🗑️ 清理超过保留天数的回收站条目（返回每张表清理的数量）
// ============================================================
#[tauri::command]
pub fn cleanup_trash(
    state: State<AppState>,
    retention_days: i64,
) -> Result<Vec<(String, usize)>, String> {
    // [data-gate:shared] Phase 2C-3 统一数据操作门（锁顺序: gate → master_wrap_gate → db → conn）
    let _data_gate = state.data_read()?;

    // v0.4.1：0 = 永久保留（不清理，返回空结果）；负数 = 非法入参，fail visible
    if retention_days < 0 {
        return Err("保留天数不能为负数".to_string());
    }
    if retention_days == 0 {
        return Ok(Vec::new());
    }

    let db = state.db.lock().unwrap();
    let retention_ms = retention_days * 24 * 60 * 60 * 1000;
    let raw = db
        .cleanup_expired_trash(retention_ms)
        .map_err(|e| e.to_string())?;
    Ok(raw.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

// ============================================================
// 🗑️ 回收站条目总数（显示在 Tab 角标）
// ============================================================
#[tauri::command]
pub fn trash_count(state: State<AppState>) -> Result<i64, String> {
    // [data-gate:shared] Phase 2C-3 统一数据操作门（锁顺序: gate → master_wrap_gate → db → conn）
    let _data_gate = state.data_read()?;

    let db_guard = state.db.lock().unwrap();
    let conn = db_guard.conn.lock().unwrap();
    let n: i64 = conn
        .query_row(
            "SELECT
                (SELECT COUNT(*) FROM apps WHERE deleted_at IS NOT NULL) +
                (SELECT COUNT(*) FROM passwords WHERE deleted_at IS NOT NULL) +
                (SELECT COUNT(*) FROM snippets WHERE deleted_at IS NOT NULL) +
                (SELECT COUNT(*) FROM temp_contents WHERE deleted_at IS NOT NULL)",
            [],
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())?;
    Ok(n)
}

// ============================================================
// 🗑️ 启动时按 trash_retention_days 自动清理（v0.4.1 语义收敛）
//
// 设置值语义（唯一实现，lib.rs setup 与单测共用）：
// - 缺失 / 非整数 / 负数 → 安全默认 30 天（与历史默认一致）
// - 0 → 永久保留：直接跳过，绝不执行任何 DELETE
// - 正整数 N → 清理 deleted_at 超过 N 天的条目
// ============================================================
pub fn startup_auto_cleanup(db: &Db) {
    let retention_days: i64 = match db.get_setting("trash_retention_days") {
        Ok(Some(v)) => match v.trim().parse::<i64>() {
            Ok(d) if d >= 0 => d,
            _ => 30,
        },
        _ => 30,
    };
    if retention_days == 0 {
        eprintln!("[trash] 保留天数=0（永久保留）：跳过启动自动清理");
        return;
    }
    let retention_ms: i64 = retention_days.saturating_mul(24 * 60 * 60 * 1000);
    eprintln!(
        "[trash] 保留天数: {} 天 ({} ms)",
        retention_days, retention_ms
    );
    match db.cleanup_expired_trash(retention_ms) {
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
}

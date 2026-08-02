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
    let db = state.db.lock().unwrap();
    db.list_trash().map_err(|e| e.to_string())
}

// ============================================================
// 🗑️ 从回收站恢复（把 deleted_at 置 NULL）
// ============================================================
#[tauri::command]
pub fn restore_from_trash(state: State<AppState>, table: String, id: i64) -> Result<usize, String> {
    let db = state.db.lock().unwrap();
    db.restore(&table, id).map_err(|e| e.to_string())
}

// ============================================================
// 🗑️ 硬删除（物理从数据库删掉，不可恢复）
// ============================================================
#[tauri::command]
pub fn permanent_delete(state: State<AppState>, table: String, id: i64) -> Result<usize, String> {
    let db = state.db.lock().unwrap();
    db.hard_delete(&table, id).map_err(|e| e.to_string())
}

// ============================================================
// 🗑️ 清空指定表的回收站（或 "all" 清全部）
// ============================================================
#[tauri::command]
pub fn empty_trash(state: State<AppState>, table: String) -> Result<usize, String> {
    let db = state.db.lock().unwrap();
    db.empty_trash(&table).map_err(|e| e.to_string())
}

// ============================================================
// 🗑️ 清理超过保留天数的回收站条目（返回每张表清理的数量）
// ============================================================
#[tauri::command]
pub fn cleanup_trash(state: State<AppState>, retention_days: i64) -> Result<Vec<(String, usize)>, String> {
    let db = state.db.lock().unwrap();
    let retention_ms = retention_days * 24 * 60 * 60 * 1000;
    let raw = db.cleanup_expired_trash(retention_ms).map_err(|e| e.to_string())?;
    Ok(raw.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

// ============================================================
// 🗑️ 回收站条目总数（显示在 Tab 角标）
// ============================================================
#[tauri::command]
pub fn trash_count(state: State<AppState>) -> Result<i64, String> {
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

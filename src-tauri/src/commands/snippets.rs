//! ===== 命令分组 ④：📋 代码段（snippets）CRUD =====
//! 包含：增删改查代码段、取内容、记录使用次数

use tauri::State;

use crate::db::{self, Db};
use crate::AppState;

// ============================================================
// 📋 列出代码段（支持搜索）
// ============================================================
#[tauri::command]
pub fn list_snippets(
    state: State<AppState>,
    query: Option<String>,
) -> Result<Vec<db::SnippetMeta>, String> {
    let db = state.db.lock().unwrap();
    db.list_snippets(&query.unwrap_or_default())
        .map_err(|e| e.to_string())
}

// ============================================================
// 📋 新增代码段
// ============================================================
#[tauri::command]
pub fn create_snippet(
    state: State<AppState>,
    title: String,
    content: String,
    language: Option<String>,
    tags: Option<String>,
) -> Result<i64, String> {
    let db = state.db.lock().unwrap();
    db.create_snippet(
        &title,
        &content,
        language.as_deref().unwrap_or("text"),
        tags.as_deref().unwrap_or(""),
    )
    .map_err(|e| e.to_string())
}

// ============================================================
// 📋 修改代码段
// ============================================================
#[tauri::command]
pub fn update_snippet(
    state: State<AppState>,
    id: i64,
    title: String,
    content: String,
    language: Option<String>,
    tags: Option<String>,
) -> Result<(), String> {
    let db = state.db.lock().unwrap();
    db.update_snippet(
        id,
        &title,
        &content,
        language.as_deref().unwrap_or("text"),
        tags.as_deref().unwrap_or(""),
    )
    .map_err(|e| e.to_string())
}

// ============================================================
// 📋 删除代码段（软删除 → 进回收站）
// ============================================================
#[tauri::command]
pub fn delete_snippet(state: State<AppState>, id: i64) -> Result<(), String> {
    let db = state.db.lock().unwrap();
    db.soft_delete("snippets", id).map_err(|e| e.to_string())?;
    Ok(())
}

// ============================================================
// 📋 获取单条代码段内容
// ============================================================
#[tauri::command]
pub fn get_snippet_content(state: State<AppState>, id: i64) -> Result<String, String> {
    let db = state.db.lock().unwrap();
    db.get_snippet_content(id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "片段不存在".to_string())
}

// ============================================================
// 📋 累加代码段使用次数
// ============================================================
#[tauri::command]
pub fn record_snippet_usage(state: State<AppState>, id: i64) -> Result<(), String> {
    let db = state.db.lock().unwrap();
    db.record_snippet_usage(id).map_err(|e| e.to_string())
}

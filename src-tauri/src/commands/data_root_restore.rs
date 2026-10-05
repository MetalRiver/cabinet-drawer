//! ===== Phase 2C-4：External → Default 恢复 + retained source 管理 =====
//!
//! 命令清单：
//! - `setup_begin_restore_default`  外部位置 → 默认位置（复用 2C-3 迁移引擎，反向目标）
//! - `delete_retained_source`       白名单删除已登记的旧数据副本（不可逆，双重确认由前端保证）
//! - `open_retained_source_folder`  只打开旧数据副本所在文件夹（绝不打开/连接归档本身）
//!
//! 不变量（全部沿用 2C-3 已封版语义，未重新设计）：
//! - exclusive gate → G1 durable → state(restore_default) → 才允许向 Config Root 写 staging
//! - staging = `drawer-v2.db.migration-<op_id>.tmp`，与 G1（`drawer-v2.db.tmp`）永不同文件
//! - 正式 drawer-v2.db 已存在 → FAIL CLOSED（绝不 REPLACE）
//! - source（external D）只做 rename 归档，绝不删除；登记进 retained_sources
//! - state 翻转（active_root=null）之后才清理 guard，顺序 G2 → G1

use std::path::PathBuf;

use tauri::{AppHandle, Manager, State};

use crate::data_root::{self, StateLoad};
use crate::migration;
use crate::AppState;

fn config_root(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_data_dir()
        .map_err(|e| format!("无法获取配置目录: {}", e))
}

/// External → Default 恢复（D→C）。流程：preflight → G2 一致性 → exclusive gate →
/// G1 durable → state(transferring, active_root 保持 D) → Online Backup 快照到
/// Config Root 的 operation-specific staging → 验证+指纹 → target_verified →
/// 激活（无 REPLACE，G1 继续保留）→ restart_required + 冻结闩 → 重启 →
/// 新进程 setup 内完成 retirement / retained 登记 / state 翻转 / guard 清理。
#[tauri::command]
pub fn setup_begin_restore_default(app: AppHandle, state: State<AppState>) -> Result<(), String> {
    let cr = config_root(&app)?;

    // 1) 仅 v2 安全模型
    if state.security_model() != crate::SecurityModel::StableDekV2 {
        return Err("仅 v2 安全模型支持恢复默认位置（legacy 数据请先完成安全升级）".to_string());
    }
    // 2) 当前必须处于稳定 external 态
    let loaded = match data_root::load_state(&cr) {
        StateLoad::Loaded(s) => s,
        _ => return Err("当前未处于外部数据位置，无需恢复".to_string()),
    };
    if loaded.pending_operation.is_some() {
        return Err("存在未完成的数据位置操作，请先重启抽屉柜完成或清理".to_string());
    }
    let source = loaded
        .active_root
        .clone()
        .ok_or("当前已处于默认数据位置，无需恢复")?;
    let source_norm = source.to_string_lossy().replace('/', "\\").to_lowercase();
    let cr_norm = cr.to_string_lossy().replace('/', "\\").to_lowercase();
    if source_norm == cr_norm {
        return Err("当前已处于默认数据位置，无需恢复".to_string());
    }
    let source_db = source.join(migration::V2_DB_FILENAME);
    if !source_db.exists() {
        return Err("当前数据位置未找到正式数据库，无法迁移".to_string());
    }

    // 3) 稳定 external 态下可证明的 G1 残留清理（上次中断 restore-begin 的孤儿；
    //    多余 tmp 会错误阻断旧版，必须先移除。非 Magic 的真实 tmp 不在此列，preflight 拒绝）
    if let Some(stale_g1) = data_root::stale_g1_on_stable_external(&cr) {
        let _ = std::fs::remove_file(&stale_g1);
    }

    // 4) preflight（§六）：Config Root 无正式库/未知中间态；已知 retained archive 允许；
    //    未知 migrated-* fail closed；可写 + 固定盘 + 空间
    let mut required = std::fs::metadata(&source_db).map(|m| m.len()).unwrap_or(0);
    required += std::fs::metadata(source.join(format!("{}-wal", migration::V2_DB_FILENAME)))
        .map(|m| m.len())
        .unwrap_or(0);
    required += 64 * 1024 * 1024; // 安全余量
    let mut known_archives: Vec<String> = Vec::new();
    if let Some(lm) = &loaded.last_migration {
        known_archives.push(lm.archive.clone());
    }
    for r in &loaded.retained_sources {
        if let Some(name) = r.archive_path.file_name() {
            known_archives.push(name.to_string_lossy().to_string());
        }
    }
    data_root::preflight_restore_default(&cr, required, &known_archives)
        .map_err(|e| e.message())?;

    // 4b) Upgrade Compatibility Gate：可信 metadata 与磁盘实际归档必须一致（§七）
    data_root::verify_retained_metadata_consistency(&loaded)?;

    // 5) G2 合法且与 active_root=D 一致（§六）
    data_root::external_guard_consistent(&cr, &source)?;

    // 6) exclusive gate（拿不到立即失败）
    let _gate = state.data_write()?;
    let op_id = uuid::Uuid::new_v4().to_string();

    // 7) G1 durable（Guard First，§八）。此时允许 G2 + G1 并存：
    //    G2 阻止旧版打开"无正式库"的稳定外置态，G1 将在正式 C 库出现后继续阻断旧版。
    data_root::write_compat_guard_g1(&cr, &op_id)?;

    // 8) source 语义指纹（exclusive 持有期间 = 冻结后的真实状态）
    let source_fp = {
        let db = state.db.lock().map_err(|_| "数据库不可用".to_string())?;
        let conn = db.conn.lock().map_err(|_| "数据库连接不可用".to_string())?;
        data_root::semantic_fingerprint(&conn)?
    };

    // 9) state(restore_default/transferring)：active_root 保持 D（§七）
    data_root::save_state(
        &cr,
        &data_root::state_pending_restore_default(
            &source,
            &cr,
            &op_id,
            "transferring",
            Some(&source_fp),
            None,
            &loaded,
        ),
    )?;

    // 10) 快照 → 验证 → 激活（激活前失败：回滚到 D，§二十八 A）
    let run = (|| -> Result<(), String> {
        let tmp = data_root::restore_staging_path(&cr, &op_id);
        {
            let db = state.db.lock().map_err(|_| "数据库不可用".to_string())?;
            let conn = db.conn.lock().map_err(|_| "数据库连接不可用".to_string())?;
            data_root::snapshot_source_db(&conn, &tmp)?;
        }
        let target_fp = data_root::verify_migration_target(&tmp, &source_fp)?;
        let verified_state = data_root::state_pending_restore_default(
            &source,
            &cr,
            &op_id,
            "target_verified",
            Some(&source_fp),
            Some(&target_fp),
            &loaded,
        );
        data_root::save_state(&cr, &verified_state)?;
        data_root::activate_target_tmp(&tmp, &cr.join(migration::V2_DB_FILENAME))?;
        let st2 = data_root::state_with_migration_phase(&verified_state, "target_activated");
        data_root::save_state(&cr, &st2)?;
        Ok(())
    })();
    if let Err(e) = run {
        // 激活成功后（正式库已出现）绝不回滚 → 引导重启由收尾流程完成
        if cr.join(migration::V2_DB_FILENAME).exists() {
            state.freeze_data_operations();
            return Err(format!(
                "{}；但默认位置正式库已就位，请重启抽屉柜以完成收尾。",
                e
            ));
        }
        let op = match data_root::load_state(&cr) {
            StateLoad::Loaded(s) => s.pending_operation,
            _ => None,
        };
        match op {
            Some(op) => match data_root::rollback_restore_default(&cr, &op) {
                Ok(()) => {
                    let mut restored = match data_root::load_state(&cr) {
                        StateLoad::Loaded(s) => s,
                        _ => return Err(e),
                    };
                    restored.pending_operation = None;
                    restored.active_root = Some(source.clone());
                    restored.updated_at = chrono::Local::now().to_rfc3339();
                    data_root::save_state(&cr, &restored)?;
                    return Err(format!("{}（已自动回滚，原数据未受影响）", e));
                }
                Err(ge) => {
                    state.freeze_data_operations();
                    return Err(format!(
                        "{}；且回滚失败：{}。已进入保护状态，请重启应用。",
                        e, ge
                    ));
                }
            },
            None => return Err(e),
        }
    }

    // 11) restart_required + 冻结闩 + 重启（gate 持有至进程退出）
    let st = match data_root::load_state(&cr) {
        StateLoad::Loaded(s) => s,
        _ => return Err("恢复收尾读取 state 失败".to_string()),
    };
    data_root::save_state(
        &cr,
        &data_root::state_with_migration_phase(&st, "restart_required"),
    )?;
    state.freeze_data_operations();
    crate::commands::restart_app(app)
}

/// 打开旧数据副本所在文件夹（§二十二：只允许打开所在文件夹，
/// 绝不打开归档本身、绝不自动连接/恢复）。
#[tauri::command]
pub fn open_retained_source_folder(app: AppHandle, op_id: String) -> Result<(), String> {
    let cr = config_root(&app)?;
    let loaded = match data_root::load_state(&cr) {
        StateLoad::Loaded(s) => s,
        _ => return Err("状态文件不可用".to_string()),
    };
    let r = loaded
        .retained_sources
        .iter()
        .find(|r| r.op_id == op_id && r.status == "retained")
        .ok_or("旧数据副本不存在或已删除")?;
    let dir = r.archive_path.parent().ok_or("归档路径无效")?.to_path_buf();
    // Offline Safety Gate：fail visible —— 原位置离线/已移除时绝不假成功
    if !dir.exists() {
        return Err(
            "旧数据副本当前不可访问（原位置离线或已被移除），无法打开所在文件夹。".to_string(),
        );
    }
    use tauri_plugin_shell::ShellExt;
    app.shell()
        .open(dir.to_string_lossy().to_string(), None)
        .map_err(|e| format!("无法打开文件夹: {}", e))
}

/// 删除旧数据副本（不可逆）。白名单 = state.retained_sources 中明确登记的
/// archive_path 及其 -wal/-shm/-journal；顺序：文件删除成功 → 验证不存在 → 更新 metadata。
/// 与迁移操作互斥（exclusive gate）。
#[tauri::command]
pub fn delete_retained_source(
    app: AppHandle,
    state: State<AppState>,
    op_id: String,
) -> Result<(), String> {
    let _gate = state.data_write()?;
    let cr = config_root(&app)?;
    data_root::delete_retained_source_checked(&cr, &op_id)
}

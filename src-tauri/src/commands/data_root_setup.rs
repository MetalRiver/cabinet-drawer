//! ===== Phase 2C-2：Data Root 首次初始化与已有数据重新连接 =====
//!
//! 命令清单：
//! - `setup_check_custom_root`    自定义位置 preflight（纯检查，不写任何东西）
//! - `setup_choose_default`       使用推荐位置（Config Root，不写 state / 不写 guard）
//! - `setup_begin_custom_init`    自定义位置初始化（guard durable → state → 初始化向导）
//! - `setup_resume_custom_init`   继续未完成的初始化（丢弃未确认 tmp，重新生成恢复词）
//! - `setup_attach_existing`      使用已有数据目录（完整验证 → guard → state → 重启提交）
//! - `setup_abandon_pending`      放弃未完成操作（白名单清理 + 删 guard + 删 state → 重启）
//!
//! 不变量：
//! - 任何向 target 写核心数据之前，Guard 必须已 durable、state 必须已落盘（§八 硬顺序）
//! - 未确认 Recovery Phrase 绝不激活正式库（finalize 是唯一激活点，见 SetupWizard 流程）
//! - target 已有正式库绝不覆盖/重命名覆盖（preflight 直接拒绝 → 转 attach）

use std::path::PathBuf;
use std::sync::Mutex;

use tauri::{AppHandle, Manager};

use crate::data_root::{self, DataRootInitCtx, InitContext, SetupModeInfo, SetupState, StateLoad};
use crate::migration;
use crate::{AppState, StartupMode};

fn config_root(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_data_dir()
        .map_err(|e| format!("无法获取配置目录: {}", e))
}

fn build_fresh_app_state(app: &AppHandle, db_path: PathBuf) -> Result<(), String> {
    if app.try_state::<AppState>().is_some() {
        return Err("应用已初始化".to_string());
    }
    let db = migration::open_v2_db_in_memory().map_err(|_| "安全数据库初始化失败".to_string())?;
    app.manage(AppState::new(db, db_path, StartupMode::FreshV2));
    Ok(())
}

fn clear_setup_mode(app: &AppHandle) {
    if let Some(state) = app.try_state::<SetupState>() {
        state.set(None);
    }
}

fn set_init_context(app: &AppHandle, ctx: Option<DataRootInitCtx>) {
    if let Some(state) = app.try_state::<InitContext>() {
        state.set(ctx);
    } else {
        app.manage(InitContext(Mutex::new(ctx)));
    }
}

/// 自定义位置 preflight（前端先检查再确认，不产生任何落盘）
#[tauri::command]
pub fn setup_check_custom_root(app: AppHandle, target: String) -> Result<(), String> {
    let cr = config_root(&app)?;
    data_root::preflight_new_root(&cr, &PathBuf::from(&target)).map_err(|e| e.message())
}

/// 使用推荐位置：Data Root = Config Root。不写 state、不写 guard（§五）。
#[tauri::command]
pub fn setup_choose_default(app: AppHandle) -> Result<(), String> {
    let cr = config_root(&app)?;
    build_fresh_app_state(&app, cr.join(migration::V2_DB_FILENAME))?;
    set_init_context(&app, None);
    clear_setup_mode(&app);
    Ok(())
}

/// 自定义位置初始化。硬顺序：preflight → G2 guard durable → state(preparing) → 才开向导。
#[tauri::command]
pub fn setup_begin_custom_init(app: AppHandle, target: String) -> Result<(), String> {
    let cr = config_root(&app)?;
    let target = PathBuf::from(&target);
    data_root::preflight_new_root(&cr, &target).map_err(|e| e.message())?;
    let op_id = uuid::Uuid::new_v4().to_string();
    let guard_path = data_root::write_compat_guard_g2(&cr, &op_id, &target)?;
    let state = data_root::state_pending("init", &op_id, "preparing", &target);
    if let Err(e) = data_root::save_state(&cr, &state) {
        let _ = std::fs::remove_file(&guard_path);
        return Err(e);
    }
    if let Err(e) = build_fresh_app_state(&app, target.join(migration::V2_DB_FILENAME)) {
        let _ = data_root::remove_compat_guard(&cr, &op_id);
        let _ = data_root::remove_state(&cr);
        return Err(e);
    }
    set_init_context(&app, Some(DataRootInitCtx { op_id, target }));
    clear_setup_mode(&app);
    Ok(())
}

/// 继续未完成的初始化：丢弃未确认的临时产物（恢复词必须重新生成）→ 回到密码步骤。
#[tauri::command]
pub fn setup_resume_custom_init(app: AppHandle) -> Result<(), String> {
    let cr = config_root(&app)?;
    let (op_id, target) = match data_root::load_state(&cr) {
        StateLoad::Loaded(s) => match s.pending_operation {
            Some(op) if op.op_type == "init" => {
                if op.phase != "preparing" && op.phase != "recovery_presented" {
                    return Err("当前状态不允许重新开始初始化".to_string());
                }
                let target = op.target.clone().ok_or("初始化记录缺少目标路径")?;
                (op.op_id, target)
            }
            _ => return Err("没有待完成的初始化".to_string()),
        },
        _ => {
            // orphan guard 情形：op_id/target 来自 guard 内容
            let cand =
                data_root::orphan_init_candidate(&cr).ok_or("没有待完成的初始化".to_string())?;
            let target = cand.target.ok_or("保护标记缺少目标路径")?;
            (cand.op_id, target)
        }
    };
    // 硬保护：target 已有正式库时绝不走清理路径（重启即可由 setup 收尾）
    if data_root::has_formal_db(&target) {
        return Err("该数据位置已存在正式数据库，请重启抽屉柜以完成收尾".to_string());
    }
    data_root::cleanup_init_artifacts(&target)?;
    data_root::save_state(
        &cr,
        &data_root::state_pending("init", &op_id, "preparing", &target),
    )?;
    build_fresh_app_state(&app, target.join(migration::V2_DB_FILENAME))?;
    set_init_context(&app, Some(DataRootInitCtx { op_id, target }));
    clear_setup_mode(&app);
    Ok(())
}

/// 使用已有数据目录：完整验证 → guard durable → state(attach_existing) → 重启提交。
#[tauri::command]
pub fn setup_attach_existing(app: AppHandle, target: String) -> Result<(), String> {
    let cr = config_root(&app)?;
    let target = PathBuf::from(&target);
    data_root::validate_existing_root(&cr, &target).map_err(|e| e.message())?;
    let op_id = uuid::Uuid::new_v4().to_string();
    let guard_path = data_root::write_compat_guard_g2(&cr, &op_id, &target)?;
    let state = data_root::state_pending("attach_existing", &op_id, "pending_restart", &target);
    if let Err(e) = data_root::save_state(&cr, &state) {
        let _ = std::fs::remove_file(&guard_path);
        return Err(e);
    }
    // 重启后由 setup 再次验证并提交 active_root；提交完成前不开放业务 UI
    crate::commands::restart_app(app)
}

/// 放弃未完成操作：白名单清理本次 init 产物 + 删 guard + 删 state → 重启回选择页。
#[tauri::command]
pub fn setup_abandon_pending(app: AppHandle) -> Result<(), String> {
    let cr = config_root(&app)?;
    match data_root::load_state(&cr) {
        StateLoad::Loaded(s) => {
            let op = s.pending_operation.ok_or("没有待取消的操作")?;
            if op.op_type == "init" {
                if let Some(t) = &op.target {
                    data_root::cleanup_init_artifacts(t)?;
                }
            }
            data_root::remove_compat_guard(&cr, &op.op_id)?;
            data_root::remove_state(&cr)?;
        }
        _ => {
            let cand = data_root::orphan_init_candidate(&cr).ok_or("没有待取消的操作")?;
            if let Some(t) = &cand.target {
                data_root::cleanup_init_artifacts(t)?;
            }
            data_root::remove_compat_guard(&cr, &cand.op_id)?;
        }
    }
    crate::commands::restart_app(app)
}

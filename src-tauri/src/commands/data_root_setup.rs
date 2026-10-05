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

use tauri::{AppHandle, Manager, State};

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

/// Phase 2C-3：数据存储位置迁移（C→D / D→E）。
/// 流程（2C-0.2 定稿）：exclusive gate → guard first → state(transferring) →
/// Online Backup 快照 → 验证+指纹 → target_verified → 激活（无 REPLACE）→
/// restart_required + 冻结闩 → 重启 → 新进程 setup 内完成 retirement/guard 切换/提交。
#[tauri::command]
pub fn setup_begin_migration(
    app: AppHandle,
    state: State<AppState>,
    target: String,
) -> Result<(), String> {
    let cr = config_root(&app)?;
    let target = PathBuf::from(&target);

    // 1) 仅 v2 安全模型（legacy 拒绝，§九）
    if state.security_model() != crate::SecurityModel::StableDekV2 {
        return Err("仅 v2 安全模型支持数据位置迁移（legacy 数据请先完成安全升级）".to_string());
    }
    // 2) source 判定（显式 SourceKind，不靠猜）
    let loaded = match data_root::load_state(&cr) {
        StateLoad::Loaded(s) => s,
        _ => data_root::DataRootState::new(None),
    };
    let (source, kind) = match &loaded.active_root {
        None => (cr.clone(), data_root::SourceKind::DefaultConfigRoot),
        Some(p) => (p.clone(), data_root::SourceKind::External),
    };
    // 3) 本阶段不支持迁回默认位置（§三十三）
    if kind == data_root::SourceKind::External {
        let cr_norm = cr.to_string_lossy().replace('/', "\\").to_lowercase();
        let t_norm = target.to_string_lossy().replace('/', "\\").to_lowercase();
        if t_norm == cr_norm {
            return Err("当前版本暂不支持直接迁回默认位置。".to_string());
        }
    }
    let source_db = source.join(migration::V2_DB_FILENAME);
    if !source_db.exists() {
        return Err("当前数据位置未找到正式数据库，无法迁移".to_string());
    }
    // 4) preflight（含基于实际占用的空间要求，§八）
    let mut required = std::fs::metadata(&source_db).map(|m| m.len()).unwrap_or(0);
    required += std::fs::metadata(source.join(format!("{}-wal", migration::V2_DB_FILENAME)))
        .map(|m| m.len())
        .unwrap_or(0);
    required += 64 * 1024 * 1024; // 安全余量
    data_root::preflight_migration_target(&source, &target, required).map_err(|e| e.message())?;

    // 5) exclusive gate（拿不到立即失败，§十一）
    let _gate = state.data_write()?;
    let op_id = uuid::Uuid::new_v4().to_string();

    // 6) Guard First（§十二/§十三）
    match kind {
        data_root::SourceKind::DefaultConfigRoot => {
            data_root::write_compat_guard_g1(&cr, &op_id)?;
        }
        data_root::SourceKind::External => {
            let ok = data_root::recognize_guards(&cr).iter().any(|g| {
                g.kind == data_root::GuardKind::G2Stray
                    && g.target.as_deref() == Some(source.as_path())
            });
            if !ok {
                return Err(
                    "Config Root 缺少与当前数据位置一致的兼容保护标记（fail closed）".to_string(),
                );
            }
        }
    }

    // 7) source 语义指纹（exclusive 持有期间 = 冻结后的真实状态）
    let source_fp = {
        let db = state.db.lock().map_err(|_| "数据库不可用".to_string())?;
        let conn = db.conn.lock().map_err(|_| "数据库连接不可用".to_string())?;
        data_root::semantic_fingerprint(&conn)?
    };

    // 8) state(transferring)：active_root 保持 source（2C-4：retained 登记随行透传）
    let mut pending_state = data_root::state_pending_migration(
        &source,
        &target,
        &op_id,
        "transferring",
        kind,
        Some(&source_fp),
        None,
        loaded.last_migration.clone(),
    );
    pending_state.retained_sources = loaded.retained_sources.clone();
    data_root::save_state(&cr, &pending_state)?;

    // 9) 快照 → 验证 → 激活（任一失败：回滚到 source，§二十八 A）
    let run = (|| -> Result<(), String> {
        let tmp = data_root::migration_tmp_path(&target, &op_id);
        {
            let db = state.db.lock().map_err(|_| "数据库不可用".to_string())?;
            let conn = db.conn.lock().map_err(|_| "数据库连接不可用".to_string())?;
            data_root::snapshot_source_db(&conn, &tmp)?;
        }
        let target_fp = data_root::verify_migration_target(&tmp, &source_fp)?;
        let st = match data_root::load_state(&cr) {
            StateLoad::Loaded(s) => s,
            _ => return Err("迁移过程中 state 丢失".to_string()),
        };
        let mut verified_state = data_root::state_pending_migration(
            &source,
            &target,
            &op_id,
            "target_verified",
            kind,
            Some(&source_fp),
            Some(&target_fp),
            loaded.last_migration.clone(),
        );
        verified_state.retained_sources = st.retained_sources.clone();
        data_root::save_state(&cr, &verified_state)?;
        data_root::activate_target_tmp(&tmp, &target.join(migration::V2_DB_FILENAME))?;
        let st2 = data_root::state_with_migration_phase(&verified_state, "target_activated");
        data_root::save_state(&cr, &st2)?;
        Ok(())
    })();
    if let Err(e) = run {
        let op = match data_root::load_state(&cr) {
            StateLoad::Loaded(s) => s.pending_operation,
            _ => None,
        };
        match op {
            Some(op) => match data_root::rollback_migration(&cr, &op) {
                Ok(()) => {
                    let restored = data_root::DataRootState {
                        pending_operation: None,
                        ..data_root::state_pending_migration(
                            &source,
                            &target,
                            &op_id,
                            "rolled_back",
                            kind,
                            None,
                            None,
                            None,
                        )
                    };
                    data_root::save_state(&cr, &restored)?;
                    return Err(format!("{}（已自动回滚，原数据未受影响）", e));
                }
                Err(ge) => {
                    // 回滚失败 → 保护状态，不得恢复普通模式
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

    // 10) restart_required + 冻结闩 + 重启（gate 持有至进程退出）
    let st = match data_root::load_state(&cr) {
        StateLoad::Loaded(s) => s,
        _ => return Err("迁移收尾读取 state 失败".to_string()),
    };
    data_root::save_state(
        &cr,
        &data_root::state_with_migration_phase(&st, "restart_required"),
    )?;
    state.freeze_data_operations();
    crate::commands::restart_app(app)
}

/// 数据位置摘要（设置页展示：canonical root + last_migration + retained sources）
#[tauri::command]
pub fn get_data_root_summary(app: AppHandle) -> serde_json::Value {
    let cr = config_root(&app).unwrap_or_default();
    let loaded = data_root::load_state(&cr);
    match loaded {
        StateLoad::Loaded(s) => {
            // Offline Safety Gate：available 为运行时派生值，不持久化
            let retained: Vec<serde_json::Value> = s
                .retained_sources
                .iter()
                .map(|r| {
                    serde_json::json!({
                        "op_id": r.op_id,
                        "archive_path": r.archive_path,
                        "original_root": r.original_root,
                        "migrated_to": r.migrated_to,
                        "created_at": r.created_at,
                        "status": r.status,
                        "deleted_at": r.deleted_at,
                        "available": r.archive_path.exists(),
                    })
                })
                .collect();
            serde_json::json!({
                "config_root": cr.to_string_lossy(),
                "active_root": s.active_root.as_ref().map(|p| p.to_string_lossy().to_string()),
                "last_migration": s.last_migration,
                "retained_sources": retained,
            })
        }
        _ => serde_json::json!({
            "config_root": cr.to_string_lossy(),
            "active_root": null,
            "last_migration": null,
            "retained_sources": [],
        }),
    }
}

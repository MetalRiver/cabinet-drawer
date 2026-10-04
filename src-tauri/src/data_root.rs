//! Phase 2C-1：Data Root 基础设施
//!
//! 职责（严格 scope）：
//! - `data-root-state.json` 单一权威状态（原子写 + last-known-good `.bak` + 损坏 fail closed）
//! - `DataRootResolver`：在任何 SQLite 打开之前解析 effective data root
//! - Compatibility Guard（G1 tmp / G2 stray）的识别与一致性检查（2C-1 不写正式 guard）
//!
//! 硬规则（Phase 2C-0.2 定稿）：
//! - 未知 state.version / 未知 pending operation type → Blocked，绝不 fallback FreshV2
//! - state 缺失但存在 recognized guard → Blocked，绝不自动删除 guard
//! - state 说 external 但 guard 缺失 → Blocked（StateGuardMismatch）
//!
//! 2C-3 未来约束登记（TARGET_ACTIVATION）：
//!   target\drawer-v2.db 已存在时默认 FAIL CLOSED。禁止直接 MOVEFILE_REPLACE_EXISTING
//!   覆盖未知正式库；只有现存 target 文件能通过 op_id / state / artifact fingerprint
//!   明确证明是本次 operation 自己的产物，才允许恢复性操作。本文件的 REPLACE_EXISTING
//!   仅限 state 文件自身（同一 authority 的替换），不得套用到用户数据库。

use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::migration::{LEGACY_DB_FILENAME, V2_DB_FILENAME, V2_TMP_FILENAME};

/// state 文件固定放在 Config Root，不随 Data Root 移动
pub const STATE_FILENAME: &str = "data-root-state.json";
pub const BAK_FILENAME: &str = "data-root-state.bak";
/// 当前 schema 版本。遇到更高版本必须 Blocked（向前兼容 fail closed）。
pub const STATE_VERSION: u32 = 1;

/// Guard Magic Header（G1/G2 通用首行）。旧版只认文件名，不看内容；
/// 新版靠内容区分「Data Root compatibility guard」与真实迁移/升级 tmp。
pub const GUARD_MAGIC: &str = "DRAWER_DATA_ROOT_GUARD_V1";
pub const GUARD_VERSION: &str = "1";
pub const GUARD_TYPE: &str = "data_root_compatibility";
/// G2 stray 文件名前缀：旧版 resolver 的 stray 检测前缀是 `drawer_box.db.stray-`（:141），
/// 我们沿用该前缀保证旧版可识别，后缀带 op_id 便于审计。
pub const GUARD_G2_PREFIX: &str = "drawer_box.db.stray-data-root-";

/// 2C-1 已知 pending operation 类型集合（versioned + extensible）。
/// 未来新增类型（如 restore_backup_to_root）→ 旧版遇到一律 Blocked，绝不忽略后继续启动。
const KNOWN_OPERATION_TYPES: &[&str] = &["migration", "init", "attach_existing", "restore_default"];

// ============================================================
// 状态 Schema（唯一权威）
// ============================================================

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PendingOperation {
    /// versioned + extensible：未知 type 一律 Blocked（见 KNOWN_OPERATION_TYPES）
    pub op_type: String,
    pub op_id: String,
    pub phase: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<PathBuf>,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DataRootState {
    pub version: u32,
    /// null = 默认 Config Root；否则为 external Data Root 绝对路径
    pub active_root: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pending_operation: Option<PendingOperation>,
    pub updated_at: String,
}

impl DataRootState {
    pub fn new(active_root: Option<PathBuf>) -> Self {
        DataRootState {
            version: STATE_VERSION,
            active_root,
            pending_operation: None,
            updated_at: chrono::Local::now().to_rfc3339(),
        }
    }

    fn validate(&self) -> Result<(), BlockedReason> {
        if self.version != STATE_VERSION {
            return Err(BlockedReason::StateVersionUnsupported(self.version));
        }
        if let Some(op) = &self.pending_operation {
            if !KNOWN_OPERATION_TYPES.contains(&op.op_type.as_str()) {
                return Err(BlockedReason::UnknownPendingOperation(op.op_type.clone()));
            }
        }
        Ok(())
    }
}

// ============================================================
// Blocked 原因（结构化；前端负责映射用户文案）
// ============================================================

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(tag = "reason", content = "detail", rename_all = "snake_case")]
pub enum BlockedReason {
    StateCorrupted(String),
    StateVersionUnsupported(u32),
    UnknownPendingOperation(String),
    ExternalRootMissing(String),
    ExternalRootUnreadable(String),
    ExternalDbMissing(String),
    ExternalDbInvalid(String),
    OrphanCompatibilityGuard(String),
    StateGuardMismatch(String),
    PendingOperationNeedsRecovery(String),
}

// ============================================================
// Resolver 输出
// ============================================================

#[derive(Debug, Clone, PartialEq)]
pub enum Resolution {
    /// effective data root = Config Root（现状默认，老用户零行为漂移）
    UseDefault(PathBuf),
    /// effective data root = external root（state.active_root）
    UseExternal(PathBuf),
    /// 存在 pending operation，需要恢复流程（2C-3 实现；2C-1 调用方映射为 Blocked）
    Pending(Box<PendingOperation>),
    Blocked(BlockedReason),
}

// ============================================================
// Guard 识别
// ============================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuardKind {
    /// `drawer-v2.db.tmp` + Magic Header（用于 Config Root 仍有正式 v2 的迁移 pending 窗口）
    G1Tmp,
    /// `drawer_box.db.stray-data-root-<op_id>`（用于 Config Root 无正式库的稳定外置态）
    G2Stray,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GuardInfo {
    pub kind: GuardKind,
    pub op_id: String,
    pub target: Option<PathBuf>,
    pub path: PathBuf,
}

/// 解析 guard 文件内容（首行 Magic，其余 key=value）。解析失败 = 不是 Data Root guard
/// （可能是真实迁移 tmp / 真实 stray），返回 None，交由既有仲裁逻辑处理。
fn parse_guard_file(path: &Path) -> Option<GuardInfo> {
    let text = fs::read_to_string(path).ok()?;
    let mut lines = text.lines();
    if lines.next()?.trim() != GUARD_MAGIC {
        return None;
    }
    let mut guard_version = None;
    let mut guard_type = None;
    let mut op_id = None;
    let mut target = None;
    for line in lines {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let (key, value) = match line.split_once('=') {
            Some(kv) => kv,
            None => continue,
        };
        match key.trim() {
            "guard_version" => guard_version = Some(value.trim().to_string()),
            "guard_type" => guard_type = Some(value.trim().to_string()),
            "op_id" => op_id = Some(value.trim().to_string()),
            "target" => target = Some(PathBuf::from(value.trim())),
            _ => {}
        }
    }
    if guard_version.as_deref() != Some(GUARD_VERSION)
        || guard_type.as_deref() != Some(GUARD_TYPE)
        || op_id.is_none()
    {
        return None;
    }
    Some(GuardInfo {
        op_id: op_id.unwrap(),
        target,
        path: path.to_path_buf(),
        kind: GuardKind::G1Tmp, // 由调用方按文件路径覆盖
    })
}

/// 扫描 Config Root，返回**可识别的** Data Root compatibility guard（G1/G2）。
/// 非 guard 形态的 tmp/stray（无 Magic）不在此列——它们是既有仲裁的输入，行为不变。
fn recognize_guards(config_root: &Path) -> Vec<GuardInfo> {
    let mut guards = Vec::new();
    // G1：固定文件名 drawer-v2.db.tmp + Magic
    let g1_path = config_root.join(V2_TMP_FILENAME);
    if g1_path.exists() {
        if let Some(mut guard) = parse_guard_file(&g1_path) {
            guard.kind = GuardKind::G1Tmp;
            guards.push(guard);
        }
    }
    // G2：drawer_box.db.stray-data-root-<op_id>（旧版按文件名前缀识别为 stray）
    if let Ok(entries) = fs::read_dir(config_root) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with(GUARD_G2_PREFIX) {
                if let Some(mut guard) = parse_guard_file(&entry.path()) {
                    guard.kind = GuardKind::G2Stray;
                    guards.push(guard);
                }
            }
        }
    }
    guards
}

/// 期望的 guard 类型：Config Root 还有正式 v2 → 旧版会先命中 v2 分支，
/// 必须用 G1 tmp（Blocked-1 优先级最高）阻断；否则 G2 stray 足够（Blocked-3）。
fn expected_guard_kind(config_root: &Path) -> GuardKind {
    if config_root.join(V2_DB_FILENAME).exists() {
        GuardKind::G1Tmp
    } else {
        GuardKind::G2Stray
    }
}

// ============================================================
// State 持久化（原子写 + last-known-good）
// ============================================================

#[derive(Debug)]
pub enum StateLoad {
    Loaded(DataRootState),
    Absent,
    Corrupted(String),
}

/// 只做 JSON 语法解析。语义校验（version / 未知 operation type）由 resolver 的
/// `state.validate()` 负责——否则具体原因会被 Corrupted 吞掉，前端拿不到结构化错误。
fn parse_state_bytes(text: &str) -> Result<DataRootState, String> {
    serde_json::from_str(text).map_err(|e| format!("state JSON 解析失败: {}", e))
}

/// 加载顺序：main → bak。两者都存在但 main 损坏 → bak 恢复；
/// 两者都不可解析 → Corrupted（调用方 Blocked，禁止猜测）。
/// 形如 `data-root-state.json.new-*` 的中断临时文件一律忽略（非权威）。
pub fn load_state(config_root: &Path) -> StateLoad {
    let main_path = config_root.join(STATE_FILENAME);
    let bak_path = config_root.join(BAK_FILENAME);
    let main_text = fs::read_to_string(&main_path);
    match main_text {
        Ok(text) => match parse_state_bytes(&text) {
            Ok(state) => StateLoad::Loaded(state),
            Err(main_err) => load_from_bak(&bak_path, main_err),
        },
        Err(main_err) => {
            if !main_path.exists() {
                // main 不存在 → 试 bak（bak-only 也算恢复成功）
                if bak_path.exists() {
                    return match fs::read_to_string(&bak_path) {
                        Ok(text) => match parse_state_bytes(&text) {
                            Ok(state) => StateLoad::Loaded(state),
                            Err(bak_err) => {
                                StateLoad::Corrupted(format!("main 不存在且 bak 损坏: {}", bak_err))
                            }
                        },
                        Err(e) => StateLoad::Corrupted(format!("main 不存在且 bak 不可读: {}", e)),
                    };
                }
                return StateLoad::Absent;
            }
            StateLoad::Corrupted(format!("main 不可读: {}", main_err))
        }
    }
}

fn load_from_bak(bak_path: &Path, main_err: String) -> StateLoad {
    match fs::read_to_string(bak_path) {
        Ok(text) => match parse_state_bytes(&text) {
            Ok(state) => StateLoad::Loaded(state),
            Err(bak_err) => StateLoad::Corrupted(format!("main: {}; bak: {}", main_err, bak_err)),
        },
        Err(e) => StateLoad::Corrupted(format!("main: {}; bak 不可读: {}", main_err, e)),
    }
}

/// Windows 原子替换：MoveFileExW(REPLACE_EXISTING | WRITE_THROUGH)。
/// WRITE_THROUGH 保证 rename 元数据落盘后才返回（Windows 无 portable 目录 fsync 的补偿）。
/// 【scope 警告】REPLACE_EXISTING 仅允许用于 state 文件自身；
/// 未来 2C-3 的 TARGET_ACTIVATE（用户数据库）见本文件头部硬约束——默认 FAIL CLOSED。
#[cfg(windows)]
fn atomic_replace(temp: &Path, dest: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::GetLastError;
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };
    let to_wide = |p: &Path| -> Vec<u16> {
        p.as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    };
    let temp_w = to_wide(temp);
    let dest_w = to_wide(dest);
    let ok = unsafe {
        MoveFileExW(
            temp_w.as_ptr(),
            dest_w.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if ok == 0 {
        let code = unsafe { GetLastError() };
        return Err(format!("MoveFileExW 失败 (os error {})", code));
    }
    Ok(())
}

#[cfg(not(windows))]
fn atomic_replace(temp: &Path, dest: &Path) -> Result<(), String> {
    fs::rename(temp, dest).map_err(|e| format!("rename 失败: {}", e))
}

/// 原子保存：serialize → 同目录 temp（create_new）→ write_all → sync_all → 关句柄
/// → 备份 last-known-good 到 .bak → MoveFileExW 原子替换正式文件。
/// bak 更新失败不得影响正式 state（只记日志，继续替换）。
pub fn save_state(config_root: &Path, state: &DataRootState) -> Result<(), String> {
    let json = serde_json::to_string_pretty(state).map_err(|e| format!("序列化失败: {}", e))?;
    let main_path = config_root.join(STATE_FILENAME);
    let bak_path = config_root.join(BAK_FILENAME);
    let temp_path = config_root.join(format!("{}.new-{}", STATE_FILENAME, uuid::Uuid::new_v4()));

    // 1. 写 temp（create_new 确保不覆盖任何已有文件）
    {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp_path)
            .map_err(|e| format!("创建 temp 失败: {}", e))?;
        file.write_all(json.as_bytes())
            .map_err(|e| format!("写 temp 失败: {}", e))?;
        file.sync_all()
            .map_err(|e| format!("sync temp 失败: {}", e))?;
    }

    // 2. 保留 last-known-good（仅当正式文件当前存在）
    if main_path.exists() {
        // bak 失败不阻断主流程：主文件替换本身是原子的，只是少一层恢复保险
        if let Err(e) = fs::copy(&main_path, &bak_path) {
            eprintln!("[data-root] bak 更新失败（不阻断）: {}", e);
        }
    }

    // 3. 原子替换
    if let Err(e) = atomic_replace(&temp_path, &main_path) {
        let _ = fs::remove_file(&temp_path);
        return Err(e);
    }
    Ok(())
}

/// 删除 state（main + bak）。仅供未来 restore-default / 显式重置流程使用，2C-1 不调用。
#[allow(dead_code)]
pub fn remove_state(config_root: &Path) -> Result<(), String> {
    for name in [STATE_FILENAME, BAK_FILENAME] {
        let path = config_root.join(name);
        if path.exists() {
            fs::remove_file(&path).map_err(|e| format!("删除 {} 失败: {}", name, e))?;
        }
    }
    Ok(())
}

// ============================================================
// DataRootResolver
// ============================================================

/// 在任何 SQLite 打开之前调用。
pub fn resolve_data_root(config_root: &Path) -> Resolution {
    let guards = recognize_guards(config_root);
    match load_state(config_root) {
        StateLoad::Absent => {
            // state 缺失 + recognized guard 存在 → 绝不自动删 guard、绝不默认启动
            // （外部 Data Root 可能真实存在，只是 state 损坏/丢失；删 guard = 重开旧版 FreshV2 风险）
            if let Some(guard) = guards.first() {
                return Resolution::Blocked(BlockedReason::OrphanCompatibilityGuard(format!(
                    "检测到 {:?} guard（op_id={}）但状态文件缺失",
                    guard.kind, guard.op_id
                )));
            }
            Resolution::UseDefault(config_root.to_path_buf())
        }
        StateLoad::Corrupted(detail) => Resolution::Blocked(BlockedReason::StateCorrupted(detail)),
        StateLoad::Loaded(state) => {
            if let Err(reason) = state.validate() {
                return Resolution::Blocked(reason);
            }
            if let Some(op) = state.pending_operation {
                return Resolution::Pending(Box::new(op));
            }
            match state.active_root {
                None => {
                    // 默认位置：出现 Data Root guard 即状态矛盾（谁写的？）→ Blocked，不自动删
                    if let Some(guard) = guards.first() {
                        return Resolution::Blocked(BlockedReason::OrphanCompatibilityGuard(
                            format!(
                                "active_root 为默认位置却存在 {:?} guard（op_id={}）",
                                guard.kind, guard.op_id
                            ),
                        ));
                    }
                    Resolution::UseDefault(config_root.to_path_buf())
                }
                Some(external) => {
                    // external 期望 guard = 按 Config Root 是否仍有正式 v2 决定（G1/G2）
                    let expected = expected_guard_kind(config_root);
                    let found = guards.iter().find(|g| g.kind == expected);
                    let guard = match found {
                        Some(g) => g,
                        None => {
                            return Resolution::Blocked(BlockedReason::StateGuardMismatch(
                                format!("external root 已激活但缺少期望的 {:?} guard", expected),
                            ));
                        }
                    };
                    // external root 文件级检查（DB 有效性交给既有 open 路径）
                    if !external.exists() {
                        return Resolution::Blocked(BlockedReason::ExternalRootMissing(
                            external.to_string_lossy().to_string(),
                        ));
                    }
                    if fs::read_dir(&external).is_err() {
                        return Resolution::Blocked(BlockedReason::ExternalRootUnreadable(
                            external.to_string_lossy().to_string(),
                        ));
                    }
                    let has_db = external.join(V2_DB_FILENAME).exists()
                        || external.join(LEGACY_DB_FILENAME).exists();
                    if !has_db {
                        return Resolution::Blocked(BlockedReason::ExternalDbMissing(
                            external.to_string_lossy().to_string(),
                        ));
                    }
                    let _ = guard; // op_id/target 一致性细查留给 2C-3 恢复流程
                    Resolution::UseExternal(external)
                }
            }
        }
    }
}

// ============================================================
// Blocked 模式（2C-1 fail-visible 基础设施）
// ============================================================

/// Blocked 模式下由 lib.rs setup 管理；DB 永不打开，托盘/快捷键不创建。
pub struct BlockedState {
    pub reason: BlockedReason,
    pub config_root: PathBuf,
}

#[tauri::command]
pub fn get_data_root_block(app: tauri::AppHandle) -> Option<serde_json::Value> {
    use tauri::Manager;
    let state = app.try_state::<BlockedState>()?;
    Some(serde_json::json!({
        "reason": state.reason,
        "config_root": state.config_root.to_string_lossy(),
    }))
}

// ============================================================
// 测试辅助（2C-1 不在生产路径写 guard；此处仅供单测构造文件）
// ============================================================

#[cfg(test)]
pub(crate) fn write_guard_file(
    config_root: &Path,
    kind: GuardKind,
    op_id: &str,
    target: Option<&Path>,
) -> PathBuf {
    let path = match kind {
        GuardKind::G1Tmp => config_root.join(V2_TMP_FILENAME),
        GuardKind::G2Stray => config_root.join(format!("{}{}", GUARD_G2_PREFIX, op_id)),
    };
    let mut text = format!(
        "{}\nguard_version={}\nguard_type={}\nop_id={}\n",
        GUARD_MAGIC, GUARD_VERSION, GUARD_TYPE, op_id
    );
    if let Some(t) = target {
        text.push_str(&format!("target={}\n", t.to_string_lossy()));
    }
    fs::write(&path, text).expect("写 guard 失败");
    path
}

// ============================================================
// 单元测试（Phase 2C-1 §十六 20 项中的可自动化子集）
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(label: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("drawer-dr-{}-{}", label, uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).expect("建临时目录失败");
        dir
    }

    fn put_state(config_root: &Path, state: &DataRootState) {
        save_state(config_root, state).expect("save_state 失败");
    }

    fn default_state() -> DataRootState {
        DataRootState::new(None)
    }

    // 1. 无 state → default root
    #[test]
    fn t01_no_state_yields_default() {
        let root = temp_root("t01");
        assert_eq!(
            resolve_data_root(&root),
            Resolution::UseDefault(root.clone())
        );
    }

    // 2. 合法 state active_root=null → default
    #[test]
    fn t02_state_default_yields_default() {
        let root = temp_root("t02");
        put_state(&root, &default_state());
        assert_eq!(
            resolve_data_root(&root),
            Resolution::UseDefault(root.clone())
        );
    }

    // 3. 合法 external root state → UseExternal
    #[test]
    fn t03_external_state_yields_external() {
        let root = temp_root("t03");
        let external = root.join("ext-data");
        fs::create_dir_all(&external).expect("建外部目录失败");
        fs::write(external.join(V2_DB_FILENAME), b"placeholder").expect("写 db 失败");
        write_guard_file(&root, GuardKind::G2Stray, "op-3", Some(&external));
        let mut state = DataRootState::new(Some(external.clone()));
        state.updated_at = chrono::Local::now().to_rfc3339();
        put_state(&root, &state);
        assert_eq!(resolve_data_root(&root), Resolution::UseExternal(external));
    }

    // 4. main truncated → bak valid → recover
    #[test]
    fn t04_main_corrupt_bak_recovers() {
        let root = temp_root("t04");
        let mut s1 = default_state();
        put_state(&root, &s1);
        s1.updated_at = chrono::Local::now().to_rfc3339();
        put_state(&root, &s1); // 第二次保存 → bak = 第一次的合法内容
        fs::write(root.join(STATE_FILENAME), "{\"version\":1,\"activ").expect("截断写入失败");
        match load_state(&root) {
            StateLoad::Loaded(state) => assert_eq!(state.version, STATE_VERSION),
            other => panic!("期望 bak 恢复，实际 {:?}", other),
        }
    }

    // 5. state + bak 都损坏 → Blocked
    #[test]
    fn t05_both_corrupt_blocked() {
        let root = temp_root("t05");
        fs::write(root.join(STATE_FILENAME), "not json").expect("写失败");
        fs::write(root.join(BAK_FILENAME), "also not json").expect("写失败");
        match resolve_data_root(&root) {
            Resolution::Blocked(BlockedReason::StateCorrupted(_)) => {}
            other => panic!("期望 StateCorrupted，实际 {:?}", other),
        }
    }

    // 6. unsupported state version → Blocked
    #[test]
    fn t06_unsupported_version_blocked() {
        let root = temp_root("t06");
        let mut state = default_state();
        state.version = 99;
        put_state(&root, &state);
        match resolve_data_root(&root) {
            Resolution::Blocked(BlockedReason::StateVersionUnsupported(99)) => {}
            other => panic!("期望 StateVersionUnsupported(99)，实际 {:?}", other),
        }
    }

    // 7. unknown pending operation → Blocked
    #[test]
    fn t07_unknown_pending_operation_blocked() {
        let root = temp_root("t07");
        let mut state = default_state();
        state.pending_operation = Some(PendingOperation {
            op_type: "hyper_migrate".to_string(),
            op_id: "op-7".to_string(),
            phase: "transferring".to_string(),
            source: None,
            target: None,
            updated_at: chrono::Local::now().to_rfc3339(),
        });
        put_state(&root, &state);
        match resolve_data_root(&root) {
            Resolution::Blocked(BlockedReason::UnknownPendingOperation(t)) => {
                assert_eq!(t, "hyper_migrate");
            }
            other => panic!("期望 UnknownPendingOperation，实际 {:?}", other),
        }
    }

    // 8. external path missing → Blocked
    #[test]
    fn t08_external_missing_blocked() {
        let root = temp_root("t08");
        let external = root.join("不存在的盘").join("DrawerData");
        write_guard_file(&root, GuardKind::G2Stray, "op-8", Some(&external));
        let mut state = DataRootState::new(Some(external.clone()));
        state.updated_at = chrono::Local::now().to_rfc3339();
        put_state(&root, &state);
        match resolve_data_root(&root) {
            Resolution::Blocked(BlockedReason::ExternalRootMissing(p)) => {
                assert!(p.contains("不存在的盘"));
            }
            other => panic!("期望 ExternalRootMissing，实际 {:?}", other),
        }
    }

    // 9. external path exists but DB missing → Blocked
    #[test]
    fn t09_external_db_missing_blocked() {
        let root = temp_root("t09");
        let external = root.join("empty-ext");
        fs::create_dir_all(&external).expect("建目录失败");
        write_guard_file(&root, GuardKind::G2Stray, "op-9", Some(&external));
        let mut state = DataRootState::new(Some(external));
        state.updated_at = chrono::Local::now().to_rfc3339();
        put_state(&root, &state);
        match resolve_data_root(&root) {
            Resolution::Blocked(BlockedReason::ExternalDbMissing(_)) => {}
            other => panic!("期望 ExternalDbMissing，实际 {:?}", other),
        }
    }

    // 10. recognized G1 guard + state missing → Blocked
    #[test]
    fn t10_g1_guard_state_missing_blocked() {
        let root = temp_root("t10");
        write_guard_file(&root, GuardKind::G1Tmp, "op-10", None);
        match resolve_data_root(&root) {
            Resolution::Blocked(BlockedReason::OrphanCompatibilityGuard(_)) => {}
            other => panic!("期望 OrphanCompatibilityGuard，实际 {:?}", other),
        }
        assert!(root.join(V2_TMP_FILENAME).exists(), "guard 不得被自动删除");
    }

    // 11. recognized G2 guard + state missing → Blocked（且绝不自动删）
    #[test]
    fn t11_g2_guard_state_missing_blocked() {
        let root = temp_root("t11");
        let guard_path = write_guard_file(&root, GuardKind::G2Stray, "op-11", None);
        match resolve_data_root(&root) {
            Resolution::Blocked(BlockedReason::OrphanCompatibilityGuard(_)) => {}
            other => panic!("期望 OrphanCompatibilityGuard，实际 {:?}", other),
        }
        assert!(guard_path.exists(), "G2 stray 绝不自动删除");
    }

    // 12. state external + expected guard missing → 不得 fallback default/FreshV2
    #[test]
    fn t12_external_guard_missing_no_fallback() {
        let root = temp_root("t12");
        let external = root.join("ext-no-guard");
        fs::create_dir_all(&external).expect("建目录失败");
        fs::write(external.join(V2_DB_FILENAME), b"placeholder").expect("写 db 失败");
        let mut state = DataRootState::new(Some(external));
        state.updated_at = chrono::Local::now().to_rfc3339();
        put_state(&root, &state); // 没写任何 guard
        match resolve_data_root(&root) {
            Resolution::Blocked(BlockedReason::StateGuardMismatch(_)) => {}
            Resolution::UseDefault(_) | Resolution::UseExternal(_) => {
                panic!("禁止 fallback 到 default/external");
            }
            other => panic!("期望 StateGuardMismatch，实际 {:?}", other),
        }
    }

    // 13. state default + unexpected Data Root guard → Blocked，不自动删
    #[test]
    fn t13_default_with_guard_blocked_no_autodelete() {
        let root = temp_root("t13");
        put_state(&root, &default_state());
        let guard_path = write_guard_file(&root, GuardKind::G2Stray, "op-13", None);
        match resolve_data_root(&root) {
            Resolution::Blocked(BlockedReason::OrphanCompatibilityGuard(_)) => {}
            other => panic!("期望 OrphanCompatibilityGuard，实际 {:?}", other),
        }
        assert!(guard_path.exists(), "不得自动删除 guard");
    }

    // 14/15. Unicode + 空格路径
    #[test]
    fn t14_t15_unicode_and_space_paths() {
        for label in ["抽屉柜数据 2C-1", "My Drawer Data (test)"] {
            let root = temp_root("t14t15");
            let external = root.join(label);
            fs::create_dir_all(&external).expect("Unicode/空格目录创建失败");
            fs::write(external.join(V2_DB_FILENAME), b"placeholder").expect("写失败");
            write_guard_file(&root, GuardKind::G2Stray, "op-1415", Some(&external));
            let mut state = DataRootState::new(Some(external.clone()));
            state.updated_at = chrono::Local::now().to_rfc3339();
            put_state(&root, &state);
            assert_eq!(resolve_data_root(&root), Resolution::UseExternal(external));
            let _ = label;
        }
    }

    // 16. 原子替换 fault injection：old/new 至少一个完整可解析；stale temp 被忽略
    #[test]
    fn t16_atomic_replace_fault_injection() {
        let root = temp_root("t16");
        let s1 = default_state();
        put_state(&root, &s1);
        // 模拟"写 temp 成功但替换前崩溃"：遗留 .new 垃圾，不影响读取
        fs::write(
            root.join(format!("{}.new-stale", STATE_FILENAME)),
            "garbage",
        )
        .expect("写 stale temp 失败");
        match load_state(&root) {
            StateLoad::Loaded(state) => assert_eq!(state, s1),
            other => panic!("stale temp 不得影响权威 state，实际 {:?}", other),
        }
        // 正常第二轮保存 → bak 存在且可解析
        let mut s2 = default_state();
        s2.updated_at = chrono::Local::now().to_rfc3339();
        put_state(&root, &s2);
        let bak_text = fs::read_to_string(root.join(BAK_FILENAME)).expect("读 bak 失败");
        assert!(
            parse_state_bytes(&bak_text).is_ok(),
            "bak 必须是完整可解析版本"
        );
    }

    // 17. bak recovery（bak-only：main 被删）
    #[test]
    fn t17_bak_only_recovery() {
        let root = temp_root("t17");
        let mut s1 = default_state();
        put_state(&root, &s1);
        s1.updated_at = chrono::Local::now().to_rfc3339();
        put_state(&root, &s1);
        fs::remove_file(root.join(STATE_FILENAME)).expect("删 main 失败");
        match load_state(&root) {
            StateLoad::Loaded(state) => assert_eq!(state.version, STATE_VERSION),
            other => panic!("期望 bak-only 恢复，实际 {:?}", other),
        }
    }

    // 18/19. 默认老用户回归：config root 有 v2 / legacy 时 resolver 仍 UseDefault，
    // 且既有仲裁行为不受影响（v2 → ExistingV2 语义由 resolve_startup_db 保持）。
    #[test]
    fn t18_t19_default_v2_and_legacy_untouched() {
        // v2 老用户
        let root = temp_root("t18");
        fs::write(root.join(V2_DB_FILENAME), b"placeholder").expect("写失败");
        assert_eq!(
            resolve_data_root(&root),
            Resolution::UseDefault(root.clone())
        );
        assert!(matches!(
            crate::migration::resolve_startup_db(&root, true).selection,
            crate::migration::DbSelection::V2(_)
        ));
        // legacy 老用户
        let root2 = temp_root("t19");
        fs::write(root2.join(LEGACY_DB_FILENAME), b"placeholder").expect("写失败");
        assert_eq!(
            resolve_data_root(&root2),
            Resolution::UseDefault(root2.clone())
        );
        assert!(matches!(
            crate::migration::resolve_startup_db(&root2, true).selection,
            crate::migration::DbSelection::Legacy(_)
        ));
    }

    // 非 guard 的真实 tmp / stray 不得被 resolver 误判为 guard（行为保持既有仲裁）
    #[test]
    fn t20_genuine_tmp_and_stray_not_mistaken_as_guard() {
        let root = temp_root("t20");
        // 真实迁移 tmp（无 Magic）→ resolver 返回 UseDefault，仲裁层负责 Blocked
        fs::write(root.join(V2_TMP_FILENAME), b"not a guard").expect("写失败");
        assert_eq!(
            resolve_data_root(&root),
            Resolution::UseDefault(root.clone())
        );
        // 真实 stray（无 Magic）→ 同上（旧仲裁 Blocked-3 兜底）
        let root2 = temp_root("t20b");
        fs::write(
            root2.join(format!("{}.stray-legacy-crash", LEGACY_DB_FILENAME)),
            b"not a guard",
        )
        .expect("写失败");
        assert_eq!(
            resolve_data_root(&root2),
            Resolution::UseDefault(root2.clone())
        );
        assert!(matches!(
            crate::migration::resolve_startup_db(&root2, true).selection,
            crate::migration::DbSelection::Blocked(_)
        ));
    }
}

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

use crate::migration::{
    LEGACY_DB_FILENAME, V2_DB_FILENAME, V2_SETUP_LOCK_FILENAME, V2_TMP_FILENAME,
};

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
pub fn recognize_guards(config_root: &Path) -> Vec<GuardInfo> {
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
// Phase 2C-2：目录文件级工具
// ============================================================

/// 旧版归档命名（migration 的 rename 目标）：`drawer-v2.db.migrated-<op_id>`
pub const ARCHIVE_PREFIX: &str = "drawer-v2.db.migrated-";
pub const MIGRATION_TMP_MARKER: &str = ".migration-tmp";

pub fn has_formal_db(dir: &Path) -> bool {
    dir.join(V2_DB_FILENAME).exists()
}

pub fn has_legacy_db(dir: &Path) -> bool {
    dir.join(LEGACY_DB_FILENAME).exists()
}

pub fn has_renamed_archive(dir: &Path) -> bool {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            if entry
                .file_name()
                .to_string_lossy()
                .starts_with(ARCHIVE_PREFIX)
            {
                return true;
            }
        }
    }
    false
}

/// 含未完成初始化/迁移中间态（attach 前必须拒绝）
pub fn has_migration_artifacts(dir: &Path) -> Option<String> {
    let mut names = Vec::new();
    if dir.join(V2_TMP_FILENAME).exists() {
        names.push(V2_TMP_FILENAME.to_string());
    }
    if dir.join(V2_SETUP_LOCK_FILENAME).exists() {
        names.push(V2_SETUP_LOCK_FILENAME.to_string());
    }
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.contains(MIGRATION_TMP_MARKER) {
                names.push(name);
            }
        }
    }
    if names.is_empty() {
        None
    } else {
        Some(names.join(", "))
    }
}

pub fn dir_is_empty(dir: &Path) -> bool {
    match fs::read_dir(dir) {
        Ok(mut entries) => entries.next().is_none(),
        Err(_) => false,
    }
}

/// 可写性真实探测：create_new → write → sync_all → read → delete（全部成功才算可写）
pub fn writable_probe(dir: &Path) -> Result<(), String> {
    const PAYLOAD: &[u8] = b"drawerbox-write-probe";
    let probe = dir.join(format!(".drawerbox-probe-{}", uuid::Uuid::new_v4()));
    {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&probe)
            .map_err(|e| format!("无法创建测试文件: {}", e))?;
        file.write_all(PAYLOAD)
            .map_err(|e| format!("写入测试文件失败: {}", e))?;
        file.sync_all()
            .map_err(|e| format!("sync 测试文件失败: {}", e))?;
    }
    let read_back = fs::read(&probe).map_err(|e| format!("读回测试文件失败: {}", e))?;
    if read_back != PAYLOAD {
        let _ = fs::remove_file(&probe);
        return Err("测试文件内容不一致（磁盘异常）".to_string());
    }
    fs::remove_file(&probe).map_err(|e| format!("删除测试文件失败: {}", e))?;
    Ok(())
}

/// 本地固定磁盘判定（Windows: GetDriveTypeW == DRIVE_FIXED(3)）。
#[cfg(windows)]
pub fn drive_is_fixed(path: &Path) -> bool {
    use windows_sys::Win32::Storage::FileSystem::GetDriveTypeW;
    /// Win32 DRIVE_FIXED（windows-sys 常量在不同版本所在模块不一致，用字面值+注释）
    const DRIVE_FIXED_VALUE: u32 = 3;
    let wide = match drive_root_wide(path) {
        Some(w) => w,
        None => return false,
    };
    let drive_type = unsafe { GetDriveTypeW(wide.as_ptr()) };
    drive_type == DRIVE_FIXED_VALUE
}

#[cfg(not(windows))]
pub fn drive_is_fixed(_path: &Path) -> bool {
    true
}

/// 剩余空间（字节）。Windows 用 GetDiskFreeSpaceExW。
#[cfg(windows)]
pub fn free_space_bytes(path: &Path) -> Option<u64> {
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    let wide = drive_root_wide(path)?;
    let mut free_to_caller = 0u64;
    let mut total = 0u64;
    let mut total_free = 0u64;
    let ok = unsafe {
        GetDiskFreeSpaceExW(
            wide.as_ptr(),
            &mut free_to_caller,
            &mut total,
            &mut total_free,
        )
    };
    if ok == 0 {
        None
    } else {
        Some(free_to_caller)
    }
}

#[cfg(not(windows))]
pub fn free_space_bytes(_path: &Path) -> Option<u64> {
    None
}

/// 取盘根宽字符（如 "D:\"）供 Win32 API 使用
#[cfg(windows)]
fn drive_root_wide(path: &Path) -> Option<Vec<u16>> {
    use std::os::windows::ffi::OsStrExt;
    let mut root = match path.components().next() {
        Some(std::path::Component::Prefix(p)) => p.as_os_str().to_string_lossy().to_string(),
        _ => return None,
    };
    if !root.ends_with('\\') {
        root.push('\\');
    }
    Some(
        std::ffi::OsStr::new(&root)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect(),
    )
}

fn normalized_lower(path: &Path) -> String {
    let mut s = path.to_string_lossy().replace('/', "\\").to_lowercase();
    while s.len() > 3 && s.ends_with('\\') {
        s.pop();
    }
    s
}

fn is_unc_path(path: &Path) -> bool {
    path.to_string_lossy().starts_with("\\\\")
}

fn looks_like_sync_folder(path: &Path) -> bool {
    normalized_lower(path).split('\\').any(|c| {
        c == "onedrive" || c.starts_with("onedrive -") || c == "dropbox" || c == "google drive"
    })
}

fn looks_like_system_dir(path: &Path) -> bool {
    let s = normalized_lower(path);
    s.contains("\\windows\\")
        || s.ends_with("\\windows")
        || s.contains("\\program files")
        || s.contains("\\programdata")
        || s.contains("\\system32")
        || s == "c:\\"
}

/// 新建数据位置的 preflight 失败分类（前端映射文案）
#[derive(Debug, Clone, PartialEq)]
pub enum PreflightError {
    NotAbsolute(String),
    IsConfigRoot(String),
    NestedWithConfigRoot(String),
    SyncFolder(String),
    NetworkOrRemovable(String),
    SystemDirectory(String),
    HasExistingDrawerData(String),
    NotEmpty(String),
    CreateFailed(String),
    NotWritable(String),
    NoSpace(String),
}

impl PreflightError {
    pub fn message(&self) -> String {
        match self {
            PreflightError::NotAbsolute(p) => format!("路径必须是绝对路径：{}", p),
            PreflightError::IsConfigRoot(_) => {
                "不能选择抽屉柜自己的配置目录作为数据位置。".to_string()
            }
            PreflightError::NestedWithConfigRoot(_) => {
                "该位置与抽屉柜配置目录互相嵌套，请选择其他位置。".to_string()
            }
            PreflightError::SyncFolder(_) => {
                "该位置在云同步目录（OneDrive / Dropbox 等）内，可能损坏数据库，请换一个本地磁盘位置。"
                    .to_string()
            }
            PreflightError::NetworkOrRemovable(_) => {
                "请选择本地固定磁盘上的目录（暂不支持移动硬盘、U 盘或网络盘）。".to_string()
            }
            PreflightError::SystemDirectory(_) => "该位置是系统目录，请选择其他位置。".to_string(),
            PreflightError::HasExistingDrawerData(_) => {
                "该位置已包含一套抽屉柜数据。如需继续使用，请改用「使用已有数据目录」。".to_string()
            }
            PreflightError::NotEmpty(_) => {
                "该目录不是空目录。请选择一个空目录，或先新建一个专用子目录（例如 抽屉柜数据）。"
                    .to_string()
            }
            PreflightError::CreateFailed(e) => format!("无法创建目标目录：{}", e),
            PreflightError::NotWritable(e) => format!("该目录不可写：{}", e),
            PreflightError::NoSpace(_) => "该磁盘可用空间不足（至少需要 100 MB）。".to_string(),
        }
    }
}

const MIN_FREE_BYTES: u64 = 100 * 1024 * 1024;

/// 自定义 Data Root（新建）preflight。目录缺失时会创建它（创建前已完成危险路径判定）。
pub fn preflight_new_root(config_root: &Path, target: &Path) -> Result<(), PreflightError> {
    if !target.is_absolute() {
        return Err(PreflightError::NotAbsolute(
            target.to_string_lossy().to_string(),
        ));
    }
    let target_s = normalized_lower(target);
    let config_s = normalized_lower(config_root);
    if target_s == config_s {
        return Err(PreflightError::IsConfigRoot(target_s));
    }
    if target_s.starts_with(&format!("{}\\", config_s))
        || config_s.starts_with(&format!("{}\\", target_s))
    {
        return Err(PreflightError::NestedWithConfigRoot(target_s));
    }
    if is_unc_path(target) {
        return Err(PreflightError::NetworkOrRemovable(target_s));
    }
    if looks_like_sync_folder(target) {
        return Err(PreflightError::SyncFolder(target_s));
    }
    if looks_like_system_dir(target) {
        return Err(PreflightError::SystemDirectory(target_s));
    }
    if !target.exists() {
        fs::create_dir_all(target).map_err(|e| PreflightError::CreateFailed(e.to_string()))?;
    }
    if !target.is_dir() {
        return Err(PreflightError::CreateFailed("目标不是目录".to_string()));
    }
    if !drive_is_fixed(target) {
        return Err(PreflightError::NetworkOrRemovable(target_s));
    }
    // 已有一套抽屉柜数据 → 禁止 fresh init（转 attach / Blocked）
    if has_formal_db(target) || has_legacy_db(target) || has_renamed_archive(target) {
        return Err(PreflightError::HasExistingDrawerData(target_s));
    }
    if !dir_is_empty(target) {
        return Err(PreflightError::NotEmpty(target_s));
    }
    writable_probe(target).map_err(PreflightError::NotWritable)?;
    if let Some(free) = free_space_bytes(target) {
        if free < MIN_FREE_BYTES {
            return Err(PreflightError::NoSpace(target_s));
        }
    }
    Ok(())
}

/// 「使用已有数据目录」验证失败分类
#[derive(Debug, Clone, PartialEq)]
pub enum AttachError {
    NotFound(String),
    NotAbsolute(String),
    ConfigRootSame(String),
    SyncFolder(String),
    NetworkOrRemovable(String),
    NotWritable(String),
    DbMissing(String),
    LegacyOnly(String),
    MigrationArtifacts(String),
    DbBusy(String),
    DbInvalid(String),
}

impl AttachError {
    pub fn message(&self) -> String {
        match self {
            AttachError::NotFound(p) => format!("目录不存在：{}", p),
            AttachError::NotAbsolute(p) => format!("路径必须是绝对路径：{}", p),
            AttachError::ConfigRootSame(_) => {
                "该目录就是抽屉柜配置目录，请选择数据所在的其他位置。".to_string()
            }
            AttachError::SyncFolder(_) => {
                "该位置在云同步目录内，出于数据库一致性考虑暂不支持。".to_string()
            }
            AttachError::NetworkOrRemovable(_) => {
                "请选择本地固定磁盘上的目录（暂不支持移动硬盘、U 盘或网络盘）。".to_string()
            }
            AttachError::NotWritable(e) => format!("该目录不可写：{}", e),
            AttachError::DbMissing(_) => "该目录内没有找到抽屉柜数据库。".to_string(),
            AttachError::LegacyOnly(_) => {
                "检测到旧版数据格式，当前版本暂不能直接连接该目录。".to_string()
            }
            AttachError::MigrationArtifacts(names) => format!(
                "该目录存在未完成的迁移/初始化痕迹（{}），请先在新版抽屉柜中完成或清理后再连接。",
                names
            ),
            AttachError::DbBusy(_) => "该数据目录正在被其他进程使用。".to_string(),
            AttachError::DbInvalid(e) => format!("该目录中的数据库无效或已损坏：{}", e),
        }
    }
}

/// Windows 独占句柄探测：数据库文件是否正被其他进程打开。
#[cfg(windows)]
fn db_file_locked_by_other(db_path: &Path) -> bool {
    use std::os::windows::fs::OpenOptionsExt;
    fs::OpenOptions::new()
        .read(true)
        .write(true)
        .share_mode(0)
        .open(db_path)
        .is_err()
}

#[cfg(not(windows))]
fn db_file_locked_by_other(_db_path: &Path) -> bool {
    false
}

/// 「使用已有数据目录」完整验证（只读判定 + 无破坏占用探测，绝不修改数据）。
pub fn validate_existing_root(config_root: &Path, target: &Path) -> Result<(), AttachError> {
    if !target.is_absolute() {
        return Err(AttachError::NotAbsolute(
            target.to_string_lossy().to_string(),
        ));
    }
    let target_s = normalized_lower(target);
    if target_s == normalized_lower(config_root) {
        return Err(AttachError::ConfigRootSame(target_s));
    }
    if !target.exists() || !target.is_dir() {
        return Err(AttachError::NotFound(target_s));
    }
    if is_unc_path(target) {
        return Err(AttachError::NetworkOrRemovable(target_s));
    }
    if looks_like_sync_folder(target) {
        return Err(AttachError::SyncFolder(target_s));
    }
    if !drive_is_fixed(target) {
        return Err(AttachError::NetworkOrRemovable(target_s));
    }
    if fs::read_dir(target).is_err() {
        return Err(AttachError::NotFound(target_s));
    }
    writable_probe(target).map_err(AttachError::NotWritable)?;
    if let Some(names) = has_migration_artifacts(target) {
        return Err(AttachError::MigrationArtifacts(names));
    }
    let db_path = target.join(V2_DB_FILENAME);
    if !db_path.exists() {
        if has_legacy_db(target) {
            return Err(AttachError::LegacyOnly(target_s));
        }
        return Err(AttachError::DbMissing(target_s));
    }
    // 其他进程占用 → 明确拒绝（不强接）
    if db_file_locked_by_other(&db_path) {
        return Err(AttachError::DbBusy(target_s));
    }
    // 无 CREATE 打开 + security metadata + integrity + schema（复用既有验证器）
    crate::migration::open_existing_v2_db(&db_path).map_err(AttachError::DbInvalid)?;
    Ok(())
}

// ============================================================
// Phase 2C-2：Guard 写入（生产路径）+ state 构造
// ============================================================

/// 在 Config Root 写入 G2 compatibility guard 并确保 durable。
/// 硬顺序（2C-0.2 §B / 2C-2 §八）：guard durable 之后才允许写 state / 向 target 写数据。
pub fn write_compat_guard_g2(
    config_root: &Path,
    op_id: &str,
    target: &Path,
) -> Result<PathBuf, String> {
    let path = config_root.join(format!("{}{}", GUARD_G2_PREFIX, op_id));
    let text = format!(
        "{}\nguard_version={}\nguard_type={}\nop_id={}\ntarget={}\n",
        GUARD_MAGIC,
        GUARD_VERSION,
        GUARD_TYPE,
        op_id,
        target.to_string_lossy()
    );
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|e| format!("创建 compatibility guard 失败: {}", e))?;
    file.write_all(text.as_bytes())
        .map_err(|e| format!("写 compatibility guard 失败: {}", e))?;
    file.sync_all()
        .map_err(|e| format!("sync compatibility guard 失败: {}", e))?;
    drop(file);
    match parse_guard_file(&path) {
        Some(g) if g.op_id == op_id => Ok(path),
        _ => Err("compatibility guard durable 复核失败".to_string()),
    }
}

pub fn state_pending(op_type: &str, op_id: &str, phase: &str, target: &Path) -> DataRootState {
    DataRootState {
        version: STATE_VERSION,
        active_root: None,
        pending_operation: Some(PendingOperation {
            op_type: op_type.to_string(),
            op_id: op_id.to_string(),
            phase: phase.to_string(),
            source: None,
            target: Some(target.to_path_buf()),
            updated_at: chrono::Local::now().to_rfc3339(),
        }),
        updated_at: chrono::Local::now().to_rfc3339(),
    }
}

pub fn state_active_external(target: &Path) -> DataRootState {
    DataRootState {
        version: STATE_VERSION,
        active_root: Some(target.to_path_buf()),
        pending_operation: None,
        updated_at: chrono::Local::now().to_rfc3339(),
    }
}

/// 未完成 external init 的可恢复判定（§九）：
/// 必须能证明这是"本次 external init 在 state 落盘前中断"，否则不给恢复入口。
/// 证据 = recognized G2 guard（含 op_id/target）+ Config Root 无正式库/无 legacy/无归档。
pub struct OrphanInitCandidate {
    pub op_id: String,
    pub target: Option<PathBuf>,
    pub guard_path: PathBuf,
}

pub fn orphan_init_candidate(config_root: &Path) -> Option<OrphanInitCandidate> {
    if matches!(load_state(config_root), StateLoad::Loaded(_)) {
        return None; // 有 state 说明不是"state 前中断"，交给 pending 流程
    }
    if has_formal_db(config_root) || has_legacy_db(config_root) || has_renamed_archive(config_root)
    {
        return None; // Config Root 有真实数据 → 可能是迁移场景，不得当作 init 恢复
    }
    let guards = recognize_guards(config_root);
    let g2 = guards.iter().find(|g| g.kind == GuardKind::G2Stray)?;
    Some(OrphanInitCandidate {
        op_id: g2.op_id.clone(),
        target: g2.target.clone(),
        guard_path: g2.path.clone(),
    })
}

/// 清理一次未完成 init 留下的抽屉柜自有文件（白名单，绝不 remove_dir_all 用户目录）
pub fn cleanup_init_artifacts(target: &Path) -> Result<(), String> {
    for name in [
        V2_DB_FILENAME.to_string(),
        format!("{}-wal", V2_DB_FILENAME),
        format!("{}-shm", V2_DB_FILENAME),
        V2_TMP_FILENAME.to_string(),
        format!("{}-wal", V2_TMP_FILENAME),
        format!("{}-shm", V2_TMP_FILENAME),
        format!("{}-journal", V2_TMP_FILENAME),
        V2_SETUP_LOCK_FILENAME.to_string(),
    ] {
        let path = target.join(&name);
        if path.exists() {
            fs::remove_file(&path).map_err(|e| format!("清理 {} 失败: {}", name, e))?;
        }
    }
    Ok(())
}

/// 删除 G2 guard（仅用于显式放弃/回滚流程；正常路径绝不自动删）
pub fn remove_compat_guard(config_root: &Path, op_id: &str) -> Result<(), String> {
    let path = config_root.join(format!("{}{}", GUARD_G2_PREFIX, op_id));
    if path.exists() {
        fs::remove_file(&path).map_err(|e| format!("删除 guard 失败: {}", e))?;
    }
    Ok(())
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
// Phase 2C-2：启动模式（DB 未打开时的前端分支）与初始化上下文
// ============================================================

/// 首次初始化 / 恢复模式（与 BlockedState 并列，互斥）
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SetupModeInfo {
    /// 空环境：请用户选择数据保存位置（不自动建库）
    Choose { config_root: String },
    /// 未完成的 external 初始化（recovery 未确认或 tmp 半成）→ 继续或放弃
    ResumeInit {
        op_id: String,
        target: String,
        phase: String,
    },
    /// G2 guard 存在但 state 缺失（state 落盘前中断）→ 继续或放弃
    OrphanInitRecover {
        op_id: String,
        target: Option<String>,
    },
}

/// setup 期间挂载；进入正常模式后 set(None)
pub struct SetupState(pub std::sync::Mutex<Option<SetupModeInfo>>);

impl Default for SetupState {
    fn default() -> Self {
        SetupState(std::sync::Mutex::new(None))
    }
}

impl SetupState {
    pub fn set(&self, mode: Option<SetupModeInfo>) {
        if let Ok(mut guard) = self.0.lock() {
            *guard = mode;
        }
    }
    pub fn peek(&self) -> Option<SetupModeInfo> {
        self.0.lock().ok().and_then(|g| g.clone())
    }
    pub fn take(&self) -> Option<SetupModeInfo> {
        self.0.lock().ok().and_then(|mut g| g.take())
    }
}

/// 自定义 Data Root 初始化上下文（进程内；finalize 时用于写入 active_root）
#[derive(Debug, Clone)]
pub struct DataRootInitCtx {
    pub op_id: String,
    pub target: PathBuf,
}

pub struct InitContext(pub std::sync::Mutex<Option<DataRootInitCtx>>);

impl Default for InitContext {
    fn default() -> Self {
        InitContext(std::sync::Mutex::new(None))
    }
}

impl InitContext {
    pub fn set(&self, ctx: Option<DataRootInitCtx>) {
        if let Ok(mut guard) = self.0.lock() {
            *guard = ctx;
        }
    }
    pub fn get(&self) -> Option<DataRootInitCtx> {
        self.0.lock().ok().and_then(|g| g.clone())
    }
}

#[tauri::command]
pub fn get_setup_mode(app: tauri::AppHandle) -> Option<SetupModeInfo> {
    use tauri::Manager;
    app.try_state::<SetupState>().and_then(|s| s.peek())
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

    // ==================== Phase 2C-2 ====================

    /// 造一个真实可用的 v2 库（复用生产 prepare/finalize，不复制密码学实现）
    fn make_valid_v2_db(dir: &Path) {
        let prepared =
            crate::migration::prepare_fresh_v2(dir, "test-master-password").expect("prepare 失败");
        crate::migration::finalize_prepared_v2(&prepared.tmp_path, &prepared.v2_path)
            .expect("finalize 失败");
        drop(prepared.setup_lock);
        let lock = dir.join(V2_SETUP_LOCK_FILENAME);
        let _ = fs::remove_file(lock);
        assert!(dir.join(V2_DB_FILENAME).exists());
    }

    /// 独立的 target 目录（不得嵌在 config root 里——那会被 preflight 正确拒绝）
    fn temp_target(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!("drawer-target-{}-{}", label, uuid::Uuid::new_v4()))
    }

    // 21. preflight：新目录（不存在）→ 创建并 Ok
    #[test]
    fn t21_preflight_creates_new_dir() {
        let cr = temp_root("t21c");
        let target = temp_target("t21");
        assert_eq!(preflight_new_root(&cr, &target), Ok(()));
        assert!(target.is_dir());
        assert!(dir_is_empty(&target));
    }

    // 22/23. preflight：Config Root 自身 / 互相嵌套 → 拒绝
    #[test]
    fn t22_t23_preflight_rejects_config_root_and_nesting() {
        let cr = temp_root("t22c");
        assert_eq!(
            preflight_new_root(&cr, &cr),
            Err(PreflightError::IsConfigRoot(normalized_lower(&cr)))
        );
        let nested = cr.join("inside");
        assert!(matches!(
            preflight_new_root(&cr, &nested),
            Err(PreflightError::NestedWithConfigRoot(_))
        ));
        let parent = cr.parent().unwrap().to_path_buf();
        assert!(matches!(
            preflight_new_root(&cr, &parent),
            Err(PreflightError::NestedWithConfigRoot(_))
        ));
    }

    // 24. preflight：UNC / 云同步目录 → 拒绝
    #[test]
    fn t24_preflight_rejects_network_and_sync_folders() {
        let cr = temp_root("t24c");
        assert!(matches!(
            preflight_new_root(&cr, Path::new("\\\\server\\share\\drawer")),
            Err(PreflightError::NetworkOrRemovable(_))
        ));
        assert!(matches!(
            preflight_new_root(&cr, Path::new("D:\\Users\\me\\OneDrive\\DrawerData")),
            Err(PreflightError::SyncFolder(_))
        ));
    }

    // 25. preflight：已有抽屉柜数据 → 拒绝（转 attach）
    #[test]
    fn t25_preflight_rejects_existing_drawer_data() {
        let cr = temp_root("t25c");
        let target = temp_target("t25");
        fs::create_dir_all(&target).unwrap();
        fs::write(target.join(V2_DB_FILENAME), b"x").unwrap();
        assert!(matches!(
            preflight_new_root(&cr, &target),
            Err(PreflightError::HasExistingDrawerData(_))
        ));
    }

    // 26. preflight：非空普通目录 → 拒绝
    #[test]
    fn t26_preflight_rejects_non_empty_dir() {
        let cr = temp_root("t26c");
        let target = temp_target("t26");
        fs::create_dir_all(&target).unwrap();
        fs::write(target.join("我的照片.jpg"), b"x").unwrap();
        assert!(matches!(
            preflight_new_root(&cr, &target),
            Err(PreflightError::NotEmpty(_))
        ));
    }

    // 27. preflight：中文 + 空格 + 括号路径 → 通过
    #[test]
    fn t27_preflight_accepts_unicode_space_paren() {
        let cr = temp_root("t27c");
        let target = temp_target("抽屉柜数据 (测试) 2C2");
        assert_eq!(preflight_new_root(&cr, &target), Ok(()));
    }

    // 28. attach：合法 v2 库 → Ok
    #[test]
    fn t28_attach_valid_v2_ok() {
        let cr = temp_root("t28c");
        let target = cr.join("existing-root");
        fs::create_dir_all(&target).unwrap();
        make_valid_v2_db(&target);
        assert_eq!(validate_existing_root(&cr, &target), Ok(()));
    }

    // 29/30/31/32/33. attach 各类拒绝
    #[test]
    fn t29_t33_attach_rejections() {
        let cr = temp_root("t29c");
        // 不存在
        assert!(matches!(
            validate_existing_root(&cr, &cr.join("nope")),
            Err(AttachError::NotFound(_))
        ));
        // 空目录 → DbMissing
        let empty = cr.join("empty");
        fs::create_dir_all(&empty).unwrap();
        assert!(matches!(
            validate_existing_root(&cr, &empty),
            Err(AttachError::DbMissing(_))
        ));
        // legacy-only → LegacyOnly（绝不当作 v2）
        let legacy = cr.join("legacy-only");
        fs::create_dir_all(&legacy).unwrap();
        fs::write(legacy.join(LEGACY_DB_FILENAME), b"x").unwrap();
        assert!(matches!(
            validate_existing_root(&cr, &legacy),
            Err(AttachError::LegacyOnly(_))
        ));
        // 损坏 db → DbInvalid
        let broken = cr.join("broken");
        fs::create_dir_all(&broken).unwrap();
        fs::write(broken.join(V2_DB_FILENAME), b"this is not sqlite").unwrap();
        assert!(matches!(
            validate_existing_root(&cr, &broken),
            Err(AttachError::DbInvalid(_))
        ));
        // tmp 残留 → MigrationArtifacts
        let dirty = cr.join("dirty");
        fs::create_dir_all(&dirty).unwrap();
        make_valid_v2_db(&dirty);
        fs::write(dirty.join(V2_TMP_FILENAME), b"leftover").unwrap();
        assert!(matches!(
            validate_existing_root(&cr, &dirty),
            Err(AttachError::MigrationArtifacts(_))
        ));
    }

    // 34. orphan init 证明：state 缺失 + G2 guard + 无正式库 → 可恢复；
    //     有正式库 / 有 state → 不可当作 init 恢复
    #[test]
    fn t34_orphan_init_candidate_proof() {
        // 可恢复
        let root = temp_root("t34a");
        let target = root.join("ext-target");
        write_compat_guard_g2(&root, "op-34", &target).expect("写 guard 失败");
        let cand = orphan_init_candidate(&root).expect("应识别为可恢复 init");
        assert_eq!(cand.op_id, "op-34");
        assert_eq!(cand.target.as_deref(), Some(target.as_path()));
        // Config Root 出现正式库 → 不是 init 恢复场景
        let root2 = temp_root("t34b");
        fs::write(root2.join(V2_DB_FILENAME), b"x").unwrap();
        write_compat_guard_g2(&root2, "op-34b", &target).unwrap();
        assert!(orphan_init_candidate(&root2).is_none());
        // 已有 state → 交给 pending 流程
        let root3 = temp_root("t34c");
        write_compat_guard_g2(&root3, "op-34c", &target).unwrap();
        put_state(
            &root3,
            &state_pending("init", "op-34c", "preparing", &target),
        );
        assert!(orphan_init_candidate(&root3).is_none());
    }

    // 35. state 构造往返（pending / active）
    #[test]
    fn t35_state_builders_roundtrip() {
        let root = temp_root("t35");
        let target = root.join("数据 目录");
        let pending = state_pending("init", "op-35", "preparing", &target);
        save_state(&root, &pending).unwrap();
        match load_state(&root) {
            StateLoad::Loaded(s) => {
                let op = s.pending_operation.expect("missing pending");
                assert_eq!(op.op_type, "init");
                assert_eq!(op.phase, "preparing");
                assert_eq!(op.target.as_deref(), Some(target.as_path()));
                assert!(s.active_root.is_none());
            }
            other => panic!("期望 Loaded，实际 {:?}", other),
        }
        let active = state_active_external(&target);
        save_state(&root, &active).unwrap();
        match load_state(&root) {
            StateLoad::Loaded(s) => {
                assert_eq!(s.active_root.as_deref(), Some(target.as_path()));
                assert!(s.pending_operation.is_none());
            }
            other => panic!("期望 Loaded，实际 {:?}", other),
        }
    }

    // 36. cleanup_init_artifacts 只删抽屉柜白名单文件，用户文件保留
    #[test]
    fn t36_cleanup_is_whitelist_only() {
        let root = temp_root("t36");
        let target = root.join("user-dir");
        fs::create_dir_all(&target).unwrap();
        fs::write(target.join("我的重要文档.txt"), b"keep me").unwrap();
        fs::write(target.join(V2_TMP_FILENAME), b"tmp").unwrap();
        fs::write(target.join(V2_SETUP_LOCK_FILENAME), b"lock").unwrap();
        cleanup_init_artifacts(&target).expect("cleanup 失败");
        assert!(!target.join(V2_TMP_FILENAME).exists());
        assert!(!target.join(V2_SETUP_LOCK_FILENAME).exists());
        assert!(target.join("我的重要文档.txt").exists(), "用户文件不得删除");
    }

    // 37. guard 写入 durable + 可解析 + 内容非 SQLite
    #[test]
    fn t37_guard_write_is_durable_and_parseable() {
        let root = temp_root("t37");
        let target = root.join("ext");
        let path = write_compat_guard_g2(&root, "op-37", &target).expect("写 guard 失败");
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.starts_with(GUARD_MAGIC));
        assert!(text.contains("op_id=op-37"));
        let guard = parse_guard_file(&path).expect("guard 应可解析");
        assert_eq!(guard.op_id, "op-37");
        assert_eq!(guard.target.as_deref(), Some(target.as_path()));
        // 同一 op_id 重复写不覆盖（create_new）
        assert!(write_compat_guard_g2(&root, "op-37", &target).is_err());
    }

    // 38. 自定义位置初始化完整序列（逻辑层端到端）：
    //     preflight → guard → state(preparing) → prepare → finalize → state(activated) → active
    //     → resolver=UseExternal → 既有仲裁命中 V2
    #[test]
    fn t38_custom_init_full_sequence() {
        let cr = temp_root("t38c");
        let target = temp_target("t38");
        assert_eq!(preflight_new_root(&cr, &target), Ok(()));
        let op_id = "op-38";
        write_compat_guard_g2(&cr, op_id, &target).expect("guard 写入失败");
        save_state(&cr, &state_pending("init", op_id, "preparing", &target)).unwrap();
        // 目标写入（等价于 prepare）：必须发生在 guard+state 之后
        let prepared =
            crate::migration::prepare_fresh_v2(&target, "test-pw-123456").expect("prepare 失败");
        save_state(
            &cr,
            &state_pending("init", op_id, "recovery_presented", &target),
        )
        .unwrap();
        save_state(
            &cr,
            &state_pending("init", op_id, "recovery_confirmed", &target),
        )
        .unwrap();
        crate::migration::finalize_prepared_v2(&prepared.tmp_path, &prepared.v2_path)
            .expect("finalize 失败");
        drop(prepared.setup_lock);
        let _ = fs::remove_file(target.join(V2_SETUP_LOCK_FILENAME));
        save_state(&cr, &state_pending("init", op_id, "activated", &target)).unwrap();
        save_state(&cr, &state_active_external(&target)).unwrap();
        assert_eq!(
            resolve_data_root(&cr),
            Resolution::UseExternal(target.clone())
        );
        assert!(matches!(
            crate::migration::resolve_startup_db(&target, true).selection,
            crate::migration::DbSelection::V2(_)
        ));
    }

    // 39. 初始化崩溃矩阵的启动决策（try_complete_pending_init）
    #[test]
    fn t39_init_crash_matrix_decisions() {
        // (a) preparing + 目标无数据 → 不自动激活（交恢复页）
        let cr = temp_root("t39a");
        let target = temp_target("t39a");
        fs::create_dir_all(&target).unwrap();
        let op = state_pending("init", "op-39a", "preparing", &target)
            .pending_operation
            .unwrap();
        assert!(crate::try_complete_pending_init(&cr, &op).is_none());
        // (b) recovery_confirmed + 半成 tmp（不可开）→ 不自动激活
        let cr2 = temp_root("t39b");
        let target2 = temp_target("t39b");
        fs::create_dir_all(&target2).unwrap();
        fs::write(target2.join(V2_TMP_FILENAME), b"half-written").unwrap();
        let op2 = state_pending("init", "op-39b", "recovery_confirmed", &target2)
            .pending_operation
            .unwrap();
        assert!(crate::try_complete_pending_init(&cr2, &op2).is_none());
        // (c) recovery_confirmed + 可开 tmp → 完成 finalize 并提交 active_root
        let cr3 = temp_root("t39c");
        let target3 = temp_target("t39c");
        fs::create_dir_all(&target3).unwrap();
        let prepared =
            crate::migration::prepare_fresh_v2(&target3, "test-pw-123456").expect("prepare 失败");
        drop(prepared.setup_lock);
        let _ = fs::remove_file(target3.join(V2_SETUP_LOCK_FILENAME));
        let op3 = state_pending("init", "op-39c", "recovery_confirmed", &target3)
            .pending_operation
            .unwrap();
        assert_eq!(
            crate::try_complete_pending_init(&cr3, &op3).as_deref(),
            Some(target3.as_path())
        );
        assert!(target3.join(V2_DB_FILENAME).exists(), "应完成 finalize");
        match load_state(&cr3) {
            StateLoad::Loaded(s) => {
                assert_eq!(s.active_root.as_deref(), Some(target3.as_path()));
                assert!(s.pending_operation.is_none());
            }
            other => panic!("期望 active state，实际 {:?}", other),
        }
        // (d) activated + 正式库存在 → 补写 active_root
        let cr4 = temp_root("t39d");
        let target4 = temp_target("t39d");
        fs::create_dir_all(&target4).unwrap();
        make_valid_v2_db(&target4);
        let op4 = state_pending("init", "op-39d", "activated", &target4)
            .pending_operation
            .unwrap();
        assert_eq!(
            crate::try_complete_pending_init(&cr4, &op4).as_deref(),
            Some(target4.as_path())
        );
    }

    // 40. attach 完整序列：验证 → guard → state(attach) → 启动收尾提交 active_root
    #[test]
    fn t40_attach_full_sequence() {
        let cr = temp_root("t40c");
        let target = temp_target("t40");
        fs::create_dir_all(&target).unwrap();
        make_valid_v2_db(&target);
        assert_eq!(validate_existing_root(&cr, &target), Ok(()));
        let op_id = "op-40";
        write_compat_guard_g2(&cr, op_id, &target).unwrap();
        save_state(
            &cr,
            &state_pending("attach_existing", op_id, "pending_restart", &target),
        )
        .unwrap();
        let op = state_pending("attach_existing", op_id, "pending_restart", &target)
            .pending_operation
            .unwrap();
        assert_eq!(
            crate::try_complete_pending_attach(&cr, &op).as_deref(),
            Some(target.as_path())
        );
        assert_eq!(
            resolve_data_root(&cr),
            Resolution::UseExternal(target.clone())
        );
        assert!(
            recognize_guards(&cr).iter().any(|g| g.op_id == op_id),
            "attach 完成后 G2 guard 必须保留"
        );
    }

    // 41. Windows 重装模拟：新 Config Root（无 state/guard）+ 外部有效数据
    //     → 初始表现为默认（新版进"选择数据位置"页，不 FreshV2）
    //     → attach 收尾后 = UseExternal；全程未产生 C 盘正式库
    #[test]
    fn t41_reinstall_simulation() {
        let external = temp_target("t41-data");
        fs::create_dir_all(&external).unwrap();
        make_valid_v2_db(&external);

        let cr = temp_root("t41c"); // 全新 Config Root（重装后）
        assert_eq!(resolve_data_root(&cr), Resolution::UseDefault(cr.clone()));
        assert!(!has_formal_db(&cr), "Config Root 不得出现正式库");

        assert_eq!(validate_existing_root(&cr, &external), Ok(()));
        write_compat_guard_g2(&cr, "op-41", &external).unwrap();
        let op = state_pending("attach_existing", "op-41", "pending_restart", &external)
            .pending_operation
            .unwrap();
        assert_eq!(
            crate::try_complete_pending_attach(&cr, &op).as_deref(),
            Some(external.as_path())
        );
        assert_eq!(
            resolve_data_root(&cr),
            Resolution::UseExternal(external.clone())
        );
        assert!(!has_formal_db(&cr));
    }
}

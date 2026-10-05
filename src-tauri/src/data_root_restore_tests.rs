//! ===== Phase 2C-4 单元测试：External → Default 恢复 + retained source 生命周期 =====
//!
//! 覆盖（§十六/§十七/§二十/§二十一/§二十五 对应的可自动化子集）：
//! - P0 隔离约束：restore staging 与 G1 永远不同文件
//! - restore-default preflight 规则（未知归档 fail closed）
//! - state schema 向后兼容（2C-3 时代 state 无 retained_sources 可解析）
//! - state_after_restore_commit 状态机（旧 last_migration 折算 + 幂等）
//! - Guard 清理顺序与失败语义（G2→G1；外来 guard fail closed）
//! - restore 回滚（staging + 本 op G1 清除，旧 G2 保留）
//! - reconcile 对账（文件已删但记录未更新）
//! - 白名单删除（只删登记目标；路径/身份不匹配拒绝）
//! - guard 残留结构化 recovery 判定
//! - C→D→C 全序列（旧 C archive 绝不当恢复源）

use std::fs;
use std::path::{Path, PathBuf};

use crate::data_root::*;
use crate::migration::{
    finalize_prepared_v2, open_existing_v2_db, prepare_fresh_v2, LEGACY_DB_FILENAME,
    V2_DB_FILENAME, V2_SETUP_LOCK_FILENAME, V2_TMP_FILENAME,
};

fn temp_root(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("drawer-dr4-{}-{}", label, uuid::Uuid::new_v4()));
    fs::create_dir_all(&dir).expect("建临时目录失败");
    dir
}

fn temp_target(label: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("drawer-target4-{}-{}", label, uuid::Uuid::new_v4()));
    fs::create_dir_all(&d).expect("建目标目录失败");
    d
}

fn put_state(config_root: &Path, state: &DataRootState) {
    save_state(config_root, state).expect("save_state 失败");
}

fn load(config_root: &Path) -> DataRootState {
    match load_state(config_root) {
        StateLoad::Loaded(s) => s,
        other => panic!("期望 Loaded，实际 {:?}", other),
    }
}

fn make_valid_v2_db(dir: &Path) {
    let prepared = prepare_fresh_v2(dir, "test-master-password").expect("prepare 失败");
    finalize_prepared_v2(&prepared.tmp_path, &prepared.v2_path).expect("finalize 失败");
    drop(prepared.setup_lock);
    let _ = fs::remove_file(dir.join(V2_SETUP_LOCK_FILENAME));
    assert!(dir.join(V2_DB_FILENAME).exists());
}

fn add_snippet_row(dir: &Path, title: &str) {
    let db = open_existing_v2_db(&dir.join(V2_DB_FILENAME)).expect("打开库失败");
    let conn = db.conn.lock().unwrap();
    conn.execute(
        "INSERT INTO snippets (title, content, language, created_at, updated_at) VALUES (?1, ?2, 'text', 1, 1)",
        rusqlite::params![title, "content"],
    )
    .expect("插入片段失败");
}

fn snippet_count_by_title(dir: &Path, title: &str) -> i64 {
    let db = open_existing_v2_db(&dir.join(V2_DB_FILENAME)).unwrap();
    let conn = db.conn.lock().unwrap();
    conn.query_row(
        "SELECT COUNT(*) FROM snippets WHERE title = ?1",
        rusqlite::params![title],
        |r| r.get(0),
    )
    .unwrap()
}

// 51. P0 隔离约束：restore staging 与 G1 永远不同文件（§四）
#[test]
fn t51_restore_staging_never_equals_g1() {
    let cr = temp_root("t51");
    let staging = restore_staging_path(&cr, "op51");
    let g1 = cr.join(V2_TMP_FILENAME);
    assert_ne!(staging, g1, "staging 不得复用 G1 文件名");
    assert_ne!(MIGRATION_TMP_PREFIX, V2_TMP_FILENAME, "前缀常量级隔离");
    assert!(staging.to_string_lossy().contains(".migration-"));
    assert!(staging.to_string_lossy().ends_with(".tmp"));
    // G1 文件名是 drawer-v2.db.tmp；staging 是 drawer-v2.db.migration-<op_id>.tmp
    assert!(staging
        .file_name()
        .unwrap()
        .to_string_lossy()
        .starts_with("drawer-v2.db.migration-"));
}

// 52. restore-default preflight 规则（§六）
#[test]
fn t52_restore_default_preflight_rules() {
    // 已知归档 → Ok
    let cr = temp_root("t52a");
    let known = vec!["drawer-v2.db.migrated-old-op".to_string()];
    fs::write(cr.join("drawer-v2.db.migrated-old-op"), b"archive").unwrap();
    assert_eq!(preflight_restore_default(&cr, 1024, &known), Ok(()));
    // 未知归档 → Fail Closed
    let cr2 = temp_root("t52b");
    fs::write(cr2.join("drawer-v2.db.migrated-ghost"), b"???").unwrap();
    assert!(matches!(
        preflight_restore_default(&cr2, 1024, &[]),
        Err(PreflightError::UnknownRetainedArchive(_))
    ));
    // 正式库 → 拒绝
    let cr3 = temp_root("t52c");
    fs::write(cr3.join(V2_DB_FILENAME), b"x").unwrap();
    assert!(matches!(
        preflight_restore_default(&cr3, 1024, &[]),
        Err(PreflightError::ConfigRootOccupied(_))
    ));
    // legacy 库 → 拒绝
    let cr4 = temp_root("t52d");
    fs::write(cr4.join(LEGACY_DB_FILENAME), b"x").unwrap();
    assert!(matches!(
        preflight_restore_default(&cr4, 1024, &[]),
        Err(PreflightError::ConfigRootOccupied(_))
    ));
    // 正式库 sidecar → 拒绝
    let cr5 = temp_root("t52e");
    fs::write(cr5.join(format!("{}-wal", V2_DB_FILENAME)), b"x").unwrap();
    assert!(matches!(
        preflight_restore_default(&cr5, 1024, &[]),
        Err(PreflightError::ConfigRootOccupied(_))
    ));
    // setup lock → 拒绝
    let cr6 = temp_root("t52f");
    fs::write(cr6.join(V2_SETUP_LOCK_FILENAME), b"x").unwrap();
    assert!(matches!(
        preflight_restore_default(&cr6, 1024, &[]),
        Err(PreflightError::ConfigRootOccupied(_))
    ));
    // 非 Magic 的真实 tmp（旧版升级遗留）→ has_migration_artifacts 拒绝
    let cr7 = temp_root("t52g");
    fs::write(cr7.join(V2_TMP_FILENAME), b"real legacy tmp").unwrap();
    assert!(matches!(
        preflight_restore_default(&cr7, 1024, &[]),
        Err(PreflightError::ConfigRootOccupied(_))
    ));
    // known 归档不含 ".migration-" 标记，不与 tmp 语义混淆
    assert!(!"drawer-v2.db.migrated-old-op".contains(MIGRATION_TMP_MARKER));
}

// 53. 向后兼容：2C-3 时代无 retained_sources 字段的 state 可解析
#[test]
fn t53_retained_sources_serde_backward_compat() {
    let legacy_json =
        r#"{"version":1,"active_root":null,"updated_at":"2026-10-05T00:00:00+08:00"}"#;
    let parsed: DataRootState = serde_json::from_str(legacy_json).expect("旧 state 必须可解析");
    assert!(parsed.retained_sources.is_empty());
    // 新 state 序列化往返不丢
    let mut st = DataRootState::new(None);
    st.retained_sources.push(RetainedSource {
        op_id: "op-53".to_string(),
        archive_path: PathBuf::from("C:\\x\\drawer-v2.db.migrated-op-53"),
        original_root: PathBuf::from("C:\\x"),
        migrated_to: PathBuf::from("D:\\y"),
        created_at: "2026-10-05T00:00:00+08:00".to_string(),
        status: "retained".to_string(),
        deleted_at: None,
    });
    let text = serde_json::to_string(&st).unwrap();
    let reparsed: DataRootState = serde_json::from_str(&text).unwrap();
    assert_eq!(reparsed.retained_sources.len(), 1);
    assert_eq!(reparsed.retained_sources[0].op_id, "op-53");
}

// 54. state_after_restore_commit：旧 last_migration 折算 + 本次归档登记（幂等）
#[test]
fn t54_restore_commit_state_machine() {
    let source = temp_target("t54d");
    let cr = temp_root("t54c");
    let mut previous = DataRootState::new(Some(source.clone()));
    previous.last_migration = Some(LastMigration {
        source: cr.clone(),
        target: source.clone(),
        op_id: "op-c2d".to_string(),
        archive: "drawer-v2.db.migrated-op-c2d".to_string(),
        completed_at: chrono::Local::now().to_rfc3339(),
    });
    let op = PendingOperation {
        op_type: "restore_default".to_string(),
        op_id: "op-d2c".to_string(),
        phase: "committing".to_string(),
        source: Some(source.clone()),
        target: Some(cr.clone()),
        source_kind: Some(SourceKind::External.as_str().to_string()),
        source_fingerprint: None,
        target_fingerprint: None,
        started_at: None,
        updated_at: chrono::Local::now().to_rfc3339(),
    };
    let committed = state_after_restore_commit(&previous, &op);
    assert!(
        committed.active_root.is_none(),
        "active_root 必须翻转为 null"
    );
    assert!(committed.pending_operation.is_none());
    assert_eq!(committed.last_migration.as_ref().unwrap().op_id, "op-d2c");
    assert_eq!(committed.retained_sources.len(), 2);
    let ids: Vec<&str> = committed
        .retained_sources
        .iter()
        .map(|r| r.op_id.as_str())
        .collect();
    assert!(ids.contains(&"op-c2d"), "旧 C→D 归档必须折算登记");
    assert!(ids.contains(&"op-d2c"), "本次 D→C 归档必须登记");
    let c2d = committed
        .retained_sources
        .iter()
        .find(|r| r.op_id == "op-c2d")
        .unwrap();
    assert_eq!(c2d.archive_path, cr.join("drawer-v2.db.migrated-op-c2d"));
    assert_eq!(c2d.status, "retained");
    // 幂等：重复提交不重复登记
    let again = state_after_restore_commit(&committed, &op);
    assert_eq!(again.retained_sources.len(), 2);
}

// 55. Guard 清理顺序与失败语义（§十五）：G2+G1 全清；外来 G1 保留并报残留
#[test]
fn t55_cleanup_guards_after_restore() {
    let cr = temp_root("t55a");
    write_compat_guard_g2(&cr, "op-c2d", &temp_target("t55x")).unwrap();
    write_compat_guard_g1(&cr, "op-d2c").unwrap();
    cleanup_guards_after_restore(&cr, "op-d2c").expect("应全部清理成功");
    assert!(recognize_guards(&cr).is_empty(), "G2 与 G1 都应删除");
    // 外来 G1（op 不匹配）→ fail closed：保留 + Err
    let cr2 = temp_root("t55b");
    write_compat_guard_g1(&cr2, "foreign-op").unwrap();
    assert!(cleanup_guards_after_restore(&cr2, "op-d2c").is_err());
    assert!(cr2.join(V2_TMP_FILENAME).exists(), "外来 G1 不得删除");
}

// 56. restore 回滚：staging + 本 op G1 清除，旧 G2 保留；外来 G1 拒绝
#[test]
fn t56_rollback_restore_default() {
    let cr = temp_root("t56a");
    let source = temp_target("t56d");
    write_compat_guard_g2(&cr, "op-c2d", &source).unwrap();
    write_compat_guard_g1(&cr, "op-d2c").unwrap();
    let tmp = restore_staging_path(&cr, "op-d2c");
    fs::write(&tmp, b"partial").unwrap();
    let op = state_pending_restore_default(
        &source,
        &cr,
        "op-d2c",
        "transferring",
        None,
        None,
        &DataRootState::new(Some(source.clone())),
    )
    .pending_operation
    .unwrap();
    rollback_restore_default(&cr, &op).unwrap();
    assert!(!tmp.exists());
    assert!(!cr.join(V2_TMP_FILENAME).exists(), "本 op G1 应回滚移除");
    assert!(
        recognize_guards(&cr).iter().any(|g| g.op_id == "op-c2d"),
        "旧 G2 必须保留"
    );
    // 外来 G1 → 拒绝清理
    let cr2 = temp_root("t56b");
    write_compat_guard_g1(&cr2, "foreign").unwrap();
    let op2 = state_pending_restore_default(
        &source,
        &cr2,
        "op-x",
        "transferring",
        None,
        None,
        &DataRootState::new(Some(source.clone())),
    )
    .pending_operation
    .unwrap();
    assert!(rollback_restore_default(&cr2, &op2).is_err());
    assert!(cr2.join(V2_TMP_FILENAME).exists());
}

// 57. Offline Safety Gate（修订）：文件缺失/位置离线 绝不自动标 removed；
//     可用性为运行时派生；对缺失条目的删除一律 fail closed
#[test]
fn t57_retained_offline_safety() {
    let cr = temp_root("t57");
    let offline_dir = temp_target("t57-offline-root");
    let mut st = DataRootState::new(None);
    // 条目 a：所在目录模拟离线（目录整体改名后 archive_path 不可达）
    st.retained_sources.push(RetainedSource {
        op_id: "op-57a".to_string(),
        archive_path: offline_dir.join("drawer-v2.db.migrated-op-57a"),
        original_root: offline_dir.clone(),
        migrated_to: cr.clone(),
        created_at: chrono::Local::now().to_rfc3339(),
        status: "retained".to_string(),
        deleted_at: None,
    });
    // 条目 b：文件在（可用）
    st.retained_sources.push(RetainedSource {
        op_id: "op-57b".to_string(),
        archive_path: cr.join("drawer-v2.db.migrated-op-57b"),
        original_root: cr.clone(),
        migrated_to: temp_target("t57y"),
        created_at: chrono::Local::now().to_rfc3339(),
        status: "retained".to_string(),
        deleted_at: None,
    });
    fs::write(
        offline_dir.join("drawer-v2.db.migrated-op-57a"),
        b"offline copy",
    )
    .unwrap();
    fs::write(cr.join("drawer-v2.db.migrated-op-57b"), b"alive").unwrap();
    put_state(&cr, &st);

    // 可用性派生（不持久化）：在位=true
    let avail = retained_availability(&st);
    assert_eq!(avail[0].1, true, "目录在位时应派生为可用");
    assert_eq!(avail[1].1, true);

    // ---- 模拟离线：目录改名 → archive_path 不可达 ----
    let renamed = offline_dir.with_extension("__OFFLINE__");
    fs::rename(&offline_dir, &renamed).unwrap();
    let avail_off = retained_availability(&load(&cr));
    assert_eq!(avail_off[0].1, false, "离线条目必须派生为不可用");

    // 状态文件必须原样保留（无任何启动路径会把它写成 removed）
    assert_eq!(
        load(&cr).retained_sources[0].status,
        "retained",
        "离线绝不等同于已删除"
    );
    assert_eq!(load(&cr).retained_sources[0].deleted_at, None);

    // 离线条目删除 → fail closed，metadata 保留
    let err = delete_retained_source_checked(&cr, "op-57a").unwrap_err();
    assert!(err.contains("不可访问"), "失败原因必须是不可访问: {}", err);
    assert_eq!(
        load(&cr).retained_sources[0].status,
        "retained",
        "删除失败后 metadata 必须保留"
    );
    assert!(
        renamed.join("drawer-v2.db.migrated-op-57a").exists(),
        "离线文件不得被触碰"
    );

    // ---- 恢复在线：同名目录回来 → 可用性自动恢复，无需重新登记 ----
    fs::rename(&renamed, &offline_dir).unwrap();
    let avail2 = retained_availability(&load(&cr));
    assert_eq!(avail2[0].1, true, "文件重新出现后必须恢复可用");
    // 显式删除现在可以成功（用户重新发起）
    delete_retained_source_checked(&cr, "op-57a").unwrap();
    assert_eq!(load(&cr).retained_sources[0].status, "removed");
    assert!(!offline_dir.join("drawer-v2.db.migrated-op-57a").exists());

    // ---- out-of-band 删除（用户资源管理器手工删）：保持 retained，不自动 removed ----
    fs::remove_file(cr.join("drawer-v2.db.migrated-op-57b")).unwrap();
    assert_eq!(
        load(&cr).retained_sources[1].status,
        "retained",
        "手工删除不得被解释为应用完成的删除"
    );
    assert!(
        delete_retained_source_checked(&cr, "op-57b").is_err(),
        "对已消失文件的删除必须 fail closed"
    );
    assert_eq!(
        load(&cr).retained_sources[1].status,
        "retained",
        "fail closed 后 metadata 仍保留"
    );
}

// 58. 白名单删除：只删登记目标 + sidecars；路径/身份不匹配一律 fail closed
#[test]
fn t58_delete_retained_source_whitelist() {
    let cr = temp_root("t58c");
    let d_root = temp_target("t58d");
    let mut st = DataRootState::new(Some(d_root.clone()));
    // 两个 retained：目标（D 盘）+ 无关项（cr）
    st.retained_sources.push(RetainedSource {
        op_id: "op-58a".to_string(),
        archive_path: d_root.join("drawer-v2.db.migrated-op-58a"),
        original_root: d_root.clone(),
        migrated_to: cr.clone(),
        created_at: chrono::Local::now().to_rfc3339(),
        status: "retained".to_string(),
        deleted_at: None,
    });
    st.retained_sources.push(RetainedSource {
        op_id: "op-58b".to_string(),
        archive_path: cr.join("drawer-v2.db.migrated-op-58b"),
        original_root: cr.clone(),
        migrated_to: d_root.clone(),
        created_at: chrono::Local::now().to_rfc3339(),
        status: "retained".to_string(),
        deleted_at: None,
    });
    fs::write(d_root.join("drawer-v2.db.migrated-op-58a"), b"target").unwrap();
    fs::write(d_root.join("drawer-v2.db.migrated-op-58a-wal"), b"sidecar").unwrap();
    fs::write(cr.join("drawer-v2.db.migrated-op-58b"), b"other").unwrap();
    put_state(&cr, &st);

    delete_retained_source_checked(&cr, "op-58a").expect("白名单删除应成功");
    assert!(!d_root.join("drawer-v2.db.migrated-op-58a").exists());
    assert!(!d_root.join("drawer-v2.db.migrated-op-58a-wal").exists());
    assert!(
        cr.join("drawer-v2.db.migrated-op-58b").exists(),
        "无关归档不得受影响"
    );
    let s = load(&cr);
    let a = s
        .retained_sources
        .iter()
        .find(|r| r.op_id == "op-58a")
        .unwrap();
    assert_eq!(a.status, "removed");
    assert!(
        s.retained_sources
            .iter()
            .any(|r| r.op_id == "op-58b" && r.status == "retained"),
        "无关登记必须保留"
    );
    // 重复删除 → 明确报错
    assert!(delete_retained_source_checked(&cr, "op-58a").is_err());
    // 未登记 op → 拒绝
    assert!(delete_retained_source_checked(&cr, "op-ghost").is_err());
    // 文件被移出登记目录 → 路径不匹配拒绝
    let cr2 = temp_root("t58e");
    let mut st2 = DataRootState::new(None);
    st2.retained_sources.push(RetainedSource {
        op_id: "op-58c".to_string(),
        archive_path: cr2.join("drawer-v2.db.migrated-op-58c"),
        original_root: cr2.join("elsewhere"),
        migrated_to: cr2.clone(),
        created_at: chrono::Local::now().to_rfc3339(),
        status: "retained".to_string(),
        deleted_at: None,
    });
    fs::write(cr2.join("drawer-v2.db.migrated-op-58c"), b"x").unwrap();
    put_state(&cr2, &st2);
    assert!(delete_retained_source_checked(&cr2, "op-58c").is_err());
    assert!(
        cr2.join("drawer-v2.db.migrated-op-58c").exists(),
        "拒绝时不得删除"
    );
    // pending 存在 → 拒绝
    let cr3 = temp_root("t58f");
    let mut st3 = state_pending("init", "op-58p", "preparing", &temp_target("t58p"));
    st3.retained_sources.push(RetainedSource {
        op_id: "op-58q".to_string(),
        archive_path: cr3.join("drawer-v2.db.migrated-op-58q"),
        original_root: cr3.clone(),
        migrated_to: cr3.clone(),
        created_at: chrono::Local::now().to_rfc3339(),
        status: "retained".to_string(),
        deleted_at: None,
    });
    fs::write(cr3.join("drawer-v2.db.migrated-op-58q"), b"x").unwrap();
    put_state(&cr3, &st3);
    assert!(delete_retained_source_checked(&cr3, "op-58q").is_err());
    assert!(
        cr3.join("drawer-v2.db.migrated-op-58q").exists(),
        "pending 期间不得删除"
    );
}

// 59. guard 残留结构化 recovery 判定（§十五）
#[test]
fn t59_restore_guard_residual_candidate() {
    // G1(restore op) + G2(历史 op) 残留 + 合法 state + 正式库 → 可清理
    let cr = temp_root("t59a");
    make_valid_v2_db(&cr);
    let mut st = DataRootState::new(None);
    st.last_migration = Some(LastMigration {
        source: temp_target("t59s"),
        target: cr.clone(),
        op_id: "op-d2c".to_string(),
        archive: "drawer-v2.db.migrated-op-d2c".to_string(),
        completed_at: chrono::Local::now().to_rfc3339(),
    });
    st.retained_sources.push(RetainedSource {
        op_id: "op-c2d".to_string(),
        archive_path: cr.join("drawer-v2.db.migrated-op-c2d"),
        original_root: cr.clone(),
        migrated_to: temp_target("t59t"),
        created_at: chrono::Local::now().to_rfc3339(),
        status: "retained".to_string(),
        deleted_at: None,
    });
    put_state(&cr, &st);
    write_compat_guard_g1(&cr, "op-d2c").unwrap();
    write_compat_guard_g2(&cr, "op-c2d", &temp_target("t59u")).unwrap();
    let paths = restore_guard_residual_candidate(&cr).expect("已知归属的 guard 残留应可清理");
    assert_eq!(paths.len(), 2);
    for p in &paths {
        fs::remove_file(p).unwrap();
    }
    assert_eq!(resolve_data_root(&cr), Resolution::UseDefault(cr.clone()));
    // 未知 op 的 guard → 拒绝（fail closed）
    let cr2 = temp_root("t59b");
    make_valid_v2_db(&cr2);
    let mut st2 = DataRootState::new(None);
    st2.last_migration = Some(LastMigration {
        source: temp_target("t59v"),
        target: cr2.clone(),
        op_id: "op-d2c".to_string(),
        archive: "drawer-v2.db.migrated-op-d2c".to_string(),
        completed_at: chrono::Local::now().to_rfc3339(),
    });
    put_state(&cr2, &st2);
    write_compat_guard_g1(&cr2, "unknown-op").unwrap();
    assert!(restore_guard_residual_candidate(&cr2).is_none());
    // external 态（active_root 存在）→ 不适用
    let cr3 = temp_root("t59c");
    make_valid_v2_db(&cr3);
    let mut st3 = DataRootState::new(Some(temp_target("t59w")));
    st3.last_migration = Some(LastMigration {
        source: cr3.clone(),
        target: temp_target("t59w"),
        op_id: "op-x".to_string(),
        archive: "drawer-v2.db.migrated-op-x".to_string(),
        completed_at: chrono::Local::now().to_rfc3339(),
    });
    put_state(&cr3, &st3);
    write_compat_guard_g1(&cr3, "op-x").unwrap();
    assert!(restore_guard_residual_candidate(&cr3).is_none());
}

// 60. 稳定 external 态的 G1 残留判定
#[test]
fn t60_stale_g1_on_stable_external() {
    // Magic G1 + 无 pending + external → 可清理
    let cr = temp_root("t60a");
    let source = temp_target("t60d");
    write_compat_guard_g2(&cr, "op-c2d", &source).unwrap();
    write_compat_guard_g1(&cr, "op-d2c").unwrap();
    put_state(&cr, &state_active_external(&source));
    assert_eq!(
        stale_g1_on_stable_external(&cr).as_ref(),
        Some(&cr.join(V2_TMP_FILENAME))
    );
    // 真实 tmp（非 Magic）→ 不得误判（preflight 拒绝路径）
    let cr2 = temp_root("t60b");
    write_compat_guard_g2(&cr2, "op-c2d", &source).unwrap();
    fs::write(cr2.join(V2_TMP_FILENAME), b"real legacy tmp").unwrap();
    put_state(&cr2, &state_active_external(&source));
    assert!(stale_g1_on_stable_external(&cr2).is_none());
    // 有 pending → 不适用
    let cr3 = temp_root("t60c");
    write_compat_guard_g1(&cr3, "op-d2c").unwrap();
    put_state(
        &cr3,
        &state_pending_restore_default(
            &source,
            &cr3,
            "op-d2c",
            "transferring",
            None,
            None,
            &DataRootState::new(Some(source.clone())),
        ),
    );
    assert!(stale_g1_on_stable_external(&cr3).is_none());
}

// 61. C→D→C 全序列（§十七 核心场景）：旧 C archive 绝不当恢复源
#[test]
fn t61_restore_default_full_c_d_c() {
    let cr = temp_root("t61c");
    let d_root = temp_target("t61d");
    // ---- C canonical，第一批数据 ----
    make_valid_v2_db(&cr);
    add_snippet_row(&cr, "batch-1");
    // ---- C→D（复用迁移引擎 + 启动收尾）----
    let src = open_existing_v2_db(&cr.join(V2_DB_FILENAME)).unwrap();
    let fp1 = {
        let conn = src.conn.lock().unwrap();
        semantic_fingerprint(&conn).unwrap()
    };
    preflight_migration_target(&cr, &d_root, 64 * 1024 * 1024).unwrap();
    write_compat_guard_g1(&cr, "op-c2d").unwrap();
    // fresh 环境（无 state）下发起 C→D：retained_sources 为空
    let pending1 = state_pending_migration(
        &cr,
        &d_root,
        "op-c2d",
        "transferring",
        SourceKind::DefaultConfigRoot,
        Some(&fp1),
        None,
        None,
    );
    put_state(&cr, &pending1);
    let tmp1 = migration_tmp_path(&d_root, "op-c2d");
    {
        let conn = src.conn.lock().unwrap();
        snapshot_source_db(&conn, &tmp1).unwrap();
    }
    let tfp1 = verify_migration_target(&tmp1, &fp1).unwrap();
    let verified1 = state_pending_migration(
        &cr,
        &d_root,
        "op-c2d",
        "target_verified",
        SourceKind::DefaultConfigRoot,
        Some(&fp1),
        Some(&tfp1),
        None,
    );
    put_state(&cr, &verified1);
    drop(src);
    activate_target_tmp(&tmp1, &d_root.join(V2_DB_FILENAME)).unwrap();
    let op1 = load(&cr).pending_operation.unwrap();
    match crate::try_complete_pending_migration(&cr, &op1) {
        crate::MigrationResume::Completed(t) => assert_eq!(t, d_root),
        other => panic!("期望 Completed，实际 {:?}", other),
    }
    let old_c_archive = cr.join("drawer-v2.db.migrated-op-c2d");
    assert!(old_c_archive.exists(), "C→D 的归档应保留在 Config Root");
    let s1 = load(&cr);
    assert_eq!(s1.retained_sources.len(), 1, "C→D 提交时应登记 retained");
    assert_eq!(s1.retained_sources[0].op_id, "op-c2d");
    // ---- D 上新增第二批数据 ----
    add_snippet_row(&d_root, "batch-2");
    // ---- D→C restore-default ----
    let src2 = open_existing_v2_db(&d_root.join(V2_DB_FILENAME)).unwrap();
    let fp2 = {
        let conn = src2.conn.lock().unwrap();
        semantic_fingerprint(&conn).unwrap()
    };
    // preflight：已知归档（op-c2d）不阻止恢复默认
    let known = vec!["drawer-v2.db.migrated-op-c2d".to_string()];
    preflight_restore_default(&cr, 64 * 1024 * 1024, &known).unwrap();
    external_guard_consistent(&cr, &d_root).expect("G2 应与 active_root 一致");
    write_compat_guard_g1(&cr, "op-d2c").unwrap();
    let prev = load(&cr);
    put_state(
        &cr,
        &state_pending_restore_default(
            &d_root,
            &cr,
            "op-d2c",
            "transferring",
            Some(&fp2),
            None,
            &prev,
        ),
    );
    let tmp2 = restore_staging_path(&cr, "op-d2c");
    {
        let conn = src2.conn.lock().unwrap();
        snapshot_source_db(&conn, &tmp2).unwrap();
    }
    let tfp2 = verify_migration_target(&tmp2, &fp2).unwrap();
    put_state(
        &cr,
        &state_pending_restore_default(
            &d_root,
            &cr,
            "op-d2c",
            "target_verified",
            Some(&fp2),
            Some(&tfp2),
            &prev,
        ),
    );
    drop(src2);
    activate_target_tmp(&tmp2, &cr.join(V2_DB_FILENAME)).unwrap();
    let op2 = load(&cr).pending_operation.unwrap();
    match crate::try_complete_pending_restore_default(&cr, &op2) {
        crate::MigrationResume::Completed(t) => assert_eq!(t, cr),
        other => panic!("期望 Completed，实际 {:?}", other),
    }
    // ---- 断言 ----
    // 1) 新 C 正式库 = 最新 D 快照（第一批+第二批），绝不是旧 C archive 恢复
    assert_eq!(snippet_count_by_title(&cr, "batch-1"), 1);
    assert_eq!(
        snippet_count_by_title(&cr, "batch-2"),
        1,
        "第二批数据必须来自 D 快照"
    );
    // 2) 旧 C archive 未被覆盖/激活/删除
    assert!(old_c_archive.exists(), "旧 C archive 必须继续保留");
    // 3) D 正式库归档 retained
    assert!(!d_root.join(V2_DB_FILENAME).exists());
    assert!(d_root.join("drawer-v2.db.migrated-op-d2c").exists());
    // 4) state：active_root=null、pending=null、双归档 metadata 可追踪
    let s2 = load(&cr);
    assert!(s2.active_root.is_none());
    assert!(s2.pending_operation.is_none());
    assert_eq!(s2.retained_sources.len(), 2, "两代归档都必须可追踪");
    let ids: Vec<&str> = s2
        .retained_sources
        .iter()
        .map(|r| r.op_id.as_str())
        .collect();
    assert!(ids.contains(&"op-c2d") && ids.contains(&"op-d2c"));
    assert_eq!(s2.last_migration.as_ref().unwrap().op_id, "op-d2c");
    // 5) 无 guard，resolver = UseDefault
    assert!(recognize_guards(&cr).is_empty());
    assert_eq!(resolve_data_root(&cr), Resolution::UseDefault(cr.clone()));
}

// 62. retained archive 不阻断默认态 resolver（state active_root=null + 归档 → UseDefault）
#[test]
fn t62_retained_archives_do_not_block_default() {
    let cr = temp_root("t62");
    make_valid_v2_db(&cr);
    let mut st = DataRootState::new(None);
    st.retained_sources.push(RetainedSource {
        op_id: "op-62".to_string(),
        archive_path: cr.join("drawer-v2.db.migrated-op-62"),
        original_root: cr.clone(),
        migrated_to: temp_target("t62x"),
        created_at: chrono::Local::now().to_rfc3339(),
        status: "retained".to_string(),
        deleted_at: None,
    });
    fs::write(cr.join("drawer-v2.db.migrated-op-62"), b"old archive").unwrap();
    put_state(&cr, &st);
    assert_eq!(resolve_data_root(&cr), Resolution::UseDefault(cr.clone()));
}

// 63. restore-default 激活前崩溃矩阵：transferring 半成 → 回滚 SourceCanonical
#[test]
fn t63_restore_crash_pre_activation() {
    let cr = temp_root("t63c");
    let d_root = temp_target("t63d");
    make_valid_v2_db(&d_root);
    add_snippet_row(&d_root, "batch-1");
    put_state(&cr, &state_active_external(&d_root));
    write_compat_guard_g2(&cr, "op-c2d", &d_root).unwrap();
    // 中断现场：staging 半成 + G1 + state(transferring)
    write_compat_guard_g1(&cr, "op-d2c").unwrap();
    let tmp = restore_staging_path(&cr, "op-d2c");
    fs::write(&tmp, b"half-written").unwrap();
    let prev = load(&cr);
    put_state(
        &cr,
        &state_pending_restore_default(&d_root, &cr, "op-d2c", "transferring", None, None, &prev),
    );
    let op = load(&cr).pending_operation.unwrap();
    match crate::try_complete_pending_restore_default(&cr, &op) {
        crate::MigrationResume::SourceCanonical => {}
        other => panic!("期望 SourceCanonical，实际 {:?}", other),
    }
    assert!(!tmp.exists(), "staging 应回滚清除");
    assert!(!cr.join(V2_TMP_FILENAME).exists(), "G1 应回滚移除");
    assert!(
        recognize_guards(&cr).iter().any(|g| g.op_id == "op-c2d"),
        "旧 G2 必须保留"
    );
    let s = load(&cr);
    assert!(s.pending_operation.is_none());
    assert_eq!(
        s.active_root.as_deref(),
        Some(d_root.as_path()),
        "D 仍 canonical"
    );
    assert!(
        cr.join(V2_DB_FILENAME).exists() == false,
        "不得产生 C 正式库"
    );
    assert_eq!(snippet_count_by_title(&d_root, "batch-1"), 1);
}

// 64. restore-default 激活后崩溃矩阵：target_activated/restart_required → 收尾完成
#[test]
fn t64_restore_crash_post_activation() {
    for phase in ["target_activated", "restart_required", "retiring_source"] {
        let cr = temp_root("t64c");
        let d_root = temp_target("t64d");
        make_valid_v2_db(&d_root);
        add_snippet_row(&d_root, "batch-1");
        put_state(&cr, &state_active_external(&d_root));
        write_compat_guard_g2(&cr, "op-c2d", &d_root).unwrap();
        write_compat_guard_g1(&cr, "op-d2c").unwrap();
        let src = open_existing_v2_db(&d_root.join(V2_DB_FILENAME)).unwrap();
        let fp = {
            let conn = src.conn.lock().unwrap();
            semantic_fingerprint(&conn).unwrap()
        };
        let tmp = restore_staging_path(&cr, "op-d2c");
        {
            let conn = src.conn.lock().unwrap();
            snapshot_source_db(&conn, &tmp).unwrap();
        }
        let tfp = verify_migration_target(&tmp, &fp).unwrap();
        drop(src);
        activate_target_tmp(&tmp, &cr.join(V2_DB_FILENAME)).unwrap();
        let prev = load(&cr);
        put_state(
            &cr,
            &state_pending_restore_default(
                &d_root,
                &cr,
                "op-d2c",
                phase,
                Some(&fp),
                Some(&tfp),
                &prev,
            ),
        );
        let op = load(&cr).pending_operation.unwrap();
        match crate::try_complete_pending_restore_default(&cr, &op) {
            crate::MigrationResume::Completed(t) => assert_eq!(t, cr),
            other => panic!("phase {} 期望 Completed，实际 {:?}", phase, other),
        }
        assert!(
            d_root.join("drawer-v2.db.migrated-op-d2c").exists(),
            "phase {} D 应归档",
            phase
        );
        assert_eq!(snippet_count_by_title(&cr, "batch-1"), 1, "phase {}", phase);
        let s = load(&cr);
        assert!(s.active_root.is_none());
        assert_eq!(s.retained_sources.len(), 1);
        assert!(
            recognize_guards(&cr).is_empty(),
            "phase {} guard 应清理",
            phase
        );
        assert_eq!(resolve_data_root(&cr), Resolution::UseDefault(cr.clone()));
    }
}

// 65. Upgrade Compatibility Gate（2C-3 → 2C-4）：旧 schema state + last_migration + 已有归档
#[test]
fn t65_upgrade_from_2c3_last_migration_preserves_retained_archive() {
    let cr = temp_root("t65c");
    let d_root = temp_target("t65d");
    // ---- External D：当前 canonical（真实 v2 库，第一批数据）----
    make_valid_v2_db(&d_root);
    add_snippet_row(&d_root, "batch-1");
    // ---- 旧 C archive（真实 v2 库改名，模拟 2C-3 时代 C→D 的 retained 产物）----
    make_valid_v2_db(&cr);
    let old_archive = cr.join("drawer-v2.db.migrated-old-op");
    fs::rename(cr.join(V2_DB_FILENAME), &old_archive).unwrap();
    // ---- G2 guard + 2C-3 旧 schema state（无 retained_sources 字段）----
    write_compat_guard_g2(&cr, "op-c2d-old", &d_root).unwrap();
    let legacy = serde_json::json!({
        "version": 1,
        "active_root": d_root.to_string_lossy(),
        "updated_at": chrono::Local::now().to_rfc3339(),
        "last_migration": {
            "source": cr.to_string_lossy(),
            "target": d_root.to_string_lossy(),
            "op_id": "op-c2d-old",
            "archive": "drawer-v2.db.migrated-old-op",
            "completed_at": chrono::Local::now().to_rfc3339(),
        }
    });
    fs::write(
        cr.join(STATE_FILENAME),
        serde_json::to_string(&legacy).unwrap(),
    )
    .unwrap();

    // 1) 旧 state 可正常解析（retained_sources serde default = 空）
    let st = load(&cr);
    assert_eq!(st.active_root.as_deref(), Some(d_root.as_path()));
    assert!(st.pending_operation.is_none());
    assert!(
        st.retained_sources.is_empty(),
        "2C-3 state 无该字段 → default 空"
    );
    assert_eq!(st.last_migration.as_ref().unwrap().op_id, "op-c2d-old");
    // resolver 正常 = UseExternal（不因升级而 Blocked）
    assert_eq!(
        resolve_data_root(&cr),
        Resolution::UseExternal(d_root.clone())
    );

    // 2/3) 已有 archive 不被误判 Unknown Archive：known 列表从可信 last_migration 构建
    let known = vec!["drawer-v2.db.migrated-old-op".to_string()];
    preflight_restore_default(&cr, 64 * 1024 * 1024, &known)
        .expect("last_migration 登记的 archive 必须被识别为已知");
    // metadata 与磁盘一致 → Gate 通过
    verify_retained_metadata_consistency(&st).expect("一致状态必须通过 Upgrade Gate");

    // 7) metadata 与磁盘不一致 → Fail Closed（登记的归档消失且无 removed 记录解释）
    let backup_bytes = fs::read(&old_archive).unwrap();
    fs::remove_file(&old_archive).unwrap();
    assert!(
        verify_retained_metadata_consistency(&st).is_err(),
        "last_migration 与实际 archive 不一致必须 Fail Closed"
    );
    // 恢复现场，继续 D→C
    fs::write(&old_archive, &backup_bytes).unwrap();
    assert!(verify_retained_metadata_consistency(&st).is_ok());

    // 4) 用户继续执行 D→C Restore Default（完整 begin + 启动收尾序列）
    let src = open_existing_v2_db(&d_root.join(V2_DB_FILENAME)).unwrap();
    let fp = {
        let conn = src.conn.lock().unwrap();
        semantic_fingerprint(&conn).unwrap()
    };
    write_compat_guard_g1(&cr, "op-d2c").unwrap();
    put_state(
        &cr,
        &state_pending_restore_default(
            &d_root,
            &cr,
            "op-d2c",
            "transferring",
            Some(&fp),
            None,
            &st,
        ),
    );
    let tmp = restore_staging_path(&cr, "op-d2c");
    {
        let conn = src.conn.lock().unwrap();
        snapshot_source_db(&conn, &tmp).unwrap();
    }
    let tfp = verify_migration_target(&tmp, &fp).unwrap();
    put_state(
        &cr,
        &state_pending_restore_default(
            &d_root,
            &cr,
            "op-d2c",
            "target_verified",
            Some(&fp),
            Some(&tfp),
            &st,
        ),
    );
    drop(src);
    activate_target_tmp(&tmp, &cr.join(V2_DB_FILENAME)).unwrap();
    let op = load(&cr).pending_operation.unwrap();
    match crate::try_complete_pending_restore_default(&cr, &op) {
        crate::MigrationResume::Completed(t) => assert_eq!(t, cr),
        other => panic!("期望 Completed，实际 {:?}", other),
    }

    // 5) 完成后：旧 C archive 仍 retained；新 D archive 也 retained；两代都可管理
    assert!(old_archive.exists(), "旧 C archive 不得被覆盖/激活/删除");
    assert!(
        d_root.join("drawer-v2.db.migrated-op-d2c").exists(),
        "D 侧归档必须 retained"
    );
    assert!(!d_root.join(V2_DB_FILENAME).exists());
    assert_eq!(
        snippet_count_by_title(&cr, "batch-1"),
        1,
        "C 正式库 = 最新 D 快照"
    );
    let s2 = load(&cr);
    assert!(s2.active_root.is_none());
    assert!(s2.pending_operation.is_none());
    assert_eq!(
        s2.retained_sources.len(),
        2,
        "两代 archive 都必须进入 retained 管理"
    );
    let ids: Vec<&str> = s2
        .retained_sources
        .iter()
        .map(|r| r.op_id.as_str())
        .collect();
    assert!(
        ids.contains(&"op-c2d-old"),
        "旧 C archive 从 last_migration 折算登记"
    );
    assert!(ids.contains(&"op-d2c"), "新 D archive 登记");
    for r in &s2.retained_sources {
        assert_eq!(r.status, "retained");
    }
    // 管理可用性：删除校验路径对两代登记均能定位（以旧 archive 为例做白名单校验前置判定）
    assert!(old_archive
        .file_name()
        .unwrap()
        .to_string_lossy()
        .starts_with("drawer-v2.db.migrated-"));
    // 6) 无 guard，resolver = UseDefault
    assert!(recognize_guards(&cr).is_empty());
    assert_eq!(resolve_data_root(&cr), Resolution::UseDefault(cr.clone()));
}

// 66. 首次启动备份恢复（default 目标）：staging → 激活 → 正式库带备份身份，无 state/guard
#[test]
fn t66_first_run_backup_restore_default() {
    use crate::backup_v2::{export_v2_to_path, prepare_restored_staging};
    use zeroize::Zeroizing;

    let cr = temp_root("t66c");
    // 源：一台"旧机器"的库（真实 AppState 路径）
    let src_dir = temp_target("t66-src");
    let prepared = prepare_fresh_v2(&src_dir, "Old-Master-Pw!").unwrap();
    let words = prepared.mnemonic.clone();
    let dek = prepared.dek.clone();
    finalize_prepared_v2(&prepared.tmp_path, &prepared.v2_path).unwrap();
    drop(prepared.setup_lock);
    let _ = fs::remove_file(src_dir.join(V2_SETUP_LOCK_FILENAME));
    let db = open_existing_v2_db(&src_dir.join(V2_DB_FILENAME)).unwrap();
    let state = crate::AppState::new(db, src_dir.join(V2_DB_FILENAME), crate::StartupMode::ExistingV2);
    state.set_stable_dek(Zeroizing::new(dek.to_vec()));
    let backup_path = src_dir.join("machine-a.drawerbox");
    export_v2_to_path(&state, "Old-Master-Pw!", &backup_path).unwrap();

    // 新机器空环境：默认目标 staging → 激活
    let staging = cr.join("drawer-v2.db.restore-op66.tmp");
    let stats = prepare_restored_staging(&backup_path, "Old-Master-Pw!", &staging).unwrap();
    assert_eq!(stats.settings, 5, "含安全元数据 rows");
    let formal = cr.join(V2_DB_FILENAME);
    assert!(!formal.exists());
    activate_staging_no_replace(&staging, &formal).unwrap();
    assert!(!staging.exists() && formal.exists());

    // 正式库 = 备份身份（旧主密码可解锁、恢复词有效、新机器无主密码概念）
    let restored = open_existing_v2_db(&formal).unwrap();
    assert!(crate::migration::unlock_v2_core(&restored, "Old-Master-Pw!").is_ok());
    assert!(crate::migration::recover_v2_core(&restored, &words.join(" ")).is_ok());
    drop(restored);
    // 无 state / 无 guard（与推荐位置语义一致）
    assert_eq!(resolve_data_root(&cr), Resolution::UseDefault(cr.clone()));
    assert!(recognize_guards(&cr).is_empty());
    assert!(!cr.join(STATE_FILENAME).exists());
    // 激活绝不覆盖已存在正式库
    let staging2 = cr.join("drawer-v2.db.restore-op66b.tmp");
    fs::write(&staging2, b"x").unwrap();
    assert!(activate_staging_no_replace(&staging2, &formal).is_err(), "目标已存在必须 fail closed");
}

// 67. 首次启动备份恢复（external 目标）：guard/state/staging 顺序 + 重启收尾提交 active_root
#[test]
fn t67_first_run_backup_restore_external() {
    use crate::backup_v2::{export_v2_to_path, prepare_restored_staging};
    use zeroize::Zeroizing;

    let cr = temp_root("t67c");
    let src_dir = temp_target("t67-src");
    let prepared = prepare_fresh_v2(&src_dir, "Old-Master-Pw!").unwrap();
    finalize_prepared_v2(&prepared.tmp_path, &prepared.v2_path).unwrap();
    drop(prepared.setup_lock);
    let _ = fs::remove_file(src_dir.join(V2_SETUP_LOCK_FILENAME));
    let db = open_existing_v2_db(&src_dir.join(V2_DB_FILENAME)).unwrap();
    let state = crate::AppState::new(db, src_dir.join(V2_DB_FILENAME), crate::StartupMode::ExistingV2);
    state.set_stable_dek(Zeroizing::new(prepared.dek.to_vec()));
    let backup_path = src_dir.join("t67.drawerbox");
    export_v2_to_path(&state, "Old-Master-Pw!", &backup_path).unwrap();

    let target = temp_target("t67-d");
    preflight_new_root(&cr, &target).unwrap();
    // Guard First：G2 durable → state(restore_backup/preparing)
    write_compat_guard_g2(&cr, "op-rb67", &target).unwrap();
    put_state(&cr, &state_pending("restore_backup", "op-rb67", "preparing", &target));
    // staging：migration-<op_id>.tmp（绝不占用 G1 文件名）
    let staging = migration_tmp_path(&target, "op-rb67");
    prepare_restored_staging(&backup_path, "Old-Master-Pw!", &staging).unwrap();
    assert_ne!(staging, cr.join(V2_TMP_FILENAME));
    put_state(&cr, &state_pending("restore_backup", "op-rb67", "activated", &target));
    activate_staging_no_replace(&staging, &target.join(V2_DB_FILENAME)).unwrap();

    // 重启收尾：Pending(restore_backup/activated) → 复用 init 收尾提交 active_root
    let op = load(&cr).pending_operation.unwrap();
    assert_eq!(crate::try_complete_pending_init(&cr, &op).as_deref(), Some(target.as_path()));
    assert_eq!(resolve_data_root(&cr), Resolution::UseExternal(target.clone()));
    assert!(recognize_guards(&cr).iter().any(|g| g.op_id == "op-rb67"), "G2 必须保留");
    assert!(target.join(V2_DB_FILENAME).exists());
    assert!(!cr.join(V2_DB_FILENAME).exists(), "Config Root 绝不能出现正式库");
}

// 68. Factory Reset v2 finalization（external intent）：canonical 清理 + retained 保留
#[test]
fn t68_factory_reset_v2_external() {
    let cr = temp_root("t68c");
    let d_root = temp_target("t68d");
    let e_root = temp_target("t68e"); // retained archive 所在的其他位置
    // D canonical：正式库 + 图标缓存目录
    make_valid_v2_db(&d_root);
    fs::create_dir_all(d_root.join("icons")).unwrap();
    fs::write(d_root.join("icons").join("app.ico"), b"icon").unwrap();
    // state：active_root=D + retained 指向 E（他盘安全副本）
    let mut st = crate::data_root::DataRootState::new(Some(d_root.clone()));
    st.retained_sources.push(RetainedSource {
        op_id: "op-ret".to_string(),
        archive_path: e_root.join("drawer-v2.db.migrated-op-ret"),
        original_root: e_root.clone(),
        migrated_to: d_root.clone(),
        created_at: chrono::Local::now().to_rfc3339(),
        status: "retained".to_string(),
        deleted_at: None,
    });
    put_state(&cr, &st);
    write_compat_guard_g2(&cr, "op-g2", &d_root).unwrap();
    fs::write(e_root.join("drawer-v2.db.migrated-op-ret"), b"retained copy").unwrap();
    // durable intent
    let flag = cr.join(".factory_reset_pending");
    let intent = serde_json::json!({"v2": true, "active_root": d_root.to_string_lossy(), "generated_at": "x"});
    fs::write(&flag, serde_json::to_string(&intent).unwrap()).unwrap();

    let n = crate::data_root::run_v2_factory_reset_finalization(&cr, &flag).unwrap();
    assert!(n >= 4);
    // D canonical DB + icons 已删
    assert!(!d_root.join(V2_DB_FILENAME).exists());
    assert!(!d_root.join("icons").exists());
    // state / guards 已清
    assert!(!cr.join(STATE_FILENAME).exists());
    assert!(!cr.join(crate::data_root::BAK_FILENAME).exists());
    assert!(recognize_guards(&cr).is_empty());
    // flag 已删
    assert!(!flag.exists());
    // retained archive 完好（他盘绝不被触碰）
    assert!(e_root.join("drawer-v2.db.migrated-op-ret").exists(), "retained archive 绝不能因 reset 被删");
    // D 根目录本身保留（目录内其他用户文件不动）
    assert!(d_root.exists());
    // 之后解析 → 空环境 → Choose
    assert_eq!(resolve_data_root(&cr), Resolution::UseDefault(cr.clone()));
    assert!(matches!(
        crate::migration::resolve_startup_db(&cr, true).selection,
        crate::migration::DbSelection::FreshV2(_)
    ));
}

// 69. Factory Reset v2 finalization 幂等：intent 消失后再跑 = 0
#[test]
fn t69_factory_reset_finalization_idempotent() {
    let cr = temp_root("t69");
    let flag = cr.join(".factory_reset_pending");
    assert_eq!(crate::data_root::run_v2_factory_reset_finalization(&cr, &flag).unwrap(), 0, "无 intent = 无操作");
    fs::write(&flag, "reset
").unwrap(); // legacy 文本 intent：v2 finalization 不碰
    assert_eq!(crate::data_root::run_v2_factory_reset_finalization(&cr, &flag).unwrap(), 0, "legacy intent 由 legacy 路径处理");
    assert!(flag.exists(), "legacy intent 不得被 v2 finalization 删除");
}
// 70. Runtime fixture：用真实 export 实现生成 .drawerbox 备份文件到固定路径，
//     供 Runtime 测试（B1/B2 导入、B3/B4 首次恢复）使用
#[test]
fn t70_write_runtime_fixture_backup() {
    use crate::backup_v2::export_v2_to_path;
    use zeroize::Zeroizing;

    let out_dir = Path::new("D:/Drawer-2C5-Harness");
    let _ = fs::create_dir_all(out_dir);
    let backup_path = out_dir.join("runtime-fixture.drawerbox");

    let src_dir = temp_target("t70-src");
    let prepared = prepare_fresh_v2(&src_dir, "2C5-Fixture-Master!").unwrap();
    let dek = prepared.dek.clone();
    finalize_prepared_v2(&prepared.tmp_path, &prepared.v2_path).unwrap();
    drop(prepared.setup_lock);
    let _ = fs::remove_file(src_dir.join(V2_SETUP_LOCK_FILENAME));
    let db = open_existing_v2_db(&src_dir.join(V2_DB_FILENAME)).unwrap();
    let state = crate::AppState::new(db, src_dir.join(V2_DB_FILENAME), crate::StartupMode::ExistingV2);
    state.set_stable_dek(Zeroizing::new(dek.to_vec()));
    // 真实业务数据（可通过解锁后 UI 验证）
    {
        let db = state.db.lock().unwrap();
        let conn = db.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO temp_contents (text,created_at,expires_at,deleted_at) VALUES ('2C5-Fixture-Temp-Content',1,99999999,NULL)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO snippets (title,content,language,tags,created_at,updated_at,use_count,last_used_at,deleted_at) VALUES ('2C5-Fixture-Snippet','body','text','',1,2,3,4,NULL)",
            [],
        )
        .unwrap();
    }
    let stats = export_v2_to_path(&state, "2C5-Fixture-Master!", &backup_path).unwrap();
    assert_eq!(stats.temps, 1);
    assert_eq!(stats.snippets, 1);
    assert!(backup_path.exists(), "runtime fixture backup 必须生成");
    drop(state);
    let _ = fs::remove_dir_all(&src_dir);
}




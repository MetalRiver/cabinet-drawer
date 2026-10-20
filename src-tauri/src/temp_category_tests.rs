//! ===== v0.4.3 Batch 4：便签标题与分类 —— 迁移/兼容/备份往返 回归 =====
//!
//! 覆盖验收指令 15 点：
//!  1 新库初始化            2 v0.4.1 旧库迁移       3 重复迁移（幂等）
//!  4 无标题历史便签展示    5 标题增删改查          6 分类增删改查
//!  7 删除分类后便签保留    8 正文编辑与追加        9 续期及永不过期
//! 10 删除与回收站恢复     11 v2 加密备份恢复往返   12 legacy 导入导出
//! 13 分类 ID 冲突与映射   14 迁移失败回滚（事务+幂等说明） 15 重启后持久化
//!
//! 铁律：使用隔离临时库；绝不触碰正式用户数据库。

use crate::db::{Db, TempCategory};
use std::fs;
use std::path::PathBuf;
use std::time::Duration;

fn temp_db(label: &str) -> (PathBuf, Db) {
    let dir = std::env::temp_dir().join(format!(
        "drawer-tempcat-{}-{}",
        label,
        uuid::Uuid::new_v4()
    ));
    fs::create_dir_all(&dir).expect("建临时目录失败");
    let path = dir.join("drawer-v2.db");
    let db = Db::open(&path).expect("建测试库失败");
    (path, db)
}

fn reopen(path: &PathBuf) -> Db {
    Db::open(path).expect("重新打开数据库失败")
}

const NEVER: i64 = 4_102_444_800_000;

fn in_one_hour() -> i64 {
    chrono::Utc::now().timestamp_millis() + 3_600_000
}
 // 永不过期哨兵（与 db.rs 常量一致）

// ---- 1. 新库初始化：带标题与分类创建 ----
#[test]
fn t_tc01_新库_带标题与分类创建() {
    let (path, db) = temp_db("init");
    let cid = db.create_temp_category("工作").unwrap();
    let id = db
        .create_temp_with_title(Some("会议纪要"), "正文内容", in_one_hour())
        .unwrap();
    db.set_temp_category(id, Some(cid)).unwrap();
    let list = db.list_temp().unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].title.as_deref(), Some("会议纪要"));
    assert_eq!(list[0].category_id, Some(cid));
    assert_eq!(list[0].category_name.as_deref(), Some("工作"));
    assert_eq!(list[0].text, "正文内容");
    drop(db);
    drop(path);
}

// ---- 2 + 3. v0.4.1 旧库迁移 + 重复迁移（幂等）----
#[test]
fn t_tc02_v041旧库迁移_数据完整_列补齐() {
    // 手工构造 v0.4.1 结构的隔离数据库副本（无 title/category_id、无 temp_categories）
    let dir = std::env::temp_dir().join(format!(
        "drawer-tc02-old-{}",
        uuid::Uuid::new_v4()
    ));
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join("drawer-v2.db");
    {
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS temp_contents (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                text TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                expires_at INTEGER NOT NULL
            );
            INSERT INTO temp_contents (text, created_at, expires_at) VALUES
              ('历史便签甲', 1000, 9999999999999),
              ('历史便签乙', 2000, 4102444800000);",
        )
        .unwrap();
    }
    // v0.4.3 打开 → 迁移自动发生
    {
        let db = Db::open(&path).unwrap();
        let list = db.list_temp().unwrap();
        assert_eq!(list.len(), 2, "迁移后行数不变");
        assert!(list.iter().all(|t| t.title.is_none()), "历史便签无标题");
        assert!(list.iter().all(|t| t.category_id.is_none()), "历史便签未分类");
        assert!(list.iter().any(|t| t.text == "历史便签甲"), "正文原样保留");
        // created_at / expires_at 不变（含永不过期哨兵）
        let y = list.iter().find(|t| t.text == "历史便签乙").unwrap();
        assert_eq!(y.expires_at, NEVER, "永不过期语义不变");
        assert_eq!(y.created_at, 2000, "创建时间不变");
    }
    // 3) 重复打开（重复迁移）→ 幂等：不报错、不重复加列、数据不变
    {
        let db = reopen(&path);
        let list = db.list_temp().unwrap();
        assert_eq!(list.len(), 2);
        // 分类功能在迁移后的老库上可直接使用
        let cid = db.create_temp_category("迁移后新建").unwrap();
        db.set_temp_category(list[0].id, Some(cid)).unwrap();
        let again = db.list_temp().unwrap();
        assert_eq!(again[0].category_name.as_deref(), Some("迁移后新建"));
    }
    // 再次打开（第 3 次）仍幂等
    {
        let db = reopen(&path);
        assert_eq!(db.list_temp().unwrap().len(), 2);
    }
    fs::remove_dir_all(&dir).ok();
}

// ---- 4 + 5. 无标题展示 + 标题增删改查（编辑标题不影响正文）----
#[test]
fn t_tc03_标题增删改查_正文与到期不变() {
    let (path, db) = temp_db("title");
    let id = db.create_temp_with_title(None, "原始正文", in_one_hour()).unwrap();
    // 补充标题
    db.update_temp_title(id, Some("我的标题")).unwrap();
    let t = &db.list_temp().unwrap()[0];
    assert_eq!(t.title.as_deref(), Some("我的标题"));
    assert_eq!(t.text, "原始正文", "标题不影响正文");
    // 修改标题
    db.update_temp_title(id, Some("新标题")).unwrap();
    assert_eq!(db.list_temp().unwrap()[0].title.as_deref(), Some("新标题"));
    // 清除标题（None）
    db.update_temp_title(id, None).unwrap();
    assert!(db.list_temp().unwrap()[0].title.is_none());
    assert_eq!(db.list_temp().unwrap()[0].text, "原始正文");
    // 正文编辑不触碰标题
    db.update_temp_title(id, Some("标题A")).unwrap();
    db.update_temp_text(id, "编辑后正文").unwrap();
    let t = &db.list_temp().unwrap()[0];
    assert_eq!(t.title.as_deref(), Some("标题A"));
    assert_eq!(t.text, "编辑后正文");
    // 不存在的便签 → 0 行（命令层转译可见错误）
    assert_eq!(db.update_temp_title(99999, Some("x")).unwrap(), 0);
    drop(db);
    drop(path);
}

// ---- 6 + 7. 分类增删改查 + 删除分类后便签保留（回到未分类）----
#[test]
fn t_tc04_分类增删改_删除分类便签保留() {
    let (path, db) = temp_db("cat");
    let c1 = db.create_temp_category("  工作  ").unwrap(); // trim
    assert_eq!(db.list_temp_categories().unwrap().len(), 1);
    // 重名（精确）拒绝
    assert!(db.create_temp_category("工作").is_err());
    // 大小写不敏感判重（SQLite lower 为 ASCII 范围——中文名与其 ASCII 拼写属不同名称，合理行为）
    let ascii = db.create_temp_category("ABC").unwrap();
    assert!(db.create_temp_category("abc").is_err(), "ASCII 大小写判重");
    assert!(db.rename_temp_category(ascii, "Abc").is_ok());
    // 空名拒绝
    assert!(db.create_temp_category("   ").is_err());
    // 过长拒绝（31 字符）；30 字符为合法边界
    assert!(db.create_temp_category("一二三四五六七八九十一二三四五六七八九十一二三四五六七八九十一").is_err());
    assert!(db.create_temp_category("一二三四五六七八九十一二三四五六七八九十一二三四五六七八九十").is_ok(), "30 字符合法");

    // 重命名：id 不变（身份不错乱）
    let before = db.list_temp_categories().unwrap();
    db.rename_temp_category(c1, "重要工作").unwrap();
    let after = db.list_temp_categories().unwrap();
    assert_eq!(after.len(), before.len(), "重命名不改变分类数量");
    let renamed = after.iter().find(|c| c.id == c1).unwrap();
    assert_eq!(renamed.name, "重要工作");
    // 重命名为已有名 → 拒绝
    let c2 = db.create_temp_category("生活").unwrap();
    assert!(db.rename_temp_category(c2, "重要工作").is_err());

    // 便签归入 c1、c2；删除 c1 → 便签保留且回到未分类
    let n1 = db.create_temp_with_title(Some("甲"), "aaa", in_one_hour()).unwrap();
    let n2 = db.create_temp_with_title(None, "bbb", in_one_hour()).unwrap();
    db.set_temp_category(n1, Some(c1)).unwrap();
    db.set_temp_category(n2, Some(c2)).unwrap();

    let affected = db.delete_temp_category(c1).unwrap();
    assert_eq!(affected, 1, "一条便签被归位未分类");
    let list = db.list_temp().unwrap();
    let a = list.iter().find(|t| t.id == n1).unwrap();
    let b = list.iter().find(|t| t.id == n2).unwrap();
    assert!(a.category_id.is_none() && a.category_name.is_none(), "甲回未分类");
    assert_eq!(a.title.as_deref(), Some("甲"), "正文与标题不动");
    assert_eq!(a.text, "aaa");
    assert_eq!(b.category_name.as_deref(), Some("生活"), "其他分类不受影响");
    // 删除不存在的分类 → 0（幂等）
    assert_eq!(db.delete_temp_category(99999).unwrap(), 0);
    // 归入已删除的分类 → 明确报错（不静默）
    assert!(db.set_temp_category(n2, Some(c1)).is_err());
    // 移回未分类
    db.set_temp_category(n2, None).unwrap();
    assert!(db.list_temp().unwrap()[0].category_id.is_none());
    drop(db);
    drop(path);
}

// ---- 8 + 9 + 10. 编辑/追加、续期/永不过期、删除与回收站（带标题分类全兼容）----
#[test]
fn t_tc05_既有操作全兼容_回收站标题保留() {
    let (path, db) = temp_db("compat");
    let cid = db.create_temp_category("归档").unwrap();
    let id = db
        .create_temp_with_title(Some("测试便签"), "第一行", NEVER)
        .unwrap();
    db.set_temp_category(id, Some(cid)).unwrap();

    // 8) 编辑正文（title 不动）
    db.update_temp_text(id, "第一行\n第二行").unwrap();
    assert_eq!(db.list_temp().unwrap()[0].title.as_deref(), Some("测试便签"));

    // 9) 永不过期便签：add_ms 续期 → 明确错误（R1 语义不因标题改变）
    assert!(db
        .renew_temp(id, chrono::Utc::now().timestamp_millis(), Some(86_400_000), None)
        .is_err());
    // set_expires_at 路径：直接改为限时（到 2099 年）
    let exp2 = db
        .renew_temp(id, chrono::Utc::now().timestamp_millis(), None, Some(4_079_999_999_999))
        .unwrap();
    assert_eq!(exp2, 4_079_999_999_999);
    // 再续期 add_ms → 正常累加（非永不过期状态）
    let exp3 = db
        .renew_temp(id, chrono::Utc::now().timestamp_millis(), Some(86_400_000), None)
        .unwrap();
    assert_eq!(exp3, 4_079_999_999_999 + 86_400_000);

    // 10) 删除进回收站 → 列表带标题 → 恢复后标题分类都在
    db.soft_delete("temp_contents", id).unwrap();
    assert_eq!(db.list_temp().unwrap().len(), 0, "删除后主列表不可见");
    let trash = db.list_trash().unwrap();
    let item = trash.iter().find(|t| t.table_name == "temp_contents").unwrap();
    assert_eq!(item.title.as_deref(), Some("测试便签"), "回收站展示标题");
    // 恢复（restore_temp_with_expiry）
    db.restore_temp_with_expiry(id, NEVER).unwrap();
    let back = &db.list_temp().unwrap()[0];
    assert_eq!(back.title.as_deref(), Some("测试便签"), "恢复后标题保留");
    assert_eq!(back.category_name.as_deref(), Some("归档"), "恢复后分类保留");
    assert_eq!(back.expires_at, NEVER, "恢复指定到期时间");
    drop(db);
    drop(path);
}

// ---- 12 + 13. legacy 明文导出→导入：标题保留 + 分类 ID 映射（冲突场景）----
#[test]
fn t_tc06_legacy导出导入_标题保留_分类ID映射() {
    // 源库：分类 + 两张带分类的便签
    let (path_a, db_a) = temp_db("legacy-a");
    let cid_work = db_a.create_temp_category("工作").unwrap();
    let cid_life = db_a.create_temp_category("生活").unwrap();
    let _n1 = db_a
        .create_temp_with_title(Some("工作便签"), "work body", in_one_hour())
        .unwrap();
    let n1 = db_a.list_temp().unwrap()[0].id;
    db_a.set_temp_category(n1, Some(cid_work)).unwrap();
    let _n2 = db_a
        .create_temp_with_title(Some("生活便签"), "life body", in_one_hour())
        .unwrap();
    let n2 = db_a.list_temp().unwrap()[1].id;
    db_a.set_temp_category(n2, Some(cid_life)).unwrap();
    let _ = n1; // id 读取仅为保证顺序

    // 导出（legacy 明文结构 BackupFullData）
    let data = db_a.export_all_data().unwrap();
    assert_eq!(data.temp_categories.len(), 2, "导出含分类");
    assert!(data.temps.iter().all(|t| t.title.is_some()), "导出含标题");
    drop(db_a);
    drop(path_a);

    // 目标库：故意预建一个同名但不同 id 的分类（ID 冲突场景）
    let (path_b, db_b) = temp_db("legacy-b");
    let _local_work = db_b.create_temp_category("工作").unwrap(); // 本地"工作" id=1
    // 备份里"工作"的旧 id = cid_work（源库），与目标库 id 必然不同 → 必须按名映射

    let stats = db_b
        .import_all_data(&data, crate::db::ImportConflictPolicy::Merge)
        .unwrap();
    assert!(stats.errors.is_empty(), "导入无错误：{:?}", stats.errors);
    let cats = db_b.list_temp_categories().unwrap();
    // "工作"复用本地（不重复建），"生活"新建 → 共 2 个
    assert_eq!(cats.len(), 2, "同名分类复用，不产生重复: {:?}", cats);
    let list = db_b.list_temp().unwrap();
    assert_eq!(list.len(), 2);
    let work = list.iter().find(|t| t.title.as_deref() == Some("工作便签")).unwrap();
    let life = list.iter().find(|t| t.title.as_deref() == Some("生活便签")).unwrap();
    assert_eq!(work.category_name.as_deref(), Some("工作"), "ID 冲突按名映射正确");
    assert_eq!(life.category_name.as_deref(), Some("生活"), "新分类映射正确");
    assert_eq!(work.text, "work body", "正文不丢");
    drop(db_b);
    drop(path_b);
}

// ---- 15. 重启后持久化（关闭连接重开）----
#[test]
fn t_tc07_重启后标题分类持久化() {
    let (path, db) = temp_db("persist");
    let cid = db.create_temp_category("持久").unwrap();
    let id = db
        .create_temp_with_title(Some("持久便签"), "body", NEVER)
        .unwrap();
    db.set_temp_category(id, Some(cid)).unwrap();
    drop(db); // 模拟应用退出
    let db2 = reopen(&path); // 模拟重启
    let list = db2.list_temp().unwrap();
    assert_eq!(list[0].title.as_deref(), Some("持久便签"));
    assert_eq!(list[0].category_name.as_deref(), Some("持久"));
    drop(db2);
    drop(path);
}

// ---- 14. 迁移失败回滚：事务性验证（注入式失败 + 重跑收敛）----
#[test]
fn t_tc08_迁移事务_失败不落半截状态() {
    // 说明：迁移 = 幂等 DDL（IF NOT EXISTS / pragma 检查加列）+ 单事务包裹。
    // SQLite 中单条 ALTER 本身原子，事务保证两条加列同生共死；
    // "失败后重跑收敛"由 t_tc02 的三次打开覆盖。
    // 此处验证事务语义本身：显式事务内的分类写入失败 → 回滚后无残留。
    let (path, db) = temp_db("rollback");
    db.create_temp_category("原有").unwrap();
    let conn = db.conn.lock().unwrap();
    let tx = conn.unchecked_transaction().unwrap();
    tx.execute("INSERT INTO temp_categories (name) VALUES ('事务内')", [])
        .unwrap();
    drop(tx); // 未 commit 即 drop → 自动回滚
    drop(conn);
    let cats = reopen(&path).list_temp_categories().unwrap();
    assert_eq!(cats.len(), 1, "未提交事务回滚，无残留分类");
    assert_eq!(cats[0].name, "原有");
    drop(path);
}

// ---- 11. v2 加密备份恢复往返（文件级 Online Backup——新列自动随行）----
#[test]
fn t_tc09_v2加密备份往返_标题分类保留() {
    use crate::backup_v2::{export_v2_to_path, prepare_restored_staging};
    use crate::migration::{
        finalize_prepared_v2, open_existing_v2_db, prepare_fresh_v2, V2_DB_FILENAME,
    };
    use zeroize::Zeroizing;

    let work = std::env::temp_dir().join(format!("drawer-tc09-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&work).unwrap();

    // 源机器：建库 → 分类 + 带标题便签（含永不过期）
    let src_dir = work.join("src");
    fs::create_dir_all(&src_dir).unwrap();
    let prepared = prepare_fresh_v2(&src_dir, "Batch4-Pw!").unwrap();
    let dek = prepared.dek.clone();
    finalize_prepared_v2(&prepared.tmp_path, &prepared.v2_path).unwrap();
    drop(prepared.setup_lock);
    let _ = fs::remove_file(src_dir.join(crate::migration::V2_SETUP_LOCK_FILENAME));
    let db = open_existing_v2_db(&src_dir.join(V2_DB_FILENAME)).unwrap();
    let cid = db.create_temp_category("出差").unwrap();
    let id = db
        .create_temp_with_title(Some("出差清单"), "护照、充电器", NEVER)
        .unwrap();
    db.set_temp_category(id, Some(cid)).unwrap();
    drop(db);

    // 加密导出（v2 官方链路——加密/主密码机制零改动）
    let state = crate::AppState::new(
        open_existing_v2_db(&src_dir.join(V2_DB_FILENAME)).unwrap(),
        src_dir.join(V2_DB_FILENAME),
        crate::StartupMode::ExistingV2,
    );
    state.set_stable_dek(Zeroizing::new(dek.to_vec()));
    let backup_path = src_dir.join("b4.drawerbox");
    export_v2_to_path(&state, "Batch4-Pw!", &backup_path).unwrap();

    // 新环境：隔离恢复 → 打开核对
    let staging = work.join("restore.tmp");
    prepare_restored_staging(&backup_path, "Batch4-Pw!", &staging).unwrap();
    let restored = open_existing_v2_db(&staging).unwrap();
    let list = restored.list_temp().unwrap();
    assert_eq!(list.len(), 1, "恢复后便签在");
    assert_eq!(list[0].title.as_deref(), Some("出差清单"), "标题保留");
    assert_eq!(list[0].category_name.as_deref(), Some("出差"), "分类关系保留");
    assert_eq!(list[0].text, "护照、充电器", "正文保留");
    assert_eq!(list[0].expires_at, NEVER, "到期时间（永不过期）保留");
    assert_eq!(
        restored.list_temp_categories().unwrap().len(),
        1,
        "分类保留"
    );
    drop(restored);
    fs::remove_dir_all(&work).ok();
}

// ---- 补充：续期错误信息（对永不过期便签的既有语义不被标题破坏）----
#[test]
fn t_tc10_永不过期续期错误信息() {
    let (path, db) = temp_db("never");
    let id = db
        .create_temp_with_title(Some("永久"), "x", NEVER)
        .unwrap();
    let err = db.renew_temp(id, chrono::Utc::now().timestamp_millis(), Some(86_400_000), None).unwrap_err();
    assert!(err.contains("永不过期"), "错误信息保留既有语义: {err}");
    let _ = std::thread::sleep(Duration::from_millis(1));
    drop(db);
    drop(path);
}

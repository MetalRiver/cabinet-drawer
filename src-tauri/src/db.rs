use rusqlite::{params, Connection, OptionalExtension, Result};

use std::path::Path;
use std::sync::Mutex;

use crate::crypto;

/// 全局数据库连接（使用 Mutex 包裹以保证线程安全）
pub struct Db {
    pub conn: Mutex<Connection>,
}

impl Db {
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)?;
        let db = Db {
            conn: Mutex::new(conn),
        };
        db.init_tables()?;
        db.run_migrations()?;
        Ok(db)
    }

    /// 打开既有数据库的只读视图，不执行建表、补列或任何兼容迁移。
    /// legacy → v2 迁移必须使用该入口，保证源库不会被读取流程改写。
    pub fn open_read_only(path: &Path) -> Result<Self> {
        let conn = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        conn.pragma_update(None, "query_only", true)?;
        Ok(Db {
            conn: Mutex::new(conn),
        })
    }

    fn init_tables(&self) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS settings (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS passwords (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                title TEXT NOT NULL,
                username TEXT,
                password TEXT NOT NULL,
                url TEXT,
                notes TEXT,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS app_categories (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT NOT NULL,
                icon TEXT,
                sort_order INTEGER DEFAULT 0
            );

            CREATE TABLE IF NOT EXISTS apps (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT NOT NULL,
                path TEXT NOT NULL,
                icon_path TEXT,
                args TEXT,
                category_id INTEGER,
                created_at INTEGER NOT NULL,
                use_count INTEGER NOT NULL DEFAULT 0,
                last_used_at INTEGER NOT NULL DEFAULT 0,
                FOREIGN KEY (category_id) REFERENCES app_categories(id)
            );

            CREATE TABLE IF NOT EXISTS snippets (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                title TEXT NOT NULL,
                content TEXT NOT NULL,
                language TEXT DEFAULT 'text',
                tags TEXT,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                use_count INTEGER NOT NULL DEFAULT 0,
                last_used_at INTEGER NOT NULL DEFAULT 0
            );

            CREATE TABLE IF NOT EXISTS temp_contents (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                text TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                expires_at INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS pinned_items (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                item_type TEXT NOT NULL,
                item_id INTEGER NOT NULL,
                sort_order INTEGER DEFAULT 0,
                UNIQUE(item_type, item_id)
            );

            -- 默认分区
            INSERT OR IGNORE INTO app_categories (id, name, icon, sort_order) VALUES
                (1, '开发工具', '⚙️', 1),
                (2, '设计软件', '🎨', 2),
                (3, '日常办公', '📊', 3);
            "#,
        )?;
        // 兼容旧库：apps / snippets 表若缺列则补上
        Self::migrate_add_column(&conn, "apps", "use_count", "INTEGER NOT NULL DEFAULT 0")?;
        Self::migrate_add_column(&conn, "apps", "last_used_at", "INTEGER NOT NULL DEFAULT 0")?;
        Self::migrate_add_column(&conn, "snippets", "use_count", "INTEGER NOT NULL DEFAULT 0")?;
        Self::migrate_add_column(&conn, "snippets", "last_used_at", "INTEGER NOT NULL DEFAULT 0")?;
        // P1-#PW#USE#PERSIST：密码表也补 use_count / last_used_at
        // 之前：前端用 in-memory 计数，刷新页面就丢
        // 现在：与 apps / snippets 一样持久化
        Self::migrate_add_column(&conn, "passwords", "use_count", "INTEGER NOT NULL DEFAULT 0")?;
        Self::migrate_add_column(&conn, "passwords", "last_used_at", "INTEGER NOT NULL DEFAULT 0")?;
        Ok(())
    }

    /// 迁移辅助：如果列不存在则 ADD COLUMN（PRAGMA table_info 检查）
    fn migrate_add_column(
        conn: &Connection,
        table: &str,
        column: &str,
        definition: &str,
    ) -> Result<()> {
        let mut stmt = conn.prepare(&format!("PRAGMA table_info({})", table))?;
        let cols: Vec<String> = stmt
            .query_map([], |row| row.get::<_, String>(1))?
            .filter_map(|r| r.ok())
            .collect();
        if !cols.iter().any(|c| c == column) {
            conn.execute(
                &format!("ALTER TABLE {} ADD COLUMN {} {}", table, column, definition),
                [],
            )?;
        }
        Ok(())
    }

    /// 升级迁移：新增列 / 索引（v1 → v1.1 加 app_type 区分 软件/文件夹/文档）
    fn run_migrations(&self) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        // P0-#Y：apps.app_type 区分 "app" / "folder" / "document" / "url"
        Self::migrate_add_column(&conn, "apps", "app_type", "TEXT NOT NULL DEFAULT 'app'")?;
        // P0-#Y#2：apps.app_subtype 细分
        //   app:        game / office / dev / utility / media / design / other
        //   document:   word / excel / ppt / pdf / text / image / other
        //   folder / url：留空
        Self::migrate_add_column(&conn, "apps", "app_subtype", "TEXT NOT NULL DEFAULT ''")?;
        // P0-#T：回收站 — 4 表都加 deleted_at（NULL = 正常，非 NULL = 删除时间戳）
        Self::migrate_add_column(&conn, "apps", "deleted_at", "INTEGER")?;
        Self::migrate_add_column(&conn, "passwords", "deleted_at", "INTEGER")?;
        Self::migrate_add_column(&conn, "snippets", "deleted_at", "INTEGER")?;
        Self::migrate_add_column(&conn, "temp_contents", "deleted_at", "INTEGER")?;
        Ok(())
    }

    // ===== 设置相关 =====

    pub fn get_setting(&self, key: &str) -> Result<Option<String>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT value FROM settings WHERE key = ?1")?;
        let result: Option<String> = stmt
            .query_row(params![key], |row| row.get(0))
            .ok();
        Ok(result)
    }

    pub fn set_setting(&self, key: &str, value: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO settings (key, value) VALUES (?1, ?2)",
            params![key, value],
        )?;
        Ok(())
    }

    pub fn delete_setting(&self, key: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM settings WHERE key = ?1", params![key])?;
        Ok(())
    }

    // ===== 密码相关 =====

    /// 创建密码（传入已加密的 password 字段）
    pub fn create_password(
        &self,
        title: &str,
        username: &str,
        encrypted_password: &str,
        url: &str,
        notes: &str,
    ) -> Result<i64> {
        let now = chrono::Utc::now().timestamp_millis();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO passwords (title, username, password, url, notes, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![title, username, encrypted_password, url, notes, now, now],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// 创建 v2 密码记录。record_uuid 由 command 层生成，一经写入不提供修改 API。
    pub fn create_password_v2(
        &self,
        record_uuid: &str,
        title: &str,
        username: &str,
        encrypted_password: &str,
        url: &str,
        notes: &str,
    ) -> Result<i64> {
        let now = chrono::Utc::now().timestamp_millis();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO passwords
             (record_uuid, title, username, password, url, notes, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                record_uuid,
                title,
                username,
                encrypted_password,
                url,
                notes,
                now,
                now
            ],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// 列出所有密码（仅元数据，password 字段是加密的）
    pub fn list_passwords(&self) -> Result<Vec<PasswordMeta>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, title, username, url, notes, created_at, updated_at, use_count, last_used_at
             FROM passwords WHERE deleted_at IS NULL ORDER BY updated_at DESC",
        )?;
        let rows = stmt
            .query_map([], |row| {
                Ok(PasswordMeta {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    username: row.get(2)?,
                    url: row.get(3)?,
                    notes: row.get(4)?,
                    created_at: row.get(5)?,
                    updated_at: row.get(6)?,
                    use_count: row.get::<_, i64>(7).unwrap_or(0),
                    last_used_at: row.get::<_, i64>(8).unwrap_or(0),
                })
            })?
            .collect::<Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// v2 列表读取入口。显式命名，避免 command 层根据列存在与否猜测安全模型。
    pub fn list_passwords_v2(&self) -> Result<Vec<PasswordMeta>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, title, username, url, notes, created_at, updated_at, use_count, last_used_at
             FROM passwords WHERE deleted_at IS NULL ORDER BY updated_at DESC",
        )?;
        let rows = stmt
            .query_map([], |row| {
                Ok(PasswordMeta {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    username: row.get(2)?,
                    url: row.get(3)?,
                    notes: row.get(4)?,
                    created_at: row.get(5)?,
                    updated_at: row.get(6)?,
                    use_count: row.get(7)?,
                    last_used_at: row.get(8)?,
                })
            })?
            .collect::<Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// 获取单条密码的加密内容
    pub fn get_password_encrypted(&self, id: i64) -> Result<Option<(String, String, String, String, String)>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT title, username, password, url, notes FROM passwords WHERE id = ?1",
        )?;
        let result = stmt
            .query_row(params![id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                ))
            })
            .ok();
        Ok(result)
    }

    /// 读取 v2 解密所需的最小材料，不加载其它用户元数据。
    pub fn get_password_v2(&self, id: i64) -> Result<Option<V2PasswordRecord>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT record_uuid, password FROM passwords WHERE id = ?1",
        )?;
        let result = stmt
            .query_row(params![id], |row| {
                Ok(V2PasswordRecord {
                    record_uuid: row.get(0)?,
                    encrypted_password: row.get(1)?,
                })
            })
            .optional()?;
        Ok(result)
    }

    #[cfg(test)]
    pub fn get_password_v2_snapshot(&self, id: i64) -> Result<Option<V2PasswordSnapshot>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT record_uuid, title, username, password, url, notes,
                    use_count, last_used_at, deleted_at
             FROM passwords WHERE id = ?1",
        )?;
        let result = stmt
            .query_row(params![id], |row| {
                Ok(V2PasswordSnapshot {
                    record_uuid: row.get(0)?,
                    title: row.get(1)?,
                    username: row.get(2)?,
                    encrypted_password: row.get(3)?,
                    url: row.get(4)?,
                    notes: row.get(5)?,
                    use_count: row.get(6)?,
                    last_used_at: row.get(7)?,
                    deleted_at: row.get(8)?,
                })
            })
            .optional()?;
        Ok(result)
    }

    /// 更新密码
    pub fn update_password(
        &self,
        id: i64,
        title: &str,
        username: &str,
        encrypted_password: &str,
        url: &str,
        notes: &str,
    ) -> Result<()> {
        let now = chrono::Utc::now().timestamp_millis();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE passwords SET title=?1, username=?2, password=?3, url=?4, notes=?5, updated_at=?6
             WHERE id=?7",
            params![title, username, encrypted_password, url, notes, now, id],
        )?;
        Ok(())
    }

    /// 显式更新 v2 密码密文与可编辑元数据；record_uuid 永不进入 UPDATE 集合。
    pub fn update_password_v2(
        &self,
        id: i64,
        title: &str,
        username: &str,
        encrypted_password: &str,
        url: &str,
        notes: &str,
    ) -> Result<()> {
        let now = chrono::Utc::now().timestamp_millis();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE passwords
             SET title=?1, username=?2, password=?3, url=?4, notes=?5, updated_at=?6
             WHERE id=?7",
            params![title, username, encrypted_password, url, notes, now, id],
        )?;
        Ok(())
    }

    /// 仅更新密码条目元数据（标题/用户名/网址/备注）。
    /// P0-B 契约：不触碰 password 字段——原密文字节级保持不变；
    /// updated_at 正常刷新（列表排序语义不变）。
    pub fn update_password_metadata(
        &self,
        id: i64,
        title: &str,
        username: &str,
        url: &str,
        notes: &str,
    ) -> Result<()> {
        let now = chrono::Utc::now().timestamp_millis();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE passwords SET title=?1, username=?2, url=?3, notes=?4, updated_at=?5 WHERE id=?6",
            params![title, username, url, notes, now, id],
        )?;
        Ok(())
    }

    /// v2 metadata-only patch：不读取也不写入 password / record_uuid。
    pub fn update_password_metadata_v2(
        &self,
        id: i64,
        title: &str,
        username: &str,
        url: &str,
        notes: &str,
    ) -> Result<()> {
        let now = chrono::Utc::now().timestamp_millis();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE passwords SET title=?1, username=?2, url=?3, notes=?4, updated_at=?5
             WHERE id=?6",
            params![title, username, url, notes, now, id],
        )?;
        Ok(())
    }

    /// 删除密码
    pub fn delete_password(&self, id: i64) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM passwords WHERE id=?1", params![id])?;
        Ok(())
    }

    /// P1-#PW#USE#PERSIST：累加密码使用次数（与 apps / snippets 一致）
    pub fn bump_password_use_count(&self, id: i64) -> Result<i64> {
        let now = chrono::Utc::now().timestamp_millis();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE passwords SET use_count = use_count + 1, last_used_at = ?1 WHERE id = ?2",
            params![now, id],
        )?;
        let mut stmt = conn.prepare("SELECT use_count FROM passwords WHERE id = ?1")?;
        let count: i64 = stmt
            .query_row(params![id], |row| row.get(0))
            .unwrap_or(0);
        Ok(count)
    }

    /// v2 使用统计入口：只更新 use_count / last_used_at。
    pub fn bump_password_use_count_v2(&self, id: i64) -> Result<i64> {
        let now = chrono::Utc::now().timestamp_millis();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE passwords SET use_count = use_count + 1, last_used_at = ?1 WHERE id = ?2",
            params![now, id],
        )?;
        let count = conn
            .query_row(
                "SELECT use_count FROM passwords WHERE id = ?1",
                params![id],
                |row| row.get(0),
            )
            .unwrap_or(0);
        Ok(count)
    }

    // ===== P1-#SETTINGS#PW#CHANGE#FULL：主密码修改时全量重加密 =====
    /// 读出所有密码（带加密字段）用于主密码修改时重加密
    pub fn list_passwords_full(&self) -> Result<Vec<PasswordWithCipher>> {
        let conn = self.conn.lock().unwrap();
        // deleted_at 可能在 schema 里没有，先尝试读，失败则当 NULL
        let mut stmt = conn.prepare(
            "SELECT id, password, deleted_at FROM passwords"
        )?;
        let rows = stmt
            .query_map([], |row| {
                Ok(PasswordWithCipher {
                    id: row.get(0)?,
                    encrypted_password: row.get(1)?,
                    deleted_at: row.get::<_, Option<i64>>(2).ok().flatten(),
                })
            })?
            .collect::<Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// 重加密后只更新 password 字段（不触发 updated_at 变化，避免误触列表重排）
    pub fn update_password_encrypted(&self, id: i64, encrypted_password: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE passwords SET password = ?1 WHERE id = ?2",
            params![encrypted_password, id],
        )?;
        Ok(())
    }

    // ===== 软件（apps）相关 =====

    /// 创建软件
    pub fn create_app(
        &self,
        name: &str,
        path: &str,
        icon_path: &str,
        args: &str,
        category_id: Option<i64>,
        app_type: &str,
        app_subtype: &str,
    ) -> Result<i64> {
        let now = chrono::Utc::now().timestamp_millis();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO apps (name, path, icon_path, args, category_id, app_type, app_subtype, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![name, path, icon_path, args, category_id, app_type, app_subtype, now],
        )?;
        let id = conn.last_insert_rowid();
        Ok(id)
    }

    /// 列出所有软件（可选搜索 + 分类过滤）
    pub fn list_apps(&self, query: &str, category_id: Option<i64>) -> Result<Vec<AppMeta>> {
        let conn = self.conn.lock().unwrap();
        let mut sql = String::from(
            "SELECT id, name, path, icon_path, args, category_id, app_type, app_subtype, use_count, last_used_at, created_at
             FROM apps WHERE deleted_at IS NULL",
        );
        let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        if !query.is_empty() {
            sql.push_str(" AND (name LIKE ?1 OR path LIKE ?1)");
            params_vec.push(Box::new(format!("%{}%", query)));
        }
        if let Some(cat) = category_id {
            let idx = params_vec.len() + 1;
            sql.push_str(&format!(" AND category_id = ?{}", idx));
            params_vec.push(Box::new(cat));
        }
        sql.push_str(" ORDER BY use_count DESC, last_used_at DESC, created_at DESC");

        let mut stmt = conn.prepare(&sql)?;
        let params_refs: Vec<&dyn rusqlite::ToSql> = params_vec.iter().map(|b| b.as_ref()).collect();
        let rows = stmt
            .query_map(params_refs.as_slice(), |row| {
                Ok(AppMeta {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    path: row.get(2)?,
                    icon_path: row.get(3)?,
                    args: row.get(4)?,
                    category_id: row.get(5)?,
                    app_type: row.get(6)?,
                    app_subtype: row.get(7)?,
                    use_count: row.get(8)?,
                    last_used_at: row.get(9)?,
                    created_at: row.get(10)?,
                })
            })?
            .collect::<Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// 更新软件（仅名称/路径/参数/分类，不动使用统计）
    pub fn update_app(
        &self,
        id: i64,
        name: &str,
        path: &str,
        icon_path: &str,
        args: &str,
        category_id: Option<i64>,
        app_subtype: &str,
    ) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE apps SET name=?1, path=?2, icon_path=?3, args=?4, category_id=?5, app_subtype=?6 WHERE id=?7",
            params![name, path, icon_path, args, category_id, app_subtype, id],
        )?;
        Ok(())
    }

    /// P0-#Y#FIX#TYPE#EDIT：完整 update（含 app_type）
    /// 之前 update_app 不更新 app_type → 右键菜单无法把"误判为 app"的网址改成"url"
    ///
    /// ## 更新契约（P0-#Y#FIX#PATCH 定稿，勿混淆）
    /// - `Option` 参数 `None` = **保持原值**（不进入本次 UPDATE 的 SET 列表）
    /// - `Some(v)` = 显式更新为 v（v 为空串即"主动清空"该文本字段）
    /// - `name`/`path` 为必填字段，每次全量写入
    /// - 列名全部来自代码内固定字面量（白名单），值一律参数绑定，无 SQL 注入面
    /// - UI 目前没有"主动清空 category/icon/subtype"的入口；未来需要时必须新增
    ///   显式信号（专用命令或哨兵值），**不能复用 None 表达清空**
    pub fn update_app_full(
        &self,
        id: i64,
        name: &str,
        path: &str,
        icon_path: Option<&str>,
        args: Option<&str>,
        category_id: Option<i64>,
        app_subtype: Option<&str>,
        app_type: Option<&str>,
    ) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        // 动态构建 SET：只更新 Some 字段（patch 语义），列名为固定白名单字面量
        let mut sets: Vec<&str> = vec!["name=?", "path=?"];
        let mut values: Vec<rusqlite::types::Value> =
            vec![rusqlite::types::Value::from(name.to_string()), rusqlite::types::Value::from(path.to_string())];
        if let Some(v) = icon_path {
            sets.push("icon_path=?");
            values.push(rusqlite::types::Value::from(v.to_string()));
        }
        if let Some(v) = args {
            sets.push("args=?");
            values.push(rusqlite::types::Value::from(v.to_string()));
        }
        if let Some(v) = category_id {
            sets.push("category_id=?");
            values.push(rusqlite::types::Value::from(v));
        }
        if let Some(v) = app_subtype {
            sets.push("app_subtype=?");
            values.push(rusqlite::types::Value::from(v.to_string()));
        }
        if let Some(v) = app_type {
            sets.push("app_type=?");
            values.push(rusqlite::types::Value::from(v.to_string()));
        }
        // WHERE 子句单独拼装——id 是 WHERE 条件,绝不能混入 SET 列表
        values.push(rusqlite::types::Value::from(id));
        let sql = format!("UPDATE apps SET {} WHERE id = ?", sets.join(", "));
        conn.execute(&sql, rusqlite::params_from_iter(values))?;
        Ok(())
    }

    /// 根据 path（小写精确匹配）查找软件 ID。用于去重。
    pub fn find_app_id_by_path(&self, path: &str) -> Result<Option<i64>> {
        let conn = self.conn.lock().unwrap();
        let lower = path.to_lowercase();
        let mut stmt = conn.prepare("SELECT id FROM apps WHERE LOWER(path)=?1 LIMIT 1")?;
        let mut rows = stmt.query(params![lower])?;
        if let Some(row) = rows.next()? {
            Ok(Some(row.get(0)?))
        } else {
            Ok(None)
        }
    }

    /// P0-#W：按 name basename（去掉扩展名）查找软件 ID。
    /// 用于跨 path 去重（桌面/开始菜单同名文件）。
    /// 例：db 有 "VRoid Studio.url"，再拖 "VRoid Studio.exe" → 都能匹配
    pub fn find_app_id_by_name_stem(&self, stem: &str) -> Result<Option<i64>> {
        let conn = self.conn.lock().unwrap();
        // 通过 file_stem 匹配：path 中文件名去掉扩展名 = stem
        // 用 LIKE '%/<stem>.%' 或 '%\<stem>.%' 匹配（Windows 路径分隔符）
        let lower_stem = stem.to_lowercase();
        let pat_linux = format!("%/{}", lower_stem);   // Linux 风格
        let pat_win = format!("%\\{}", lower_stem);    // Windows 风格
        let mut stmt = conn.prepare(
            "SELECT id, path FROM apps
             WHERE (LOWER(path) LIKE ?1 OR LOWER(path) LIKE ?2)
             LIMIT 5"
        )?;
        let rows = stmt.query_map(params![pat_linux, pat_win], |row| {
            let id: i64 = row.get(0)?;
            let path: String = row.get(1)?;
            // 双重确认：path 的 file_stem 真的等于 stem
            if let Some(p) = std::path::Path::new(&path).file_stem().and_then(|s| s.to_str()) {
                if p.to_lowercase() == lower_stem {
                    return Ok(Some(id));
                }
            }
            Ok(None)
        })?;
        for r in rows {
            if let Some(id) = r? {
                return Ok(Some(id));
            }
        }
        Ok(None)
    }

    /// P0-#F：只更新 path（重定位时用）
    pub fn update_app_path(&self, id: i64, new_path: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE apps SET path = ?1 WHERE id = ?2",
            params![new_path, id],
        )?;
        Ok(())
    }

    /// 记录软件使用次数
    pub fn record_app_usage(&self, id: i64) -> Result<()> {
        let now = chrono::Utc::now().timestamp_millis();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE apps SET use_count = use_count + 1, last_used_at = ?1 WHERE id=?2",
            params![now, id],
        )?;
        Ok(())
    }

    // ===== 回收站（P0-#T）=====
    // 4 表统一软删除/恢复/硬删/列表/清理接口

    /// ✅ 表名兼容映射：前端可能传旧的 kind（单数），或者正确表名，全部统一成真实表名
    fn normalize_table_name(table: &str) -> Option<&'static str> {
        match table {
            // 正确复数表名（直接用）
            "apps" => Some("apps"),
            "passwords" => Some("passwords"),
            "snippets" => Some("snippets"),
            "temp_contents" => Some("temp_contents"),
            // 兼容旧前端传的 kind（单数）
            "app" => Some("apps"),
            "password" => Some("passwords"),
            "snippet" => Some("snippets"),
            "temp" => Some("temp_contents"),
            "temp_content" => Some("temp_contents"),
            // 其他 → 无效
            _ => None,
        }
    }

    /// 软删除（标记 deleted_at = now）。返回是否成功（0 = 已删除或不存在）
    pub fn soft_delete(&self, table: &str, id: i64) -> Result<usize> {
        let table = Self::normalize_table_name(table).ok_or(rusqlite::Error::InvalidQuery)?;
        let now = chrono::Utc::now().timestamp_millis();
        let conn = self.conn.lock().unwrap();
        let sql = format!(
            "UPDATE {} SET deleted_at = ?1 WHERE id = ?2 AND deleted_at IS NULL",
            table
        );
        let n = conn.execute(&sql, params![now, id])?;
        Ok(n)
    }

    /// 恢复（deleted_at 置 NULL）
    pub fn restore(&self, table: &str, id: i64) -> Result<usize> {
        let table = Self::normalize_table_name(table).ok_or(rusqlite::Error::InvalidQuery)?;
        let conn = self.conn.lock().unwrap();
        let sql = format!("UPDATE {} SET deleted_at = NULL WHERE id = ?1", table);
        let n = conn.execute(&sql, params![id])?;
        Ok(n)
    }

    /// 硬删（永久删除）
    pub fn hard_delete(&self, table: &str, id: i64) -> Result<usize> {
        let table = Self::normalize_table_name(table).ok_or(rusqlite::Error::InvalidQuery)?;
        let conn = self.conn.lock().unwrap();
        let sql = format!("DELETE FROM {} WHERE id = ?1", table);
        let n = conn.execute(&sql, params![id])?;
        Ok(n)
    }

    /// 清空某表回收站
    pub fn empty_trash(&self, table: &str) -> Result<usize> {
        // empty_trash 还支持 "all" 清全部
        let is_all = table == "all";
        let conn = self.conn.lock().unwrap();
        let mut total = 0usize;
        if is_all {
            for t in ["apps", "passwords", "snippets", "temp_contents"] {
                let sql = format!("DELETE FROM {} WHERE deleted_at IS NOT NULL", t);
                total += conn.execute(&sql, [])?;
            }
        } else {
            let t = Self::normalize_table_name(table).ok_or(rusqlite::Error::InvalidQuery)?;
            let sql = format!("DELETE FROM {} WHERE deleted_at IS NOT NULL", t);
            total += conn.execute(&sql, [])?;
        }
        Ok(total)
    }

    /// 自动清理过期回收站项（默认 30 天前）
    pub fn cleanup_expired_trash(&self, retention_ms: i64) -> Result<Vec<(&'static str, usize)>> {
        let now = chrono::Utc::now().timestamp_millis();
        let cutoff = now - retention_ms;
        let conn = self.conn.lock().unwrap();
        let mut out: Vec<(&'static str, usize)> = Vec::new();
        for table in ["apps", "passwords", "snippets", "temp_contents"] {
            let sql = format!(
                "DELETE FROM {} WHERE deleted_at IS NOT NULL AND deleted_at < ?1",
                table
            );
            let n = conn.execute(&sql, params![cutoff])?;
            out.push((table, n));
        }
        Ok(out)
    }

    /// 查询回收站项（按删除时间倒序）
    pub fn list_trash(&self) -> Result<Vec<TrashItem>> {
        let conn = self.conn.lock().unwrap();
        let mut out: Vec<TrashItem> = Vec::new();
        {
            let mut stmt = conn.prepare(
                "SELECT id, name, app_type, app_subtype, deleted_at FROM apps WHERE deleted_at IS NOT NULL ORDER BY deleted_at DESC LIMIT 500"
            )?;
            let rows = stmt.query_map([], |row| {
                Ok(TrashItem {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    kind: "app".to_string(),
                    table_name: "apps".to_string(), // ✅ 真实表名
                    kind_detail: row.get::<_, String>(2).unwrap_or_default(),
                    sub_detail: row.get::<_, String>(3).unwrap_or_default(),
                    deleted_at: row.get(4)?,
                    source: "软件".to_string(),
                })
            })?;
            for r in rows { out.push(r?); }
        }
        {
            let mut stmt = conn.prepare(
                "SELECT id, title, username, url, deleted_at FROM passwords WHERE deleted_at IS NOT NULL ORDER BY deleted_at DESC LIMIT 500"
            )?;
            let rows = stmt.query_map([], |row| {
                Ok(TrashItem {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    kind: "password".to_string(),
                    table_name: "passwords".to_string(), // ✅ 真实表名
                    kind_detail: row.get::<_, String>(2).unwrap_or_default(),
                    sub_detail: row.get::<_, String>(3).unwrap_or_default(),
                    deleted_at: row.get(4)?,
                    source: "密码".to_string(),
                })
            })?;
            for r in rows { out.push(r?); }
        }
        {
            let mut stmt = conn.prepare(
                "SELECT id, title, language, tags, deleted_at FROM snippets WHERE deleted_at IS NOT NULL ORDER BY deleted_at DESC LIMIT 500"
            )?;
            let rows = stmt.query_map([], |row| {
                Ok(TrashItem {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    kind: "snippet".to_string(),
                    table_name: "snippets".to_string(), // ✅ 真实表名
                    kind_detail: row.get::<_, String>(2).unwrap_or_default(),
                    sub_detail: row.get::<_, String>(3).unwrap_or_default(),
                    deleted_at: row.get(4)?,
                    source: "命令行".to_string(),
                })
            })?;
            for r in rows { out.push(r?); }
        }
        {
            let mut stmt = conn.prepare(
                "SELECT id, text, created_at, deleted_at FROM temp_contents WHERE deleted_at IS NOT NULL ORDER BY deleted_at DESC LIMIT 500"
            )?;
            let rows = stmt.query_map([], |row| {
                let text: String = row.get(1)?;
                let preview = if text.chars().count() > 20 {
                    format!("{}…", text.chars().take(20).collect::<String>())
                } else {
                    text
                };
                Ok(TrashItem {
                    id: row.get(0)?,
                    name: preview,
                    kind: "temp".to_string(),
                    table_name: "temp_contents".to_string(), // ✅ 真实表名
                    kind_detail: String::new(),
                    sub_detail: String::new(),
                    deleted_at: row.get(3)?,
                    source: "便签".to_string(),
                })
            })?;
            for r in rows { out.push(r?); }
        }
        out.sort_by(|a, b| b.deleted_at.cmp(&a.deleted_at));
        Ok(out)
    }

    /// 仅更新 icon_path（用于 fill_missing_icons 后回写）
    pub fn update_app_icon(&self, id: i64, icon_path: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE apps SET icon_path = ?1 WHERE id = ?2",
            params![icon_path, id],
        )?;
        Ok(())
    }

    // P0-#Y#FIX#ICON#REEXTRACT：清空所有 app 的 icon_path（强制重新抽图）
    pub fn clear_all_icon_paths(&self) -> Result<usize> {
        let conn = self.conn.lock().unwrap();
        let n = conn.execute("UPDATE apps SET icon_path = ''", [])?;
        Ok(n)
    }

    /// 删除软件
    pub fn delete_app(&self, id: i64) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM apps WHERE id=?1", params![id])?;
        Ok(())
    }

    // ===== 软件分类 =====

    /// 列出所有软件分类
    pub fn list_app_categories(&self) -> Result<Vec<AppCategoryMeta>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, name, icon, sort_order FROM app_categories ORDER BY sort_order, id",
        )?;
        let rows = stmt
            .query_map([], |row| {
                Ok(AppCategoryMeta {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    icon: row.get(2)?,
                    sort_order: row.get(3)?,
                })
            })?
            .collect::<Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// 新增分类
    pub fn create_app_category(&self, name: &str, icon: &str) -> Result<i64> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO app_categories (name, icon) VALUES (?1, ?2)",
            params![name, icon],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// 更新分类（重命名 / 改图标）
    pub fn update_app_category(&self, id: i64, name: &str, icon: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE app_categories SET name = ?1, icon = ?2 WHERE id = ?3",
            params![name, icon, id],
        )?;
        Ok(())
    }

    /// 删除分类（注意：被引用的 app.category_id 会被设为 NULL）
    pub fn delete_app_category(&self, id: i64) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        // 先把被引用软件的 category_id 设为 NULL（不级联删除软件本身）
        conn.execute(
            "UPDATE apps SET category_id = NULL WHERE category_id = ?1",
            params![id],
        )?;
        conn.execute(
            "DELETE FROM app_categories WHERE id = ?1",
            params![id],
        )?;
        Ok(())
    }

    // ===== 片段（snippets）相关 =====

    pub fn create_snippet(
        &self,
        title: &str,
        content: &str,
        language: &str,
        tags: &str,
    ) -> Result<i64> {
        let now = chrono::Utc::now().timestamp_millis();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO snippets (title, content, language, tags, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![title, content, language, tags, now, now],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn list_snippets(&self, query: &str) -> Result<Vec<SnippetMeta>> {
        let conn = self.conn.lock().unwrap();
        let mut sql = String::from(
            "SELECT id, title, content, language, tags, use_count, last_used_at, created_at, updated_at
             FROM snippets WHERE deleted_at IS NULL",
        );
        let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        if !query.is_empty() {
            sql.push_str(" AND (title LIKE ?1 OR content LIKE ?1 OR tags LIKE ?1)");
            params_vec.push(Box::new(format!("%{}%", query)));
        }
        sql.push_str(" ORDER BY updated_at DESC");

        let mut stmt = conn.prepare(&sql)?;
        let params_refs: Vec<&dyn rusqlite::ToSql> = params_vec.iter().map(|b| b.as_ref()).collect();
        let rows = stmt
            .query_map(params_refs.as_slice(), |row| {
                Ok(SnippetMeta {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    content: row.get(2)?,
                    language: row.get(3)?,
                    tags: row.get(4)?,
                    use_count: row.get(5)?,
                    last_used_at: row.get(6)?,
                    created_at: row.get(7)?,
                    updated_at: row.get(8)?,
                })
            })?
            .collect::<Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn get_snippet_content(&self, id: i64) -> Result<Option<String>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT content FROM snippets WHERE id=?1")?;
        let result = stmt.query_row(params![id], |row| row.get::<_, String>(0)).ok();
        Ok(result)
    }

    pub fn update_snippet(
        &self,
        id: i64,
        title: &str,
        content: &str,
        language: &str,
        tags: &str,
    ) -> Result<()> {
        let now = chrono::Utc::now().timestamp_millis();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE snippets SET title=?1, content=?2, language=?3, tags=?4, updated_at=?5 WHERE id=?6",
            params![title, content, language, tags, now, id],
        )?;
        Ok(())
    }

    pub fn record_snippet_usage(&self, id: i64) -> Result<()> {
        let now = chrono::Utc::now().timestamp_millis();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE snippets SET use_count = use_count + 1, last_used_at = ?1 WHERE id=?2",
            params![now, id],
        )?;
        Ok(())
    }

    pub fn delete_snippet(&self, id: i64) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM snippets WHERE id=?1", params![id])?;
        Ok(())
    }

    // ===== 临时内容（temp_contents）相关 =====

    pub fn create_temp(&self, text: &str, expires_at: i64) -> Result<i64> {
        let now = chrono::Utc::now().timestamp_millis();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO temp_contents (text, created_at, expires_at) VALUES (?1, ?2, ?3)",
            params![text, now, expires_at],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn list_temp(&self) -> Result<Vec<TempMeta>> {
        let now = chrono::Utc::now().timestamp_millis();
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, text, created_at, expires_at FROM temp_contents
             WHERE expires_at > ?1 AND deleted_at IS NULL ORDER BY expires_at ASC",
        )?;
        let rows = stmt
            .query_map(params![now], |row| {
                Ok(TempMeta {
                    id: row.get(0)?,
                    text: row.get(1)?,
                    created_at: row.get(2)?,
                    expires_at: row.get(3)?,
                })
            })?
            .collect::<Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn delete_temp(&self, id: i64) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM temp_contents WHERE id=?1", params![id])?;
        Ok(())
    }

    /// 清理所有已过期的临时内容（返回清理条数）
    pub fn cleanup_expired_temp(&self) -> Result<usize> {
        let now = chrono::Utc::now().timestamp_millis();
        let conn = self.conn.lock().unwrap();
        let n = conn.execute(
            "DELETE FROM temp_contents WHERE expires_at <= ?1",
            params![now],
        )?;
        Ok(n)
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct PasswordMeta {
    pub id: i64,
    pub title: String,
    pub username: String,
    pub url: String,
    pub notes: String,
    pub created_at: i64,
    pub updated_at: i64,
    /// P1-#PW#USE#PERSIST：累计复制次数（与 apps / snippets 一致）
    #[serde(default)]
    pub use_count: i64,
    #[serde(default)]
    pub last_used_at: i64,
}

/// v2 解密所需的最小 DB 内部表示。record_uuid 只读，不提供普通更新 API。
#[derive(Debug, Clone)]
pub struct V2PasswordRecord {
    pub record_uuid: String,
    pub encrypted_password: String,
}

#[cfg(test)]
#[derive(Debug, Clone)]
pub struct V2PasswordSnapshot {
    pub record_uuid: String,
    pub title: String,
    pub username: String,
    pub encrypted_password: String,
    pub url: String,
    pub notes: String,
    pub use_count: i64,
    pub last_used_at: i64,
    pub deleted_at: Option<i64>,
}

/// P1-#SETTINGS#PW#CHANGE#FULL：主密码修改时全量重加密用的内部结构
/// 与 PasswordMeta 区别：包含 encrypted_password 字段（外部 list_passwords 不返回密文）
#[derive(Debug, Clone)]
pub struct PasswordWithCipher {
    pub id: i64,
    pub encrypted_password: String,
    pub deleted_at: Option<i64>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct AppMeta {
    pub id: i64,
    pub name: String,
    pub path: String,
    pub icon_path: String,
    pub args: String,
    pub category_id: Option<i64>,
    pub app_type: String,    // "app" / "folder" / "document" / "url"
    pub app_subtype: String, // P0-#Y#2：细分（game/office/dev/utility/media/design/other/word/excel/ppt/pdf/text/image/other）
    pub use_count: i64,
    pub last_used_at: i64,
    pub created_at: i64,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct AppCategoryMeta {
    pub id: i64,
    pub name: String,
    pub icon: String,
    pub sort_order: i64,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct SnippetMeta {
    pub id: i64,
    pub title: String,
    pub content: String,
    pub language: String,
    pub tags: String,
    pub use_count: i64,
    pub last_used_at: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct TempMeta {
    pub id: i64,
    pub text: String,
    pub created_at: i64,
    pub expires_at: i64,
}

/// 回收站项统一结构（前端显示）
#[derive(Debug, Clone, serde::Serialize)]
pub struct TrashItem {
    pub id: i64,
    pub name: String,
    pub kind: String,         // "app" / "password" / "snippet" / "temp"（前端显示用，历史保留）
    pub table_name: String,   // ✅ 新增：真实表名，恢复/硬删时直接传这个！"apps" / "passwords" / "snippets" / "temp_contents"
    pub kind_detail: String,  // app_type / username / language
    pub sub_detail: String,   // app_subtype / url / tags
    pub deleted_at: i64,      // ms timestamp
    pub source: String,       // "软件" / "密码" / "命令行" / "便签"
}

/// 初始化数据库（兼容旧 API）
pub fn init_db(path: &Path) -> Result<()> {
    let _ = Db::open(path)?;
    Ok(())
}

/// 加密辅助（给 Tauri command 调用）
pub fn encrypt_string(plaintext: &str, key: &[u8]) -> Result<String, String> {
    crypto::encrypt(plaintext, key)
}

pub fn decrypt_string(encrypted: &str, key: &[u8]) -> Result<String, String> {
    crypto::decrypt(encrypted, key)
}

// ============================================================
// 🗄️ 备份 / 导入 / 导出 结构体
// ============================================================

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ImportConflictPolicy {
    /// 跳过（保留本地数据）
    Skip,
    /// 覆盖（用备份数据覆盖本地）
    Overwrite,
    /// 合并（备份数据改名后并存）
    Merge,
}

impl Default for ImportConflictPolicy {
    fn default() -> Self { ImportConflictPolicy::Skip }
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct ImportStats {
    pub backup_version: u32,
    pub security_model: String,
    pub full_restore: bool,
    pub settings: usize,
    pub categories_inserted: usize,
    pub categories_conflict: usize,
    pub apps_inserted: usize,
    pub apps_skipped: usize,
    pub apps_overwritten: usize,
    pub apps_merged: usize,
    pub passwords_inserted: usize,
    pub passwords_skipped: usize,
    pub passwords_overwritten: usize,
    pub passwords_merged: usize,
    pub snippets_inserted: usize,
    pub snippets_skipped: usize,
    pub snippets_overwritten: usize,
    pub snippets_merged: usize,
    pub temps_inserted: usize,
    pub temps_skipped: usize,
    pub temps_overwritten: usize,
    pub temps_merged: usize,
    pub pinned_inserted: usize,
    pub total_bytes: usize,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SettingRow { pub key: String, pub value: String }

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct PasswordRow {
    #[serde(default)]
    pub id: Option<i64>,
    pub title: String,
    #[serde(default)]
    pub username: String,
    // ✅ 旧版备份字段名就叫 password！新版叫 encrypted_password！两个都认！
    // 注意：encrypted_password 是数据库里的「密文」，仅兼容旧备份/同电脑导入用
    // 换电脑导入必须走 password_plaintext（明文，新导出格式）
    #[serde(alias = "password")]
    #[serde(default)]
    pub encrypted_password: String,
    // ✨ 新格式：明文密码（导出时先解密存这里，导入时用新电脑密钥重新加密）
    // 为什么用明文？因为加密密钥 = 主密码 + salt，换电脑 salt 不一样，密文直接拷过去必然解不开
    // 明文会被外层 AES-256-GCM 整体加密，所以备份文件本身还是安全的
    #[serde(default)]
    pub password_plaintext: Option<String>,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub use_count: i64,
    #[serde(default)]
    pub last_used_at: i64,
    #[serde(default)]
    pub created_at: i64,
    #[serde(default)]
    pub updated_at: i64,
    #[serde(default)]
    pub deleted_at: Option<i64>,
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct CategoryRow {
    #[serde(default)]
    pub id: Option<i64>,
    pub name: String,
    #[serde(default)]
    pub icon: String,
    #[serde(default)]
    pub sort_order: i64,
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct AppRow {
    #[serde(default)]
    pub id: Option<i64>,
    pub name: String,
    pub path: String,
    #[serde(default)]
    pub icon_path: String,
    #[serde(default, alias = "arguments")]
    pub args: String,
    // ✅ 旧版可能叫 category / category_id！三个都认！
    #[serde(default, alias = "category", alias = "category_id")]
    pub category_name: Option<String>,
    pub app_type: String,
    #[serde(default)]
    pub app_subtype: String,
    #[serde(default)]
    pub use_count: i64,
    #[serde(default)]
    pub last_used_at: i64,
    #[serde(default)]
    pub created_at: i64,
    #[serde(default)]
    pub deleted_at: Option<i64>,
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct SnippetRow {
    #[serde(default)]
    pub id: Option<i64>,
    pub title: String,
    // ✅ 旧版可能叫 body / text！三个都认！
    #[serde(default, alias = "body", alias = "text")]
    pub content: String,
    #[serde(default)]
    pub language: String,
    #[serde(default)]
    pub tags: String,
    #[serde(default)]
    pub use_count: i64,
    #[serde(default)]
    pub last_used_at: i64,
    #[serde(default)]
    pub created_at: i64,
    #[serde(default)]
    pub updated_at: i64,
    #[serde(default)]
    pub deleted_at: Option<i64>,
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct TempRow {
    #[serde(default)]
    pub id: Option<i64>,
    // ✅ 旧版可能叫 content / body！三个都认！
    #[serde(default, alias = "content", alias = "body")]
    pub text: String,
    #[serde(default)]
    pub created_at: i64,
    #[serde(default)]
    pub expires_at: i64,
    #[serde(default)]
    pub deleted_at: Option<i64>,
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct BackupFullData {
    // ✅ 旧版备份没有这两个字段！给默认值！缺失不会报错！
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub generated_at: i64,
    // ✨ V2 新增：导出时的 master_password_salt（BASE64）
    // 目的：工厂重置后导入旧格式密文备份，也能用旧salt+主密码派生出旧key解密重加密
    // 安全性：备份文件本身用 AES-256-GCM 加密（用户备份密码保护），存 salt 不影响安全
    #[serde(default)]
    pub export_master_password_salt_b64: Option<String>,
    // 兼容两种格式：
    //   - 旧格式（Map）：  {"k1":"v1", "k2":"v2"}
    //   - 新格式（Array）：[{"key":"k1","value":"v1"}, ...]
    #[serde(default)]
    pub settings: serde_json::Value,
    #[serde(default)]
    pub categories: Vec<CategoryRow>,
    #[serde(default)]
    pub passwords: Vec<PasswordRow>,
    #[serde(default)]
    pub apps: Vec<AppRow>,
    #[serde(default)]
    pub snippets: Vec<SnippetRow>,
    #[serde(default)]
    pub temps: Vec<TempRow>,
}

impl BackupFullData {
    /// ✅ 把 settings 字段从兼容格式（Map 或 Array）统一转成 Vec<SettingRow>
    pub fn settings_rows(&self) -> Result<Vec<SettingRow>, String> {
        match &self.settings {
            // 情况 1：数组（新格式）→ 直接反序列化成 Vec<SettingRow>
            serde_json::Value::Array(arr) => {
                let rows: Vec<SettingRow> = serde_json::from_value(serde_json::Value::Array(arr.clone()))
                    .map_err(|e| format!("settings 数组格式错误: {}", e))?;
                Ok(rows)
            }
            // 情况 2：对象（旧格式 Map<String, String>）→ 转成 Vec<SettingRow>
            serde_json::Value::Object(map) => {
                let mut out: Vec<SettingRow> = Vec::with_capacity(map.len());
                for (k, v) in map {
                    let v_str = match v {
                        serde_json::Value::String(s) => s.clone(),
                        other => other.to_string(),
                    };
                    out.push(SettingRow { key: k.clone(), value: v_str });
                }
                Ok(out)
            }
            // 情况 3：空（Null）→ 返回空数组
            serde_json::Value::Null => Ok(Vec::new()),
            // 其他情况 → 报错
            other => Err(format!("settings 字段格式异常（既不是对象也不是数组）: {:?}", other)),
        }
    }
}

impl Db {
    // ============================================================
    // 🗄️ 备份导出：读取全部 6 张表数据（不含恢复短语/主密码 hash 等敏感设置）
    // ============================================================
    pub fn export_all_data(&self) -> Result<BackupFullData> {
        let conn = self.conn.lock().unwrap();
        let mut out = BackupFullData {
            version: 1,
            generated_at: chrono::Utc::now().timestamp_millis(),
            ..Default::default()
        };

        // 1) 设置：只导出 widget.*/ui.* 前缀的非敏感项（跳过密码 hash/salt/恢复短语/二次验证密码）
        {
            let mut stmt = conn.prepare(
                "SELECT key, value FROM settings
                 WHERE key NOT IN ('master_password_hash','master_password_salt','recovery_phrase_encrypted','pw2nd_hash','pw2nd_salt')"
            )?;
            let rows = stmt.query_map([], |row| Ok(SettingRow {
                key: row.get(0)?,
                value: row.get(1)?,
            }))?.collect::<Result<Vec<_>>>()?;
            out.settings = serde_json::to_value(&rows).map_err(|e| rusqlite::Error::ToSqlConversionFailure(e.into()))?;
        }

        // 2) 软件分类
        {
            let mut stmt = conn.prepare(
                "SELECT id, name, icon, sort_order FROM app_categories ORDER BY sort_order, id"
            )?;
            let rows = stmt.query_map([], |row| Ok(CategoryRow {
                id: Some(row.get(0)?),
                name: row.get(1)?,
                icon: row.get(2)?,
                sort_order: row.get(3)?,
            }))?.collect::<Result<Vec<_>>>()?;
            out.categories = rows;
        }

        // 3) 密码（密文导出，含 use_count/last_used_at/deleted_at）
        {
            let mut stmt = conn.prepare(
                "SELECT id, title, username, password, url, notes, use_count, last_used_at, created_at, updated_at, deleted_at
                 FROM passwords"
            )?;
            let rows = stmt.query_map([], |row| Ok(PasswordRow {
                id: Some(row.get(0)?),
                title: row.get(1)?,
                username: row.get(2)?,
                encrypted_password: row.get(3)?,
                password_plaintext: None, // 导出时先空，settings_backup.rs 会解密填明文
                url: row.get(4)?,
                notes: row.get(5)?,
                use_count: row.get::<_, i64>(6).unwrap_or(0),
                last_used_at: row.get::<_, i64>(7).unwrap_or(0),
                created_at: row.get(8)?,
                updated_at: row.get(9)?,
                deleted_at: row.get::<_, Option<i64>>(10).ok().flatten(),
            }))?.collect::<Result<Vec<_>>>()?;
            out.passwords = rows;
        }

        // 4) 软件（把 category_id → 分类名，跨库导入时用名称匹配）
        {
            let mut cat_stmt = conn.prepare("SELECT id, name FROM app_categories")?;
            let cat_rows = cat_stmt.query_map([], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })?;
            let mut cat_id_to_name: std::collections::HashMap<i64, String> = std::collections::HashMap::new();
            for r in cat_rows {
                let (id, name) = r?;
                cat_id_to_name.insert(id, name);
            }

            let mut stmt = conn.prepare(
                "SELECT id, name, path, icon_path, args, category_id, app_type, app_subtype, use_count, last_used_at, created_at, deleted_at
                 FROM apps"
            )?;
            let rows = stmt.query_map([], |row| Ok(AppRow {
                id: Some(row.get(0)?),
                name: row.get(1)?,
                path: row.get(2)?,
                icon_path: row.get(3)?,
                args: row.get(4)?,
                category_name: row.get::<_, Option<i64>>(5).ok().flatten().and_then(|id| cat_id_to_name.get(&id).cloned()),
                app_type: row.get(6)?,
                app_subtype: row.get(7)?,
                use_count: row.get::<_, i64>(8).unwrap_or(0),
                last_used_at: row.get::<_, i64>(9).unwrap_or(0),
                created_at: row.get(10)?,
                deleted_at: row.get::<_, Option<i64>>(11).ok().flatten(),
            }))?.collect::<Result<Vec<_>>>()?;
            out.apps = rows;
        }

        // 5) 代码段
        {
            let mut stmt = conn.prepare(
                "SELECT id, title, content, language, tags, use_count, last_used_at, created_at, updated_at, deleted_at
                 FROM snippets"
            )?;
            let rows = stmt.query_map([], |row| Ok(SnippetRow {
                id: Some(row.get(0)?),
                title: row.get(1)?,
                content: row.get(2)?,
                language: row.get(3)?,
                tags: row.get(4)?,
                use_count: row.get::<_, i64>(5).unwrap_or(0),
                last_used_at: row.get::<_, i64>(6).unwrap_or(0),
                created_at: row.get(7)?,
                updated_at: row.get(8)?,
                deleted_at: row.get::<_, Option<i64>>(9).ok().flatten(),
            }))?.collect::<Result<Vec<_>>>()?;
            out.snippets = rows;
        }

        // 6) 便签
        {
            let mut stmt = conn.prepare(
                "SELECT id, text, created_at, expires_at, deleted_at FROM temp_contents"
            )?;
            let rows = stmt.query_map([], |row| Ok(TempRow {
                id: Some(row.get(0)?),
                text: row.get(1)?,
                created_at: row.get(2)?,
                expires_at: row.get(3)?,
                deleted_at: row.get::<_, Option<i64>>(4).ok().flatten(),
            }))?.collect::<Result<Vec<_>>>()?;
            out.temps = rows;
        }

        Ok(out)
    }

    // ============================================================
    // 🗄️ 备份导入：按策略合并，用事务保证原子性（失败全回滚）
    // ============================================================
    pub fn import_all_data(&self, data: &BackupFullData, policy: ImportConflictPolicy) -> Result<ImportStats> {
        let conn = self.conn.lock().unwrap();
        // 开启事务（任何一步失败自动回滚）
        conn.execute("BEGIN IMMEDIATE", [])?;
        let mut stats = ImportStats::default();
        let tx_result = (|| -> Result<(), Box<dyn std::error::Error>> {
            use std::collections::HashMap;

            // 1) 设置：先兼容转换（Map / Array 两种格式），再 INSERT OR REPLACE（保留本地主密码相关设置不动）
            let settings_rows = data.settings_rows().map_err(|e| -> Box<dyn std::error::Error> { e.into() })?;
            for s in &settings_rows {
                if matches!(s.key.as_str(),
                    "master_password_hash" | "master_password_salt" | "recovery_phrase_encrypted"
                    | "pw2nd_hash" | "pw2nd_salt") { continue; }
                conn.execute(
                    "INSERT OR REPLACE INTO settings (key, value) VALUES (?1, ?2)",
                    params![&s.key, &s.value],
                )?;
                stats.settings += 1;
            }

            // 2) 分类：按 name 匹配 → 生成本地 id 映射（cat_name_to_id）
            let mut cat_name_to_id: HashMap<String, i64> = HashMap::new();
            {
                let mut stmt = conn.prepare("SELECT id, name FROM app_categories")?;
                let rows = stmt.query_map([], |row| {
                    Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
                })?;
                for r in rows {
                    let (id, name) = r?;
                    cat_name_to_id.insert(name, id);
                }
            }
            for c in &data.categories {
                if let Some(&existing_id) = cat_name_to_id.get(&c.name) {
                    // 分类名已存在 → 冲突（更新 icon 和 sort_order，不建新分类）
                    conn.execute(
                        "UPDATE app_categories SET icon=?1, sort_order=?2 WHERE id=?3",
                        params![&c.icon, c.sort_order, existing_id],
                    )?;
                    stats.categories_conflict += 1;
                } else {
                    // 不存在 → 新建
                    conn.execute(
                        "INSERT INTO app_categories (name, icon, sort_order) VALUES (?1, ?2, ?3)",
                        params![&c.name, &c.icon, c.sort_order],
                    )?;
                    let new_id = conn.last_insert_rowid();
                    cat_name_to_id.insert(c.name.clone(), new_id);
                    stats.categories_inserted += 1;
                }
            }

            // 辅助：按策略决定最终名称（冲突时用）
            fn resolve_name(existing: Option<()>, name: &str, policy: &ImportConflictPolicy)
                -> std::result::Result<Option<String>, ()>
            {
                match (existing, policy) {
                    (Some(_), ImportConflictPolicy::Skip) => Ok(None),
                    (Some(_), ImportConflictPolicy::Overwrite) => Ok(Some(name.to_string())),
                    (Some(_), ImportConflictPolicy::Merge) => Ok(Some(format!("{} (导入副本)", name))),
                    (None, _) => Ok(Some(name.to_string())),
                }
            }

            // 3) 密码（冲突判断：同 title + 同 username）
            for p in &data.passwords {
                // 本地是否已存在（title + username 完全一致）【只看正常的，不看回收站的】
                let dup_id: Option<i64> = {
                    let mut stmt = conn.prepare(
                        "SELECT id FROM passwords
                         WHERE title = ?1 AND username = ?2 AND deleted_at IS NULL LIMIT 1"
                    )?;
                    stmt.query_row(params![&p.title, &p.username], |row| row.get::<_, i64>(0)).ok()
                };
                let final_name = match resolve_name(dup_id.map(|_| ()), &p.title, &policy) {
                    Ok(Some(n)) => n,
                    Ok(None) => { stats.passwords_skipped += 1; continue; },
                    Err(_) => { stats.passwords_skipped += 1; continue; },
                };
                if dup_id.is_some() {
                    match policy {
                        ImportConflictPolicy::Overwrite => {
                            let id = dup_id.unwrap();
                            // 🔴 【风险4修复】UPDATE 也写 deleted_at！但注意：如果本地是正常的，备份是回收站的，要不要把本地正常的搞成回收站？
                            // 这里按"备份里的值为准"写 deleted_at（如果备份里是回收站，导入后本地也变回收站）
                            conn.execute(
                                "UPDATE passwords SET title=?1, username=?2, password=?3, url=?4, notes=?5,
                                 use_count=?6, last_used_at=?7, updated_at=?8, deleted_at=?9 WHERE id=?10",
                                params![&final_name, &p.username, &p.encrypted_password,
                                    &p.url, &p.notes, p.use_count, p.last_used_at,
                                    chrono::Utc::now().timestamp_millis(), p.deleted_at, id],
                            )?;
                            stats.passwords_overwritten += 1;
                        },
                        ImportConflictPolicy::Merge => {
                            conn.execute(
                                "INSERT INTO passwords (title, username, password, url, notes, use_count, last_used_at, created_at, updated_at, deleted_at)
                                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                                params![&final_name, &p.username, &p.encrypted_password,
                                    &p.url, &p.notes, p.use_count, p.last_used_at,
                                    p.created_at, chrono::Utc::now().timestamp_millis(), p.deleted_at],
                            )?;
                            stats.passwords_merged += 1;
                        },
                        _ => unreachable!(), // Skip 已在 resolve_name 中处理
                    }
                } else {
                    // 🔴 【风险4修复】INSERT 必须写 deleted_at！否则回收站的密码导入后变成正常密码！
                    conn.execute(
                        "INSERT INTO passwords (title, username, password, url, notes, use_count, last_used_at, created_at, updated_at, deleted_at)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                        params![&final_name, &p.username, &p.encrypted_password,
                            &p.url, &p.notes, p.use_count, p.last_used_at,
                            p.created_at, p.updated_at, p.deleted_at],
                    )?;
                    stats.passwords_inserted += 1;
                }
            }

            // 4) 软件（冲突判断：同 path，或同 name stem）
            for a in &data.apps {
                let local_cat_id = a.category_name.as_ref().and_then(|n| cat_name_to_id.get(n).copied());
                // 查找本地是否存在相同路径或同名【只看正常的，不看回收站的】
                let dup_id: Option<i64> = {
                    let lower = a.path.to_lowercase();
                    let mut stmt = conn.prepare(
                        "SELECT id FROM apps WHERE LOWER(path) = ?1 AND deleted_at IS NULL LIMIT 1"
                    )?;
                    if let Ok(id) = stmt.query_row(params![&lower], |row| row.get::<_, i64>(0)) {
                        Some(id)
                    } else if let Some(stem) = std::path::Path::new(&a.path)
                        .file_stem().and_then(|s| s.to_str()) {
                        let pat1 = format!("%/{}", stem.to_lowercase());
                        let pat2 = format!("%\\{}", stem.to_lowercase());
                        let mut stmt2 = conn.prepare(
                            "SELECT id, path FROM apps
                             WHERE (LOWER(path) LIKE ?1 OR LOWER(path) LIKE ?2) AND deleted_at IS NULL
                             LIMIT 5"
                        )?;
                        let rows = stmt2.query_map(params![&pat1, &pat2], |row| {
                            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
                        })?;
                        let mut found = None;
                        for r in rows {
                            let (id, p) = r?;
                            if let Some(s2) = std::path::Path::new(&p).file_stem().and_then(|s| s.to_str()) {
                                if s2.to_lowercase() == stem.to_lowercase() { found = Some(id); break; }
                            }
                        }
                        found
                    } else { None }
                };
                let final_name = match resolve_name(dup_id.map(|_| ()), &a.name, &policy) {
                    Ok(Some(n)) => n,
                    Ok(None) => { stats.apps_skipped += 1; continue; },
                    Err(_) => { stats.apps_skipped += 1; continue; },
                };
                if dup_id.is_some() {
                    match policy {
                        ImportConflictPolicy::Overwrite => {
                            let id = dup_id.unwrap();
                            // 🔴 【风险4修复】UPDATE 也写 deleted_at
                            conn.execute(
                                "UPDATE apps SET name=?1, path=?2, icon_path=?3, args=?4, category_id=?5,
                                 app_type=?6, app_subtype=?7, use_count=?8, last_used_at=?9, deleted_at=?10 WHERE id=?11",
                                params![&final_name, &a.path, &a.icon_path, &a.args, local_cat_id,
                                    &a.app_type, &a.app_subtype, a.use_count, a.last_used_at, a.deleted_at, id],
                            )?;
                            stats.apps_overwritten += 1;
                        },
                        ImportConflictPolicy::Merge => {
                            conn.execute(
                                "INSERT INTO apps (name, path, icon_path, args, category_id, app_type, app_subtype, use_count, last_used_at, created_at, deleted_at)
                                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                                params![&final_name, &a.path, &a.icon_path, &a.args, local_cat_id,
                                    &a.app_type, &a.app_subtype, a.use_count, a.last_used_at,
                                    a.created_at, a.deleted_at],
                            )?;
                            stats.apps_merged += 1;
                        },
                        _ => unreachable!(),
                    }
                } else {
                    // 🔴 【风险4修复】INSERT 必须写 deleted_at
                    conn.execute(
                        "INSERT INTO apps (name, path, icon_path, args, category_id, app_type, app_subtype, use_count, last_used_at, created_at, deleted_at)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                        params![&final_name, &a.path, &a.icon_path, &a.args, local_cat_id,
                            &a.app_type, &a.app_subtype, a.use_count, a.last_used_at,
                            a.created_at, a.deleted_at],
                    )?;
                    stats.apps_inserted += 1;
                }
            }

            // 5) 代码段（冲突判断：同 title + 同 language）
            for s in &data.snippets {
                let dup_id: Option<i64> = {
                    let mut stmt = conn.prepare(
                        "SELECT id FROM snippets
                         WHERE title = ?1 AND language = ?2 AND deleted_at IS NULL LIMIT 1"
                    )?;
                    stmt.query_row(params![&s.title, &s.language], |row| row.get::<_, i64>(0)).ok()
                };
                let final_name = match resolve_name(dup_id.map(|_| ()), &s.title, &policy) {
                    Ok(Some(n)) => n,
                    Ok(None) => { stats.snippets_skipped += 1; continue; },
                    Err(_) => { stats.snippets_skipped += 1; continue; },
                };
                if dup_id.is_some() {
                    match policy {
                        ImportConflictPolicy::Overwrite => {
                            let id = dup_id.unwrap();
                            // 🔴 【风险4修复】UPDATE 也写 deleted_at
                            conn.execute(
                                "UPDATE snippets SET title=?1, content=?2, language=?3, tags=?4,
                                 use_count=?5, last_used_at=?6, updated_at=?7, deleted_at=?8 WHERE id=?9",
                                params![&final_name, &s.content, &s.language, &s.tags,
                                    s.use_count, s.last_used_at,
                                    chrono::Utc::now().timestamp_millis(), s.deleted_at, id],
                            )?;
                            stats.snippets_overwritten += 1;
                        },
                        ImportConflictPolicy::Merge => {
                            conn.execute(
                                "INSERT INTO snippets (title, content, language, tags, use_count, last_used_at, created_at, updated_at, deleted_at)
                                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                                params![&final_name, &s.content, &s.language, &s.tags,
                                    s.use_count, s.last_used_at,
                                    s.created_at, chrono::Utc::now().timestamp_millis(), s.deleted_at],
                            )?;
                            stats.snippets_merged += 1;
                        },
                        _ => unreachable!(),
                    }
                } else {
                    // 🔴 【风险4修复】INSERT 必须写 deleted_at
                    conn.execute(
                        "INSERT INTO snippets (title, content, language, tags, use_count, last_used_at, created_at, updated_at, deleted_at)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                        params![&final_name, &s.content, &s.language, &s.tags,
                            s.use_count, s.last_used_at,
                            s.created_at, s.updated_at, s.deleted_at],
                    )?;
                    stats.snippets_inserted += 1;
                }
            }

            // 6) 便签（冲突判断：同 text + 同 expires_at）
            for t in &data.temps {
                let dup_id: Option<i64> = {
                    let mut stmt = conn.prepare(
                        "SELECT id FROM temp_contents
                         WHERE text = ?1 AND expires_at = ?2 AND deleted_at IS NULL LIMIT 1"
                    )?;
                    stmt.query_row(params![&t.text, t.expires_at], |row| row.get::<_, i64>(0)).ok()
                };
                let final_text = match resolve_name(dup_id.map(|_| ()), &t.text, &policy) {
                    Ok(Some(n)) => n,
                    Ok(None) => { stats.temps_skipped += 1; continue; },
                    Err(_) => { stats.temps_skipped += 1; continue; },
                };
                if dup_id.is_some() {
                    match policy {
                        ImportConflictPolicy::Overwrite => {
                            let id = dup_id.unwrap();
                            // 🔴 【风险4修复】UPDATE 也写 deleted_at
                            conn.execute(
                                "UPDATE temp_contents SET text=?1, expires_at=?2, deleted_at=?3 WHERE id=?4",
                                params![&final_text, t.expires_at, t.deleted_at, id],
                            )?;
                            stats.temps_overwritten += 1;
                        },
                        ImportConflictPolicy::Merge => {
                            conn.execute(
                                "INSERT INTO temp_contents (text, created_at, expires_at, deleted_at)
                                 VALUES (?1, ?2, ?3, ?4)",
                                params![&final_text, t.created_at, t.expires_at, t.deleted_at],
                            )?;
                            stats.temps_merged += 1;
                        },
                        _ => unreachable!(),
                    }
                } else {
                    // 🔴 【风险4修复】INSERT 必须写 deleted_at
                    conn.execute(
                        "INSERT INTO temp_contents (text, created_at, expires_at, deleted_at)
                         VALUES (?1, ?2, ?3, ?4)",
                        params![&final_text, t.created_at, t.expires_at, t.deleted_at],
                    )?;
                    stats.temps_inserted += 1;
                }
            }

            Ok(())
        })();

        match tx_result {
            Ok(_) => {
                conn.execute("COMMIT", [])?;
            },
            Err(e) => {
                conn.execute("ROLLBACK", [])?;
                stats.errors.push(format!("导入事务失败，已回滚：{}", e));
            },
        }

        Ok(stats)
    }
}

#[cfg(test)]
mod password_patch_tests {
    use super::*;
    use std::path::PathBuf;

    fn temp_db() -> (Db, PathBuf) {
        let dir = std::env::temp_dir().join(format!("drawer_box_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("pw_{}.db", std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos()));
        let _ = std::fs::remove_file(&path);
        (Db::open(&path).unwrap(), path)
    }

    fn cleanup(path: &PathBuf) {
        let _ = std::fs::remove_file(path);
    }

    /// P0-B 场景1-4：单独修改标题/用户名/URL/备注，password 密文必须字节级不变
    #[test]
    fn metadata_field_updates_keep_ciphertext() {
        let (db, path) = temp_db();
        let cipher = "ciphertext-placeholder";
        let id = db.create_password("t", "u", cipher, "u0", "n0").unwrap();

        db.update_password_metadata(id, "t2", "u", "u0", "n0").unwrap();
        assert_eq!(db.get_password_encrypted(id).unwrap().unwrap().2, cipher, "改标题不得动密文");

        db.update_password_metadata(id, "t2", "u2", "u0", "n0").unwrap();
        assert_eq!(db.get_password_encrypted(id).unwrap().unwrap().2, cipher, "改用户名不得动密文");

        db.update_password_metadata(id, "t2", "u2", "u2", "n0").unwrap();
        assert_eq!(db.get_password_encrypted(id).unwrap().unwrap().2, cipher, "改URL不得动密文");

        db.update_password_metadata(id, "t2", "u2", "u2", "n2").unwrap();
        let (_, _, encrypted, url, notes) = db.get_password_encrypted(id).unwrap().unwrap();
        assert_eq!(encrypted, cipher);
        assert_eq!(url, "u2");
        assert_eq!(notes, "n2");
        cleanup(&path);
    }

    /// P0-B 场景5：明确更新密码 → 密文改变，且新密文可用同一密钥解出明文
    #[test]
    fn explicit_password_update_replaces_ciphertext() {
        let (db, path) = temp_db();
        let salt = crypto::generate_salt();
        let key = crypto::derive_key("master-pw", &salt);
        let c1 = crypto::encrypt("old-pass", &key).unwrap();
        let id = db.create_password("t", "u", &c1, "", "").unwrap();

        let c2 = crypto::encrypt("new-pass", &key).unwrap();
        db.update_password(id, "t", "u", &c2, "", "").unwrap();

        let (_, _, encrypted, _, _) = db.get_password_encrypted(id).unwrap().unwrap();
        assert_ne!(encrypted, c1, "改密码后密文必须改变");
        assert_eq!(crypto::decrypt(&encrypted, &key).unwrap(), "new-pass");
        cleanup(&path);
    }

    /// P0-B 场景6（DB 侧）：元数据更新绝不触碰密文（哨兵值只允许出现在标题位）
    #[test]
    fn metadata_update_never_touches_ciphertext() {
        let (db, path) = temp_db();
        let cipher = "real-cipher-bytes";
        let id = db.create_password("t", "u", cipher, "", "").unwrap();
        for sentinel in ["UNCHANGED", "（解密失败）"] {
            db.update_password_metadata(id, sentinel, "u", "", "").unwrap();
            let (title, _username, encrypted, _, _) = db.get_password_encrypted(id).unwrap().unwrap();
            assert_eq!(encrypted, cipher, "哨兵串不得进入密码位");
            assert_eq!(title, sentinel);
        }
        cleanup(&path);
    }
}

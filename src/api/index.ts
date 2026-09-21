/**
 * Tauri 命令封装
 */
import { invoke } from "@tauri-apps/api/core";

export const isFirstRun = () => invoke<boolean>("is_first_run");

export const initializeV2Security = (masterPassword: string) =>
  invoke<string[]>("initialize_v2_security", { masterPassword });

export const finalizeV2Security = () => invoke<void>("finalize_v2_security");

export const setupMasterPassword = (masterPassword: string, recoveryPhrase: string[]) =>
  invoke<void>("setup_master_password", { masterPassword, recoveryPhrase });

export const unlockApp = (masterPassword: string) =>
  invoke<string[]>("unlock_app", { masterPassword });

export const lockApp = () => invoke<void>("lock_app");

// P0-#PW#PER#ITEM#VERIFY / P0-#PW#2ND：密码区二次验证
// 优先比对独立二次验证密码（pw2nd_hash），未启用则回退到主密码 hash
export const verifyPasswordForPwView = (inputPassword: string) =>
  invoke<boolean>("verify_password_for_pw_view", { inputPassword });

// 兼容旧调用（内部直接路由到 verify_password_for_pw_view，推荐新代码用 verifyPasswordForPwView）
export const verifyMasterPassword = (masterPassword: string) =>
  invoke<boolean>("verify_master_password", { masterPassword });

// P0-#PW#2ND：是否启用了独立二次验证密码（true=独立；false=跟随主密码）
export const hasSecondPassword = () =>
  invoke<boolean>("has_second_password");

// P0-#PW#2ND：修改/启用/清空独立二次验证密码
//   oldVerifyInput：
//     - 未启用时 = 当前主密码
//     - 已启用时 = 旧二次验证密码（主密码无效）
//   newSecondPasswordOpt：Some("")/None → 清空（跟随主密码）；Some(v) → 新独立密码
export const changeSecondPassword = (
  oldVerifyInput: string,
  newSecondPasswordOpt: string | null
) =>
  invoke<void>("change_second_password", {
    oldVerifyInput,
    newSecondPasswordOpt,
  });

export const listPasswords = () => invoke<PasswordMeta[]>("list_passwords");

export const createPassword = (params: {
  title: string;
  username: string;
  password: string;
  url: string;
  notes: string;
}) => invoke<number>("create_password", params);

export const getPasswordDecrypted = (id: number) =>
  invoke<string>("get_password_decrypted", { id });

export const updatePassword = (params: {
  id: number;
  title: string;
  username: string;
  /** P0-B 契约：缺省（undefined/null）= 本次不修改密码，DB 原密文字节级保持不变 */
  password?: string;
  url: string;
  notes: string;
}) =>
  invoke<void>("update_password", {
    id: params.id,
    title: params.title,
    username: params.username,
    password: params.password ?? null,
    url: params.url,
    notes: params.notes,
  });

export const deletePassword = (id: number) =>
  invoke<void>("delete_password", { id });

/// P1-#PW#USE#PERSIST：累加密码使用次数（与 apps / snippets 一致）
export const bumpPasswordUseCount = (id: number) =>
  invoke<number>("bump_password_use_count", { id });

// P1-#SETTINGS#PW#CHANGE#FULL：主密码修改 + 全量重加密
// 返回 { passwords_reencrypted, passwords_failed, recovery_phrase_reencrypted }
export interface ReencryptStats {
  passwords_reencrypted: number;
  passwords_failed: number;
  recovery_phrase_reencrypted: boolean;
}
export const changeMasterPassword = (currentPassword: string, newPassword: string) =>
  invoke<ReencryptStats>("change_master_password", { currentPassword, newPassword });

// 🚨 紧急救援：密码全部解密失败时救回
// 场景：启用独立密码后/改主密码后/导入后 发现所有密码都「解密失败」
// 原理：输入「当初加密这些密码时用的主密码」，用旧密钥解密，再用当前密钥重加密
export interface RescueStats {
  tried: number;
  rescued: number;
  failed: number;
}
export const rescuePasswordsWithMaster = (oldMasterPassword: string) =>
  invoke<RescueStats>("rescue_passwords_with_master", { oldMasterPassword });

export const generateRandomPassword = (
  length: number,
  useUpper: boolean,
  useLower: boolean,
  useDigits: boolean,
  useSymbols: boolean
) =>
  invoke<string>("generate_random_password", {
    length,
    useUpper,
    useLower,
    useDigits,
    useSymbols,
  });

export const passwordStrength = (password: string) =>
  invoke<number>("password_strength", { password });

export const copyToClipboardWithTimeout = (text: string, timeoutSecs: number) =>
  invoke<void>("copy_to_clipboard_with_timeout", { text, timeoutSecs });

export const toggleWindow = () => invoke<void>("toggle_window");
export const showWindow = () => invoke<void>("show_window");
export const hideWindow = () => invoke<void>("hide_window");

// ===== Widget 窗口配置 =====

export const saveWidgetConfig = (key: string, value: string) =>
  invoke<void>("save_widget_config", { key, value });

export const loadWidgetConfig = (key: string) =>
  invoke<string | null>("load_widget_config", { key });

export const getWindowPosition = () =>
  invoke<[number, number]>("get_window_position");

export const getWindowSize = () =>
  invoke<[number, number]>("get_window_size");

export const setWindowPosition = (x: number, y: number) =>
  invoke<void>("set_window_position", { x, y });

export const setWindowSize = (width: number, height: number) =>
  invoke<void>("set_window_size", { width, height });

export const setAlwaysOnTop = (onTop: boolean) =>
  invoke<void>("set_always_on_top", { onTop });

/** 切换鼠标穿透（true=穿透到桌面，false=接收事件） */
export const setIgnoreCursorEvents = (ignore: boolean) =>
  invoke<void>("set_ignore_cursor_events", { ignore });

/** 切换迷你模式（仅改变窗口尺寸） */
export const setMiniMode = (mini: boolean) =>
  invoke<void>("set_mini_mode", { mini });

/** 切换迷你模式并自动调整位置（保持原 X） */
export const setMiniModeWithPos = (mini: boolean) =>
  invoke<void>("set_mini_mode_with_pos", { mini });

export const applyWidgetConfig = () =>
  invoke<void>("apply_widget_config");

// ===== 贴边自动隐藏（Rust 端 Win32 钩子）=====
export const setAutoHideEnabled = (enabled: boolean) =>
  invoke<void>("set_auto_hide_enabled", { enabled });
export const isDockHidden = () => invoke<boolean>("is_dock_hidden");
export const forceDockReveal = () => invoke<void>("force_dock_reveal");

// ===== 软件（apps）相关 =====

export interface ScannedApp {
  name: string;
  path: string;
  icon_path: string;
  args: string;
  source: string; // "StartMenu" / "Desktop" / "PATH" / "Folder" / "Document"
  app_type: string; // "app" / "folder" / "document" / "url"
  app_subtype: string; // P0-#Y#2：细分（game/office/dev/utility/media/design/other/word/excel/ppt/pdf/text/image/other）
  // P0-#Y#FIX#SCAN#LAUNCHABLE：能否直接启动
  // .exe/.lnk/.url/.bat/.cmd/.msi → true
  // 文件夹/文档/其他 → false
  is_launchable: boolean;
}

export interface AppMeta {
  id: number;
  name: string;
  path: string;
  icon_path: string;
  args: string;
  category_id: number | null;
  app_type: string; // "app" / "folder" / "document" / "url"
  app_subtype: string; // P0-#Y#2：细分（game/office/dev/utility/media/design/other/word/excel/ppt/pdf/text/image/other）
  use_count: number;
  last_used_at: number;
  created_at: number;
}

export interface AppCategoryMeta {
  id: number;
  name: string;
  icon: string;
  sort_order: number;
}

export const scanInstalledSoftware = () =>
  invoke<ScannedApp[]>("scan_installed_software");

/** 关键修复：自动补齐 db 里所有 icon_path 为空的 apps 图标
 *  AppView 加载时调一次，之后从 db 直接拿，不需要重扫 */
export const fillMissingIcons = () => invoke<number>("fill_missing_icons");

/** P0-#Y#FIX#ICON#REEXTRACT：用户反馈"图标不对"时强制清空 + 重新抽图 */
export const forceReextractIcons = () => invoke<number>("force_reextract_icons");

/** P0-#Y#FIX#ICON#RESTART：重启 app（让 webview 加载新代码） */
export const restartApp = () => invoke<void>("restart_app");

/** P0-#Y#FIX#VERSION#DISPLAY：返回当前 exe 版本号 */
export const getAppVersion = () => invoke<string>("get_app_version");

/// P0-#Y#3：一次性回填所有 app 的 subtype（启动时调用）
export const rescanSubtypes = () => invoke<number>("rescan_subtypes");

export const listApps = (query?: string, categoryId?: number) =>
  invoke<AppMeta[]>("list_apps", { query: query ?? "", categoryId });

// P0-#X：导入结果（区分新增/跳过/失败）
export interface ImportResult {
  new_ids: number[];
  skipped: string[];
  errors: string[];
}

export const importPaths = (paths: string[], categoryId?: number) =>
  invoke<ImportResult>("import_paths", { paths, categoryId });

export const listAppCategories = () =>
  invoke<AppCategoryMeta[]>("list_app_categories");

export const createAppCategory = (name: string, icon?: string) =>
  invoke<number>("create_app_category", { name, icon });

export const updateAppCategory = (id: number, name: string, icon: string) =>
  invoke<void>("update_app_category", { id, name, icon });

export const deleteAppCategory = (id: number) =>
  invoke<void>("delete_app_category", { id });

export const createApp = (params: {
  name: string;
  path: string;
  iconPath?: string;
  args?: string;
  categoryId?: number;
  appType?: string; // "app" / "folder" / "document" / "url"
  appSubtype?: string; // P0-#Y#2：细分
}) => invoke<number>("create_app", {
  name: params.name,
  path: params.path,
  iconPath: params.iconPath,
  args: params.args,
  categoryId: params.categoryId,
  appType: params.appType,
  appSubtype: params.appSubtype,
});

export const updateApp = (params: {
  id: number;
  name: string;
  path: string;
  iconPath?: string;
  args?: string;
  categoryId?: number;
  appSubtype?: string; // P0-#Y#2：细分
  appType?: string; // P0-#Y#FIX#TYPE#EDIT：一级分类（右键改分类用）
}) => invoke<void>("update_app", {
  id: params.id,
  name: params.name,
  path: params.path,
  iconPath: params.iconPath,
  args: params.args,
  categoryId: params.categoryId,
  appSubtype: params.appSubtype,
  appType: params.appType,
});

export const deleteApp = (id: number) =>
  invoke("delete_app", { id });

/** 读图标转 base64 data URL（绕过 Tauri asset protocol 加载失败） */
export const readIconAsDataUrl = (path: string): Promise<string | null> =>
  invoke("read_icon_as_data_url", { path });

/** P0-#ICON#NUCLEAR#ALL#IN#ONE：一次拿所有 app + 全部 icon data URL
 *  替代 loadIconDataUrls 缓存机制：零竞态、零异步等待、零 IPC 失败窗口 */
export interface AppWithIcon {
  id: number;
  name: string;
  path: string;
  icon_path: string;
  icon_data_url: string | null;
  args: string;
  category_id: number | null;
  app_type: string;
  app_subtype: string;
  use_count: number;
  last_used_at: number;
  created_at: number;
}
export const listAppsWithIcons = (
  query = "",
  categoryId?: number
): Promise<AppWithIcon[]> =>
  invoke<AppWithIcon[]>("list_apps_with_icons", { query, categoryId });

/** P0-#LOCK#POLLING#FALLBACK：查询后端 lock 状态 + build 时间戳 */
export interface AppStatus {
  unlocked: boolean;
  build_timestamp: string;
}
export const getAppStatus = (): Promise<AppStatus> =>
  invoke<AppStatus>("get_app_status");

export const recordAppUsage = (id: number) =>
  invoke<void>("record_app_usage", { id });

export const launchApp = (id: number) => invoke<string>("launch_app", { id });

/** P0-#Y#FIX#PICK：原生文件 / 文件夹选择对话框
 *  mode = "file" → 选 .exe/.lnk/.url 等
 *  mode = "folder" → 选文件夹
 *  返回用户选中的完整路径（用户取消返回 null）
 *  title 用来在对话框标题上显示提示文字 */
export const pickPath = (opts: { mode?: "file" | "folder"; title?: string } = {}) =>
  invoke<string | null>("pick_path", { mode: opts.mode || "folder", title: opts.title });

/** P0-#Y#FIX#MIGRATE：迁移数据到新位置
 * 复制 db + icons 缓存 + trash 表到 target_path
 * 不破坏旧数据，迁移完成后下次启动用新路径（需重启） */
export interface MigrateResult {
  files_copied: number;
  bytes_copied: number;
  source_dir: string;
  target_dir: string;
}
export const migrateData = (target: string) =>
  invoke<MigrateResult>("migrate_data", { targetPath: target });

/** P0-#Y#FIX#TRASH#HARD：硬清空回收站（立即永久删除所有项） */
export const hardPurgeTrash = () => invoke<number>("hard_purge_trash");

/** 通用设置存取（key-value，存到 db 的 settings 表） */
export const getSetting = (key: string) => invoke<string | null>("get_setting", { key });
export const setSetting = (key: string, value: string) =>
  invoke<void>("set_setting", { key, value });

// ===== 片段（snippets）相关 =====

export interface SnippetMeta {
  id: number;
  title: string;
  content: string;
  language: string;
  tags: string;
  use_count: number;
  last_used_at: number;
  created_at: number;
  updated_at: number;
}

export const listSnippets = (query?: string) =>
  invoke<SnippetMeta[]>("list_snippets", { query });

export const createSnippet = (params: {
  title: string;
  content: string;
  language?: string;
  tags?: string;
}) => invoke<number>("create_snippet", params);

export const updateSnippet = (params: {
  id: number;
  title: string;
  content: string;
  language?: string;
  tags?: string;
}) => invoke<void>("update_snippet", params);

export const deleteSnippet = (id: number) =>
  invoke<void>("delete_snippet", { id });

export const getSnippetContent = (id: number) =>
  invoke<string>("get_snippet_content", { id });

export const recordSnippetUsage = (id: number) =>
  invoke<void>("record_snippet_usage", { id });

// ===== 临时内容（temp_contents）相关 =====

export interface TempMeta {
  id: number;
  text: string;
  created_at: number;
  expires_at: number;
}

export const createTemp = (text: string, ttlMinutes: number) =>
  invoke<number>("create_temp", { text, ttlMinutes });

export const listTemp = () => invoke<TempMeta[]>("list_temp");

export const deleteTemp = (id: number) => invoke<void>("delete_temp", { id });

export const cleanupExpiredTemp = () =>
  invoke<number>("cleanup_expired_temp");

// ===== 回收站 (P0-#T) =====

export interface TrashItem {
  id: number;
  name: string;
  kind: "app" | "password" | "snippet" | "temp";
  kind_detail: string;
  sub_detail: string;
  deleted_at: number;
  source: string; // "软件" / "密码" / "命令行" / "便签"
}

export const listTrash = () => invoke<TrashItem[]>("list_trash");
export const restoreFromTrash = (table: string, id: number) =>
  invoke<number>("restore_from_trash", { table, id });
export const permanentDelete = (table: string, id: number) =>
  invoke<number>("permanent_delete", { table, id });
export const emptyTrash = (table: string) =>
  invoke<number>("empty_trash", { table });
export const cleanupTrash = (retentionDays: number) =>
  invoke<[string, number][]>("cleanup_trash", { retentionDays });
export const trashCount = () => invoke<number>("trash_count");

// kind → table 反向映射
export const kindToTable = (kind: TrashItem["kind"]): string => {
  switch (kind) {
    case "app": return "apps";
    case "password": return "passwords";
    case "snippet": return "snippets";
    case "temp": return "temp_contents";
  }
};

// P0-#F：启动健康检查 — 自动修复失效 app path
export const healthCheckApps = () => invoke<number>("health_check_apps");

// P0-#T#3：获取数据存储位置（SQLite 数据库路径）
export const getDataDir = () => invoke<string>("get_data_dir");

// ============ B1+B3：导入导出备份 + 重置数据 ============

/** B1-2：加密导出一份 .drawerbox 备份文件（返回路径+密码解密状态统计）*/
export interface ExportResult {
  path: string;
  passwords_decrypted_ok: number;
  passwords_decrypted_failed: number;
  trash_passwords: number;
}
export const exportEncryptedBackup = (masterPassword: string) =>
  invoke<ExportResult>("export_encrypted_backup", { masterPassword });

/** B1-3：按冲突策略导入 .drawerbox 备份（返回各表新增/覆盖数统计）*/
export type ConflictPolicy = "skip" | "overwrite" | "merge";
export interface ImportStats {
  settings: number;
  categories_inserted: number;
  categories_conflict: number;
  apps_inserted: number;
  apps_skipped: number;
  apps_overwritten: number;
  passwords_inserted: number;
  passwords_skipped: number;
  passwords_overwritten: number;
  snippets_inserted: number;
  snippets_skipped: number;
  snippets_overwritten: number;
  temps_inserted: number;
  temps_skipped: number;
  temps_overwritten: number;
  pinned_inserted: number;
  total_bytes: number;
}
export const importEncryptedBackup = (
  filePath: string,
  masterPassword: string,
  policy: ConflictPolicy,
) => invoke<ImportStats>("import_encrypted_backup", { filePath, masterPassword, policy });

/** B3：工厂还原（删除 db + 图标缓存 → 重启 app → 下次启动重新走设置向导）*/
export const factoryReset = () => invoke<void>("factory_reset");

export interface PasswordMeta {
  id: number;
  title: string;
  username: string;
  url: string;
  notes: string;
  created_at: number;
  updated_at: number;
  /// P1-#PW#USE#PERSIST：累计复制次数（与 apps / snippets 一致），持久化
  use_count?: number;
  last_used_at?: number;
}

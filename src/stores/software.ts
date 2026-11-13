import { defineStore } from "pinia";
import { ref, computed } from "vue";
import {
  bumpAfterIconRefresh,
  iconDataUrls as moduleIconDataUrls,
} from "../lib/app-icon-state";
import {
  listApps as apiListApps,
  listAppCategories as apiListAppCategories,
  createApp as apiCreateApp,
  updateApp as apiUpdateApp,
  deleteApp as apiDeleteApp,
  launchApp as apiLaunchApp,
  scanInstalledSoftware as apiScanInstalled,
  createAppCategory as apiCreateCategory,
  updateAppCategory as apiUpdateCategory,
  deleteAppCategory as apiDeleteCategory,
  importPaths as apiImportPaths,
  importScannedApps as apiImportScanned,
  scanCustomDir as apiScanCustomDir,
  cancelScan as apiCancelScan,
  fillMissingIcons as apiFillMissingIcons,
  forceReextractIcons as apiForceReextractIcons,
  listAppsWithIcons,
  type AppMeta,
  type ScannedApp,
  type AppCategoryMeta,
  listInvalidApps,
  batchMoveAppsToTrash,
  seedTestFixtures as apiSeedTestFixtures,
  clearTestFixtures as apiClearTestFixtures,
  type SeedFixturesResult,
  healthCheckApps,
  type InvalidAppInfo,
  type BatchTrashResult
} from "../api";
import { useAppStore } from "./app";

export const useSoftwareStore = defineStore("software", () => {
  const appStore = useAppStore();
  const items = ref<AppMeta[]>([]);
  const categories = ref<AppCategoryMeta[]>([]);
  const activeCategoryId = ref<number | null>(null); // null = 全部
  /// P0-#Y：类型筛选 null=全部 / "app" / "folder" / "document" / "url"
  const activeAppType = ref<string | null>(null);
  /// P0-#Y#2：细分筛选 null=全部 / "game" / "office" / "dev" / "utility" / "media" / "design" / "other" / "word" / "excel" ...
  const activeSubtype = ref<string | null>(null);
  const loading = ref(false);
  const error = ref("");
  // 关键修复 P1-#X：hasLoaded 用于 UI 区分"加载中" vs "加载完但确实为空"
  // 之前只靠 loading 状态判断，导致首次切到该 tab 时短暂 loading=true
  // + items.length=0 → 模板切到空状态（📦）一闪，体感"纸箱子闪屏"
  // 修复后：首次加载显示骨架（不闪），加载完确实为空才显示纸箱子
  const hasLoaded = ref(false);

  // 图标状态（M1-P2 F1）：data URL 受控回退缓存与失败/恢复状态统一收归
  // src/lib/app-icon-state.ts 模块（asset URL 版本化/失败链/单次恢复重试）。
  // store 仅保留同一引用的导出（AppView 模板与性能探针沿用 softwareStore.iconDataUrls）。
  const iconDataUrls = moduleIconDataUrls;

  async function loadCategories() {
    try {
      categories.value = await apiListAppCategories();
    } catch (e) {
      console.error("loadAppCategories failed", e);
    }
  }

  async function loadItems() {
    // 防并发：避免快速重复触发导致 loading 闪烁
    if (loading.value) return;
    loading.value = true;
    error.value = "";
    // 超时：8s 后强制结束（避免 IPC 卡死导致 UI 永远转）
    const timeoutPromise = new Promise<never>((_, reject) =>
      setTimeout(() => reject(new Error("加载超时（8s）")), 8000)
    );
    try {
      // G3-1B（性能架构升级）：元数据与图标分离。
      // 列表只取元数据（listApps）；图标由 <img :src="convertFileSrc(icon_path)">
      // 经 Tauri 资产协议加载（浏览器缓存、渐进解码，不进 IPC payload）。
      // base64 仅作为【按需回退】缓存（G3-1C：资产加载失败时由受控命令按 id 取一次）。
      // 旧接口 listAppsWithIcons 保留在后端作为兼容回退，前端不再默认调用。
      // 实测依据（G2.1-C）：图标 base64 占列表 payload 的 84.5%（500 条 ≈ 1.02MB）。
      const list = await Promise.race([
        apiListApps("", activeCategoryId.value ?? undefined),
        timeoutPromise,
      ]);
      items.value = list;
      iconDataUrls.value = {};
      hasLoaded.value = true;
    } catch (e) {
      error.value = String(e);
    } finally {
      loading.value = false;
    }
  }

  // O3：软件列表初始化去重——**视图 onMounted 专用入口**。
  // 语义：本进程内列表已加载（hasLoaded）且不在加载中 → 跳过
  //   （锁屏期间后端锁定、DB 零写入，跨锁屏数据不变 → 跳过安全）。
  // 与 loadItems 的分工（O3-0 报告 §五 的竞态结论）：
  //   - loadItems = 强制刷新（CRUD 写入 / O2 延迟扫描修复 / refreshAllIcons / reloadAll
  //     专用，**总是加载**——去重守卫绝不能放进 loadItems 本体，否则 O2 扫描的
  //     修复刷新会被吞掉，UI 与 DB 不一致）；
  //   - loadItemsIfNeeded = 初始化去重（仅 AppView/FrequentView 挂载时调用）。
  // 首次加载失败时 hasLoaded 保持 false → 后续调用仍会真正加载（可重试，不误标）。
  async function loadItemsIfNeeded() {
    if (hasLoaded.value && !loading.value) return;
    await loadItems();
  }

  // P1-#APPC#ICON#SYNC：图标重建互斥锁 + 一次性刷新接口
  // 之前问题：
  //   1. AppView onMounted 调 fillMissingIcons（耗时 10-30s）
  //   2. 用户切到设置 → 点"清空图标缓存 + 重新抽图" → 又调一次 forceReextractIcons + fillMissingIcons
  //   3. 两个调用并发跑，后端同时改 icon_path，前端 iconDataUrls 缓存和 db 不一致
  //   4. 抽完后 settings 没有清空 iconDataUrls 缓存 → UI 仍显示旧图标
  // 修复：
  //   - 全局互斥：iconBusyLock 期间其他调用 await busyPromise
  //   - 重建后清空 iconDataUrls + 重新 loadItems + loadIconDataUrls → 前后端一致
  //
  // P0-#ICON#REFRESH#SMART：智能刷新（避免不必要的清空）
  // 之前 bug：fillMissingIcons 返回 0（没需要抽的）→ 也清空 cache → UI ~30s 空白
  //   根因：AppView onMounted 无条件调 refreshAllIcons
  //   - 先 loadItems 触发 loadIconDataUrls → cache 填满
  //   - 再 refreshAllIcons → fillMissingIcons (即使 n=0) → 清 cache → 重 loadItems
  //   - 期间 UI 看到 fallback（"图标消失"现象）
  //   - 常见区 (FrequentView) 抢先把 cache 填了 → 软件区一进去就被清空
  // 修复：fillMissingIcons 返回 0 → 不清 cache，不重 load
  //        fillMissingIcons > 0 → 必须清（icon_path 变了，旧的 data URL 失效）
  // ===== G3-1C：受控图标回退（资产 URL 失败 → 按 id 定向取一次 base64） =====
  // M1-P2：受控回退/失败状态全部迁入 app-icon-state 模块（onAssetError 统一入口），
  // AppView/FrequentView 直接调用模块；store 不再持有失败状态。

  let iconBusyPromise: Promise<number> | null = null;
  async function refreshAllIcons(opts: { forceClear?: boolean } = {}): Promise<number> {
    if (iconBusyPromise) {
      // 已有任务在跑 → 等它完成（避免并发抽图）
      return iconBusyPromise;
    }
    iconBusyPromise = (async () => {
      try {
        if (opts.forceClear) {
          // 用户在设置点"清空图标缓存 + 重新抽图"：先清空 db 的 icon_path + 本地 PNG
          await apiForceReextractIcons();
        }
        // 抽图（fillMissingIcons 会跳过已有 icon_path 且文件存在的项）
        const n = await apiFillMissingIcons();
        if (n === 0 && !opts.forceClear) {
          // P0-#ICON#REFRESH#SMART：没需要抽的 → 保持现有 cache
          // 之前：无条件清空 cache → UI ~30s 空白（"图标消失"假象）
          return n;
        }
        // 清空前端缓存 → 否则 UI 还显示旧图标（G3-1C：同时清回退失败记录，允许重试）
        // M1-P2 F4：交给统一状态模块——n>0 时失败 icon 的 asset URL 版本化重新请求，
        // 文件恢复后无需 reload 即可显示（E1 闭环）；data URL 回退缓存同步清理。
        bumpAfterIconRefresh(n);
        // G3-1B：重新拉元数据（新 icon_path）→ 资产 URL 自动更新。
        // 修复：此处曾调用遗留 loadIconDataUrls(items) 把全部图标按旧通道
        // read_icon_as_data_url 逐条拉回 base64（G3-1 验证实测 500 条全部回填），
        // 使 M1 的分离形同虚设 —— 现已移除；图标展示交给资产 URL，失败走 G3-1C 受控回退。
        await loadItems();
        return n;
      } finally {
        iconBusyPromise = null;
      }
    })();
    return iconBusyPromise;
  }

  async function create(params: {
    name: string;
    path: string;
    iconPath?: string;
    args?: string;
    categoryId?: number;
    appType?: string; // "app" / "folder" / "document" / "url"
    appSubtype?: string; // P0-#Y#FIX#URL#SUB：细分（url-web / url-steam / url-epic / url-other / game / office / ...）
  }) {
    await apiCreateApp(params);
    await loadItems();
  }

  /// P0-#Y：拖入/选中多个路径，后端自动按扩展名或 .lnk 目标识别 app_type
  async function importPaths(paths: string[], categoryId?: number) {
    if (!paths.length) return { new_ids: [], skipped: [], errors: [] };
    const result = await apiImportPaths(paths, categoryId);
    await loadItems();
    return result;
  }

  async function update(params: {
    id: number;
    name: string;
    path: string;
    iconPath?: string;
    args?: string;
    categoryId?: number;
    appSubtype?: string; // P0-#Y#4：支持二次修改分类
  }) {
    await apiUpdateApp(params as any);
    await loadItems();
  }

  // P0-#Y#4：单独修改某个 app 的 subtype（不改其他字段）
  async function changeSubtype(id: number, appSubtype: string) {
    const it = items.value.find((x) => x.id === id);
    if (!it) return;
    await apiUpdateApp({
      id: it.id,
      name: it.name,
      path: it.path,
      iconPath: it.icon_path,
      args: it.args || "",
      categoryId: it.category_id,
      appSubtype,
    } as any);
    await loadItems();
  }

  // P0-#Y#FIX#TYPE#EDIT：单独修改某个 app 的 type（一级分类），不改其他字段
  async function changeType(id: number, appType: string) {
    const it = items.value.find((x) => x.id === id);
    if (!it) return;
    await apiUpdateApp({
      id: it.id,
      name: it.name,
      path: it.path,
      iconPath: it.icon_path,
      args: it.args || "",
      categoryId: it.category_id,
      appType,
    } as any);
    await loadItems();
  }

  async function remove(id: number) {
    await apiDeleteApp(id);
    await loadItems();
    // P0-#T：通知 appStore 刷新 trash count
    useAppStore().refreshTrashCount();
  }

  async function launch(id: number) {
    // C1（v0.4.3）：使用次数改由后端在启动请求受理成功后计数——
    // 旧实现此处先 record 再 launch，失败（文件缺失/重定位仍失败/ShellExecute 报错）
    // 也 +1，统计口径失真。删除前端预调用，避免双计数。
    const launched = items.value.find((x) => x.id === id);
    try {
      // P0-#APPC#PER#LAUNCH：启动成功后不再整表重拉。
      // 旧实现每次点击启动都 await loadItems() → listAppsWithIcons 全量返回
      // 全部条目 + 全部图标 base64（500 条 ≈ 600KB、真实库更大），点击→toast
      // 延迟随条目数线性恶化（实测 20→500 条：130ms→540ms，v0.4.3 用户实测同感）。
      // 常规成功路径只需同步该条使用计数（后端 record_app_usage 已落库，
      // 下次进入软件区/常用区自会取到真值；此处乐观 +1 保持界面即时一致）。
      const result: string = await apiLaunchApp(id);
      if (launched) {
        launched.use_count += 1;
      }
      // 自动重定位成功 → path 已被后端改写，需要刷新该条元数据（罕见路径，可接受一次全量拉取）
      if (result.startsWith("ok:auto-relocated:")) {
        await loadItems();
        const arrow = result.slice("ok:auto-relocated:".length);
        const [from, to] = arrow.split(" → ");
        return {
          ok: true,
          autoRelocated: true,
          message: `文件已自动定位到新位置：${to}`,
          from,
          to,
        };
      }
      if (result === "ok:steam-fallback") {
        return { ok: true, message: "已通过 Steam 库打开" };
      }
      return { ok: true };
    } catch (e: any) {
      // 失败路径保留全量刷新：available / 重定位后的 path 需要与后端对齐
      await loadItems();
      // P0-#F：精确错误信息
      const msg = String(e);
      if (msg.startsWith("relocated-but-still-fail:")) {
        return { ok: false, error: "missing", message: "文件搬迁到新位置但仍无法启动，请检查目标文件是否完整" };
      }
      if (msg.includes("找不到文件")) {
        return { ok: false, error: "missing", message: msg };
      }
      return { ok: false, error: "launch", message: msg };
    }
  }

  /** Batch 2.1/2.3：扫描候选（返回 ScanOutcome：apps + cancelled/truncated） */
  async function scan(
    level: "recommended" | "advanced",
    includePath = false,
    includeUwp = false
  ) {
    return await apiScanInstalled(level, includePath, includeUwp);
  }

  /** Batch 2.3：自定义目录扫描（便携软件 / 指定磁盘目录） */
  async function scanCustomDir(path: string) {
    return await apiScanCustomDir(path);
  }

  /** Batch 2.3：真正终止扫描 */
  async function cancelScan() {
    return await apiCancelScan();
  }

  /** Batch 2.1：批量导入扫描候选（幂等，返回 新增/已存在/回收站/失败 汇总） */
  async function importScanned(candidates: ScannedApp[], categoryId?: number) {
    return await apiImportScanned(candidates, categoryId);
  }

  async function createCategory(name: string, icon?: string) {
    await apiCreateCategory(name, icon);
    await loadCategories();
  }

  async function updateCategory(id: number, name: string, icon: string) {
    await apiUpdateCategory(id, name, icon);
    await loadCategories();
  }

  async function deleteCategory(id: number) {
    await apiDeleteCategory(id);
    // 如果当前正选这个分类，切回"全部"
    if (activeCategoryId.value === id) {
      activeCategoryId.value = null;
    }
    await loadCategories();
    // 重新加载 items（被引用的 app.category_id 已被置 NULL）
    await loadItems();
  }

  function setCategory(id: number | null) {
    activeCategoryId.value = id;
    loadItems();
  }

  // 客户端搜索过滤（不触发 DB 查询，避免闪屏）
  const filteredItems = computed(() => {
    const q = appStore.searchQuery.trim().toLowerCase();
    let result = items.value;
    // 分类过滤
    if (activeCategoryId.value != null) {
      result = result.filter((a) => a.category_id === activeCategoryId.value);
    }
    // 类型过滤（P0-#Y：app / folder / document / url）
    // P0-#Y#FIX#UNCAT#REMOVE：删掉"uncategorized"分支 — 侧边栏不再有"未分类"一级 chip
    if (activeAppType.value != null) {
      result = result.filter((a) => (a.app_type || "app") === activeAppType.value);
    }
    // 细分过滤（空字符串 / null 表示"全部细分"）
    // P0-#Y#FIX#UNCAT#REMOVE：删掉"uncategorized"分支 — 二级 chip 不再有"未细分"
    if (activeSubtype.value != null) {
      result = result.filter((a) => (a.app_subtype || "") === activeSubtype.value);
    }
    // 搜索过滤
    if (q) {
      result = result.filter(
        (a) =>
          a.name.toLowerCase().includes(q) ||
          a.path.toLowerCase().includes(q) ||
          (a.icon_path && a.icon_path.toLowerCase().includes(q))
      );
    }
    return result;
  });

  const filteredCount = computed(() => filteredItems.value.length);

  /// P0-#Y：切换 app_type 筛选（切大类型时重置细分）
  function setAppType(t: string | null) {
    activeAppType.value = t;
    activeSubtype.value = null;
  }
  /// P0-#Y#2：切换细分筛选（自动联动大类型：清空时也保留大类型）
  function setSubtype(s: string | null) {
    activeSubtype.value = s;
  }

  // 导入备份后整体刷新：分类 + 列表 + 图标缓存全部重拉
  async function reloadAll() {
    // M1-P2：旧 data URL 缓存作废 + 失败标记清零（新数据的路径由模块按 path 变化自动接管）
    iconDataUrls.value = {};
    bumpAfterIconRefresh(0);
    await loadCategories();
    await loadItems();
    // R4.1 闭环修复：恢复/扫描等 reloadAll 路径此前漏刷失效列表，
    // 导致软件区徽标停在旧值（实测：回收站恢复后徽标 失效(2)，后端实时 3）。
    // loadInvalidApps 内部仅置 invalidLoading，无 items 级防并发冲突，可安全并行。
    await loadInvalidApps();
  }

  // ===== v0.4.1 软件失效治理 =====
  const invalidItems = ref<InvalidAppInfo[]>([]);
  const invalidLoading = ref(false);
  const healthChecking = ref(false);
  const invalidCount = computed(() => invalidItems.value.length);
  // 可清退候选：仅固定盘确认缺失；移动盘/网络/离线/不确定一律保护
  const clearableInvalid = computed(() =>
    invalidItems.value.filter((it) => it.kind === "fixed_missing")
  );
  const protectedInvalid = computed(() =>
    invalidItems.value.filter((it) => it.kind !== "fixed_missing")
  );

  async function loadInvalidApps() {
    invalidLoading.value = true;
    try {
      invalidItems.value = await listInvalidApps();
    } catch (e) {
      console.warn("加载失效列表失败", e);
    } finally {
      invalidLoading.value = false;
    }
  }

  /**
   * 手动重新检查：先跑既有安全自动重定位（health_check_apps：.lnk 目标重写 +
   * Start Menu/Program Files/Steam 库按名重定位），再刷新列表与失效分类，
   * 让用户看到重定位后的最新状态。
   */
  async function runHealthCheck(): Promise<number> {
    healthChecking.value = true;
    try {
      const fixed = await healthCheckApps();
      await Promise.all([loadItems(), loadInvalidApps()]);
      return fixed;
    } finally {
      healthChecking.value = false;
    }
  }

  /** 批量移入回收站（软删除；幂等；结果含成功/跳过/逐条错误） */
  async function moveAppsToTrash(ids: number[]): Promise<BatchTrashResult> {
    const result = await batchMoveAppsToTrash(ids);
    await Promise.all([loadItems(), loadInvalidApps()]);
    return result;
  }

  /** 任务 A：播种验收测试数据（仅便携测试版；后端门禁） */
  async function seedTestFixtures(): Promise<SeedFixturesResult> {
    const r = await apiSeedTestFixtures();
    await Promise.all([loadItems(), loadInvalidApps()]);
    return r;
  }

  /** 任务 A：重置测试数据（软删除 [TEST] 记录，可从回收站恢复） */
  async function clearTestFixtures(): Promise<number> {
    const n = await apiClearTestFixtures();
    await Promise.all([loadItems(), loadInvalidApps()]);
    return n;
  }

  return {
    invalidItems,
    invalidLoading,
    healthChecking,
    invalidCount,
    clearableInvalid,
    protectedInvalid,
    loadInvalidApps,
    runHealthCheck,
    moveAppsToTrash,
    seedTestFixtures,
    clearTestFixtures,
    items,
    filteredItems,
    categories,
    activeCategoryId,
    activeAppType,
    activeSubtype,
    loading,
    error,
    hasLoaded,
    iconDataUrls,
    filteredCount,
    loadCategories,
    loadItems,
    loadItemsIfNeeded,
    reloadAll,
    // P1-#APPC#ICON#SYNC：暴露 refreshAllIcons 给 SettingsView/AppView 共用
    refreshAllIcons,
    create,
    update,
    changeSubtype,
    changeType,
    remove,
    launch,
    scan,
    scanCustomDir,
    cancelScan,
    importScanned,
    importPaths,
    createCategory,
    updateCategory,
    deleteCategory,
    setCategory,
    setAppType,
    setSubtype,
  };
});

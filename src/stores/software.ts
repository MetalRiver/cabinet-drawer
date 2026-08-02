import { defineStore } from "pinia";
import { ref, computed } from "vue";
import {
  listApps as apiListApps,
  listAppCategories as apiListAppCategories,
  createApp as apiCreateApp,
  updateApp as apiUpdateApp,
  deleteApp as apiDeleteApp,
  launchApp as apiLaunchApp,
  recordAppUsage as apiRecordAppUsage,
  scanInstalledSoftware as apiScanInstalled,
  createAppCategory as apiCreateCategory,
  updateAppCategory as apiUpdateCategory,
  deleteAppCategory as apiDeleteCategory,
  importPaths as apiImportPaths,
  readIconAsDataUrl,
  fillMissingIcons as apiFillMissingIcons,
  forceReextractIcons as apiForceReextractIcons,
  listAppsWithIcons,
  type AppMeta,
  type AppCategoryMeta,
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

  // 图标 data URL 缓存：id → "data:image/png;base64,..."
  // 关键修复：Tauri 2 asset protocol 加载本地 PNG 在 WebView2 不可靠，
  // 改为后端读 png → base64 data URL，前端 100% 能渲染
  const iconDataUrls = ref<Record<number, string>>({});

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
      // P0-#ICON#NUCLEAR#ALL#IN#ONE：用 listAppsWithIcons 替代 listApps + loadIconDataUrls
      // 之前：两次调用（listApps 拿元数据 + 17 次 readIconAsDataUrl 拿图标）
      //   → 任何一个 IPC 失败 / 时序错位 / WebView2 拒绝 base64 → UI 显示 fallback
      //   → 18 个紫色羽毛就是这么来的
      // 现在：一次调用拿全部（包括 base64 data URL）→ 零竞态、零等待、零失败窗口
      const list = await Promise.race([
        listAppsWithIcons("", activeCategoryId.value ?? undefined),
        timeoutPromise,
      ]);
      // 拆分：items 存元数据，iconDataUrls 存 data URL（保持现有 UI 兼容）
      items.value = list.map((it) => ({
        id: it.id,
        name: it.name,
        path: it.path,
        icon_path: it.icon_path,
        args: it.args,
        category_id: it.category_id,
        app_type: it.app_type,
        app_subtype: it.app_subtype,
        use_count: it.use_count,
        last_used_at: it.last_used_at,
        created_at: it.created_at,
      }));
      // P0-#DEBUG#APPTYPE：深度调试 - 打印从 DB 加载的每条记录的 app_type
      console.group("[softwareStore.loadItems] 从后端 list_apps_with_icons 加载完成，共", items.value.length, "条记录：");
      for (const it of items.value) {
        console.log(`  id=${it.id}  name=${JSON.stringify(it.name)}  path=${JSON.stringify(it.path)}  app_type=${JSON.stringify(it.app_type)}  app_subtype=${JSON.stringify(it.app_subtype)}`);
      }
      // 统计各类数量
      const counts = items.value.reduce((acc, x) => {
        const t = x.app_type || "EMPTY_STRING_DB_BUG";
        acc[t] = (acc[t] || 0) + 1;
        return acc;
      }, {} as Record<string, number>);
      console.log("📊 app_type 分类统计：", counts);
      console.groupEnd();
      // 一次性填满 cache（所有 data URL 已经在 list 里了）
      const newCache: Record<number, string> = {};
      for (const it of list) {
        if (it.icon_data_url) {
          newCache[it.id] = it.icon_data_url;
        }
      }
      iconDataUrls.value = newCache;
      hasLoaded.value = true;
    } catch (e) {
      error.value = String(e);
    } finally {
      loading.value = false;
    }
  }

  /// 关键修复：异步批量把 icon_path 转 data URL
  /// 不阻塞列表渲染：fire-and-forget，9 个软件 ~10-30ms 完成
  /// 失败项静默忽略（前端走 v-else 显示首字母卡片）
  async function loadIconDataUrls(list: AppMeta[]) {
    const promises = list
      .filter((a) => a.icon_path && !iconDataUrls.value[a.id])
      .map(async (a) => {
        try {
          const dataUrl = await readIconAsDataUrl(a.icon_path);
          if (dataUrl) {
            iconDataUrls.value = { ...iconDataUrls.value, [a.id]: dataUrl };
          }
        } catch {
          // 静默忽略
        }
      });
    await Promise.all(promises);
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
        // 清空前端缓存 → 否则 UI 还显示旧图标
        iconDataUrls.value = {};
        // 重新拉列表（带新 icon_path） + 重新加载 data URLs
        await loadItems();
        // loadItems 末尾已经 void loadIconDataUrls(items)，这里再等一次保证全部完成
        await loadIconDataUrls(items.value);
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
    // P0-#DEBUG#APPTYPE：深度调试 - 打印传给后端 createApp 的完整参数
    console.log("[softwareStore.create] 传入参数 =", JSON.stringify(params, null, 2));
    console.log("[softwareStore.create] 重点检查：appType =", params.appType, "| 类型 =", typeof params.appType);
    await apiCreateApp(params);
    await loadItems();
    // P0-#DEBUG#APPTYPE：重新加载后检查新记录在 items 列表里的 app_type 实际值
    const match = items.value.findLast((x) => x.path === params.path && x.name === params.name);
    if (match) {
      console.log(`[softwareStore.create] ✅ 数据库已写入：id=${match.id}, name=${match.name}, path=${match.path}, DB.app_type=${JSON.stringify(match.app_type)}`);
      if (match.app_type !== params.appType) {
        console.warn(`[softwareStore.create] ⚠️  app_type MISMATCH! 前端传 ${JSON.stringify(params.appType)}，DB 实际存 ${JSON.stringify(match.app_type)}`);
      }
    } else {
      console.warn("[softwareStore.create] ⚠️  写入后未在 items 中找到匹配项（可能 loadItems 尚未刷新完成）");
      console.log("[softwareStore.create] 当前 items 所有记录的 app_type：", items.value.map(x => ({id:x.id, name:x.name, app_type: x.app_type})));
    }
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
    await apiRecordAppUsage(id);
    try {
      // P0-#F#RUNTIME：后端可能返回 "ok" / "ok:auto-relocated:旧 → 新" / "ok:steam-fallback"
      const result: string = await apiLaunchApp(id);
      await loadItems();
      // 自动重定位成功 → 通知前端 toast（让用户知道"路径已自动更新"）
      if (result.startsWith("ok:auto-relocated:")) {
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

  async function scan() {
    return await apiScanInstalled();
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
    iconDataUrls.value = {};
    await loadCategories();
    await loadItems();
  }

  return {
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
    loadIconDataUrls,
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
    importPaths,
    createCategory,
    updateCategory,
    deleteCategory,
    setCategory,
    setAppType,
    setSubtype,
  };
});

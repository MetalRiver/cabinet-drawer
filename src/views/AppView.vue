<script setup lang="ts">
import { ref, onMounted, onUnmounted, computed, h, nextTick } from "vue";
import { convertFileSrc } from "@tauri-apps/api/core";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { useAppStore } from "../stores/app";
import { useSoftwareStore } from "../stores/software";
import { type ScannedApp } from "../api";

// P0-#Y#FIX#ICON#FOLDER：folder/url/document 没真实图标时显示内置 SVG（Windows 风格）
// 之前：emoji 📁 / 🔗 / 📄（不真实，看起来像字符而不是图标）
// 现在：inline SVG 接近 Windows 原生视觉，立体感、配色一致
const FolderIconSvg = () =>
  h("svg", { viewBox: "0 0 64 64", xmlns: "http://www.w3.org/2000/svg" }, [
    h("defs", null, [
      h("linearGradient", { id: "folder-back", x1: "0%", y1: "0%", x2: "0%", y2: "100%" }, [
        h("stop", { offset: "0%", "stop-color": "#FFD466" }),
        h("stop", { offset: "100%", "stop-color": "#F5A623" }),
      ]),
      h("linearGradient", { id: "folder-front", x1: "0%", y1: "0%", x2: "0%", y2: "100%" }, [
        h("stop", { offset: "0%", "stop-color": "#FFE082" }),
        h("stop", { offset: "100%", "stop-color": "#FFC044" }),
      ]),
    ]),
    // 后面板
    h("path", { d: "M6 18 L26 18 L30 22 L58 22 L58 50 L6 50 Z", fill: "url(#folder-back)" }),
    // 前面板
    h("path", { d: "M6 22 L58 22 L58 52 Q58 54 56 54 L8 54 Q6 54 6 52 Z", fill: "url(#folder-front)" }),
    // 高光
    h("path", { d: "M8 24 L56 24 L56 28 L8 28 Z", fill: "rgba(255,255,255,0.15)" }),
  ]);

const UrlIconSvg = () =>
  h("svg", { viewBox: "0 0 64 64", xmlns: "http://www.w3.org/2000/svg" }, [
    h("defs", null, [
      h("radialGradient", { id: "globe-grad", cx: "50%", cy: "50%", r: "50%" }, [
        h("stop", { offset: "0%", "stop-color": "#7DD3FC" }),
        h("stop", { offset: "100%", "stop-color": "#0284C7" }),
      ]),
    ]),
    h("circle", { cx: "32", cy: "32", r: "26", fill: "url(#globe-grad)" }),
    // 经线
    h("ellipse", { cx: "32", cy: "32", rx: "12", ry: "26", fill: "none", stroke: "rgba(255,255,255,0.5)", "stroke-width": "1.5" }),
    h("ellipse", { cx: "32", cy: "32", rx: "22", ry: "26", fill: "none", stroke: "rgba(255,255,255,0.4)", "stroke-width": "1.5" }),
    h("line", { x1: "6", y1: "32", x2: "58", y2: "32", stroke: "rgba(255,255,255,0.5)", "stroke-width": "1.5" }),
    h("path", { d: "M10 22 Q32 28 54 22", fill: "none", stroke: "rgba(255,255,255,0.4)", "stroke-width": "1.5" }),
    h("path", { d: "M10 42 Q32 36 54 42", fill: "none", stroke: "rgba(255,255,255,0.4)", "stroke-width": "1.5" }),
  ]);

const DocumentIconSvg = () =>
  h("svg", { viewBox: "0 0 64 64", xmlns: "http://www.w3.org/2000/svg" }, [
    h("defs", null, [
      h("linearGradient", { id: "doc-grad", x1: "0%", y1: "0%", x2: "0%", y2: "100%" }, [
        h("stop", { offset: "0%", "stop-color": "#F1F5F9" }),
        h("stop", { offset: "100%", "stop-color": "#CBD5E1" }),
      ]),
    ]),
    h("path", { d: "M14 6 L42 6 L54 18 L54 58 L14 58 Z", fill: "url(#doc-grad)" }),
    h("path", { d: "M42 6 L42 18 L54 18 Z", fill: "#94A3B8" }),
    h("line", { x1: "20", y1: "28", x2: "48", y2: "28", stroke: "#64748B", "stroke-width": "1.5" }),
    h("line", { x1: "20", y1: "36", x2: "48", y2: "36", stroke: "#64748B", "stroke-width": "1.5" }),
    h("line", { x1: "20", y1: "44", x2: "40", y2: "44", stroke: "#64748B", "stroke-width": "1.5" }),
  ]);

const appStore = useAppStore();
const softwareStore = useSoftwareStore();

// 扫描模态框
const showScanModal = ref(false);
const scannedApps = ref<ScannedApp[]>([]);
const scanning = ref(false);
const selectedPaths = ref<Set<string>>(new Set());
const scanSearch = ref(""); // 扫描内搜索
const scanTargetCategoryId = ref<number | undefined>(undefined); // 扫描导入目标分类
// 自定义下拉：目标分类

// P0-#Z：行内 confirm（统一 5 区风格）
const confirmingDeleteId = ref<number | null>(null);
const confirmingDeleteName = ref<string>("");
const deletingId = ref<number | null>(null);
const scanTargetDropdownOpen = ref(false);

// P0-#Y：拖拽状态（系统级拖入，应用级生效；不仅限本 tab）
const isDraggingOver = ref(false);
let unlistenDragDrop: (() => void) | null = null;

// 左侧 sidebar 展开状态：默认收起（仅图标列），hover 时展开显示文字 + 二级
const sidebarExpanded = ref(false);

// 修复 P1-#X：图标加载失败追踪。db.icon_path 不为空但 webview 访问失败时
// 直接切到首字母彩色卡片，避免显示灰方块
const iconError = ref<Record<number, boolean>>({});
function onAppIconError(id: number) {
  iconError.value = { ...iconError.value, [id]: true };
}
// 扫描结果专用：路径级别（不是 db id），用于扫描模态框里的图标
const scanIconError = ref<Record<string, boolean>>({});
function onScanIconError(path: string) {
  scanIconError.value = { ...scanIconError.value, [path]: true };
}

// 表单：手动添加
const showAddForm = ref(false);
const addForm = ref({
  name: "",
  path: "",
  args: "",
  categoryId: undefined as number | undefined,
});

// 编辑
const editingId = ref<number | null>(null);
const editForm = ref({
  name: "",
  path: "",
  args: "",
  categoryId: undefined as number | undefined,
  appSubtype: "" as string, // P0-#Y#5：细分分类
});

// P0-#Y#5：编辑时根据当前 item 的 app_type 返回可选的 subtype 列表
const _editSubtypeOptions = computed(() => {
  if (editingId.value == null) return [];
  const it = softwareStore.items.find((x) => x.id === editingId.value);
  if (!it) return [];
  if (it.app_type === "app") return APP_SUBTYPE_OPTIONS;
  if (it.app_type === "document") return DOC_SUBTYPE_OPTIONS;
  return [];
});

// P0-#Y#4：右键菜单 - 修改单条 app 的细分分类
interface ContextMenu {
  visible: boolean;
  x: number;
  y: number;
  item: any | null;
}
const ctxMenu = ref<ContextMenu>({ visible: false, x: 0, y: 0, item: null });
const ctxMenuEl = ref<HTMLElement | null>(null);
// P1-#APPC#CTX#EDGE：右键菜单边缘裁切修复
// 之前：固定 menuW=180, menuH=240 + Math.min 钳位 → 实际菜单高度会变（type/subtype 选项数不同）
//       → 在窗口底部/右侧点击时，菜单下半部分被窗口裁切看不见
//       → 窗口尺寸变化时（拉宽/拉窄）菜单位置不更新
// 现在：渲染后用 ctxMenuEl.getBoundingClientRect() 拿真实尺寸，4 边全部钳位
//       窗口 resize / DPR 变化时重新定位
function clampMenuPos(x: number, y: number, w: number, h: number) {
  const vw = window.innerWidth;
  const vh = window.innerHeight;
  // 4 边留 4px 边距，避免紧贴边缘
  const minMargin = 4;
  // 右下超出 → 反向靠
  let nx = Math.min(x, vw - w - minMargin);
  let ny = Math.min(y, vh - h - minMargin);
  // 左上超出 → 拉回
  if (nx < minMargin) nx = minMargin;
  if (ny < minMargin) ny = minMargin;
  return { x: Math.max(0, nx), y: Math.max(0, ny) };
}
function openContextMenu(e: MouseEvent, item: any) {
  e.preventDefault();
  e.stopPropagation();
  // 先以一个保守估计值定位（180×280），下一帧用真实尺寸再钳位
  const estW = 180;
  const estH = 280;
  const { x, y } = clampMenuPos(e.clientX, e.clientY, estW, estH);
  ctxMenu.value = { visible: true, x, y, item };
  // nextTick 拿到真实 DOM 尺寸后再校正一次
  void nextTick(() => {
    if (!ctxMenuEl.value) return;
    const rect = ctxMenuEl.value.getBoundingClientRect();
    if (rect.width === 0 || rect.height === 0) return;
    const fixed = clampMenuPos(x, y, rect.width, rect.height);
    if (fixed.x !== x || fixed.y !== y) {
      ctxMenu.value = { ...ctxMenu.value, x: fixed.x, y: fixed.y };
    }
  });
}
function closeContextMenu() {
  ctxMenu.value = { ...ctxMenu.value, visible: false };
}
// 窗口尺寸变化时重新钳位（避免菜单跑出可视区）
function repositionCtxMenu() {
  if (!ctxMenu.value.visible || !ctxMenuEl.value) return;
  const rect = ctxMenuEl.value.getBoundingClientRect();
  if (rect.width === 0 || rect.height === 0) return;
  const fixed = clampMenuPos(ctxMenu.value.x, ctxMenu.value.y, rect.width, rect.height);
  if (fixed.x !== ctxMenu.value.x || fixed.y !== ctxMenu.value.y) {
    ctxMenu.value = { ...ctxMenu.value, x: fixed.x, y: fixed.y };
  }
}
async function pickSubtype(sub: string) {
  // P0-#Y#FIX#UNCAT：右键菜单里"未细分"是筛选态不是真值，挡住
  if (sub === "uncategorized") {
    closeContextMenu();
    return;
  }
  const it = ctxMenu.value.item;
  if (!it) return;
  try {
    await softwareStore.changeSubtype(it.id, sub);
    appStore.showClipToast("success", `已归类为「${subtypeLabel(sub)}」`);
  } catch (e) {
    appStore.showClipToast("info", "修改失败：" + String(e));
  } finally {
    closeContextMenu();
  }
}

// P0-#Y#FIX#TYPE#EDIT：右键改一级分类
async function pickType(newType: string | null) {
  if (!newType) {
    closeContextMenu();
    return;
  }
  const it = ctxMenu.value.item;
  if (!it) return;
  if (it.app_type === newType) {
    closeContextMenu();
    return;
  }
  try {
    await softwareStore.changeType(it.id, newType);
    appStore.showClipToast("success", `已归为「${typeLabel(newType)}」`);
  } catch (e) {
    appStore.showClipToast("info", "修改失败：" + String(e));
  } finally {
    closeContextMenu();
  }
}
// 当前右键项的可选 subtype（按 app_type 过滤）
const ctxSubtypeOptions = computed(() => {
  const it = ctxMenu.value.item;
  if (!it) return [];
  if (it.app_type === "app") return APP_SUBTYPE_OPTIONS;
  if (it.app_type === "document") return DOC_SUBTYPE_OPTIONS;
  return []; // folder / url 没有细分
});
// 修复 P0-#Y#FIX#UNCAT：右键菜单不应包含"未细分"项（这是筛选态，不是 subtype 本身）
const ctxSubtypeOptionsNoUncat = computed(() =>
  ctxSubtypeOptions.value.filter((s) => s.key !== "uncategorized")
);

// P0-#ICON#MISSING#HINT：图标缺失检测
// 计算有多少个 app（type=app/folder/document/url）没拿到 data URL
// > 0 时 header 显示"重抽图标"按钮（用户手动触发兜底闭环）
const missingIconCount = computed(() => {
  return softwareStore.items.filter(
    (it) => it.icon_path && !softwareStore.iconDataUrls[it.id]
  ).length;
});
const reextractingIcons = ref(false);
async function reextractIcons() {
  if (reextractingIcons.value) return;
  reextractingIcons.value = true;
  try {
    // forceClear=true：清空 db.icon_path + 本地 PNG → 强制重抽所有
    const n = await softwareStore.refreshAllIcons({ forceClear: true });
    appStore.showClipToast(
      "success",
      n > 0 ? `已重新抽取 ${n} 个图标` : "图标已是最新"
    );
  } catch (e) {
    appStore.showClipToast("info", "重抽失败：" + String(e));
  } finally {
    reextractingIcons.value = false;
  }
}

// P0-#ICON#DEBUG#OVERLAY：诊断浮层（按 Alt+Shift+D 临时切换）
// 原因：用户报"图标变紫色羽毛"，但 cache 文件 + db 路径都对，需要看实际渲染状态
const showIconDebug = ref(false);
// ✅ 命名处理器（原为匿名监听且从不清理，锁屏-解锁每轮累积一份）
const onIconDebugKeydown = (e: KeyboardEvent) => {
  if (e.altKey && e.shiftKey && (e.key === "D" || e.key === "d")) {
    e.preventDefault();
    showIconDebug.value = !showIconDebug.value;
  }
};
onMounted(() => {
  document.addEventListener("keydown", onIconDebugKeydown);
});
const debugSample = computed(() => {
  // 拿前 5 个 app 类型的 item 状态给用户看
  const apps = softwareStore.items.filter(
    (i) => (i.app_type || "app") === "app"
  ).slice(0, 5);
  return apps.map((it) => ({
    id: it.id,
    name: it.name,
    icon_path: it.icon_path,
    hasDataUrl: !!softwareStore.iconDataUrls[it.id],
    dataUrlLen: (softwareStore.iconDataUrls[it.id] || "").length,
    dataUrlPrefix: (softwareStore.iconDataUrls[it.id] || "").slice(0, 60),
    iconError: !!iconError.value[it.id],
  }));
});

onMounted(() => {
  // 修复 P1：fire-and-forget，不再 await，避免首次切到该 tab 时整个 setup 阻塞
  // IPC 慢的 view 不应该让路由组件等数据，应该先渲染骨架（空态）再异步填充
  void softwareStore.loadCategories();
  void softwareStore.loadItems();
  // 关键修复：自动补齐 db 里所有 icon_path 为空的 apps 图标
  // 用户不需要每次点"扫描"才能看到图标 —— 首次进软件区时后台抽图存 db
  // 抽完刷新列表，图标会显示出来
  // P1-#APPC#ICON#SYNC：改用 softwareStore.refreshAllIcons()
  //   - 与 SettingsView 走同一条路径，全局互斥避免并发
  //   - 自动清空 iconDataUrls 缓存 + 重新加载
  // P0-#ICON#MISSING#AUTOHEAL：配合后端 fill_missing_icons 新增的"文件丢失自愈"逻辑
  //   - db.icon_path 不为空但 PNG 文件丢失 → 自动重抽
  //   - 用户即使手动清过 cache 目录也能恢复
  void softwareStore.refreshAllIcons();

  // P0-#Y：注册系统级拖拽监听（Tauri 2 webview 事件）
  // 关键修复：HTML5 drag&drop 也作为兜底，防止 transparent 窗口下 Tauri 事件不响应
  let dragCounter = 0;
  const onNativeDragOver = (e: DragEvent) => {
    e.preventDefault();
    if (e.dataTransfer) e.dataTransfer.dropEffect = "copy";
    isDraggingOver.value = true;
  };
  const onNativeDragEnter = (e: DragEvent) => {
    e.preventDefault();
    dragCounter++;
    isDraggingOver.value = true;
  };
  const onNativeDragLeave = (e: DragEvent) => {
    e.preventDefault();
    dragCounter = Math.max(0, dragCounter - 1);
    if (dragCounter === 0) isDraggingOver.value = false;
  };
  const onNativeDrop = async (e: DragEvent) => {
    e.preventDefault();
    dragCounter = 0;
    isDraggingOver.value = false;
    const files = e.dataTransfer?.files;
    console.log("[drag:drop] native event, files.length =", files?.length ?? 0);
    if (!files || files.length === 0) return;
    // Tauri 2 会把 webkitRelativePath 写成空，但 path 属性包含 Windows 路径
    const paths: string[] = [];
    for (let i = 0; i < files.length; i++) {
      const f = files[i] as any;
      const p: string = f.path || (f as any).name || "";
      if (p) paths.push(p);
    }
    // P0-#V：拖拽去重 + 详细日志
    const uniquePaths = [...new Set(paths.map((p) => p.toLowerCase()))];
    console.log("[drag:drop] paths:", paths, "→ unique:", uniquePaths);
    if (!uniquePaths.length) {
      appStore.showClipToast("info", "无法识别拖入的文件路径");
      return;
    }
    try {
      const result = await softwareStore.importPaths(uniquePaths);
      console.log("[drag:drop] importPaths →", result);
      // P0-#X：精确 toast 反馈
      const newCount = result.new_ids.length;
      const skipCount = result.skipped.length;
      const errCount = result.errors.length;
      // P1-#APPC#ICON#SYNC：改用 store.refreshAllIcons() 走互斥路径
      void softwareStore.refreshAllIcons();
      void appStore.showClipToast(
        "success",
        `已导入 ${newCount} 个（跳过 ${skipCount}）`
      );
      // 拼 toast：区分新增/跳过/失败
      const parts: string[] = [];
      if (newCount > 0) parts.push(`✅ 新增 ${newCount}`);
      if (skipCount > 0) parts.push(`⏭️ 跳过 ${skipCount}（已存在）`);
      if (errCount > 0) parts.push(`❌ 失败 ${errCount}`);
      if (parts.length === 0) parts.push("无变化");
      appStore.showClipToast(newCount > 0 ? "success" : "info", `已导入：${parts.join(" · ")}`);
    } catch (err) {
      console.error("[drag:drop] importPaths failed", err);
      appStore.showClipToast("info", "导入失败：" + String(err));
    }
  };

  window.addEventListener("dragover", onNativeDragOver);
  window.addEventListener("dragenter", onNativeDragEnter);
  window.addEventListener("dragleave", onNativeDragLeave);
  window.addEventListener("drop", onNativeDrop);

  void (async () => {
    try {
      unlistenDragDrop = await getCurrentWebview().onDragDropEvent(async (event) => {
        const p = event.payload as any;
        if (p.type === "over") {
          isDraggingOver.value = true;
        } else if (p.type === "leave") {
          isDraggingOver.value = false;
        } else if (p.type === "drop") {
          isDraggingOver.value = false;
          const paths: string[] = Array.isArray(p.paths) ? p.paths : [];
          console.log("[drag:drop] tauri event, paths:", paths);
          if (!paths.length) return;
          const uniquePaths = [...new Set(paths.map((p) => p.toLowerCase()))];
          try {
            const result = await softwareStore.importPaths(uniquePaths);
            console.log("[drag:drop] tauri importPaths →", result);
            const newCount = result.new_ids.length;
            const skipCount = result.skipped.length;
            const errCount = result.errors.length;
            // P1-#APPC#ICON#SYNC：改用 store.refreshAllIcons() 走互斥路径
            void softwareStore.refreshAllIcons();
            const parts: string[] = [];
            if (newCount > 0) parts.push(`✅ 新增 ${newCount}`);
            if (skipCount > 0) parts.push(`⏭️ 跳过 ${skipCount}（已存在）`);
            if (errCount > 0) parts.push(`❌ 失败 ${errCount}`);
            if (parts.length === 0) parts.push("无变化");
            appStore.showClipToast(newCount > 0 ? "success" : "info", `已导入：${parts.join(" · ")}`);
          } catch (e) {
            console.error("[drag:drop] tauri importPaths failed", e);
            appStore.showClipToast("info", "导入失败：" + String(e));
          }
        }
      });
    } catch (e) {
      console.warn("[AppView] 监听系统拖拽事件失败", e);
    }
  })();

  // ✅ 泄漏修复：在 mounted 上下文注册卸载清理（Vue 3 支持钩子内注册钩子）。
  // 原 (onUnmounted as any).__dragCleanup 是伪清理写法（给函数对象赋属性），永不执行，
  // 导致 4 个拖拽监听随每次锁屏-解锁循环累积一份。
  onUnmounted(() => {
    window.removeEventListener("dragover", onNativeDragOver);
    window.removeEventListener("dragenter", onNativeDragEnter);
    window.removeEventListener("dragleave", onNativeDragLeave);
    window.removeEventListener("drop", onNativeDrop);
  });

  // P0-#Y#4：右键菜单 - 全局监听点击/滚动关闭
  // 注意：contextmenu 上要 stopPropagation，否则这里的 listener 会立即把它关掉
  window.addEventListener("click", closeContextMenu);
  window.addEventListener("contextmenu", closeContextMenu);
  window.addEventListener("scroll", closeContextMenu, true);
  // P1-#APPC#CTX#EDGE：窗口 resize 时重新钳位右键菜单位置
  window.addEventListener("resize", repositionCtxMenu);

  window.addEventListener("click", onDocClick);
});

// 自定义下拉"导入到"点击外部关闭（提升到 setup 作用域，供 onUnmounted 移除；
// 原定义在 onMounted 内且用 (onUnmounted as any).__ddCleanup 伪清理，永不执行）
const onDocClick = (e: MouseEvent) => {
  if (!scanTargetDropdownOpen.value) return;
  const target = e.target as HTMLElement;
  if (!target.closest(".scan-target-dd")) {
    scanTargetDropdownOpen.value = false;
  }
};

onUnmounted(() => {
  unlistenDragDrop?.();
  unlistenDragDrop = null;
  // P1-#APPC#CTX#EDGE：清理 resize 监听
  window.removeEventListener("resize", repositionCtxMenu);
  // ✅ 泄漏修复：以下监听此前从未被移除，随锁屏-解锁循环持续累积
  window.removeEventListener("click", closeContextMenu);
  window.removeEventListener("contextmenu", closeContextMenu);
  window.removeEventListener("scroll", closeContextMenu, true);
  window.removeEventListener("click", onDocClick);
  document.removeEventListener("keydown", onIconDebugKeydown);
});

// 关键修复 P1-#X：删除 watch(() => appStore.searchQuery, () => loadItems())！
// 原因：
// 1. 搜索过滤已在 store 内通过 computed (filteredItems) 客户端完成
// 2. 之前这里 watch 会导致每次输入都打 DB → loading=true → 模板
//    `v-if="loading && items.length === 0"` 触发骨架，
//    紧接着 `v-else-if="filteredItems.length === 0"` 触发空状态（📦）
//    → 反复切换 = "纸箱子闪屏"
// 3. 搜索已不再影响是否加载数据，只影响客户端过滤结果
//    删掉 watch 后：首次加载只看 loading+hasLoaded，搜索时直接 filter 不会触发布局重排

// ===== 真实图标渲染（核心：convertFileSrc 包装 Windows 路径） =====
// Tauri assetProtocol 必须开启（tauri.conf.json 已配）
// icon_path 形如 "C:\Windows\System32\notepad.exe" → convertFileSrc 转 "http://asset.localhost/..."
function iconSrc(iconPath: string | null | undefined): string {
  if (!iconPath) return "";
  // 已经是 http(s) / data: 开头直接返回
  if (iconPath.startsWith("http") || iconPath.startsWith("data:")) return iconPath;
  try {
    return convertFileSrc(iconPath);
  } catch {
    return "";
  }
}

// 用户反馈：hash 颜色难看，删了，改用类型本身辨识
// 应用 → 💻 灰底、网址 → 🔗 灰底、文件夹 → 📁 灰底、文档 → 📄 灰底、其他 → 📦 灰底
function scanTypeIcon(a: ScannedApp): string {
  const t = a.app_type || "app";
  if (t === "url") return "🔗";
  if (t === "folder") return "📁";
  if (t === "document") return "📄";
  // app 类型：用户只关心能否启动
  return a.is_launchable ? "💻" : "📦";
}

// 用户反馈：扫描顶部加 checkbox
// 4 种类型：launchable（可启动项）/ folder / document / other
const SCAN_TYPE_FILTERS = [
  { key: "launchable", label: "启动项", icon: "💻" },
  { key: "folder",     label: "文件夹", icon: "📁" },
  { key: "document",   label: "文档",   icon: "📄" },
  { key: "other",      label: "其他",   icon: "📦" },
] as const;

// 4 个 checkbox 状态（默认全选）
const scanTypeFilter = ref<Record<string, boolean>>({
  launchable: true,
  folder: false,    // 默认**不**显示文件夹（噪音大、用户主要关心启动项）
  document: false,  // 同上
  other: false,     // 同上
});

// 按类型计数（用于 checkbox 后面的数字）
const countByType = computed(() => {
  const c = { launchable: 0, folder: 0, document: 0, other: 0 };
  for (const a of scannedApps.value) {
    if (a.is_launchable) c.launchable++;
    else if (a.app_type === "folder") c.folder++;
    else if (a.app_type === "document") c.document++;
    else c.other++;
  }
  return c;
});

// 扫描结果：搜索过滤
// 扫描结果：搜索 + 类型过滤
const filteredScannedApps = computed(() => {
  let list = scannedApps.value;
  // 用户反馈：checkbox 过滤
  list = list.filter((a) => {
    if (a.is_launchable) return scanTypeFilter.value.launchable;
    if (a.app_type === "folder") return scanTypeFilter.value.folder;
    if (a.app_type === "document") return scanTypeFilter.value.document;
    return scanTypeFilter.value.other;
  });
  const q = scanSearch.value.trim().toLowerCase();
  if (!q) return list;
  return list.filter(
    (a) =>
      a.name.toLowerCase().includes(q) ||
      a.path.toLowerCase().includes(q) ||
      a.source.toLowerCase().includes(q)
  );
});

// 软件区展示：直接用 store 的客户端 filter 结果（搜索/分类/类型都已在 store 内处理）
const filteredItems = computed(() => softwareStore.filteredItems);

// P0-#Y：app_type → 中文 + emoji（侧边栏一级 chip）
// P0-#Y#FIX#UNCAT#REMOVE：删除"未分类"一级
//  原因：用户要求"侧边栏都有归属对应，而不是有些都是虚拟的"
//  之前：📦 未分类 单独占一个 chip（占空间 + 容易和"未细分"二级混淆）
//  现在：5 个一级：✨ 全部 / 🚀 应用(软件) / 🔗 链接 / 📁 文件夹 / 📄 文档
const TYPE_OPTIONS: { key: string | null; label: string; emoji: string }[] = [
  { key: null, label: "全部", emoji: "✨" },
  { key: "app", label: "应用", emoji: "🚀" },
  // P0-#Y#FIX#LABEL#URL：把"链接"改成"网址"，更直白
  { key: "url", label: "网址", emoji: "🔗" },
  { key: "folder", label: "文件夹", emoji: "📁" },
  { key: "document", label: "文档", emoji: "📄" },
];
const TYPE_LABEL: Record<string, string> = {
  app: "应用",
  folder: "文件夹",
  document: "文档",
  // P0-#Y#FIX#LABEL#URL：链接 → 网址
  url: "网址",
};
function typeEmoji(t: string | undefined): string {
  if (!t) return "🚀";
  return TYPE_OPTIONS.find((o) => o.key === t)?.emoji || "📦";
}
function typeLabel(t: string | undefined): string {
  if (!t) return "应用";
  return TYPE_LABEL[t] || t;
}

// P0-#Y#2：app 时的细分（6 大类）— **自动分配**，不展示"未细分"
// P0-#Y#FIX#UNCAT#REMOVE：删除"📦 未细分"二级选项
//  原因：用户要求"自动分配有问题，人工二次精确修改"——所以二级只能是具体类型
//  自动分配失败的项：保留 subtype="" 但不在侧边栏二级里展示（用"全部"看）
const APP_SUBTYPE_OPTIONS: { key: string; label: string; emoji: string }[] = [
  { key: "game", label: "游戏", emoji: "🎮" },
  { key: "office", label: "办公", emoji: "📊" },
  { key: "dev", label: "开发", emoji: "💻" },
  { key: "utility", label: "工具", emoji: "🛠️" },
  { key: "media", label: "媒体", emoji: "🎬" },
  { key: "design", label: "设计", emoji: "🎨" },
];
// P0-#Y#2：document 时的细分（6 类）— **自动分配**，不展示"未细分"
const DOC_SUBTYPE_OPTIONS: { key: string; label: string; emoji: string }[] = [
  { key: "word", label: "Word", emoji: "📘" },
  { key: "excel", label: "Excel", emoji: "📗" },
  { key: "ppt", label: "PPT", emoji: "📙" },
  { key: "pdf", label: "PDF", emoji: "📕" },
  { key: "text", label: "文本", emoji: "📝" },
  { key: "image", label: "图片", emoji: "🖼️" },
];
// P0-#Y#2：细分中文 label（用于卡片下方副标）
const SUBTYPE_LABEL: Record<string, string> = {
  game: "游戏", office: "办公", dev: "开发", utility: "工具",
  media: "媒体", design: "设计", other: "其他",
  word: "Word", excel: "Excel", ppt: "PowerPoint", pdf: "PDF",
  text: "文本", image: "图片",
  "url-web": "网页", "url-steam": "Steam", "url-epic": "Epic", "url-other": "其他",
};
function subtypeLabel(s: string | undefined | null): string {
  if (!s) return "";
  return SUBTYPE_LABEL[s] || s;
}

/// P0-#Y#2：根据当前 activeAppType 算出应该展示的二级 chip 列表
//  P0-#Y#FIX#URL#NOSUB：网址（url）和文件夹（folder）不展示二级细分
//  之前：url 也有细分（普通网页/Steam/Epic/其他）—— 但用户要求"网址对应链接区，不要二次细分"
//  现在：只有 app / document 展示二级；url / folder / 全部 都不展示
const currentSubtypeOptions = computed(() => {
  const t = softwareStore.activeAppType;
  if (t === "app") return APP_SUBTYPE_OPTIONS;
  if (t === "document") return DOC_SUBTYPE_OPTIONS;
  return []; // url / folder / null 都不展示二级
});

// 关键修复 P1-#X：空态显示条件
// 之前：v-else-if="filteredItems.length === 0" → 即使首次加载中也会显示纸箱子
// 现在：只在"已加载完 + 确实没数据"才显示纸箱子；首次加载中显示骨架
const showEmptyState = computed(
  () => softwareStore.hasLoaded && !softwareStore.loading && filteredItems.value.length === 0
);
const showSkeleton = computed(
  () => !softwareStore.hasLoaded || (softwareStore.loading && softwareStore.items.length === 0)
);

// 打开扫描
async function openScan() {
  showScanModal.value = true;
  scanning.value = true;
  scannedApps.value = [];
  selectedPaths.value = new Set();
  scanSearch.value = "";
  scanTargetCategoryId.value = undefined; // 重置目标分类
  scanTargetDropdownOpen.value = false; // 关闭下拉
  try {
    const result = await softwareStore.scan();
    scannedApps.value = result;
  } catch (e) {
    console.error("scan failed", e);
    appStore.showClipToast("info", "扫描失败：" + String(e));
  } finally {
    scanning.value = false;
  }
}

function closeScan() {
  showScanModal.value = false;
  scanSearch.value = "";
  scanTargetDropdownOpen.value = false;
}

function toggleScanTargetDropdown() {
  scanTargetDropdownOpen.value = !scanTargetDropdownOpen.value;
}

function pickScanTarget(id: number | undefined) {
  scanTargetCategoryId.value = id;
  scanTargetDropdownOpen.value = false;
}

// 计算当前显示的目标分类 label
const scanTargetLabel = computed(() => {
  if (scanTargetCategoryId.value == null) {
    return { icon: "✨", name: "不分类" };
  }
  const c = softwareStore.categories.find(
    (c) => c.id === scanTargetCategoryId.value
  );
  return c ? { icon: c.icon, name: c.name } : { icon: "✨", name: "不分类" };
});

function toggleSelect(path: string) {
  if (selectedPaths.value.has(path)) {
    selectedPaths.value.delete(path);
  } else {
    selectedPaths.value.add(path);
  }
  selectedPaths.value = new Set(selectedPaths.value); // 触发响应式
}

function toggleSelectAll() {
  if (selectedPaths.value.size === filteredScannedApps.value.length) {
    selectedPaths.value = new Set();
  } else {
    selectedPaths.value = new Set(filteredScannedApps.value.map((a) => a.path));
  }
}

async function confirmScan() {
  // 用户反馈：用户只关心"启动项"，默认过滤掉文件夹/文档/其他
  // 但 checkbox 取消的项也不导入——尊重用户选择
  const toAdd = scannedApps.value.filter(
    (a) => selectedPaths.value.has(a.path) && a.is_launchable
  );
  if (toAdd.length === 0) {
    appStore.showClipToast("info", "请至少勾选一个可启动的应用（.exe/.lnk/.url）");
    return;
  }
  let success = 0;
  let failed: string[] = [];
  let skipped: number = 0;
  for (const a of toAdd) {
    try {
      await softwareStore.create({
        name: a.name,
        path: a.path,
        iconPath: a.icon_path,
        args: a.args,
        // P0-#Y#FIX#SCAN#IMPORT：传递后端返回的 app_type/subtype，避免 create_app 重新推断时误判
        appType: a.app_type,
        appSubtype: a.app_subtype,
        categoryId: scanTargetCategoryId.value,
      });
      success++;
    } catch (e) {
      failed.push(`${a.name}: ${e}`);
    }
  }
  // 跳过未启动的（用户勾了但 is_launchable=false）
  const skippedCount = scannedApps.value.filter(
    (a) => selectedPaths.value.has(a.path) && !a.is_launchable
  ).length;
  if (skippedCount > 0) {
    appStore.showClipToast(
      "info",
      `已导入 ${success} 个应用，跳过 ${skippedCount} 个非启动项（文件夹/文档/其他）`
    );
  }
  showScanModal.value = false;
  scanTargetCategoryId.value = undefined; // 重置
  scanTargetDropdownOpen.value = false; // 关闭下拉
  // 关键：清空搜索 + 切回"全部"分类（如果目标分类被删则切回全部）
  appStore.searchQuery = "";
  if (scanTargetCategoryId.value == null) {
    softwareStore.setCategory(null);
  } else {
    softwareStore.setCategory(scanTargetCategoryId.value);
  }
  if (failed.length > 0) {
    appStore.showClipToast("info", `导入 ${success} 个，失败 ${failed.length} 个`);
    console.warn("scan import failures:", failed);
  } else {
    appStore.showClipToast("success", `已导入 ${success} 个软件`);
  }
}

// 手动添加
function openAddForm() {
  addForm.value = { name: "", path: "", args: "", categoryId: undefined };
  showAddForm.value = true;
}

// P0-#Y#FIX#URL#UI：路径快速模板（解决"链接不知道怎么上传"的问题）
// 设计：3 大类就够（应用 / 网址 / 文件夹）
//   网址里展开 3 个常见协议子模板：普通 HTTPS / Steam 游戏 / Epic 游戏
// 选完 chip → 自动填到 addForm.value.path 作为示例（用户可继续编辑）
interface PathTemplate {
  key: string;
  emoji: string;
  label: string;
  example: string;        // 点击后填入 path 输入框的示例
  hint: string;           // 输入框 placeholder 提示
  presetArgs?: string;    // 一些模板预填 args
}
// 一级模板（用户能直接点的 3 个）
const PATH_TEMPLATES: PathTemplate[] = [
  {
    key: "exe",
    emoji: "🚀",
    label: "应用",
    example: "C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe",
    hint: ".exe / .lnk 路径（拖入文件 / 粘贴路径）",
  },
  {
    key: "url",
    emoji: "🌐",
    label: "网址",
    example: "https://v.qq.com",
    hint: "网址 / Steam URI / Epic URI / Microsoft Store 链接 等",
  },
  {
    key: "folder",
    emoji: "📁",
    label: "文件夹",
    example: "C:\\Users\\Public\\Documents",
    hint: "文件夹路径（点开后用资源管理器浏览）",
  },
];
// P0-#Y#FIX#REMOVE#URL#SUB：删除 URL_SUBTEMPLATES（"协议："子模板区段）
// 原因：用户要求"上传时不让选子协议（普通网页/Steam/Epic）"
// 后端按 path 前缀自动判：https:// → url，steam:// → url，等等
const activeTemplate = ref<string>("exe");
// P0-#Y#FIX#PICK#NOECHO：pickTemplate 只在 path 为空时覆盖，**避免"回弹"**
// 之前：每次点 chip 都会强制覆盖 addForm.value.path → 用户自己输了一半后
//   点子 chip（想换协议）→ 整段被清空 + 替换成 example → 用户感觉"被弹回"
// 现在：path 为空时 → 自动填 example（让用户看到示例）
//      path 已有内容时 → **完全不覆盖**（用户输入的优先于示例）
//      用户想强制用示例 → 清空 path 后再点 chip
function pickTemplate(t: PathTemplate) {
  activeTemplate.value = t.key;
  // 关键：仅在 path 为空时填示例，避免覆盖用户已填的内容
  if (!addForm.value.path || !addForm.value.path.trim()) {
    addForm.value.path = t.example;
  }
  if (t.presetArgs !== undefined && !addForm.value.args) {
    addForm.value.args = t.presetArgs;
  }
}
// P0-#Y#FIX#PICK#UI：浏览按钮 — 调原生 Windows 文件/文件夹选择对话框
async function browsePath() {
  try {
    const { pickPath } = await import("../api");
    const mode = activeTemplate.value === "folder" ? "folder" : "file";
    const picked = await pickPath({ mode });
    if (picked) {
      addForm.value.path = picked;
      // 如果用户没填名称，用文件名（去掉扩展名）作为默认名
      if (!addForm.value.name) {
        const fileName = picked
          .replace(/\\/g, "/")
          .split("/")
          .pop() || "未命名";
        addForm.value.name = fileName.replace(/\.[^.]+$/, "");
      }
    }
  } catch (e) {
    appStore.showClipToast("info", "浏览对话框打开失败：" + String(e));
  }
}

// P0-#Y#FIX#CAT#AUTO：根据 path 推断 app_type + app_subtype
// 返回 [appType, appSubtype]
//   appType:    "app" / "folder" / "url" / "document"
//   appSubtype: app → "game" / "office" / "dev" / "utility" / "media" / "design" / ""
//                document → "word" / "excel" / "ppt" / "pdf" / "text" / "image" / ""
//                url / folder → ""（无细分）
// P0-#Y#FIX#URL#NOSUB：url 不再细分（之前误加了 url-web/url-steam/url-epic/url-other，
//   用户要求"网址对应链接区，不要二次细分"，已删）
//
// P0-#Y#FIX#URL#MISJUDGE：修复 3 种网址被误判为 folder 的情况
//   1. 带查询参数：https://www.bilibili.com/video/BVxxx?p=1 → lastSegment "BVxxx?p=1" 无扩展名 → 误判 folder
//   2. 无扩展名路径：https://github.com/user/repo → lastSegment "repo" 无扩展名 → 误判 folder
//   3. 无协议前缀域名：www.baidu.com / github.com / bbs.nga.cn/read.php?tid=xxx → 未识别为 URL

// P0-#Y#FIX#GLOBAL#TLD：🔴 把 TLDS + looksLikeDomain 提升到全局作用域（从 inferAppAndSubtype 内部提出来）
//   致命闭包问题：之前 TLDS 和 looksLikeDomain 是 inferAppAndSubtype 的内部闭包变量
//   → 模板（诊断面板）/ submitAdd 外部调 looksLikeDomain → 直接报错 "is not defined"
//   → 诊断面板不显示 + submitAdd 里 looksLikeDomain 强制 URL 不生效！
const TLDS = [
  // 中国区域后缀（长的放前面，避免被短的抢先匹配）
  ".com.cn", ".net.cn", ".org.cn", ".gov.cn", ".edu.cn", ".ac.cn",
  // 通用顶级域名 gTLD（最常用）
  ".com", ".cn", ".net", ".org", ".io", ".dev", ".cc", ".co", ".ai", ".app",
  ".top", ".xyz", ".club", ".shop", ".site", ".vip", ".tech", ".store",
  ".me", ".tv", ".fm", ".info", ".biz", ".us", ".jp", ".kr", ".ru", ".uk",
  ".de", ".fr", ".edu", ".gov", ".mil",
  // 高频率新型 TLD
  ".fun", ".online", ".live", ".news", ".blog", ".wiki", ".video", ".cloud",
  ".work", ".link", ".win", ".space", ".website", ".press", ".today", ".run",
  ".life", ".group", ".design", ".art", ".photo", ".pics", ".pictures",
  ".show", ".watch", ".games", ".game", ".play", ".plus", ".pro", ".name",
  ".mobi", ".icu", ".xin", ".ren", ".wang", ".so", ".lu",
];
// 全局调用：模板/诊断面板/submitAdd 都能调用（判断是否为裸域名/URL 形式）
function looksLikeDomain(s: string): boolean {
  const sLower = (s || "").toLowerCase().trim();
  if (!sLower) return false;
  if (/^[a-z]:[\\/]/.test(sLower)) return false; // Windows 盘符排除
  for (const tld of TLDS) {
    const idx = sLower.indexOf(tld);
    if (idx < 0) continue;
    const after = sLower[idx + tld.length];
    // TLD 后面必须是结尾 或 / ? : #（端口/查询/锚点）
    if (after === undefined || after === "/" || after === "?" || after === ":" || after === "#") {
      const before = idx > 0 ? sLower[idx - 1] : "";
      if (before !== "\\") return true; // TLD 前不能是反斜杠（Windows 路径）
    }
  }
  return false;
}

function inferAppAndSubtype(path: string): { appType: string; appSubtype: string } {
  const p = path.trim();
  if (!p) return { appType: "app", appSubtype: "" };
  const pLower = p.toLowerCase();

  // ===== 第一步：URL 协议（完整协议前缀，最高优先级） =====
  const URL_PROTOCOLS = [
    "http://", "https://", "steam://", "com.epicgames.launcher://",
    "ms-settings:", "ms-store:", "mailto:", "discord:", "spotify:",
    "obsidian://", "typora://", "vscode://", "jetbrains://",
  ];
  if (URL_PROTOCOLS.some((proto) => pLower.startsWith(proto))) {
    return { appType: "url", appSubtype: "" };
  }
  if (pLower.includes("store.steampowered.com") || pLower.includes("epicgames.com")) {
    return { appType: "url", appSubtype: "" };
  }
  if (pLower.endsWith(".url")) {
    return { appType: "url", appSubtype: "" };
  }

  // ===== 第二步：无协议前缀但明显是域名 → 直接调用全局 looksLikeDomain（不再重复定义！） =====
  if (looksLikeDomain(pLower)) {
    return { appType: "url", appSubtype: "" };
  }

  // ===== 第三步：文件夹判断 =====
  let pathForExt = pLower;
  const qIdx = pathForExt.indexOf("?");
  if (qIdx >= 0) pathForExt = pathForExt.slice(0, qIdx);
  const hIdx = pathForExt.indexOf("#");
  if (hIdx >= 0) pathForExt = pathForExt.slice(0, hIdx);

  const lastSegment = pathForExt.split(/[\\/]/).pop() || "";
  const hasExt = /\.[a-z0-9]{1,5}$/.test(lastSegment);
  if (!hasExt) return { appType: "folder", appSubtype: "" };

  // ===== 第四步：文档判断 =====
  const docExts = ["doc", "docx", "pdf", "xls", "xlsx", "ppt", "pptx", "txt", "md", "rtf", "odt", "png", "jpg", "jpeg", "gif", "bmp", "svg"];
  const ext = lastSegment.split(".").pop() || "";
  if (docExts.includes(ext)) return { appType: "document", appSubtype: "" };
  return { appType: "app", appSubtype: "" };
}

// P0-#Y#FIX#CAT#2：保留旧函数（向后兼容），内部调新函数
function inferAppTypeFromPath(path: string): "app" | "folder" | "url" | "document" {
  const r = inferAppAndSubtype(path);
  return r.appType as any;
}

// P0-#Y#FINAL#DIAG：🟢 最终写入 DB 的 app_type（全局唯一真值来源！）
//   逻辑：模板显示 / submitAdd 写入 / 保存按钮文案 / footer 诊断芯片 全部用这个值
//   规则 1：用户选 🌐 网址 chip (activeTemplate === 'url') → 100% url（无视一切推断！）
//   规则 2：用户选 📁 文件夹 chip (activeTemplate === 'folder') → 100% folder
//   规则 3：用户选 🚀 应用 chip (activeTemplate === 'exe')
//           → 3A: looksLikeDomain 命中 TLD → 强制 url（即使在应用模板下也判为网址）
//           → 3B: 否则 → 用 inferAppAndSubtype 自动推断
const finalDiagnosedAppType = computed<string>(() => {
  if (activeTemplate.value === "url") return "url";
  if (activeTemplate.value === "folder") return "folder";
  // activeTemplate === 'exe' 或其他
  if (looksLikeDomain(addForm.value.path)) return "url";
  return inferAppAndSubtype(addForm.value.path).appType;
});

async function submitAdd() {
  if (!addForm.value.name || !addForm.value.path) {
    appStore.showClipToast("info", "请填写名称和路径");
    return;
  }
  const inferred = inferAppAndSubtype(addForm.value.path);
  // P0-#Y#FINAL#DIAG：直接用 finalDiagnosedAppType.value（唯一真值，显示/写入 100% 对齐）
  const appType = finalDiagnosedAppType.value;
  // url/folder 清空 subtype；app/document 保留推断结果
  let appSubtype = (appType === "app" || appType === "document")
    ? inferred.appSubtype
    : "";
  console.log(
    `[submitAdd] 🟢 最终写入 DB：appType=${JSON.stringify(appType)}, ` +
    `activeTemplate=${JSON.stringify(activeTemplate.value)}, ` +
    `looksLikeDomain=${looksLikeDomain(addForm.value.path)}, ` +
    `inferred.appType=${JSON.stringify(inferred.appType)}, ` +
    `path=${addForm.value.path}`
  );

  try {
    await softwareStore.create({
      name: addForm.value.name,
      path: addForm.value.path,
      args: addForm.value.args,
      categoryId: addForm.value.categoryId,
      appType: appType,
      appSubtype: appSubtype,
    });
    showAddForm.value = false;
    appStore.showClipToast("success", "已添加");
    // P0-#Y#FIX#CAT#AUTO：保存成功后按 activeTemplate/appType 自动切侧边栏
    softwareStore.setAppType(appType as any);
    // P0-#Y#FIX#URL#NOSUB：url/folder 不切细分（这两个无二级）
    // app/document 切到对应二级细分（让用户立刻看到）
    if ((inferred.appType === "app" || inferred.appType === "document") && inferred.appSubtype) {
      softwareStore.setSubtype(inferred.appSubtype);
    }
  } catch (e) {
    appStore.showClipToast("info", String(e));
  }
}

// 启动
async function launchApp(id: number) {
  // P0-#F：根据软件类型 + 启动结果给精确 toast
  const item = softwareStore.items.find((it) => it.id === id);
  const result = await softwareStore.launch(id);
  if (result.ok) {
    if (!item) return;
    const p = (item.path || "").toLowerCase();
    // P0-#Y#URL#TOAST：根据 app_type / path 形式给精确成功提示
    if (item.app_type === "url" || p.startsWith("http://") || p.startsWith("https://") || p.startsWith("steam://") || p.startsWith("com.epicgames.launcher://")) {
      appStore.showClipToast("success", `已在浏览器中打开「${item.name}」`);
    } else if (p.endsWith(".url")) {
      appStore.showClipToast("success", `已在浏览器中打开「${item.name}」`);
    } else if (p.endsWith(".lnk")) {
      appStore.showClipToast("success", `已启动「${item.name}」`);
    } else if (item.app_type === "folder") {
      appStore.showClipToast("success", `已打开文件夹「${item.name}」`);
    } else {
      appStore.showClipToast("success", `已启动「${item.name}」`);
    }
  } else {
    if (result.error === "missing") {
      // 路径不存在 — 提示用户重新拖入
      // P0-#Y#URL#TOAST：URL 类型不应该说"重新拖入 .exe 或 .lnk"
      if (item && (item.app_type === "url" ||
          (item.path || "").toLowerCase().startsWith("http://") ||
          (item.path || "").toLowerCase().startsWith("https://") ||
          (item.path || "").toLowerCase().startsWith("steam://"))) {
        appStore.showClipToast("info", `网页打开失败：${item.path}`);
      } else if (item && item.path.toLowerCase().endsWith(".url")) {
        appStore.showClipToast("info", `「${item?.name}」网页快捷方式已失效。已尝试启动 Steam，请在库中找到该游戏`);
      } else {
        appStore.showClipToast("info", `找不到「${item?.name ?? ""}」的安装位置。请重新拖入 .exe 或 .lnk`);
      }
    } else {
      appStore.showClipToast("info", result.message || "启动失败");
    }
  }
}

// 编辑
function startEdit(item: any) {
  editingId.value = item.id;
  editForm.value = {
    name: item.name,
    path: item.path,
    args: item.args || "",
    categoryId: item.category_id,
    appSubtype: item.app_subtype || "",
  };
}
async function submitEdit() {
  if (editingId.value == null) return;
  try {
    await softwareStore.update({
      id: editingId.value,
      name: editForm.value.name,
      path: editForm.value.path,
      args: editForm.value.args,
      categoryId: editForm.value.categoryId,
    });
    editingId.value = null;
    appStore.showClipToast("success", "已保存");
  } catch (e) {
    appStore.showClipToast("info", String(e));
  }
}

// 删除
function askDeleteApp(id: number, name: string) {
  confirmingDeleteId.value = id;
  confirmingDeleteName.value = name;
}

function cancelDeleteApp() {
  confirmingDeleteId.value = null;
  confirmingDeleteName.value = "";
}

async function confirmDeleteApp(id: number | null) {
  if (id == null) return;
  deletingId.value = id;
  try {
    await softwareStore.remove(id);
    appStore.showClipToast("success", "已删除");
  } catch (e) {
    appStore.showClipToast("info", "删除失败：" + String(e));
  } finally {
    deletingId.value = null;
    confirmingDeleteId.value = null;
    confirmingDeleteName.value = "";
  }
}
</script>

<template>
  <div class="view">
    <div class="view-header">
      <div class="view-header-left">
        <div class="view-icon icon-blue">🚀</div>
        <div class="view-header-info">
          <h2 class="view-title">
            <span class="view-title-text">软件</span>
            <span class="title-count">{{ filteredItems.length }}</span>
          </h2>
          <p class="view-subtitle">
            <template v-if="appStore.searchQuery">
              搜索 "{{ appStore.searchQuery }}" 的结果
            </template>
            <template v-else>点图标启动 · 拖入或扫描导入</template>
          </p>
        </div>
      </div>
      <div class="view-header-actions">
        <!-- P0-#ICON#MISSING#HINT：缺失图标兜底按钮（手动触发重抽）
             之前：图标丢失时用户只能去设置里点"清空图标缓存 + 重新抽图"——多一步路径
             现在：缺失数 > 0 直接在 header 显示"🎨 重抽图标"按钮（1 步恢复） -->
        <button
          v-if="missingIconCount > 0"
          class="action-btn action-btn-warn tap"
          :disabled="reextractingIcons"
          :title="`${missingIconCount} 个软件图标缺失，点击重抽`"
          @click="reextractIcons"
        >
          <span v-if="!reextractingIcons">🎨 重抽 ({{ missingIconCount }})</span>
          <span v-else>抽图中…</span>
        </button>
        <button class="action-btn action-btn-text tap" @click="openScan" title="扫描系统">
          🔍 <span>扫描</span>
        </button>
        <button class="action-btn action-btn-primary tap" @click="openAddForm" title="手动添加">+</button>
      </div>
    </div>

    <!-- P0-#Y：类型筛选 chips 改为左侧可收缩 sidebar（默认仅图标列、hover 展开） -->
    <!-- 主内容容器：左 sidebar + 右主区 -->
    <div class="apps-body">
      <!-- 左侧可收缩 sidebar -->
      <div class="filter-sidebar" :class="{ expanded: sidebarExpanded }"
           @mouseenter="sidebarExpanded = true"
           @mouseleave="sidebarExpanded = false">
        <div class="sidebar-section-label" v-if="sidebarExpanded">类型</div>
        <div class="type-chips sidebar-chips">
          <button
            v-for="t in TYPE_OPTIONS"
            :key="t.key ?? 'all'"
            class="type-chip tap"
            :class="{ active: softwareStore.activeAppType === t.key }"
            :data-type="t.key ?? 'all'"
            :title="t.label"
            @click="softwareStore.setAppType(t.key)"
          >
            <span class="type-chip-emoji">{{ t.emoji }}</span>
            <span class="type-chip-label">{{ t.label }}</span>
          </button>
        </div>
        <!-- P0-#Y#2：二级 chips（按 activeAppType 联动，folder/url 全部时无二级） -->
        <Transition name="subtype-fade">
          <div v-if="currentSubtypeOptions.length" class="sidebar-subtype">
            <div class="sidebar-divider"></div>
            <div class="sidebar-section-label" v-if="sidebarExpanded">细分</div>
            <div class="type-chips sidebar-chips">
              <button
                v-for="s in currentSubtypeOptions"
                :key="s.key"
                class="type-chip tap"
                :class="{ active: softwareStore.activeSubtype === s.key }"
                :data-sub="s.key"
                :title="s.label"
                @click="softwareStore.setSubtype(s.key)"
              >
                <span class="type-chip-emoji">{{ s.emoji }}</span>
                <span class="type-chip-label">{{ s.label }}</span>
              </button>
            </div>
          </div>
        </Transition>
      </div>

      <!-- 右侧主内容 -->
      <div class="apps-main">
        <!-- P0-#Z：行内 confirm 浮条（软件区，浮在网格上方，全宽不挡卡片） -->
        <Transition name="confirm-bar">
          <div v-if="confirmingDeleteId !== null" class="app-confirm-bar">
            <span class="confirm-bar-icon">⚠️</span>
            <span class="confirm-bar-text">确定删除「{{ confirmingDeleteName }}」？</span>
            <div class="confirm-bar-actions">
              <button
                class="confirm-btn cancel tap"
                :disabled="deletingId !== null"
                @click="cancelDeleteApp"
              >取消</button>
              <button
                class="confirm-btn danger tap"
                :disabled="deletingId !== null"
                @click="confirmDeleteApp(confirmingDeleteId)"
              >{{ deletingId !== null ? "删除中..." : "删除" }}</button>
            </div>
          </div>
        </Transition>

        <!-- 加载中：骨架网格（不闪） -->
        <div v-if="showSkeleton" class="app-grid">
          <div v-for="i in 6" :key="i" class="app-cell skeleton-cell">
            <div class="skeleton-icon"></div>
            <div class="skeleton-line" style="width: 70%"></div>
          </div>
        </div>

        <!-- 空状态：仅当已加载完 + 确实无数据（用 showEmptyState 避免纸箱子闪屏） -->
        <div v-else-if="showEmptyState" class="empty">
          <div class="empty-icon">📦</div>
          <div class="empty-text">
            <template v-if="appStore.searchQuery">
              没有匹配 "{{ appStore.searchQuery }}" 的软件
            </template>
            <template v-else>还没有软件，点 + 添加 或 🔍 扫描</template>
          </div>
        </div>

        <!-- 网格：3 列软件卡片（真实图标 + 名称 + 启动） -->
        <div v-else class="app-grid">
          <div
            v-for="(item, index) in filteredItems"
            :key="item.id"
            class="app-cell stagger-item"
            :class="{
              'app-cell-editing': editingId === item.id,
              'app-cell-confirming': confirmingDeleteId === item.id
            }"
            :data-type="item.app_type || 'app'"
            :style="{ '--delay': `${index * 0.03}s` }"
            @click="launchApp(item.id)"
            @contextmenu="openContextMenu($event, item)"
          >
            <template v-if="editingId === item.id">
              <div class="edit-form" @click.stop>
                <input v-model="editForm.name" class="form-input" placeholder="名称" />
                <input v-model="editForm.path" class="form-input" placeholder="路径" />
                <input v-model="editForm.args" class="form-input" placeholder="参数（可选）" />
                <div class="edit-actions">
                  <button class="btn-primary tap" @click.stop="submitEdit">✓ 保存</button>
                  <button class="btn-secondary tap" @click.stop="editingId = null">✕ 取消</button>
                </div>
              </div>
            </template>
            <template v-else>
              <!-- P0-#Y：类型色条（顶部 2.5px 彩色横条，颜色映射 app_type） -->
              <div class="type-stripe" :class="`stripe-${item.app_type || 'app'}`"></div>
              <!-- P0-#ICON#DUAL#FALLBACK：三级回退渲染（恢复之前能用的 asset:// 兜底）
                   优先级：
                   1. softwareStore.iconDataUrls[id] ← 后端 base64 data URL（最稳）
                   2. convertFileSrc(icon_path) ← Tauri asset:// 协议（之前能用，恢复它！）
                   3. folder/url/document → 内置 SVG
                   4. app → emoji 🚀（用 emoji 字体，避开之前的 CSS bug）

                   之前我的"修复"把 #2 删了 → asset:// 失败时直接走 #4
                     → CSS bug 让 🚀 透明化成"紫色羽毛" → 用户看到"图标全没了"
                   修复：恢复 #2 兜底 + 用 @error 自动降级到 #4（不是 fixed 隐藏） -->
              <div class="app-icon">
                <img
                  v-if="softwareStore.iconDataUrls[item.id]"
                  :src="softwareStore.iconDataUrls[item.id]"
                  :alt="item.name"
                  class="app-icon-img"
                  @error="onAppIconError(item.id)"
                />
                <img
                  v-else-if="item.icon_path && !iconError[item.id]"
                  :src="iconSrc(item.icon_path)"
                  :alt="item.name"
                  class="app-icon-img"
                  @error="onAppIconError(item.id)"
                />
                <!-- P0-#Y#FIX#ICON#FOLDER：folder/url 没真实图标时显示内置 SVG（Windows 风格） -->
                <FolderIconSvg v-else-if="(item.app_type || 'app') === 'folder'" class="app-icon-img" />
                <UrlIconSvg v-else-if="(item.app_type || 'app') === 'url'" class="app-icon-img" />
                <DocumentIconSvg v-else-if="(item.app_type || 'app') === 'document'" class="app-icon-img" />
                <!-- P0-#ICON#CSS#BUG#FIX：typeEmoji 永远返回 emoji（不是首字母）
                     之前误用 typeEmoji(...).length > 1 判 emoji
                     → JS 里 emoji 字符 length=1 → 永远走 is-text 渐变透明化
                     → 全部 emoji 变"渐变描边"（用户称之为"紫色羽毛"）
                     现在：直接固定 is-emoji（typeEmoji 函数定义保证返回 emoji 字符） -->
                <span
                  v-else
                  class="app-icon-text is-emoji"
                >{{ typeEmoji(item.app_type) }}</span>
              </div>
              <div class="app-name" :title="item.name">{{ item.name }}</div>
              <!-- P0-#DIAG#APPTYPE：🔴 诊断标签 - 直接显示 SQLite 中 app_type 字段的原始值（肉眼排查数据错误） -->
              <div
                class="app_type_diag_chip"
                :class="'dtc-' + (item.app_type || 'EMPTY')"
                :title="'DB.app_type = ' + JSON.stringify(item.app_type) + '; DB.app_subtype = ' + JSON.stringify(item.app_subtype)"
              >
                DB:{{ item.app_type || "空!" }}
              </div>
              <!-- P0-#Y#FIX#LABEL#FALLBACK：副标显示逻辑
                   1. 有 subtype（细分）→ 显示细分 label（游戏 / Word / PDF / 工具...）
                   2. 没 subtype（自动判失败或 url/folder）→ 显示一级 label（应用 / 网址 / 文件夹 / 文档）
                   之前：url/folder 卡片下面完全是空（因为没 subtype）
                   现在：url 卡片显示"网址"、folder 显示"文件夹"、document 显示"文档"或细分类（Word/Excel/...） -->
              <div class="app-subtype">
                {{ subtypeLabel(item.app_subtype) || typeLabel(item.app_type) }}
              </div>
              <div v-if="item.use_count > 0" class="app-badge">🔥 {{ item.use_count }}</div>
              <div class="app-actions" @click.stop>
                <button class="app-action-btn" title="编辑" @click.stop="startEdit(item)">✏️</button>
                <button class="app-action-btn" title="删除" @click.stop="askDeleteApp(item.id, item.name)">🗑️</button>
              </div>
            </template>
          </div>
        </div>
      </div>
    </div>

    <!-- P0-#Y：系统级拖入提示遮罩（Tauri 2 webview drag-drop 事件驱动） -->
    <Transition name="drop-fade">
      <div v-if="isDraggingOver" class="drop-overlay">
        <div class="drop-overlay-inner">
          <div class="drop-overlay-icon">📥</div>
          <div class="drop-overlay-text">拖入文件 / 文件夹 / 快捷方式</div>
          <div class="drop-overlay-hint">自动识别软件 / 文件夹 / 文档类型</div>
        </div>
      </div>
    </Transition>

    <!-- P0-#Y#4：右键菜单 - 修改单条 app 的细分分类 -->
    <Teleport to="body">
      <Transition name="ctx-fade">
        <div
          v-if="ctxMenu.visible"
          ref="ctxMenuEl"
          class="ctx-menu"
          :style="{ left: ctxMenu.x + 'px', top: ctxMenu.y + 'px' }"
          @click.stop
          @contextmenu.stop
        >
          <div class="ctx-menu-title">
            <span class="ctx-menu-name">{{ ctxMenu.item?.name }}</span>
            <span class="ctx-menu-type">{{ typeLabel(ctxMenu.item?.app_type) }}</span>
          </div>
          <div class="ctx-menu-divider"></div>
          <template v-if="ctxSubtypeOptionsNoUncat.length">
            <div class="ctx-menu-section">修改细分分类</div>
            <button
              v-for="s in ctxSubtypeOptionsNoUncat"
              :key="s.key"
              class="ctx-menu-item tap"
              :class="{ active: ctxMenu.item?.app_subtype === s.key }"
              @click="pickSubtype(s.key)"
            >
              <span class="ctx-menu-emoji">{{ s.emoji }}</span>
              <span>{{ s.label }}</span>
              <span v-if="ctxMenu.item?.app_subtype === s.key" class="ctx-menu-check">✓</span>
            </button>
          </template>
          <div v-else class="ctx-menu-empty">
            该类型无细分分类（仅 app / document 可细分）
          </div>
          <div class="ctx-menu-divider"></div>
          <!-- P0-#Y#FIX#TYPE#EDIT：修改一级分类
               让用户能把"误判为 app"的网址改成"url"，或把 app 改成 folder
               之前只能改 subtype（细分），不能改 type（一级） -->
          <div class="ctx-menu-section">修改一级分类</div>
          <button
            v-for="t in TYPE_OPTIONS.filter((o): o is { key: string; label: string; emoji: string } => o.key != null)"
            :key="t.key"
            class="ctx-menu-item tap"
            :class="{ active: ctxMenu.item?.app_type === t.key }"
            @click="pickType(t.key)"
          >
            <span class="ctx-menu-emoji">{{ t.emoji }}</span>
            <span>{{ t.label }}</span>
            <span v-if="ctxMenu.item?.app_type === t.key" class="ctx-menu-check">✓</span>
          </button>
          <div class="ctx-menu-divider"></div>
          <button class="ctx-menu-item tap" @click="(closeContextMenu(), startEdit(ctxMenu.item))">
            <span class="ctx-menu-emoji">✏️</span>
            <span>编辑名称/路径</span>
          </button>
        </div>
      </Transition>
    </Teleport>

    <!-- 手动添加表单 -->
    <Transition name="modal">
      <div v-if="showAddForm" class="modal-mask" @click.self="showAddForm = false">
        <div class="modal">
          <div class="modal-header">
            <span>添加软件</span>
            <button class="modal-close tap" @click="showAddForm = false">×</button>
          </div>
          <div class="modal-body">
            <!-- P0-#Y#FIX#URL#UI：路径类型快速模板
                 解决"链接 / Steam 游戏 / Epic 游戏不知道如何上传"的问题
                 设计：3 大类（一级 chip）+ 网址下展开 3 个子协议 chip
                 点 chip → 自动填到 path 输入框作为示例，用户可继续编辑
                 后端 detect_subtype_from_path 会根据 path 字符串自动归类 -->
            <div class="form-row">
              <label>类型（点击填入示例）</label>
              <div class="path-templates">
                <button
                  v-for="t in PATH_TEMPLATES"
                  :key="t.key"
                  type="button"
                  class="path-template-chip tap"
                  :class="{ active: activeTemplate === t.key }"
                  :title="t.hint"
                  @click="pickTemplate(t)"
                >
                  <span class="path-template-emoji">{{ t.emoji }}</span>
                  <span class="path-template-label">{{ t.label }}</span>
                </button>
              </div>
              <!-- 🔴 P0-#UI#DIAG：诊断面板直接放在 chip 红框正下方（用户一打开弹窗就能看到，不会被裁掉！） -->
              <div class="diag-panel">
                <div class="diag-title">🔴 实时诊断面板（写入前请核对！）</div>
                <div class="diag-row">
                  <span class="diag-k">① 当前选中的 chip → key:</span>
                  <span class="diag-v" :class="{ 'dtc-url': activeTemplate === 'url', 'dtc-folder': activeTemplate === 'folder', 'dtc-app': activeTemplate === 'exe' }">
                    activeTemplate = <b>{{ JSON.stringify(activeTemplate) }}</b>
                    （{{ activeTemplate === 'exe' ? '🚀 应用模板' : activeTemplate === 'url' ? '🌐 网址模板 - 100% 写入 DB.url' : activeTemplate === 'folder' ? '📁 文件夹模板 - 100% 写入 DB.folder' : '未知模板！' }}）
                  </span>
                </div>
                <div class="diag-row">
                  <span class="diag-k">② inferAppAndSubtype 推断:</span>
                  <span class="diag-v" :class="'dtc-' + inferAppAndSubtype(addForm.path).appType">
                    appType={{ JSON.stringify(inferAppAndSubtype(addForm.path).appType) }},
                    subtype={{ JSON.stringify(inferAppAndSubtype(addForm.path).appSubtype) }}
                  </span>
                </div>
                <div class="diag-row">
                  <span class="diag-k">③ TLD 域名识别 (looksLikeDomain):</span>
                  <span class="diag-v" :class="{ 'dtc-url': looksLikeDomain(addForm.path), 'dtc-folder': !looksLikeDomain(addForm.path) }">
                    {{ looksLikeDomain(addForm.path) ? '✅ 命中 TLD（.com/.cn/.fun 等）是 URL 形式' : '❌ 未命中 TLD，不是明显 URL' }}
                  </span>
                </div>
                <div class="diag-row diag-final">
                  <span class="diag-k">④ 🔥 最终写入 DB.appType:</span>
                  <span
                    class="diag-v diag-final-v"
                    :class="'dtc-' + finalDiagnosedAppType"
                  >
                    <b>{{ JSON.stringify(finalDiagnosedAppType) }}</b>
                    <template v-if="activeTemplate === 'url'">
                      🌐（用户选了「🌐网址」模板 → 100% 强制 url，完全覆盖推断结果）
                    </template>
                    <template v-else-if="activeTemplate === 'folder'">
                      📁（用户选了「📁文件夹」模板 → 100% 强制 folder）
                    </template>
                    <template v-else-if="looksLikeDomain(addForm.path)">
                      🌐（TLD 命中 URL → 强制 url，覆盖 exe 模板默认）
                    </template>
                    <template v-else>
                      （🚀 应用模板自动推断结果）
                    </template>
                  </span>
                </div>
                <div class="diag-hint">
                  <b>核心规则：</b>用户选 🌐网址 / 📁文件夹 模板 → 100% 按模板写入（无视路径推断），🚀应用模板才走自动识别。
                  <br>网址（DB:app_type=url）会出现在侧边栏「🔗 网址」tab，文件夹（DB:app_type=folder）出现在「📁 文件夹」tab！
                </div>
              </div>
              <!-- P0-#Y#FIX#REMOVE#URL#SUB：删掉"协议："子模板区段
                   用户要求"如果虚类型选择了网址后，那么填写上传的就默认分类链接中，
                   人工可二次修改"——所以上传时不再让用户选子协议（普通网页/Steam/Epic）
                   后端按 path 前缀自动判：https://→url，steam://→url，等等 -->
            </div>
            <div class="form-row">
              <label>名称</label>
              <input v-model="addForm.name" class="form-input" placeholder="如：腾讯视频 / GitHub / 我的文档" />
            </div>
            <div class="form-row">
              <label>路径</label>
              <!-- P0-#Y#FIX#PICK#UI：路径输入框 + 浏览... 按钮
                   URL 类不放"浏览"按钮（没有"浏览 URL"的概念）
                   应用 / 文件夹类放"浏览"按钮，点开原生资源管理器选 -->
              <div class="path-input-row">
                <input
                  v-model="addForm.path"
                  class="form-input path-input"
                  :placeholder="PATH_TEMPLATES.find(t => t.key === activeTemplate)?.hint || '路径'"
                />
                <button
                  v-if="activeTemplate === 'exe' || activeTemplate === 'folder'"
                  type="button"
                  class="browse-btn tap"
                  :title="activeTemplate === 'folder' ? '浏览文件夹…' : '浏览应用…'"
                  @click="browsePath"
                >
                  <span class="browse-icon">📂</span>
                  <span>浏览…</span>
                </button>
              </div>
              <div class="form-hint">
                💡 <strong>网页</strong>直接填 https://... 即可（点 🌐 网址 → 🔗 普通网页）<br>
                💡 <strong>Steam / Epic 游戏</strong>点 🌐 网址 → 🎮 / 🌀 模板看示例<br>
                💡 <strong>桌面 .url / .lnk 文件</strong>直接拖入此框
              </div>
            </div>
            <div class="form-row">
              <label>参数（可选）</label>
              <input v-model="addForm.args" class="form-input" placeholder="启动参数（链接类一般留空）" />
            </div>
            <!-- P0-#Y#FIX#AUTO：删除分类下拉框，分类由后端根据 path / 文件名 / 扩展名自动判定
                 （app/document/folder/url + subtype：game/office/dev/utility/media/design 等） -->
            <div class="form-row auto-classify-info">
              <span class="auto-icon">✨</span>
              <span class="auto-text">分类自动识别 — 上传后如需调整，可右键单条修改</span>
            </div>
          </div>
          <div class="modal-footer">
            <div class="footer-diag">
              <span class="footer-diag-label">写入 DB:</span>
              <span
                class="footer-diag-chip"
                :class="'dtc-' + finalDiagnosedAppType"
              >
                app_type = <b>{{ JSON.stringify(finalDiagnosedAppType) }}</b>
                <template v-if="finalDiagnosedAppType === 'url'">🌐 网址分类</template>
                <template v-else-if="finalDiagnosedAppType === 'folder'">📁 文件夹分类</template>
                <template v-else-if="finalDiagnosedAppType === 'document'">📄 文档分类</template>
                <template v-else>🚀 应用分类</template>
              </span>
            </div>
            <button class="btn-secondary tap" @click="showAddForm = false">取消</button>
            <button class="btn-primary tap" @click="submitAdd">
              保存（写入 {{ JSON.stringify(finalDiagnosedAppType) }}）
            </button>
          </div>
        </div>
      </div>
    </Transition>

    <!-- P0-#ICON#DEBUG#OVERLAY：诊断浮层（按 Alt+Shift+D 切换）
         显示前 5 个 app 的实际状态：icon_path、data URL 长度、是否标记 iconError
         用户看到紫色羽毛时打开这个 → 一眼判断是数据问题还是渲染问题 -->
    <Transition name="fade">
      <div v-if="showIconDebug" class="icon-debug-overlay">
        <div class="icon-debug-card">
          <div class="icon-debug-title">🔍 图标诊断（Alt+Shift+D 关闭）</div>
          <div class="icon-debug-meta">
            <span>items: <b>{{ softwareStore.items.length }}</b></span>
            <span>cache: <b>{{ Object.keys(softwareStore.iconDataUrls).length }}</b></span>
            <span>missing: <b :style="{ color: missingIconCount > 0 ? '#ff8a5e' : '#7dffb0' }">{{ missingIconCount }}</b></span>
          </div>
          <div v-for="d in debugSample" :key="d.id" class="icon-debug-row">
            <div class="icon-debug-name">#{{ d.id }} {{ d.name }}</div>
            <div class="icon-debug-fields">
              <span :style="{ color: d.hasDataUrl ? '#7dffb0' : '#ff8a5e' }">
                dataURL: {{ d.hasDataUrl ? "✓ (" + d.dataUrlLen + "B)" : "✗" }}
              </span>
              <span :style="{ color: d.iconError ? '#ff8a5e' : '#7dffb0' }">
                iconError: {{ d.iconError ? "✓" : "✗" }}
              </span>
            </div>
            <div class="icon-debug-path">{{ d.icon_path }}</div>
            <div v-if="d.hasDataUrl" class="icon-debug-prefix">prefix: {{ d.dataUrlPrefix }}</div>
          </div>
        </div>
      </div>
    </Transition>

    <!-- 扫描模态框：网格 + 顶部搜索框 + 全选 -->
    <Transition name="modal">
      <div v-if="showScanModal" class="modal-mask" @click.self="closeScan">
        <div class="modal modal-scan">
          <div class="modal-header">
            <span>{{ scanning ? "正在扫描系统…" : `找到 ${scannedApps.length} 个软件` }}</span>
            <button class="modal-close tap" @click="closeScan">×</button>
          </div>
          <!-- 搜索框（全宽，置顶） -->
          <div v-if="!scanning" class="scan-search-row">
            <input
              v-model="scanSearch"
              class="scan-search-input"
              placeholder="🔍 搜索软件名 / 路径 / 来源"
              autofocus
            />
            <span class="scan-filtered">显示 {{ filteredScannedApps.length }} 个</span>
          </div>
          <!-- 用户反馈：顶部加 checkbox 让用户选择看哪些类型
               之前：全部 800+ 项堆一起，无法快速筛
               现在：4 个 checkbox（启动项/文件夹/文档/其他），默认全选
               - 启动项 = is_launchable=true（.exe/.lnk/.url/.bat/.cmd/.msi）
               - 文件夹 = app_type=folder
               - 文档   = app_type=document
               - 其他   = 其他不能启动的杂项（.ini/.dat/.log 等） -->
          <div v-if="!scanning" class="scan-type-filter">
            <label
              v-for="f in SCAN_TYPE_FILTERS"
              :key="f.key"
              class="scan-type-chip"
              :class="{ active: scanTypeFilter[f.key] }"
            >
              <input type="checkbox" v-model="scanTypeFilter[f.key]" />
              <span class="scan-type-icon">{{ f.icon }}</span>
              <span class="scan-type-label">{{ f.label }}</span>
              <span class="scan-type-count">{{ countByType[f.key] }}</span>
            </label>
          </div>
          <div class="modal-body">
            <div v-if="scanning" class="scan-loading">
              <div class="spinner"></div>
              <div>扫描开始菜单 / 桌面 / PATH…</div>
            </div>
            <div v-else class="scan-grid">
              <div
                v-for="a in filteredScannedApps"
                :key="a.path"
                class="scan-cell tap"
                :class="{ selected: selectedPaths.has(a.path) }"
                :title="`${a.name}\n${a.path}`"
                @click="toggleSelect(a.path)"
              >
                <div class="scan-check">{{ selectedPaths.has(a.path) ? "✓" : "" }}</div>
                <div class="scan-icon">
                  <img
                    v-if="a.icon_path && !scanIconError[a.path]"
                    :src="iconSrc(a.icon_path)"
                    :alt="a.name"
                    class="scan-icon-img"
                    @error="onScanIconError(a.path)"
                  />
                  <!-- 用户反馈：不要 hash 颜色块，改用类型本身辨识
                       之前：hash 颜色花花绿绿 → 用户："丑，无法区分"
                       现在：按 a.is_launchable / a.app_type 显示对应的"灰底+类型符号"
                       应用 → 💻、网址 → 🔗、文件夹 → 📁、文档 → 📄、其他 → 📦 -->
                  <span v-else class="scan-icon-text" :class="`scan-icon-${a.app_type || 'app'}`">
                    {{ scanTypeIcon(a) }}
                  </span>
                </div>
                <div class="scan-name">{{ a.name }}</div>
                <div class="scan-source">{{ a.source }}</div>
              </div>
              <div v-if="filteredScannedApps.length === 0" class="scan-empty">
                没有匹配 "{{ scanSearch }}" 的软件
              </div>
            </div>
          </div>
          <div class="modal-footer">
            <span class="scan-count">已选 {{ selectedPaths.size }} 个</span>
            <!-- P0-#Y#FIX#AUTO：移除"导入到"分类下拉框
                 分类全部由后端 detect_subtype_from_path 自动判（game/office/dev/...） -->
            <div class="scan-auto-hint" v-if="!scanning">
              <span class="auto-icon">✨</span>
              <span>分类自动识别</span>
            </div>
            <button class="btn-ghost tap" @click="toggleSelectAll">
              {{ selectedPaths.size === filteredScannedApps.length && filteredScannedApps.length > 0 ? "取消全选" : "全选" }}
            </button>
            <button class="btn-secondary tap" @click="closeScan">取消</button>
            <button
              class="btn-primary tap"
              :disabled="selectedPaths.size === 0"
              @click="confirmScan"
            >
              导入
            </button>
          </div>
        </div>
      </div>
    </Transition>
  </div>
</template>

<style scoped>
.view { padding-bottom: 20px; }

/* ============ P0-#Y-Collapsible：左侧可收缩 sidebar ============ */
.apps-body {
  display: flex;
  gap: 8px;
  align-items: flex-start;
}
.apps-main {
  flex: 1;
  min-width: 0;
}
.filter-sidebar {
  width: 42px; /* 收起态：只显示 emoji */
  flex-shrink: 0;
  display: flex;
  flex-direction: column;
  gap: 6px;
  padding: 6px 4px;
  background: var(--bg-primary);
  border: 1px solid var(--border);
  border-radius: 12px;
  transition: width 0.22s var(--ease-smooth), box-shadow 0.22s var(--ease-smooth);
  overflow: hidden;
  position: relative;
  align-self: stretch;
}
.filter-sidebar:hover,
.filter-sidebar.expanded {
  width: 178px; /* 展开态：emoji + label */
  box-shadow: 0 8px 24px rgba(0, 0, 0, 0.4);
  background: var(--bg-secondary);
}
.filter-sidebar::after {
  /* 收起态：右侧半透明提示条，提示"hover 可展开" */
  content: "";
  position: absolute;
  top: 50%;
  right: 2px;
  width: 2px;
  height: 28px;
  margin-top: -14px;
  background: linear-gradient(180deg, transparent, var(--accent) 50%, transparent);
  opacity: 0.4;
  border-radius: 1px;
  transition: opacity 0.2s;
  pointer-events: none;
}
.filter-sidebar:hover::after { opacity: 0; }

.sidebar-section-label {
  font-size: 10px;
  color: var(--text-faint);
  padding: 0 6px 2px;
  text-transform: uppercase;
  letter-spacing: 0.5px;
  font-weight: 600;
  white-space: nowrap;
  overflow: hidden;
}
.sidebar-divider {
  height: 1px;
  background: var(--border);
  margin: 4px 4px;
  flex-shrink: 0;
}
.sidebar-subtype {
  display: flex;
  flex-direction: column;
  gap: 4px;
}
.filter-sidebar .type-chips {
  display: flex;
  flex-direction: column;
  gap: 3px;
  flex-wrap: nowrap;
  margin-bottom: 0;
  width: 100%;
}
.filter-sidebar .type-chip {
  width: 100%;
  justify-content: flex-start;
  padding: 6px 8px;
  font-size: 11.5px;
  border-radius: 8px;
  min-height: 28px;
}
.filter-sidebar .type-chip-emoji {
  font-size: 14px;
  flex-shrink: 0;
  width: 18px;
  text-align: center;
  line-height: 1;
}
.filter-sidebar .type-chip-label {
  max-width: 0;
  opacity: 0;
  overflow: hidden;
  white-space: nowrap;
  transition: max-width 0.22s var(--ease-smooth) 0.04s, opacity 0.16s;
  pointer-events: none;
}
.filter-sidebar:hover .type-chip-label,
.filter-sidebar.expanded .type-chip-label {
  max-width: 110px;
  opacity: 1;
  pointer-events: auto;
}

/* AppView 专属的扫描模态框样式（其它 view-header / 按钮 / chip 样式都用 global） */
.scan-target {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  margin-right: auto;
  margin-left: 8px;
}
.scan-target-label {
  font-size: 11px;
  color: var(--text-muted);
  white-space: nowrap;
}
.scan-target-select {
  background: var(--bg-tertiary);
  color: var(--text-primary);
  border: 1px solid var(--border);
  border-radius: 6px;
  padding: 4px 8px;
  font-size: 11px;
  cursor: pointer;
  -webkit-app-region: no-drag;
  outline: none;
  max-width: 140px;
}
.scan-target-select:hover { border-color: var(--accent); }

/* ============ 自定义下拉（替代原生 select，emoji + 名称都能看清） ============ */
.scan-target-dd {
  position: relative;
  display: inline-block;
}
.scan-target-trigger {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  padding: 4px 10px;
  background: var(--bg-tertiary);
  color: var(--text-primary);
  border: 1px solid var(--border);
  border-radius: 6px;
  font-size: 12px;
  font-weight: 500;
  cursor: pointer;
  transition: all 0.15s var(--ease-smooth);
  -webkit-app-region: no-drag;
  outline: none;
  min-width: 110px;
}
.scan-target-trigger:hover {
  border-color: var(--accent);
  background: var(--bg-glass);
}
.scan-target-dd.open .scan-target-trigger {
  border-color: var(--accent);
  box-shadow: 0 0 0 2px var(--accent-soft);
}
.scan-target-icon { font-size: 13px; line-height: 1; flex-shrink: 0; }
.scan-target-name { font-size: 12px; flex: 1; text-align: left; }
.scan-target-arrow {
  font-size: 9px;
  color: var(--text-muted);
  margin-left: 2px;
  transition: transform 0.18s;
}
.scan-target-dd.open .scan-target-arrow { transform: rotate(180deg); }

.scan-target-menu {
  position: absolute;
  bottom: calc(100% + 6px); /* 向上展开，避免被 modal-footer 裁掉 */
  left: 0;
  min-width: 180px;
  max-height: 280px;
  overflow-y: auto;
  background: rgba(20, 20, 32, 0.96);
  backdrop-filter: blur(20px) saturate(160%);
  -webkit-backdrop-filter: blur(20px) saturate(160%);
  border: 1px solid var(--border-strong);
  border-radius: 10px;
  padding: 4px;
  box-shadow:
    0 -12px 32px rgba(0, 0, 0, 0.5),
    inset 0 1px 0 rgba(255, 255, 255, 0.08);
  display: flex;
  flex-direction: column;
  gap: 1px;
  z-index: 50;
}
/* P0-#ICON#DEBUG#OVERLAY：图标诊断浮层（按 Alt+Shift+D 切换） */
.icon-debug-overlay {
  position: fixed;
  top: 80px;
  right: 16px;
  z-index: 9999;
  pointer-events: none; /* 不挡点击 */
}
.icon-debug-card {
  background: rgba(15, 17, 30, 0.95);
  border: 1px solid rgba(255, 138, 94, 0.5);
  border-radius: 10px;
  padding: 12px 14px;
  font-family: ui-monospace, "Consolas", monospace;
  font-size: 11px;
  color: #cbd5e1;
  max-width: 520px;
  max-height: 70vh;
  overflow-y: auto;
  box-shadow: 0 8px 24px rgba(0, 0, 0, 0.6);
  pointer-events: auto;
  -webkit-app-region: no-drag;
}
.icon-debug-title { font-weight: 700; color: #ffc832; margin-bottom: 8px; font-size: 12px; }
.icon-debug-meta { display: flex; gap: 12px; margin-bottom: 10px; padding-bottom: 8px; border-bottom: 1px solid rgba(255, 255, 255, 0.1); }
.icon-debug-meta b { color: #fff; }
.icon-debug-row { padding: 6px 0; border-bottom: 1px dashed rgba(255, 255, 255, 0.06); }
.icon-debug-row:last-child { border-bottom: none; }
.icon-debug-name { color: #7dd3fc; font-weight: 600; margin-bottom: 3px; }
.icon-debug-fields { display: flex; gap: 10px; margin-bottom: 3px; }
.icon-debug-path { color: #94a3b8; font-size: 10px; word-break: break-all; opacity: 0.7; }
.icon-debug-prefix { color: #64748b; font-size: 10px; word-break: break-all; margin-top: 2px; }
.scan-target-item {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 6px 10px;
  border-radius: 6px;
  font-size: 12px;
  font-weight: 500;
  background: transparent;
  border: none;
  color: var(--text-secondary);
  cursor: pointer;
  transition: all 0.12s var(--ease-smooth);
  text-align: left;
  -webkit-app-region: no-drag;
}
.scan-target-item:hover {
  background: var(--accent-soft);
  color: var(--text-primary);
}
.scan-target-item.active {
  background: var(--accent-soft);
  color: var(--accent-bright);
  font-weight: 600;
}
.scan-target-item .scan-target-icon { font-size: 14px; }
.scan-target-item .scan-target-name { font-size: 12px; }
.scan-target-check {
  margin-left: auto;
  color: var(--accent-bright);
  font-weight: 700;
  font-size: 12px;
}
.dd-fade-enter-active { animation: ddIn 0.15s var(--ease-smooth); }
.dd-fade-leave-active { animation: ddOut 0.1s var(--ease-smooth); }
@keyframes ddIn {
  from { opacity: 0; transform: translateY(4px); }
  to   { opacity: 1; transform: translateY(0); }
}
@keyframes ddOut {
  from { opacity: 1; transform: translateY(0); }
  to   { opacity: 0; transform: translateY(4px); }
}

.empty {
  text-align: center;
  padding: 60px 20px;
  color: var(--text-muted);
}
.empty-icon { font-size: 48px; margin-bottom: 12px; opacity: 0.5; }
.empty-text { font-size: 13px; }

/* ============ 软件网格（默认 6 列，整体紧凑） ============ */
.app-grid {
  display: grid;
  grid-template-columns: repeat(6, 1fr);
  gap: 8px;
  padding: 4px 0;
}
/* 窄屏：5 列 */
@media (max-width: 559px) {
  .app-grid { grid-template-columns: repeat(5, 1fr); gap: 6px; }
  .app-icon { width: 36px; height: 36px; }
  .app-name { font-size: 11px; }
}
/* 中等宽度：6 列 + 略大按钮 */
@media (min-width: 700px) {
  .app-grid { gap: 10px; }
  .app-icon { width: 40px; height: 40px; }
  .app-name { font-size: 12px; }
}
/* 宽屏：6 列更大 + 间距 */
@media (min-width: 900px) {
  .app-grid { grid-template-columns: repeat(6, 1fr); gap: 14px; }
  .app-icon { width: 48px; height: 48px; }
  .app-name { font-size: 13px; }
}
/* 超大：8 列 */
@media (min-width: 1200px) {
  .app-grid { grid-template-columns: repeat(8, 1fr); gap: 16px; }
  .app-icon { width: 52px; height: 52px; }
}
.app-cell {
  position: relative;
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 6px;
  padding: 12px 6px 10px;
  background: var(--bg-secondary);
  border: 1px solid var(--border);
  border-radius: 14px;
  cursor: pointer;
  transition: all 0.18s var(--ease-smooth);
  user-select: none;
  -webkit-app-region: no-drag;
  overflow: hidden;
}
.app-cell::before {
  content: "";
  position: absolute;
  inset: 0;
  background: linear-gradient(135deg, rgba(99, 102, 241, 0) 0%, rgba(99, 102, 241, 0.08) 100%);
  opacity: 0;
  transition: opacity 0.2s;
  pointer-events: none;
}
.app-cell:hover {
  background: var(--bg-tertiary);
  border-color: var(--accent-soft);
  transform: translateY(-2px);
  box-shadow: 0 8px 20px rgba(0, 0, 0, 0.4), 0 0 0 1px var(--accent-soft);
}
.app-cell:hover::before { opacity: 1; }
.app-cell:active { transform: translateY(0) scale(0.97); }

.app-icon {
  width: 44px;
  height: 44px;
  display: flex;
  align-items: center;
  justify-content: center;
  border-radius: 10px;
  background: var(--bg-tertiary);
  box-shadow: 0 4px 12px rgba(0, 0, 0, 0.3);
  flex-shrink: 0;
  overflow: hidden;
  position: relative;
}
.app-icon-img {
  width: 100%;
  height: 100%;
  object-fit: contain;
  display: block;
}
.app-icon-text {
  font-size: 22px;
  font-weight: 700;
  line-height: 1;
  display: flex;
  align-items: center;
  justify-content: center;
}
/* P0-#ICON#TEXT#FIX：emoji 用 emoji 字体直接显示（彩色）
   之前：共用 background-clip:text + -webkit-text-fill-color:transparent
     → emoji 被透明化变成渐变轮廓（"紫色羽毛" bug） */
.app-icon-text.is-emoji {
  font-family: "Apple Color Emoji", "Segoe UI Emoji", "Noto Color Emoji", "Segoe UI Symbol", sans-serif;
  -webkit-text-fill-color: initial;
  color: initial;
  background: none;
}
/* 首字母用渐变文字（保持原设计） */
.app-icon-text.is-text {
  background: linear-gradient(135deg, var(--accent) 0%, var(--highlight) 100%);
  -webkit-background-clip: text;
  background-clip: text;
  -webkit-text-fill-color: transparent;
}

.app-name {
  font-size: 12px;
  font-weight: 500;
  color: var(--text-primary);
  text-align: center;
  width: 100%;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  line-height: 1.3;
}
.app-badge {
  font-size: 10px;
  color: var(--highlight-bright);
  font-weight: 600;
  background: var(--highlight-soft);
  padding: 1px 6px;
  border-radius: 8px;
}

.app-actions {
  position: absolute;
  top: 4px;
  right: 4px;
  display: flex;
  gap: 2px;
  opacity: 0;
  transition: opacity 0.15s;
}
.app-cell:hover .app-actions { opacity: 1; }
/* P0-#Z：软件区行内 confirm 浮条（view 级别，浮在网格上方不挡卡片） */
.app-confirm-bar {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 10px 14px;
  margin: 0 0 10px 0;
  background: linear-gradient(135deg, rgba(255, 94, 126, 0.16) 0%, rgba(255, 78, 110, 0.1) 100%);
  border: 1px solid rgba(255, 94, 126, 0.45);
  border-radius: 12px;
  box-shadow: 0 4px 16px rgba(255, 78, 110, 0.2), 0 0 0 1px rgba(0, 0, 0, 0.2);
  font-size: 13px;
  color: rgba(255, 255, 255, 0.95);
  backdrop-filter: blur(8px);
  -webkit-backdrop-filter: blur(8px);
}
.confirm-bar-icon {
  font-size: 16px;
  line-height: 1;
  flex-shrink: 0;
}
.confirm-bar-text {
  flex: 1;
  font-weight: 500;
}
.confirm-bar-actions {
  display: flex;
  gap: 6px;
  flex-shrink: 0;
}
.app-confirm-bar .confirm-btn {
  height: 28px;
  padding: 0 14px;
  border: none;
  border-radius: 8px;
  font-size: 12px;
  font-weight: 600;
  cursor: pointer;
  transition: all 0.15s var(--ease-smooth);
  display: inline-flex;
  align-items: center;
  justify-content: center;
}
.app-confirm-bar .confirm-btn:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}
.app-confirm-bar .confirm-btn.cancel {
  background: rgba(255, 255, 255, 0.1);
  color: rgba(255, 255, 255, 0.88);
  border: 1px solid rgba(255, 255, 255, 0.16);
}
.app-confirm-bar .confirm-btn.cancel:hover:not(:disabled) {
  background: rgba(255, 255, 255, 0.18);
}
.app-confirm-bar .confirm-btn.danger {
  background: linear-gradient(135deg, #ff5e7e 0%, #ff3b5c 100%);
  color: #fff;
  box-shadow: 0 2px 6px rgba(255, 59, 92, 0.4);
}
.app-confirm-bar .confirm-btn.danger:hover:not(:disabled) {
  transform: translateY(-1px);
  box-shadow: 0 4px 10px rgba(255, 59, 92, 0.55);
}
.confirm-bar-enter-active,
.confirm-bar-leave-active {
  transition: all 0.2s var(--ease-smooth);
}
.confirm-bar-enter-from,
.confirm-bar-leave-to {
  opacity: 0;
  transform: translateY(-8px);
}
.app-cell-confirming {
  z-index: 100;
  background: rgba(255, 78, 110, 0.1) !important;
  border-color: rgba(255, 94, 126, 0.5) !important;
}
.app-cell-confirming .app-icon,
.app-cell-confirming .app-name,
.app-cell-confirming .app-badge,
.app-cell-confirming .app-subtype {
  opacity: 0.35;
  transition: opacity 0.15s;
}
.app-action-btn {
  width: 22px;
  height: 22px;
  border: none;
  background: rgba(0, 0, 0, 0.5);
  color: white;
  border-radius: 6px;
  cursor: pointer;
  font-size: 10px;
  display: flex;
  align-items: center;
  justify-content: center;
  transition: all 0.15s;
  -webkit-app-region: no-drag;
  backdrop-filter: blur(4px);
}
.app-action-btn:hover { background: var(--accent); transform: scale(1.1); }

.app-cell-editing {
  grid-column: 1 / -1;
  background: var(--bg-tertiary);
  border-color: var(--accent);
}

.edit-form { display: flex; flex-direction: column; gap: 6px; width: 100%; }
.edit-actions { display: flex; gap: 6px; justify-content: flex-end; }

/* P0-#Y#5：编辑表单内嵌细分分类 chips */
.edit-subtype {
  display: flex;
  flex-direction: column;
  gap: 4px;
  margin-top: 2px;
}
.edit-subtype-label {
  font-size: 10px;
  color: var(--text-faint);
  text-transform: uppercase;
  letter-spacing: 0.5px;
  padding-left: 2px;
}
.edit-subtype-chips {
  display: flex;
  flex-wrap: wrap;
  gap: 4px;
}
.edit-subtype-chip {
  display: inline-flex;
  align-items: center;
  gap: 3px;
  padding: 3px 7px;
  border-radius: 9px;
  font-size: 10.5px;
  font-weight: 500;
  background: rgba(255, 255, 255, 0.04);
  color: var(--text-muted);
  border: 1px solid var(--border);
  cursor: pointer;
  transition: all 0.12s var(--ease-smooth);
  -webkit-app-region: no-drag;
  line-height: 1.2;
}
.edit-subtype-chip:hover {
  background: var(--bg-tertiary);
  color: var(--text-primary);
}
.edit-subtype-chip.active {
  background: var(--accent-soft);
  color: var(--accent-bright);
  border-color: var(--accent);
  font-weight: 600;
}
.edit-subtype-emoji { font-size: 11px; }

.form-input {
  background: rgba(0, 0, 0, 0.3);
  border: 1px solid var(--border-input);
  border-radius: 8px;
  padding: 7px 10px;
  color: var(--text-primary);
  font-size: 13px;
  font-family: inherit;
}
.form-input:focus { outline: none; border-color: var(--accent); }

/* ============ 模态框 ============ */
.modal-mask {
  position: fixed;
  inset: 0;
  background: rgba(0, 0, 0, 0.65);
  backdrop-filter: blur(8px);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 100;
}
.modal {
  background: linear-gradient(135deg, #1a1d2e 0%, #221a32 100%);
  border: 1px solid var(--border);
  border-radius: 16px;
  width: 90%;
  max-width: 380px;
  max-height: 80vh;
  display: flex;
  flex-direction: column;
  box-shadow: 0 24px 80px rgba(0, 0, 0, 0.7);
  overflow: hidden;
}
.modal-scan { max-width: 540px; }
.modal-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding: 12px 16px;
  border-bottom: 1px solid var(--border);
  font-size: 14px;
  font-weight: 600;
  color: var(--text-primary);
  background: rgba(0, 0, 0, 0.2);
}
.modal-close {
  width: 26px;
  height: 26px;
  border: none;
  background: transparent;
  color: var(--text-muted);
  font-size: 18px;
  cursor: pointer;
  border-radius: 6px;
  -webkit-app-region: no-drag;
}
.modal-close:hover { background: var(--danger); color: white; }
.modal-body { padding: 14px 16px; flex: 1; overflow-y: auto; }
.modal-footer {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
  padding: 10px 16px;
  border-top: 1px solid var(--border);
  align-items: center;
  background: rgba(0, 0, 0, 0.2);
}

.form-row { display: flex; flex-direction: column; gap: 4px; margin-bottom: 10px; }
.form-row label { font-size: 11px; color: var(--text-muted); }
.form-hint {
  font-size: 11px;
  color: var(--text-faint);
  margin-top: 2px;
  line-height: 1.4;
}
.auto-classify-info {
  flex-direction: row !important;
  align-items: center;
  gap: 6px;
  background: rgba(79, 124, 255, 0.06);
  border: 1px solid rgba(79, 124, 255, 0.18);
  border-radius: 8px;
  padding: 8px 12px;
  margin-top: 4px;
}
.auto-classify-info .auto-icon { font-size: 13px; }
.auto-classify-info .auto-text {
  font-size: 11.5px;
  color: var(--text-muted);
  line-height: 1.4;
}
.scan-auto-hint {
  display: flex;
  align-items: center;
  gap: 4px;
  font-size: 11px;
  color: var(--text-muted);
  padding: 3px 8px;
  background: rgba(79, 124, 255, 0.06);
  border: 1px solid rgba(79, 124, 255, 0.18);
  border-radius: 10px;
  margin-right: auto;
}
.scan-auto-hint .auto-icon { font-size: 12px; }

/* P0-#Y#FIX#URL#UI：路径类型快速模板 chips */
.path-templates {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
  margin-top: 4px;
}
.path-template-chip {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  padding: 5px 10px;
  background: var(--bg-tertiary);
  border: 1px solid var(--border);
  border-radius: 14px;
  cursor: pointer;
  font-size: 11.5px;
  color: var(--text-muted);
  transition: all 0.15s var(--ease-smooth);
  -webkit-app-region: no-drag;
  app-region: no-drag;
}
.path-template-chip:hover {
  background: var(--bg-secondary);
  border-color: var(--accent);
  color: var(--text-primary);
  transform: translateY(-1px);
}
.path-template-chip.active {
  background: rgba(79, 124, 255, 0.15);
  border-color: var(--accent);
  color: var(--accent-bright);
}
.path-template-emoji { font-size: 13px; }
.path-template-label { font-weight: 500; }

.path-input {
  font-family: ui-monospace, "Cascadia Code", "Source Code Pro", Menlo, Consolas, monospace;
  font-size: 12.5px;
}

/* P0-#Y#FIX#PICK#UI：路径行 — 输入框 + 浏览按钮并排 */
.path-input-row {
  display: flex;
  gap: 6px;
  align-items: stretch;
}
.path-input-row .path-input {
  flex: 1 1 0;
  min-width: 0;
}
.browse-btn {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  padding: 0 12px;
  background: var(--bg-tertiary);
  border: 1px solid var(--border);
  border-radius: var(--radius);
  cursor: pointer;
  font-size: 12px;
  color: var(--text-muted);
  white-space: nowrap;
  transition: all 0.15s var(--ease-smooth);
  -webkit-app-region: no-drag;
  app-region: no-drag;
}
.browse-btn:hover {
  background: var(--bg-secondary);
  border-color: var(--accent);
  color: var(--text-primary);
  transform: translateY(-1px);
}
.browse-icon { font-size: 13px; }

/* P0-#Y#FIX#URL#UI：网址类下展开 3 个子协议 chip */
.url-subtemplates {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 6px;
  margin-top: 8px;
  padding: 8px 10px;
  background: rgba(79, 124, 255, 0.04);
  border: 1px dashed rgba(79, 124, 255, 0.2);
  border-radius: 8px;
}
.url-sub-label {
  font-size: 11px;
  color: var(--text-muted);
  margin-right: 2px;
}
.url-sub-chip {
  display: inline-flex;
  align-items: center;
  gap: 3px;
  padding: 3px 8px;
  background: var(--bg-primary);
  border: 1px solid var(--border);
  border-radius: 10px;
  cursor: pointer;
  font-size: 11px;
  color: var(--text-muted);
  transition: all 0.15s var(--ease-smooth);
  -webkit-app-region: no-drag;
  app-region: no-drag;
}
.url-sub-chip:hover {
  background: rgba(79, 124, 255, 0.12);
  border-color: var(--accent);
  color: var(--text-primary);
}
.url-sub-enter-active, .url-sub-leave-active {
  transition: all 0.2s var(--ease-smooth);
  overflow: hidden;
}
.url-sub-enter-from, .url-sub-leave-to {
  opacity: 0;
  max-height: 0;
  margin-top: 0;
  padding-top: 0;
  padding-bottom: 0;
}
.url-sub-enter-to, .url-sub-leave-from {
  opacity: 1;
  max-height: 60px;
}

/* 扫描模态：搜索框 */
.scan-search-row {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 10px 16px;
  background: rgba(0, 0, 0, 0.2);
  border-bottom: 1px solid var(--border);
}
.scan-search-input {
  flex: 1;
  background: rgba(0, 0, 0, 0.35);
  border: 1px solid var(--border-input);
  border-radius: 10px;
  padding: 8px 12px;
  color: var(--text-primary);
  font-size: 13px;
  outline: none;
  transition: all 0.15s;
  font-family: inherit;
}
.scan-search-input:focus {
  border-color: var(--accent);
  background: rgba(0, 0, 0, 0.5);
  box-shadow: 0 0 0 3px var(--accent-soft);
}
.scan-search-input::placeholder { color: var(--text-faint); }
.scan-filtered {
  font-size: 11px;
  color: var(--text-muted);
  white-space: nowrap;
}

/* 用户反馈：扫描顶部 checkbox 类型过滤
   4 个 chip：启动项/文件夹/文档/其他，点击切换 active */
.scan-type-filter {
  display: flex;
  gap: 8px;
  padding: 8px 16px;
  flex-wrap: wrap;
  border-bottom: 1px solid rgba(255, 255, 255, 0.06);
}
.scan-type-chip {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  padding: 4px 10px;
  background: rgba(255, 255, 255, 0.04);
  border: 1px solid rgba(255, 255, 255, 0.08);
  border-radius: 12px;
  font-size: 11px;
  color: rgba(255, 255, 255, 0.5);
  cursor: pointer;
  user-select: none;
  transition: all 0.15s;
}
.scan-type-chip input {
  display: none;
}
.scan-type-chip.active {
  background: rgba(99, 102, 241, 0.18);
  border-color: rgba(99, 102, 241, 0.5);
  color: #fff;
}
.scan-type-icon { font-size: 12px; }
.scan-type-label { font-weight: 500; }
.scan-type-count {
  display: inline-block;
  min-width: 16px;
  padding: 0 4px;
  background: rgba(255, 255, 255, 0.08);
  border-radius: 8px;
  text-align: center;
  font-size: 10px;
  color: rgba(255, 255, 255, 0.7);
}
.scan-type-chip.active .scan-type-count {
  background: rgba(99, 102, 241, 0.4);
  color: #fff;
}

.scan-loading {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 12px;
  padding: 40px 20px;
  color: var(--text-muted);
}
.spinner {
  width: 32px;
  height: 32px;
  border: 3px solid var(--border);
  border-top-color: var(--accent);
  border-radius: 50%;
  animation: spin 0.8s linear infinite;
}
@keyframes spin { to { transform: rotate(360deg); } }

/* 扫描网格：6 列（关键修复：之前 4 列明显不够） */
.scan-grid {
  display: grid;
  grid-template-columns: repeat(6, 1fr);
  gap: 8px;
}
/* 窄屏：5 列 */
@media (max-width: 559px) {
  .scan-grid { grid-template-columns: repeat(5, 1fr); }
}
.scan-cell {
  position: relative;
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 4px;
  padding: 10px 4px 8px;
  background: var(--bg-secondary);
  border: 1.5px solid transparent;
  border-radius: 12px;
  cursor: pointer;
  transition: all 0.15s var(--ease-smooth);
  overflow: hidden;
  -webkit-app-region: no-drag;
}
.scan-cell:hover {
  background: var(--bg-tertiary);
  border-color: var(--border-strong);
  transform: translateY(-1px);
}
.scan-cell.selected {
  background: var(--accent-soft);
  border-color: var(--accent);
  box-shadow: 0 0 0 1px var(--accent), 0 4px 12px rgba(99, 102, 241, 0.3);
}
.scan-check {
  position: absolute;
  top: 4px;
  right: 4px;
  width: 16px;
  height: 16px;
  background: var(--accent);
  color: white;
  border-radius: 50%;
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: 10px;
  font-weight: 700;
  opacity: 0;
  transform: scale(0.5);
  transition: all 0.15s var(--ease-spring);
}
.scan-cell.selected .scan-check { opacity: 1; transform: scale(1); }

.scan-icon {
  width: 38px;
  height: 38px;
  display: flex;
  align-items: center;
  justify-content: center;
  border-radius: 8px;
  background: var(--bg-tertiary);
  overflow: hidden;
  flex-shrink: 0;
}
.scan-icon-img {
  width: 100%;
  height: 100%;
  object-fit: contain;
  display: block;
}
.scan-icon-text {
  font-size: 18px;
  line-height: 1;
  color: rgba(255, 255, 255, 0.55);  /* 浅灰半透明，统一克制，不抢戏 */
  display: flex;
  align-items: center;
  justify-content: center;
  width: 100%;
  height: 100%;
}
.scan-name {
  font-size: 11px;
  font-weight: 500;
  color: var(--text-primary);
  text-align: center;
  width: 100%;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.scan-source {
  font-size: 9px;
  color: var(--text-faint);
  background: rgba(0, 0, 0, 0.3);
  padding: 0 5px;
  border-radius: 4px;
  font-family: monospace;
}
.scan-empty {
  grid-column: 1 / -1;
  text-align: center;
  padding: 40px 20px;
  color: var(--text-muted);
  font-size: 13px;
}
.scan-count { font-size: 11px; color: var(--text-muted); margin-right: auto; }

.stagger-item { animation: staggerIn 0.35s var(--ease-out) backwards; }
@keyframes staggerIn {
  from { opacity: 0; transform: translateY(8px) scale(0.95); }
  to   { opacity: 1; transform: translateY(0) scale(1); }
}

/* 骨架单元格（与 app-cell 同布局，纯视觉占位） */
.skeleton-cell {
  pointer-events: none;
  border: 1px solid transparent;
  background: var(--bg-secondary);
}
.skeleton-cell .skeleton-icon {
  width: 44px;
  height: 44px;
  border-radius: 10px;
  background: linear-gradient(
    90deg,
    rgba(255, 255, 255, 0.04) 0%,
    rgba(255, 255, 255, 0.12) 50%,
    rgba(255, 255, 255, 0.04) 100%
  );
  background-size: 200% 100%;
  animation: skeletonShimmer 1.6s ease-in-out infinite;
}
.skeleton-cell .skeleton-line {
  height: 8px;
  border-radius: 4px;
  background: linear-gradient(
    90deg,
    rgba(255, 255, 255, 0.04) 0%,
    rgba(255, 255, 255, 0.12) 50%,
    rgba(255, 255, 255, 0.04) 100%
  );
  background-size: 200% 100%;
  animation: skeletonShimmer 1.6s ease-in-out infinite;
  margin-top: 6px;
}
@keyframes skeletonShimmer {
  0% { background-position: 200% 0; }
  100% { background-position: -200% 0; }
}

.modal-enter-active, .modal-leave-active { transition: opacity 0.2s; }
.modal-enter-from, .modal-leave-to { opacity: 0; }

/* ============ P0-#Y：类型色条（顶部 2.5px 彩色横条）============ */
.type-stripe {
  position: absolute;
  top: 0;
  left: 0;
  right: 0;
  height: 2.5px;
  border-radius: 14px 14px 0 0;
  z-index: 1;
  pointer-events: none;
}
.stripe-app      { background: linear-gradient(90deg, #6366f1, #8b5cf6); }
.stripe-folder   { background: linear-gradient(90deg, #fbbf24, #f59e0b); }
.stripe-document { background: linear-gradient(90deg, #10b981, #34d399); }
.stripe-url      { background: linear-gradient(90deg, #ec4899, #f472b6); }

/* 类型细分标签（替代原 app-type-badge，写在 app-name 下方一行） */
.app-subtype {
  font-size: 10px;
  font-weight: 500;
  color: var(--text-muted);
  line-height: 1;
  margin-top: -2px;
  letter-spacing: 0.2px;
  opacity: 0.85;
  max-width: 100%;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

/* 🔴 诊断标签：直接显示 SQLite 中 app_type 字段的原始值（肉眼排查数据错误） */
.app_type_diag_chip {
  font-size: 9px;
  font-weight: 700;
  line-height: 1;
  padding: 3px 6px;
  border-radius: 6px;
  letter-spacing: 0.2px;
  margin-top: -2px;
  user-select: all;
  cursor: help;
  font-family: "Consolas", "JetBrains Mono", monospace;
}
/* url：蓝色 */
.app_type_diag_chip.dtc-url {
  background: rgba(59, 130, 246, 0.18);
  color: #60a5fa;
  border: 1px solid rgba(59, 130, 246, 0.45);
}
/* folder：黄色（告警！如果是网址卡片显示黄色，说明 DB 存错了！） */
.app_type_diag_chip.dtc-folder {
  background: rgba(234, 179, 8, 0.22);
  color: #eab308;
  border: 1px solid rgba(234, 179, 8, 0.55);
  animation: dtc-warn-blink 1.8s ease-in-out infinite;
}
@keyframes dtc-warn-blink {
  0%, 100% { opacity: 1; }
  50% { opacity: 0.55; }
}
/* app：灰色 */
.app_type_diag_chip.dtc-app {
  background: rgba(156, 163, 175, 0.18);
  color: #9ca3af;
  border: 1px solid rgba(156, 163, 175, 0.4);
}
/* document：绿色 */
.app_type_diag_chip.dtc-document {
  background: rgba(34, 197, 94, 0.18);
  color: #22c55e;
  border: 1px solid rgba(34, 197, 94, 0.45);
}
/* EMPTY / 其他：红色严重告警（DB 里 app_type 是空字符串或未知值！） */
.app_type_diag_chip.dtc-EMPTY,
.app_type_diag_chip:not(.dtc-url):not(.dtc-folder):not(.dtc-app):not(.dtc-document) {
  background: rgba(239, 68, 68, 0.22);
  color: #ef4444;
  border: 1px solid rgba(239, 68, 68, 0.6);
  animation: dtc-err-blink 1s ease-in-out infinite;
}
@keyframes dtc-err-blink {
  0%, 100% { opacity: 1; }
  50% { opacity: 0.4; }
}

/* 🔴 诊断面板（添加弹窗里实时显示 activeTemplate / 推断结果 / 最终写入值） */
.diag-panel {
  background: rgba(239, 68, 68, 0.08);
  border: 1.5px dashed rgba(239, 68, 68, 0.55);
  border-radius: 10px;
  padding: 10px 12px;
  margin-top: 8px;
  user-select: text;
}
.diag-title {
  font-size: 11.5px;
  font-weight: 700;
  color: #ef4444;
  margin-bottom: 8px;
  letter-spacing: 0.2px;
}
.diag-row {
  display: flex;
  flex-direction: row;
  gap: 8px;
  align-items: flex-start;
  padding: 3px 0;
  font-size: 11px;
  line-height: 1.5;
}
.diag-row.diag-final {
  border-top: 1px dashed rgba(239, 68, 68, 0.4);
  margin-top: 5px;
  padding-top: 7px;
}
.diag-k {
  flex-shrink: 0;
  color: var(--text-muted);
  min-width: 200px;
  font-weight: 600;
}
.diag-v {
  flex: 1;
  font-family: "Consolas", "JetBrains Mono", monospace;
  padding: 2px 6px;
  border-radius: 5px;
  font-weight: 700;
  word-break: break-all;
  white-space: pre-wrap;
}
.diag-v.dtc-url {
  background: rgba(59, 130, 246, 0.18);
  color: #60a5fa;
}
.diag-v.dtc-folder {
  background: rgba(234, 179, 8, 0.22);
  color: #eab308;
}
.diag-v.dtc-app {
  background: rgba(156, 163, 175, 0.18);
  color: #9ca3af;
}
.diag-v.dtc-document {
  background: rgba(34, 197, 94, 0.18);
  color: #22c55e;
}
.diag-v.dtc-EMPTY,
.diag-v:not(.dtc-url):not(.dtc-folder):not(.dtc-app):not(.dtc-document) {
  background: rgba(239, 68, 68, 0.22);
  color: #ef4444;
}
.diag-final-v {
  font-size: 12px;
  padding: 4px 8px;
  animation: dtc-warn-blink 2.5s ease-in-out infinite;
}
.diag-hint {
  font-size: 10px;
  color: var(--text-muted);
  margin-top: 6px;
  line-height: 1.5;
}
.diag-hint code {
  background: rgba(255, 255, 255, 0.08);
  padding: 1px 5px;
  border-radius: 4px;
  font-family: "Consolas", monospace;
  color: var(--accent-bright);
}

/* 🔴 modal-footer 诊断区（保存按钮旁边直接显示最终写入值） */
.modal-footer {
  display: flex;
  align-items: center;
  justify-content: flex-end;
  gap: 10px;
  padding: 12px 16px;
  border-top: 1px solid var(--border);
  background: var(--bg-tertiary);
  border-radius: 0 0 14px 14px;
}
.footer-diag {
  flex: 1;
  display: flex;
  align-items: center;
  gap: 8px;
}
.footer-diag-label {
  font-size: 11px;
  font-weight: 600;
  color: var(--text-muted);
}
.footer-diag-chip {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  padding: 5px 10px;
  border-radius: 8px;
  font-family: "Consolas", "JetBrains Mono", monospace;
  font-size: 11px;
  font-weight: 700;
}
.footer-diag-chip.dtc-url {
  background: rgba(59, 130, 246, 0.18);
  color: #60a5fa;
  border: 1px solid rgba(59, 130, 246, 0.5);
}
.footer-diag-chip.dtc-folder {
  background: rgba(234, 179, 8, 0.22);
  color: #eab308;
  border: 1px solid rgba(234, 179, 8, 0.55);
  animation: dtc-warn-blink 1.5s ease-in-out infinite;
}
.footer-diag-chip.dtc-document {
  background: rgba(34, 197, 94, 0.18);
  color: #22c55e;
  border: 1px solid rgba(34, 197, 94, 0.45);
}
.footer-diag-chip.dtc-app,
.footer-diag-chip:not(.dtc-url):not(.dtc-folder):not(.dtc-document) {
  background: rgba(156, 163, 175, 0.18);
  color: #9ca3af;
  border: 1px solid rgba(156, 163, 175, 0.4);
}

/* ============ P0-#Y：类型筛选 chips ============ */
.type-chips {
  display: flex;
  gap: 6px;
  margin-bottom: 8px;
  flex-wrap: wrap;
  align-items: center;
}
.type-chip {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  padding: 5px 10px;
  border-radius: 14px;
  font-size: 11.5px;
  font-weight: 500;
  cursor: pointer;
  background: var(--bg-secondary);
  color: var(--text-muted);
  border: 1px solid var(--border);
  transition: all 0.15s var(--ease-smooth);
  -webkit-app-region: no-drag;
  white-space: nowrap;
  line-height: 1.2;
}
.type-chip:hover {
  background: var(--bg-tertiary);
  color: var(--text-primary);
  border-color: var(--border-strong);
  transform: translateY(-1px);
}
.type-chip.active {
  background: var(--accent-soft);
  color: var(--accent-bright);
  border-color: var(--accent);
  font-weight: 600;
  box-shadow: 0 2px 6px rgba(99, 102, 241, 0.25);
}
.type-chip-emoji { font-size: 12px; line-height: 1; }

/* 不同类型 chip active 时的颜色（让"文件夹/文档"和"软件"区分开） */
.type-chip[data-type="folder"].active {
  background: rgba(251, 191, 36, 0.2);
  border-color: #f59e0b;
  color: #fcd34d;
}
.type-chip[data-type="document"].active {
  background: rgba(16, 185, 129, 0.2);
  border-color: #10b981;
  color: #6ee7b7;
}
.type-chip[data-type="url"].active {
  background: rgba(236, 72, 153, 0.2);
  border-color: #ec4899;
  color: #f9a8d4;
}

/* ============ P0-#Y#4：右键菜单 - 修改细分分类 ============ */
.ctx-menu {
  position: fixed;
  z-index: 9999;
  min-width: 180px;
  max-width: 220px;
  background: rgba(20, 20, 32, 0.96);
  backdrop-filter: blur(20px) saturate(160%);
  -webkit-backdrop-filter: blur(20px) saturate(160%);
  border: 1px solid rgba(255, 255, 255, 0.12);
  border-radius: 12px;
  padding: 6px;
  box-shadow:
    0 16px 40px rgba(0, 0, 0, 0.5),
    inset 0 1px 0 rgba(255, 255, 255, 0.08);
  display: flex;
  flex-direction: column;
  gap: 1px;
}
.ctx-menu-title {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  padding: 6px 10px 4px;
  font-size: 11px;
}
.ctx-menu-name {
  color: var(--text-primary);
  font-weight: 600;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  max-width: 130px;
}
.ctx-menu-type {
  font-size: 10px;
  color: var(--text-muted);
  padding: 1px 6px;
  background: var(--bg-secondary);
  border-radius: 8px;
  flex-shrink: 0;
}
.ctx-menu-divider {
  height: 1px;
  background: rgba(255, 255, 255, 0.08);
  margin: 4px 0;
}
.ctx-menu-section {
  padding: 4px 10px 2px;
  font-size: 10px;
  color: var(--text-faint);
  text-transform: uppercase;
  letter-spacing: 0.5px;
}
.ctx-menu-item {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 6px 10px;
  border-radius: 8px;
  font-size: 12px;
  font-weight: 500;
  background: transparent;
  border: none;
  color: var(--text-secondary);
  cursor: pointer;
  transition: all 0.12s var(--ease-smooth);
  text-align: left;
  -webkit-app-region: no-drag;
}
.ctx-menu-item:hover {
  background: var(--accent-soft);
  color: var(--text-primary);
}
.ctx-menu-item.active {
  background: var(--accent-soft);
  color: var(--accent-bright);
  font-weight: 600;
}
.ctx-menu-emoji { font-size: 14px; line-height: 1; flex-shrink: 0; }
.ctx-menu-check {
  margin-left: auto;
  color: var(--accent-bright);
  font-weight: 700;
}
.ctx-menu-empty {
  padding: 8px 10px;
  font-size: 11px;
  color: var(--text-faint);
  text-align: center;
  font-style: italic;
}
.ctx-fade-enter-active { animation: ctxIn 0.12s var(--ease-smooth); }
.ctx-fade-leave-active { animation: ctxOut 0.08s var(--ease-smooth); }
@keyframes ctxIn {
  from { opacity: 0; transform: scale(0.92) translateY(-4px); }
  to { opacity: 1; transform: scale(1) translateY(0); }
}
@keyframes ctxOut {
  from { opacity: 1; transform: scale(1); }
  to { opacity: 0; transform: scale(0.95); }
}

/* ============ P0-#Y#2：二级 chips ============ */

/* 二级 chip 按数据 sub 高亮不同色（游戏红 / 办公蓝 / 开发绿 / ...） */
.type-chip[data-sub="game"].active {
  background: rgba(239, 68, 68, 0.2);
  border-color: #ef4444;
  color: #fca5a5;
}
.type-chip[data-sub="office"].active {
  background: rgba(59, 130, 246, 0.2);
  border-color: #3b82f6;
  color: #93c5fd;
}
.type-chip[data-sub="dev"].active {
  background: rgba(16, 185, 129, 0.2);
  border-color: #10b981;
  color: #6ee7b7;
}
.type-chip[data-sub="utility"].active {
  background: rgba(148, 163, 184, 0.2);
  border-color: #94a3b8;
  color: #cbd5e1;
}
.type-chip[data-sub="media"].active {
  background: rgba(217, 70, 239, 0.2);
  border-color: #d946ef;
  color: #f0abfc;
}
.type-chip[data-sub="design"].active {
  background: rgba(244, 114, 182, 0.2);
  border-color: #f472b6;
  color: #f9a8d4;
}
.type-chip[data-sub="word"].active,
.type-chip[data-sub="excel"].active,
.type-chip[data-sub="ppt"].active,
.type-chip[data-sub="pdf"].active,
.type-chip[data-sub="text"].active,
.type-chip[data-sub="image"].active {
  background: rgba(99, 102, 241, 0.18);
  border-color: var(--accent);
  color: var(--accent-bright);
}

.subtype-fade-enter-active, .subtype-fade-leave-active { transition: opacity 0.18s, transform 0.18s; }
.subtype-fade-enter-from, .subtype-fade-leave-to { opacity: 0; transform: translateY(-4px); }

/* ============ P0-#Y：系统级拖入遮罩 ============ */
.drop-overlay {
  position: absolute;
  inset: 0;
  z-index: 200;
  display: flex;
  align-items: center;
  justify-content: center;
  background: rgba(99, 102, 241, 0.18);
  backdrop-filter: blur(8px);
  -webkit-backdrop-filter: blur(8px);
  border: 2px dashed var(--accent);
  border-radius: 14px;
  margin: 8px;
  pointer-events: none; /* 遮罩不挡事件，让 drop 透传给 webview */
}
.drop-overlay-inner {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 8px;
  padding: 20px 32px;
  background: rgba(20, 20, 28, 0.85);
  border-radius: 16px;
  box-shadow: 0 8px 32px rgba(0, 0, 0, 0.5);
}
.drop-overlay-icon { font-size: 42px; }
.drop-overlay-text {
  font-size: 14px;
  font-weight: 600;
  color: var(--text-primary);
}
.drop-overlay-hint {
  font-size: 11px;
  color: var(--text-muted);
}
.drop-fade-enter-active, .drop-fade-leave-active { transition: opacity 0.18s; }
.drop-fade-enter-from, .drop-fade-leave-to { opacity: 0; }
</style>

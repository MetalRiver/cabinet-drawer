<script setup lang="ts">
import { ref, onMounted, onUnmounted, computed, h, nextTick } from "vue";
import { convertFileSrc } from "@tauri-apps/api/core";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { useAppStore } from "../stores/app";
import { useSoftwareStore } from "../stores/software";
import { type ScannedApp, relinkApp, openAppLocation, pickPath, getAppEdition, readIconAsDataUrl, restoreFromTrash, perfProbeEnabled, perfProbeReport } from "../api";
import ConfirmInline from "../components/ConfirmInline.vue";
import StatusInline from "../components/settings/StatusLine.vue";

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

// ===== v0.4.1 软件失效治理：集中列表 + 手动检查 + 批量清退 =====
const showInvalidPanel = ref(false);
const selectedInvalidIds = ref<Set<number>>(new Set());
const invalidChecking = ref(false);
const invalidBatchConfirm = ref(false);
const invalidBatchBusy = ref(false);
const invalidBatchResult = ref<{ moved: number; skipped: number; failed: number } | null>(null);
const invalidPanelError = ref("");

const isTestEdition = ref(false);
void getAppEdition()
  .then((e) => (isTestEdition.value = e === "acceptance"))
  .catch(() => {});

// 任务 A：验收测试数据播种 / 重置（仅便携测试版；后端同规则门禁）
const seedingFixtures = ref(false);
async function seedFixtures() {
  if (seedingFixtures.value) return;
  seedingFixtures.value = true;
  try {
    const r = await softwareStore.seedTestFixtures();
    appStore.showClipToast(
      "success",
      `已播种 ${r.inserted} 条验收记录（清理上轮 ${r.cleared_previous} 条），失效面板已刷新`
    );
  } catch (e) {
    appStore.showClipToast("info", "播种失败：" + String(e));
  } finally {
    seedingFixtures.value = false;
  }
}
const clearingFixtures = ref(false);
async function resetFixtures() {
  if (clearingFixtures.value) return;
  clearingFixtures.value = true;
  try {
    const n = await softwareStore.clearTestFixtures();
    appStore.showClipToast(
      "success",
      n > 0 ? `已重置：${n} 条测试记录移入回收站（可恢复）` : "没有需要重置的测试记录"
    );
  } catch (e) {
    appStore.showClipToast("info", "重置失败：" + String(e));
  } finally {
    clearingFixtures.value = false;
  }
}

const KIND_LABELS: Record<string, string> = {
  fixed_missing: "固定盘缺失 · 可清退",
  removable_offline: "移动盘/光盘不在位（保护）",
  network_offline: "网络位置离线（保护）",
  uncertain: "状态不确定（保护）",
};

// ===== U2：失效治理——用户可读的检查界面 =====
const invalidReasons = ref<Set<number>>(new Set());
function toggleInvalidReason(id: number) {
  const next = new Set(invalidReasons.value);
  if (next.has(id)) next.delete(id);
  else next.add(id);
  invalidReasons.value = next;
}
const invalidFixedCount = computed(
  () => softwareStore.invalidItems.filter((i) => i.kind === "fixed_missing").length
);
const invalidProtectedCount = computed(
  () => softwareStore.invalidItems.length - invalidFixedCount.value
);
const invalidSummary = computed(() => {
  const n = softwareStore.invalidItems.length;
  if (n === 0) return "未发现需要处理的记录";
  return `真实缺失 ${invalidFixedCount.value} 条 · 无法确认 ${invalidProtectedCount.value} 条（受保护，不可清退）`;
});
function invalidIssue(it: { kind: string }): string {
  switch (it.kind) {
    case "fixed_missing":
      return "程序文件已不存在，点击无法启动";
    case "network_offline":
      return "网络位置暂时连不上";
    case "removable_offline":
      return "移动盘 / 光盘当前不在位";
    default:
      return "无法确认这个入口当前是否可用";
  }
}
function invalidAdvice(it: { kind: string }): string {
  switch (it.kind) {
    case "fixed_missing":
      return "建议：重新关联程序；确认不用了再移入回收站（可恢复）";
    case "network_offline":
      return "建议：连上网络后点「重新检查」";
    case "removable_offline":
      return "建议：接回磁盘后点「重新检查」";
    default:
      return "建议：先重新关联，确认后再处理；不推荐直接清退";
  }
}
async function relinkInvalid(item: unknown) {
  await relinkMissing(item);
  await softwareStore.loadInvalidApps();
}
function goTrashFromPanel() {
  showInvalidPanel.value = false;
  window.dispatchEvent(new CustomEvent("open-trash"));
}

function toggleInvalidPanel() {
  showInvalidPanel.value = !showInvalidPanel.value;
  invalidBatchResult.value = null;
  invalidPanelError.value = "";
  if (showInvalidPanel.value) {
    void softwareStore.loadInvalidApps();
  } else {
    selectedInvalidIds.value = new Set();
  }
}

/** 手动重新检查：先自动重定位（.lnk 目标/常见目录按名），再刷新最新状态 */
async function runInvalidCheck() {
  if (invalidChecking.value) return;
  invalidChecking.value = true;
  invalidPanelError.value = "";
  try {
    const fixed = await softwareStore.runHealthCheck();
    appStore.showClipToast(
      "success",
      fixed > 0 ? `自动重定位 ${fixed} 项，列表已更新` : "检查完成：没有可自动重定位的条目"
    );
  } catch (e) {
    invalidPanelError.value = String(e);
  } finally {
    invalidChecking.value = false;
  }
}

function toggleSelectInvalid(id: number) {
  const next = new Set(selectedInvalidIds.value);
  if (next.has(id)) next.delete(id);
  else next.add(id);
  selectedInvalidIds.value = next;
}

function selectAllClearable() {
  const next = new Set<number>();
  for (const it of softwareStore.clearableInvalid) next.add(it.id);
  selectedInvalidIds.value = next;
}

async function confirmBatchTrash() {
  if (invalidBatchBusy.value) return;
  const ids = Array.from(selectedInvalidIds.value);
  if (ids.length === 0) return;
  invalidBatchBusy.value = true;
  invalidPanelError.value = "";
  try {
    const r = await softwareStore.moveAppsToTrash(ids);
    invalidBatchResult.value = { moved: r.moved, skipped: r.skipped, failed: r.errors.length };
    selectedInvalidIds.value = new Set();
    appStore.refreshTrashCount();
  } catch (e) {
    invalidPanelError.value = String(e);
  } finally {
    invalidBatchBusy.value = false;
  }
}

// P0-#ICON#DEBUG#OVERLAY：诊断浮层（按 Alt+Shift+D 临时切换）
// 原因：用户报"图标变紫色羽毛"，但 cache 文件 + db 路径都对，需要看实际渲染状态
const showIconDebug = ref(false);
// ✅ 命名处理器（原为匿名监听且从不清理，锁屏-解锁每轮累积一份）
// 仅开发构建可用：vite build 产物 import.meta.env.DEV=false → 生产按键不生效、浮层永不出现
const onIconDebugKeydown = (e: KeyboardEvent) => {
  if (!import.meta.env.DEV) return;
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
  // P1-C 测量（仅测试版 + 环境变量开关；正式版恒 false）
  void perfProbeEnabled()
    .then((on) => {
      perfProbe.value = on;
      if (on && "PerformanceObserver" in window) {
        try {
          new PerformanceObserver((list) => {
            for (const e of list.getEntries()) perfLongTasks.push(Math.round(e.duration));
          }).observe({ entryTypes: ["longtask"] });
        } catch {}
      }
      // 测量模式：无需手动点「扫描」，进入软件区后自动跑一次默认扫描，
      // 减少 GUI 操作对帧率的干扰（仅测试版开关生效）
      if (on) {
        void perfProbeReport(JSON.stringify({ stage: "enabled", at: new Date().toISOString() })).catch(
          (e) => console.warn("[perf] enabled-marker failed", e)
        );
        setTimeout(() => {
          if (!showScanModal.value) void openScan();
        }, 2500);
      }
    })
    .catch(() => {});
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
    if (!uniquePaths.length) {
      appStore.showClipToast("info", "无法识别拖入的文件路径");
      return;
    }
    try {
      const result = await softwareStore.importPaths(uniquePaths);
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
          if (!paths.length) return;
          const uniquePaths = [...new Set(paths.map((p) => p.toLowerCase()))];
          try {
            const result = await softwareStore.importPaths(uniquePaths);
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
// Batch 2.2 验收修正：长来源简写（"开始菜单(所有用户)"占两行不可接受）；
// 完整来源仍保留在 title 悬停与来源徽标原始值中
const SOURCE_SHORT: Record<string, string> = {
  "开始菜单": "菜单",
  "开始菜单(所有用户)": "公共菜单",
  "桌面": "桌面",
  "桌面(公共)": "公共桌面",
  "Program Files": "PF",
  "Program Files (x86)": "PF x86",
  "PATH": "PATH",
  "自定义目录": "自定义",
  "Store/应用": "Store",
};
function shortSource(s: string): string {
  return SOURCE_SHORT[s] ?? s;
}

// Batch 2.2 验收修正：扫描弹窗图标改走 data-URL 通道（与软件库同源），
// 不依赖 asset 协议 scope 配置。
const scanIconDataUrls = ref<Record<string, string>>({});
async function loadScanIcons() {
  const t0 = performance.now();
  try {
    const withIcons = scannedApps.value.filter((a) => a.icon_path);
    const results = await Promise.allSettled(
      withIcons.map(async (a) => {
        const url = await readIconAsDataUrl(a.icon_path);
        return { path: a.path, url };
      })
    );
    const map: Record<string, string> = {};
    for (const r of results) {
      if (r.status === "fulfilled" && r.value.url) map[r.value.path] = r.value.url;
    }
    scanIconDataUrls.value = map;
    if (perfProbe.value) {
      perfIconsReadyMs.value = Math.round(performance.now() - t0);
      perfIconCount.value = Object.keys(map).length;
      perfIconBytes.value = Object.values(map).reduce((n, u) => n + u.length, 0);
    }
  } catch (e) {
    console.warn("[scan] 图标 data-URL 加载失败", e);
  }
}
function scanIconUrl(a: ScannedApp): string {
  return scanIconDataUrls.value[a.path] || iconSrc(a.icon_path);
}

function scanTypeIcon(a: ScannedApp): string {
  const t = a.app_type || "app";
  if (t === "url") return "🔗";
  if (t === "folder") return "📁";
  if (t === "document") return "📄";
  // app 类型：用户只关心能否启动
  return a.is_launchable ? "💻" : "📦";
}

// ===== Batch 2.2：扫描弹窗（推荐/高级 + 分组 + 状态词）=====
// 措辞约定：只用「已识别 / 需要确认」描述入口的解析状态，
// 不使用"可信度/安全等级"字样——分组不构成对软件安全性的判断。
// 默认零勾选；「选择全部推荐项」是显式按钮。
const scanMode = ref<"recommended" | "advanced">("recommended");

// U2：扫描结果视图切换（列表 / 卡片），真实生效并记住选择
const SCAN_VIEW_KEY = "drawer_scan_view";
const scanViewMode = ref<"list" | "card">(
  (() => {
    try {
      // 默认紧凑图标网格；用户手动切到列表后记住选择
      return localStorage.getItem(SCAN_VIEW_KEY) === "list" ? "list" : "card";
    } catch {
      return "card";
    }
  })()
);
function setScanView(m: "list" | "card") {
  scanViewMode.value = m;
  try {
    localStorage.setItem(SCAN_VIEW_KEY, m);
  } catch {}
}
/** 行/卡状态：✓ 选中 / 在库 / 待恢复 / 需确认 */
function scanRowStatus(a: ScannedApp): string {
  if (selectedPaths.value.has(a.path)) return "✓";
  if (a.already_imported) return "在库";
  if (a.in_trash) return "待恢复";
  if (a.status === "needs_confirmation") return "需确认";
  return "";
}
/** 状态色：选中=强调色 / 在库=绿 / 待恢复=琥珀 / 需确认=蓝（一眼可分辨） */
function scanRowStatusClass(a: ScannedApp): string {
  if (selectedPaths.value.has(a.path)) return "st-sel";
  if (a.already_imported) return "st-ok";
  if (a.in_trash) return "st-trash";
  if (a.status === "needs_confirmation") return "st-confirm";
  return "";
}

function matchesScanSearch(a: ScannedApp): boolean {
  const q = scanSearch.value.trim().toLowerCase();
  if (!q) return true;
  return (
    a.name.toLowerCase().includes(q) ||
    a.path.toLowerCase().includes(q) ||
    (a.target || "").toLowerCase().includes(q) ||
    a.source.toLowerCase().includes(q)
  );
}

// 推荐应用：已识别的软件入口（开始菜单/桌面）
const recommendedApps = computed(() =>
  scannedApps.value.filter(
    (a) =>
      a.status !== "needs_confirmation" &&
      a.app_type === "app" &&
      matchesScanSearch(a)
  )
);
// 网址：独立类型，不混入推荐软件
const urlCandidates = computed(() =>
  scannedApps.value.filter((a) => a.app_type === "url" && matchesScanSearch(a))
);
// 需要确认：解析失败/失效目标/.ps1 等无法自动确认的入口
const needsConfirmApps = computed(() =>
  scannedApps.value.filter(
    (a) =>
      a.status === "needs_confirmation" &&
      a.app_type === "app" &&
      matchesScanSearch(a)
  )
);

// Store 应用：UWP/MSIX 打包应用（Batch 2.4）
const storeApps = computed(() =>
  scannedApps.value.filter((a) => a.app_type === "uwp" && matchesScanSearch(a))
);

const scanAnyVisible = computed(
  () =>
    recommendedApps.value.length +
      urlCandidates.value.length +
      storeApps.value.length +
      needsConfirmApps.value.length >
    0
);

function selectAllRecommended() {
  for (const a of recommendedApps.value) {
    if (isScanSelectable(a)) selectedPaths.value.add(a.path);
  }
  selectedPaths.value = new Set(selectedPaths.value); // 触发响应式
}

function clearScanSelection() {
  selectedPaths.value = new Set();
}

function toggleSelect(a: ScannedApp) {
  if (!isScanSelectable(a)) {
    // P1-B：已在库 / 在回收站都不参与批量导入，给出明确下一步
    appStore.showClipToast(
      "info",
      a.in_trash
        ? `「${a.name}」在回收站中，点该行的「↩ 恢复此软件」即可重新入库（保留原分类）`
        : `「${a.name}」已在软件库中，无需重复导入`
    );
    return;
  }
  if (selectedPaths.value.has(a.path)) {
    selectedPaths.value.delete(a.path);
  } else {
    selectedPaths.value.add(a.path);
  }
  selectedPaths.value = new Set(selectedPaths.value); // 触发响应式
}

/** P1-B：把扫描命中的回收站条目显式恢复（不自动恢复、不新建重复条目）。
 *  恢复的是原记录本身：分类 / 使用次数 / 密码关联全部保留。 */
const restoringPath = ref<string | null>(null);
async function restoreTrashedCandidate(a: ScannedApp) {
  if (!a.trash_id) {
    appStore.showClipToast("info", "未找到对应的回收站记录，请重新扫描后再试");
    return;
  }
  restoringPath.value = a.path;
  try {
    const n = await restoreFromTrash("apps", a.trash_id);
    if (!n) {
      appStore.showClipToast("info", "该记录已不在回收站（可能已被清理），请重新扫描");
    } else {
      appStore.showClipToast("success", `已恢复「${a.name}」到软件库（保留原分类与使用记录）`);
    }
    await softwareStore.loadItems();
    appStore.refreshTrashCount();
    // 当前候选就地转为「已在库」，避免再被勾选或再次提示恢复
    a.in_trash = false;
    a.trash_id = null;
    a.already_imported = n > 0;
    selectedPaths.value.delete(a.path);
    selectedPaths.value = new Set(selectedPaths.value);
  } catch (e) {
    appStore.showClipToast("info", "恢复失败：" + String(e));
  } finally {
    restoringPath.value = null;
  }
}

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

import { mergeScanCandidates, applyScanOutcomeIfCurrent, isScanSelectable } from "../lib/scan-merge";
// ===== Batch 2.3：扫描进度监听（真实阶段反馈，来自后端 scan:progress 事件）=====
const scanProgress = ref<{ stage: string; done: number; total: number } | null>(null);
const advIncludePath = ref(false); // PATH 开关：默认关、不记住（每次进入弹窗重置）
const advIncludeUwp = ref(false); // Store 应用开关：默认关、不记住（Batch 2.4 受控接入）
let unlistenScanProgress: (() => void) | null = null;
// Batch 2.4（D1/D3）：扫描会话令牌 + 在飞守卫——
// D1 旧缺陷：openScan 无守卫，弹窗关闭后立刻重开会与在飞 IPC 并发；
// D3 旧缺陷：旧任务迟到结果会合入新任务候选列表。
let scanSession = 0;
function nextScanSession(): number {
  scanSession += 1;
  return scanSession;
}

function scanProgressText(): string {
  const p = scanProgress.value;
  if (!p) return "正在准备扫描…";
  if (p.stage === "enumerate") return `枚举入口…已发现 ${p.done} 个`;
  if (p.stage === "enumerate-path") return `PATH 枚举…已发现 ${p.done} 个`;
  if (p.stage === "resolve") return `解析快捷方式目标（批 ${p.done}/${p.total}）…`;
  if (p.stage === "icons") return "提取图标…";
  return "扫描中…";
}

async function listenScanProgress() {
  unlistenScanProgress?.();
  try {
    const { listen } = await import("@tauri-apps/api/event");
    unlistenScanProgress = await listen<{
      stage: string;
      done: number;
      total: number;
    }>("scan:progress", (e) => {
      // Batch 2.4 收口：非扫描状态下忽略进度事件（旧任务迟到事件不得覆盖新任务状态）
      if (!scanning.value) return;
      scanProgress.value = e.payload;
    });
  } catch (e) {
    console.warn("scan progress 监听失败", e);
  }
}

function applyScanOutcome(
  outcome: {
    apps: ScannedApp[];
    cancelled: boolean;
    truncated: boolean;
  },
  session: number
) {
  // Batch 2.4（D3）：非当前会话的结果（旧任务迟到返回）直接丢弃
  const current = applyScanOutcomeIfCurrent(outcome, session, scanSession);
  if (!current) {
    console.warn("[scan] 丢弃旧会话的迟到结果（session", session, "≠", scanSession, "）");
    return;
  }
  scannedApps.value = mergeScanCandidates(scannedApps.value, current.apps);
  if (current.cancelled) {
    appStore.showClipToast("info", "扫描已取消：保留已完成部分");
  }
  if (current.truncated) {
    appStore.showClipToast(
      "info",
      "目录内容超过扫描上限（20000 项），结果可能不完整"
    );
  }
}

// ===== P1-C（v0.4.3 验收）：扫描列表滚动性能测量脚手架 =====
// 仅测试版 + DRAWER_PERF_PROBE=1 生效（正式版命令恒 false，不执行任何测量代码）。
// 目的：在真实 WebView2 里拿到「图标加载完成后的滚动帧间隔」基线，
// 而不是用静态推断代替测量。结果写入 setting perf.scan_probe 供隔离机读取。
const perfProbe = ref(false);
const perfIconsReadyMs = ref(0);
const perfIconCount = ref(0);
const perfIconBytes = ref(0);
const perfLongTasks: number[] = [];

function perfPercentile(arr: number[], p: number): number {
  if (!arr.length) return 0;
  const a = [...arr].sort((x, y) => x - y);
  return a[Math.min(a.length - 1, Math.floor(a.length * p))];
}

function sweepScrollFrames(el: HTMLElement, durationMs: number): Promise<number[]> {
  return new Promise((resolve) => {
    const frames: number[] = [];
    const start = performance.now();
    let last = start;
    let done = false;
    const finish = () => {
      if (done) return;
      done = true;
      clearTimeout(guard);
      resolve(frames);
    };
    // 兜底：窗口一旦被系统判定遮挡，Chromium 会暂停 rAF（测量循环会挂死）。
    // 超时后带已采集帧返回，保证结果一定落盘；帧数不足时数据本身会显示 count 偏低。
    const guard = setTimeout(finish, durationMs + 2000);
    const max = Math.max(0, el.scrollHeight - el.clientHeight);
    const step = () => {
      if (done) return;
      const now = performance.now();
      frames.push(now - last);
      last = now;
      const p = Math.min(1, (now - start) / durationMs);
      el.scrollTop = Math.round(max * p);
      if (p < 1) requestAnimationFrame(step);
      else finish();
    };
    requestAnimationFrame(step);
  });
}

async function runScanPerfProbe(scanMs: number) {
  if (!perfProbe.value) return;
  const body = document.querySelector<HTMLElement>(".modal-scan .modal-body");
  if (!body) return;
  await new Promise((r) => setTimeout(r, 800)); // 等首帧图标解码/布局稳定
  try {
    await perfProbeReport(JSON.stringify({ stage: "sweep-start", maxScroll: body.scrollHeight - body.clientHeight }));
  } catch (e) { console.warn("[perf] marker failed", e); }
  const warm = await sweepScrollFrames(body, 2400);
  body.scrollTop = 0;
  await new Promise((r) => setTimeout(r, 400));
  const second = await sweepScrollFrames(body, 2400);
  const frames = [...warm, ...second];
  const result = {
    at: new Date().toISOString(),
    scanMs,
    iconsReadyMs: perfIconsReadyMs.value,
    iconCount: perfIconCount.value,
    iconBytes: perfIconBytes.value,
    candidates: scannedApps.value.length,
    domRows: body.querySelectorAll(".scan-cell").length,
    frames: {
      count: frames.length,
      p50: +perfPercentile(frames, 0.5).toFixed(1),
      p95: +perfPercentile(frames, 0.95).toFixed(1),
      max: +Math.max(...frames).toFixed(1),
      over20ms: frames.filter((f) => f > 20).length,
      over33ms: frames.filter((f) => f > 33).length,
    },
    longTasks: perfLongTasks.slice(0, 40),
  };
  try {
    await perfProbeReport(JSON.stringify(result, null, 1));
  } catch (e) {
    console.warn("[perf] probe persist failed", e);
  }
}

// 打开扫描（默认推荐扫描，自动执行）
async function openScan() {
  if (scanning.value) {
    // Batch 2.4（D1）：上一任务 IPC 仍在飞时拒绝重入
    appStore.showClipToast("info", "上一次扫描尚未结束，请稍候");
    return;
  }
  showScanModal.value = true;
  scanning.value = true;
  scannedApps.value = [];
  selectedPaths.value = new Set();
  scanSearch.value = "";
  scanMode.value = "recommended";
  advIncludePath.value = false; // PATH 开关默认关、不记住
  advIncludeUwp.value = false; // Store 开关默认关、不记住
  scanTargetCategoryId.value = undefined;
  scanTargetDropdownOpen.value = false;
  void listenScanProgress();
  const session = nextScanSession();
  const scanT0 = performance.now();
  try {
    const outcome = await softwareStore.scan(scanMode.value);
    applyScanOutcome(outcome, session);
    void loadScanIcons().then(() =>
      runScanPerfProbe(Math.round(performance.now() - scanT0))
    );
  } catch (e) {
    console.error("scan failed", e);
    appStore.showClipToast("info", "扫描失败：" + String(e));
  } finally {
    scanning.value = false;
    scanProgress.value = null;
  }
}

function closeScan() {
  showScanModal.value = false;
  scanSearch.value = "";
  scanTargetDropdownOpen.value = false;
  // 若扫描仍在进行，通知后端终止（返回后由 finally 收尾）
  if (scanning.value) {
    void softwareStore.cancelScan();
  }
  unlistenScanProgress?.();
  unlistenScanProgress = null;
}

// Batch 2.2/2.3：模式切换。推荐=自动执行；高级=仅显示控制面板，
// 由用户主动选择扫什么（PF / +PATH / 自定义目录），绝不自动跑。
async function switchScanMode(mode: "recommended" | "advanced") {
  if (scanMode.value === mode || scanning.value) return;
  scanMode.value = mode;
  selectedPaths.value = new Set();
  if (mode === "recommended") {
    scanning.value = true;
    const session = nextScanSession();
    try {
      const outcome = await softwareStore.scan("recommended");
      applyScanOutcome(outcome, session);
    void loadScanIcons();
    } catch (e) {
      console.error("scan failed", e);
      appStore.showClipToast("info", "扫描失败：" + String(e));
    } finally {
      scanning.value = false;
      scanProgress.value = null;
    }
  }
}

// Batch 2.3：高级扫描——Program Files（可选连带 PATH）
async function runAdvancedScan() {
  if (scanning.value) return;
  scanning.value = true;
  scanProgress.value = null;
  const session = nextScanSession();
  try {
    const outcome = await softwareStore.scan(
      "advanced",
      advIncludePath.value,
      advIncludeUwp.value
    );
    applyScanOutcome(outcome, session);
    void loadScanIcons();
  } catch (e) {
    console.error("advanced scan failed", e);
    appStore.showClipToast("info", "高级扫描失败：" + String(e));
  } finally {
    scanning.value = false;
    scanProgress.value = null;
  }
}

// Batch 2.3：自定义目录扫描（C/D/E 等本地磁盘；便携软件入口）
async function runCustomDirScan() {
  if (scanning.value) return;
  const p = await pickPath({ mode: "folder", title: "选择要扫描的目录（便携软件所在文件夹）" });
  if (!p) return;
  scanning.value = true;
  scanProgress.value = null;
  const session = nextScanSession();
  try {
    const outcome = await softwareStore.scanCustomDir(p);
    applyScanOutcome(outcome, session);
    void loadScanIcons();
  } catch (e) {
    appStore.showClipToast("info", "目录扫描失败：" + String(e));
  } finally {
    scanning.value = false;
    scanProgress.value = null;
  }
}

// Batch 2.3：取消（真正终止——后端在遍历/解析批边界停止并返回部分结果）
async function onCancelScan() {
  try {
    await softwareStore.cancelScan();
  } catch (e) {
    console.warn("cancel scan failed", e);
  }
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


// Batch 2.1：批量导入（幂等）——汇总展示 新增/已存在/回收站/失败
async function confirmScan() {
  const selected = scannedApps.value.filter(
    (a) => selectedPaths.value.has(a.path) && !a.already_imported
  );
  if (selected.length === 0) {
    appStore.showClipToast("info", "请先勾选要导入的软件（已在库中的条目无需导入）");
    return;
  }
  scanning.value = true;
  try {
    const summary = await softwareStore.importScanned(
      selected,
      scanTargetCategoryId.value
    );
    await softwareStore.loadItems();
    softwareStore.setCategory(scanTargetCategoryId.value ?? null);
    const parts = [`新增 ${summary.added} 个`, `已存在跳过 ${summary.already_exists} 个`];
    if (summary.in_trash > 0)
      parts.push(`回收站已有 ${summary.in_trash} 个同身份记录（在扫描列表点「↩ 恢复此软件」）`);
    if (summary.failed.length > 0) {
      parts.push(`失败 ${summary.failed.length} 个`);
      appStore.showClipToast("info", "导入完成：" + parts.join(" · "));
      console.warn("scan import failures:", summary.failed);
    } else {
      appStore.showClipToast("success", "导入完成：" + parts.join(" · "));
    }
    showScanModal.value = false;
    scanSearch.value = "";
    scanTargetCategoryId.value = undefined;
    scanTargetDropdownOpen.value = false;
    appStore.searchQuery = "";
  } catch (e) {
    appStore.showClipToast("info", "导入失败：" + String(e));
  } finally {
    scanning.value = false;
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
// ===== Phase 2D：应用生命周期失效治理 =====
const relinkingId = ref<number | null>(null);

async function relinkMissing(item: any) {
  relinkingId.value = item.id;
  try {
    const picked = await pickPath({ mode: "file", title: `重新关联「${item.name}」` });
    if (!picked) return;
    await relinkApp(item.id, picked);
    appStore.showClipToast("success", `已重新关联「${item.name}」`);
    await softwareStore.loadItems();
  } catch (e) {
    appStore.showClipToast("info", "重新关联失败：" + String(e));
  } finally {
    relinkingId.value = null;
  }
}

async function openLocation(item: any) {
  try {
    await openAppLocation(item.id);
  } catch (e) {
    appStore.showClipToast("info", String(e));
  }
}

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
    appStore.showClipToast("success", "已从抽屉柜移除");
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
        <!-- 任务 A：健康检查常驻入口（正常=健康态，异常=失效数） -->
        <button
          class="action-btn tap"
          :class="softwareStore.invalidCount > 0 ? 'action-btn-warn' : 'action-btn-text'"
          :title="softwareStore.invalidCount > 0
            ? `${softwareStore.invalidCount} 条失效记录，点击集中治理`
            : '软件健康检查：当前全部正常，点击查看'"
          @click="toggleInvalidPanel"
        >
          <template v-if="softwareStore.invalidCount > 0">⚠️ 失效 ({{ softwareStore.invalidCount }})</template>
          <template v-else>🩺 健康</template>
        </button>
        <button class="action-btn action-btn-text tap" @click="openScan" title="扫描系统">
          🔍 <span>扫描</span>
        </button>
        <button class="action-btn action-btn-primary tap" @click="openAddForm" title="手动添加">+</button>
      </div>
    </div>

    <!-- U2：软件检查（用户可读；技术原因折叠在「查看原因」） -->
    <div v-if="showInvalidPanel" class="invalid-panel">
      <div class="invalid-panel-head">
        <div class="invalid-head-text">
          <b>{{ softwareStore.invalidItems.length ? `${softwareStore.invalidItems.length} 条记录需要处理` : "软件检查" }}</b>
          <span class="invalid-sub">{{ invalidSummary }}</span>
        </div>
        <div class="invalid-head-actions">
          <button class="s-btn-mini tap" :disabled="invalidChecking" @click="runInvalidCheck" data-interactive>
            {{ invalidChecking ? "检查中…" : "🔄 重新检查" }}
          </button>
          <button class="s-btn-mini tap" @click="toggleInvalidPanel" data-interactive>收起</button>
        </div>
      </div>

      <div v-if="isTestEdition" class="invalid-toolbar" style="margin-top: 0;">
        <button class="s-btn-mini tap" :disabled="seedingFixtures" data-interactive @click="seedFixtures">
          {{ seedingFixtures ? "播种中…" : "🧪 播种验收测试数据" }}
        </button>
        <button class="s-btn-mini tap" :disabled="clearingFixtures" data-interactive @click="resetFixtures">
          {{ clearingFixtures ? "重置中…" : "♻️ 重置测试数据" }}
        </button>
        <span class="invalid-hint">仅作用于本测试版数据目录（[TEST] 前缀记录，软删除可恢复）</span>
      </div>

      <div v-if="softwareStore.invalidLoading" class="invalid-empty">检查中…</div>
      <div v-else-if="softwareStore.invalidItems.length === 0" class="invalid-empty">
        ✅ 全部正常：没有发现失效或需要确认的软件
      </div>
      <template v-else>
        <div class="inv-legend">
          <span class="inv-tag missing">真实缺失 {{ invalidFixedCount }}</span>
          <span class="inv-tag protected">无法确认 {{ invalidProtectedCount }} · 已保护</span>
        </div>
        <div class="invalid-list">
          <div
            v-for="it in softwareStore.invalidItems"
            :key="it.id"
            class="inv-row"
            :class="{ protected: it.kind !== 'fixed_missing' }"
          >
            <label
              v-if="it.kind === 'fixed_missing'"
              class="invalid-check"
              title="勾选后可批量移入回收站"
            >
              <input
                type="checkbox"
                :checked="selectedInvalidIds.has(it.id)"
                @change="toggleSelectInvalid(it.id)"
              />
            </label>
            <span v-else class="invalid-shield" title="受保护：不提供清退操作">🛡</span>
            <div class="inv-main">
              <div class="inv-name">{{ it.name }}</div>
              <div class="inv-issue">{{ invalidIssue(it) }}</div>
              <div class="inv-advice">{{ invalidAdvice(it) }}</div>
              <button
                class="inv-reason-btn tap"
                data-interactive
                @click.stop="toggleInvalidReason(it.id)"
              >{{ invalidReasons.has(it.id) ? "收起原因" : "查看原因" }}</button>
              <div v-if="invalidReasons.has(it.id)" class="inv-details">
                <div v-if="it.path">路径：<code>{{ it.path }}</code></div>
                <div v-if="it.lnk_target">目标：<code>{{ it.lnk_target }}</code></div>
                <div>检查结果：{{ KIND_LABELS[it.kind] || it.kind }}</div>
              </div>
            </div>
            <div class="inv-actions">
              <button class="s-btn-mini tap" data-interactive @click.stop="relinkInvalid(it)">重新关联</button>
              <button
                v-if="(it.app_type || 'app') !== 'url'"
                class="s-btn-mini tap"
                data-interactive
                @click.stop="openLocation(it)"
              >打开位置</button>
            </div>
          </div>
        </div>

        <div class="inv-foot">
          <button class="s-btn-mini tap" data-interactive @click="selectAllClearable">
            全选可清退（{{ softwareStore.clearableInvalid.length }}）
          </button>
          <button
            class="s-btn-mini s-btn-warn tap"
            :disabled="selectedInvalidIds.size === 0 || invalidBatchBusy"
            data-interactive
            @click="invalidBatchConfirm = true"
          >移入回收站（{{ selectedInvalidIds.size }}）</button>
          <button class="s-btn-mini tap" data-interactive @click="goTrashFromPanel">
            🗑️ 去回收站恢复
          </button>
          <span class="invalid-hint">只删除收录记录，不卸载任何 Windows 程序</span>
        </div>
        <ConfirmInline
          v-if="invalidBatchConfirm"
          :title="`将把选中的 ${selectedInvalidIds.size} 条收录记录移入回收站（可恢复）。不卸载任何 Windows 程序。确定继续吗？`"
          confirm-text="移入回收站"
          :loading="invalidBatchBusy"
          @cancel="invalidBatchConfirm = false"
          @confirm="confirmBatchTrash"
        />
        <StatusInline v-if="invalidBatchResult" tone="ok" :text="`✓ 已移入回收站 ${invalidBatchResult.moved} 条 ｜ 跳过（已在回收站）${invalidBatchResult.skipped} 条${invalidBatchResult.failed ? ` ｜ 失败 ${invalidBatchResult.failed} 条` : ''}`" />
        <StatusInline v-if="invalidPanelError" tone="err" :text="invalidPanelError" />
      </template>
    </div>

    <!-- P0-#Y：类型筛选 chips 改为左侧可收缩 sidebar（默认仅图标列、hover 展开） -->
    <!-- 主内容容器：左 sidebar + 右主区 -->
    <!-- P0-#APPC#PER#OVERLAY：展开层改为浮在网格上方（absolute），不再挤压 apps-main ——
         旧实现 hover 时 sidebar 宽度 42→178 动画，apps-main 每帧重排、全部卡片重算布局，
         大列表下滚轮/悬停明显卡顿；浮层方案让网格零重排 -->
    <div class="apps-body">
      <!-- 左侧可收缩 sidebar（固定 42px 轨道） -->
      <div class="filter-sidebar">
        <div
          class="sidebar-inner"
          :class="{ expanded: sidebarExpanded }"
          @mouseenter="sidebarExpanded = true"
          @mouseleave="sidebarExpanded = false"
        >
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
              'app-cell-confirming': confirmingDeleteId === item.id,
              'app-cell-missing': item.available === false && (item.app_type || 'app') !== 'url'
            }"
            :data-type="item.app_type || 'app'"
            :style="{ '--delay': `${Math.min(index * 0.03, 0.3)}s` }"
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
                <span
                  v-if="item.available === false && (item.app_type || 'app') !== 'url'"
                  class="app-icon-warn"
                  title="路径失效 / 应用不可用"
                >!</span>
              </div>
              <!-- 图标式网格：名称最多两行；异常仅角标，不遮挡名称 -->
              <div class="app-name" :title="item.name">{{ item.name }}</div>
              <button
                class="app-more tap"
                data-interactive
                title="更多操作（启动 / 编辑 / 打开位置 / 重新关联 / 移除）"
                @click.stop="openContextMenu($event, item)"
              >···</button>
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
          <!-- P1 修复（v0.4.3 验收轮）：正常软件卡补「打开所在位置」——
               该入口原先只在失效卡上，正常软件查看实际路径不方便（用户原始反馈缺口） -->
          <button
            v-if="ctxMenu.item && (ctxMenu.item.app_type || 'app') !== 'url'"
            class="ctx-menu-item tap"
            @click="(closeContextMenu(), openAppLocation(ctxMenu.item.id))"
          >
            <span class="ctx-menu-emoji">📂</span>
            <span>打开所在位置</span>
          </button>
          <button class="ctx-menu-item tap" @click="(closeContextMenu(), startEdit(ctxMenu.item))">
            <span class="ctx-menu-emoji">✏️</span>
            <span>编辑名称/路径</span>
          </button>
          <template v-if="ctxMenu.item">
            <div class="ctx-menu-divider"></div>
            <button
              v-if="ctxMenu.item.available === false && (ctxMenu.item.app_type || 'app') !== 'url'"
              class="ctx-menu-item tap"
              @click="(closeContextMenu(), relinkMissing(ctxMenu.item))"
            >
              <span class="ctx-menu-emoji">🔗</span>
              <span>重新关联程序</span>
            </button>
            <button class="ctx-menu-item tap" @click="(closeContextMenu(), launchApp(ctxMenu.item.id))">
              <span class="ctx-menu-emoji">▶️</span>
              <span>启动</span>
            </button>
            <button class="ctx-menu-item danger tap" @click="(closeContextMenu(), askDeleteApp(ctxMenu.item.id, ctxMenu.item.name))">
              <span class="ctx-menu-emoji">🗑️</span>
              <span>从抽屉柜移除</span>
            </button>
          </template>
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
            <button class="btn-secondary tap" @click="showAddForm = false">取消</button>
            <button class="btn-primary tap" @click="submitAdd">保存</button>
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
            <span>{{ scanning ? "正在扫描系统…" : `找到 ${scannedApps.length} 个入口` }}</span>
            <button class="modal-close tap" @click="closeScan">×</button>
          </div>
          <!-- Batch 2.2：推荐/高级模式切换（均为真实扫描；未实现的入口不提供按钮） -->
          <div class="scan-mode-tabs">
            <button
              class="scan-mode-tab"
              :class="{ active: scanMode === 'recommended' }"
              :disabled="scanning"
              @click="switchScanMode('recommended')"
            >
              ⭐ 推荐扫描
            </button>
            <button
              class="scan-mode-tab"
              :class="{ active: scanMode === 'advanced' }"
              :disabled="scanning"
              @click="switchScanMode('advanced')"
            >
              🔍 高级扫描
            </button>
            <span class="scan-mode-hint">{{
              scanMode === "recommended"
                ? "开始菜单与桌面的应用入口"
                : "按需选择扫描范围；结果累积显示"
            }}</span>
          </div>
          <!-- Batch 2.3：高级扫描控制（全部真实入口；PATH 默认关、不记住） -->
          <div v-if="!scanning && scanMode === 'advanced'" class="scan-adv-controls">
            <button class="scan-adv-btn" data-interactive @click="runAdvancedScan">
              📁 扫描 Program Files
            </button>
            <label class="scan-adv-path-toggle">
              <input type="checkbox" v-model="advIncludePath" />
              连带 PATH 目录（开发命令较多，默认关）
            </label>
            <label class="scan-adv-path-toggle">
              <input type="checkbox" v-model="advIncludeUwp" />
              连带 Store 应用（UWP/MSIX，启动走系统激活接口）
            </label>
            <button class="scan-adv-btn" data-interactive @click="runCustomDirScan">
              📂 扫描指定目录…（便携软件 / D 盘等）
            </button>
          </div>
          <!-- 搜索框（全宽，置顶） -->
          <div v-if="!scanning" class="scan-search-row">
            <input
              v-model="scanSearch"
              class="scan-search-input"
              placeholder="🔍 搜索软件名 / 路径 / 来源"
              autofocus
            />
            <div class="scan-view-toggle" role="group" aria-label="扫描结果视图">
              <button
                class="scan-view-btn tap"
                :class="{ active: scanViewMode === 'list' }"
                :aria-pressed="scanViewMode === 'list'"
                title="列表视图"
                data-interactive
                @click="setScanView('list')"
              >☰</button>
              <button
                class="scan-view-btn tap"
                :class="{ active: scanViewMode === 'card' }"
                :aria-pressed="scanViewMode === 'card'"
                title="卡片视图"
                data-interactive
                @click="setScanView('card')"
              >▦</button>
            </div>
          </div>
          <div class="modal-body">
            <div v-if="scanning" class="scan-loading">
              <div class="spinner"></div>
              <div>{{ scanProgressText() }}</div>
              <button class="scan-cancel-btn" data-interactive @click="onCancelScan">
                取消扫描
              </button>
            </div>
            <template v-else>
              <!-- 推荐应用：已识别的软件入口 -->
              <div v-if="recommendedApps.length" class="scan-group-head">
                ⭐ 推荐应用
                <span class="scan-group-count">{{ recommendedApps.length }}</span>
                <span class="scan-group-tag">已识别</span>
                <button class="scan-group-select tap" data-interactive @click="selectAllRecommended">
                  全选推荐项
                </button>
              </div>
              <div v-if="recommendedApps.length" class="scan-grid" :class="scanViewMode === 'list' ? 'scan-list' : 'scan-cards'">
                <div
                  v-for="a in recommendedApps"
                  :key="a.path"
                  class="scan-cell tap"
                  :class="{ selected: selectedPaths.has(a.path), added: a.already_imported, trash: a.in_trash }"
                  :title="`${a.name}\n入口：${a.path}${a.target ? `\n目标：${a.target}` : ''}${a.target_args ? `\n参数：${a.target_args}` : ''}`"
                  @click="toggleSelect(a)"
                >
                  <div class="scan-check" :class="scanRowStatusClass(a)">{{ scanRowStatus(a) }}</div>
                  <div class="scan-icon">
                    <img
                      v-if="scanIconDataUrls[a.path]"
                      :src="scanIconDataUrls[a.path]"
                      :alt="a.name"
                      class="scan-icon-img"
                      @error="onScanIconError(a.path)"
                    />
                    <span v-else class="scan-icon-text" :class="`scan-icon-${a.app_type || 'app'}`">
                      {{ scanTypeIcon(a) }}
                    </span>
                  </div>
                  <div class="scan-info">
                    <div class="scan-name">{{ a.name }}</div>
                    <div class="scan-source">{{ shortSource(a.source) }}</div>
                    <button
                      v-if="a.in_trash"
                      class="scan-restore tap"
                      data-interactive
                      :disabled="restoringPath === a.path"
                      @click.stop="restoreTrashedCandidate(a)"
                    >{{ restoringPath === a.path ? "恢复中…" : "↩ 恢复此软件" }}</button>
                  </div>
                </div>
              </div>
              <!-- 网址：独立类型 -->
              <div v-if="urlCandidates.length" class="scan-group-head">
                🔗 网址
                <span class="scan-group-count">{{ urlCandidates.length }}</span>
              </div>
              <div v-if="urlCandidates.length" class="scan-grid" :class="scanViewMode === 'list' ? 'scan-list' : 'scan-cards'">
                <div
                  v-for="a in urlCandidates"
                  :key="a.path"
                  class="scan-cell tap"
                  :class="{ selected: selectedPaths.has(a.path), added: a.already_imported, trash: a.in_trash }"
                  :title="`${a.name}\n${a.target || a.path}`"
                  @click="toggleSelect(a)"
                >
                  <div class="scan-check" :class="scanRowStatusClass(a)">{{ scanRowStatus(a) }}</div>
                  <div class="scan-icon">
                    <img
                      v-if="scanIconDataUrls[a.path]"
                      :src="scanIconDataUrls[a.path]"
                      :alt="a.name"
                      class="scan-icon-img"
                      @error="onScanIconError(a.path)"
                    />
                    <span v-else class="scan-icon-text scan-icon-url">🔗</span>
                  </div>
                  <div class="scan-info">
                    <div class="scan-name">{{ a.name }}</div>
                    <div class="scan-source">{{ shortSource(a.source) }}</div>
                    <button
                      v-if="a.in_trash"
                      class="scan-restore tap"
                      data-interactive
                      :disabled="restoringPath === a.path"
                      @click.stop="restoreTrashedCandidate(a)"
                    >{{ restoringPath === a.path ? "恢复中…" : "↩ 恢复此软件" }}</button>
                  </div>
                </div>
              </div>
              <!-- Store 应用：UWP/MSIX（Batch 2.4 受控接入；图标用通用占位） -->
              <div v-if="storeApps.length" class="scan-group-head">
                🛍 Store 应用
                <span class="scan-group-count">{{ storeApps.length }}</span>
                <span class="scan-group-tag">系统激活接口启动</span>
              </div>
              <div v-if="storeApps.length" class="scan-grid" :class="scanViewMode === 'list' ? 'scan-list' : 'scan-cards'">
                <div
                  v-for="a in storeApps"
                  :key="a.path"
                  class="scan-cell tap"
                  :class="{ selected: selectedPaths.has(a.path), added: a.already_imported, trash: a.in_trash }"
                  :title="`${a.name}
AppID：${a.target || a.path}`"
                  @click="toggleSelect(a)"
                >
                  <div class="scan-check" :class="scanRowStatusClass(a)">{{ scanRowStatus(a) }}</div>
                  <div class="scan-icon">
                    <span class="scan-icon-text scan-icon-app">🛍</span>
                  </div>
                  <div class="scan-info">
                    <div class="scan-name">{{ a.name }}</div>
                    <div class="scan-source">{{ shortSource(a.source) }}</div>
                    <button
                      v-if="a.in_trash"
                      class="scan-restore tap"
                      data-interactive
                      :disabled="restoringPath === a.path"
                      @click.stop="restoreTrashedCandidate(a)"
                    >{{ restoringPath === a.path ? "恢复中…" : "↩ 恢复此软件" }}</button>
                  </div>
                </div>
              </div>
              <!-- 需要确认：解析失败/失效目标/.ps1 等（可见可选，不静默丢弃） -->
              <div v-if="needsConfirmApps.length" class="scan-group-head warn">
                ❓ 需要确认
                <span class="scan-group-count">{{ needsConfirmApps.length }}</span>
                <span class="scan-group-tag">无法自动确认，请自行判断</span>
              </div>
              <div v-if="needsConfirmApps.length" class="scan-grid" :class="scanViewMode === 'list' ? 'scan-list' : 'scan-cards'">
                <div
                  v-for="a in needsConfirmApps"
                  :key="a.path"
                  class="scan-cell tap"
                  :class="{ selected: selectedPaths.has(a.path), added: a.already_imported, trash: a.in_trash }"
                  :title="`${a.name}\n入口：${a.path}\n状态：无法自动识别目标（可能已失效或为特殊入口）`"
                  @click="toggleSelect(a)"
                >
                  <div class="scan-check" :class="scanRowStatusClass(a)">{{ scanRowStatus(a) }}</div>
                  <div class="scan-icon">
                    <img
                      v-if="scanIconDataUrls[a.path]"
                      :src="scanIconDataUrls[a.path]"
                      :alt="a.name"
                      class="scan-icon-img"
                      @error="onScanIconError(a.path)"
                    />
                    <span v-else class="scan-icon-text scan-icon-app">❓</span>
                  </div>
                  <div class="scan-info">
                    <div class="scan-name">{{ a.name }}</div>
                    <div class="scan-source">{{ shortSource(a.source) }}</div>
                    <button
                      v-if="a.in_trash"
                      class="scan-restore tap"
                      data-interactive
                      :disabled="restoringPath === a.path"
                      @click.stop="restoreTrashedCandidate(a)"
                    >{{ restoringPath === a.path ? "恢复中…" : "↩ 恢复此软件" }}</button>
                  </div>
                </div>
              </div>
              <div v-if="!scanAnyVisible" class="scan-empty">
                {{ scanSearch ? `没有匹配 “${scanSearch}” 的入口` : "没有发现可用入口" }}
              </div>
            </template>
          </div>
          <div class="modal-footer scan-footer">
            <span class="scan-count">已选 <b>{{ selectedPaths.size }}</b> 个</span>
            <div class="scan-footer-actions">
              <button class="s-btn-secondary tap" :disabled="scanning || selectedPaths.size === 0" @click="clearScanSelection">
                清除
              </button>
              <button class="s-btn-secondary tap" @click="closeScan">取消</button>
              <button
                class="s-btn-primary tap"
                :disabled="scanning || selectedPaths.size === 0"
                @click="confirmScan"
              >
                导入已选
              </button>
            </div>
          </div>
        </div>
      </div>
    </Transition>
  </div>
</template>

<style scoped>
.view { padding-bottom: 20px; }

/* ============ P0-#Y-Collapsible：左侧可收缩 sidebar ============ */
/* P0-#APPC#PER#OVERLAY：外层只占 42px 轨道；展开是 inner 浮层，网格零重排 */
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
  width: 42px; /* 固定轨道：只显示 emoji 列 */
  flex-shrink: 0;
  position: relative;
  align-self: stretch;
}
.sidebar-inner {
  position: absolute;
  left: 0;
  top: 0;
  bottom: 0;
  width: 42px;
  display: flex;
  flex-direction: column;
  gap: 6px;
  padding: 6px 4px;
  background: var(--bg-primary);
  border: 1px solid var(--border);
  border-radius: 12px;
  transition: width 0.22s var(--ease-smooth), box-shadow 0.22s var(--ease-smooth), background-color 0.22s var(--ease-smooth);
  overflow: hidden;
  z-index: 30; /* 浮在网格上方；展开不挤压 apps-main（零重排） */
}
.sidebar-inner:hover,
.sidebar-inner.expanded {
  width: 178px; /* 展开态：emoji + label */
  box-shadow: 0 8px 24px rgba(0, 0, 0, 0.4);
  background: var(--bg-secondary);
}
/* 收起态：右侧半透明提示条，提示"hover 可展开"（挂在轨道上，不随浮层展开移动） */
.filter-sidebar::after {
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
  z-index: 31;
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
.sidebar-inner:hover .type-chip-label,
.sidebar-inner.expanded .type-chip-label {
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

/* ============ 软件网格（图标式紧凑网格，容器安全列宽） ============ */
.app-grid {
  display: grid;
  grid-template-columns: repeat(4, minmax(0, 1fr));
  gap: 6px;
  padding: 4px 0;
}
@media (min-width: 420px) {
  .app-grid { grid-template-columns: repeat(5, minmax(0, 1fr)); }
}
@media (min-width: 640px) {
  .app-grid { grid-template-columns: repeat(6, minmax(0, 1fr)); gap: 8px; }
}
@media (min-width: 900px) {
  .app-grid { grid-template-columns: repeat(7, minmax(0, 1fr)); gap: 10px; }
}
@media (min-width: 1200px) {
  .app-grid { grid-template-columns: repeat(8, minmax(0, 1fr)); gap: 12px; }
}
.app-cell {
  position: relative;
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 5px;
  padding: 9px 4px 8px;
  text-align: center;
  background: var(--bg-secondary);
  border: 1px solid var(--border);
  border-radius: 12px;
  cursor: pointer;
  /* P0-#APPC#PER：旧 transition:all 会连带过渡布局属性（width 等）——
     侧栏展开/窗口缩放触发网格重排时全部卡片同步跑布局过渡动画，
     大列表下造成持续 style/layout 开销（v0.4.3 滚动/点击卡顿诱因之一）。
     收窄为合成器友好属性。 */
  transition:
    transform 0.16s var(--ease-smooth),
    box-shadow 0.16s var(--ease-smooth),
    background-color 0.16s var(--ease-smooth),
    border-color 0.16s var(--ease-smooth);
  user-select: none;
  -webkit-app-region: no-drag;
  overflow: hidden;
  min-width: 0;
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
@media (max-width: 419px) {
  .app-cell { padding: 8px 3px 7px; gap: 4px; }
  .app-icon { width: 34px; height: 34px; border-radius: 8px; }
  .app-name { font-size: 10.5px; }
}

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
  font-size: 11.5px;
  font-weight: 500;
  color: var(--text-primary);
  text-align: center;
  width: 100%;
  white-space: normal;
  display: -webkit-box;
  -webkit-line-clamp: 2;
  -webkit-box-orient: vertical;
  overflow: hidden;
  word-break: break-word;
  line-height: 1.3;
}
.app-icon-warn {
  position: absolute;
  top: -4px;
  right: -4px;
  width: 15px;
  height: 15px;
  border-radius: 50%;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  font-size: 10px;
  font-weight: 700;
  color: #1b1205;
  background: #fbbf24;
  box-shadow: 0 1px 4px rgba(0, 0, 0, 0.45);
  pointer-events: none;
}
.app-chip {
  font-size: 10px;
  line-height: 1.6;
  padding: 0 6px;
  border-radius: 999px;
  white-space: nowrap;
}
.app-more {
  position: absolute;
  top: 4px;
  right: 4px;
  width: 22px;
  height: 22px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border: none;
  border-radius: 6px;
  background: transparent;
  color: var(--text-muted);
  font-size: 13px;
  line-height: 1;
  cursor: pointer;
  opacity: 0;
  transition: all 0.15s;
}
.app-cell:hover .app-more,
.app-cell:focus-within .app-more { opacity: 0.9; }
.app-more:hover { color: var(--text-primary); background: var(--bg-tertiary); }
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

/* Batch 2.2 验收修正：扫描结果默认紧凑单列行布局（480px 小窗主场景）。
   行 = 勾选 | 36px 图标 | 名称+来源 | （整行可点）。宽屏仍可用网格但不拥挤。 */
.scan-grid.scan-list {
  display: flex;
  flex-direction: column;
  gap: 4px;
}
.scan-grid.scan-list .scan-cell {
  display: flex;
  flex-direction: row;
  align-items: center;
  gap: 10px;
  padding: 6px 10px;
  border: 1px solid #232b42;
  border-radius: 8px;
  background: #131a2a;
  overflow: visible;
  transform: none;
}
.scan-grid.scan-list .scan-cell.selected {
  border-color: #3b6ef5;
  background: #1d2a4a;
}
.scan-grid.scan-list .scan-cell.added { opacity: 0.55; cursor: default; }
/* P1-B：回收站命中行——弱化但仍可交互，恢复按钮独立可见 */
.scan-grid.scan-list .scan-cell.trash { opacity: 0.85; }
.scan-restore {
  margin-top: 4px;
  align-self: flex-start;
  padding: 2px 9px;
  font-size: 11px;
  line-height: 1.5;
  border-radius: 999px;
  border: 1px solid rgba(120, 180, 255, 0.45);
  background: rgba(80, 140, 255, 0.14);
  color: #a8cfff;
  cursor: pointer;
}
.scan-restore:hover { background: rgba(80, 140, 255, 0.26); }
.scan-restore:disabled { opacity: 0.6; cursor: default; }
.scan-grid.scan-list .scan-check {
  position: static;
  width: 44px;
  flex-shrink: 0;
  text-align: center;
  font-size: 11px;
  font-weight: 600;
  color: #6fd0a0;
  /* 旧多列网格的勾选圆点默认 opacity:0（只在选中时出现）；
     行列表里这一列是常驻状态列（✓ / 在库 / 待恢复），必须可见 */
  background: none;
  border-radius: 0;
  opacity: 1;
  transform: none;
}
.scan-grid.scan-list .scan-cell.added .scan-check { color: #6fd0a0; }
.scan-grid.scan-list .scan-cell.trash .scan-check { color: #f0b45c; }
.scan-check.st-ok { color: #6fd0a0 !important; }
.scan-check.st-trash { color: #f0b45c !important; }
.scan-check.st-confirm { color: #7ea6ff !important; }
.scan-check.st-sel { color: var(--accent-bright, #8b9dff) !important; }
.scan-grid.scan-list .scan-icon {
  position: static;
  width: 36px;
  height: 36px;
  flex-shrink: 0;
}
.scan-grid.scan-list .scan-icon-img { width: 32px; height: 32px; }
.scan-grid.scan-list .scan-info {
  flex: 1;
  min-width: 0;
}
.scan-grid.scan-list .scan-name {
  font-size: 13px;
  color: #e8ecf3;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  text-align: left;
}
.scan-grid.scan-list .scan-source {
  font-size: 10px;
  color: #7c8aa8;
  margin-top: 1px;
  display: inline-block;
  align-self: flex-start;
}
/* 宽窗口（≥640px）：行卡改两列网格，仍为横向行布局 */
@media (min-width: 640px) {
  .scan-grid.scan-list {
    display: grid;
    grid-template-columns: repeat(2, 1fr);
  }
}

/* 默认紧凑图标网格（列表模式保留，用于查看来源/路径/参数） */
.scan-grid.scan-cards {
  display: grid;
  grid-template-columns: repeat(3, minmax(0, 1fr));
  gap: 6px;
}
@media (min-width: 360px) {
  .scan-grid.scan-cards { grid-template-columns: repeat(4, minmax(0, 1fr)); }
}
@media (min-width: 560px) {
  .scan-grid.scan-cards { grid-template-columns: repeat(5, minmax(0, 1fr)); }
}
.scan-grid.scan-cards .scan-cell {
  flex-direction: column;
  align-items: center;
  text-align: center;
  gap: 4px;
  padding: 8px 4px 7px;
  min-width: 0;
}
.scan-grid.scan-cards .scan-cell.selected {
  border-color: var(--accent);
  background: rgba(99, 102, 241, 0.16);
}
.scan-grid.scan-cards .scan-check {
  position: absolute;
  top: 4px;
  left: 4px;
  width: auto;
  height: auto;
  background: none;
  border-radius: 0;
  opacity: 1;
  transform: none;
  font-size: 9.5px;
  font-weight: 600;
  color: #6fd0a0;
  line-height: 1.5;
  pointer-events: none;
}
.scan-grid.scan-cards .scan-check:not(:empty) {
  padding: 0 5px;
  border-radius: 999px;
  background: rgba(0, 0, 0, 0.38);
}
.scan-grid.scan-cards .scan-cell.trash .scan-check { color: #f0b45c; }
.scan-grid.scan-cards .scan-icon { width: 36px; height: 36px; }
.scan-grid.scan-cards .scan-icon-img { width: 32px; height: 32px; }
.scan-grid.scan-cards .scan-info { width: 100%; min-width: 0; }
.scan-grid.scan-cards .scan-name {
  white-space: normal;
  display: -webkit-box;
  -webkit-line-clamp: 2;
  -webkit-box-orient: vertical;
  overflow: hidden;
  font-size: 11px;
  line-height: 1.28;
  text-align: center;
  word-break: break-word;
}
/* 网格模式不显示来源（列表模式保留，用于查看来源/路径/参数） */
.scan-grid.scan-cards .scan-source { display: none; }
.scan-grid.scan-cards .scan-restore {
  align-self: center;
  margin-top: 2px;
  padding: 1px 7px;
  font-size: 10px;
}
@media (max-width: 359px) {
  .scan-grid.scan-cards { gap: 5px; }
  .scan-grid.scan-cards .scan-cell { padding: 7px 3px 6px; }
  .scan-grid.scan-cards .scan-name { font-size: 10.5px; }
  .scan-grid.scan-cards .scan-icon { width: 32px; height: 32px; }
  .scan-grid.scan-cards .scan-icon-img { width: 28px; height: 28px; }
}
@media (max-width: 419px) {
  .scan-grid.scan-cards { gap: 6px; }
  .scan-grid.scan-cards .scan-cell { padding: 10px 6px 8px; }
}

/* U2：搜索行视图切换 */
.scan-view-toggle {
  display: flex;
  flex-shrink: 0;
  gap: 2px;
  padding: 3px;
  border: 1px solid var(--border);
  border-radius: 8px;
  background: var(--bg-secondary);
}
.scan-view-btn {
  width: 28px;
  height: 24px;
  border: none;
  border-radius: 6px;
  background: transparent;
  color: var(--text-muted);
  font-size: 13px;
  cursor: pointer;
  transition: all 0.15s;
}
.scan-view-btn.active {
  background: rgba(99, 102, 241, 0.22);
  color: var(--text-primary);
}

/* U2：底部操作栏（一个主操作 + 两个次级，窄屏可换行） */
.scan-footer {
  justify-content: space-between;
  gap: 12px;
  padding: 10px 16px;
  background: var(--bg-tertiary);
  border-top: 1px solid var(--border);
  border-radius: 0 0 14px 14px;
}
.scan-count {
  font-size: 12px;
  color: var(--text-muted);
  margin-right: auto;
  white-space: nowrap;
}
.scan-count b { color: var(--text-primary); font-weight: 600; }
.scan-footer-actions { display: flex; align-items: center; gap: 8px; flex-shrink: 0; }
.scan-footer-actions .s-btn-secondary,
.scan-footer-actions .s-btn-primary {
  padding: 8px 16px;
  border-radius: 9px;
  font-size: 12.5px;
  min-height: 34px;
  line-height: 1.2;
}
@media (max-width: 359px) {
  .scan-footer { flex-wrap: wrap; row-gap: 8px; }
  .scan-footer-actions { width: 100%; }
  .scan-footer-actions .s-btn-secondary,
  .scan-footer-actions .s-btn-primary { flex: 1; padding-left: 8px; padding-right: 8px; }
}

/* 旧多列网格（保留给可能的宽屏场景；默认单列行布局覆盖） */
.scan-grid {
  display: grid;
  grid-template-columns: repeat(6, 1fr);
  gap: 8px;
}

/* ===== Batch 2.3：高级控制面板 / 进度取消 ===== */
.scan-adv-controls {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 8px 14px 0;
  flex-wrap: wrap;
}
.scan-adv-btn {
  border: 1px solid #3a4568;
  background: #232b42;
  color: #e8ecf3;
  border-radius: 8px;
  padding: 7px 12px;
  font-size: 12px;
  cursor: pointer;
  white-space: nowrap;
}
.scan-adv-btn:hover { border-color: #3b6ef5; }
.scan-adv-path-toggle {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 11px;
  color: #9aa5bd;
  cursor: pointer;
  white-space: nowrap;
}
.scan-cancel-btn {
  margin-top: 10px;
  border: 1px solid #3a4568;
  background: #232b42;
  color: #e8ecf3;
  border-radius: 8px;
  padding: 6px 16px;
  font-size: 12px;
  cursor: pointer;
}
.scan-cancel-btn:hover { border-color: #ff8fa3; }

/* ===== Batch 2.2：模式切换 + 分组头 + 条目状态徽标 ===== */
.scan-mode-tabs {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 8px 14px 0;
}
.scan-mode-tab {
  border: 1px solid #2c3550;
  background: #161c2c;
  color: #9aa5bd;
  border-radius: 8px;
  padding: 7px 14px;
  font-size: 12px;
  cursor: pointer;
  white-space: nowrap;
}
.scan-mode-tab.active {
  border-color: #3b6ef5;
  background: #1d2a4a;
  color: #e8ecf3;
  font-weight: 600;
}
.scan-mode-tab:disabled { opacity: 0.6; cursor: default; }
.scan-mode-hint {
  font-size: 11px;
  color: #7c8aa8;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.scan-group-head {
  margin: 12px 0 8px;
  font-size: 13px;
  font-weight: 600;
  color: #e8ecf3;
  display: flex;
  align-items: center;
  gap: 8px;
}
.scan-group-head:first-child { margin-top: 0; }
.scan-group-head.warn { color: #e8c268; }
.scan-group-count {
  font-size: 11px;
  color: #7c8aa8;
  background: #0f1422;
  border-radius: 6px;
  padding: 1px 7px;
}
.scan-group-tag {
  font-size: 10px;
  color: #7c8aa8;
  font-weight: 400;
}
/* 组头「全选推荐项」按钮（Batch 2.2 验收修正：补齐深色样式，
   之前缺 CSS 导致浏览器默认白底空白块） */
.scan-group-select {
  margin-left: auto;
  border: 1px solid #3a4568;
  background: #232b42;
  color: #c6cede;
  border-radius: 6px;
  padding: 2px 10px;
  font-size: 10px;
  font-weight: 400;
  cursor: pointer;
  white-space: nowrap;
}
.scan-group-select:hover:not(:disabled) {
  border-color: #3b6ef5;
  color: #e8ecf3;
}
.scan-group-select:disabled { opacity: 0.5; cursor: default; }
.scan-cell.added { opacity: 0.55; cursor: default; }
.scan-cell.added .scan-check {
  font-size: 9px;
  color: #6fd0a0;
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
  font-size: 10.5px;
  font-weight: 500;
  color: var(--text-muted);
  line-height: 1.2;
  text-align: left;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  margin-top: 0;
  letter-spacing: 0.2px;
  opacity: 0.85;
  max-width: 100%;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

/* modal-footer */

/* modal-footer */
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
  max-height: calc(100vh - 16px);
  overflow-y: auto;
  overscroll-behavior: contain;
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
/* ===== Phase 2D：失效应用状态 ===== */
.app-cell-missing .app-icon-img,
.app-cell-missing .app-icon-text {
  filter: grayscale(1);
  opacity: 0.45;
}
.app-cell-missing .app-name {
  opacity: 0.6;
}
.app-badge-missing {
  background: rgba(239, 68, 68, 0.16);
  color: #f87171;
  font-size: 10px;
  padding: 1px 6px;
  border-radius: 999px;
}

/* ===== v0.4.1 失效治理面板 ===== */
.invalid-panel {
  margin: 0 0 10px;
  padding: 12px;
  background: var(--bg-primary);
  border: 1px solid rgba(245, 158, 11, 0.45);
  border-radius: 12px;
}
.invalid-panel-head {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 10px;
  flex-wrap: wrap;
  margin-bottom: 10px;
}
.invalid-head-text { display: flex; flex-direction: column; gap: 3px; min-width: 0; flex: 1; }
.invalid-panel-head b { font-size: 13.5px; }
.invalid-sub { font-size: 11.5px; color: var(--text-muted); line-height: 1.5; }
.invalid-head-actions { display: flex; gap: 6px; flex-shrink: 0; }
.invalid-empty { font-size: 12.5px; color: var(--text-muted); padding: 12px 4px; }
.invalid-toolbar { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; margin: 8px 0; }
.invalid-hint { font-size: 11px; color: var(--text-muted); }
.inv-legend { display: flex; gap: 6px; flex-wrap: wrap; margin: 8px 0 4px; }
.inv-tag {
  font-size: 10.5px; font-weight: 600;
  padding: 2px 8px; border-radius: 999px;
  border: 1px solid var(--border);
  color: var(--text-muted);
  background: var(--bg-secondary);
}
.inv-tag.missing { color: #fb7185; border-color: rgba(244, 63, 94, 0.4); background: rgba(244, 63, 94, 0.1); }
.inv-tag.protected { color: #fbbf24; border-color: rgba(245, 158, 11, 0.35); background: rgba(245, 158, 11, 0.08); }
.invalid-list { display: flex; flex-direction: column; gap: 8px; margin-top: 6px; }
.inv-row {
  display: flex; align-items: flex-start; gap: 10px;
  padding: 10px;
  background: var(--bg-secondary);
  border: 1px solid var(--border);
  border-radius: 10px;
}
.inv-row.protected { border-color: rgba(245, 158, 11, 0.25); }
.invalid-check { padding-top: 2px; }
.invalid-check input { width: 16px; height: 16px; accent-color: #f43f5e; cursor: pointer; }
.invalid-shield { font-size: 14px; width: 16px; text-align: center; opacity: 0.8; padding-top: 1px; }
.inv-main { flex: 1; min-width: 0; }
.inv-name { font-size: 13px; font-weight: 600; color: var(--text-primary); }
.inv-issue { font-size: 12px; color: var(--text-secondary); margin-top: 2px; }
.inv-advice { font-size: 11.5px; color: var(--text-muted); margin-top: 3px; line-height: 1.5; }
.inv-reason-btn {
  margin-top: 4px;
  padding: 0;
  border: none;
  background: transparent;
  color: #7ea6ff;
  font-size: 11.5px;
  cursor: pointer;
}
.inv-reason-btn:hover { text-decoration: underline; }
.inv-details {
  margin-top: 6px;
  padding: 8px 10px;
  border-radius: 8px;
  background: var(--bg-primary);
  border: 1px solid var(--border);
  font-size: 11px;
  color: var(--text-muted);
  line-height: 1.7;
  word-break: break-all;
}
.inv-details code { font-family: Consolas, monospace; color: var(--text-secondary); }
.inv-actions { display: flex; flex-direction: column; gap: 6px; flex-shrink: 0; }
.inv-foot {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
  margin-top: 10px;
  padding-top: 10px;
  border-top: 1px solid var(--border);
}
@media (max-width: 419px) {
  .inv-row { flex-wrap: wrap; }
  .inv-actions { flex-direction: row; width: 100%; }
  .inv-actions .s-btn-mini { flex: 1; }
}
</style>

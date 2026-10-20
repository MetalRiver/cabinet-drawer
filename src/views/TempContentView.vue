<script setup lang="ts">
import { ref, onMounted, onUnmounted, onActivated, onDeactivated, computed, nextTick } from "vue";
import { useAppStore } from "../stores/app";
import { useWidgetStore } from "../stores/widget";
import { useTempStore } from "../stores/temp";
import { useSnippetsStore } from "../stores/snippets";
import { TEMP_NEVER_EXPIRES_MS, isTempPermanent, type TempMeta } from "../api";
import DarkSelect from "../components/DarkSelect.vue";
import ConfirmInline from "../components/ConfirmInline.vue";

const appStore = useAppStore();
const widgetStore = useWidgetStore();
const tempStore = useTempStore();
const snippetsStore = useSnippetsStore();

// 新增
const showAdd = ref(false);
// v0.4.1：设置"永不过期"(0) 时默认 TTL = 0（创建即永久），不再被 Math.max(1,·) 强改成 1 天
const defaultTtl = computed(() =>
  widgetStore.tempExpireDays === 0 ? 0 : widgetStore.tempExpireDays * 24 * 60
);
const ttlOptions = computed(() => {
  const base = [
    { value: 60, label: "1 小时" },
    { value: 360, label: "6 小时" },
    { value: 1440, label: "1 天" },
    { value: 4320, label: "3 天" },
    { value: 10080, label: "7 天" },
    { value: 43200, label: "30 天" },
    { value: 0, label: "永不过期" },
  ];
  return base.map((o) => ({
    ...o,
    label: o.value === defaultTtl.value && o.value !== 0 ? `${o.label}（默认）` : o.label,
  }));
});
const addForm = ref({ text: "", ttlMinutes: 0, title: "" });

// ===== U3：详情浮层（阅读 / 编辑 / 追加的统一入口） =====
const detailTarget = ref<TempMeta | null>(null);
const detailEditing = ref(false);
const detailMode = ref<"edit" | "append">("edit");
const detailTitle = ref("");
const detailDraft = ref("");
const detailAppend = ref("");
const detailBusy = ref(false);
const detailDeleteConfirm = ref(false);
const detailDiscardConfirm = ref(false);
const detailEl = ref<HTMLElement | null>(null);

function openDetail(item: TempMeta) {
  closeItemMenu();
  detailTarget.value = item;
  detailEditing.value = false;
  detailDiscardConfirm.value = false;
  detailDeleteConfirm.value = false;
  void nextTick(() => detailEl.value?.focus());
}

function openDetailAppend(item: TempMeta) {
  openDetail(item);
  startDetailEdit("append");
}

function startDetailEdit(mode: "edit" | "append") {
  if (!detailTarget.value) return;
  detailMode.value = mode;
  detailTitle.value = detailTarget.value.title || "";
  detailDraft.value = detailTarget.value.text;
  detailAppend.value = "";
  detailEditing.value = true;
  detailDiscardConfirm.value = false;
}

const detailDirty = computed(() => {
  if (!detailTarget.value || !detailEditing.value) return false;
  if (detailTitle.value.trim() !== (detailTarget.value.title || "").trim()) return true;
  if (detailMode.value === "append") return detailAppend.value.trim().length > 0;
  return detailDraft.value !== detailTarget.value.text;
});

function requestCloseDetail() {
  if (detailEditing.value && detailDirty.value) {
    detailDiscardConfirm.value = true;
    return;
  }
  forceCloseDetail();
}

function forceCloseDetail() {
  detailTarget.value = null;
  detailEditing.value = false;
  detailDiscardConfirm.value = false;
  detailDeleteConfirm.value = false;
  detailTitle.value = "";
  detailDraft.value = "";
  detailAppend.value = "";
}

async function saveDetail() {
  if (!detailTarget.value || detailBusy.value) return;
  const id = detailTarget.value.id;
  const newTitle = detailTitle.value.trim();
  let finalText = detailTarget.value.text;
  if (detailMode.value === "append") {
    if (!detailAppend.value.trim()) {
      appStore.showClipToast("info", "请输入要追加的内容");
      return;
    }
    finalText =
      finalText.endsWith("\n") || finalText === ""
        ? finalText + detailAppend.value
        : finalText + "\n" + detailAppend.value;
  } else {
    if (!detailDraft.value.trim()) {
      appStore.showClipToast("info", "内容不能为空");
      return;
    }
    finalText = detailDraft.value;
  }
  detailBusy.value = true;
  try {
    if (newTitle !== (detailTarget.value.title || "")) {
      await tempStore.updateTitle(id, newTitle || null);
    }
    if (finalText !== detailTarget.value.text) {
      await tempStore.updateText(id, finalText);
    }
    await tempStore.loadItems();
    const fresh = tempStore.items.find((i) => i.id === id) || null;
    detailTarget.value = fresh;
    detailEditing.value = false;
    detailDiscardConfirm.value = false;
    if (fresh) {
      detailEl.value?.focus();
    } else {
      forceCloseDetail();
    }
    appStore.showClipToast("success", detailMode.value === "append" ? "已追加" : "已保存");
  } catch (e) {
    appStore.showClipToast("info", String(e));
  } finally {
    detailBusy.value = false;
  }
}

function catSelectValue(item: TempMeta | null): number {
  return item?.category_id ?? -1;
}
const catOptions = computed(() => [
  { value: -1, label: "未分类" },
  ...tempStore.categories.map((c) => ({ value: c.id, label: c.name })),
]);
async function onDetailCatChange(v: number) {
  if (!detailTarget.value) return;
  const id = detailTarget.value.id;
  try {
    await tempStore.setCategory(id, v === -1 ? null : v);
    detailTarget.value = tempStore.items.find((i) => i.id === id) || null;
  } catch (e) {
    appStore.showClipToast("info", String(e));
  }
}

async function setDetailPermanent() {
  if (!detailTarget.value) return;
  const id = detailTarget.value.id;
  try {
    await tempStore.renew(id, null, TEMP_NEVER_EXPIRES_MS);
    detailTarget.value = tempStore.items.find((i) => i.id === id) || null;
    appStore.showClipToast("success", "已设为永不过期");
  } catch (e) {
    appStore.showClipToast("info", String(e));
  }
}

async function confirmDetailDelete() {
  if (!detailTarget.value) return;
  const id = detailTarget.value.id;
  try {
    await tempStore.remove(id);
    forceCloseDetail();
    appStore.showClipToast("success", "已删除，可在回收站恢复");
  } catch (e) {
    appStore.showClipToast("info", String(e));
  }
}

// ===== U3：右键 / ⋯ 共用同一套操作菜单 =====
const itemMenu = ref<{
  visible: boolean;
  x: number;
  y: number;
  item: TempMeta | null;
  showCategories: boolean;
}>({ visible: false, x: 0, y: 0, item: null, showCategories: false });
const itemMenuEl = ref<HTMLElement | null>(null);

function clampMenu(x: number, y: number, w: number, h: number) {
  const pad = 8;
  return {
    x: Math.max(pad, Math.min(x, Math.max(pad, window.innerWidth - w - pad))),
    y: Math.max(pad, Math.min(y, Math.max(pad, window.innerHeight - h - pad))),
  };
}

function openItemMenu(e: MouseEvent, item: TempMeta) {
  e.preventDefault();
  e.stopPropagation();
  const pos = clampMenu(e.clientX, e.clientY, 200, 260);
  itemMenu.value = { visible: true, x: pos.x, y: pos.y, item, showCategories: false };
  void nextTick(() => {
    if (!itemMenuEl.value) return;
    const r = itemMenuEl.value.getBoundingClientRect();
    if (r.width === 0 || r.height === 0) return;
    const fixed = clampMenu(pos.x, pos.y, r.width, r.height);
    if (fixed.x !== pos.x || fixed.y !== pos.y) {
      itemMenu.value = { ...itemMenu.value, x: fixed.x, y: fixed.y };
    }
  });
}

function closeItemMenu() {
  if (!itemMenu.value.visible) return;
  itemMenu.value = { ...itemMenu.value, visible: false, showCategories: false };
}

async function menuPickCategory(categoryId: number | null) {
  const target = itemMenu.value.item;
  closeItemMenu();
  if (!target) return;
  try {
    await tempStore.setCategory(target.id, categoryId);
    if (detailTarget.value?.id === target.id) {
      detailTarget.value = tempStore.items.find((i) => i.id === target.id) || null;
    }
    appStore.showClipToast("success", "分类已更新");
  } catch (e) {
    appStore.showClipToast("info", String(e));
  }
}

function menuAction(action: "open" | "append" | "copy" | "renew" | "promote" | "delete") {
  const target = itemMenu.value.item;
  closeItemMenu();
  if (!target) return;
  if (action === "open") openDetail(target);
  else if (action === "append") openDetailAppend(target);
  else if (action === "copy") void copy(target.text);
  else if (action === "renew") openRenew(target);
  else if (action === "promote") openPromote(target);
  else if (action === "delete") {
    openDetail(target);
    detailDeleteConfirm.value = true;
  }
}

// 点击外部关闭菜单；Esc 关闭菜单或详情
function onDocPointerDown(e: PointerEvent) {
  if (!itemMenu.value.visible) return;
  const el = itemMenuEl.value;
  if (el && e.target instanceof Node && el.contains(e.target)) return;
  closeItemMenu();
}
function onWindowScroll() {
  if (itemMenu.value.visible) closeItemMenu();
}
function onGlobalKeydown(e: KeyboardEvent) {
  if (e.key !== "Escape") return;
  // 关闭顺序 = 叠放顺序：菜单 > 上层弹窗 > 详情浮层
  if (itemMenu.value.visible) {
    closeItemMenu();
    return;
  }
  if (showRenew.value) {
    showRenew.value = false;
    return;
  }
  if (showPromote.value) {
    showPromote.value = false;
    return;
  }
  if (showAdd.value) {
    showAdd.value = false;
    return;
  }
  if (showCatManage.value) {
    showCatManage.value = false;
    return;
  }
  if (detailTarget.value) {
    e.stopPropagation();
    requestCloseDetail();
  }
}

// ===== 续期（保持 v0.4.1 逻辑） =====
const RENEW_PRESETS = [
  { value: 1 * 24 * 60 * 60 * 1000, label: "延长 1 天" },
  { value: 3 * 24 * 60 * 60 * 1000, label: "延长 3 天" },
  { value: 7 * 24 * 60 * 60 * 1000, label: "延长 7 天" },
  { value: 30 * 24 * 60 * 60 * 1000, label: "延长 30 天" },
  { value: -1, label: "永不过期" }, // -1 = 哨兵标记，提交时换算 TEMP_NEVER_EXPIRES_MS
  { value: -2, label: "自定义到期时间" }, // -2 = 哨兵标记
] as const;
// 自定义到期时间上限：2099-12-31T23:59（不允许接近/越过永久哨兵，防显示歧义）
const RENEW_DT_MAX = "2099-12-31T23:59";
// 永久便签的续期选项：延长无意义（后端明确拒绝），只留"自定义到期时间"作为改回限时的唯一路径
const renewOptions = computed(() => {
  const target = renewTarget.value;
  if (!target || !isTempPermanent(target.expires_at)) return RENEW_PRESETS.map((p) => ({ value: p.value as number, label: p.label }));
  return [{ value: -2 as number, label: "自定义到期时间（改回限时）" }];
});
const renewTargetIsPermanent = computed(
  () => !!renewTarget.value && isTempPermanent(renewTarget.value.expires_at)
);
const showRenew = ref(false);
const renewTarget = ref<TempMeta | null>(null);
const renewChoice = ref<number>(-3); // -3 = 未选择
const renewCustom = ref("");
const renewBusy = ref(false);
const renewError = ref("");
function openRenew(item: TempMeta) {
  renewTarget.value = item;
  renewChoice.value = -3;
  renewCustom.value = "";
  renewError.value = "";
  showRenew.value = true;
}
const renewCustomMs = computed(() => {
  if (renewChoice.value !== -2) return null;
  if (!renewCustom.value) return null;
  const t = new Date(renewCustom.value).getTime();
  // 合法区间：(now, 永久哨兵) 开区间；越过上限的日期会被后端拒绝且会造成"显示永久实际会过期"歧义
  return Number.isFinite(t) && t > Date.now() && t < TEMP_NEVER_EXPIRES_MS ? t : null;
});
const renewPreview = computed(() => {
  if (!renewTarget.value) return "";
  const cur = renewTarget.value.expires_at;
  if (renewChoice.value === -1) return "永不过期";
  if (renewChoice.value === -2) {
    if (renewCustomMs.value === null) {
      if (!renewCustom.value) return "";
      const t = new Date(renewCustom.value).getTime();
      if (Number.isFinite(t) && t >= TEMP_NEVER_EXPIRES_MS) return "超出支持范围（最晚 2099-12-31）";
      return "无效时间（必须晚于现在且早于 2099-12-31）";
    }
    return fmtDateTime(renewCustomMs.value);
  }
  if (renewChoice.value > 0) return fmtDateTime(cur + renewChoice.value);
  return "";
});
const renewReady = computed(
  () =>
    renewChoice.value === -1 ||
    (renewChoice.value === -2 && renewCustomMs.value !== null) ||
    renewChoice.value > 0
);
function fmtDateTime(ms: number): string {
  return new Date(ms).toLocaleString("zh-CN", { hour12: false });
}
async function submitRenew() {
  if (!renewTarget.value || !renewReady.value || renewBusy.value) return;
  renewBusy.value = true;
  renewError.value = "";
  try {
    let setExpiresAt: number | null = null;
    let addMs: number | null = null;
    if (renewChoice.value === -1) setExpiresAt = TEMP_NEVER_EXPIRES_MS;
    else if (renewChoice.value === -2) setExpiresAt = renewCustomMs.value;
    else addMs = renewChoice.value;
    // 永久便签只能走 setExpiresAt（-1 选项不在其选项列表内，防御再拦一层）
    if (renewTargetIsPermanent.value && addMs !== null) {
      renewError.value = "该便签已永不过期：如需改为限时，请选择自定义到期时间";
      renewBusy.value = false;
      return;
    }
    const id = renewTarget.value.id;
    const newExpires = await tempStore.renew(id, addMs, setExpiresAt);
    if (detailTarget.value?.id === id) {
      detailTarget.value = tempStore.items.find((i) => i.id === id) || null;
    }
    showRenew.value = false;
    appStore.showClipToast(
      "success",
      newExpires >= TEMP_NEVER_EXPIRES_MS ? "已设为永不过期" : `已续期，新到期时间：${fmtDateTime(newExpires)}`
    );
  } catch (e) {
    renewError.value = String(e);
  } finally {
    renewBusy.value = false;
  }
}

// 倒计时实时刷新（KeepAlive 缓存下切走 tab 走 deactivated 路径，
// 必须用 onActivated/onDeactivated 管理计时器，否则离开便签页后每秒空转）
const now = ref(Date.now());
let timer: number | null = null;
const startTimer = () => {
  if (timer === null) {
    timer = window.setInterval(() => {
      now.value = Date.now();
    }, 1000);
  }
};
const stopTimer = () => {
  if (timer !== null) {
    clearInterval(timer);
    timer = null;
  }
};
onMounted(() => {
  // 修复 P1：fire-and-forget，先启动倒计时刷新，再后台拉数据
  // 视图能立即渲染（空态），数据到达后自动填充
  void tempStore.loadItems();
  void tempStore.loadCategories(); // Batch 4：分类列表
  startTimer();
  document.addEventListener("pointerdown", onDocPointerDown, true);
  window.addEventListener("scroll", onWindowScroll, true);
  window.addEventListener("keydown", onGlobalKeydown);
});
onUnmounted(() => {
  stopTimer();
  document.removeEventListener("pointerdown", onDocPointerDown, true);
  window.removeEventListener("scroll", onWindowScroll, true);
  window.removeEventListener("keydown", onGlobalKeydown);
});
onActivated(startTimer);
onDeactivated(stopTimer);

// 关键修复：去掉 watch(searchQuery) → loadItems
// 之前每次按键都触发后端 DB 查询，导致列表项被整个替换 → 闪屏
// 现在搜索过滤完全在客户端 computed 完成

// Batch 4：分类筛选 + 标题/正文搜索（store.filteredItems）叠加全局搜索框
const filteredItems = computed(() => {
  let list = tempStore.filteredItems;
  if (appStore.searchQuery) {
    const q = appStore.searchQuery.toLowerCase();
    list = list.filter(
      (i) => i.text.toLowerCase().includes(q) || (i.title || "").toLowerCase().includes(q)
    );
  }
  return list;
});

// ===== 文章/分类 =====
function notePreview(item: TempMeta): string {
  return item.text.length > 90 ? item.text.slice(0, 90) + "…" : item.text;
}

// 分类管理弹窗（复用现有分类 CRUD）
const showCatManage = ref(false);
const catNewName = ref("");
const catRenameId = ref<number | null>(null);
const catRenameName = ref("");
async function submitNewCategory() {
  if (!catNewName.value.trim()) return;
  try {
    await tempStore.createCategory(catNewName.value);
    catNewName.value = "";
    appStore.showClipToast("success", "分类已创建");
  } catch (e) {
    appStore.showClipToast("info", "创建失败：" + String(e));
  }
}
async function submitRenameCategory() {
  if (catRenameId.value == null || !catRenameName.value.trim()) return;
  try {
    await tempStore.renameCategory(catRenameId.value, catRenameName.value);
    catRenameId.value = null;
    appStore.showClipToast("success", "分类已重命名");
  } catch (e) {
    appStore.showClipToast("info", "重命名失败：" + String(e));
  }
}
async function submitDeleteCategory(id: number) {
  try {
    const n = await tempStore.deleteCategory(id);
    if (detailTarget.value?.category_id === id) {
      detailTarget.value = tempStore.items.find((i) => i.id === detailTarget.value?.id) || null;
    }
    appStore.showClipToast("success", `分类已删除，${n} 张便签回到未分类`);
  } catch (e) {
    appStore.showClipToast("info", "删除失败：" + String(e));
  }
}

// 分类筛选：全部 / 未分类 / 分类
function selectAllCategories() {
  tempStore.activeCategoryId = null;
  tempStore.showUncategorized = false;
}
function selectUncategorized() {
  tempStore.activeCategoryId = null;
  tempStore.showUncategorized = true;
}
function selectCategory(id: number) {
  tempStore.showUncategorized = false;
  tempStore.activeCategoryId = id;
}

function timeLeft(expires: number): string {
  if (isTempPermanent(expires)) return "永久保留";
  const diff = expires - now.value;
  if (diff <= 0) return "已过期";
  const sec = Math.floor(diff / 1000);
  const h = Math.floor(sec / 3600);
  const m = Math.floor((sec % 3600) / 60);
  const s = sec % 60;
  if (h >= 24) return `${Math.floor(h / 24)}天${h % 24}小时`;
  if (h > 0) return `${h}h${m}m`;
  if (m > 0) return `${m}m${s}s`;
  return `${s}s`;
}

function expiryLabel(item: TempMeta): string {
  if (isTempPermanent(item.expires_at)) return "永不过期";
  if (item.expires_at <= now.value) return "已过期";
  return `剩余 ${timeLeft(item.expires_at)}`;
}
function expiryTone(item: TempMeta): string {
  if (isTempPermanent(item.expires_at)) return "permanent";
  const diff = item.expires_at - now.value;
  if (diff <= 0) return "expired";
  if (diff <= 24 * 60 * 60 * 1000) return "soon";
  return "";
}

function progress(item: TempMeta): number {
  if (isTempPermanent(item.expires_at)) return 100;
  const total = item.expires_at - item.created_at;
  const left = item.expires_at - now.value;
  if (total <= 0) return 0;
  return Math.max(0, Math.min(100, (left / total) * 100));
}

function openSummary() {
  appStore.showSummaryModal = true;
}

function openAdd() {
  addForm.value = { text: "", ttlMinutes: defaultTtl.value, title: "" };
  showAdd.value = true;
}

async function submitAdd() {
  if (!addForm.value.text) {
    appStore.showClipToast("info", "请输入内容");
    return;
  }
  try {
    await tempStore.create(addForm.value.text, addForm.value.ttlMinutes, addForm.value.title);
    showAdd.value = false;
    appStore.showClipToast("success", "已添加");
  } catch (e) {
    appStore.showClipToast("info", String(e));
  }
}

async function copy(text: string) {
  try {
    await tempStore.copy(text);
    appStore.showClipToast("success", "已复制");
  } catch (e) {
    appStore.showClipToast("info", String(e));
  }
}

// R4：转存为命令改为应用内弹窗（Tauri webview 原生 prompt 不可靠）
const showPromote = ref(false);
const promoteTarget = ref<TempMeta | null>(null);
const promoteTitle = ref("");
function openPromote(item: TempMeta) {
  promoteTarget.value = item;
  promoteTitle.value = item.text.slice(0, 30) + (item.text.length > 30 ? "..." : "");
  showPromote.value = true;
}
async function submitPromote() {
  if (!promoteTarget.value) return;
  if (!promoteTitle.value.trim()) {
    appStore.showClipToast("info", "请输入命令标题");
    return;
  }
  try {
    await snippetsStore.create({ title: promoteTitle.value.trim(), content: promoteTarget.value.text });
    showPromote.value = false;
    appStore.showClipToast("success", "已转存为命令");
  } catch (e) {
    appStore.showClipToast("info", String(e));
  }
}
</script>

<template>
  <div class="view">
    <div class="view-header">
      <div class="view-header-left">
        <div class="view-icon icon-orange">⏳</div>
        <div class="view-header-info">
          <h2 class="view-title">
            <span class="view-title-text">便签</span>
            <span class="title-count">{{ filteredItems.length }}</span>
          </h2>
          <p class="view-subtitle">{{ widgetStore.tempExpireDays === 0 ? "新建便签默认永不过期" : `新建便签默认 ${widgetStore.tempExpireDays} 天后过期` }}</p>
        </div>
      </div>
      <div class="view-header-actions">
        <button class="action-btn action-btn-primary tap" title="汇总便签" @click="openSummary">
          <span class="action-btn-glyph">📋</span>
        </button>
        <button class="action-btn action-btn-primary tap" title="添加便签" @click="openAdd">+</button>
      </div>
    </div>

    <!-- U3：分类筛选常驻（全部 / 未分类 / 自定义分类 + 明确的管理入口） -->
    <div class="cat-bar">
      <div class="cat-bar-scroll">
        <button
          class="cat-chip"
          :class="{ active: !tempStore.showUncategorized && tempStore.activeCategoryId === null }"
          @click="selectAllCategories"
        >全部</button>
        <button
          class="cat-chip"
          :class="{ active: tempStore.showUncategorized }"
          @click="selectUncategorized"
        >未分类</button>
        <button
          v-for="c in tempStore.categories"
          :key="c.id"
          class="cat-chip"
          :class="{ active: !tempStore.showUncategorized && tempStore.activeCategoryId === c.id }"
          @click="selectCategory(c.id)"
        >{{ c.name }}</button>
      </div>
      <button class="cat-manage-btn tap" data-interactive @click="showCatManage = true">
        ⚙️ 管理分类
      </button>
    </div>

    <!-- 空状态 -->
    <div v-if="filteredItems.length === 0 && !tempStore.loading" class="empty">
      <div class="empty-icon">⏳</div>
      <div class="empty-text">
        <template v-if="appStore.searchQuery">没有匹配的便签</template>
        <template v-else-if="tempStore.showUncategorized">这个分类下还没有便签</template>
        <template v-else-if="tempStore.activeCategoryId !== null">该分类下还没有便签</template>
        <template v-else>还没有便签，点 + 添加</template>
      </div>
    </div>

    <TransitionGroup name="list" tag="div" class="list-container">
      <article
        v-for="(item, index) in filteredItems"
        :key="item.id"
        class="note-card stagger-item"
        :style="{ '--delay': `${index * 0.04}s` }"
        :title="'单击查看详情，右键更多操作'"
        @click="openDetail(item)"
        @contextmenu.prevent.stop="openItemMenu($event, item)"
      >
        <div class="note-main">
          <div class="item-icon icon-orange note-icon"><span>⏳</span></div>
          <div class="note-info">
            <div v-if="item.title" class="note-title">{{ item.title }}</div>
            <div
              class="note-text"
              :class="{ 'note-text-preview': !!item.title, 'note-text-empty': !item.title && !item.text }"
            >{{ item.title ? notePreview(item) : item.text }}</div>
            <div class="note-meta">
              <span class="note-cat" :class="{ 'note-cat-empty': !item.category_name }">
                🏷 {{ item.category_name || "未分类" }}
              </span>
              <span class="note-time" :class="`note-time-${expiryTone(item)}`">
                ⏱ {{ expiryLabel(item) }}
              </span>
            </div>
            <div v-if="!isTempPermanent(item.expires_at)" class="note-progress">
              <div class="note-progress-bar" :style="{ width: progress(item) + '%' }"></div>
            </div>
          </div>
        </div>
        <button
          class="note-more tap"
          data-interactive
          title="更多操作"
          aria-label="更多操作"
          @click.stop="openItemMenu($event, item)"
          @contextmenu.stop.prevent="openItemMenu($event, item)"
        >···</button>
      </article>
    </TransitionGroup>

    <!-- ===== U3：详情浮层（阅读 / 编辑 / 追加 / 分类 / 续期 / 复制 / 删除） ===== -->
    <Teleport to="body">
      <Transition name="modal">
        <div v-if="detailTarget" class="dt-mask" @click.self="requestCloseDetail">
          <div class="dt-sheet" role="dialog" aria-modal="true" tabindex="-1" ref="detailEl">
            <div class="dt-head">
              <div class="dt-head-text">
                <span class="dt-head-title">{{ detailEditing ? (detailMode === "append" ? "追加内容" : "编辑便签") : "便签详情" }}</span>
                <span class="dt-head-meta">{{ detailTarget.category_name || "未分类" }} · {{ expiryLabel(detailTarget) }}</span>
              </div>
              <button class="dt-close tap" data-interactive title="关闭（Esc）" @click="requestCloseDetail">×</button>
            </div>

            <div class="dt-body">
              <template v-if="!detailEditing">
                <h3 v-if="detailTarget.title" class="dt-title">{{ detailTarget.title }}</h3>
                <pre class="dt-text">{{ detailTarget.text }}</pre>
                <div class="dt-meta-line">
                  创建于 {{ fmtDateTime(detailTarget.created_at) }} · 到期：{{ isTempPermanent(detailTarget.expires_at) ? "永不过期" : fmtDateTime(detailTarget.expires_at) }}
                </div>
              </template>
              <template v-else>
                <div class="dt-field">
                  <label class="dt-label">标题（选填，≤100 字符）</label>
                  <input
                    v-model="detailTitle"
                    class="form-input"
                    maxlength="100"
                    placeholder="不填则列表显示正文预览"
                  />
                </div>
                <div v-if="detailMode === 'append'" class="dt-field">
                  <label class="dt-label">追加到末尾（原文完整保留）</label>
                  <textarea
                    v-model="detailAppend"
                    class="form-input form-textarea"
                    rows="5"
                    placeholder="输入要追加到末尾的内容…"
                  ></textarea>
                </div>
                <div v-else class="dt-field">
                  <label class="dt-label">正文</label>
                  <textarea v-model="detailDraft" class="form-input form-textarea" rows="8"></textarea>
                </div>
              </template>

              <div class="dt-cat-row">
                <span class="dt-label">所属分类</span>
                <DarkSelect
                  :model-value="catSelectValue(detailTarget)"
                  :options="catOptions"
                  aria-label="所属分类"
                  @update:model-value="onDetailCatChange"
                />
                <button class="dt-link tap" data-interactive @click="showCatManage = true">管理分类</button>
              </div>

              <div v-if="detailDiscardConfirm" class="dt-discard">
                <span>有未保存的修改</span>
                <button class="btn-secondary tap" data-interactive @click="detailDiscardConfirm = false">继续编辑</button>
                <button class="btn-secondary danger-text tap" data-interactive @click="forceCloseDetail">放弃修改</button>
              </div>

              <ConfirmInline
                v-if="detailDeleteConfirm"
                title="确定删除这张便签？删除后可在回收站恢复。"
                confirm-text="删除"
                @cancel="detailDeleteConfirm = false"
                @confirm="confirmDetailDelete"
              />
            </div>

            <div class="dt-foot">
              <template v-if="!detailEditing">
                <button class="btn-primary tap" data-interactive @click="startDetailEdit('edit')">✏️ 编辑</button>
                <button class="btn-secondary tap" data-interactive @click="startDetailEdit('append')">➕ 追加</button>
                <button class="btn-secondary tap" data-interactive @click="copy(detailTarget.text)">📋 复制</button>
                <button class="btn-secondary tap" data-interactive @click="openRenew(detailTarget)">⏰ 续期</button>
                <button
                  v-if="!isTempPermanent(detailTarget.expires_at)"
                  class="btn-secondary tap"
                  data-interactive
                  @click="setDetailPermanent"
                >∞ 永不过期</button>
                <button class="dt-delete tap" data-interactive @click="detailDeleteConfirm = true">删除</button>
              </template>
              <template v-else>
                <button class="btn-secondary tap" data-interactive @click="requestCloseDetail">取消</button>
                <button class="btn-primary tap" data-interactive :disabled="detailBusy" @click="saveDetail">
                  {{ detailBusy ? "保存中…" : "保存" }}
                </button>
              </template>
            </div>
          </div>
        </div>
      </Transition>
    </Teleport>

    <!-- ===== U3：右键 / ⋯ 共用菜单 ===== -->
    <Teleport to="body">
      <Transition name="modal">
        <div
          v-if="itemMenu.visible && itemMenu.item"
          ref="itemMenuEl"
          class="tm-menu"
          :style="{ left: itemMenu.x + 'px', top: itemMenu.y + 'px' }"
          @click.stop
          @contextmenu.prevent.stop
        >
          <template v-if="!itemMenu.showCategories">
            <div class="tm-menu-title">{{ itemMenu.item.title || notePreview(itemMenu.item) }}</div>
            <button class="tm-menu-item tap" data-interactive @click="menuAction('open')">👁 查看 / 编辑</button>
            <button class="tm-menu-item tap" data-interactive @click="menuAction('append')">➕ 追加内容</button>
            <button class="tm-menu-item tap" data-interactive @click="menuAction('copy')">📋 复制</button>
            <button class="tm-menu-item tap" data-interactive @click="itemMenu.showCategories = true">
              🏷 修改分类
              <span class="tm-menu-hint">{{ itemMenu.item.category_name || "未分类" }}</span>
            </button>
            <button class="tm-menu-item tap" data-interactive @click="menuAction('renew')">⏰ 续期</button>
            <button class="tm-menu-item tap" data-interactive @click="menuAction('promote')">📌 转存为命令</button>
            <div class="tm-menu-divider"></div>
            <button class="tm-menu-item danger tap" data-interactive @click="menuAction('delete')">🗑 删除（可恢复）</button>
          </template>
          <template v-else>
            <button class="tm-menu-item tm-menu-back tap" data-interactive @click="itemMenu.showCategories = false">
              ‹ 返回
            </button>
            <button class="tm-menu-item tap" data-interactive @click="menuPickCategory(null)">
              未分类 <span v-if="!itemMenu.item.category_id" class="tm-menu-check">✓</span>
            </button>
            <button
              v-for="c in tempStore.categories"
              :key="c.id"
              class="tm-menu-item tap"
              data-interactive
              @click="menuPickCategory(c.id)"
            >
              {{ c.name }} <span v-if="itemMenu.item.category_id === c.id" class="tm-menu-check">✓</span>
            </button>
            <button class="tm-menu-item tm-menu-manage tap" data-interactive @click="closeItemMenu(); showCatManage = true">
              ⚙️ 管理分类…
            </button>
          </template>
        </div>
      </Transition>
    </Teleport>

    <!-- 分类管理（新建/重命名/删除；删除分类不删除便签） -->
    <Transition name="modal">
      <div v-if="showCatManage" class="modal-mask" @click.self="showCatManage = false">
        <div class="modal">
          <div class="modal-header">
            <span>管理便签分类</span>
            <button class="modal-close tap" @click="showCatManage = false">×</button>
          </div>
          <div class="modal-body">
            <div class="form-row">
              <label>新建分类</label>
              <div class="cat-manage-row">
                <input v-model="catNewName" class="form-input" maxlength="30" placeholder="分类名称（≤30 字符）" @keyup.enter="submitNewCategory" />
                <button class="btn-primary tap" @click="submitNewCategory">新建</button>
              </div>
            </div>
            <div class="cat-manage-list">
              <div v-for="c in tempStore.categories" :key="c.id" class="cat-manage-item">
                <template v-if="catRenameId === c.id">
                  <input v-model="catRenameName" class="form-input" maxlength="30" @keyup.enter="submitRenameCategory" />
                  <button class="btn-primary tap" @click="submitRenameCategory">保存</button>
                  <button class="btn-secondary tap" @click="catRenameId = null">取消</button>
                </template>
                <template v-else>
                  <span class="cat-manage-name">{{ c.name }}</span>
                  <button class="btn-secondary tap" title="重命名" @click="catRenameId = c.id; catRenameName = c.name">重命名</button>
                  <button class="btn-secondary danger-text tap" title="删除（便签回到未分类）" @click="submitDeleteCategory(c.id)">删除</button>
                </template>
              </div>
              <div v-if="!tempStore.categories.length" class="cat-manage-empty">还没有分类，先新建一个</div>
            </div>
            <p class="cat-manage-hint">删除分类不会删除便签——相关便签自动回到「未分类」。</p>
          </div>
        </div>
      </div>
    </Transition>

    <!-- 剪贴板倒计时提示 -->
    <Transition name="toast">
      <div v-if="appStore.clipboardCountdown > 0" class="clipboard-toast">
        <span>📋</span>
        <span>剪贴板将在 {{ appStore.clipboardCountdown }} 秒后清空</span>
      </div>
    </Transition>

    <!-- 添加表单 -->
    <Transition name="modal">
      <div v-if="showAdd" class="modal-mask" @click.self="showAdd = false">
        <div class="modal">
          <div class="modal-header">
            <span>添加便签</span>
            <button class="modal-close tap" @click="showAdd = false">×</button>
          </div>
          <div class="modal-body">
            <div class="form-row">
              <label>标题（选填，≤100 字符）</label>
              <input v-model="addForm.title" class="form-input" maxlength="100" placeholder="不填则显示正文预览" />
            </div>
            <div class="form-row">
              <label>内容</label>
              <textarea v-model="addForm.text" class="form-input form-textarea" rows="6" placeholder="临时保存的内容…"></textarea>
            </div>
            <div class="form-row">
              <label>保留时长</label>
              <DarkSelect v-model="addForm.ttlMinutes" :options="ttlOptions" aria-label="保留时长" block />
            </div>
          </div>
          <div class="modal-footer">
            <button class="btn-secondary tap" @click="showAdd = false">取消</button>
            <button class="btn-primary tap" @click="submitAdd">保存</button>
          </div>
        </div>
      </div>
    </Transition>

    <!-- 续期（v0.4.1 逻辑保持不变） -->
    <Transition name="modal">
      <div v-if="showRenew && renewTarget" class="modal-mask modal-mask-top" @click.self="showRenew = false">
        <div class="modal">
          <div class="modal-header">
            <span>续期便签</span>
            <button class="modal-close tap" @click="showRenew = false">×</button>
          </div>
          <div class="modal-body">
            <div class="form-row">
              <label>
                当前到期：{{ isTempPermanent(renewTarget.expires_at) ? "永不过期" : fmtDateTime(renewTarget.expires_at) }}
              </label>
              <DarkSelect
                v-model="renewChoice"
                :options="renewOptions"
                aria-label="续期方式"
                placeholder="选择续期方式"
                block
              />
              <span v-if="renewTargetIsPermanent" class="field-hint">
                当前为永久便签：延长选项不适用；如需改回限时，请选择自定义到期时间。
              </span>
            </div>
            <div v-if="renewChoice === -2" class="form-row">
              <label>自定义到期时间（最晚 2099-12-31）</label>
              <input v-model="renewCustom" type="datetime-local" class="form-input" :max="RENEW_DT_MAX" />
            </div>
            <p class="renew-preview" v-if="renewPreview">
              预计新到期时间：<b>{{ renewPreview }}</b>
              <template v-if="renewChoice > 0">（在当前到期时间上延长）</template>
            </p>
            <p class="renew-error" v-if="renewError">⚠️ {{ renewError }}</p>
          </div>
          <div class="modal-footer">
            <button class="btn-secondary tap" @click="showRenew = false">取消</button>
            <button class="btn-primary tap" :disabled="!renewReady || renewBusy" @click="submitRenew">
              {{ renewBusy ? "续期中…" : "确认续期" }}
            </button>
          </div>
        </div>
      </div>
    </Transition>

    <!-- 转存为命令（R4：替代原生 prompt） -->
    <Transition name="modal">
      <div v-if="showPromote && promoteTarget" class="modal-mask" @click.self="showPromote = false">
        <div class="modal">
          <div class="modal-header">
            <span>转存为命令</span>
            <button class="modal-close tap" @click="showPromote = false">×</button>
          </div>
          <div class="modal-body">
            <div class="form-row">
              <label>命令标题</label>
              <input v-model="promoteTitle" class="form-input" placeholder="如：Docker 常用命令" />
            </div>
            <div class="form-row">
              <label>内容（来自便签原文）</label>
              <div class="edit-original">{{ promoteTarget.text }}</div>
            </div>
          </div>
          <div class="modal-footer">
            <button class="btn-secondary tap" @click="showPromote = false">取消</button>
            <button class="btn-primary tap" @click="submitPromote">保存</button>
          </div>
        </div>
      </div>
    </Transition>
  </div>
</template>

<style scoped>
.view { padding-bottom: 20px; }
/* view-header / view-title 等都用 global 统一样式 */

.empty { text-align: center; padding: 60px 20px; color: var(--text-muted); }
.empty-icon { font-size: 48px; margin-bottom: 12px; opacity: 0.5; }
.empty-text { font-size: 13px; }

/* ===== U3：分类条（常驻、可横滑、管理入口固定可见） ===== */
.cat-bar {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-bottom: 12px;
}
.cat-bar-scroll {
  display: flex;
  align-items: center;
  gap: 6px;
  flex: 1;
  min-width: 0;
  overflow-x: auto;
  scrollbar-width: none;
  padding-bottom: 2px;
}
.cat-bar-scroll::-webkit-scrollbar { display: none; }
.cat-chip {
  border: 1px solid var(--border);
  background: var(--bg-secondary);
  color: var(--text-muted);
  border-radius: 999px;
  padding: 5px 12px;
  font-size: 11.5px;
  cursor: pointer;
  white-space: nowrap;
  flex-shrink: 0;
  transition: all 0.15s;
  -webkit-app-region: no-drag;
}
.cat-chip:hover { color: var(--text-secondary); border-color: rgba(255, 184, 77, 0.4); }
.cat-chip.active {
  border-color: rgba(255, 184, 77, 0.65);
  background: rgba(255, 184, 77, 0.14);
  color: #ffd48a;
  font-weight: 600;
}
.cat-manage-btn {
  flex-shrink: 0;
  border: 1px solid var(--border);
  background: var(--bg-secondary);
  color: var(--text-secondary);
  border-radius: 999px;
  padding: 5px 10px;
  font-size: 11.5px;
  cursor: pointer;
  white-space: nowrap;
  -webkit-app-region: no-drag;
}
.cat-manage-btn:hover { border-color: var(--accent); color: var(--text-primary); }

/* ===== U3：便签卡片（阅读优先，单一更多入口） ===== */
.list-container { display: flex; flex-direction: column; gap: 8px; }
.note-card {
  position: relative;
  display: flex;
  align-items: flex-start;
  gap: 10px;
  padding: 12px;
  background: var(--bg-primary);
  border: 1px solid var(--border);
  border-radius: 12px;
  cursor: pointer;
  transition: border-color 0.18s, background 0.18s, transform 0.18s;
  -webkit-app-region: no-drag;
}
.note-card:hover {
  border-color: rgba(255, 184, 77, 0.45);
  background: var(--bg-secondary);
  transform: translateX(2px);
}
.note-main { display: flex; align-items: flex-start; gap: 10px; flex: 1; min-width: 0; }
.note-icon { width: 30px; height: 30px; border-radius: 9px; font-size: 14px; flex-shrink: 0; }
.note-info { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 3px; }
.note-title {
  font-size: 13px;
  font-weight: 600;
  color: var(--text-primary);
  line-height: 1.35;
  word-break: break-word;
  display: -webkit-box;
  -webkit-line-clamp: 2;
  -webkit-box-orient: vertical;
  overflow: hidden;
  padding-right: 22px;
}
.note-text {
  font-size: 12.5px;
  color: var(--text-secondary);
  line-height: 1.5;
  word-break: break-word;
  white-space: pre-wrap;
  overflow: hidden;
  display: -webkit-box;
  -webkit-line-clamp: 3;
  -webkit-box-orient: vertical;
}
.note-text-preview {
  -webkit-line-clamp: 1;
  color: var(--text-muted);
  font-size: 12px;
  white-space: normal;
  padding-right: 22px;
}
.note-meta { display: flex; align-items: center; gap: 10px; flex-wrap: wrap; margin-top: 2px; }
.note-cat {
  font-size: 10.5px;
  color: #e8c268;
  background: rgba(232, 194, 104, 0.1);
  border-radius: 6px;
  padding: 1px 7px;
  max-width: 55%;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.note-cat-empty { color: var(--text-faint); background: rgba(255, 255, 255, 0.05); }
.note-time { font-size: 10.5px; color: var(--text-muted); white-space: nowrap; }
.note-time-permanent { color: #34d399; }
.note-time-soon { color: #fbbf24; }
.note-time-expired { color: #f87171; }
.note-progress { height: 2px; background: rgba(0, 0, 0, 0.3); border-radius: 2px; overflow: hidden; margin-top: 4px; }
.note-progress-bar { height: 100%; background: linear-gradient(90deg, var(--warning), #ff8a5e); transition: width 1s linear; }
.note-more {
  position: absolute;
  top: 8px;
  right: 8px;
  width: 26px;
  height: 26px;
  border: none;
  border-radius: 7px;
  background: transparent;
  color: var(--text-muted);
  font-size: 15px;
  line-height: 1;
  cursor: pointer;
  opacity: 0.55;
  transition: all 0.15s;
  -webkit-app-region: no-drag;
}
.note-card:hover .note-more { opacity: 1; }
.note-more:hover { background: var(--bg-tertiary); color: var(--text-primary); }

/* ===== U3：详情浮层 ===== */
.dt-mask {
  position: fixed;
  inset: 0;
  z-index: 300;
  background: rgba(0, 0, 0, 0.62);
  backdrop-filter: blur(6px);
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 16px;
}
.dt-sheet {
  width: min(520px, 96vw);
  max-height: 88vh;
  display: flex;
  flex-direction: column;
  background: var(--bg-secondary);
  border: 1px solid var(--border);
  border-radius: 14px;
  box-shadow: 0 24px 70px rgba(0, 0, 0, 0.6);
  outline: none;
  overflow: hidden;
}
.dt-head {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 10px;
  padding: 12px 14px;
  border-bottom: 1px solid var(--border);
}
.dt-head-text { display: flex; flex-direction: column; gap: 2px; min-width: 0; }
.dt-head-title { font-size: 14px; font-weight: 600; color: var(--text-primary); }
.dt-head-meta { font-size: 11px; color: var(--text-muted); }
.dt-close {
  width: 28px; height: 28px; flex-shrink: 0;
  border: none; border-radius: 7px; background: transparent;
  color: var(--text-muted); font-size: 18px; cursor: pointer;
}
.dt-close:hover { background: var(--bg-tertiary); color: var(--text-primary); }
.dt-body { flex: 1; overflow-y: auto; padding: 14px; }
.dt-title { margin: 0 0 8px; font-size: 16px; font-weight: 700; color: var(--text-primary); word-break: break-word; }
.dt-text {
  margin: 0;
  font-family: inherit;
  font-size: 13px;
  line-height: 1.7;
  color: var(--text-secondary);
  white-space: pre-wrap;
  word-break: break-word;
  max-height: 40vh;
  overflow-y: auto;
}
.dt-meta-line { margin-top: 10px; font-size: 11px; color: var(--text-muted); }
.dt-field { display: flex; flex-direction: column; gap: 4px; margin-bottom: 10px; }
.dt-label { font-size: 11.5px; color: var(--text-muted); }
.dt-cat-row { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; padding-top: 10px; border-top: 1px dashed var(--border); }
.dt-link {
  border: none; background: transparent; color: #7ea6ff;
  font-size: 11.5px; cursor: pointer; padding: 2px 4px;
}
.dt-link:hover { text-decoration: underline; }
/* 详情内的删除确认：允许换行，避免长文案被省略 */
.dt-body :deep(.confirm-inline) {
  display: flex;
  flex-wrap: wrap;
  white-space: normal;
  width: 100%;
  box-sizing: border-box;
}
.dt-body :deep(.confirm-text) {
  flex: 1 1 100%;
  white-space: normal;
  line-height: 1.5;
}
.dt-body :deep(.confirm-actions) { margin-left: auto; }

.dt-discard {
  display: flex; align-items: center; gap: 8px; flex-wrap: wrap;
  margin-top: 10px; padding: 8px 10px;
  border-radius: 8px;
  background: rgba(245, 158, 11, 0.1);
  border: 1px solid rgba(245, 158, 11, 0.35);
  font-size: 12px; color: #fbbf24;
}
.dt-foot {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
  padding: 10px 14px;
  border-top: 1px solid var(--border);
  background: rgba(0, 0, 0, 0.15);
}
.dt-foot .btn-primary, .dt-foot .btn-secondary { flex-shrink: 0; }
.dt-delete {
  margin-left: auto;
  border: none;
  background: transparent;
  color: #a56b6b;
  font-size: 11.5px;
  cursor: pointer;
  padding: 6px 4px;
}
.dt-delete:hover { color: #f87171; text-decoration: underline; }
.danger-text { color: #f87171 !important; }

/* ===== U3：共用操作菜单 ===== */
.tm-menu {
  position: fixed;
  z-index: 400;
  min-width: 196px;
  max-width: 240px;
  max-height: calc(100vh - 16px);
  overflow-y: auto;
  padding: 6px;
  background: rgba(20, 20, 32, 0.97);
  border: 1px solid rgba(255, 255, 255, 0.12);
  border-radius: 12px;
  box-shadow: 0 16px 40px rgba(0, 0, 0, 0.55);
  display: flex;
  flex-direction: column;
  gap: 1px;
}
.tm-menu-title {
  font-size: 11px;
  color: var(--text-muted);
  padding: 6px 8px 4px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.tm-menu-item {
  display: flex;
  align-items: center;
  gap: 8px;
  width: 100%;
  padding: 8px 10px;
  border: none;
  border-radius: 8px;
  background: transparent;
  color: var(--text-secondary);
  font-size: 12.5px;
  text-align: left;
  cursor: pointer;
  -webkit-app-region: no-drag;
}
.tm-menu-item:hover { background: var(--bg-tertiary); color: var(--text-primary); }
.tm-menu-item.danger { color: #f87171; }
.tm-menu-item.danger:hover { background: rgba(244, 63, 94, 0.12); }
.tm-menu-hint { margin-left: auto; font-size: 10.5px; color: var(--text-faint); }
.tm-menu-check { margin-left: auto; color: #34d399; }
.tm-menu-divider { height: 1px; background: var(--border); margin: 4px 2px; }
.tm-menu-back { color: var(--text-muted); font-size: 11.5px; }
.tm-menu-manage { color: #7ea6ff; }

/* 续期等旧弹窗置顶于详情浮层之上 */
.modal-mask-top { z-index: 350; }

/* 旧弹窗与表单样式（保持 v0.4.1 行为） */
.edit-original {
  font-size: 12px; color: var(--text-secondary);
  background: rgba(0, 0, 0, 0.25); border: 1px dashed var(--border);
  border-radius: 8px; padding: 8px 10px;
  max-height: 120px; overflow-y: auto; white-space: pre-wrap; word-break: break-all;
}
.field-hint { font-size: 10.5px; color: var(--text-faint); margin-top: 2px; }
.renew-preview {
  font-size: 12px; color: var(--text-secondary);
  background: rgba(16, 185, 129, 0.08); border-left: 3px solid #10b981;
  border-radius: 6px; padding: 8px 10px; margin: 4px 0 0;
}
.renew-error { font-size: 12px; color: #f87171; margin: 4px 0 0; }

.clipboard-toast { position: fixed; bottom: 16px; right: 16px; padding: 10px 16px; background: rgba(0, 0, 0, 0.8); backdrop-filter: blur(8px); border: 1px solid var(--accent); border-radius: 10px; display: flex; align-items: center; gap: 8px; font-size: 12px; z-index: 99; }

.modal-mask { position: fixed; inset: 0; background: rgba(0, 0, 0, 0.6); backdrop-filter: blur(8px); display: flex; align-items: center; justify-content: center; z-index: 100; padding: 12px; }
.modal { background: var(--bg-secondary); border: 1px solid var(--border); border-radius: 12px; width: 92%; max-width: 420px; max-height: 85vh; display: flex; flex-direction: column; box-shadow: 0 20px 60px rgba(0, 0, 0, 0.5); }
.modal-header { display: flex; justify-content: space-between; align-items: center; padding: 12px 16px; border-bottom: 1px solid var(--border); font-size: 14px; font-weight: 600; }
.modal-close { width: 26px; height: 26px; border: none; background: transparent; color: var(--text-muted); font-size: 18px; cursor: pointer; border-radius: 6px; -webkit-app-region: no-drag; }
.modal-close:hover { background: var(--highlight); color: white; }
.modal-body { padding: 16px; flex: 1; overflow-y: auto; }
.modal-footer { display: flex; justify-content: flex-end; gap: 8px; padding: 12px 16px; border-top: 1px solid var(--border); }
.form-row { display: flex; flex-direction: column; gap: 4px; margin-bottom: 10px; }
.form-row label { font-size: 11px; color: var(--text-muted); }
.form-input { background: rgba(0, 0, 0, 0.3); border: 1px solid var(--border); border-radius: 8px; padding: 7px 10px; color: var(--text-primary); font-size: 13px; font-family: inherit; min-width: 0; }
.form-input:focus { outline: none; border-color: var(--accent); }
.form-textarea { resize: vertical; font-family: inherit; min-height: 100px; }

.btn-primary, .btn-secondary { border: none; border-radius: 8px; padding: 8px 14px; font-size: 12px; cursor: pointer; font-weight: 500; transition: all 0.15s; -webkit-app-region: no-drag; }
.btn-primary { background: linear-gradient(135deg, var(--accent) 0%, #4fc3f7 100%); color: white; }
.btn-primary:hover { transform: translateY(-1px); }
.btn-primary:disabled { opacity: 0.55; cursor: not-allowed; transform: none; }
.btn-secondary { background: var(--bg-primary); color: var(--text-secondary); border: 1px solid var(--border); }
.btn-secondary:hover { background: var(--bg-secondary); border-color: var(--accent); }

.stagger-item { animation: slideIn 0.4s var(--ease-smooth) backwards; }
@keyframes slideIn { from { opacity: 0; transform: translateX(-10px); } to { opacity: 1; transform: translateX(0); } }
.list-enter-active, .list-leave-active { transition: all 0.3s var(--ease-smooth); }
.list-enter-from, .list-leave-to { opacity: 0; transform: translateX(-20px); }
.modal-enter-active, .modal-leave-active { transition: opacity 0.2s; }
.modal-enter-from, .modal-leave-to { opacity: 0; }
.toast-enter-active, .toast-leave-active { transition: all 0.3s var(--ease-smooth); }
.toast-enter-from, .toast-leave-to { opacity: 0; transform: translateY(20px); }

/* 分类管理弹窗 */
.cat-manage-row { display: flex; gap: 8px; }
.cat-manage-row .form-input { flex: 1; min-width: 0; }
.cat-manage-list { margin-top: 8px; display: flex; flex-direction: column; gap: 6px; }
.cat-manage-item { display: flex; align-items: center; gap: 8px; }
.cat-manage-item .form-input { flex: 1; min-width: 0; }
.cat-manage-name { flex: 1; font-size: 13px; color: #e8ecf3; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.cat-manage-empty { font-size: 12px; color: #7c8aa8; padding: 8px 0; }
.cat-manage-hint { font-size: 11px; color: #7c8aa8; margin-top: 10px; }

/* 窄窗（≤419）：详情底部操作两列，不溢出 */
@media (max-width: 419px) {
  .dt-mask { padding: 8px; }
  .dt-sheet { max-height: 92vh; }
  .dt-foot { gap: 6px; }
  .dt-foot .btn-primary, .dt-foot .btn-secondary { flex: 1 1 calc(50% - 6px); min-width: 0; }
  .dt-delete { flex: 1 1 100%; margin-left: 0; text-align: center; }
}
</style>

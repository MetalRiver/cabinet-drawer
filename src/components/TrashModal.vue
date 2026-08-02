<script setup lang="ts">
import { ref, onMounted, computed } from "vue";
import {
  listTrash,
  restoreFromTrash,
  permanentDelete,
  emptyTrash,
  kindToTable,
  getSetting,
  type TrashItem,
} from "../api";
import { useAppStore } from "../stores/app";
import { useWidgetStore } from "../stores/widget";
import { useSoftwareStore } from "../stores/software";
import { usePasswordStore } from "../stores/passwords";
import { useSnippetsStore } from "../stores/snippets";
import { useTempStore } from "../stores/temp";

const appStore = useAppStore();
const widgetStore = useWidgetStore();
const softwareStore = useSoftwareStore();
const passwordStore = usePasswordStore();
const snippetsStore = useSnippetsStore();
const tempStore = useTempStore();

const emit = defineEmits<{
  (e: "close"): void;
  (e: "restored"): void;
}>();

const items = ref<TrashItem[]>([]);
const loading = ref(false);
const filterSource = ref<"all" | "软件" | "密码" | "命令行" | "便签">("all");
const confirmAction = ref<{ kind: "delete" | "empty"; item?: TrashItem } | null>(null);
const retentionDays = ref<number>(30);

const filtered = computed(() => {
  if (filterSource.value === "all") return items.value;
  return items.value.filter((i) => i.source === filterSource.value);
});

const counts = computed(() => {
  const c: Record<string, number> = { all: items.value.length, 软件: 0, 密码: 0, 命令行: 0, 便签: 0 };
  for (const i of items.value) c[i.source] = (c[i.source] || 0) + 1;
  return c;
});

async function load() {
  loading.value = true;
  try {
    items.value = await listTrash();
  } finally {
    loading.value = false;
  }
}

function formatTimeAgo(ms: number): string {
  const diff = Date.now() - ms;
  const sec = Math.floor(diff / 1000);
  if (sec < 60) return "刚刚";
  const min = Math.floor(sec / 60);
  if (min < 60) return `${min} 分钟前`;
  const h = Math.floor(min / 60);
  if (h < 24) return `${h} 小时前`;
  const d = Math.floor(h / 24);
  if (d < 30) return `${d} 天前`;
  return new Date(ms).toLocaleDateString();
}

function formatDateTime(ms: number): string {
  const d = new Date(ms);
  return d.toLocaleString("zh-CN", { hour12: false });
}

function kindIcon(kind: TrashItem["kind"]): string {
  switch (kind) {
    case "app": return "🚀";
    case "password": return "🔑";
    case "snippet": return "⚡";
    case "temp": return "📝";
  }
}

function kindLabel(item: TrashItem): string {
  // 密码显示用户名/URL；软件显示 type；命令行显示 language；便签显示时间
  if (item.kind === "app") {
    const t = item.kind_detail || "app";
    return `${item.sub_detail || t}`;
  }
  if (item.kind === "password") {
    return item.kind_detail || "—";
  }
  if (item.kind === "snippet") {
    return item.kind_detail || "text";
  }
  return "便签";
}

// ✨ 关键修复：操作后刷新对应区域的 Store + 置顶区（全局）
// 否则界面不更新，用户必须重启 app 才看得到变化！
async function reloadStoresByKind(kind?: TrashItem["kind"]) {
  // 置顶区（全局搜索下面的 pinned）必须每次都刷新，因为置顶的数据可能来自任何区域
  await widgetStore.reloadAll();
  if (!kind) {
    // kind 为空 = 清空全部 / 不知道啥类型 → 所有区域全刷一遍
    await Promise.all([
      softwareStore.reloadAll(),
      passwordStore.reloadAll(),
      snippetsStore.reloadAll(),
      tempStore.reloadAll(),
    ]);
    return;
  }
  // 按类型只刷对应区域（省性能）
  switch (kind) {
    case "app": await softwareStore.reloadAll(); break;
    case "password": await passwordStore.reloadAll(); break;
    case "snippet": await snippetsStore.reloadAll(); break;
    case "temp": await tempStore.reloadAll(); break;
  }
}

async function doRestore(item: TrashItem) {
  const table = kindToTable(item.kind);
  await restoreFromTrash(table, item.id);
  await load();
  // 🔴 修复：恢复后刷新对应区域 Store，否则界面看不到恢复的内容！
  await reloadStoresByKind(item.kind);
  emit("restored");
}

function askDelete(item: TrashItem) {
  confirmAction.value = { kind: "delete", item };
}

function askEmpty() {
  confirmAction.value = { kind: "empty" };
}

async function confirmYes() {
  if (!confirmAction.value) return;
  let affectedKind: TrashItem["kind"] | undefined;
  if (confirmAction.value.kind === "delete" && confirmAction.value.item) {
    const item = confirmAction.value.item;
    affectedKind = item.kind;
    await permanentDelete(kindToTable(item.kind), item.id);
  } else if (confirmAction.value.kind === "empty") {
    // 全部清空：依次清空 4 个表
    affectedKind = undefined; // 清空所有，kind 传 undefined → 全刷
    await emptyTrash("apps");
    await emptyTrash("passwords");
    await emptyTrash("snippets");
    await emptyTrash("temp_contents");
  }
  confirmAction.value = null;
  await load();
  // 🔴 修复：删除/清空后也要刷新对应区域 Store！
  await reloadStoresByKind(affectedKind);
  // P0-T#FIX#UI：清空 / 永久删除后**强制刷新**外部徽章计数
  // 之前只在 doRestore 里 emit("restored")，清空路径漏掉 → 外部红色徽章不更新
  // 现在 delete + empty + restore 三个路径都 emit，MainLayout 收到后调 refreshTrashCount
  emit("restored");
}

function cancelConfirm() {
  confirmAction.value = null;
}

onMounted(async () => {
  try {
    const days = await getSetting("trash_retention_days");
    if (days) retentionDays.value = parseInt(days, 10) || 30;
  } catch {}
  await load();
});
</script>

<template>
  <div class="trash-overlay" @click.self="emit('close')">
    <div class="trash-modal">
      <!-- Header -->
      <div class="trash-header">
        <div class="trash-title">
          <span class="trash-icon">🗑️</span>
          <h2>回收站</h2>
          <span class="trash-count">{{ counts.all }} 项</span>
        </div>
        <div class="trash-subtitle">{{ retentionDays }} 天后自动清理 · 永久删除前请三思</div>
        <button class="trash-close" @click="emit('close')" title="关闭">×</button>
      </div>

      <!-- Filter chips -->
      <div class="trash-filters">
        <button
          class="filter-chip"
          :class="{ active: filterSource === 'all' }"
          @click="filterSource = 'all'"
        >全部 <span class="chip-count">{{ counts.all }}</span></button>
        <button
          class="filter-chip"
          :class="{ active: filterSource === '软件' }"
          @click="filterSource = '软件'"
        >🚀 软件 <span class="chip-count">{{ counts.软件 }}</span></button>
        <button
          class="filter-chip"
          :class="{ active: filterSource === '密码' }"
          @click="filterSource = '密码'"
        >🔑 密码 <span class="chip-count">{{ counts.密码 }}</span></button>
        <button
          class="filter-chip"
          :class="{ active: filterSource === '命令行' }"
          @click="filterSource = '命令行'"
        >⚡ 命令行 <span class="chip-count">{{ counts.命令行 }}</span></button>
        <button
          class="filter-chip"
          :class="{ active: filterSource === '便签' }"
          @click="filterSource = '便签'"
        >📝 便签 <span class="chip-count">{{ counts.便签 }}</span></button>
        <div class="filter-spacer"></div>
        <button
          v-if="counts.all > 0"
          class="empty-btn"
          @click="askEmpty"
          title="清空所有回收站项"
        >🗑️ 清空回收站</button>
      </div>

      <!-- Content -->
      <div class="trash-body">
        <div v-if="loading" class="trash-loading">加载中...</div>
        <div v-else-if="filtered.length === 0" class="trash-empty">
          <span class="empty-icon">✨</span>
          <div>回收站是空的</div>
        </div>
        <div v-else class="trash-table">
          <div class="trash-row trash-row-header">
            <div class="col col-name">名称</div>
            <div class="col col-type">类型</div>
            <div class="col col-source">来源专区</div>
            <div class="col col-detail">详情</div>
            <div class="col col-time">删除时间</div>
            <div class="col col-actions">操作</div>
          </div>
          <div
            v-for="item in filtered"
            :key="`${item.kind}-${item.id}`"
            class="trash-row"
          >
            <div class="col col-name" :title="item.name">
              <span class="kind-icon">{{ kindIcon(item.kind) }}</span>
              <span class="item-name">{{ item.name }}</span>
            </div>
            <div class="col col-type">{{ kindLabel(item) }}</div>
            <div class="col col-source">
              <span class="source-badge" :data-source="item.source">{{ item.source }}</span>
            </div>
            <div class="col col-detail" :title="item.sub_detail">
              <span v-if="item.sub_detail" class="detail-text">{{ item.sub_detail }}</span>
              <span v-else class="detail-empty">—</span>
            </div>
            <div class="col col-time" :title="formatDateTime(item.deleted_at)">
              {{ formatTimeAgo(item.deleted_at) }}
            </div>
            <div class="col col-actions">
              <button class="row-btn restore-btn" @click="doRestore(item)" title="恢复">↺</button>
              <button class="row-btn delete-btn" @click="askDelete(item)" title="永久删除">×</button>
            </div>
          </div>
        </div>
      </div>

      <!-- Confirm bar -->
      <Transition name="trash-confirm">
        <div v-if="confirmAction" class="trash-confirm-bar">
          <span class="confirm-icon">⚠️</span>
          <span v-if="confirmAction.kind === 'delete'" class="confirm-text">
            永久删除「{{ confirmAction.item?.name }}」？此操作不可撤销。
          </span>
          <span v-else class="confirm-text">
            确定清空整个回收站？共 {{ counts.all }} 项将被永久删除。
          </span>
          <div class="confirm-actions">
            <button class="confirm-cancel" @click="cancelConfirm">取消</button>
            <button class="confirm-confirm" @click="confirmYes">永久删除</button>
          </div>
        </div>
      </Transition>
    </div>
  </div>
</template>

<style scoped>
.trash-overlay {
  position: fixed;
  inset: 0;
  background: rgba(0, 0, 0, 0.55);
  backdrop-filter: blur(6px);
  z-index: 9999;
  display: flex;
  align-items: center;
  justify-content: center;
  animation: trashFadeIn 0.2s ease-out;
}
@keyframes trashFadeIn {
  from { opacity: 0; }
  to { opacity: 1; }
}

.trash-modal {
  width: min(900px, 90vw);
  max-height: 85vh;
  background: var(--bg-secondary, #1a1d24);
  border: 1px solid var(--border, rgba(255,255,255,0.08));
  border-radius: 14px;
  box-shadow: 0 20px 60px rgba(0,0,0,0.5);
  display: flex;
  flex-direction: column;
  overflow: hidden;
  animation: trashSlideUp 0.25s cubic-bezier(0.16, 1, 0.3, 1);
}
@keyframes trashSlideUp {
  from { transform: translateY(20px); opacity: 0; }
  to { transform: translateY(0); opacity: 1; }
}

.trash-header {
  position: relative;
  padding: 20px 24px 16px;
  border-bottom: 1px solid var(--border, rgba(255,255,255,0.06));
  background: linear-gradient(180deg, rgba(255,255,255,0.02), transparent);
}
.trash-title {
  display: flex;
  align-items: center;
  gap: 12px;
}
.trash-icon {
  font-size: 24px;
}
.trash-title h2 {
  margin: 0;
  font-size: 18px;
  color: var(--text-primary, #f0f2f5);
  font-weight: 600;
}
.trash-count {
  font-size: 12px;
  color: var(--text-muted, #8b95a7);
  background: var(--bg-tertiary, rgba(255,255,255,0.06));
  padding: 3px 10px;
  border-radius: 10px;
  font-weight: 500;
}
.trash-subtitle {
  margin-top: 6px;
  font-size: 12px;
  color: var(--text-faint, #5a6373);
  margin-left: 36px;
}
.trash-close {
  position: absolute;
  top: 16px;
  right: 18px;
  width: 28px;
  height: 28px;
  border: none;
  background: var(--bg-tertiary, rgba(255,255,255,0.06));
  color: var(--text-muted, #8b95a7);
  border-radius: 50%;
  cursor: pointer;
  font-size: 18px;
  line-height: 1;
  transition: all 0.15s;
}
.trash-close:hover {
  background: var(--highlight, #ef4444);
  color: white;
  transform: scale(1.05);
}

.trash-filters {
  padding: 12px 24px;
  display: flex;
  align-items: center;
  gap: 8px;
  border-bottom: 1px solid var(--border, rgba(255,255,255,0.06));
  background: var(--bg-primary, rgba(0,0,0,0.15));
  flex-wrap: wrap;
}
.filter-chip {
  background: transparent;
  border: 1px solid var(--border, rgba(255,255,255,0.08));
  color: var(--text-muted, #8b95a7);
  padding: 5px 12px;
  border-radius: 14px;
  cursor: pointer;
  font-size: 12px;
  transition: all 0.15s;
  display: flex;
  align-items: center;
  gap: 6px;
}
.filter-chip:hover {
  background: var(--bg-tertiary, rgba(255,255,255,0.06));
  color: var(--text-primary, #f0f2f5);
}
.filter-chip.active {
  background: var(--accent, #4f7cff);
  border-color: var(--accent, #4f7cff);
  color: white;
}
.chip-count {
  font-size: 10px;
  background: rgba(255,255,255,0.2);
  padding: 1px 6px;
  border-radius: 8px;
  font-weight: 500;
}
.filter-chip:not(.active) .chip-count {
  background: var(--bg-tertiary, rgba(255,255,255,0.08));
}
.filter-spacer {
  flex: 1;
}
.empty-btn {
  background: var(--bg-tertiary, rgba(255,255,255,0.06));
  border: 1px solid var(--border, rgba(255,255,255,0.1));
  color: var(--text-muted, #8b95a7);
  padding: 5px 12px;
  border-radius: 14px;
  cursor: pointer;
  font-size: 12px;
  transition: all 0.15s;
}
.empty-btn:hover {
  background: var(--highlight, #ef4444);
  border-color: var(--highlight, #ef4444);
  color: white;
}

.trash-body {
  flex: 1;
  overflow-y: auto;
  padding: 4px 0;
}
.trash-loading, .trash-empty {
  padding: 60px 20px;
  text-align: center;
  color: var(--text-muted, #8b95a7);
  font-size: 14px;
}
.empty-icon {
  font-size: 40px;
  display: block;
  margin-bottom: 12px;
  opacity: 0.6;
}

.trash-table {
  width: 100%;
}
.trash-row {
  display: grid;
  grid-template-columns: 1.6fr 1fr 0.7fr 1.2fr 0.9fr 0.6fr;
  gap: 12px;
  padding: 10px 24px;
  align-items: center;
  font-size: 13px;
  border-bottom: 1px solid rgba(255,255,255,0.03);
  transition: background 0.1s;
}
.trash-row:not(.trash-row-header):hover {
  background: rgba(255,255,255,0.025);
}
.trash-row-header {
  font-size: 11px;
  color: var(--text-faint, #5a6373);
  text-transform: uppercase;
  letter-spacing: 0.5px;
  padding-top: 12px;
  padding-bottom: 8px;
  font-weight: 500;
}
.col {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.col-name {
  display: flex;
  align-items: center;
  gap: 8px;
  color: var(--text-primary, #f0f2f5);
  font-weight: 500;
}
.kind-icon {
  font-size: 16px;
  flex-shrink: 0;
}
.item-name {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.col-type {
  color: var(--text-muted, #8b95a7);
  font-size: 12px;
}
.col-source {
  display: flex;
}
.source-badge {
  font-size: 11px;
  padding: 2px 8px;
  border-radius: 8px;
  background: var(--bg-tertiary, rgba(255,255,255,0.06));
  color: var(--text-muted, #8b95a7);
}
.source-badge[data-source="软件"] { background: rgba(79,124,255,0.15); color: #4f7cff; }
.source-badge[data-source="密码"] { background: rgba(251,191,36,0.15); color: #fbbf24; }
.source-badge[data-source="命令行"] { background: rgba(168,85,247,0.15); color: #a855f7; }
.source-badge[data-source="便签"] { background: rgba(34,211,238,0.15); color: #22d3ee; }

.col-detail {
  color: var(--text-muted, #8b95a7);
  font-size: 12px;
}
.detail-text {
  overflow: hidden;
  text-overflow: ellipsis;
  display: block;
}
.detail-empty {
  opacity: 0.4;
}
.col-time {
  color: var(--text-muted, #8b95a7);
  font-size: 12px;
}
.col-actions {
  display: flex;
  gap: 6px;
  justify-content: flex-end;
}
.row-btn {
  width: 26px;
  height: 26px;
  border: none;
  border-radius: 6px;
  cursor: pointer;
  font-size: 14px;
  display: flex;
  align-items: center;
  justify-content: center;
  transition: all 0.15s;
}
.restore-btn {
  background: rgba(52, 211, 153, 0.15);
  color: #34d399;
}
.restore-btn:hover {
  background: #34d399;
  color: white;
  transform: scale(1.1);
}
.delete-btn {
  background: rgba(239, 68, 68, 0.15);
  color: #ef4444;
}
.delete-btn:hover {
  background: #ef4444;
  color: white;
  transform: scale(1.1);
}

.trash-confirm-bar {
  position: absolute;
  left: 24px;
  right: 24px;
  bottom: 24px;
  background: linear-gradient(135deg, rgba(239, 68, 68, 0.95), rgba(220, 38, 38, 0.95));
  padding: 12px 18px;
  border-radius: 10px;
  display: flex;
  align-items: center;
  gap: 12px;
  box-shadow: 0 10px 30px rgba(239, 68, 68, 0.3);
  border: 1px solid rgba(255,255,255,0.2);
}
.confirm-icon {
  font-size: 18px;
}
.confirm-text {
  flex: 1;
  color: white;
  font-size: 13px;
  font-weight: 500;
}
.confirm-actions {
  display: flex;
  gap: 8px;
}
.confirm-cancel, .confirm-confirm {
  padding: 6px 14px;
  border: none;
  border-radius: 6px;
  font-size: 12px;
  cursor: pointer;
  font-weight: 500;
  transition: all 0.15s;
}
.confirm-cancel {
  background: rgba(255,255,255,0.2);
  color: white;
}
.confirm-cancel:hover {
  background: rgba(255,255,255,0.3);
}
.confirm-confirm {
  background: white;
  color: #dc2626;
}
.confirm-confirm:hover {
  background: #fee2e2;
}

.trash-confirm-enter-active, .trash-confirm-leave-active {
  transition: all 0.2s ease;
}
.trash-confirm-enter-from, .trash-confirm-leave-to {
  opacity: 0;
  transform: translateY(20px);
}
</style>

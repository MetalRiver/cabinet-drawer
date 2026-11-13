<script setup lang="ts">
/**
 * HealthView —— 软件健康检查独立页（R4，0.4.7）
 * 数据同源：softwareStore.items + invalidItems（与 header 徽标同一 computed 链）。
 * 对账：收录总数 = 正常 + 真实缺失 + 无法确认（全部由同一数组派生）。
 * 入口：软件区 header 🩺/⚠️ 失效(N) → /health；返回 → /apps。
 */
import { computed, onMounted, ref, watch } from "vue";
import { useRouter } from "vue-router";
import { useAppStore } from "../stores/app";
import { useSoftwareStore } from "../stores/software";
import { relinkApp, openAppLocation, pickPath, listApps } from "../api";
import ConfirmModal from "../components/ConfirmModal.vue";
import StatusLine from "../components/settings/StatusLine.vue";

defineOptions({ name: "HealthView" });

const router = useRouter();
const appStore = useAppStore();
const softwareStore = useSoftwareStore();

const checking = ref(false);
const checkError = ref("");
const reasons = ref<Set<number>>(new Set());
const selectedIds = ref<Set<number>>(new Set());
const batchBusy = ref(false);
const batchConfirm = ref(false);
const batchResult = ref<{ moved: number; skipped: number; failed: number } | null>(null);
const seedingFixtures = ref(false);
const clearingFixtures = ref(false);

const isTestEdition = ref(false);
void import("../api").then(({ getAppEdition }) =>
  getAppEdition().then((e) => (isTestEdition.value = e === "acceptance")).catch(() => {})
);

// R4 对账（修正 v2，实测纠正）：list_apps 返回全部未删除记录（含失效记录），
// list_invalid_apps 返回其中失效子集——二者是包含关系，不可相加（相加会重复计数）。
// 正确口径：总数 = 无过滤的 list_apps 数量；正常 = 总数 − 失效数；恒等式
// 总数 = 正常 + 真实缺失 + 无法确认 由「失效 ⊆ 全部」保证。
// 此前实测 -3 的根因是 HealthView 挂载时 items 尚未加载（陈旧空值），故本页
// 挂载与重新检查时都自行拉取一次无分类过滤的 list_apps（同一命令，仅去过滤），
// 不依赖软件区可能带有分类筛选的 store.items。
const allItems = ref<{ id: number }[]>([]);
const invalidItems = computed(() => softwareStore.invalidItems);
const fixedMissing = computed(() => softwareStore.clearableInvalid);
const protectedItems = computed(() => invalidItems.value.filter((it) => it.kind !== "fixed_missing"));
const invalidCount = computed(() => softwareStore.invalidCount);
const totalCount = computed(() => allItems.value.length);
const normalCount = computed(() => Math.max(0, totalCount.value - invalidItems.value.length));

async function refreshAllItems() {
  try {
    allItems.value = await listApps("", undefined);
  } catch {
    // 保持上次可信数据；错误由 checkError 展示
  }
}

// R4.1 闭环修复：回收站恢复发生在本页之外（TrashModal → softwareStore.reloadAll），
// KeepAlive 下本页不会重挂载 → 本地 allItems 变陈旧（实测恢复后总数显示 3、实际 4）。
// 监听 store.items 变化（恢复/批量删除/重定位都会 reloadAll）→ 重拉无过滤列表对齐。
watch(
  () => softwareStore.items.length,
  () => {
    void refreshAllItems();
  },
);

const summary = computed(() => {
  const n = invalidItems.value.length;
  if (n === 0) return "未发现需要处理的记录";
  return `真实缺失 ${fixedMissing.value.length} 条 · 无法确认 ${protectedItems.value.length} 条（受保护，不可清退）`;
});

function issue(it: { kind: string }): string {
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

function advice(it: { kind: string }): string {
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

function kindLabel(kind: string): string {
  switch (kind) {
    case "fixed_missing":
      return "固定盘缺失 · 可清退";
    case "removable_offline":
      return "移动盘 / 光盘不在位（保护）";
    case "network_offline":
      return "网络位置离线（保护）";
    default:
      return "状态不确定（保护）";
  }
}

function goBack() {
  void router.push("/apps");
}

function goTrash() {
  window.dispatchEvent(new CustomEvent("open-trash"));
}

async function runCheck() {
  if (checking.value) return;
  checking.value = true;
  checkError.value = "";
  try {
    const fixed = await softwareStore.runHealthCheck();
    await refreshAllItems();
    appStore.showClipToast(
      "success",
      fixed > 0 ? `自动重定位 ${fixed} 项，列表已更新` : "检查完成：没有可自动重定位的条目"
    );
  } catch (e) {
    checkError.value = String(e);
  } finally {
    checking.value = false;
  }
}

function toggleReason(id: number) {
  const next = new Set(reasons.value);
  if (next.has(id)) next.delete(id);
  else next.add(id);
  reasons.value = next;
}

function toggleSelect(id: number) {
  const next = new Set(selectedIds.value);
  if (next.has(id)) next.delete(id);
  else next.add(id);
  selectedIds.value = next;
}

function selectAllClearable() {
  const next = new Set<number>();
  for (const it of softwareStore.clearableInvalid) next.add(it.id);
  selectedIds.value = next;
}

async function relink(item: unknown) {
  try {
    const picked = await pickPath({ mode: "file", title: `重新关联「${(item as { name: string }).name}」` });
    if (!picked) return;
    await relinkApp((item as { id: number }).id, picked);
    appStore.showClipToast("success", `已重新关联「${(item as { name: string }).name}」`);
    await softwareStore.loadItems();
    await softwareStore.loadInvalidApps();
  } catch (e) {
    appStore.showClipToast("error", "重新关联失败：" + String(e));
  }
}

async function openLocation(item: unknown) {
  try {
    await openAppLocation((item as { id: number }).id);
  } catch (e) {
    appStore.showClipToast("error", String(e));
  }
}

async function confirmBatch() {
  if (batchBusy.value) return;
  const ids = Array.from(selectedIds.value);
  if (ids.length === 0) return;
  batchBusy.value = true;
  checkError.value = "";
  try {
    const r = await softwareStore.moveAppsToTrash(ids);
    batchResult.value = { moved: r.moved, skipped: r.skipped, failed: r.errors.length };
    selectedIds.value = new Set();
    appStore.refreshTrashCount();
    await Promise.all([refreshAllItems(), softwareStore.loadInvalidApps()]);
  } catch (e) {
    checkError.value = String(e);
  } finally {
    batchBusy.value = false;
  }
}

// 测试版工具（原 AppView 面板迁入）
async function seedFixtures() {
  if (seedingFixtures.value) return;
  seedingFixtures.value = true;
  try {
    const r = await softwareStore.seedTestFixtures();
    appStore.showClipToast("success", `已播种 ${r.inserted} 条验收记录（清理上轮 ${r.cleared_previous} 条），列表已刷新`);
  } catch (e) {
    appStore.showClipToast("error", "播种失败：" + String(e));
  } finally {
    seedingFixtures.value = false;
  }
}

async function resetFixtures() {
  if (clearingFixtures.value) return;
  clearingFixtures.value = true;
  try {
    const n = await softwareStore.clearTestFixtures();
    appStore.showClipToast("success", n > 0 ? `已重置：${n} 条测试记录移入回收站（可恢复）` : "没有需要重置的测试记录");
  } catch (e) {
    appStore.showClipToast("error", "重置失败：" + String(e));
  } finally {
    clearingFixtures.value = false;
  }
}

onMounted(() => {
  void refreshAllItems();
  void softwareStore.loadInvalidApps();
});
</script>

<template>
  <div class="page-wrapper health-page">
    <div class="view-header">
      <div class="view-header-left">
        <div class="view-icon icon-blue">🩺</div>
        <div class="view-header-info">
          <div class="view-title">
            <span class="view-title-text">软件健康检查</span>
            <span class="title-count">{{ totalCount }}</span>
          </div>
          <div class="view-subtitle">{{ summary }}</div>
        </div>
      </div>
      <div class="view-header-actions">
        <button class="action-btn action-btn-secondary tap" title="返回软件区域" @click="goBack">‹ 返回软件</button>
        <button class="action-btn action-btn-text tap" :disabled="checking" title="重新执行健康检查" @click="runCheck">
          {{ checking ? "检查中…" : "🔄 重新检查" }}
        </button>
        <button class="action-btn action-btn-secondary tap" title="打开回收站" @click="goTrash">🗑️ 回收站</button>
      </div>
    </div>

    <!-- 概况卡：四态对账（总数 = 正常 + 真实缺失 + 无法确认） -->
    <div class="hp-overview">
      <div class="hp-stat">
        <span class="hp-num muted">{{ totalCount }}</span>
        <span class="hp-label">收录总数</span>
      </div>
      <div class="hp-stat">
        <span class="hp-num ok">{{ normalCount }}</span>
        <span class="hp-label">正常</span>
      </div>
      <div class="hp-stat">
        <span class="hp-num bad">{{ fixedMissing.length }}</span>
        <span class="hp-label">真实缺失</span>
      </div>
      <div class="hp-stat">
        <span class="hp-num warn">{{ protectedItems.length }}</span>
        <span class="hp-label">无法确认 · 已保护</span>
      </div>
    </div>

    <!-- 空态：全部正常 -->
    <div v-if="!softwareStore.invalidLoading && invalidItems.length === 0" class="hp-empty">
      <div class="hp-empty-icon">✅</div>
      <div class="hp-empty-text">全部正常：没有发现失效或需要确认的软件</div>
    </div>

    <div v-if="softwareStore.invalidLoading" class="hp-empty">检查中…</div>
    <StatusLine v-if="checkError" tone="err" :text="checkError" />

    <!-- 真实缺失 · 可清退 -->
    <template v-if="fixedMissing.length">
      <div class="hp-group-title">真实缺失 · 可清退（{{ fixedMissing.length }}）</div>
      <div class="hp-list">
        <div v-for="it in fixedMissing" :key="it.id" class="hp-row">
          <label class="hp-check" title="勾选后可批量移入回收站">
            <input
              type="checkbox"
              :checked="selectedIds.has(it.id)"
              @change="toggleSelect(it.id)"
            />
          </label>
          <div class="hp-main">
            <div class="hp-name">{{ it.name }} <span class="hp-type">{{ it.app_type || "app" }}</span></div>
            <div class="hp-issue">{{ issue(it) }}</div>
            <div class="hp-advice">{{ advice(it) }}</div>
            <button class="hp-reason-btn tap" data-interactive @click.stop="toggleReason(it.id)">
              {{ reasons.has(it.id) ? "收起原因" : "查看原因" }}
            </button>
            <div v-if="reasons.has(it.id)" class="hp-details">
              <div v-if="it.path">路径：<code>{{ it.path }}</code></div>
              <div v-if="it.lnk_target">目标：<code>{{ it.lnk_target }}</code></div>
              <div>检查结果：{{ kindLabel(it.kind) }}</div>
            </div>
          </div>
          <div class="hp-actions">
            <button class="s-btn-mini tap" data-interactive @click.stop="relink(it)">重新关联</button>
            <button
              v-if="(it.app_type || 'app') !== 'url'"
              class="s-btn-mini tap"
              data-interactive
              @click.stop="openLocation(it)"
            >打开位置</button>
          </div>
        </div>
      </div>
    </template>

    <!-- 无法确认 · 已保护 -->
    <template v-if="protectedItems.length">
      <div class="hp-group-title">无法确认 · 已保护（{{ protectedItems.length }}）—— 不提供清退，避免误删可用软件</div>
      <div class="hp-list">
        <div v-for="it in protectedItems" :key="it.id" class="hp-row protected">
          <span class="hp-shield" title="受保护：不提供清退操作">🛡</span>
          <div class="hp-main">
            <div class="hp-name">{{ it.name }} <span class="hp-type">{{ it.app_type || "app" }}</span></div>
            <div class="hp-issue">{{ issue(it) }}</div>
            <div class="hp-advice">{{ advice(it) }}</div>
            <button class="hp-reason-btn tap" data-interactive @click.stop="toggleReason(it.id)">
              {{ reasons.has(it.id) ? "收起原因" : "查看原因" }}
            </button>
            <div v-if="reasons.has(it.id)" class="hp-details">
              <div v-if="it.path">路径：<code>{{ it.path }}</code></div>
              <div v-if="it.lnk_target">目标：<code>{{ it.lnk_target }}</code></div>
              <div>检查结果：{{ kindLabel(it.kind) }}</div>
            </div>
          </div>
          <div class="hp-actions">
            <button class="s-btn-mini tap" data-interactive @click.stop="relink(it)">重新关联</button>
            <button
              v-if="(it.app_type || 'app') !== 'url'"
              class="s-btn-mini tap"
              data-interactive
              @click.stop="openLocation(it)"
            >打开位置</button>
          </div>
        </div>
      </div>
    </template>

    <!-- 批量操作 -->
    <div v-if="fixedMissing.length" class="hp-foot">
      <button class="s-btn-mini tap" data-interactive @click="selectAllClearable">
        全选可清退（{{ softwareStore.clearableInvalid.length }}）
      </button>
      <button
        class="s-btn-mini s-btn-warn tap"
        :disabled="selectedIds.size === 0 || batchBusy"
        @click="batchConfirm = true"
      >移入回收站（{{ selectedIds.size }}）</button>
      <span class="hp-hint">只删除收录记录，不卸载任何 Windows 程序</span>
    </div>
    <ConfirmModal
      :open="batchConfirm"
      icon="🛡️"
      :title="`将 ${selectedIds.size} 条收录记录移入回收站？`"
      subtitle="来源：软件健康检查页"
      consequence="仅删除抽屉柜中的收录记录，不会卸载任何 Windows 程序。"
      recovery="可在回收站恢复"
      tone="warn"
      confirm-text="移入回收站"
      :loading="batchBusy"
      @cancel="batchConfirm = false"
      @confirm="confirmBatch"
    />
    <StatusLine v-if="batchResult" tone="ok" :text="`✓ 已移入回收站 ${batchResult.moved} 条 ｜ 跳过（已在回收站）${batchResult.skipped} 条${batchResult.failed ? ` ｜ 失败 ${batchResult.failed} 条` : ''}`" />

    <!-- 测试版工具（原软件区面板迁入） -->
    <div v-if="isTestEdition" class="hp-toolbar">
      <button class="s-btn-mini tap" :disabled="seedingFixtures" data-interactive @click="seedFixtures">
        {{ seedingFixtures ? "播种中…" : "🧪 播种验收测试数据" }}
      </button>
      <button class="s-btn-mini tap" :disabled="clearingFixtures" data-interactive @click="resetFixtures">
        {{ clearingFixtures ? "重置中…" : "♻️ 重置测试数据" }}
      </button>
      <span class="hp-hint">仅作用于本测试版数据目录（[TEST] 前缀记录，软删除可恢复）</span>
    </div>
  </div>
</template>

<style scoped>
.health-page {
  padding: 0;
}
.hp-overview {
  display: grid;
  grid-template-columns: repeat(4, 1fr);
  gap: 8px;
  margin-bottom: 14px;
  padding: 14px 16px;
  background: var(--bg-primary);
  border: 1px solid var(--border);
  border-radius: 14px;
}
.hp-stat {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 2px;
  padding: 6px 0;
}
.hp-num {
  font-size: 22px;
  font-weight: 700;
  line-height: 1.1;
}
.hp-num.muted { color: var(--text-secondary); }
.hp-num.ok { color: var(--success); }
.hp-num.bad { color: #fb7185; }
.hp-num.warn { color: #fbbf24; }
.hp-label {
  font-size: 11.5px;
  color: var(--text-muted);
}
.hp-group-title {
  font-size: 13px;
  font-weight: 700;
  color: var(--text-secondary);
  margin: 14px 0 8px;
}
.hp-list {
  display: flex;
  flex-direction: column;
  gap: 8px;
}
.hp-row {
  display: flex;
  align-items: flex-start;
  gap: 10px;
  padding: 10px;
  background: var(--bg-secondary);
  border: 1px solid var(--border);
  border-radius: 10px;
}
.hp-row.protected { border-color: rgba(245, 158, 11, 0.25); }
.hp-check { padding-top: 2px; }
.hp-check input { width: 16px; height: 16px; accent-color: #f43f5e; cursor: pointer; }
.hp-shield { font-size: 14px; width: 16px; text-align: center; opacity: 0.8; padding-top: 1px; }
.hp-main { flex: 1; min-width: 0; }
.hp-name { font-size: 13px; font-weight: 600; color: var(--text-primary); word-break: break-all; }
.hp-type {
  font-size: 10px;
  color: var(--text-muted);
  border: 1px solid var(--border);
  border-radius: 5px;
  padding: 0 5px;
  margin-left: 4px;
}
.hp-issue { font-size: 12px; color: var(--text-secondary); margin-top: 2px; }
.hp-advice { font-size: 11.5px; color: var(--text-muted); margin-top: 3px; line-height: 1.5; }
.hp-reason-btn {
  margin-top: 4px;
  padding: 0;
  border: none;
  background: transparent;
  color: #7ea6ff;
  font-size: 11.5px;
  cursor: pointer;
}
.hp-reason-btn:hover { text-decoration: underline; }
.hp-details {
  margin-top: 6px;
  font-size: 11.5px;
  color: var(--text-muted);
  line-height: 1.6;
  word-break: break-all;
}
.hp-actions {
  display: flex;
  flex-direction: column;
  gap: 6px;
  flex-shrink: 0;
}
.hp-foot {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
  margin-top: 14px;
}
.hp-hint { font-size: 11px; color: var(--text-muted); }
.hp-toolbar {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
  margin-top: 16px;
  padding-top: 12px;
  border-top: 1px dashed var(--border);
}
.hp-empty {
  padding: 18px 4px;
  text-align: center;
  color: var(--text-muted);
  font-size: 12.5px;
}
.hp-empty-icon { font-size: 34px; margin-bottom: 6px; }
@media (max-width: 419px) {
  .hp-overview { grid-template-columns: repeat(2, 1fr); }
  .hp-actions { flex-direction: row; }
}
</style>

<script setup lang="ts">
import { ref, onMounted, onUnmounted, onActivated, onDeactivated, computed } from "vue";
import { useAppStore } from "../stores/app";
import { useWidgetStore } from "../stores/widget";
import { useTempStore } from "../stores/temp";
import { useSnippetsStore } from "../stores/snippets";
import ConfirmInline from "../components/ConfirmInline.vue";

const appStore = useAppStore();
const widgetStore = useWidgetStore();
const tempStore = useTempStore();
const snippetsStore = useSnippetsStore();

// P0-#Z：行内 confirm（统一 5 区风格）
const confirmingDeleteId = ref<number | null>(null);
const deletingId = ref<number | null>(null);

// 新增
const showAdd = ref(false);
const defaultTtl = computed(() => Math.max(1, widgetStore.tempExpireDays) * 24 * 60);
const ttlOptions = computed(() => [
  { value: 60, label: "1 小时" },
  { value: 360, label: "6 小时" },
  { value: 1440, label: "1 天" },
  { value: 4320, label: "3 天" },
  { value: 10080, label: "7 天" },
  { value: 43200, label: "30 天" },
].map(o => ({ ...o, label: o.value === defaultTtl.value ? `${o.label}（默认）` : o.label })));
const addForm = ref({ text: "", ttlMinutes: defaultTtl.value });

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
  startTimer();
});
onUnmounted(stopTimer);
onActivated(startTimer);
onDeactivated(stopTimer);

// 关键修复：去掉 watch(searchQuery) → loadItems
// 之前每次按键都触发后端 DB 查询，导致列表项被整个替换 → 闪屏
// 现在搜索过滤完全在客户端 computed 完成

const filteredItems = computed(() => {
  if (!appStore.searchQuery) return tempStore.items;
  const q = appStore.searchQuery.toLowerCase();
  return tempStore.items.filter((i) => i.text.toLowerCase().includes(q));
});

function timeLeft(expires: number): string {
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

function progress(expires: number, created: number): number {
  const total = expires - created;
  const left = expires - now.value;
  if (total <= 0) return 0;
  return Math.max(0, Math.min(100, (left / total) * 100));
}

function openSummary() {
  appStore.showSummaryModal = true;
}

function openAdd() {
  addForm.value = { text: "", ttlMinutes: defaultTtl.value };
  showAdd.value = true;
}

async function submitAdd() {
  if (!addForm.value.text) {
    appStore.showClipToast("info", "请输入内容");
    return;
  }
  try {
    await tempStore.create(addForm.value.text, addForm.value.ttlMinutes);
    showAdd.value = false;
    appStore.showClipToast("success", "已添加");
  } catch (e) {
    appStore.showClipToast("info", String(e));
  }
}

async function copy(text: string) {
  try {
    await tempStore.copy(text);
    appStore.showClipToast("success", "已复制，30 秒后自动清空");
  } catch (e) {
    appStore.showClipToast("info", String(e));
  }
}

function askRemove(id: number) {
  confirmingDeleteId.value = id;
}

function cancelRemove() {
  confirmingDeleteId.value = null;
}

async function confirmRemove(id: number) {
  deletingId.value = id;
  try {
    await tempStore.remove(id);
    appStore.showClipToast("success", "已删除");
  } catch (e) {
    appStore.showClipToast("info", String(e));
  } finally {
    deletingId.value = null;
    confirmingDeleteId.value = null;
  }
}

async function promoteToSnippet(item: any) {
  const title = prompt("命令标题", item.text.slice(0, 30) + "...");
  if (!title) return;
  try {
    await snippetsStore.create({ title, content: item.text });
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
          <p class="view-subtitle">默认 3 天后过期 · {{ new Date(now).toLocaleString("zh-CN") }}</p>
        </div>
      </div>
      <div class="view-header-actions">
        <button class="action-btn action-btn-primary tap" title="汇总便签" @click="openSummary">
          <span class="action-btn-glyph">📋</span>
        </button>
        <button class="action-btn action-btn-primary tap" title="添加临时内容" @click="openAdd">+</button>
      </div>
    </div>

    <!-- 空状态 -->
    <div v-if="filteredItems.length === 0 && !tempStore.loading" class="empty">
      <div class="empty-icon">⏳</div>
      <div class="empty-text">
        <template v-if="appStore.searchQuery">没有匹配的临时内容</template>
        <template v-else>没有临时内容，点 + 添加</template>
      </div>
    </div>

    <TransitionGroup name="list" tag="div" class="list-container">
      <div
        v-for="(item, index) in filteredItems"
        :key="item.id"
        class="list-item stagger-item"
        :style="{ '--delay': `${index * 0.05}s` }"
      >
        <div class="item-main">
          <div class="item-icon icon-orange">
            <span>⏳</span>
            <div class="item-icon-glow"></div>
          </div>
          <div class="item-info">
            <div class="item-text">{{ item.text }}</div>
            <div class="item-meta">
              <span class="meta-time">⏱ 剩余 {{ timeLeft(item.expires_at) }}</span>
              <div class="meta-progress">
                <div
                  class="meta-progress-bar"
                  :style="{ width: progress(item.expires_at, item.created_at) + '%' }"
                ></div>
              </div>
            </div>
          </div>
        </div>
        <div v-if="confirmingDeleteId !== item.id" class="item-actions">
          <button class="action-btn" title="转存为命令" @click="promoteToSnippet(item)">📌</button>
          <button class="action-btn" title="复制" @click="copy(item.text)">📋</button>
          <button class="action-btn" title="删除" @click="askRemove(item.id)">🗑️</button>
        </div>
        <div v-else class="item-confirm-wrap">
          <ConfirmInline
            title="确定删除此便签？"
            :loading="deletingId === item.id"
            @cancel="cancelRemove"
            @confirm="confirmRemove(item.id)"
          />
        </div>
      </div>
    </TransitionGroup>

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
            <span>添加临时内容</span>
            <button class="modal-close tap" @click="showAdd = false">×</button>
          </div>
          <div class="modal-body">
            <div class="form-row">
              <label>内容</label>
              <textarea v-model="addForm.text" class="form-input form-textarea" rows="6" placeholder="临时保存的内容…"></textarea>
            </div>
            <div class="form-row">
              <label>保留时长</label>
              <select v-model.number="addForm.ttlMinutes" class="form-input">
                <option v-for="o in ttlOptions" :key="o.value" :value="o.value">{{ o.label }}</option>
              </select>
            </div>
          </div>
          <div class="modal-footer">
            <button class="btn-secondary tap" @click="showAdd = false">取消</button>
            <button class="btn-primary tap" @click="submitAdd">保存</button>
          </div>
        </div>
      </div>
    </Transition>
  </div>
</template>

<style scoped>
.view { padding-bottom: 20px; }
/* view-header / view-title 等都用 global 统一样式 */
.view-subtitle { font-size: 12px; color: var(--text-muted); font-family: monospace; }

.btn-add, .btn-summary { display: flex; align-items: center; gap: 6px; padding: 9px 18px; border-radius: var(--radius); font-size: 13px; font-weight: 500; cursor: pointer; border: none; transition: all 0.2s; -webkit-app-region: no-drag; }
.btn-add { background: linear-gradient(135deg, var(--accent) 0%, #4fc3f7 100%); color: white; box-shadow: 0 2px 8px rgba(79, 124, 255, 0.3); }
.btn-add:hover { transform: translateY(-1px); }
.btn-summary { background: linear-gradient(135deg, #ffb84d 0%, #ff8a5e 100%); color: white; box-shadow: 0 4px 16px rgba(255, 184, 77, 0.3); }
.btn-summary:hover { transform: translateY(-2px); }

.empty { text-align: center; padding: 60px 20px; color: var(--text-muted); }
.empty-icon { font-size: 48px; margin-bottom: 12px; opacity: 0.5; }
.empty-text { font-size: 13px; }

.list-container { display: flex; flex-direction: column; gap: 8px; }
.list-item { display: flex; align-items: center; gap: 12px; padding: 10px; background: var(--bg-primary); border: 1px solid var(--border); border-radius: 12px; transition: all 0.2s var(--ease-smooth); }
.list-item:hover { border-color: rgba(255, 184, 77, 0.4); background: var(--bg-secondary); transform: translateX(2px); }
.item-main { display: flex; align-items: center; gap: 10px; flex: 1; min-width: 0; }
.item-icon { width: 36px; height: 36px; border-radius: 10px; display: flex; align-items: center; justify-content: center; font-size: 16px; position: relative; flex-shrink: 0; }
.icon-orange { background: linear-gradient(135deg, rgba(255, 184, 77, 0.3), rgba(255, 184, 77, 0.1)); color: var(--warning); }
.item-icon-glow { position: absolute; inset: 0; background: radial-gradient(circle, rgba(255, 255, 255, 0.2), transparent); pointer-events: none; }
.item-info { flex: 1; min-width: 0; }
.item-text { font-size: 12px; color: var(--text-primary); overflow: hidden; text-overflow: ellipsis; display: -webkit-box; -webkit-line-clamp: 2; -webkit-box-orient: vertical; line-height: 1.4; }
.item-meta { display: flex; align-items: center; gap: 8px; margin-top: 4px; }
.meta-time { font-size: 10px; color: var(--text-muted); font-family: monospace; white-space: nowrap; }
.meta-progress { flex: 1; height: 3px; background: rgba(0, 0, 0, 0.3); border-radius: 2px; overflow: hidden; }
.meta-progress-bar { height: 100%; background: linear-gradient(90deg, var(--warning), #ff8a5e); transition: width 1s linear; }

.item-actions { display: flex; gap: 4px; flex-shrink: 0; }
.action-btn { width: 30px; height: 30px; border: none; background: var(--bg-tertiary); color: var(--text-muted); border-radius: 8px; cursor: pointer; display: flex; align-items: center; justify-content: center; transition: all 0.15s; -webkit-app-region: no-drag; }
.action-btn:hover { background: var(--warning); color: white; transform: scale(1.08); }

.clipboard-toast { position: fixed; bottom: 16px; right: 16px; padding: 10px 16px; background: rgba(0, 0, 0, 0.8); backdrop-filter: blur(8px); border: 1px solid var(--accent); border-radius: 10px; display: flex; align-items: center; gap: 8px; font-size: 12px; z-index: 99; }

.modal-mask { position: fixed; inset: 0; background: rgba(0, 0, 0, 0.6); backdrop-filter: blur(8px); display: flex; align-items: center; justify-content: center; z-index: 100; }
.modal { background: var(--bg-secondary); border: 1px solid var(--border); border-radius: 12px; width: 90%; max-width: 420px; max-height: 85vh; display: flex; flex-direction: column; box-shadow: 0 20px 60px rgba(0, 0, 0, 0.5); }
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

.btn-primary, .btn-secondary { border: none; border-radius: 8px; padding: 7px 14px; font-size: 12px; cursor: pointer; font-weight: 500; transition: all 0.15s; -webkit-app-region: no-drag; }
.btn-primary { background: linear-gradient(135deg, var(--accent) 0%, #4fc3f7 100%); color: white; }
.btn-primary:hover { transform: translateY(-1px); }
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
</style>

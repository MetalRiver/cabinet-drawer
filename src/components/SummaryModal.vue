<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";
import { useAppStore } from "../stores/app";
import { useTempStore } from "../stores/temp";
import { useWidgetStore } from "../stores/widget";
import { copyToClipboardWithTimeout } from "../api";

const appStore = useAppStore();
const tempStore = useTempStore();
const widgetStore = useWidgetStore();

// P0-#SUMMARY#REAL#DATA：汇总按钮的真正用途
// 之前：硬编码 mockData（3 条假数据） + 复制按钮没接 → 用户根本不知道这按钮干嘛的
// 现在：聚合所有便签（temp_contents），按创建时间倒序，格式化为可读文本
//       一键复制整个汇总（30 秒后自动清空剪贴板）→ 适合「一次性导出所有便签」场景
//   格式：`[N] yyyy-MM-dd HH:mm  text  ⏱ 剩余时间`
// 之所以需要：便签是「临时」性质（3 天默认过期），用户偶尔想一次性导出/备份
//             避免过期后找不到 → 手动汇总复制走，不依赖云端
const now = ref(Date.now());
let timer: number | null = null;
onMounted(() => {
  // 模态打开时，如果 store 还没数据，先拉一次
  if (tempStore.items.length === 0) {
    void tempStore.loadItems();
  }
  // 1s 驱动倒计时显示（剩余时间字段）
  timer = window.setInterval(() => {
    now.value = Date.now();
  }, 1000);
});
onUnmounted(() => {
  if (timer) clearInterval(timer);
});

// 排序：创建时间倒序
const sortedItems = computed(() => {
  return [...tempStore.items].sort((a, b) => b.created_at - a.created_at);
});

function fmtDateTime(ts: number): string {
  const d = new Date(ts);
  const pad = (n: number) => n.toString().padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

function fmtRemaining(expires: number): string {
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

// 格式化后的汇总文本（每行一条便签）
const summaryText = computed(() => {
  if (sortedItems.value.length === 0) return "（暂无便签）";
  return sortedItems.value
    .map(
      (it, i) =>
        `[${i + 1}] ${fmtDateTime(it.created_at)}  ${it.text}  ⏱ 剩余 ${fmtRemaining(it.expires_at)}`
    )
    .join("\n");
});

function close() {
  appStore.showSummaryModal = false;
}

// 复制整个汇总到剪贴板，30 秒后自动清空（与便签区一致）
const copyBusy = ref(false);
const copyTip = ref("");
async function copySummary() {
  if (copyBusy.value) return;
  if (sortedItems.value.length === 0) {
    appStore.showClipToast("info", "暂无便签可汇总");
    return;
  }
  copyBusy.value = true;
  try {
    const secs = widgetStore.clipboardClearSeconds;
    await copyToClipboardWithTimeout(summaryText.value, secs);
    copyTip.value = `✓ 已复制，${secs} 秒后自动清空`;
    appStore.showClipToast("success", `汇总已复制，${secs} 秒后自动清空`);
    setTimeout(() => (copyTip.value = ""), 2000);
  } catch (e) {
    appStore.showClipToast("info", "复制失败：" + String(e));
  } finally {
    copyBusy.value = false;
  }
}
</script>

<template>
  <Transition name="modal">
    <div v-if="appStore.showSummaryModal" class="modal-mask" @click.self="close">
      <div class="modal-card">
        <div class="modal-header">
          <div class="modal-title">
            <span class="title-icon">📋</span>
            <div>
              <div class="title-text">便签汇总</div>
              <div class="title-hint">共 {{ sortedItems.length }} 条便签 · 按创建时间倒序</div>
            </div>
          </div>
          <button class="modal-close tap" @click="close" data-interactive>✕</button>
        </div>

        <div class="modal-body">
          <pre class="summary-content">{{ summaryText }}</pre>
        </div>

        <div class="modal-footer">
          <span v-if="copyTip" class="copy-tip">{{ copyTip }}</span>
          <button class="btn-secondary tap" @click="close" data-interactive>取消</button>
          <button class="btn-primary tap" :disabled="copyBusy || sortedItems.length === 0" @click="copySummary" data-interactive>
            <span>📋</span>
            <span>{{ copyBusy ? "复制中…" : "复制到剪贴板" }}</span>
          </button>
        </div>
      </div>
    </div>
  </Transition>
</template>

<style scoped>
.modal-mask {
  position: fixed;
  inset: 0;
  background: rgba(0, 0, 0, 0.6);
  backdrop-filter: blur(8px);
  -webkit-backdrop-filter: blur(8px);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 1000;
}

.modal-card {
  width: 90%;
  max-width: 520px;
  background: var(--bg-glass);
  backdrop-filter: blur(30px) saturate(180%);
  -webkit-backdrop-filter: blur(30px) saturate(180%);
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
  box-shadow: var(--shadow-lg);
  overflow: hidden;
}

.modal-header {
  padding: 18px 20px;
  display: flex;
  align-items: center;
  justify-content: space-between;
  border-bottom: 1px solid var(--border);
}

.modal-title {
  display: flex;
  align-items: center;
  gap: 12px;
}

.title-icon {
  width: 40px;
  height: 40px;
  background: linear-gradient(135deg, var(--accent) 0%, var(--highlight) 100%);
  border-radius: var(--radius);
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: 20px;
  box-shadow: 0 4px 12px rgba(79, 124, 255, 0.3);
}

.title-text {
  font-size: 16px;
  font-weight: 600;
  margin-bottom: 2px;
}

.title-hint {
  font-size: 12px;
  color: var(--text-muted);
}

.modal-close {
  width: 32px;
  height: 32px;
  border-radius: var(--radius-sm);
  background: var(--bg-tertiary);
  border: 1px solid var(--border);
  color: var(--text-muted);
  cursor: pointer;
  font-size: 14px;
  transition: all 0.2s;
}

.modal-close:hover {
  background: var(--highlight);
  border-color: var(--highlight);
  color: white;
  transform: rotate(90deg);
}

.modal-body {
  padding: 20px;
  max-height: 50vh;
  overflow-y: auto;
}

.summary-content {
  font-family: 'Cascadia Code', 'Consolas', monospace;
  font-size: 13px;
  line-height: 1.7;
  white-space: pre-wrap;
  word-break: break-word;
  color: var(--text-secondary);
  background: rgba(0, 0, 0, 0.3);
  padding: 16px;
  border-radius: var(--radius);
  border: 1px solid var(--border);
  margin: 0;
}

.modal-footer {
  padding: 14px 20px;
  display: flex;
  justify-content: flex-end;
  align-items: center;
  gap: 10px;
  border-top: 1px solid var(--border);
}
.copy-tip {
  flex: 1;
  font-size: 11px;
  color: #4ade80;
  font-weight: 500;
}

.btn-secondary,
.btn-primary {
  padding: 9px 18px;
  border-radius: var(--radius);
  font-size: 13px;
  font-weight: 500;
  cursor: pointer;
  border: none;
  display: flex;
  align-items: center;
  gap: 6px;
  transition: all 0.2s;
}

.btn-secondary {
  background: var(--bg-tertiary);
  color: var(--text-secondary);
  border: 1px solid var(--border);
}

.btn-secondary:hover {
  background: var(--bg-glass);
  border-color: var(--border-bright);
}

.btn-primary {
  background: linear-gradient(135deg, var(--accent) 0%, var(--accent-bright) 100%);
  color: white;
  box-shadow: 0 4px 12px rgba(79, 124, 255, 0.3);
}

.btn-primary:hover {
  transform: translateY(-1px);
  box-shadow: 0 6px 16px rgba(79, 124, 255, 0.5);
}

/* 弹窗动画 */
.modal-enter-active .modal-card {
  animation: cardZoom 0.3s var(--ease-spring);
}

.modal-leave-active .modal-card {
  animation: cardZoomOut 0.2s var(--ease-smooth);
}

.modal-enter-active,
.modal-leave-active {
  transition: opacity 0.25s;
}

.modal-enter-from,
.modal-leave-to { opacity: 0; }

@keyframes cardZoom {
  from { transform: scale(0.9) translateY(20px); }
  to { transform: scale(1) translateY(0); }
}

@keyframes cardZoomOut {
  from { transform: scale(1); }
  to { transform: scale(0.95) translateY(10px); }
}
</style>
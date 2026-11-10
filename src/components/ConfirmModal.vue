<script setup lang="ts">
/**
 * ConfirmModal —— 统一删除/危险操作中心模态（R2，0.4.7）
 *
 * 三要素齐备：操作对象（icon+name）+ 操作后果 + 恢复能力。
 * 交互：Esc=取消 / Enter=确认（条件满足时）/ Tab 焦点锁定 / 关闭后焦点回触发元素。
 * 永久删除类：requireText 输入确认（如「清空」「删除」），未满足时确认按钮禁用。
 * 取消永不触发业务 API —— 仅 emit('cancel')，由调用方复位状态。
 */
import { computed, nextTick, onBeforeUnmount, ref, watch } from "vue";

const props = withDefaults(defineProps<{
  open: boolean;
  icon?: string;            // 对象图标（emoji 或缩写字母）
  title: string;            // 操作标题（如 删除「记事本」）
  subtitle?: string;        // 对象补充信息（类型/路径等）
  consequence: string;      // 后果文案（醒目）
  recovery?: string;        // 恢复能力说明（空 = 不可恢复样式）
  tone?: "danger" | "warn"; // danger=永久类，warn=可恢复/配置类
  confirmText?: string;
  cancelText?: string;
  loading?: boolean;
  requireText?: string;     // 输入确认（永久删除类）
}>(), {
  icon: "⚠️",
  subtitle: "",
  recovery: "",
  tone: "danger",
  confirmText: "确认",
  cancelText: "取消",
  loading: false,
  requireText: "",
});

const emit = defineEmits<{ cancel: []; confirm: [] }>();

const cardEl = ref<HTMLElement | null>(null);
const typed = ref("");
let lastFocus: HTMLElement | null = null;

const confirmReady = computed(() => !props.requireText || typed.value === props.requireText);

watch(() => props.open, async (v) => {
  if (v) {
    typed.value = "";
    lastFocus = document.activeElement as HTMLElement | null;
    window.addEventListener("keydown", onKey, true);
    await nextTick();
    // 焦点初始落在取消按钮（危险操作默认安全侧）
    const cancelBtn = cardEl.value?.querySelector<HTMLButtonElement>("[data-role=cancel]");
    cancelBtn?.focus();
  } else {
    window.removeEventListener("keydown", onKey, true);
  }
});

function onKey(e: KeyboardEvent) {
  if (!props.open) return;
  if (e.key === "Escape") {
    e.stopPropagation();
    doCancel();
  } else if (e.key === "Enter" && confirmReady.value && !props.loading) {
    e.preventDefault();
    doConfirm();
  } else if (e.key === "Tab" && cardEl.value) {
    // 焦点锁定：Tab 在卡片内循环
    const els = [...cardEl.value.querySelectorAll<HTMLElement>("button, input, [tabindex]")];
    if (!els.length) return;
    const idx = els.indexOf(document.activeElement as HTMLElement);
    e.preventDefault();
    const next = e.shiftKey ? (idx <= 0 ? els.length - 1 : idx - 1) : (idx < 0 || idx === els.length - 1 ? 0 : idx + 1);
    els[next].focus();
  }
}

function doCancel() {
  emit("cancel");
}

function doConfirm() {
  if (!confirmReady.value || props.loading) return;
  emit("confirm");
}

function onCardClick(e: MouseEvent) {
  // 点击遮罩区（卡片外）= 取消
  if (!cardEl.value?.contains(e.target as Node)) doCancel();
}

onBeforeUnmount(() => {
  window.removeEventListener("keydown", onKey, true);
  restoreFocus();
});

function restoreFocus() {
  lastFocus?.focus?.();
  lastFocus = null;
}

function close() {
  emit("cancel");
  restoreFocus();
}
</script>

<template>
  <Teleport to="body">
    <Transition name="cm-fade">
      <div v-if="open" class="cm-mask" @mousedown.self="onCardClick">
        <Transition name="cm-zoom" appear>
          <div
            v-if="open"
            ref="cardEl"
            class="cm-card"
            :class="{ danger: tone === 'danger' }"
            role="alertdialog"
            aria-modal="true"
            :aria-label="title"
          >
            <div class="cm-icon-row">
              <span class="cm-icon">{{ icon }}</span>
              <span class="cm-tone-chip" :class="tone">{{ recovery ? recovery : (tone === "danger" ? "不可恢复" : "需确认") }}</span>
            </div>
            <div class="cm-title">{{ title }}</div>
            <div v-if="subtitle" class="cm-subtitle">{{ subtitle }}</div>
            <div class="cm-consequence" :class="tone">{{ consequence }}</div>
            <div v-if="requireText" class="cm-require">
              <div class="cm-require-hint">输入 <b>{{ requireText }}</b> 以确认执行：</div>
              <input
                v-model="typed"
                class="cm-require-input"
                type="text"
                :placeholder="requireText"
                autocomplete="off"
                spellcheck="false"
                data-interactive
              />
            </div>
            <div class="cm-actions">
              <button
                class="cm-btn cancel tap"
                data-role="cancel"
                :disabled="loading"
                @click="doCancel"
              >{{ cancelText }}</button>
              <button
                class="cm-btn confirm tap"
                :class="tone"
                :disabled="!confirmReady || loading"
                @click="doConfirm"
              >{{ loading ? "处理中…" : confirmText }}</button>
            </div>
          </div>
        </Transition>
      </div>
    </Transition>
  </Teleport>
</template>

<style scoped>
.cm-mask {
  position: fixed;
  inset: 0;
  z-index: 3000;
  background: rgba(0, 0, 0, 0.65);
  backdrop-filter: blur(8px);
  -webkit-backdrop-filter: blur(8px);
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 16px;
}
.cm-card {
  width: 100%;
  max-width: 400px;
  max-height: 80vh;
  overflow-y: auto;
  background: linear-gradient(135deg, #1a1d2e 0%, #221a32 100%);
  border: 1px solid var(--border-strong);
  border-radius: 16px;
  box-shadow: 0 24px 80px rgba(0, 0, 0, 0.7), 0 0 0 1px rgba(255, 94, 126, 0.12);
  padding: 18px 20px 16px;
  color: var(--text-primary);
}
.cm-icon-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: 10px;
}
.cm-icon {
  font-size: 30px;
  line-height: 1;
}
.cm-tone-chip {
  font-size: 10.5px;
  font-weight: 700;
  padding: 2px 9px;
  border-radius: 999px;
  letter-spacing: 0.5px;
}
.cm-tone-chip.danger {
  color: #fb7185;
  background: rgba(244, 63, 94, 0.12);
  border: 1px solid rgba(244, 63, 94, 0.45);
}
.cm-tone-chip.warn {
  color: #fbbf24;
  background: rgba(245, 158, 11, 0.1);
  border: 1px solid rgba(245, 158, 11, 0.4);
}
.cm-title {
  font-size: 15px;
  font-weight: 700;
  line-height: 1.35;
  word-break: break-all;
}
.cm-subtitle {
  font-size: 12px;
  color: var(--text-muted);
  margin-top: 4px;
  word-break: break-all;
}
.cm-consequence {
  font-size: 12.5px;
  line-height: 1.6;
  margin-top: 10px;
  padding: 8px 10px;
  border-radius: 10px;
  word-break: break-all;
}
.cm-consequence.danger {
  background: rgba(244, 63, 94, 0.1);
  border: 1px solid rgba(244, 63, 94, 0.35);
  color: #fda4af;
}
.cm-consequence.warn {
  background: rgba(245, 158, 11, 0.08);
  border: 1px solid rgba(245, 158, 11, 0.35);
  color: #fcd34d;
}
.cm-require {
  margin-top: 10px;
}
.cm-require-hint {
  font-size: 11.5px;
  color: var(--text-secondary);
  margin-bottom: 6px;
}
.cm-require-hint b {
  color: #fda4af;
  font-weight: 700;
}
.cm-require-input {
  width: 100%;
  padding: 8px 10px;
  background: rgba(0, 0, 0, 0.3);
  border: 1px solid rgba(244, 63, 94, 0.4);
  border-radius: 8px;
  color: var(--text-primary);
  font-size: 13px;
  outline: none;
  box-sizing: border-box;
}
.cm-require-input:focus {
  border-color: rgba(244, 63, 94, 0.7);
  box-shadow: 0 0 0 3px rgba(244, 63, 94, 0.15);
}
.cm-actions {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
  margin-top: 16px;
}
.cm-btn {
  min-height: 34px;
  min-width: 88px;
  padding: 0 14px;
  border: none;
  border-radius: 10px;
  font-size: 12.5px;
  font-weight: 600;
  cursor: pointer;
  transition: background-color 0.15s var(--ease-smooth), transform 0.1s;
}
.cm-btn:disabled {
  opacity: 0.45;
  cursor: not-allowed;
}
.cm-btn.cancel {
  background: var(--bg-tertiary);
  color: var(--text-secondary);
  border: 1px solid var(--border-strong);
}
.cm-btn.cancel:hover:not(:disabled) {
  background: var(--bg-secondary);
  color: var(--text-primary);
}
.cm-btn.confirm.danger {
  background: linear-gradient(135deg, #ff5e7e 0%, #ff3b5c 100%);
  color: #fff;
  box-shadow: 0 2px 8px rgba(255, 59, 92, 0.35);
}
.cm-btn.confirm.danger:hover:not(:disabled) {
  box-shadow: 0 4px 12px rgba(255, 59, 92, 0.5);
}
.cm-btn.confirm.warn {
  background: linear-gradient(135deg, var(--accent) 0%, #818cf8 100%);
  color: #fff;
  box-shadow: 0 2px 8px rgba(99, 102, 241, 0.3);
}
.cm-btn.confirm.warn:hover:not(:disabled) {
  box-shadow: 0 4px 12px rgba(99, 102, 241, 0.45);
}
/* 入场动画（≤300ms 规范） */
.cm-fade-enter-active, .cm-fade-leave-active { transition: opacity 0.18s var(--ease-smooth); }
.cm-fade-enter-from, .cm-fade-leave-to { opacity: 0; }
.cm-zoom-enter-active { animation: cmZoomIn 0.22s var(--ease-spring); }
.cm-zoom-leave-active { animation: cmZoomOut 0.15s var(--ease-smooth); }
@keyframes cmZoomIn {
  from { opacity: 0; transform: scale(0.94) translateY(10px); }
  to { opacity: 1; transform: scale(1) translateY(0); }
}
@keyframes cmZoomOut {
  from { opacity: 1; }
  to { opacity: 0; transform: scale(0.97); }
}
</style>

<script setup lang="ts" generic="T extends string | number">
// ===== DarkSelect：深色主题下拉选择（v0.4.1）=====
// 背景：Windows WebView2 原生 <select> 弹出层是系统浅色样式，与深色主题冲突
//（用户反馈：设置界面下拉出现白色背景）。本组件用 Teleport + 固定定位自绘弹层。
// 特性：深色背景 / 悬停与选中态 / 键盘导航（↑↓ Home End Enter Esc）/
// aria（listbox + aria-activedescendant）/ 视口下方放不下自动上翻 /
// Teleport 到 body（不受父容器 overflow 裁切）/ 长列表滚动 / 点击外部关闭。

import { computed, nextTick, onBeforeUnmount, ref, useId, watch } from "vue";

export interface DarkSelectOption<T extends string | number> {
  value: T;
  label: string;
}

const props = withDefaults(
  defineProps<{
    modelValue: T;
    options: DarkSelectOption<T>[];
    disabled?: boolean;
    placeholder?: string;
    ariaLabel?: string;
    /** true = 撑满父容器宽度（表单/弹窗内）；默认 inline（pref 行内） */
    block?: boolean;
  }>(),
  { disabled: false, placeholder: "请选择", ariaLabel: "", block: false }
);

const emit = defineEmits<{ (e: "update:modelValue", v: T): void }>();

// R7：listbox 弹层的稳定 id（aria-controls / aria-activedescendant 用）
const popupId = `dark-select-popup-${useId()}`;

const open = ref(false);
const rootEl = ref<HTMLElement | null>(null);
const triggerEl = ref<HTMLElement | null>(null);
const popupEl = ref<HTMLElement | null>(null);
const popupStyle = ref<{ left: string; top: string; minWidth: string; maxWidth: string; upward: boolean }>({
  left: "0px",
  top: "0px",
  minWidth: "0px",
  maxWidth: "480px",
  upward: false,
});
const activeIndex = ref(0);

const selectedLabel = computed(
  () => props.options.find((o) => o.value === props.modelValue)?.label ?? props.placeholder
);
const selectedIndex = computed(() =>
  props.options.findIndex((o) => o.value === props.modelValue)
);

function positionPopup() {
  const el = triggerEl.value;
  if (!el) return;
  const rect = el.getBoundingClientRect();
  const VIEWPORT_MARGIN = 8;
  const ESTIMATED_ITEM_H = 34;
  const estimatedH = Math.min(props.options.length * ESTIMATED_ITEM_H + 12, 280);
  const spaceBelow = window.innerHeight - rect.bottom - VIEWPORT_MARGIN;
  const upward = spaceBelow < estimatedH && rect.top - VIEWPORT_MARGIN > spaceBelow;
  popupStyle.value = {
    left: `${Math.round(rect.left)}px`,
    top: upward
      ? `${Math.round(Math.max(VIEWPORT_MARGIN, rect.top - estimatedH - 4))}px`
      : `${Math.round(rect.bottom + 4)}px`,
    minWidth: `${Math.round(rect.width)}px`,
    maxWidth: `${Math.round(Math.min(480, window.innerWidth - VIEWPORT_MARGIN * 2))}px`,
    upward,
  };
}

async function toggleOpen() {
  if (props.disabled) return;
  if (open.value) {
    close();
    return;
  }
  activeIndex.value = selectedIndex.value >= 0 ? selectedIndex.value : 0;
  open.value = true;
  await nextTick();
  positionPopup();
  popupEl.value?.focus({ preventScroll: true });
  // 滚动到当前选中项
  const sel = popupEl.value?.querySelector('[aria-selected="true"]');
  sel?.scrollIntoView({ block: "nearest" });
}

function close() {
  open.value = false;
}

function choose(opt: DarkSelectOption<T>) {
  emit("update:modelValue", opt.value);
  close();
  triggerEl.value?.focus({ preventScroll: true });
}

function moveActive(delta: number) {
  if (props.options.length === 0) return;
  activeIndex.value = (activeIndex.value + delta + props.options.length) % props.options.length;
  const el = popupEl.value?.querySelector<HTMLElement>(`[data-idx="${activeIndex.value}"]`);
  el?.scrollIntoView({ block: "nearest" });
}

function onTriggerKeydown(e: KeyboardEvent) {
  if (props.disabled) return;
  switch (e.key) {
    case "Enter":
    case " ":
    case "ArrowDown":
      e.preventDefault();
      if (!open.value) void toggleOpen();
      break;
    case "ArrowUp":
      e.preventDefault();
      if (!open.value) void toggleOpen();
      break;
  }
}

function onPopupKeydown(e: KeyboardEvent) {
  switch (e.key) {
    case "ArrowDown":
      e.preventDefault();
      moveActive(1);
      break;
    case "ArrowUp":
      e.preventDefault();
      moveActive(-1);
      break;
    case "Home":
      e.preventDefault();
      activeIndex.value = 0;
      break;
    case "End":
      e.preventDefault();
      activeIndex.value = props.options.length - 1;
      break;
    case "Enter":
    case " ": {
      e.preventDefault();
      const opt = props.options[activeIndex.value];
      if (opt) choose(opt);
      break;
    }
    case "Escape":
    case "Tab":
      e.preventDefault();
      close();
      triggerEl.value?.focus({ preventScroll: true });
      break;
  }
}

function onDocMousedown(e: MouseEvent) {
  if (!open.value) return;
  const target = e.target as Node;
  if (rootEl.value?.contains(target)) return;
  if (popupEl.value?.contains(target)) return;
  close();
}

function onWindowScrollOrResize() {
  if (open.value) positionPopup();
}

watch(open, (v) => {
  if (v) {
    document.addEventListener("mousedown", onDocMousedown, true);
    window.addEventListener("scroll", onWindowScrollOrResize, true);
    window.addEventListener("resize", onWindowScrollOrResize);
  } else {
    document.removeEventListener("mousedown", onDocMousedown, true);
    window.removeEventListener("scroll", onWindowScrollOrResize, true);
    window.removeEventListener("resize", onWindowScrollOrResize);
  }
});

onBeforeUnmount(() => {
  document.removeEventListener("mousedown", onDocMousedown, true);
  window.removeEventListener("scroll", onWindowScrollOrResize, true);
  window.removeEventListener("resize", onWindowScrollOrResize);
});
</script>

<template>
  <div ref="rootEl" class="dark-select" :class="{ open, 'is-block': block }">
    <button
      ref="triggerEl"
      type="button"
      class="dark-select-trigger"
      :class="{ 'is-open': open, 'is-disabled': disabled }"
      :disabled="disabled"
      role="combobox"
      :aria-expanded="open"
      :aria-label="ariaLabel || undefined"
      :aria-controls="open ? popupId : undefined"
      :aria-activedescendant="open ? `${popupId}-opt-${activeIndex}` : undefined"
      aria-haspopup="listbox"
      data-interactive
      @click="toggleOpen"
      @keydown="onTriggerKeydown"
    >
      <span class="dark-select-value" :class="{ 'is-placeholder': selectedIndex < 0 }" :title="selectedLabel">
        {{ selectedLabel }}
      </span>
      <span class="dark-select-arrow" aria-hidden="true">▾</span>
    </button>

    <Teleport to="body">
      <div
        v-if="open"
        :id="popupId"
        ref="popupEl"
        class="dark-select-popup"
        :class="{ upward: popupStyle.upward }"
        :style="popupStyle"
        role="listbox"
        tabindex="-1"
        :aria-label="ariaLabel || undefined"
        @keydown="onPopupKeydown"
      >
        <div
          v-for="(opt, i) in options"
          :id="`${popupId}-opt-${i}`"
          :key="String(opt.value)"
          class="dark-select-option"
          :class="{
            active: i === activeIndex,
            selected: opt.value === modelValue,
          }"
          role="option"
          :data-idx="i"
          :aria-selected="opt.value === modelValue"
          @mouseenter="activeIndex = i"
          @mousedown.prevent
          @click="choose(opt)"
        >
          <span class="option-label" :title="opt.label">{{ opt.label }}</span>
          <span v-if="opt.value === modelValue" class="option-check" aria-hidden="true">✓</span>
        </div>
      </div>
    </Teleport>
  </div>
</template>

<style scoped>
.dark-select {
  position: relative;
  display: inline-block;
  min-width: 0;
}
.dark-select.is-block {
  display: block;
  width: 100%;
}

.dark-select-trigger {
  display: inline-flex;
  align-items: center;
  justify-content: space-between;
  gap: 6px;
  width: 100%;
  min-width: 128px;
  padding: 7px 10px;
  background: var(--bg-secondary, #232a3d);
  border: 1px solid var(--border, rgba(255, 255, 255, 0.14));
  border-radius: 8px;
  color: var(--text-primary, #e8ecf3);
  font-size: 11.5px;
  cursor: pointer;
  transition: border-color 0.2s, box-shadow 0.2s;
  -webkit-app-region: no-drag;
  box-sizing: border-box;
}
.dark-select-trigger:hover {
  border-color: rgba(120, 130, 255, 0.45);
}
.dark-select-trigger.is-open,
.dark-select-trigger:focus-visible {
  border-color: var(--accent, #6366f1);
  box-shadow: 0 0 0 3px rgba(99, 102, 241, 0.15);
  outline: none;
}
.dark-select-trigger.is-disabled {
  opacity: 0.5;
  cursor: not-allowed;
}
.dark-select-value {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.dark-select-value.is-placeholder {
  color: var(--text-faint, #8a95ad);
}
.dark-select-arrow {
  flex-shrink: 0;
  font-size: 10px;
  color: var(--text-muted, #aab4c8);
  transition: transform 0.18s;
}
.is-open .dark-select-arrow {
  transform: rotate(180deg);
}

/* 弹层：Teleport 到 body，fixed 定位 */
.dark-select-popup {
  position: fixed;
  z-index: 10000;
  max-height: 280px;
  overflow-y: auto;
  background: #1c2233;
  border: 1px solid rgba(255, 255, 255, 0.16);
  border-radius: 10px;
  box-shadow: 0 12px 32px rgba(0, 0, 0, 0.55);
  padding: 4px;
  box-sizing: border-box;
  outline: none;
}

.dark-select-option {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  padding: 8px 10px;
  border-radius: 7px;
  font-size: 12px;
  color: #d7dcea;
  cursor: pointer;
  user-select: none;
  white-space: nowrap;
}
.dark-select-option .option-label {
  overflow: hidden;
  text-overflow: ellipsis;
}
.dark-select-option.active {
  background: rgba(99, 102, 241, 0.22);
  color: #ffffff;
}
.dark-select-option.selected {
  color: #a5b4fc;
  font-weight: 600;
}
.dark-select-option.selected.active {
  background: rgba(99, 102, 241, 0.32);
}
.option-check {
  flex-shrink: 0;
  font-size: 11px;
  color: #a5b4fc;
}
</style>

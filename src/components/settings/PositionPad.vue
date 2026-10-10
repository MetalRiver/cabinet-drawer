<script lang="ts">
// 类型独立导出（script setup 不能 export 类型）
export type PadPreset =
  | "top-left" | "top-center" | "top-right"
  | "middle-left" | "center" | "middle-right"
  | "bottom-left" | "bottom-center" | "bottom-right";
</script>

<script setup lang="ts">
// ===== PositionPad：3×3 九宫格窗口位置板（Settings UI V2）=====
// 九格全部是位置动作（八方位 + 中心=屏幕居中），不混入保存操作；
// 每格带中文文字标注；基于真实显示器工作区域（见 widget.applyPresetPosition）。

const CELLS: { preset: PadPreset; arrow: string; label: string }[] = [
  { preset: "top-left", arrow: "↖", label: "左上" },
  { preset: "top-center", arrow: "↑", label: "上中" },
  { preset: "top-right", arrow: "↗", label: "右上" },
  { preset: "middle-left", arrow: "←", label: "左中" },
  { preset: "center", arrow: "⊙", label: "居中" },
  { preset: "middle-right", arrow: "→", label: "右中" },
  { preset: "bottom-left", arrow: "↙", label: "左下" },
  { preset: "bottom-center", arrow: "↓", label: "下中" },
  { preset: "bottom-right", arrow: "↘", label: "右下" },
];

withDefaults(defineProps<{ disabled?: boolean; activePreset?: string | null }>(), {
  disabled: false,
  activePreset: null,
});
const emit = defineEmits<{ (e: "select", preset: PadPreset): void }>();
</script>

<template>
  <div class="pad" role="group" aria-label="窗口预设位置">
    <button
      v-for="cell in CELLS"
      :key="cell.preset"
      class="pad-cell tap"
      :class="{ 'pad-center': cell.preset === 'center', 'pad-active': activePreset === cell.preset }"
      :disabled="disabled"
      data-interactive
      @click="emit('select', cell.preset)"
    >
      <span class="pad-arrow" aria-hidden="true">{{ cell.arrow }}</span>
      <span class="pad-label">{{ cell.label }}</span>
    </button>
  </div>
</template>

<style scoped>
.pad {
  display: grid;
  grid-template-columns: repeat(3, 1fr);
  gap: 6px;
}
.pad-cell {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 2px;
  padding: 9px 4px;
  background: var(--bg-secondary);
  border: 1px solid var(--border);
  border-radius: 9px;
  color: var(--text-secondary);
  cursor: pointer;
  transition: all 0.15s;
  -webkit-app-region: no-drag;
}
.pad-cell:hover:not(:disabled) {
  border-color: rgba(120, 130, 255, 0.5);
  color: var(--text-primary);
  background: var(--bg-tertiary);
}
.pad-cell:disabled { opacity: 0.45; cursor: not-allowed; }
.pad-arrow { font-size: 15px; line-height: 1; }
.pad-label { font-size: 11px; }
.pad-center {
  background: linear-gradient(135deg, rgba(99, 102, 241, 0.2), rgba(59, 130, 246, 0.12));
  border-color: rgba(120, 130, 255, 0.4);
  color: #c7d2fe;
}
.pad-active {
  border-color: var(--accent);
  box-shadow: 0 0 0 2px rgba(99, 102, 241, 0.25);
}
</style>

<script setup lang="ts">
// ===== SettingsRow：设置行（图标 + 名称 + 说明 + 控件槽，Settings UI V2）=====
// 统一行为偏好/回收站等行的视觉；≤379px 自动上下堆叠，控件不压缩。
withDefaults(
  defineProps<{
    icon?: string;
    label: string;
    sub?: string;
  }>(),
  { icon: "", sub: "" }
);
</script>

<template>
  <div class="s-row">
    <div class="s-row-left">
      <span v-if="icon" class="s-row-icon">{{ icon }}</span>
      <div class="s-row-text">
        <span class="s-row-label">{{ label }}</span>
        <span v-if="sub" class="s-row-sub">{{ sub }}</span>
      </div>
    </div>
    <div class="s-row-control">
      <slot />
    </div>
  </div>
</template>

<style scoped>
.s-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 11px 12px;
  min-height: 48px;
  border-bottom: 1px solid var(--border);
  box-sizing: border-box;
}
.s-row:last-child { border-bottom: none; }
.s-row-left {
  display: flex;
  align-items: center;
  gap: 10px;
  min-width: 0;
  flex: 1;
}
.s-row-icon {
  width: 30px;
  height: 30px;
  border-radius: 8px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  background: linear-gradient(135deg, rgba(99, 102, 241, 0.18), rgba(59, 130, 246, 0.1));
  font-size: 14px;
  flex-shrink: 0;
}
.s-row-text { display: flex; flex-direction: column; min-width: 0; }
.s-row-label { font-size: 13px; font-weight: 600; color: var(--text-primary); line-height: 1.35; }
.s-row-sub { font-size: 11.5px; color: var(--text-muted); line-height: 1.45; margin-top: 2px; }
.s-row-control { flex-shrink: 0; display: flex; align-items: center; }

@media (max-width: 379px) {
  .s-row { flex-direction: column; align-items: stretch; }
  .s-row-control { justify-content: flex-end; }
}
</style>

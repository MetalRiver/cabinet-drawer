<script setup lang="ts">
// ===== StatusLine：统一内联状态反馈（✓ / ✗ / ⚠ / ℹ，Settings UI V2）=====
// 替代散落的 hint-inline 手写 class 组合；多行文本用 pre-line。
withDefaults(
  defineProps<{
    tone: "ok" | "err" | "warn" | "info";
    text: string;
  }>(),
  {}
);
</script>

<template>
  <p v-if="text" class="s-status" :class="`s-status-${tone}`">{{ text }}</p>
</template>

<style scoped>
.s-status {
  font-size: 12px;
  line-height: 1.6;
  margin: 8px 0 0;
  padding: 8px 10px;
  border-radius: 8px;
  white-space: pre-line;
  word-break: break-word;
}
.s-status-ok { color: #34d399; background: rgba(16, 185, 129, 0.08); border-left: 3px solid #10b981; }
.s-status-err { color: #f87171; background: rgba(244, 63, 94, 0.08); border-left: 3px solid #f43f5e; }
.s-status-warn { color: #fbbf24; background: rgba(245, 158, 11, 0.08); border-left: 3px solid #f59e0b; }
.s-status-info { color: #93c5fd; background: rgba(59, 130, 246, 0.08); border-left: 3px solid #3b82f6; }
</style>

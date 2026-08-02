<script setup lang="ts">
/**
 * 统一 confirm 组件
 * P0-#Z：全 5 区删除/危险操作一律用此组件
 *
 * 三种模式：
 * - 默认（行内）：用于密码/命令行/便签展开行，宽度充足
 * - compact：用于软件区小卡片，按钮变紧凑
 * - 卡片小屏时建议改用 toast 模式（顶部固定条）
 *
 * 用法：
 *   <ConfirmInline
 *     v-if="confirmingId === item.id"
 *     :title="`确定删除「${item.name}」？`"
 *     compact
 *     @cancel="confirmingId = null"
 *     @confirm="confirmDelete(item.id)"
 *   />
 */
defineProps<{
  title?: string;
  cancelText?: string;
  confirmText?: string;
  loading?: boolean;
  compact?: boolean; // 紧凑模式：用于 80px 卡片内
}>();

defineEmits<{
  cancel: [];
  confirm: [];
}>();
</script>

<template>
  <div class="confirm-inline" :class="{ compact }">
    <span v-if="!compact" class="confirm-icon">⚠️</span>
    <span class="confirm-text" :class="{ ellipsis: !compact }">{{ title || "确定删除？" }}</span>
    <div class="confirm-actions">
      <button
        class="confirm-btn cancel tap"
        :disabled="loading"
        @click="$emit('cancel')"
      >{{ cancelText || "取消" }}</button>
      <button
        class="confirm-btn danger tap"
        :disabled="loading"
        @click="$emit('confirm')"
      >{{ confirmText || "删除" }}</button>
    </div>
  </div>
</template>

<style scoped>
.confirm-inline {
  display: inline-flex;
  align-items: center;
  gap: 8px;
  padding: 8px 10px;
  background: linear-gradient(135deg, rgba(255, 94, 126, 0.18) 0%, rgba(255, 78, 110, 0.12) 100%);
  border: 1px solid rgba(255, 94, 126, 0.5);
  border-radius: 10px;
  box-shadow: 0 4px 14px rgba(255, 78, 110, 0.35), 0 0 0 1px rgba(0, 0, 0, 0.2);
  font-size: 12px;
  animation: confirmSlideIn 0.2s var(--ease-smooth);
  white-space: nowrap;
  backdrop-filter: blur(8px);
  -webkit-backdrop-filter: blur(8px);
}
@keyframes confirmSlideIn {
  0% { opacity: 0; transform: translateY(-4px) scale(0.95); }
  100% { opacity: 1; transform: translateY(0) scale(1); }
}
.confirm-icon {
  font-size: 14px;
  line-height: 1;
  flex-shrink: 0;
}
.confirm-text {
  color: rgba(255, 255, 255, 0.95);
  font-weight: 500;
  font-size: 12px;
  max-width: 200px;
}
.confirm-text.ellipsis {
  flex: 1;
  min-width: 0;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.confirm-actions {
  display: flex;
  gap: 4px;
  flex-shrink: 0;
}
.confirm-btn {
  height: 24px;
  padding: 0 10px;
  border: none;
  border-radius: 6px;
  font-size: 11px;
  font-weight: 600;
  cursor: pointer;
  transition: all 0.15s var(--ease-smooth);
  display: inline-flex;
  align-items: center;
  justify-content: center;
}
.confirm-btn:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}
.confirm-btn.cancel {
  background: rgba(255, 255, 255, 0.1);
  color: rgba(255, 255, 255, 0.88);
  border: 1px solid rgba(255, 255, 255, 0.16);
}
.confirm-btn.cancel:hover:not(:disabled) {
  background: rgba(255, 255, 255, 0.18);
}
.confirm-btn.danger {
  background: linear-gradient(135deg, #ff5e7e 0%, #ff3b5c 100%);
  color: #fff;
  box-shadow: 0 2px 6px rgba(255, 59, 92, 0.4);
}
.confirm-btn.danger:hover:not(:disabled) {
  transform: translateY(-1px);
  box-shadow: 0 4px 10px rgba(255, 59, 92, 0.55);
}

/* 紧凑模式：用于 80px 软件卡片 */
.confirm-inline.compact {
  gap: 4px;
  padding: 4px 6px;
  font-size: 11px;
  border-radius: 8px;
}
.confirm-inline.compact .confirm-text {
  font-size: 11px;
  max-width: 130px;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.confirm-inline.compact .confirm-btn {
  height: 20px;
  padding: 0 7px;
  font-size: 10px;
  border-radius: 5px;
}
</style>

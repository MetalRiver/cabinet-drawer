<script setup lang="ts">
import { useAppStore } from "../stores/app";

const appStore = useAppStore();
</script>

<template>
  <Teleport to="body">
    <Transition name="toast">
      <div v-if="appStore.clipToast || appStore.clipboardCountdown > 0" class="clip-overlay">
        <Transition name="toast" mode="out-in">
          <!-- 复制成功 + 倒计时 -->
          <div v-if="appStore.clipboardCountdown > 0" key="countdown" class="clip-toast clip-countdown">
            <div class="clip-icon">📋</div>
            <div class="clip-info">
              <div class="clip-title">已复制到剪贴板</div>
              <div class="clip-bar">
                <div
                  class="clip-bar-fill"
                  :style="{ width: (appStore.clipboardCountdown / 15 * 100) + '%' }"
                ></div>
              </div>
              <div class="clip-count-text">
                <span class="count-num">{{ appStore.clipboardCountdown }}</span> 秒后自动清除
              </div>
            </div>
          </div>
          <!-- 普通提示 -->
          <div v-else-if="appStore.clipToast" :key="appStore.clipToast.text" class="clip-toast">
            <div class="clip-icon">{{ appStore.clipToast.type === "success" ? "✓" : "ℹ" }}</div>
            <div class="clip-info">
              <div class="clip-title">{{ appStore.clipToast.text }}</div>
            </div>
          </div>
        </Transition>
      </div>
    </Transition>
  </Teleport>
</template>

<style scoped>
.clip-overlay {
  position: fixed;
  bottom: 24px;
  left: 50%;
  transform: translateX(-50%);
  z-index: 2000;
  pointer-events: none;
}

.clip-toast {
  background: var(--bg-glass);
  backdrop-filter: blur(20px) saturate(180%);
  -webkit-backdrop-filter: blur(20px) saturate(180%);
  border: 1px solid var(--border-bright);
  border-radius: var(--radius);
  padding: 12px 18px;
  display: flex;
  align-items: center;
  gap: 12px;
  box-shadow: var(--shadow-lg), 0 0 20px rgba(79, 124, 255, 0.3);
  min-width: 280px;
}

.clip-icon {
  width: 36px;
  height: 36px;
  background: linear-gradient(135deg, var(--accent) 0%, var(--highlight) 100%);
  border-radius: var(--radius-sm);
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: 18px;
  flex-shrink: 0;
  box-shadow: 0 4px 12px rgba(79, 124, 255, 0.4);
}

.clip-info { flex: 1; min-width: 0; }

.clip-title {
  font-size: 13px;
  font-weight: 500;
  margin-bottom: 6px;
}

.clip-bar {
  height: 3px;
  background: var(--bg-tertiary);
  border-radius: 2px;
  overflow: hidden;
  margin-bottom: 4px;
}

.clip-bar-fill {
  height: 100%;
  background: linear-gradient(90deg, var(--accent) 0%, var(--highlight) 100%);
  border-radius: 2px;
  transition: width 1s linear;
  box-shadow: 0 0 8px rgba(79, 124, 255, 0.6);
}

.clip-count-text {
  font-size: 11px;
  color: var(--text-muted);
}

.count-num {
  color: var(--highlight);
  font-weight: 600;
  font-family: monospace;
  font-size: 13px;
}

.toast-enter-active {
  animation: toastIn 0.3s var(--ease-spring);
}
.toast-leave-active {
  animation: toastOut 0.2s var(--ease-smooth);
}

@keyframes toastIn {
  from {
    opacity: 0;
    transform: translateY(20px) scale(0.9);
  }
  to {
    opacity: 1;
    transform: translateY(0) scale(1);
  }
}

@keyframes toastOut {
  to {
    opacity: 0;
    transform: translateY(20px) scale(0.95);
  }
}
</style>
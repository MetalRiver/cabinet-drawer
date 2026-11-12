<script setup lang="ts">
import { useAppStore } from "../stores/app";
import { useWidgetStore } from "../stores/widget";

const appStore = useAppStore();
const widgetStore = useWidgetStore();
</script>

<template>
  <Teleport to="body">
    <Transition name="toast">
      <!-- R6-02C-04（PL-087）：overlay 改为纵向堆叠容器——剪贴板倒计时与普通 Toast
           并行渲染、互不抑制。修复前两者经 v-if/v-else-if 互斥：倒计时 14s 窗口期内
           任何普通 Toast 被 store 接收却不可见（且 2200ms 定时器照跑，倒计时一结束
           即被清除）。修复后互不覆盖；剪贴板自动清除机制原样保留。 -->
      <div
        v-if="appStore.clipToast || appStore.clipboardCountdown > 0"
        class="clip-overlay"
      >
        <!-- 倒计时条（复制成功专用，独立渲染单元） -->
        <Transition name="toast">
          <div
            v-if="appStore.clipboardCountdown > 0"
            key="countdown"
            class="clip-toast clip-countdown"
          >
            <div class="clip-icon">📋</div>
            <div class="clip-info">
              <div class="clip-title">已复制到剪贴板</div>
              <div class="clip-bar">
                <div
                  class="clip-bar-fill"
                  :style="{ width: Math.min(100, appStore.clipboardCountdown / Math.max(1, widgetStore.clipboardClearSeconds) * 100) + '%' }"
                ></div>
              </div>
              <div class="clip-count-text">
                <span class="count-num">{{ appStore.clipboardCountdown }}</span> 秒后自动清除
              </div>
            </div>
          </div>
        </Transition>
        <!-- 普通 Toast（独立渲染单元；四级：success/info/warning/error） -->
        <Transition name="toast">
          <div
            v-if="appStore.clipToast"
            :key="appStore.clipToast.text"
            class="clip-toast"
            :class="'level-' + appStore.clipToast.type"
          >
            <div class="clip-icon" :class="appStore.clipToast.type">
              {{ appStore.clipToast.type === "success" ? "✓"
               : appStore.clipToast.type === "error"   ? "✕"
               : appStore.clipToast.type === "warning" ? "⚠"
               : "ℹ" }}
            </div>
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
  /* PL-087：倒计时条与 Toast 纵向堆叠，同时可见 */
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 8px;
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

/* R6-02C-04（PL-085）：四级等级视觉区分——图标底色 + 主体描边。
   success=蓝（原 accent 渐变）｜info=中性灰｜warning=琥珀｜error=红。
   warning/error 主体描边同色系，远距离可辨识（此前三等级主体逐字节相同）。 */
.clip-toast.level-success { border-color: rgba(79, 124, 255, 0.55); }
.clip-toast.level-info    { border-color: var(--border-bright); }
.clip-toast.level-warning { border-color: rgba(251, 191, 36, 0.6); }
.clip-toast.level-error   { border-color: rgba(248, 113, 113, 0.65); }

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

.clip-icon.info {
  background: var(--bg-tertiary);
  color: var(--text-secondary);
  box-shadow: none;
}
.clip-icon.warning {
  background: linear-gradient(135deg, #d97706 0%, #fbbf24 100%);
  color: #fff;
  box-shadow: 0 4px 12px rgba(251, 191, 36, 0.35);
}
.clip-icon.error {
  background: linear-gradient(135deg, #dc2626 0%, #f87171 100%);
  color: #fff;
  box-shadow: 0 4px 12px rgba(248, 113, 113, 0.4);
}

.clip-info { flex: 1; min-width: 0; }

.clip-title {
  font-size: 13px;
  font-weight: 500;
  margin-bottom: 6px;
}

/* error/warning 文字以等级色强调，info/success 维持正文色 */
.clip-toast.level-error .clip-title { color: #fca5a5; }
.clip-toast.level-warning .clip-title { color: #fcd34d; }

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

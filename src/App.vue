<script setup lang="ts">
import { ref, onMounted, onUnmounted } from "vue";
import { useAppStore } from "./stores/app";
import { useWidgetStore } from "./stores/widget";
import { applyWidgetConfig, setWindowSize, getAppStatus } from "./api";
import { listen } from "@tauri-apps/api/event";
import LockScreen from "./components/LockScreen.vue";
import SetupWizard from "./components/SetupWizard.vue";
import MainLayout from "./components/MainLayout.vue";
import ClipToast from "./components/ClipToast.vue";

const appStore = useAppStore();
const widgetStore = useWidgetStore();
const ready = ref(false);

// P0-#LOCK#GLOBAL#LISTENER：把"app:lock"事件监听从 LockScreen 提到 App.vue
// 原因：之前只在 LockScreen 里 listen，但用户已解锁进入 MainLayout 时
//   LockScreen 没挂载 → 后端触发"app:lock"（Alt+L / 托盘菜单）→ 事件丢失
//   → 前端 appStore.isLocked 仍是 false（界面没回退到锁屏）
//   → 用户点密码区时，后端 list_passwords 返回"应用已锁定"错误条
//   → "重试"按钮没用（永远返回同一个错误）
// 修复：在 App.vue（永远挂载）监听 app:lock
//   - 立即把 appStore.isLocked=true → 界面自动切到 LockScreen
//   - 切到 LockScreen 时它会自己再 lock 一次（幂等，安全）
//   - 清空密码区 store 缓存（防止旧数据残留）
let unlistenLock: (() => void) | null = null;
let unlistenToggleMini: (() => void) | null = null;

// P0-#LOCK#POLLING#FALLBACK：除了事件监听，再加 2s 一次的轮询兜底
// 原因：用户 build 后还是看到"应用已锁定"错误条
//   → 说明事件机制在某些路径下不可靠（可能是 webview 状态问题）
// 修复：每 2s 查询后端 lock 状态 → 如果后端 lock 但前端不知道 → 强制切回锁屏
// 代价：2s 一次 IPC，可忽略不计（< 1KB 返回）
let pollTimer: number | null = null;

// P0-#IDLE#AUTOLOCK：空闲超时自动锁定（行为偏好 → autoLockMinutes）
// 用户无操作（鼠标/键盘/触屏）超过 autoLockMinutes 分钟 → 自动锁屏
let lastActivity = Date.now();
let idleTimer: number | null = null;
function onUserActivity() {
  lastActivity = Date.now();
}
const IDLE_CHECK_MS = 30_000; // 每 30s 检测一次
const ACTIVITY_EVENTS = ["mousedown", "keydown", "touchstart", "wheel"] as const;

onMounted(async () => {
  // 注册全局锁定事件监听（必须在主流程之前，避免 race）
  try {
    unlistenLock = await listen("app:lock", () => {
      console.log("[App.vue] received app:lock event → 切换到锁屏");
      appStore.lock();
    });
  } catch (e) {
    console.warn("[App.vue] 监听 app:lock 失败", e);
  }
  // Alt+M 切换 完整/迷你模式
  try {
    unlistenToggleMini = await listen("widget:toggle-mini", () => {
      console.log("[App.vue] received widget:toggle-mini → 切换迷你模式");
      void widgetStore.toggleMiniMode();
    });
  } catch (e) {
    console.warn("[App.vue] 监听 widget:toggle-mini 失败", e);
  }
  // 先应用后端保存的窗口配置（位置/大小/置顶）
  try {
    await applyWidgetConfig();
  } catch (e) {
    console.warn("应用窗口配置失败", e);
  }
  await appStore.init();
  await widgetStore.init();
  // 若上次以迷你模式退出，启动时按迷你模式尺寸显示
  if (widgetStore.miniMode) {
    try {
      await setWindowSize(56, 320);
    } catch (e) {
      console.warn("恢复迷你模式尺寸失败", e);
    }
  }

  // P0-#LOCK#POLLING#FALLBACK：启动 lock 状态轮询
  // 2s 一次查询后端 lock 状态 → 如果后端 lock 但前端 isLocked=false → 强制切锁屏
  // 这是事件监听的兜底（防止事件丢失 / 监听器未注册时就触发锁定）
  pollTimer = window.setInterval(async () => {
    if (appStore.isFirstRun) return; // 首次设置主密码时不要干扰
    try {
      const status = await getAppStatus();
      if (!status.unlocked && !appStore.isLocked) {
        console.warn("[App.vue] polling detected backend locked → 强制切锁屏");
        appStore.lock();
      }
    } catch {
      // IPC 失败（罕见）忽略，等下个周期
    }
  }, 2000);

  // P0-#IDLE#AUTOLOCK：注册用户活动监听 + 空闲检测轮询
  lastActivity = Date.now();
  for (const ev of ACTIVITY_EVENTS) {
    window.addEventListener(ev, onUserActivity, { passive: true });
  }
  idleTimer = window.setInterval(() => {
    const minutes = widgetStore.autoLockMinutes;
    if (appStore.isFirstRun || appStore.isLocked || minutes <= 0) return;
    const idleMs = Date.now() - lastActivity;
    if (idleMs >= minutes * 60 * 1000) {
      console.log(`[App.vue] 空闲 ${(idleMs / 60000).toFixed(1)}min ≥ ${minutes}min → 自动锁屏`);
      appStore.lock();
    }
  }, IDLE_CHECK_MS);

  // 修复 P2-#28：requestAnimationFrame 套 setTimeout 50ms 等于"下一帧 + 50ms"，无实际意义
  // 直接 setTimeout 即可
  setTimeout(() => {
    console.log("[App.vue] ready = true, 渲染 MainLayout (isFirstRun=" + appStore.isFirstRun + " isLocked=" + appStore.isLocked + ")");
    ready.value = true;
  }, 50);
});

onUnmounted(() => {
  unlistenLock?.();
  unlistenLock = null;
  unlistenToggleMini?.();
  unlistenToggleMini = null;
  if (pollTimer != null) {
    window.clearInterval(pollTimer);
    pollTimer = null;
  }
  if (idleTimer != null) {
    window.clearInterval(idleTimer);
    idleTimer = null;
  }
  for (const ev of ACTIVITY_EVENTS) {
    window.removeEventListener(ev, onUserActivity);
  }
});
</script>

<template>
  <Transition name="boot" appear>
    <div v-if="ready" class="app-root">
      <Transition name="scene" mode="out-in">
        <SetupWizard v-if="appStore.isFirstRun" key="wizard" />
        <LockScreen v-else-if="appStore.isLocked" key="lock" />
        <MainLayout v-else key="main" />
      </Transition>
      <ClipToast />
    </div>
  </Transition>
</template>

<style scoped>
.app-root {
  height: 100%;
  width: 100%;
  position: relative;
  overflow: hidden;
}

/* 启动动画 - 从中心缩放进入 */
.boot-enter-active {
  animation: bootIn 0.6s var(--ease-out);
}

@keyframes bootIn {
  from {
    opacity: 0;
    transform: scale(0.92);
    filter: blur(8px);
  }
  to {
    opacity: 1;
    transform: scale(1);
    filter: blur(0);
  }
}

/* 场景切换 - 缩放 + 淡入 */
.scene-enter-active {
  animation: sceneIn 0.4s var(--ease-out);
}
.scene-leave-active {
  animation: sceneOut 0.2s var(--ease-smooth);
  position: absolute;
  inset: 0;
}

@keyframes sceneIn {
  from {
    opacity: 0;
    transform: scale(0.96) translateY(8px);
  }
  to {
    opacity: 1;
    transform: scale(1) translateY(0);
  }
}

@keyframes sceneOut {
  from { opacity: 1; }
  to { opacity: 0; transform: scale(0.98); }
}
</style>
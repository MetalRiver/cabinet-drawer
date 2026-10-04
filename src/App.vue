<script setup lang="ts">
import { ref, onMounted, onUnmounted } from "vue";
import { useAppStore } from "./stores/app";
import { useWidgetStore } from "./stores/widget";
import { applyWidgetConfig, setWindowSize, getAppStatus, getDataRootBlock, restartApp } from "./api";
import type { DataRootBlockInfo } from "./api";
import { listen } from "@tauri-apps/api/event";
import LockScreen from "./components/LockScreen.vue";
import SetupWizard from "./components/SetupWizard.vue";
import MigrationFlow from "./components/MigrationFlow.vue";
import MainLayout from "./components/MainLayout.vue";
import ClipToast from "./components/ClipToast.vue";

const appStore = useAppStore();
const widgetStore = useWidgetStore();
const ready = ref(false);

// Phase 2C-1：Data Root 启动阻断。非 null 时整页只渲染阻断视图：
// 仅提供真正可用的「重试启动」「查看错误详情」，不放任何未实现功能的假按钮。
const dataRootBlock = ref<DataRootBlockInfo | null>(null);
const showBlockDetail = ref(false);
const BLOCK_REASON_TEXT: Record<string, string> = {
  state_corrupted: "数据位置状态文件损坏，且备份恢复失败。为保护数据已停止启动。",
  state_version_unsupported: "数据位置状态文件版本过新，当前版本无法识别，请升级抽屉柜。",
  unknown_pending_operation: "存在本版本无法识别的未完成数据操作，请升级抽屉柜。",
  external_root_missing: "数据存储位置不可访问（磁盘不存在或目录已被移除）。",
  external_root_unreadable: "数据存储位置存在但无法读取（权限不足或被占用）。",
  external_db_missing: "数据存储位置存在，但未找到抽屉柜数据库。",
  external_db_invalid: "数据存储位置的数据库无效或已损坏。",
  orphan_compatibility_guard: "检测到数据位置保护标记：数据可能保存在外部位置。请勿删除该标记文件；请安装最新版本抽屉柜并通过「使用已有数据目录」重新连接。",
  state_guard_mismatch: "数据位置记录与保护标记不一致，已停止启动以保护数据。",
  pending_operation_needs_recovery: "存在未完成的数据位置操作，需要恢复流程。请安装最新版本抽屉柜。",
};
function blockText(info: DataRootBlockInfo): string {
  return BLOCK_REASON_TEXT[info.reason.reason] ?? "数据存储位置状态异常，已停止启动以保护数据。";
}

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
  // Phase 2C-1：Data Root 阻断优先判断。阻断模式下后端未打开 DB，
  // 任何业务 IPC 都不可用 → 只渲染阻断页，立即 return。
  try {
    const block = await getDataRootBlock();
    if (block) {
      dataRootBlock.value = block;
      ready.value = true;
      return;
    }
  } catch (e) {
    console.warn("[App.vue] 查询 Data Root 阻断状态失败（视为正常启动）", e);
  }
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
  <!-- Phase 2C-1：Data Root 启动阻断页（fail-visible；只有真实可用的动作） -->
  <div v-if="dataRootBlock" class="block-root">
    <div class="block-card">
      <div class="block-icon">⚠️</div>
      <h2 class="block-title">抽屉柜无法启动</h2>
      <p class="block-text">{{ blockText(dataRootBlock) }}</p>
      <div class="block-actions">
        <button class="block-btn primary" @click="restartApp()">重试启动</button>
        <button class="block-btn" @click="showBlockDetail = !showBlockDetail">
          {{ showBlockDetail ? "收起详情" : "查看错误详情" }}
        </button>
      </div>
      <div v-if="showBlockDetail" class="block-detail">
        <p><b>错误代码：</b>{{ dataRootBlock.reason.reason }}</p>
        <p><b>技术详情：</b>{{ dataRootBlock.reason.detail }}</p>
        <p><b>配置目录：</b>{{ dataRootBlock.config_root }}</p>
      </div>
    </div>
  </div>
  <Transition v-else name="boot" appear>
    <div v-if="ready" class="app-root">
      <Transition name="scene" mode="out-in">
        <SetupWizard v-if="appStore.isFirstRun" key="wizard" />
        <!-- 0.3.0 安全升级：legacy-only 启动态必须走两阶段迁移，不允许普通解锁 -->
        <MigrationFlow v-else-if="appStore.migrationRequired && appStore.isLocked" key="migration" />
        <LockScreen v-else-if="appStore.isLocked" key="lock" />
        <MainLayout v-else key="main" />
      </Transition>
      <ClipToast />
    </div>
  </Transition>
</template>

<style scoped>
/* Phase 2C-1：Data Root 阻断页样式（无外链依赖，纯本地） */
.block-root {
  height: 100%;
  width: 100%;
  display: flex;
  align-items: center;
  justify-content: center;
  background: #10141c;
  color: #e8ecf3;
  padding: 24px;
  box-sizing: border-box;
}
.block-card {
  max-width: 420px;
  background: #1a2030;
  border: 1px solid #2c3550;
  border-radius: 14px;
  padding: 28px;
  text-align: center;
}
.block-icon { font-size: 40px; margin-bottom: 8px; }
.block-title { margin: 0 0 10px; font-size: 18px; }
.block-text { margin: 0 0 18px; font-size: 13px; line-height: 1.7; color: #aab4c8; }
.block-actions { display: flex; gap: 10px; justify-content: center; }
.block-btn {
  border: 1px solid #3a4568;
  background: #232b42;
  color: #e8ecf3;
  border-radius: 8px;
  padding: 8px 16px;
  font-size: 13px;
  cursor: pointer;
}
.block-btn.primary { background: #3b6ef5; border-color: #3b6ef5; }
.block-detail {
  margin-top: 16px;
  padding: 12px;
  border-radius: 8px;
  background: #141926;
  text-align: left;
  font-size: 12px;
  color: #8a95ad;
  word-break: break-all;
}
.block-detail p { margin: 4px 0; }

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
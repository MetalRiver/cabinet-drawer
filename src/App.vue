<script setup lang="ts">
import { ref, watch, onMounted, onUnmounted } from "vue";
import { useAppStore } from "./stores/app";
import { useWidgetStore } from "./stores/widget";
import { applyWidgetConfig, exitApp, getAppStatus, getDataRootBlock, getSetupMode, lockApp, restartApp } from "./api";
import type { DataRootBlockInfo, SetupModeInfo } from "./api";
import { clearBackendKeyWithRetry } from "./lib/backend-lock-retry";
import { listen } from "@tauri-apps/api/event";
import LockScreen from "./components/LockScreen.vue";
import SetupWizard from "./components/SetupWizard.vue";
import MigrationFlow from "./components/MigrationFlow.vue";
import MainLayout from "./components/MainLayout.vue";
import ClipToast from "./components/ClipToast.vue";
import DataRootSetup from "./components/DataRootSetup.vue";

const appStore = useAppStore();
const widgetStore = useWidgetStore();
const ready = ref(false);

// Phase 2C-2：首次初始化模式（选择数据位置 / 恢复未完成初始化）。与 Blocked 互斥。
const setupMode = ref<SetupModeInfo | null>(null);

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
let unwatchIsLocked: (() => void) | null = null;

// P0-#LOCK#POLLING#FALLBACK：除了事件监听，再加 2s 一次的轮询兜底
// 原因：用户 build 后还是看到"应用已锁定"错误条
//   → 说明事件机制在某些路径下不可靠（可能是 webview 状态问题）
// 修复：每 2s 查询后端 lock 状态 → 如果后端 lock 但前端不知道 → 强制切回锁屏
// 代价：2s 一次 IPC，可忽略不计（< 1KB 返回）
let pollTimer: number | null = null;
let pollInFlight = false; // 轮询单飞标志（拥塞期不堆积挂起连接）
let lastPollOkAt = Date.now(); // 最近一次成功响应时刻（fail-closed 计时基准）

// P0-#IDLE#AUTOLOCK：空闲超时自动锁定（行为偏好 → autoLockMinutes）
// 用户无操作（鼠标/键盘/触屏）超过 autoLockMinutes 分钟 → 自动锁屏
let lastActivity = Date.now();
let idleTimer: number | null = null;
function onUserActivity() {
  lastActivity = Date.now();
}
const IDLE_CHECK_MS = 30_000; // 每 30s 检测一次
const ACTIVITY_EVENTS = ["mousedown", "keydown", "touchstart", "wheel"] as const;

// O1.3 fail-closed 锁定状态机（代次令牌）：
// Locking（UI 已遮蔽 + 后端清钥在途）→ Locked（后端确认清除）
//                                  ├→ Superseded（用户在重试窗口内解锁——放弃剩余重试，不清新密钥）
//                                  └→ Exhausted（重试耗尽）→ Restarting（安全重启）→ 兜底 ForceExit
// lockingSeq 代次递增使旧重试任务自然失效；disposed 在组件卸载（应用退出）时中止一切在途重试。
let lockingSeq = 0;
let disposed = false;

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
  // Phase 2C-2：首次初始化 / 恢复模式优先于正常流程（此时 DB 尚未打开）
  try {
    const setup = await getSetupMode();
    if (setup) {
      setupMode.value = setup;
      ready.value = true;
      return;
    }
  } catch (e) {
    console.warn("[App.vue] 查询初始化模式失败（视为正常启动）", e);
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
  // 修复「首次解锁后被空闲自动锁立即重新上锁，需输第二次」：
  // 锁屏期间空闲检测器因 isLocked==true 提前 return，lastActivity 停留在上锁前的旧值；
  // 解锁落定后第一个 tick 用陈旧 idleMs 判定 → 立即 ≥ autoLockMinutes → 误触发自动上锁。
  // 监听 isLocked true→false（解锁落定）→ 重置 lastActivity，给完整空闲窗口。
  // 用 watch 而非在 LockScreen 里改，可覆盖所有解锁路径（主密码 / 恢复短语）。
  unwatchIsLocked = watch(
    () => appStore.isLocked,
    (locked, prev) => {
      if (prev && !locked) {
        lastActivity = Date.now();
        // 关键修复：轮询 fail-closed 计时基线也要重置。
        // 锁定期间 poll 在 isLocked 检查处提前 return，lastPollOkAt 停留在上锁前的旧值；
        // 解锁后第一个 poll tick 的 fail-closed 检查（Date.now()-lastPollOkAt>30000）
        // 会因陈旧基线立即成立 → window.location.reload() → 页面重载把 Pinia 重置为
        // isLocked=true（不走 lock()），后端仍解锁 → 用户被迫二次解锁。
        // 重置 lastPollOkAt 让 fail-closed 从解锁时刻重新计时，消除误重载。
        lastPollOkAt = Date.now();
        console.log("[App.vue] 解锁落定 → 重置 lastActivity + lastPollOkAt（防 fail-closed 误重载致二次解锁）");
      }
    },
  );
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
  // v0.4.1：删除"上次以迷你模式退出则恢复迷你尺寸"分支。
  // mini_mode 已不持久化（widget.ts init 不读取），启动恒为完整模式，此分支是死代码；
  // 且其 56×320 尺寸常量与后端 64×360 不一致。

  // P0-#LOCK#POLLING#FALLBACK：启动 lock 状态轮询
  // 2s 一次查询后端 lock 状态 → 如果后端 lock 但前端 isLocked=false → 强制切锁屏
  // 这是事件监听的兜底（防止事件丢失 / 监听器未注册时就触发锁定）
  pollTimer = window.setInterval(async () => {
    if (appStore.isFirstRun) return; // 首次设置主密码时不要干扰
    // G4-2B O1（安全版本）：轮询的唯一用途是检测「后端已被锁、但前端仍解锁」。
    // 前端已明确锁定时该检测无意义——解锁由前端驱动（仅 LockScreen 调用 unlock_app，
    // 不存在无事件解锁），故锁定期间暂停状态查询；窗口可见/隐藏不做区分：
    // Rust 侧 hide 不通知前端，无法可靠识别隐藏态 → 不降频，安全标准不降。
    if (appStore.isLocked) return;
    // 2026-11-05 加固（真实冻结取证：进入窗口拥塞曾使轮询响应被堵 270s）：
    //  a) 单飞——上一轮未返回则跳过本轮，不再堆积挂起连接（此前一次拥塞可耗尽连接池，
    //     令 launch_app 等 IPC 全部排队 = "点击启动卡住"）；
    //  b) 10s 超时——超时的轮询不再等待；
    //  c) 30s 无任何成功响应 → fail-closed：重载前端回到锁屏（释放连接池，且绝不在
    //     无法确认后端状态时维持旧解锁画面）。检测语义不变（后端锁 → 前端强制锁屏），
    //     失效路径从"挂起"变"回锁屏"——这是收紧而非放宽 O1.3。
    // 失效检测（按"最后一次成功响应"计时，与 in-flight 无关——被拒/挂起/超时都算失效）
    if (Date.now() - lastPollOkAt > 30000) {
      console.warn("[App.vue] 状态轮询 30s 无成功响应 → fail-closed 重载（回锁屏）");
      window.location.reload();
      return;
    }
    if (pollInFlight) return;
    pollInFlight = true;
    try {
      const status = await Promise.race([
        getAppStatus(),
        new Promise<null>((resolve) => setTimeout(() => resolve(null), 10000)),
      ]);
      if (status == null) return; // 超时：不计成功，保持 in-flight 计时（30s 后 fail-closed）
      lastPollOkAt = Date.now();
      if (!status.unlocked && !appStore.isLocked) {
        console.warn("[App.vue] polling detected backend locked → 强制切锁屏");
        appStore.lock();
      }
    } catch {
      // IPC 失败（罕见）忽略，等下个周期
    } finally {
      pollInFlight = false;
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
      // O1.3（P0）fail-closed 锁定状态机：
      // ① Locking：同步 appStore.lock() 立即遮蔽敏感内容（明文不得因等待后端而继续显示）；
      // ② 后端清钥（lock_app=clear_key，幂等）带 3 次重试；shouldAbort 检查代次/解锁态/isUnlocking——
      //    用户在重试窗口内开始解锁 → 立即放弃剩余重试，绝不清除即将设置的新密钥；
      // ③ Locked：后端确认清除；
      // ④ Exhausted（重试耗尽且未被取代）→ fail-closed：restart_app 安全重启（销毁原进程密钥）；
      //    重启失败 → 再试一次 → 仍失败 → exit_app 强制退出兜底——旧进程绝不以可访问的
      //    解锁状态继续运行。每一步均显式记录，不静默。
      // 耗尽时若用户正在解锁（isUnlocking，KDF 验证 1–3s 期间后端新密钥已就位）→ 搁置重启，
      // 等解锁落定：成功=Superseded；失败（仍锁定）=恢复 fail-closed。
      const runFailClosed = async () => {
        if (appStore.isUnlocking) {
          console.warn("[App.vue] O1.3 fail-closed：执行前发现用户再次开始解锁 → 再次搁置");
          return;
        }
        console.error("[App.vue] O1.3 空闲自动锁：后端密钥清除失败（3 次重试均失败）→ Exhausted → fail-closed 安全重启");
        for (let restartAttempt = 1; restartAttempt <= 2; restartAttempt++) {
          try {
            await restartApp();
            console.warn(`[App.vue] O1.3 fail-closed：安全重启已发起（第 ${restartAttempt} 次）→ 进程即将退出，密钥随之销毁`);
            return;
          } catch (e) {
            console.error(`[App.vue] O1.3 fail-closed：restart_app 失败（第 ${restartAttempt}/2 次）`, e);
            if (restartAttempt < 2) await new Promise((r) => setTimeout(r, 500));
          }
        }
        console.error("[App.vue] O1.3 fail-closed：安全重启两次失败 → exit_app 强制退出兜底（旧进程绝不以解锁状态继续运行）");
        try {
          await exitApp();
        } catch (e) {
          console.error("[App.vue] O1.3 fail-closed：exit_app 兜底也失败——进程无法自行终止，需要用户/外部干预", e);
        }
      };
      appStore.lock();
      const seq = ++lockingSeq;
      void clearBackendKeyWithRetry(() => lockApp(), {
        onError: (e, attempt) => {
          console.error(`[App.vue] O1.3 空闲自动锁：lock_app 失败（第 ${attempt}/3 次）`, e);
        },
        shouldAbort: () => disposed || seq !== lockingSeq || !appStore.isLocked || appStore.isUnlocking,
      }).then((ok) => {
        if (disposed || seq !== lockingSeq) return; // 应用退出或新一轮锁定已接管
        if (ok) {
          console.log("[App.vue] O1.3 空闲自动锁：后端密钥已清除 → Locked");
          return;
        }
        if (!appStore.isLocked) {
          console.warn("[App.vue] O1.3 空闲自动锁：重试期间用户已解锁 → Superseded，放弃剩余重试（不清除新密钥）");
          return;
        }
        if (appStore.isUnlocking) {
          // 解锁在途（unlock_app 已发出、KDF 验证中，后端新密钥即将/已经就位）：
          // 搁置 fail-closed，等解锁落定——成功则 Superseded；失败（仍锁定）则恢复 fail-closed。
          console.warn("[App.vue] O1.3 空闲自动锁：重试耗尽时用户正在解锁 → 搁置 fail-closed，等待解锁落定");
          void (async () => {
            for (let i = 0; i < 100 && !disposed && seq === lockingSeq && appStore.isUnlocking; i++) {
              await new Promise((r) => setTimeout(r, 200)); // 最多等 20s（KDF + UI 切换实测 1–3s）
            }
            if (disposed || seq !== lockingSeq) return;
            if (!appStore.isLocked) {
              console.warn("[App.vue] O1.3 空闲自动锁：用户解锁成功 → Superseded（新密钥保持，不重启）");
              return;
            }
            await runFailClosed();
          })();
          return;
        }
        void runFailClosed();
      });
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
  disposed = true; // O1.3：应用退出/卸载时中止一切在途锁定重试
  unlistenLock?.();
  unlistenLock = null;
  unlistenToggleMini?.();
  unlistenToggleMini = null;
  unwatchIsLocked?.();
  unwatchIsLocked = null;
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
  <Transition v-else-if="setupMode" name="boot" appear>
    <DataRootSetup :mode="setupMode" />
  </Transition>
  <Transition v-else name="boot" appear>
    <div v-if="ready" class="app-root">
      <!-- R6-02C-06-P1-01：场景切换层加 KeepAlive——仅缓存 MainLayout。
           修复前锁屏（失焦空闲自动锁/Alt+L/托盘）时 MainLayout 从 DOM 卸载，
           其内部 KeepAlive 缓存随之销毁，TempContentView/PasswordView/SnippetView
           组件实例销毁 → 编辑草稿（detailDraft/form/editForm）静默丢失。
           缓存 MainLayout 后：锁屏→解锁走 onDeactivated/onActivated，
           组件实例与编辑草稿保留；密码区 onDeactivated 收起明文的保护仍然生效。
           SetupWizard/MigrationFlow/LockScreen 不在缓存内，正常销毁（一次性界面）。 -->
      <Transition name="scene" mode="out-in">
        <KeepAlive :include="['MainLayout']">
          <SetupWizard v-if="appStore.isFirstRun" key="wizard" />
          <!-- 0.3.0 安全升级：legacy-only 启动态必须走两阶段迁移，不允许普通解锁 -->
          <MigrationFlow v-else-if="appStore.migrationRequired && appStore.isLocked" key="migration" />
          <LockScreen v-else-if="appStore.isLocked" key="lock" />
          <MainLayout v-else key="main" />
        </KeepAlive>
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
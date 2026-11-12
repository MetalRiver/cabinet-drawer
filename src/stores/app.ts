import { defineStore } from "pinia";
import { ref } from "vue";
import {
  isFirstRun as apiIsFirstRun,
  getSecurityStatus,
  trashCount as apiTrashCount,
} from "../api";

export const useAppStore = defineStore("app", () => {
  const isLocked = ref(true);
  // fail closed：只有后端明确确认 FreshV2 才进入初始化向导。
  // IPC 失败或 legacy/v2 已存在时都不得默认创建新密码库。
  const isFirstRun = ref(false);
  // 0.3.0 安全升级：legacy-only 启动态必须走两阶段迁移流程，
  // 不允许普通解锁进入 legacy 工作模式。fail closed：仅当后端明确
  // 返回 legacy_security_model + migration_required 时才置 true。
  const migrationRequired = ref(false);
  const currentView = ref("passwords");
  const searchQuery = ref("");
  const showSummaryModal = ref(false);
  const clipboardCountdown = ref(0); // 剪贴板倒计时（秒），0=未触发
  // 修复 P1-#16：恢复短语不再放到全局 store（之前 DevTools 可读、关掉 LockScreen 不清），
  // 改为 LockScreen 组件内部 ref，组件 unmount 时自动 GC。
  // P0-#T：回收站计数（全局共享，SearchBar 徽章用）
  const trashCount = ref(0);

  // 启动时初始化（异步）
  async function init() {
    try {
      isFirstRun.value = await apiIsFirstRun();
    } catch (e) {
      isFirstRun.value = false;
      console.error("检查首次启动失败", e);
    }
    try {
      const status = await getSecurityStatus();
      migrationRequired.value =
        status.security_model === "legacy_security_model" && status.migration_required;
    } catch (e) {
      // fail closed：查询失败不开启迁移流程（此时普通解锁也被后端硬闸拒绝）
      migrationRequired.value = false;
      console.error("检查安全状态失败", e);
    }
  }

  // 两阶段迁移 confirm 成功后由 MigrationFlow 调用
  function completeMigration() {
    migrationRequired.value = false;
  }

  async function refreshTrashCount() {
    try {
      trashCount.value = await apiTrashCount();
    } catch (e) {
      // 静默
    }
  }

  function lock() {
    isLocked.value = true;
    clipClearAll(); // PL-086：锁屏不残留过期反馈
  }

  function unlock() {
    isLocked.value = false;
  }

  // O1.3：解锁进行中信号（beginUnlock 在 unlock_app 发出前置位，endUnlock 在成功/失败落定后复位）。
  // 空闲自动锁的重试中止与耗尽判定必须以此为准——前端 isLocked 翻转要等 KDF 验证 + UI 切换
  // （实测 1–3s），期间后端新密钥已就位；仅看 isLocked 会让旧重试清掉新密钥、
  // 甚至把刚完成的解锁会话 fail-closed 重启掉。
  const isUnlocking = ref(false);
  function beginUnlock() {
    isUnlocking.value = true;
  }
  function endUnlock() {
    isUnlocking.value = false;
  }

  function setFirstRun(v: boolean) {
    isFirstRun.value = v;
  }

  // R6-02C-04（PL-085）：四级反馈体系——success=成功 / info=中性 / warning=部分成功或
  // 非致命异常 / error=操作失败。warning/error 展示 3500ms（阅读失败原因需要更久），
  // 其余维持 2200ms；显式 durationMs 优先。
  // R6-02C-05（PL-086）：单条展示 + 有界队列（上限 3）——新消息排队而非覆盖；
  // 同文本 1s 内重复触发防抖；队列满时仅丢弃队尾低级（info/success）消息，
  // warning/error 永不被静默覆盖；锁屏清空队列（避免锁屏残留过期反馈）。
  type ToastItem = { type: "success" | "info" | "warning" | "error"; text: string; enqueuedAt: number };
  const clipQueue = ref<ToastItem[]>([]);
  let clipShowing = false;
  let clipTimer: ReturnType<typeof setTimeout> | null = null;
  const clipToast = ref<{ type: ToastItem["type"]; text: string; enqueuedAt?: number } | null>(null);

  function clipTtl(type: ToastItem["type"], explicit?: number) {
    return explicit ?? (type === "warning" || type === "error" ? 3500 : 2200);
  }

  function clipDrain() {
    if (clipShowing || !clipQueue.value.length) return;
    const head = clipQueue.value[0];
    clipToast.value = { type: head.type, text: head.text, enqueuedAt: head.enqueuedAt };
    clipShowing = true;
    clipTimer = setTimeout(() => {
      clipQueue.value.shift();
      clipShowing = false;
      clipTimer = null;
      clipDrain();
    }, clipTtl(head.type, (head as { durationMs?: number }).durationMs));
  }

  function showClipToast(
    type: "success" | "info" | "warning" | "error",
    text: string,
    durationMs?: number,
  ) {
    const now = Date.now();
    // 防抖：同文本 1s 内重复触发 → 忽略（当前显示中的队头仍在队列内，一并覆盖）
    const dup = clipQueue.value.find((q) => q.text === text && now - q.enqueuedAt < 1000);
    if (dup) return;
    // 容量上限 3：满时从队尾找低级（info/success）丢弃；全是高级则丢最旧
    if (clipQueue.value.length >= 3) {
      let dropIdx = -1;
      for (let i = clipQueue.value.length - 1; i >= 1; i--) {
        if (clipQueue.value[i].type === "info" || clipQueue.value[i].type === "success") { dropIdx = i; break; }
      }
      clipQueue.value.splice(dropIdx >= 0 ? dropIdx : 0, 1);
    }
    const item: ToastItem = { type, text, enqueuedAt: now };
    if (durationMs !== undefined) (item as { durationMs?: number }).durationMs = durationMs;
    clipQueue.value.push(item);
    clipDrain();
  }

  function clipClearAll() {
    clipQueue.value = [];
    clipToast.value = null;
    clipShowing = false;
    if (clipTimer) { clearTimeout(clipTimer); clipTimer = null; }
  }

  return {
    isLocked,
    isFirstRun,
    migrationRequired,
    currentView,
    searchQuery,
    showSummaryModal,
    clipboardCountdown,
    clipToast,
    trashCount,
    init,
    lock,
    unlock,
    isUnlocking,
    beginUnlock,
    endUnlock,
    setFirstRun,
    completeMigration,
    showClipToast,
    refreshTrashCount,
  };
});

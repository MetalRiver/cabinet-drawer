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
  const clipToast = ref<{ type: "success" | "info"; text: string } | null>(null);
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
  }

  function unlock() {
    isLocked.value = false;
  }

  function setFirstRun(v: boolean) {
    isFirstRun.value = v;
  }

  function showClipToast(type: "success" | "info", text: string) {
    clipToast.value = { type, text };
    setTimeout(() => {
      clipToast.value = null;
    }, 2200);
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
    setFirstRun,
    completeMigration,
    showClipToast,
    refreshTrashCount,
  };
});

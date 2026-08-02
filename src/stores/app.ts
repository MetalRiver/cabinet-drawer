import { defineStore } from "pinia";
import { ref } from "vue";
import { isFirstRun as apiIsFirstRun, trashCount as apiTrashCount } from "../api";

export const useAppStore = defineStore("app", () => {
  const isLocked = ref(true);
  const isFirstRun = ref(true);
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
      console.error("检查首次启动失败", e);
    }
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
    showClipToast,
    refreshTrashCount,
  };
});
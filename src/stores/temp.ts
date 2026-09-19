import { defineStore } from "pinia";
import { ref, computed } from "vue";
import { useAppStore } from "./app";
import { useWidgetStore } from "./widget";
import {
  listTemp as apiListTemp,
  createTemp as apiCreateTemp,
  deleteTemp as apiDeleteTemp,
  cleanupExpiredTemp as apiCleanupExpired,
  copyToClipboardWithTimeout,
  type TempMeta,
} from "../api";

export const useTempStore = defineStore("temp", () => {
  const items = ref<TempMeta[]>([]);
  const loading = ref(false);
  const error = ref("");

  // 修复 P2-#26：删除 store 端 10s 定时器（`[...items.value]` 啥也没刷，是无意义 setInterval）。
  // 剩余时间倒计时由 view 端 1s 的 `now` ref 驱动即可。

  async function loadItems() {
    loading.value = true;
    error.value = "";
    try {
      items.value = await apiListTemp();
    } catch (e) {
      error.value = String(e);
    } finally {
      loading.value = false;
    }
  }

  async function create(text: string, ttlMinutes: number) {
    await apiCreateTemp(text, ttlMinutes);
    await loadItems();
  }

  async function remove(id: number) {
    await apiDeleteTemp(id);
    await loadItems();
    useAppStore().refreshTrashCount();
  }

  async function cleanupExpired() {
    const n = await apiCleanupExpired();
    if (n > 0) {
      await loadItems();
    }
    return n;
  }

  /** 复制临时内容到剪贴板（清除时长=设置页唯一事实源） */
  async function copy(text: string) {
    const widgetStore = useWidgetStore();
    await copyToClipboardWithTimeout(text, widgetStore.clipboardClearSeconds);
  }

  /** 把剩余时间格式化为 mm:ss */
  function remaining(item: TempMeta): string {
    const ms = item.expires_at - Date.now();
    if (ms <= 0) return "已过期";
    const sec = Math.floor(ms / 1000);
    const h = Math.floor(sec / 3600);
    const m = Math.floor((sec % 3600) / 60);
    const s = sec % 60;
    if (h > 0) return `${h}h${m}m`;
    if (m > 0) return `${m}m${s}s`;
    return `${s}s`;
  }

  // P0-#Z#6：底部栏红点 - 倒计时 ≤ 24h 的便签数量
  const expiringSoonCount = computed(() => {
    const now = Date.now();
    return items.value.filter((it) => {
      const ms = it.expires_at - now;
      return ms > 0 && ms <= 24 * 60 * 60 * 1000;
    }).length;
  });

  return {
    items,
    loading,
    error,
    loadItems,
    // 导入备份后整体刷新（调用方：SettingsView 导入完成后）
    reloadAll: loadItems,
    create,
    remove,
    cleanupExpired,
    copy,
    remaining,
    expiringSoonCount,
  };
});

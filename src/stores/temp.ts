import { defineStore } from "pinia";
import { ref, computed } from "vue";
import { useAppStore } from "./app";
import { useWidgetStore } from "./widget";
import {
  listTemp as apiListTemp,
  listTempCategories as apiListTempCategories,
  createTempCategory as apiCreateTempCategory,
  renameTempCategory as apiRenameTempCategory,
  deleteTempCategory as apiDeleteTempCategory,
  setTempCategory as apiSetTempCategory,
  updateTempTitle as apiUpdateTempTitle,
  createTemp as apiCreateTemp,
  deleteTemp as apiDeleteTemp,
  cleanupExpiredTemp as apiCleanupExpired,
  updateTempText as apiUpdateTempText,
  renewTemp as apiRenewTemp,
  isTempPermanent,
  copyToClipboardWithTimeout,
  type TempMeta,
  type TempCategory,
} from "../api";

export const useTempStore = defineStore("temp", () => {
  const items = ref<TempMeta[]>([]);
  const loading = ref(false);
  const error = ref("");

  // 修复 P2-#26：删除 store 端 10s 定时器（`[...items.value]` 啥也没刷，是无意义 setInterval）。
  // 剩余时间倒计时由 view 端 1s 的 `now` ref 驱动即可。

  // ===== Batch 4：分类与搜索 =====
  const categories = ref<TempCategory[]>([]);
  const activeCategoryId = ref<number | null>(null); // null = 未分类筛选关（全部）
  const showUncategorized = ref(false); // 「未分类」筛选
  const searchQuery = ref("");

  async function loadCategories() {
    try {
      categories.value = await apiListTempCategories();
    } catch (e) {
      console.warn("加载便签分类失败", e);
    }
  }

  async function createCategory(name: string) {
    await apiCreateTempCategory(name);
    await loadCategories();
  }
  async function renameCategory(id: number, name: string) {
    await apiRenameTempCategory(id, name);
    await loadCategories();
  }
  /** 删除分类：便签安全回到未分类（后端事务保证），分类列表刷新 */
  async function deleteCategory(id: number) {
    await apiDeleteTempCategory(id);
    if (activeCategoryId.value === id) activeCategoryId.value = null;
    await loadCategories();
    await loadItems();
  }
  async function setCategory(id: number, categoryId: number | null) {
    await apiSetTempCategory(id, categoryId);
    await loadItems();
  }
  async function updateTitle(id: number, title: string | null) {
    await apiUpdateTempTitle(id, title);
    await loadItems();
  }

  async function create(text: string, ttlMinutes: number, title?: string) {
    await apiCreateTemp(text, ttlMinutes, title);
    await loadItems();
  }

  /** 客户端过滤：分类筛选 + 标题/正文搜索 */
  const filteredItems = computed(() => {
    let list = items.value;
    if (showUncategorized.value) {
      list = list.filter((it) => !it.category_id);
    } else if (activeCategoryId.value != null) {
      list = list.filter((it) => it.category_id === activeCategoryId.value);
    }
    const q = searchQuery.value.trim().toLowerCase();
    if (!q) return list;
    return list.filter(
      (it) =>
        it.text.toLowerCase().includes(q) ||
        (it.title || "").toLowerCase().includes(q)
    );
  });

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

  /** v0.4.1：编辑/追加便签内容（后端只改 text，到期时间保持不变） */
  async function updateText(id: number, text: string) {
    await apiUpdateTempText(id, text);
    await loadItems();
  }

  /**
   * v0.4.1：续期。
   * - addMs：在现有到期时间上加毫秒（1/3/7/30 天预设）
   * - setExpiresAt：直接设定绝对到期时间（TEMP_NEVER_EXPIRES_MS = 永不过期）
   * 返回后端给出的新到期时间。
   */
  async function renew(id: number, addMs: number | null, setExpiresAt: number | null) {
    const newExpiresAt = await apiRenewTemp(id, addMs, setExpiresAt);
    await loadItems();
    return newExpiresAt;
  }

  /** 是否永久便签（哨兵值） */
  function isPermanent(expiresAt: number): boolean {
    return isTempPermanent(expiresAt);
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

  /** 把剩余时间格式化为 mm:ss（永久便签返回"永久"） */
  function remaining(item: TempMeta): string {
    if (isTempPermanent(item.expires_at)) return "永久";
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
    categories,
    activeCategoryId,
    showUncategorized,
    searchQuery,
    filteredItems,
    loadCategories,
    createCategory,
    renameCategory,
    deleteCategory,
    setCategory,
    updateTitle,
    loadItems,
    // 导入备份后整体刷新（调用方：SettingsView 导入完成后）
    reloadAll: loadItems,
    create,
    updateText,
    renew,
    isPermanent,
    remove,
    cleanupExpired,
    copy,
    remaining,
    expiringSoonCount,
  };
});

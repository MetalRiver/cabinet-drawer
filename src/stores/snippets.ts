import { defineStore } from "pinia";
import { ref, computed } from "vue";
import {
  listSnippets as apiListSnippets,
  createSnippet as apiCreateSnippet,
  updateSnippet as apiUpdateSnippet,
  deleteSnippet as apiDeleteSnippet,
  recordSnippetUsage as apiRecordSnippetUsage,
  copyToClipboardWithTimeout,
  type SnippetMeta,
} from "../api";
import { useAppStore } from "./app";
import { useWidgetStore } from "./widget";
import { useClipboardCountdown } from "../composables/useClipboardCountdown";

export const useSnippetsStore = defineStore("snippets", () => {
  const appStore = useAppStore();
  const widgetStore = useWidgetStore();
  const clipboardCountdown = useClipboardCountdown();
  const items = ref<SnippetMeta[]>([]);
  const loading = ref(false);
  const error = ref("");
  // 当前激活的语言筛选，null = 全部
  const activeLanguage = ref<string | null>(null);
  const hasLoaded = ref(false);

  async function loadItems() {
    loading.value = true;
    error.value = "";
    try {
      // P1（搜索修复）：全量拉取 + 客户端过滤。
      // 之前把当时的 searchQuery 传给后端 LIKE——加载后清空搜索框列表也不会恢复（陈旧查询污染）。
      items.value = await apiListSnippets();
      hasLoaded.value = true;
    } catch (e) {
      error.value = String(e);
    } finally {
      loading.value = false;
    }
  }

  function setLanguage(lang: string | null) {
    activeLanguage.value = lang;
  }

  // 客户端过滤（P1 搜索修复）：搜索（标题/内容/标签，不区分大小写）与语言筛选叠加
  const filteredItems = computed(() => {
    const q = appStore.searchQuery.trim().toLowerCase();
    let result = items.value;
    if (q) {
      result = result.filter(
        (s) =>
          s.title.toLowerCase().includes(q) ||
          s.content.toLowerCase().includes(q) ||
          (s.tags || "").toLowerCase().includes(q)
      );
    }
    if (!activeLanguage.value) return result;
    if (activeLanguage.value === "uncategorized") {
      // 未分类：language 为空 / null / "text"（text 是默认值）
      return result.filter(
        (s) => !s.language || !s.language.trim() || s.language === "text"
      );
    }
    return result.filter(
      (s) => (s.language || "text") === activeLanguage.value
    );
  });

  async function create(params: {
    title: string;
    content: string;
    language?: string;
    tags?: string;
  }) {
    await apiCreateSnippet(params);
    await loadItems();
  }

  async function update(params: {
    id: number;
    title: string;
    content: string;
    language?: string;
    tags?: string;
  }) {
    await apiUpdateSnippet(params);
    await loadItems();
  }

  async function remove(id: number) {
    await apiDeleteSnippet(id);
    await loadItems();
    useAppStore().refreshTrashCount();
  }

  /** 复制片段到剪贴板（清除时长=设置页唯一事实源 widget.clipboard_clear_seconds） */
  async function copy(id: number) {
    const item = items.value.find((i) => i.id === id);
    if (!item) return;
    const secs = widgetStore.clipboardClearSeconds;
    await copyToClipboardWithTimeout(item.content, secs);
    await apiRecordSnippetUsage(id);
    // 修复 P1-#19：使用统一 composable，与 PasswordView 共享一个 timer
    clipboardCountdown.start(secs);
  }

  return {
    items,
    filteredItems,
    loading,
    error,
    hasLoaded,
    activeLanguage,
    loadItems,
    // 导入备份后整体刷新（调用方：SettingsView 导入完成后）
    reloadAll: loadItems,
    create,
    update,
    remove,
    copy,
    setLanguage,
  };
});

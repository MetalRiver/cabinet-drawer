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
import { useClipboardCountdown } from "../composables/useClipboardCountdown";

export const useSnippetsStore = defineStore("snippets", () => {
  const appStore = useAppStore();
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
      items.value = await apiListSnippets(appStore.searchQuery);
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

  // 客户端按语言过滤（不再触发 DB 查询）
  // P0-#Y#FIX#UNCAT：activeLanguage="uncategorized" 命中 language 为空或 "text" 的默认项
  const filteredItems = computed(() => {
    if (!activeLanguage.value) return items.value;
    if (activeLanguage.value === "uncategorized") {
      // 未分类：language 为空 / null / "text"（text 是默认值）
      return items.value.filter(
        (s) => !s.language || !s.language.trim() || s.language === "text"
      );
    }
    return items.value.filter(
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

  /** 复制片段到剪贴板，带自动清空倒计时（统一由 useClipboardCountdown 管理） */
  async function copy(id: number, timeoutSecs = 30) {
    const item = items.value.find((i) => i.id === id);
    if (!item) return;
    await copyToClipboardWithTimeout(item.content, timeoutSecs);
    await apiRecordSnippetUsage(id);
    // 修复 P1-#19：使用统一 composable，与 PasswordView 共享一个 timer
    clipboardCountdown.start(timeoutSecs);
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

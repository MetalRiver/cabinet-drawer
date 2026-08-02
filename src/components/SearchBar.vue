<script setup lang="ts">
import { useAppStore } from "../stores/app";
import { computed } from "vue";

const appStore = useAppStore();
const query = computed({
  get: () => appStore.searchQuery,
  set: (v) => { appStore.searchQuery = v; },
});
</script>

<template>
  <!--
    P0-T#FIX：回收站按钮已从 search-box 内部移除
    现在由 MainLayout 顶部的 .trash-btn-fixed 承担（搜索框外、最右面、自适应）
    这里只保留纯搜索框，input 宽度由父容器 flex 决定（占满剩余空间）
  -->
  <div class="search-box" :class="{ active: query.length > 0 }">
    <span class="search-icon">🔍</span>
    <input
      v-model="query"
      type="text"
      class="search-input"
      placeholder="搜索密码 / 软件 / 命令行 / 临时..."
    />
    <button
      v-if="query.length > 0"
      class="search-clear tap"
      title="清空"
      @click="query = ''"
    >×</button>
    <span v-else class="search-shortcut">
      <span class="kbd">Ctrl</span>
      <span class="kbd">K</span>
    </span>
  </div>
</template>

<style scoped>
.search-box {
  background: var(--bg-primary);
  border: 1px solid var(--border);
  border-radius: var(--radius);
  padding: 12px 16px;
  display: flex;
  align-items: center;
  /* 父容器 .top-header-inner 是 flex:1（搜索框） + auto（回收站），
     搜索框自身用 width:100% 占满父分配给它的空间 */
  width: 100%;
  min-width: 0; /* 关键：允许在窄窗口下被压缩而不是撑爆布局 */
  transition: all 0.3s var(--ease-smooth);
  position: relative;
}

.search-box:hover {
  border-color: rgba(79, 124, 255, 0.3);
  background: var(--bg-secondary);
}

.search-box:focus-within,
.search-box.active {
  border-color: var(--accent);
  background: var(--bg-secondary);
  box-shadow:
    0 0 0 3px var(--accent-soft),
    0 4px 20px rgba(79, 124, 255, 0.15);
  transform: translateY(-1px);
}

.search-icon {
  color: var(--text-muted);
  margin-right: 12px;
  font-size: 16px;
  transition: color 0.2s;
  flex-shrink: 0;
}

.search-box:focus-within .search-icon,
.search-box.active .search-icon {
  color: var(--accent);
}

.search-input {
  flex: 1 1 0;
  min-width: 60px; /* 缩到极窄也至少能输入 */
  background: transparent;
  border: none;
  outline: none;
  color: var(--text-primary);
  font-size: 14px;
}

.search-input::placeholder {
  color: var(--text-muted);
  transition: color 0.2s;
}

.search-box:focus-within .search-input::placeholder {
  color: var(--text-faint);
}

.search-clear {
  width: 22px;
  height: 22px;
  border: none;
  background: var(--bg-tertiary);
  color: var(--text-muted);
  font-size: 16px;
  border-radius: 50%;
  cursor: pointer;
  display: flex;
  align-items: center;
  justify-content: center;
  line-height: 1;
  padding: 0;
  transition: all 0.15s;
  -webkit-app-region: no-drag;
  flex-shrink: 0;
}
.search-clear:hover {
  background: var(--highlight);
  color: white;
  transform: scale(1.1);
}

.search-shortcut {
  display: flex;
  gap: 3px;
  opacity: 0.5;
  transition: opacity 0.2s;
  flex-shrink: 0;
}

.search-box:focus-within .search-shortcut {
  opacity: 0;
}

.kbd {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  min-width: 20px;
  height: 20px;
  padding: 0 5px;
  background: var(--bg-tertiary);
  border: 1px solid var(--border);
  border-radius: 4px;
  font-size: 10px;
  font-family: monospace;
  color: var(--text-muted);
}
</style>

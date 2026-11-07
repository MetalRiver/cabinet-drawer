<script setup lang="ts">
import { ref, onMounted, onUnmounted, watch, computed } from "vue";
import { useAppStore } from "../stores/app";
import { useSnippetsStore } from "../stores/snippets";
import ConfirmInline from "../components/ConfirmInline.vue";
import DarkSelect from "../components/DarkSelect.vue";

const appStore = useAppStore();
const snippetsStore = useSnippetsStore();

// P0-#Z：行内 confirm（统一 5 区风格）
const confirmingDeleteId = ref<number | null>(null);
const deletingId = ref<number | null>(null);

// 编辑/添加
const showForm = ref(false);
const isEdit = ref(false);
const form = ref({
  id: null as number | null,
  title: "",
  content: "",
  language: "text",
  tags: "",
});

// 左侧 sidebar 展开状态（默认收起，hover 展开）
// ===== 语言分组迁移：右上角「筛选」浮层面板（替代左侧分组栏）=====
const filterOpen = ref(false);
const filterPos = ref<Record<string, string>>({});
const filterBtnEl = ref<HTMLElement | null>(null);
const activeFilterCount = computed(() => (snippetsStore.activeLanguage ? 1 : 0));
function toggleFilter() {
  if (filterOpen.value) { filterOpen.value = false; return; }
  const el = filterBtnEl.value;
  if (el) {
    const r = el.getBoundingClientRect();
    const width = 220;
    const left = Math.max(8, Math.min(r.right - width, window.innerWidth - width - 8));
    filterPos.value = { position: "fixed", top: Math.round(r.bottom + 6) + "px", left: Math.round(left) + "px", width: width + "px" };
  }
  filterOpen.value = true;
}
function clearFilters() {
  snippetsStore.setLanguage(null);
}
function onFilterOutside(e: MouseEvent) {
  const t = e.target as HTMLElement | null;
  if (!t) return;
  if (t.closest(".filter-panel") || t.closest(".filter-btn")) return;
  filterOpen.value = false;
}
function onFilterEsc(e: KeyboardEvent) {
  if (e.key === "Escape") filterOpen.value = false;
}
watch(filterOpen, (open: boolean) => {
  if (open) {
    document.addEventListener("mousedown", onFilterOutside, true);
    document.addEventListener("keydown", onFilterEsc);
  } else {
    document.removeEventListener("mousedown", onFilterOutside, true);
    document.removeEventListener("keydown", onFilterEsc);
  }
});
onUnmounted(() => {
  document.removeEventListener("mousedown", onFilterOutside, true);
  document.removeEventListener("keydown", onFilterEsc);
});

// 语言分类：emoji + label
// P0-#Y#FIX#UNCAT#REMOVE：删除"未分类"分类
//  原因：用户要求"人工二次修改即可，不需要未分类分组"
//  之前：📦 未分类 chip 单独占空间
//  现在：language 为空/null 的 snippet 不在任何 chip 下，只在 ✨ 全部 看到
//       用户右键 → 修改 language 即可
const LANG_OPTIONS: { key: string | null; label: string; emoji: string; color: string }[] = [
  { key: null, label: "全部", emoji: "✨", color: "#94a3b8" },
  { key: "text", label: "纯文本", emoji: "📝", color: "#94a3b8" },
  { key: "bash", label: "Bash", emoji: "🐚", color: "#10b981" },
  { key: "javascript", label: "JavaScript", emoji: "🟨", color: "#facc15" },
  { key: "typescript", label: "TypeScript", emoji: "🔷", color: "#3b82f6" },
  { key: "python", label: "Python", emoji: "🐍", color: "#3b82f6" },
  { key: "rust", label: "Rust", emoji: "🦀", color: "#f97316" },
  { key: "vue", label: "Vue", emoji: "💚", color: "#22c55e" },
  { key: "sql", label: "SQL", emoji: "🗄️", color: "#a855f7" },
];

onMounted(() => {
  snippetsStore.loadItems();
});

// 关键修复：去掉 watch(searchQuery) → loadItems
// 之前每次按键都触发后端 DB 查询，导致列表项被整个替换 → 闪屏
// 现在搜索过滤完全在客户端 computed 完成

function openAdd() {
  isEdit.value = false;
  form.value = { id: null, title: "", content: "", language: "text", tags: "" };
  showForm.value = true;
}

/** 语言 code → 中文标签（带简短解释） */
const languageOptions = [
  { value: "text", label: "纯文本" },
  { value: "bash", label: "Bash（终端脚本）" },
  { value: "javascript", label: "JavaScript（前端脚本）" },
  { value: "typescript", label: "TypeScript（带类型 JS）" },
  { value: "python", label: "Python（脚本）" },
  { value: "rust", label: "Rust（系统级语言）" },
  { value: "vue", label: "Vue（前端框架）" },
  { value: "sql", label: "SQL（数据库查询）" },
];

function languageLabel(code: string): string {
  const map: Record<string, string> = {
    bash: "Bash（终端脚本）",
    javascript: "JavaScript（前端脚本）",
    typescript: "TypeScript（带类型 JS）",
    python: "Python（蟒蛇脚本）",
    rust: "Rust（系统级语言）",
    vue: "Vue（前端框架）",
    sql: "SQL（数据库查询）",
  };
  return map[code] || code;
}

function openEdit(item: any) {
  isEdit.value = true;
  form.value = {
    id: item.id,
    title: item.title,
    content: item.content,
    language: item.language || "text",
    tags: item.tags || "",
  };
  showForm.value = true;
}

async function submit() {
  if (!form.value.title || !form.value.content) {
    appStore.showClipToast("info", "请填写标题和内容");
    return;
  }
  try {
    if (isEdit.value && form.value.id != null) {
      await snippetsStore.update({
        id: form.value.id,
        title: form.value.title,
        content: form.value.content,
        language: form.value.language,
        tags: form.value.tags,
      });
    } else {
      await snippetsStore.create({
        title: form.value.title,
        content: form.value.content,
        language: form.value.language,
        tags: form.value.tags,
      });
    }
    showForm.value = false;
    appStore.showClipToast("success", isEdit.value ? "已保存" : "已添加");
  } catch (e) {
    appStore.showClipToast("info", String(e));
  }
}

async function copy(item: any) {
  try {
    await snippetsStore.copy(item.id);
    appStore.showClipToast("success", "已复制");
  } catch (e) {
    appStore.showClipToast("info", String(e));
  }
}

function askRemove(id: number) {
  confirmingDeleteId.value = id;
}

function cancelRemove() {
  confirmingDeleteId.value = null;
}

async function confirmRemove(id: number) {
  deletingId.value = id;
  try {
    await snippetsStore.remove(id);
    appStore.showClipToast("success", "已删除");
  } catch (e) {
    appStore.showClipToast("info", String(e));
  } finally {
    deletingId.value = null;
    confirmingDeleteId.value = null;
  }
}
</script>

<template>
  <div class="view">
    <div class="view-header">
      <div class="view-header-left">
        <div class="view-icon icon-cyan">💻</div>
        <div class="view-header-info">
          <h2 class="view-title">
            <span class="view-title-text">命令行</span>
            <span class="title-count">{{ snippetsStore.filteredItems.length }}</span>
          </h2>
        </div>
      </div>
      <div class="view-header-actions">
        <!-- 2026-11-05：语言分组迁移到右上角「筛选」按钮（原左侧分组栏移除） -->
        <button
          ref="filterBtnEl"
          class="filter-btn tap"
          :class="{ active: filterOpen || activeFilterCount > 0 }"
          title="筛选语言分组"
          @click.stop="toggleFilter"
        >
          <span class="filter-btn-icon">☰</span>
          <span class="filter-btn-label">筛选</span>
          <span v-if="activeFilterCount > 0" class="filter-btn-count">{{ activeFilterCount }}</span>
        </button>
        <button class="action-btn action-btn-primary tap" title="添加命令" @click="openAdd">+</button>
      </div>
    </div>

    <!-- 主体：左 sidebar + 右列表 -->
    
        <!-- 空状态 -->
        <div v-if="snippetsStore.filteredItems.length === 0 && !snippetsStore.loading" class="empty">
          <div class="empty-icon">💻</div>
          <div class="empty-text">
            <template v-if="appStore.searchQuery">没有匹配的命令</template>
            <template v-else-if="snippetsStore.activeLanguage">该语言下还没有命令</template>
            <template v-else>还没有命令，点 + 添加</template>
          </div>
        </div>

        <TransitionGroup name="list" tag="div" class="list-container">
          <div
            v-for="(item, index) in snippetsStore.filteredItems"
            :key="item.id"
            class="list-item stagger-item"
            :style="{ '--delay': `${index * 0.05}s` }"
          >
            <div class="item-main">
              <div :class="['item-icon', item.language && item.language !== 'text' ? 'icon-cyan' : 'icon-gray']">
                <span>{{ item.language && item.language !== 'text' ? '💡' : '📋' }}</span>
                <div class="item-icon-glow"></div>
              </div>
              <div class="item-info">
                <!-- P0-#SNIP#TAG#RIGHT：lang-tag 从副标搬到标题正右侧（紧凑、不与内容混在一起，字号更显眼） -->
                <div class="item-title-row">
                  <span class="item-title">{{ item.title }}</span>
                  <span v-if="item.language && item.language !== 'text'" class="lang-tag">{{ languageLabel(item.language) }}</span>
                </div>
                <div class="item-subtitle">
                  <span>{{ item.content.split('\n').length }} 行</span>
                  <span class="subtitle-preview">{{ item.content.split('\n')[0] }}</span>
                </div>
              </div>
            </div>
            <div v-if="confirmingDeleteId !== item.id" class="item-actions">
              <span v-if="item.use_count > 0" class="item-flame" :title="`已使用 ${item.use_count} 次`">🔥 {{ item.use_count }}</span>
              <button class="action-btn" title="复制" @click="copy(item)">📋</button>
              <button class="action-btn" title="编辑" @click="openEdit(item)">✏️</button>
              <button class="action-btn" title="删除" @click="askRemove(item.id)">🗑️</button>
            </div>
            <div v-else class="item-confirm-wrap">
              <ConfirmInline
                :title="`确定删除「${item.title}」？`"
                :loading="deletingId === item.id"
                @cancel="cancelRemove"
                @confirm="confirmRemove(item.id)"
              />
            </div>
          </div>
        

        </TransitionGroup>

    <!-- 表单 -->
    <Transition name="modal">
      <div v-if="showForm" class="modal-mask" @click.self="showForm = false">
        <div class="modal">
          <div class="modal-header">
            <span>{{ isEdit ? "编辑命令" : "添加命令" }}</span>
            <button class="modal-close tap" @click="showForm = false">×</button>
          </div>
          <div class="modal-body">
            <div class="form-row">
              <label>标题</label>
              <input v-model="form.title" class="form-input" placeholder="如：Docker 常用命令" />
            </div>
            <div class="form-row">
              <label>语言（可选）</label>
              <DarkSelect
                v-model="form.language"
                :options="languageOptions"
                aria-label="语言（可选）"
                block
              />
            </div>
            <div class="form-row">
              <label>内容</label>
              <textarea v-model="form.content" class="form-input form-textarea" rows="8" placeholder="内容..."></textarea>
            </div>
            <div class="form-row">
              <label>标签（逗号分隔）</label>
              <input v-model="form.tags" class="form-input" placeholder="docker, 命令" />
            </div>
          </div>
          <div class="modal-footer">
            <button class="btn-secondary tap" @click="showForm = false">取消</button>
            <button class="btn-primary tap" @click="submit">保存</button>
          </div>
        </div>
      </div>
    </Transition>
  </div>
    <!-- 2026-11-05：语言分组「筛选」浮层面板（Teleport 到 body，不受滚动容器裁剪） -->
    <Teleport to="body">
      <Transition name="filter-pop">
        <div v-if="filterOpen" class="filter-panel" :style="filterPos" @click.stop>
          <div class="filter-panel-head">
            <span class="filter-panel-title">语言分组</span>
            <button v-if="activeFilterCount > 0" class="filter-clear tap" @click="clearFilters">清除</button>
          </div>
          <div class="lang-chips">
            <button
              v-for="l in LANG_OPTIONS"
              :key="l.key ?? 'all'"
              class="lang-chip tap"
              :class="{ active: snippetsStore.activeLanguage === l.key }"
              @click="snippetsStore.setLanguage(l.key)"
            >
              <span class="lang-chip-emoji">{{ l.emoji }}</span>
              <span>{{ l.label }}</span>
            </button>
          </div>
        </div>
      </Transition>
    </Teleport>
</template>

<style scoped>
.form-select-block { width: 100%; }

.view { padding-bottom: 20px; }
/* ===== 2026-11-05：顶部工具栏 sticky + 右上角筛选按钮/浮层面板 ===== */
.view-header {
  position: sticky;
  top: 0;
  z-index: 20;
  background: #16121f;
  padding: 4px 0 10px;
  margin-bottom: 12px;
  border-bottom: 1px solid var(--border);
  box-shadow: 0 6px 18px rgba(0, 0, 0, 0.28);
}
.filter-btn {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  padding: 6px 10px;
  border-radius: 10px;
  font-size: 12px;
  font-weight: 600;
  cursor: pointer;
  background: var(--bg-secondary);
  color: var(--text-muted);
  border: 1px solid var(--border);
  transition: all 0.15s var(--ease-smooth);
  -webkit-app-region: no-drag;
  white-space: nowrap;
}
.filter-btn:hover {
  background: var(--bg-tertiary);
  color: var(--text-primary);
  border-color: var(--border-strong);
}
.filter-btn.active {
  background: var(--accent-soft);
  color: var(--accent-bright);
  border-color: var(--accent);
}
.filter-btn-icon { font-size: 13px; line-height: 1; }
.filter-btn-count {
  min-width: 16px; height: 16px; padding: 0 4px; border-radius: 8px;
  background: var(--accent); color: #fff; font-size: 10px; font-weight: 700;
  display: inline-flex; align-items: center; justify-content: center; line-height: 1;
}
.filter-panel {
  position: fixed;
  z-index: 60;
  background: #1b1727;
  border: 1px solid var(--border-strong);
  border-radius: 14px;
  box-shadow: 0 16px 40px rgba(0, 0, 0, 0.5);
  padding: 12px;
  max-height: min(420px, calc(100vh - 140px));
  overflow-y: auto;
}
.filter-panel-head {
  display: flex; align-items: center; justify-content: space-between; margin-bottom: 8px;
}
.filter-panel-title {
  font-size: 11px; color: var(--text-faint); text-transform: uppercase;
  letter-spacing: 0.5px; font-weight: 700;
}
.filter-clear {
  border: 1px solid var(--border); background: transparent; color: var(--text-muted);
  font-size: 11px; padding: 2px 8px; border-radius: 8px; cursor: pointer;
}
.filter-clear:hover { color: var(--text-primary); border-color: var(--border-strong); }
.filter-panel .lang-chips { display: flex; flex-direction: column; gap: 4px; margin-bottom: 0; }
.filter-panel .lang-chip { width: 100%; justify-content: flex-start; }
.filter-pop-enter-active,
.filter-pop-leave-active { transition: opacity 0.15s var(--ease-smooth), transform 0.15s var(--ease-smooth); }
.filter-pop-enter-from,
.filter-pop-leave-to { opacity: 0; transform: translateY(-6px); }


/* ============ 左侧可收缩 sidebar（语言分类） ============ */

.lang-chips {
  display: flex;
  flex-direction: column;
  gap: 3px;
  width: 100%;
}
.lang-chip {
  width: 100%;
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 6px 8px;
  border-radius: 8px;
  font-size: 11.5px;
  font-weight: 500;
  cursor: pointer;
  background: var(--bg-secondary);
  color: var(--text-muted);
  border: 1px solid var(--border);
  transition: all 0.15s var(--ease-smooth);
  -webkit-app-region: no-drag;
  white-space: nowrap;
  min-height: 28px;
}
.lang-chip:hover {
  background: var(--bg-tertiary);
  color: var(--text-primary);
  border-color: var(--border-strong);
  transform: translateY(-1px);
}
.lang-chip.active {
  background: var(--accent-soft);
  color: var(--accent-bright);
  border-color: var(--accent);
  font-weight: 600;
  box-shadow: 0 2px 6px rgba(99, 102, 241, 0.25);
}
.lang-chip-emoji {
  font-size: 14px;
  flex-shrink: 0;
  width: 18px;
  text-align: center;
  line-height: 1;
}
.lang-chip-label {
  max-width: 0;
  opacity: 0;
  overflow: hidden;
  white-space: nowrap;
  transition: max-width 0.22s var(--ease-smooth) 0.04s, opacity 0.16s;
  pointer-events: none;
}

/* view-header / view-title 等都用 global 统一样式 */
.view-subtitle { font-size: 12px; color: var(--text-muted); }
.btn-add {
  display: flex; align-items: center; gap: 6px; padding: 9px 18px; border-radius: var(--radius); font-size: 13px; font-weight: 500;
  background: linear-gradient(135deg, var(--accent) 0%, #4fc3f7 100%); color: white; border: none; cursor: pointer;
  box-shadow: 0 2px 8px rgba(79, 124, 255, 0.3); transition: all 0.2s var(--ease-smooth);
  -webkit-app-region: no-drag;
}
.btn-add:hover { transform: translateY(-1px); box-shadow: 0 4px 12px rgba(79, 124, 255, 0.4); }
.btn-icon { font-size: 16px; font-weight: 600; }

.empty { text-align: center; padding: 60px 20px; color: var(--text-muted); }
.empty-icon { font-size: 48px; margin-bottom: 12px; opacity: 0.5; }
.empty-text { font-size: 13px; }

.list-container { display: flex; flex-direction: column; gap: 8px; }
.list-item {
  display: flex; align-items: stretch; gap: 12px; padding: 10px;
  background: var(--bg-primary); border: 1px solid var(--border); border-radius: 12px;
  transition: all 0.2s var(--ease-smooth);
}
.list-item:hover { border-color: rgba(79, 124, 255, 0.4); background: var(--bg-secondary); transform: translateX(2px); }
.item-main { display: flex; align-items: center; gap: 10px; flex: 1; min-width: 0; align-self: stretch; }
.item-icon {
  width: 36px; height: 36px; border-radius: 10px;
  display: flex; align-items: center; justify-content: center; font-size: 16px; font-weight: 600; position: relative; flex-shrink: 0;
  align-self: center;
}
.icon-cyan { background: linear-gradient(135deg, rgba(79, 199, 247, 0.3), rgba(79, 199, 247, 0.1)); color: #4fc3f7; }
.icon-gray { background: linear-gradient(135deg, rgba(150, 150, 150, 0.3), rgba(150, 150, 150, 0.1)); color: #aaa; }
.item-icon-glow { position: absolute; inset: 0; background: radial-gradient(circle, rgba(255, 255, 255, 0.2), transparent); pointer-events: none; }
.item-info { flex: 1; min-width: 0; display: flex; flex-direction: column; justify-content: center; gap: 4px; }
.item-title-row {
  display: flex;
  align-items: stretch;
  gap: 6px;
  min-width: 0;
  min-height: 30px;
  height: 30px;
}
/* P0-#SNIP#TAG#HEIGHT#30PX：标题高度 = lang-tag 高度 = 右侧 action-btn 高度 = 30px，像素级齐高 */
.item-title { font-size: 13px; font-weight: 600; color: var(--text-primary); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; flex: 1; min-width: 0; line-height: 30px; height: 30px; display: inline-flex; align-items: center; }
/* 方案②：lang-tag 与右侧 📋 ✏️ 🗑️ action-btn 严格 30px 同高 + 同圆角 (8px)，顶边底边完全对齐 */
.lang-tag {
  background: linear-gradient(135deg, rgba(99, 102, 241, 0.25), rgba(79, 195, 247, 0.18));
  color: var(--accent-bright);
  padding: 0 10px;
  height: 30px;
  line-height: 28px;
  border-radius: 8px;
  font-size: 11.5px;
  font-weight: 600;
  font-family: 'Cascadia Code', 'Consolas', monospace;
  border: 1px solid rgba(99, 102, 241, 0.35);
  white-space: nowrap;
  flex-shrink: 0;
  display: inline-flex;
  align-items: center;
  box-sizing: border-box;
}
.item-subtitle { font-size: 11px; color: var(--text-muted); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; margin-top: 0; display: flex; align-items: center; gap: 8px; min-height: 16px; line-height: 16px; }
.subtitle-preview { font-family: monospace; color: var(--text-faint); flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; }

.item-actions { display: flex; gap: 4px; flex-shrink: 0; align-items: center; align-self: flex-start; padding-top: 0; }
.item-confirm-wrap {
  flex-shrink: 0;
  max-width: 380px;
  align-self: center;
}
.item-flame {
  display: inline-flex;
  align-items: center;
  gap: 2px;
  padding: 3px 8px;
  border-radius: 9px;
  font-size: 10px;
  font-weight: 700;
  background: linear-gradient(135deg, rgba(255, 138, 94, 0.18) 0%, rgba(255, 78, 110, 0.18) 100%);
  color: #ff8a5e;
  border: 1px solid rgba(255, 138, 94, 0.3);
  letter-spacing: 0.2px;
  margin-right: 4px;
}
.action-btn {
  width: 30px; height: 30px; border: none; background: var(--bg-tertiary); color: var(--text-muted);
  border-radius: 8px; cursor: pointer; display: flex; align-items: center; justify-content: center; transition: all 0.15s;
  -webkit-app-region: no-drag;
}
.action-btn:hover { background: var(--accent); color: white; transform: scale(1.08); }

.clipboard-toast {
  position: fixed; bottom: 16px; right: 16px; padding: 10px 16px;
  background: rgba(0, 0, 0, 0.8); backdrop-filter: blur(8px);
  border: 1px solid var(--accent); border-radius: 10px;
  display: flex; align-items: center; gap: 8px; font-size: 12px; z-index: 99;
  box-shadow: 0 4px 16px rgba(79, 124, 255, 0.3);
}

.modal-mask { position: fixed; inset: 0; background: rgba(0, 0, 0, 0.6); backdrop-filter: blur(8px); display: flex; align-items: center; justify-content: center; z-index: 100; }
.modal { background: var(--bg-secondary); border: 1px solid var(--border); border-radius: 12px; width: 90%; max-width: 420px; max-height: 85vh; display: flex; flex-direction: column; box-shadow: 0 20px 60px rgba(0, 0, 0, 0.5); }
.modal-header { display: flex; justify-content: space-between; align-items: center; padding: 12px 16px; border-bottom: 1px solid var(--border); font-size: 14px; font-weight: 600; }
.modal-close { width: 26px; height: 26px; border: none; background: transparent; color: var(--text-muted); font-size: 18px; cursor: pointer; border-radius: 6px; -webkit-app-region: no-drag; }
.modal-close:hover { background: var(--highlight); color: white; }
.modal-body { padding: 16px; flex: 1; overflow-y: auto; }
.modal-footer { display: flex; justify-content: flex-end; gap: 8px; padding: 12px 16px; border-top: 1px solid var(--border); }
.form-row { display: flex; flex-direction: column; gap: 4px; margin-bottom: 10px; }
.form-row label { font-size: 11px; color: var(--text-muted); }
.form-input { background: rgba(0, 0, 0, 0.3); border: 1px solid var(--border); border-radius: 8px; padding: 7px 10px; color: var(--text-primary); font-size: 13px; font-family: inherit; min-width: 0; }
.form-input:focus { outline: none; border-color: var(--accent); }
.form-textarea { resize: vertical; font-family: monospace; min-height: 100px; }

.btn-primary, .btn-secondary { border: none; border-radius: 8px; padding: 7px 14px; font-size: 12px; cursor: pointer; font-weight: 500; transition: all 0.15s; -webkit-app-region: no-drag; }
.btn-primary { background: linear-gradient(135deg, var(--accent) 0%, #4fc3f7 100%); color: white; box-shadow: 0 2px 8px rgba(79, 124, 255, 0.3); }
.btn-primary:hover { transform: translateY(-1px); }
.btn-secondary { background: var(--bg-primary); color: var(--text-secondary); border: 1px solid var(--border); }
.btn-secondary:hover { background: var(--bg-secondary); border-color: var(--accent); }

.stagger-item { animation: slideIn 0.4s var(--ease-smooth) backwards; }
@keyframes slideIn { from { opacity: 0; transform: translateX(-10px); } to { opacity: 1; transform: translateX(0); } }
.list-enter-active, .list-leave-active { transition: all 0.3s var(--ease-smooth); }
.list-enter-from, .list-leave-to { opacity: 0; transform: translateX(-20px); }
.modal-enter-active, .modal-leave-active { transition: opacity 0.2s; }
.modal-enter-from, .modal-leave-to { opacity: 0; }
.toast-enter-active, .toast-leave-active { transition: all 0.3s var(--ease-smooth); }
.toast-enter-from, .toast-leave-to { opacity: 0; transform: translateY(20px); }
</style>

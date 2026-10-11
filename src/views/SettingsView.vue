<script setup lang="ts">
// ======================================================================
// Settings UI V2 · 设置中心容器
// - 五分类真切换：Tab 常驻挂载（v-show），长操作状态跨分类保留；
//   敏感输入清理由各 Tab 监听 active 完成；
// - 导航：320–899px 顶部单行横向滚动（选中项自动滚入视野，不折行、不缩成图标）；
//   ≥900px 左侧竖排导航 + 右侧内容区；
// - ≥1100px 内容区按实际可用宽度 auto-fit 双列（危险/全宽卡强制单列）；
// - 后台操作横幅：其他分类进行中的迁移/备份/更新在此可见。
// ======================================================================
import { ref, watch, nextTick, onMounted, onBeforeUnmount } from "vue";
import SettingsCard from "../components/settings/SettingsCard.vue";
import BasicTab from "./settings/tabs/BasicTab.vue";
import SecurityTab from "./settings/tabs/SecurityTab.vue";
import WindowTab from "./settings/tabs/WindowTab.vue";
import DataTab from "./settings/tabs/DataTab.vue";
import AdvancedTab from "./settings/tabs/AdvancedTab.vue";
import { provideSettingsOps } from "./settings/ops";
import { revealNavTab, updateScrollHints } from "./settings/revealTab";

provideSettingsOps();

const TABS = [
  { key: "basic", label: "基础", icon: "🛠️" },
  { key: "security", label: "安全与密码", icon: "🔒" },
  { key: "window", label: "窗口与外观", icon: "🖼️" },
  { key: "data", label: "数据与备份", icon: "📦" },
  { key: "advanced", label: "高级与更新", icon: "🔬" },
] as const;

const activeTab = ref<string>("basic");
const navEl = ref<HTMLElement | null>(null);
// 左右渐隐提示（仅当导航横向可滚时出现；白色系统滚动条已隐藏，滚动能力保留）
const canScrollLeft = ref(false);
const canScrollRight = ref(false);

function refreshScrollHints() {
  const nav = navEl.value;
  if (!nav) return;
  const hints = updateScrollHints(nav);
  canScrollLeft.value = hints.left;
  canScrollRight.value = hints.right;
}

function selectTab(key: string) {
  activeTab.value = key;
  void nextTick(() => {
    // P0 修复：只对导航容器自身 scrollTo（旧 scrollIntoView 会级联滚动
    // 应用根/content-scroll 造成整页左移裁切）
    if (navEl.value) revealNavTab(navEl.value, key);
    refreshScrollHints();
  });
}

// 导航键盘可达：ArrowLeft/ArrowRight 在分类间移动（roving 到选中项）
function onNavKeydown(e: KeyboardEvent) {
  if (e.key !== "ArrowLeft" && e.key !== "ArrowRight") return;
  e.preventDefault();
  const idx = TABS.findIndex((t) => t.key === activeTab.value);
  const delta = e.key === "ArrowRight" ? 1 : -1;
  const next = TABS[(idx + delta + TABS.length) % TABS.length];
  selectTab(next.key);
  void nextTick(() => {
    navEl.value?.querySelector<HTMLElement>(`[data-tab-key="${next.key}"]`)?.focus();
  });
}

// ---- 后台操作横幅（长操作状态跨分类可见）----
const { ops } = provideSettingsOps();
const backgroundOps = ref<{ label: string; originTab: string }[]>([]);
let stopOpsWatch: (() => void) | null = null;
onMounted(() => {
  navEl.value?.addEventListener("scroll", refreshScrollHints, { passive: true });
  window.addEventListener("resize", refreshScrollHints);
  void nextTick(refreshScrollHints);
  stopOpsWatch = watch(
    () => ({ ...ops }),
    (val) => {
      backgroundOps.value = Object.entries(val)
        .filter(([, v]) => v && v.originTab !== activeTab.value)
        .map(([, v]) => ({ label: v.label, originTab: v.originTab }));
    },
    { deep: true, immediate: true }
  );
});
onBeforeUnmount(() => {
  stopOpsWatch?.();
  navEl.value?.removeEventListener("scroll", refreshScrollHints);
  window.removeEventListener("resize", refreshScrollHints);
});
watch(activeTab, () => {
  backgroundOps.value = Object.entries(ops)
    .filter(([, v]) => v && v.originTab !== activeTab.value)
    .map(([, v]) => ({ label: v.label, originTab: v.originTab }));
});

const ORIGIN_LABEL: Record<string, string> = {
  data: "数据与备份",
  advanced: "高级与更新",
  security: "安全与密码",
};
</script>

<template>
  <div class="view settings-root">
    <!-- 后台操作横幅 -->
    <div v-if="backgroundOps.length" class="ops-banner" role="status">
      <span class="ops-spin" aria-hidden="true">⏳</span>
      <span>
        后台操作进行中：<b>{{ backgroundOps.map((o) => o.label).join("、") }}</b>
        <template v-if="backgroundOps.some((o) => o.originTab === 'data')">
          —— 完成前请勿关闭抽屉柜（详情在「{{ ORIGIN_LABEL.data }}」分类）
        </template>
      </span>
    </div>

    <div class="settings-body">
      <!-- 分类导航：窄窗口顶部横向滚动（隐藏原生滚动条 + 左右渐隐提示）；≥900px 左侧竖排 -->
      <div class="s-nav-wrap">
        <span v-show="canScrollLeft" class="s-nav-fade left" aria-hidden="true"></span>
        <span v-show="canScrollRight" class="s-nav-fade right" aria-hidden="true"></span>
        <nav
          ref="navEl"
        class="s-nav"
        role="tablist"
        aria-label="设置分类"
        @keydown="onNavKeydown"
      >
        <button
          v-for="t in TABS"
          :id="`tab-${t.key}`"
          :key="t.key"
          class="s-nav-btn tap"
          :class="{ active: activeTab === t.key }"
          role="tab"
          :aria-selected="activeTab === t.key"
          :aria-controls="`panel-${t.key}`"
          :tabindex="activeTab === t.key ? 0 : -1"
          :title="t.label"
          data-interactive
          :data-tab-key="t.key"
          @click="selectTab(t.key)"
        >
          <span class="s-nav-icon">{{ t.icon }}</span>
          <span class="s-nav-label">{{ t.label }}</span>
        </button>
      </nav>
      </div>

      <!-- 内容区：Tab 常驻挂载，仅当前分类可见 -->
      <div class="s-content">
        <div v-show="activeTab === 'basic'" id="panel-basic" class="s-panel" role="tabpanel" aria-labelledby="tab-basic">
          <BasicTab :active="activeTab === 'basic'" />
        </div>
        <div v-show="activeTab === 'security'" id="panel-security" class="s-panel" role="tabpanel" aria-labelledby="tab-security">
          <SecurityTab :active="activeTab === 'security'" />
        </div>
        <div v-show="activeTab === 'window'" id="panel-window" class="s-panel" role="tabpanel" aria-labelledby="tab-window">
          <WindowTab :active="activeTab === 'window'" />
        </div>
        <div v-show="activeTab === 'data'" id="panel-data" class="s-panel" role="tabpanel" aria-labelledby="tab-data">
          <DataTab :active="activeTab === 'data'" />
        </div>
        <div v-show="activeTab === 'advanced'" id="panel-advanced" class="s-panel" role="tabpanel" aria-labelledby="tab-advanced">
          <AdvancedTab :active="activeTab === 'advanced'" />
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.settings-root {
  display: flex;
  flex-direction: column;
  min-height: 0;
  min-width: 0;
  max-width: 100%;
  position: relative;
  box-sizing: border-box;
}

/* ===== 后台操作横幅 ===== */
.ops-banner {
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
  max-width: 100%;
  overflow-wrap: anywhere;
  padding: 8px 12px;
  margin-bottom: 10px;
  border-radius: 9px;
  background: rgba(59, 130, 246, 0.1);
  border: 1px solid rgba(59, 130, 246, 0.4);
  color: #93c5fd;
  font-size: 12px;
  line-height: 1.5;
}
.ops-spin { animation: opsPulse 1.2s ease-in-out infinite; }
@keyframes opsPulse {
  0%, 100% { opacity: 1; }
  50% { opacity: 0.45; }
}

/* ===== 布局：默认（<900px）顶部横向导航 ===== */
.settings-body {
  display: flex;
  flex-direction: column;
  min-height: 0;
  gap: 12px;
}
.s-nav {
  display: flex;
  gap: 6px;
  padding: 5px;
  background: var(--bg-primary);
  border: 1px solid var(--border);
  border-radius: 10px;
  position: sticky;
  top: 0;
  z-index: 5;
  backdrop-filter: blur(10px);
  -webkit-backdrop-filter: blur(10px);
  box-shadow: 0 2px 8px rgba(0, 0, 0, 0.18);
  overflow-x: auto;
  /* 隐藏原生滚动条视觉（白色系统条），滚动能力保留；左右可滚提示用渐隐遮罩 */
  scrollbar-width: none;
  -ms-overflow-style: none;
}
.s-nav::-webkit-scrollbar { display: none; }
.s-nav-wrap {
  position: relative;
  min-width: 0;
  max-width: 100%;
}
.s-nav-fade {
  position: absolute;
  top: 0;
  bottom: 0;
  width: 28px;
  pointer-events: none;
  z-index: 6;
}
.s-nav-fade.left { left: 0; background: linear-gradient(90deg, var(--bg-primary), transparent); }
.s-nav-fade.right { right: 0; background: linear-gradient(270deg, var(--bg-primary), transparent); }
.s-nav-btn {
  flex: none;
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 6px;
  padding: 9px 14px;
  border: none;
  border-radius: 8px;
  background: transparent;
  color: var(--text-muted);
  font-size: 12.5px;
  font-weight: 500;
  cursor: pointer;
  transition: all 0.18s var(--ease-smooth);
  -webkit-app-region: no-drag;
  white-space: nowrap;
}
.s-nav-btn:hover { background: var(--bg-tertiary); color: var(--text-primary); }
.s-nav-btn.active {
  background: linear-gradient(135deg, var(--accent) 0%, var(--highlight) 100%);
  color: white;
  font-weight: 600;
  box-shadow: 0 2px 8px rgba(99, 102, 241, 0.35);
}
.s-nav-btn:focus-visible {
  outline: 2px solid var(--accent);
  outline-offset: 1px;
}
.s-nav-icon { font-size: 14px; line-height: 1; }
.s-nav-label { letter-spacing: 0.2px; }

/* ≥900px：左侧竖排导航 + 右侧内容 */
@media (min-width: 900px) {
  .settings-body { flex-direction: row; align-items: flex-start; }
  .s-nav {
    flex-direction: column;
    gap: 4px;
    width: 168px;
    flex-shrink: 0;
    position: sticky;
    top: 0;
    overflow-x: visible;
    overflow-y: auto;
  }
  .s-nav-btn { justify-content: flex-start; padding: 10px 12px; width: 100%; }
  .s-content { flex: 1; min-width: 0; }
}

/* 内容区与双列（≥1100px 且实际宽度足够时 auto-fit 双列；危险/全宽卡强制单列） */
.s-panel { min-width: 0; }
@media (min-width: 1100px) {
  .s-panel {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(430px, 1fr));
    gap: 14px;
    align-items: start;
  }
  .s-panel > * { min-width: 0; }
  /* 危险与全宽卡片：不进双列 */
  .s-panel :deep(.s-card-danger) { grid-column: 1 / -1; }
}
@media (max-width: 899px) {
  .s-content { width: 100%; min-width: 0; }
}
</style>

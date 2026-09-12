<script setup lang="ts">
import { onMounted, onUnmounted, ref, watch } from "vue";
import { useRouter, useRoute } from "vue-router";
import { useAppStore } from "../stores/app";
import { useWidgetStore } from "../stores/widget";
import { hideWindow, lockApp, setWindowSize, rescanSubtypes, getAppVersion } from "../api";
import { listen } from "@tauri-apps/api/event";
import { useWindowDrag } from "../composables/useWindowDrag";
import { useSoftwareStore } from "../stores/software";
import { useTempStore } from "../stores/temp";
import SearchBar from "./SearchBar.vue";
import SummaryModal from "./SummaryModal.vue";
import TrashModal from "./TrashModal.vue";
import { RouterView } from "vue-router";
// 品牌 Logo
import logo from "@/assets/logo.png";

const router = useRouter();
const route = useRoute();
const appStore = useAppStore();
const widgetStore = useWidgetStore();
const softwareStore = useSoftwareStore();
const tempStore = useTempStore();
const showTrash = ref(false);

// P0-#Y#FIX#VERSION#DISPLAY：版本号（启动时调一次）
// 用户看这个判断 exe 是不是最新的
const versionInfo = ref<string>("");
getAppVersion().then((v) => (versionInfo.value = v)).catch(() => {});

interface Tab {
  key: string;
  label: string;
  icon: string;
  path: string;
  query?: Record<string, string>;
}

// 侧边栏 5 个 tab：常用（跨区聚合）+ 密码 + 软件 + 命令行 + 临时
// 已删 PinnedSection（去重）
const tabs: Tab[] = [
  { key: "frequent", label: "常用", icon: "⭐", path: "/frequent" },
  { key: "password", label: "密码", icon: "🔑", path: "/passwords" },
  { key: "app", label: "软件", icon: "🚀", path: "/apps" },
  { key: "snippet", label: "命令行", icon: "💻", path: "/snippets" },
  { key: "temp", label: "便签", icon: "📝", path: "/temp" },
];

const isActive = (t: Tab) => {
  if (t.query) {
    return route.path === t.path && Object.entries(t.query).every(
      ([k, v]) => route.query[k] === v
    );
  }
  return route.path === t.path;
};

const navigate = (t: Tab) => {
  router.push({ path: t.path, query: t.query });
};

const closeWindow = () => hideWindow();
const lockNow = () => {
  lockApp();
  appStore.lock();
  hideWindow();
};

const toggleMini = async () => {
  await widgetStore.toggleMiniMode();
};

// 轮询 Rust 端隐藏态已移除：性能优化，每 500ms IPC 持续打 WebView 队列
// 隐藏态现在通过 widget:force-expand / dock:hidden 事件驱动（Rust 端主动推）

// ===== 拖动时边缘吸附 =====
// 修复 P0-#X：拖动期间严禁 IPC 风暴！
// 之前：unlistenMove (tauri://move 每帧) → scheduleSavePosition → 4个 IPC
//       + onTitleBarMouseDown setTimeout 300ms → scheduleCheckEdgeProximity → 2个 IPC
//       拖动中每秒上百次 IPC 调用把 WebView 队列打满 → 主线程卡死 → "未响应"
// 修复：拖动中完全冻结 IPC，拖动结束统一做一次"位置持久化 + 边缘检测"
let unlistenMove: (() => void) | null = null;
let unlistenForceExpand: (() => void) | null = null;

// ===== 拖动：使用共享的 useWindowDrag 组合式函数 =====
// 拖动结束时只做"位置持久化"；边缘检测完全交给 Rust 端 WndProc
// （WM_WINDOWPOSCHANGING 会在用户拖动时持续触发，Rust 自动判断贴边 + 启动 350ms 倒计时）
function onDragEnd() {
  // 仅一次 IPC 风暴（位置 + 大小 + 2 个 saveWidgetConfig）
  widgetStore.savePositionNow();
  // 不再调 scheduleCheckEdgeProximity —— Rust 端 WndProc 已全程接管
}
const { onDragHandleMouseDown } = useWindowDrag({ onDragEnd });

function onTitleBarMouseDown(e: MouseEvent) {
  onDragHandleMouseDown(e);
  // 不再 setTimeout 调 scheduleCheckEdgeProximity（在 onDragEnd 统一处理）
}

// 迷你模式拖动：图标列本身可拖，但按钮内点击不触发拖动（composable 已做判断）
function onMiniMouseDown(e: MouseEvent) {
  onDragHandleMouseDown(e);
}

// 暴露给模板
defineExpose({});

// P0-#Y#FIX#TRASH#OPEN：设置界面"打开回收站"按钮派发的 window 事件处理
// （提升到 setup 作用域供 onUnmounted 移除；MainLayout 每次锁屏-解锁重新挂载，
//   原来只加不减会累积监听）
const onOpenTrash = () => { showTrash.value = true; };

onMounted(() => {
  // P0-#PERF#INIT：onMounted **绝不 await 任何 IPC** — 改为 fire-and-forget
  // 之前：await getDataDir() / await widgetStore.init() / await listen() 全部同步 block
  //   → MainLayout mount 完要等 N 个 IPC 返回，UI 渲染被卡住
  //   → 用户感觉"启动卡顿"
  // 现在：所有初始化都进 void 包裹，UI 立即渲染，IPC 在后台跑
  console.log("[MainLayout] onMounted START (fire-and-forget init)");
  (window as any).__ml_mounted = true;

  // ===== 后台任务 1：后端日志冒泡（证明 MainLayout 真的 mount 了）=====
  void (async () => {
    try {
      const { getDataDir } = await import("../api");
      await getDataDir();
    } catch (e) {
      console.warn("[MainLayout] getDataDir 失败", e);
    }
  })();

  // ===== 后台任务 2：widget 配置初始化（曾经同步 await — 现在后台跑）=====
  void (async () => {
    try {
      await widgetStore.init();
    } catch (e) {
      console.warn("[MainLayout] widgetStore.init 失败", e);
    }
  })();

  // ===== 后台任务 3：便签列表 + 回收站徽章 =====
  void tempStore.loadItems().catch(() => {});
  void appStore.refreshTrashCount().catch(() => {});

  // ===== 后台任务 4：启动时健康检查 — 自动修复失效 app path =====
  // 关键：等 UI 渲染**完成**后再跑（避免阻塞首帧）
  // 用 setTimeout 50ms 让浏览器先绘制一次
  setTimeout(() => {
    void (async () => {
      try {
        const { healthCheckApps } = await import("../api");
        const fixed = await healthCheckApps();
        if (fixed > 0) {
          appStore.showClipToast("info", `已自动修复 ${fixed} 个失效应用路径`);
          await softwareStore.loadItems();
        }
      } catch (e) {
        console.warn("[healthCheck] 失败", e);
      }
    })();
  }, 200);

  // ===== 后台任务 5：启动时回填所有 app 的 subtype =====
  setTimeout(() => {
    void (async () => {
      try {
        const n = await rescanSubtypes();
        if (n > 0) {
          await softwareStore.loadItems();
        }
      } catch (e) {
        console.warn("[MainLayout] rescanSubtypes 失败", e);
      }
    })();
  }, 400);

  // ===== 后台任务 6：监听窗口事件 =====
  void (async () => {
    try {
      // 修复 P0-#Y：恢复 tauri://move 监听 + 500ms debounce 保存位置
      // 拖动期间 tauri://move 每秒触发 60+ 次，debounce 500ms 兜住 IPC 风暴
      let moveDebounce: number | null = null;
      unlistenMove = await listen("tauri://move", () => {
        if (moveDebounce) clearTimeout(moveDebounce);
        moveDebounce = window.setTimeout(() => {
          widgetStore.savePositionNow();
        }, 500);
      });
      unlistenForceExpand = await listen("widget:force-expand", async () => {
        await widgetStore.forceExpand();
      });
      // P0-#Y#FIX#TRASH#OPEN：监听设置界面"打开回收站"按钮派发的事件
      // （移出 try：listen() 失败不应连累这个本地监听的注册）
      window.addEventListener("open-trash", onOpenTrash);
    } catch (e) {
      console.warn("监听窗口事件失败", e);
    }
  })();
});

onUnmounted(() => {
  unlistenMove?.();
  unlistenForceExpand?.();
  // ✅ 泄漏修复：与 onMounted 中的 addEventListener 成对移除
  window.removeEventListener("open-trash", onOpenTrash);
});

// ===== 迷你模式自适应窗口尺寸 =====
watch(
  () => widgetStore.miniMode,
  async (isMini) => {
    if (isMini) {
      await setWindowSize(64, 360);
    } else {
      // 默认 420×640（不覆盖用户手动调过的尺寸）
      // 用户在主界面里拖拽过窗口后，apply_widget_config 会用保存的尺寸
      // 这里只在"刚展开回完整模式"时给个建议默认值（可被用户后续拖拽覆盖）
      await setWindowSize(420, 640);
    }
  }
);
</script>

<template>
  <!-- 迷你模式：极简图标列 -->
  <Transition name="mode-flip" mode="out-in">
    <div
      v-if="widgetStore.miniMode"
      key="mini"
      class="mini-widget"
      data-tauri-drag-region
      @mousedown="onMiniMouseDown"
    >
      <!-- 顶部展开按钮 -->
      <div
        class="mini-handle"
        data-interactive
        @mousedown.stop
        @click="toggleMini"
        title="展开抽屉柜"
      >
        <span class="mini-handle-icon">▴</span>
      </div>

      <!-- 图标列 -->
      <div class="mini-icons">
        <button
          v-for="t in tabs.slice(0, 6)"
          :key="t.key"
          class="mini-icon tap"
          :class="{ active: isActive(t) }"
          :title="t.label"
          data-interactive
          @click="async () => { await widgetStore.toggleMiniMode(); navigate(t); }"
        >
          <span class="mini-icon-glyph">{{ t.icon }}</span>
          <span class="mini-icon-dot" v-if="isActive(t)"></span>
        </button>
      </div>

      <!-- 底部状态指示 -->
      <div class="mini-foot">
        <div class="mini-dot" :class="{ on: widgetStore.alwaysOnTop }" :title="widgetStore.alwaysOnTop ? '已置顶' : '未置顶'"></div>
      </div>
    </div>

    <!-- 完整模式：玻璃卡片 -->
    <div v-else key="full" class="widget">
      <!-- ===== 标题栏（拖拽区）· 深色玻璃统一 ===== -->
      <div class="titlebar" data-tauri-drag-region @mousedown="onTitleBarMouseDown">
        <div class="titlebar-left" data-tauri-drag-region>
          <img :src="logo" alt="抽屉柜 Drawer" class="logo" />
          <div class="title">抽屉柜 Drawer</div>
          <!-- P0-#Y#FIX#VERSION#DISPLAY：标题栏显示版本号，用户能肉眼看出是不是新版本 -->
          <div class="titlebar-version" :title="`编译时间：${versionInfo || '加载中'}`">
            {{ versionInfo || '...' }}
          </div>
        </div>
        <div class="titlebar-right">
          <button class="titlebar-btn" title="缩小为图标列" @click="toggleMini" data-interactive>
            <span>◂</span>
          </button>
          <!-- P0-T#FIX：回收站已从 titlebar 移走，放到搜索框外的最右面（全局可见、自适应） -->
          <button class="titlebar-btn" title="设置" @click="router.push('/settings')" data-interactive>
            <span>⚙</span>
          </button>
          <button class="titlebar-btn" title="锁定 (Alt+L)" @click="lockNow" data-interactive>
            <span>🔒</span>
          </button>
          <button class="titlebar-btn close" title="隐藏 (Alt+Q)" @click="closeWindow" data-interactive>
            <span>✕</span>
          </button>
        </div>
      </div>

      <!-- ===== 顶部头：搜索框（flex:1）+ 回收站（最右，跟随窗口缩放自适应） ===== -->
      <div class="top-header" data-tauri-drag-region>
        <div class="top-header-inner" data-tauri-drag-region>
          <SearchBar v-model="appStore.searchQuery" />
          <!-- P0-T#FIX#UI：回收站按钮移到搜索框外、自适应最右面 -->
          <button
            class="trash-btn-fixed"
            :class="{ 'has-items': appStore.trashCount > 0 }"
            :title="`回收站（${appStore.trashCount} 项）`"
            @click="showTrash = true"
            data-interactive
          >
            <span class="trash-icon-fixed">🗑️</span>
            <span v-if="appStore.trashCount > 0" class="trash-badge-fixed">{{ appStore.trashCount > 99 ? "99+" : appStore.trashCount }}</span>
          </button>
        </div>
      </div>

      <!-- ===== 主体：内容（占满宽度）+ 底部 Tab 栏 ===== -->
      <div class="body">
        <!-- 主内容区 -->
        <div class="content">
          <div class="content-scroll">
            <RouterView v-slot="{ Component }">
              <!-- 修复 P0：去掉 :key="route.fullPath"，让 KeepAlive 按 component name 缓存
                   之前用 fullPath 每次切 tab key 变 → 缓存失效，组件每次都重建 → 慢 -->
              <KeepAlive>
                <component :is="Component" class="page-wrapper" />
              </KeepAlive>
            </RouterView>
          </div>
        </div>

        <!-- 底部水平 Tab 栏（5 个 tab 平均分配 76px） -->
        <nav class="bottom-tabs">
          <button
            v-for="t in tabs"
            :key="t.key"
            class="bottom-tab tap"
            :class="{ active: isActive(t) }"
            :title="t.label"
            @click="navigate(t)"
          >
            <!-- 便签 tab 红点：剩余 ≤24h 的数量 -->
            <span
              v-if="t.key === 'temp' && tempStore.expiringSoonCount > 0"
              class="tab-badge tap"
              :title="`${tempStore.expiringSoonCount} 个便签将在 24 小时内过期`"
            >{{ tempStore.expiringSoonCount }}</span>
            <span class="bottom-tab-icon">{{ t.icon }}</span>
            <span class="bottom-tab-label">{{ t.label }}</span>
          </button>
        </nav>
      </div>

      <SummaryModal />
      <TrashModal v-if="showTrash" @close="showTrash = false" @restored="appStore.refreshTrashCount()" />
    </div>
  </Transition>
</template>

<style scoped>
/* ===== 模式切换动画 ===== */
/* 关键修复：移除 scale + blur + rotateY 动画，改为 0.1s 透明度淡入
 * 之前：0.22s spring 动画 + 0.4s widgetIn 入场 + 0.4s miniIn 入场 = 3 层叠加
 *       在小窗口切 mini 模式时整窗模糊缩放 → "闪屏"假象
 * 现在：只保留淡入，肉眼几乎无感
 */
.mode-flip-enter-active { animation: fadeIn 0.1s linear; }
.mode-flip-leave-active { animation: fadeOut 0.06s linear; position: absolute; inset: 0; }

@keyframes fadeIn { from { opacity: 0.4; } to { opacity: 1; } }
@keyframes fadeOut { from { opacity: 1; } to { opacity: 0.4; } }

/* =================================================== */
/* ============ 完整模式：玻璃卡片 widget ============ */
/* =================================================== */

.widget {
  width: 100%;
  height: 100%;
  /* 深色玻璃（与 LockScreen 统一）：深紫蓝底渐变 */
  background: linear-gradient(
    135deg,
    #14141c 0%,
    #1a1428 50%,
    #1e1832 100%
  );
  /* 性能优化：blur 从 60px 降到 16px，saturate 从 180% 降到 140%
   * 60px 在 WebView2 中每帧重算会拖到 5-10 fps，按钮"按不动"的元凶
   * 16px 视觉差异极小但性能提升 10x+ */
  backdrop-filter: blur(16px) saturate(140%);
  -webkit-backdrop-filter: blur(16px) saturate(140%);
  border: 1px solid rgba(255, 255, 255, 0.08);
  border-radius: 24px;
  box-shadow:
    0 20px 60px rgba(0, 0, 0, 0.5),
    inset 0 1px 0 rgba(255, 255, 255, 0.08);
  display: flex;
  flex-direction: column;
  overflow: hidden;
  position: relative;
  /* 关键修复：移除 widgetIn 入场动画（0.4s 缩放+模糊+位移）
   * 之前完整模式每次显示都从模糊缩放到清晰 → "闪屏"
   * 现在直接稳定显示 */
  color: rgba(255, 255, 255, 0.96);
  /* 在 widget 内部重写 CSS 变量，让所有 view 子组件自动跟随深色主题 */
  --text-primary: rgba(255, 255, 255, 0.96);
  --text-secondary: rgba(255, 255, 255, 0.78);
  --text-muted: rgba(255, 255, 255, 0.56);
  --text-faint: rgba(255, 255, 255, 0.4);
  --border: rgba(255, 255, 255, 0.1);
  --border-strong: rgba(255, 255, 255, 0.18);
  --border-input: rgba(255, 255, 255, 0.14);
  --bg-primary: rgba(255, 255, 255, 0.05);
  --bg-secondary: rgba(255, 255, 255, 0.09);
  --bg-tertiary: rgba(255, 255, 255, 0.13);
  --bg-glass: rgba(255, 255, 255, 0.08);
  --accent-soft: rgba(99, 102, 241, 0.25);
  --highlight-soft: rgba(236, 72, 153, 0.22);
  --shadow-xs: 0 1px 2px rgba(0, 0, 0, 0.3);
  --shadow-sm: 0 2px 8px rgba(0, 0, 0, 0.35), 0 1px 2px rgba(0, 0, 0, 0.25);
  --shadow-md: 0 8px 24px rgba(0, 0, 0, 0.45), 0 2px 4px rgba(0, 0, 0, 0.3);
  --shadow-lg: 0 16px 40px rgba(0, 0, 0, 0.55), 0 4px 8px rgba(0, 0, 0, 0.35);
}

/* 拖动中：阴影变深 + 整窗半透明，让"飘浮"感更强 */
:global(.is-dragging) .widget {
  opacity: 0.78;
  box-shadow:
    0 32px 100px rgba(79, 124, 255, 0.25),
    0 0 0 1px rgba(79, 124, 255, 0.3),
    inset 0 1px 0 rgba(255, 255, 255, 0.1);
}

/* 装饰光晕 */
.widget::before {
  content: "";
  position: absolute;
  top: -40px;
  left: -40px;
  width: 180px;
  height: 180px;
  background: radial-gradient(circle, rgba(79, 124, 255, 0.18) 0%, transparent 70%);
  pointer-events: none;
  z-index: 0;
}
.widget::after {
  content: "";
  position: absolute;
  bottom: -60px;
  right: -60px;
  width: 200px;
  height: 200px;
  background: radial-gradient(circle, rgba(255, 94, 126, 0.12) 0%, transparent 70%);
  pointer-events: none;
  z-index: 0;
}

/* ===== 标题栏 · 深色玻璃（统一主题） ===== */
.titlebar {
  height: 48px;
  padding: 0 10px 0 14px;
  display: flex;
  align-items: center;
  justify-content: space-between;
  background: rgba(0, 0, 0, 0.18);                  /* 深色底 */
  border-bottom: 1px solid rgba(255, 255, 255, 0.08);
  position: relative;
  z-index: 2;
  cursor: grab;
  flex-shrink: 0;
  -webkit-app-region: drag;
  app-region: drag;
  user-select: none;
  -webkit-user-select: none;
  transition: background 0.18s var(--ease-smooth);
}
.titlebar:active { cursor: grabbing; }

:global(.is-dragging) .titlebar {
  background: rgba(99, 102, 241, 0.18);
  border-bottom-color: var(--accent-soft);
}

.titlebar-left {
  display: flex;
  align-items: center;
  gap: 8px;
  user-select: none;
  -webkit-app-region: drag; /* 子元素也声明，避免按钮影响 */
  app-region: drag;
}
.logo {
  /* 关键修复：img 用 width/height，font-size 无效！ */
  width: 18px;
  height: 18px;
  object-fit: contain;
  border-radius: 4px;
  flex-shrink: 0;
  filter: drop-shadow(0 1px 2px rgba(99, 102, 241, 0.4));
}
.title {
  font-size: 14px;
  font-weight: 700;
  background: linear-gradient(135deg, #818cf8 0%, #f472b6 100%);
  -webkit-background-clip: text;
  background-clip: text;
  -webkit-text-fill-color: transparent;
  letter-spacing: 0.3px;
}
.titlebar-version {
  margin-left: 8px;
  padding: 1px 6px;
  font-size: 9px;
  color: rgba(255, 255, 255, 0.45);
  background: rgba(255, 255, 255, 0.06);
  border: 1px solid rgba(255, 255, 255, 0.08);
  border-radius: 4px;
  font-family: monospace;
  font-weight: 500;
}
.titlebar-right { display: flex; align-items: center; gap: 2px; }

.debug-info {
  margin-left: 10px;
  padding: 2px 8px;
  background: rgba(0, 0, 0, 0.3);
  border: 1px solid var(--border);
  border-radius: 4px;
  font-size: 10px;
  font-family: monospace;
  color: var(--text-faint);
  -webkit-app-region: no-drag;
  app-region: no-drag;
  user-select: text;
  cursor: text;
  white-space: nowrap;
}

.titlebar-btn {
  width: 30px;
  height: 30px;
  border-radius: 8px;
  border: none;
  background: transparent;
  color: var(--text-secondary);
  cursor: pointer;
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: 15px;
  font-weight: 600;
  transition: all 0.15s var(--ease-smooth);
  -webkit-app-region: no-drag;
  app-region: no-drag;
  position: relative;
  overflow: hidden;
}
.titlebar-btn:hover { background: var(--accent-soft); color: var(--text-primary); }
.titlebar-btn.close:hover { background: var(--danger); color: #fff; }
/* 回收站按钮 — 高可见性 */
.titlebar-btn.trash-btn {
  position: relative;
  font-size: 15px;
}
.titlebar-btn.trash-btn.has-items {
  background: rgba(239, 68, 68, 0.12);
  color: #fca5a5;
}
.titlebar-trash-badge {
  position: absolute;
  top: -2px;
  right: -2px;
  min-width: 14px;
  height: 14px;
  padding: 0 4px;
  background: linear-gradient(135deg, #ef4444, #dc2626);
  color: white;
  font-size: 9px;
  font-weight: 700;
  border-radius: 7px;
  display: flex;
  align-items: center;
  justify-content: center;
  line-height: 1;
  border: 1.5px solid var(--bg-primary, #0f1115);
  box-shadow: 0 1px 4px rgba(239, 68, 68, 0.5);
  animation: trashBadgePulse 2s ease-in-out infinite;
}
@keyframes trashBadgePulse {
  0%, 100% { transform: scale(1); }
  50% { transform: scale(1.1); }
}
.titlebar-btn:active {
  transform: scale(0.92);
  background: var(--accent);
  color: #fff;
}

/* ===== 主体布局：顶部头 + 侧栏 + 内容 ===== */
.top-header {
  flex-shrink: 0;
  background: linear-gradient(180deg, rgba(0, 0, 0, 0.25) 0%, transparent 100%);
  border-bottom: 1px solid rgba(255, 255, 255, 0.05);
  -webkit-app-region: drag;
  app-region: drag;
}
.top-header-inner {
  padding: 10px 14px 8px;
  -webkit-app-region: drag;
  app-region: drag;
  /* P0-T#FIX：flex 布局，搜索框占剩余空间，回收站贴右自适应 */
  display: flex;
  align-items: center;
  gap: 10px;
}
/* 搜索框区域：占满剩余空间 */
.top-header-inner > :first-child {
  flex: 1 1 0;
  min-width: 0;
}

/* ===== P0-T#FIX：回收站按钮（搜索框外、最右面、自适应）===== */
.trash-btn-fixed {
  position: relative;
  width: 38px;
  height: 38px;
  flex-shrink: 0;          /* 永远不被压缩，确保可见 */
  -webkit-app-region: no-drag;
  app-region: no-drag;
  border: 1px solid var(--border);
  background: var(--bg-primary);
  border-radius: 10px;
  cursor: pointer;
  display: flex;
  align-items: center;
  justify-content: center;
  transition: all 0.2s var(--ease-smooth);
}
.trash-btn-fixed:hover {
  background: var(--bg-secondary);
  border-color: var(--accent);
  transform: scale(1.05);
  box-shadow: 0 4px 12px rgba(79, 124, 255, 0.25);
}
.trash-btn-fixed:active {
  transform: scale(0.95);
}
.trash-icon-fixed { font-size: 17px; }
.trash-badge-fixed {
  position: absolute;
  top: -5px;
  right: -5px;
  min-width: 18px;
  height: 18px;
  padding: 0 5px;
  background: linear-gradient(135deg, #ef4444, #dc2626);
  color: white;
  font-size: 10px;
  font-weight: 700;
  border-radius: 9px;
  display: flex;
  align-items: center;
  justify-content: center;
  border: 1.5px solid #14141c;        /* 与背景同色，让徽章"浮"在按钮上 */
  box-shadow: 0 2px 6px rgba(239, 68, 68, 0.5);
  animation: trashBadgePop 0.3s cubic-bezier(0.34, 1.56, 0.64, 1);
  pointer-events: none;
}
.trash-btn-fixed.has-items {
  border-color: rgba(239, 68, 68, 0.3);
  background: rgba(239, 68, 68, 0.06);
}
@keyframes trashBadgePop {
  0% { transform: scale(0); }
  60% { transform: scale(1.25); }
  100% { transform: scale(1); }
}

.body {
  flex: 1;
  display: flex;
  flex-direction: column;
  min-height: 0; /* 让 flex 子项可滚动 */
}

/* ===== 底部水平 Tab 栏（5 个 tab，380px 内紧凑清晰） ===== */
/* 关键修复：5 个 tab 在 380px 宽下平分 76px，必须紧凑布局：
 *   - icon 居中放大（更突出）
 *   - label 字号 11px + 截断 + active 加粗
 *   - active 用左侧 3px 高亮条 + 圆角背景块双重指示
 *   - 整栏 60px 高度，让 label 不被压缩
 */
.bottom-tabs {
  display: flex;
  height: 60px;
  flex-shrink: 0;
  border-top: 1px solid rgba(255, 255, 255, 0.08);
  background: rgba(0, 0, 0, 0.28);
  -webkit-app-region: no-drag;
  app-region: no-drag;
  backdrop-filter: blur(8px);
  -webkit-backdrop-filter: blur(8px);
  padding: 0 4px;
  gap: 2px;
}
.bottom-tab {
  flex: 1;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 3px;
  border: none;
  background: transparent;
  color: var(--text-muted);
  cursor: pointer;
  font-family: inherit;
  padding: 6px 2px;
  border-radius: 10px;
  transition: all 0.18s var(--ease-smooth);
  position: relative;
  -webkit-app-region: no-drag;
  app-region: no-drag;
  min-width: 0;
  overflow: hidden;
}
.bottom-tab:hover {
  background: rgba(99, 102, 241, 0.12);
  color: var(--text-primary);
}
.bottom-tab:active {
  transform: scale(0.94);
}
.bottom-tab.active {
  color: #fff;
  background: linear-gradient(180deg, rgba(99, 102, 241, 0.18) 0%, rgba(99, 102, 241, 0.08) 100%);
}
/* 顶部高亮条 */
.bottom-tab.active::before {
  content: "";
  position: absolute;
  top: 0;
  left: 50%;
  transform: translateX(-50%);
  width: 28px;
  height: 2.5px;
  background: linear-gradient(90deg, var(--accent) 0%, var(--highlight) 100%);
  border-radius: 0 0 3px 3px;
  box-shadow: 0 1px 8px var(--accent-soft);
}
.bottom-tab-icon {
  font-size: 20px;
  line-height: 1;
  filter: drop-shadow(0 1px 2px rgba(0, 0, 0, 0.4));
  transition: transform 0.2s var(--ease-spring);
}
.bottom-tab:hover .bottom-tab-icon { transform: translateY(-1px) scale(1.08); }
.bottom-tab.active .bottom-tab-icon { transform: scale(1.15); }

/* P0-#Z#6：底部 tab 红点（便签 ≤24h 过期） */
.bottom-tab { position: relative; }
.tab-badge {
  position: absolute;
  top: 4px;
  right: 50%;
  transform: translateX(20px);
  min-width: 16px;
  height: 16px;
  padding: 0 4px;
  border-radius: 8px;
  background: linear-gradient(135deg, #ff5e7e 0%, #ff3b5c 100%);
  color: #fff;
  font-size: 10px;
  font-weight: 700;
  line-height: 16px;
  text-align: center;
  box-shadow: 0 2px 6px rgba(255, 59, 92, 0.5), 0 0 0 1.5px rgba(20, 20, 28, 0.9);
  pointer-events: auto;
  animation: badgePop 0.3s var(--ease-spring);
}
@keyframes badgePop {
  0% { transform: translateX(20px) scale(0); }
  60% { transform: translateX(20px) scale(1.2); }
  100% { transform: translateX(20px) scale(1); }
}
.bottom-tab-label {
  font-size: 11px;
  font-weight: 500;
  letter-spacing: 0.2px;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  max-width: 100%;
  line-height: 1.1;
}
.bottom-tab.active .bottom-tab-label {
  font-weight: 700;
  color: #fff;
}

/* P0-#Z#6：底部 tab 右上角红点徽章 */
.bottom-tab {
  position: relative;
}
.bottom-tab-badge {
  position: absolute;
  top: 4px;
  right: 14px;
  min-width: 16px;
  height: 16px;
  border-radius: 8px;
  background: linear-gradient(135deg, #ef4444 0%, #dc2626 100%);
  color: #fff;
  font-size: 10px;
  font-weight: 700;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 0 4px;
  box-shadow: 0 2px 6px rgba(239, 68, 68, 0.5);
  animation: badgePop 0.4s var(--ease-smooth);
  pointer-events: none;
  line-height: 1;
}
@keyframes badgePop {
  from { transform: scale(0); opacity: 0; }
  60% { transform: scale(1.2); opacity: 1; }
  to { transform: scale(1); opacity: 1; }
}

/* 主内容区（占满整个窗口宽度，底部 tab 在下） */
.content {
  flex: 1;
  display: flex;
  flex-direction: column;
  min-width: 0;
  min-height: 0;
  background: transparent; /* 让玻璃感穿透到背景 */
  position: relative;
  z-index: 1;
}
.content-scroll {
  flex: 1;
  overflow-y: auto;
  overflow-x: hidden;
  padding: 12px 14px 16px;
  min-height: 0;
}
.content-scroll::-webkit-scrollbar { width: 6px; }
.content-scroll::-webkit-scrollbar-track { background: transparent; }
.content-scroll::-webkit-scrollbar-thumb { background: var(--bg-tertiary); border-radius: 3px; }
.content-scroll::-webkit-scrollbar-thumb:hover { background: var(--accent); }

/* ===== 响应式：窗口拉大时调整内边距和字号 ===== */
/* 中等宽度（560-800px） */
@media (min-width: 560px) {
  .content-scroll { padding: 14px 20px 20px; }
  .top-header-inner { padding: 12px 20px 10px; }
  .titlebar { padding: 0 12px 0 18px; }
  .titlebar-btn { width: 32px; height: 32px; font-size: 16px; }
  .logo { width: 20px; height: 20px; }
  .title { font-size: 15px; }
}
/* 大宽度（800-1100px）：双列布局的容器 */
@media (min-width: 800px) {
  .content-scroll { padding: 16px 28px 24px; }
  .top-header-inner { padding: 14px 28px 12px; }
  .titlebar { padding: 0 16px 0 22px; }
  .titlebar-btn { width: 32px; height: 32px; font-size: 16px; }
  .logo { width: 22px; height: 22px; }
  .title { font-size: 16px; }
  .bottom-tabs { height: 64px; padding: 0 8px; gap: 4px; }
  .bottom-tab-icon { font-size: 22px; }
  .bottom-tab-label { font-size: 12px; }
}
/* 超大宽度（1100+）：可显示更宽内容 */
@media (min-width: 1100px) {
  .content-scroll { padding: 18px 36px 28px; }
  .top-header-inner { padding: 16px 36px 14px; }
  .titlebar { padding: 0 20px 0 28px; }
  .logo { font-size: 24px; }
  .title { font-size: 18px; }
  .bottom-tabs { height: 68px; }
}

.page-wrapper { min-height: 100%; }

.page-enter-active { animation: pageIn 0.3s var(--ease-out); }
.page-leave-active { animation: pageOut 0.18s var(--ease-smooth); position: absolute; width: calc(100% - 32px); }
@keyframes pageIn {
  from { opacity: 0; transform: translateY(8px); filter: blur(4px); }
  to { opacity: 1; transform: translateY(0); filter: blur(0); }
}
@keyframes pageOut {
  from { opacity: 1; }
  to { opacity: 0; transform: scale(0.98); }
}

/* =================================================== */
/* ============ 迷你模式：极简图标列 =================== */
/* =================================================== */

/* 装饰光晕（浅色更淡） */
.mini-widget::before {
  content: "";
  position: absolute;
  inset: -20px;
  background:
    radial-gradient(circle at 50% 0%, rgba(99, 102, 241, 0.15) 0%, transparent 50%),
    radial-gradient(circle at 50% 100%, rgba(236, 72, 153, 0.12) 0%, transparent 50%);
  pointer-events: none;
  z-index: 0;
}

.mini-widget {
  width: 100%;
  height: 100%;
  background: linear-gradient(180deg, rgba(20, 20, 28, 0.6) 0%, rgba(28, 22, 36, 0.5) 100%);
  backdrop-filter: blur(40px) saturate(180%);
  -webkit-backdrop-filter: blur(40px) saturate(180%);
  border: 1px solid rgba(255, 255, 255, 0.08);
  border-radius: 28px;
  box-shadow:
    0 24px 60px rgba(0, 0, 0, 0.45),
    inset 0 1px 0 rgba(255, 255, 255, 0.08);
  display: flex;
  flex-direction: column;
  align-items: center;
  padding: 6px 0;
  position: relative;
  /* 关键修复：移除 miniIn 入场动画（0.4s scaleX + translateX）
   * 之前切到 mini 模式整窗从右侧滑入缩放 → "闪屏" */
  overflow: hidden;
  color: #ffffff;
}

/* 装饰光晕（角落透出，避免遮挡图标） */
.mini-widget::before {
  content: "";
  position: absolute;
  inset: -20px;
  background:
    radial-gradient(circle at 50% 0%, rgba(79, 124, 255, 0.18) 0%, transparent 50%),
    radial-gradient(circle at 50% 100%, rgba(255, 94, 126, 0.12) 0%, transparent 50%);
  pointer-events: none;
  z-index: 0;
}

.mini-handle {
  width: 36px;
  height: 16px;
  display: flex;
  align-items: center;
  justify-content: center;
  border-radius: 8px;
  color: var(--text-faint);
  cursor: pointer;
  margin-bottom: 6px;
  transition: all 0.2s;
  position: relative;
  z-index: 1;
}
.mini-handle:hover { background: var(--accent-soft); color: var(--accent-bright); }
.mini-handle-icon { font-size: 10px; }

.mini-icons {
  flex: 1;
  display: flex;
  flex-direction: column;
  gap: 6px;
  padding: 4px 0;
  width: 100%;
  align-items: center;
  position: relative;
  z-index: 1;
}

.mini-icon {
  position: relative;
  width: 40px;
  height: 40px;
  border-radius: 12px;
  border: none;
  background: transparent;
  color: var(--text-muted);
  cursor: pointer;
  display: flex;
  align-items: center;
  justify-content: center;
  transition: all 0.25s var(--ease-spring);
}
.mini-icon:hover {
  background: var(--bg-tertiary);
  color: var(--text-primary);
  transform: scale(1.08);
  box-shadow: 0 4px 12px rgba(0, 0, 0, 0.25);
}
.mini-icon.active {
  background: linear-gradient(135deg, var(--accent) 0%, var(--highlight) 100%);
  color: white;
  box-shadow: 0 4px 12px rgba(79, 124, 255, 0.4);
}
.mini-icon-glyph { font-size: 18px; filter: drop-shadow(0 1px 2px rgba(0, 0, 0, 0.3)); }

.mini-icon-dot {
  position: absolute;
  bottom: 2px;
  right: 6px;
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: white;
  box-shadow: 0 0 6px rgba(255, 255, 255, 0.8);
}

.mini-foot {
  width: 100%;
  display: flex;
  justify-content: center;
  padding: 4px 0 2px;
  position: relative;
  z-index: 1;
}
.mini-dot {
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: var(--text-faint);
  transition: all 0.3s;
}
.mini-dot.on {
  background: var(--success);
  box-shadow: 0 0 6px var(--success);
}
</style>

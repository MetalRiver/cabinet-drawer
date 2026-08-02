<script setup lang="ts">
import { ref, computed, onMounted, watch } from "vue";
import { useRouter } from "vue-router";
import { convertFileSrc } from "@tauri-apps/api/core";
import { useAppStore } from "../stores/app";
import { useSoftwareStore } from "../stores/software";
import { useSnippetsStore } from "../stores/snippets";
import { useTempStore } from "../stores/temp";
import { usePasswordStore as usePasswordsStore } from "../stores/passwords";
import { copyToClipboardWithTimeout } from "../api";

const appStore = useAppStore();
const router = useRouter();
const softwareStore = useSoftwareStore();
const snippetsStore = useSnippetsStore();
const tempStore = useTempStore();
const passwordsStore = usePasswordsStore();

// 包装 iconSrc：本地路径 → webview 可访问的 URL（asset protocol）
// 优先用 softwareStore.iconDataUrls[id]（base64 data URL，100% 可渲染）
function _iconSrc(iconPath: string | null | undefined): string {
  if (!iconPath) return "";
  if (iconPath.startsWith("http") || iconPath.startsWith("data:")) return iconPath;
  try {
    return convertFileSrc(iconPath);
  } catch {
    return "";
  }
}

interface FrequentItem {
  type: "password" | "app" | "snippet" | "temp";
  id: number;
  title: string;
  subtitle: string;
  icon: string;          // 软件：实际图标路径；其他：emoji
  iconFallback: string;  // 失败时显示的首字母 / emoji
  appType?: string;      // P0-#FREQ#ICON#SYNC：app 类型的细分（url/folder/document/app）
  color: "blue" | "cyan" | "orange" | "purple";
  useCount: number;
  lastUsedAt: number;
}

// P0-#FREQ#ICON#SYNC：app type → emoji（与软件区 typeEmoji 一致）
// 之前 app 项 url/folder 类型时常用区显示首字母 → 不一致
// 现在用 emoji 兜底（与软件区视觉统一）
function typeFallback(t: string): string {
  switch (t) {
    case "url": return "🔗";
    case "folder": return "📁";
    case "document": return "📄";
    case "game": return "🎮";
    case "office": return "📊";
    case "dev": return "💻";
    case "utility": return "🛠️";
    case "media": return "🎬";
    case "design": return "🎨";
    case "word": return "📘";
    case "excel": return "📗";
    case "ppt": return "📙";
    case "pdf": return "📕";
    case "text": return "📝";
    case "image": return "🖼️";
    case "app":
    default: return "🚀";
  }
}

// P0-#FREQ#ICON#SYNC#2：snippet 按 language 判断 emoji
// ≠text → 💡代码（与 SnippetView 一致）；=text → 📋文本
function snippetIconFallback(language: string | undefined | null): string {
  return language && language !== "text" ? "💡" : "📋";
}

// P0-#FREQ#ICON#SYNC#3：便签图标与 TempContentView 一致（⏳）
// 之前：硬编码 ⏳ 也算对，但和 view 端保持显式一致
function tempIconFallback(): string {
  return "⏳";
}

const items = ref<FrequentItem[]>([]);

// 关键修复：app 类型实时从 store 读 data URL
// 不依赖 watch 链（FrequentView 的 watch 在 setup 阶段注册，store 异步填充时偶尔不 trigger）
// 这样 store 变化时模板自动 reactive 更新
function getIcon(item: FrequentItem): string {
  if (item.type === "app") {
    return softwareStore.iconDataUrls[item.id] || "";
  }
  return item.icon;
}

onMounted(() => {
  void softwareStore.loadItems();
  void snippetsStore.loadItems();
  void passwordsStore.refresh();
  void tempStore.loadItems();
});

watch(
  () => [
    appStore.searchQuery,
    softwareStore.items.length,
    // P0-#Y#FIX#FREQ#SYNC：监听 app_type / app_subtype / icon_path 变化
    // 之前：只监听 use_count / last_used_at → 用户右键改 type 不会触发 refresh
    //  看到"软件区是 🔗 但常用区仍是"创"字"——就是 watch 漏了 app_type
    softwareStore.items
      .map((i) => `${i.id}:${i.use_count}:${i.last_used_at}:${i.app_type}:${i.app_subtype}:${i.icon_path}`)
      .join(","),
    // 关键修复：监听 iconDataUrls 变化，让常用区重新用 data URL 渲染
    Object.keys(softwareStore.iconDataUrls).length,
    snippetsStore.items.length,
    snippetsStore.items.map((i) => `${i.id}:${i.use_count}:${i.last_used_at}:${i.language}`).join(","),
    tempStore.items.length,
    tempStore.items.map((i) => i.id).join(","),
    passwordsStore.items.length,
    passwordsStore.items.map((i) => i.id).join(","),
    // P0-#FREQ#PW#USE#COUNT：监听 useCounts 变化，让密码排序实时更新
    Object.keys(passwordsStore.useCounts).map((k) => `${k}:${passwordsStore.useCounts[Number(k)]}`).join(","),
  ],
  () => refresh()
);

async function refresh() {
  const all: FrequentItem[] = [];
  // 软件
  const allApps = softwareStore.items.slice();
  allApps.sort((a, b) => {
    if (b.use_count !== a.use_count) return b.use_count - a.use_count;
    if (b.last_used_at !== a.last_used_at) return b.last_used_at - a.last_used_at;
    return b.id - a.id;
  });
  for (const a of allApps) {
    all.push({
      type: "app",
      id: a.id,
      title: a.name,
      subtitle: a.path,
      // item.icon 留空：模板里通过 computed getIcon(item) 实时读 softwareStore.iconDataUrls
      // 这样不需要 watch trigger，store 变化时模板自动响应
      icon: "",
      // P0-#FREQ#ICON#SYNC：app 项类型变 url/folder 时不再显示首字母
      // 之前：用户把"创意设计"从 app 改成 url → 软件区有 UrlIconSvg → 常用区仍是"创"字
      // 现在：iconFallback 优先返回 type 对应的 emoji（与软件区副标 fallback 一致）
      iconFallback: typeFallback(a.app_type || "app"),
      appType: a.app_type || "app", // P0-#FREQ#ICON#SYNC：传给模板用
      color: "blue",
      useCount: a.use_count,
      lastUsedAt: a.last_used_at || 0,
    });
  }
  // 命令行（snippets）
  for (const s of snippetsStore.items) {
    all.push({
      type: "snippet",
      id: s.id,
      title: s.title,
      subtitle: s.content.split("\n")[0].slice(0, 40) + (s.content.length > 40 ? "…" : ""),
      icon: "",
      // P0-#FREQ#ICON#SYNC#2：按 language 动态判断，与 SnippetView 视觉一致
      iconFallback: snippetIconFallback(s.language),
      color: "cyan",
      useCount: s.use_count,
      lastUsedAt: s.last_used_at,
    });
  }
  // 密码
  for (const p of passwordsStore.items) {
    if (p.title) {
      all.push({
        type: "password",
        id: p.id,
        title: p.title,
        subtitle: p.username ? `@${p.username}` : "—",
        icon: "",
        iconFallback: p.title.charAt(0).toUpperCase(),
        color: "purple",
        // P0-#FREQ#PW#USE#COUNT：读 in-memory 计数（copy 时累加），让排序实时
        useCount: passwordsStore.getUseCount(p.id),
        lastUsedAt: p.updated_at,
      });
    }
  }
  // 临时
  for (const t of tempStore.items) {
    all.push({
      type: "temp",
      id: t.id,
      title: t.text.split("\n")[0].slice(0, 20),
      subtitle: t.text.length > 20 ? t.text.slice(0, 20) + "…" : t.text,
      icon: "",
      iconFallback: tempIconFallback(),
      color: "orange",
      useCount: 0,
      lastUsedAt: t.created_at,
    });
  }
  // 排序：use_count desc + lastUsedAt desc
  all.sort((a, b) => {
    if (b.useCount !== a.useCount) return b.useCount - a.useCount;
    return b.lastUsedAt - a.lastUsedAt;
  });
  // 全局搜索过滤
  const q = appStore.searchQuery.trim().toLowerCase();
  const filtered = q
    ? all.filter(
        (it) =>
          it.title.toLowerCase().includes(q) ||
          it.subtitle.toLowerCase().includes(q)
      )
    : all;
  items.value = filtered.slice(0, 24); // 3 大卡 + 21 网格
}

// Top 3：use_count 最高的 3 个（用于大卡展示）
const topThree = computed(() => items.value.slice(0, 3));
// 其余走 8 列网格
const gridItems = computed(() => items.value.slice(3));

// 密码折叠：同标题的密码归到第一个，标记 dupCount
const foldedGrid = computed(() => {
  const list = gridItems.value;
  const seen = new Map<string, number>();
  const out: (FrequentItem & { dupCount?: number })[] = [];
  for (const it of list) {
    if (it.type === "password") {
      const key = it.title.toLowerCase();
      if (seen.has(key)) {
        out[seen.get(key)!].dupCount = (out[seen.get(key)!].dupCount || 1) + 1;
        continue;
      }
      seen.set(key, out.length);
    }
    out.push(it);
  }
  return out;
});

async function activate(item: FrequentItem) {
  try {
    if (item.type === "app") {
      // P0-#F：和 AppView 一样，精确错误反馈
      const result = await softwareStore.launch(item.id);
      if (result.ok) {
        appStore.showClipToast("success", `已启动 ${item.title}`);
      } else if (result.error === "missing") {
        appStore.showClipToast("info", `找不到「${item.title}」的安装位置。请重新拖入`);
      } else {
        appStore.showClipToast("info", result.message || "启动失败");
      }
    } else if (item.type === "snippet") {
      await copyToClipboardWithTimeout(item.subtitle.replace(/…$/, ""), 30);
      await snippetsStore.copy(item.id, 30);
      appStore.showClipToast("success", `已复制 "${item.title}"`);
    } else if (item.type === "password") {
      router.push("/passwords");
    } else if (item.type === "temp") {
      router.push("/temp");
    }
  } catch (e) {
    appStore.showClipToast("info", String(e));
  }
}

const typeLabel = {
  app: { label: "软件", color: "blue" },
  snippet: { label: "命令行", color: "cyan" },
  password: { label: "密码", color: "purple" },
  temp: { label: "临时", color: "orange" },
} as const;
const _typeEmoji = {
  app: "🚀",
  password: "🔑",
  snippet: "💻",
  temp: "⏳",
} as const;

// 图标加载失败时切到 fallback（首字母 / emoji）
function onIconError(e: Event) {
  const img = e.target as HTMLImageElement;
  img.style.display = "none";
  const fallback = img.nextElementSibling as HTMLElement | null;
  if (fallback) fallback.style.display = "flex";
}
</script>

<template>
  <div class="view">
    <div class="view-header">
      <div class="view-header-left">
        <div class="view-icon icon-yellow">⭐</div>
        <div class="view-header-info">
          <h2 class="view-title">
            <span class="view-title-text">常用</span>
            <span class="title-count">{{ items.length }}</span>
          </h2>
          <p class="view-subtitle">点一下直达 · 高频项置顶</p>
        </div>
      </div>
    </div>

    <!-- Top 3 大卡：横向 3 列 -->
    <div v-if="topThree.length > 0" class="top-three">
      <button
        v-for="(item, i) in topThree"
        :key="`top-${item.type}-${item.id}`"
        class="top-card tap"
        :class="`top-${typeLabel[item.type].color}`"
        @click="activate(item)"
      >
        <div class="type-stripe" :class="`stripe-${item.type}`"></div>
        <div class="top-rank">#{{ i + 1 }}</div>
        <!-- P0-#Z：火苗徽章放回右上角（缩小，不遮挡） -->
        <span
          v-if="item.useCount > 0"
          class="top-flame-pill"
          :title="`已使用 ${item.useCount} 次`"
        >🔥{{ item.useCount }}</span>
        <div class="top-icon" :class="`icon-${typeLabel[item.type].color}`">
          <img
            v-if="getIcon(item)"
            :src="getIcon(item)"
            :alt="item.title"
            class="top-icon-img"
            @error="onIconError"
          />
          <span
            v-else
            class="top-icon-fallback"
            :class="(item.iconFallback || '').length > 1 ? 'is-emoji' : 'is-letter'"
          >{{ item.iconFallback }}</span>
        </div>
        <div class="top-info">
          <div class="top-title">{{ item.title }}</div>
          <div class="top-meta">
            <span class="top-type">{{ typeLabel[item.type].label }}</span>
          </div>
        </div>
      </button>
    </div>

    <!-- 8 列网格：其余所有项 -->
    <TransitionGroup name="grid" tag="div" class="grid">
      <button
        v-for="item in foldedGrid"
        :key="`grid-${item.type}-${item.id}`"
        class="cell tap"
        :class="`cell-${typeLabel[item.type].color}`"
        :title="item.title"
        @click="activate(item)"
      >
        <div class="cell-icon">
          <img
            v-if="getIcon(item)"
            :src="getIcon(item)"
            :alt="item.title"
            class="cell-icon-img"
            @error="onIconError"
          />
          <span
            v-else
            class="cell-icon-fallback"
            :class="(item.iconFallback || '').length > 1 ? 'is-emoji' : 'is-letter'"
          >{{ item.iconFallback }}</span>
        </div>
        <div class="cell-name">{{ item.title }}</div>
        <div v-if="(item as any).dupCount" class="cell-badge">×{{ (item as any).dupCount }}</div>
        <div v-else-if="item.useCount > 1" class="cell-badge">🔥{{ item.useCount }}</div>
      </button>
    </TransitionGroup>

    <!-- 空状态 -->
    <div v-if="items.length === 0" class="empty">
      <div class="empty-icon">⭐</div>
      <div class="empty-text">还没有使用记录<br />去密码/软件/命令行/临时 区点点看</div>
    </div>
  </div>
</template>

<style scoped>
.view { padding-bottom: 20px; }
/* view-header / view-title 等都用 global 统一样式 */

/* ===== Top 3 大卡 ===== */
.top-three {
  display: grid;
  grid-template-columns: repeat(3, 1fr);
  gap: 8px;
  margin-bottom: 14px;
}
.top-card {
  position: relative;
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 6px;
  padding: 12px 8px 10px;
  border-radius: 12px;
  background: var(--bg-secondary);
  border: 1.5px solid var(--border);
  cursor: pointer;
  transition: all 0.18s var(--ease-spring);
  -webkit-app-region: no-drag;
  overflow: visible;
  isolation: isolate;
}
.top-card::before {
  content: "";
  position: absolute;
  inset: 0;
  background: linear-gradient(135deg, transparent 0%, rgba(255, 255, 255, 0.06) 100%);
  pointer-events: none;
}
.top-card:hover { transform: translateY(-2px); border-color: var(--accent); box-shadow: 0 6px 20px rgba(0, 0, 0, 0.4); }
.top-card:active { transform: translateY(0) scale(0.97); }
.top-rank {
  position: absolute;
  top: 6px;
  left: 8px;
  font-size: 10px;
  font-weight: 700;
  color: #ffc832;
  font-family: monospace;
  z-index: 1;
}
/* P0-#Z：top3 右上角火苗徽章（缩小版，不遮挡） */
.top-flame-pill {
  position: absolute;
  top: 4px;
  right: 4px;
  z-index: 2;
  display: inline-flex;
  align-items: center;
  gap: 1px;
  padding: 1px 4px;
  border-radius: 7px;
  font-size: 9px;
  font-weight: 700;
  line-height: 1.2;
  background: linear-gradient(135deg, rgba(255, 138, 94, 0.95) 0%, rgba(255, 78, 110, 0.95) 100%);
  color: #fff;
  box-shadow: 0 1px 3px rgba(255, 78, 110, 0.4);
  letter-spacing: 0;
  pointer-events: none;
}
.top-icon {
  width: 52px;
  height: 52px;
  border-radius: 12px;
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: 22px;
  font-weight: 700;
  background: var(--bg-tertiary);
  position: relative;
  flex-shrink: 0;
}
.top-icon-img { width: 100%; height: 100%; object-fit: contain; display: block; }
.top-icon-fallback {
  position: absolute;
  inset: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: 26px;
  /* P0-#Y#FIX#FREQ#EMOJI：emoji 不能用 background-clip:text
   * 之前：linear-gradient + -webkit-text-fill-color:transparent → emoji 透明变乱码
   * 现在：emoji 直接显示（用字体 emoji 颜色） + 普通文字（首字母）才用渐变
   * 区分方式：fallback 长度 > 1 → emoji；长度 = 1 → 首字母 */
  line-height: 1;
}
.top-icon-fallback.is-emoji {
  /* emoji 用 emoji 字体渲染（彩色） */
  font-family: "Apple Color Emoji", "Segoe UI Emoji", "Noto Color Emoji", sans-serif;
}
.top-icon-fallback.is-letter {
  background: linear-gradient(135deg, var(--accent) 0%, var(--highlight) 100%);
  -webkit-background-clip: text;
  background-clip: text;
  -webkit-text-fill-color: transparent;
  font-weight: 700;
  font-size: 22px;
}
.top-icon-img[style*="display: none"] + .top-icon-fallback { display: flex !important; }
.top-info { text-align: center; width: 100%; min-width: 0; }
.top-title {
  font-size: 12px;
  font-weight: 600;
  color: var(--text-primary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  line-height: 1.3;
}
.top-meta {
  display: flex;
  justify-content: center;
  align-items: center;
  gap: 6px;
  margin-top: 3px;
  font-size: 10px;
  color: var(--text-muted);
}
.top-type { color: var(--text-faint); }
.top-uses { color: #ff8a5e; font-weight: 600; }

/* ===== 8 列网格 ===== */
.grid {
  display: grid;
  grid-template-columns: repeat(8, 1fr);
  gap: 6px;
}
@media (max-width: 559px) {
  .grid { grid-template-columns: repeat(6, 1fr); }
}
@media (min-width: 700px) {
  .grid { grid-template-columns: repeat(8, 1fr); gap: 8px; }
}
@media (min-width: 900px) {
  .grid { grid-template-columns: repeat(10, 1fr); gap: 10px; }
}
.cell {
  position: relative;
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 4px;
  padding: 8px 2px 6px;
  border-radius: 10px;
  background: var(--bg-secondary);
  border: 1px solid var(--border);
  cursor: pointer;
  transition: all 0.15s var(--ease-smooth);
  -webkit-app-region: no-drag;
  overflow: hidden;
}
.cell:hover {
  background: var(--bg-tertiary);
  border-color: var(--accent-soft);
  transform: translateY(-1px);
}
.cell:active { transform: scale(0.95); }
.cell-icon {
  width: 32px;
  height: 32px;
  border-radius: 8px;
  display: flex;
  align-items: center;
  justify-content: center;
  background: var(--bg-tertiary);
  position: relative;
  flex-shrink: 0;
  overflow: hidden;
}
.cell-icon-img { width: 100%; height: 100%; object-fit: contain; display: block; }
.cell-icon-fallback {
  position: absolute;
  inset: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: 16px;
  line-height: 1;
  /* P0-#Y#FIX#FREQ#EMOJI：emoji 不能用 background-clip:text */
}
.cell-icon-fallback.is-emoji {
  font-family: "Apple Color Emoji", "Segoe UI Emoji", "Noto Color Emoji", sans-serif;
}
.cell-icon-fallback.is-letter {
  background: linear-gradient(135deg, var(--accent) 0%, var(--highlight) 100%);
  -webkit-background-clip: text;
  background-clip: text;
  -webkit-text-fill-color: transparent;
  font-weight: 700;
  font-size: 14px;
}
.cell-icon-img[style*="display: none"] + .cell-icon-fallback { display: flex !important; }
.cell-name {
  font-size: 10px;
  font-weight: 500;
  color: var(--text-secondary);
  text-align: center;
  width: 100%;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  line-height: 1.2;
}
.cell-badge {
  position: absolute;
  top: 2px;
  right: 2px;
  font-size: 8px;
  font-weight: 700;
  background: var(--highlight);
  color: white;
  padding: 0 4px;
  border-radius: 6px;
  line-height: 1.3;
}

.empty { text-align: center; padding: 60px 20px; color: var(--text-muted); }
.empty-icon { font-size: 48px; margin-bottom: 12px; opacity: 0.5; }
.empty-text { font-size: 12px; line-height: 1.6; }

.grid-enter-active, .grid-leave-active { transition: all 0.3s var(--ease-smooth); }
.grid-enter-from, .grid-leave-to { opacity: 0; transform: scale(0.8); }
</style>

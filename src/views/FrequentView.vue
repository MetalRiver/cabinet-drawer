<script setup lang="ts">
import { ref, computed, onMounted, watch, onUnmounted } from "vue";
import { useRouter } from "vue-router";
import { useAppStore } from "../stores/app";
import { useSoftwareStore } from "../stores/software";
import { launchCoolingDown, launchMarkAccepted, launchSuccessToast, launchFailureToast, nextLaunchSeq, isLatestLaunch } from "../lib/launch-feedback";
import { assetUrl as moduleAssetUrl, assetFailed as moduleAssetFailed, onAssetError as moduleOnAssetError } from "../lib/app-icon-state";
import { useSnippetsStore } from "../stores/snippets";
import { useTempStore } from "../stores/temp";
import { usePasswordStore as usePasswordsStore } from "../stores/passwords";

const appStore = useAppStore();
const router = useRouter();
const softwareStore = useSoftwareStore();
const snippetsStore = useSnippetsStore();
const tempStore = useTempStore();
const passwordsStore = usePasswordsStore();

interface FrequentItem {
  type: "password" | "app" | "snippet" | "temp";
  id: number;
  title: string;
  subtitle: string;
  icon: string;          // 软件：实际图标路径；其他：emoji
  iconPath?: string;     // G3-1B：软件记录的 icon_path（资产 URL 主路径用）
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
// M1-P2（F1）：与软件区完全一致——统一走 app-icon-state 模块（asset URL 主路径 + 版本化 +
// 受控回退 + 恢复重试）；原「base64 优先」与注释矛盾的实现一并修正，消除两视图机制分裂。
function getIcon(item: FrequentItem): string {
  if (item.type === "app") {
    if (item.iconPath && !moduleAssetFailed(item.id)) return moduleAssetUrl(item.id, item.iconPath);
    if (softwareStore.iconDataUrls[item.id]) return softwareStore.iconDataUrls[item.id];
    return "";
  }
  // M1-P2：非 app 类型（snippet/password/temp）不再渲染 img（src=emoji 本就不可加载，
  // 旧 DOM 隐藏 hack 依赖不存在的 v-else 兄弟节点）——返回空让 iconFallback span 正常显示。
  return "";
}

onMounted(() => {
  // O3：loadItems → loadItemsIfNeeded——本视图是解锁后默认路由的首个挂载点，
  //   正常路径这里就是首次全量加载；若列表已加载（如锁屏-解锁后的重挂载）则跳过
  //   重复的全量拉取。CRUD/扫描/reloadAll 的强制刷新仍走 loadItems（语义不变）。
  void softwareStore.loadItemsIfNeeded();
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
      iconPath: a.icon_path,
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

const launchingIds = ref(new Set<number>());
async function activate(item: FrequentItem) {
  try {
    if (item.type === "app") {
      // 2026-11-06：与软件区完全一致——拦截给提示、点击即确认、序号守卫防迟到覆盖
      const name = item.title;
      if (launchingIds.value.has(item.id) || launchCoolingDown(item.id)) {
        appStore.showClipToast("info", `⏳ 「${name}」正在处理中，请勿重复启动`, 1200);
        return;
      }
      const seq = nextLaunchSeq();
      launchingIds.value.add(item.id);
      appStore.showClipToast("info", `🚀 已收到启动请求：「${name}」`, 3000);
      try {
        const result = await softwareStore.launch(item.id);
        if (result.ok) launchMarkAccepted(item.id);
        if (!isLatestLaunch(seq)) return;
        const path = softwareStore.items.find((it) => it.id === item.id)?.path;
        const info = { name: item.title, path, app_type: item.appType };
        if (result.ok) {
          launchSuccessToast((kind, msg, dur) => appStore.showClipToast(kind, msg, dur), info);
        } else {
          launchFailureToast((kind, msg, dur) => appStore.showClipToast(kind, msg, dur), info, result);
        }
      } finally {
        launchingIds.value.delete(item.id);
      }
    } else if (item.type === "snippet") {
      await snippetsStore.copy(item.id);
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

// 图标加载失败时切到 fallback（首字母 / emoji）
// M1-P2（F1）：不再用 DOM display hack——与软件区同走统一状态模块
// （asset 失败 → 受控回退 → SVG/emoji 兜底 → 单次恢复重试），失败状态随模块响应式驱动模板。
// 非 app 类型不产生 img（getIcon 返回空），不会进入本函数。
function onIconError(item: FrequentItem) {
  if (item.type === "app" && item.iconPath) {
    moduleOnAssetError(item.id, item.iconPath);
  }
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
        :class="[`top-${typeLabel[item.type].color}`, { 'launching': launchingIds.has(item.id) }]"
        @click="activate(item)"
      >
        <div class="type-stripe" :class="`stripe-${item.type}`"></div>
        <!-- R6-02C-05（PL-071）：零使用记录不显示排名——排序仍按 use_count，但「#N 高频」语义只属于真实使用过的条目 -->
        <div v-if="item.useCount > 0" class="top-rank">#{{ i + 1 }}</div>
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
            @error="onIconError(item)"
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
        :class="[`cell-${typeLabel[item.type].color}`, { 'launching': launchingIds.has(item.id) }]"
        :title="item.title"
        @click="activate(item)"
      >
        <div class="cell-icon">
          <img
            v-if="getIcon(item)"
            :src="getIcon(item)"
            :alt="item.title"
            class="cell-icon-img"
            @error="onIconError(item)"
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
      <div class="empty-text">这里会自动收录你添加的软件、密码、命令和便签<br />去各专区添加内容后，就会出现在这里，点一下直达</div>
    </div>
  </div>
</template>

<style scoped>
.view { padding-bottom: 20px; }
/* view-header / view-title 等都用 global 统一样式 */

/* ===== Top 3 大卡 ===== */
.top-three {
  display: grid;
  /* minmax(0,1fr)：禁止网格项按 nowrap 标题的 min-content 撑宽（否则整页横向溢出被裁） */
  grid-template-columns: repeat(3, minmax(0, 1fr));
  gap: 8px;
  margin-bottom: 14px;
}
.top-card {
  position: relative;
  display: flex;
  flex-direction: column;
  min-width: 0;
  align-items: center;
  gap: 5px;
  padding: 10px 8px 9px; /* 2026-11-08 尺寸收敛：12/10 → 10/9（轻量收敛，不改结构） */
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
/* 2026-11-08：hover 去位移、阴影减负（与软件区同策略），保留描边高亮 */
.top-card:hover { border-color: var(--accent); box-shadow: 0 4px 12px rgba(0, 0, 0, 0.3); }
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
  /* 2026-11-08 尺寸收敛：52 → 44（与软件区 app-icon 一致） */
  width: 44px;
  height: 44px;
  border-radius: 11px;
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: 19px;
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
  font-size: 22px;
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
.top-info { text-align: center; width: 100%; min-width: 0; max-width: 100%; }
.top-title {
  font-size: 12px;
  font-weight: 600;
  color: var(--text-primary);
  max-width: 100%;
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

/* ===== 8 列网格 ===== */
.grid {
  display: grid;
  /* 所有档位都用 minmax(0,1fr)：列宽可收缩，标题省略，不再撑破容器 */
  grid-template-columns: repeat(8, minmax(0, 1fr));
  gap: 6px;
}
@media (max-width: 419px) {
  .grid { grid-template-columns: repeat(4, minmax(0, 1fr)); }
  .top-card { padding: 10px 6px 8px; gap: 4px; }
  .top-icon { width: 42px; height: 42px; }
  .top-title { font-size: 11px; }
}
@media (min-width: 420px) and (max-width: 559px) {
  .grid { grid-template-columns: repeat(5, minmax(0, 1fr)); }
}
@media (min-width: 700px) {
  .grid { grid-template-columns: repeat(auto-fill, minmax(96px, 1fr)); gap: 8px; }
}
@media (min-width: 900px) {
  .grid { grid-template-columns: repeat(auto-fill, minmax(96px, 1fr)); gap: 10px; }
}
.cell {
  position: relative;
  display: flex;
  flex-direction: column;
  min-width: 0;
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
.cell-name {
  font-size: 10px;
  font-weight: 500;
  color: var(--text-secondary);
  text-align: center;
  width: 100%;
  max-width: 100%;
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

/* 2026-11-05：白色方块修复——.tap::after 涟漪用 currentcolor（卡片文字色=白），
   点击时 scale(1.6)+白 12% 覆盖图标区域，被用户当成"白色方块"。
   常用页卡片涟漪改用主题靛色（与 accent 体系一致），不再出现白色覆盖。 */
.top-card.tap::after,
.grid .cell.tap::after {
  background: var(--accent-bright);
}
/* 2026-11-05：启动中视觉态（与软件区 app-cell-launching 一致：即时可见反馈） */
.launching {
  animation: freqLaunchPulse 0.9s var(--ease-smooth) infinite;
}
@keyframes freqLaunchPulse {
  0%, 100% { box-shadow: 0 0 0 0 rgba(99, 102, 241, 0.55); }
  50% { box-shadow: 0 0 0 6px rgba(99, 102, 241, 0); }
}
/* （2026-11-06：::after 徽章已移除——与全局 .tap::after 涟漪级联合并会变成
   整卡不透明覆盖层；启动中反馈由脉冲动画 + 3s 结果 toast 承担，视觉更干净） */
</style>

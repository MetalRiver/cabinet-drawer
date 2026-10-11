<script setup lang="ts">
// ===== Settings UI V2 · 高级与更新 Tab =====
// 更新走既有 Tauri Updater（通道/签名不动）；图标诊断两个真实动作保留内联确认。
import { ref, computed, onUnmounted } from "vue";
import { getAppVersion, restartApp } from "../../../api";
import { useAppStore } from "../../../stores/app";
import { useSoftwareStore } from "../../../stores/software";
import ConfirmInline from "../../../components/ConfirmInline.vue";
import { RELEASES_PAGE_URL, VERSION_HISTORY, findInstalledEntry } from "../../../data/version-history";
import SettingsCard from "../../../components/settings/SettingsCard.vue";
import StatusLine from "../../../components/settings/StatusLine.vue";
import { useSettingsOps } from "../ops";

const appStore = useAppStore();
const softwareStore = useSoftwareStore();
const ops = useSettingsOps();

const versionInfo = ref<string>("");
const updateBusy = ref(false);
const updateResult = ref("");
const updateReady = ref<null | { version: string; notes: string; download: () => Promise<void> }>(null);
const updateProgress = ref<string>("");
getAppVersion().then((v) => (versionInfo.value = v)).catch(() => {});

async function onCheckUpdate() {
  updateBusy.value = true;
  updateResult.value = "正在检查更新…";
  updateReady.value = null;
  updateProgress.value = "";
  ops.begin("update", "检查更新", "advanced");
  try {
    const { check } = await import("@tauri-apps/plugin-updater");
    const update = await check();
    if (!update) {
      updateResult.value = `✓ 当前已是最新版本（${versionInfo.value}）`;
      return;
    }
    updateReady.value = {
      version: update.version,
      notes: update.body || "",
      download: async () => {
        let total = 0, received = 0;
        await update.downloadAndInstall((event) => {
          if (event.event === "Started") { total = event.data.contentLength ?? 0; }
          else if (event.event === "Progress") { received += event.data.chunkLength; }
          else if (event.event === "Finished") { received = total; }
          updateProgress.value = total > 0
            ? `下载中 ${Math.round((received / total) * 100)}%（${Math.round(received / 1024)} / ${Math.round(total / 1024)} KB）`
            : `下载中 ${Math.round(received / 1024)} KB`;
        });
      },
    };
    updateResult.value = `发现新版本 v${update.version}` + (update.body ? `：${update.body}` : "");
  } catch (e) {
    updateResult.value = "✗ 检查更新失败：" + e + "（请检查网络连接）";
  } finally {
    updateBusy.value = false;
    ops.end("update");
  }
}

async function onUpdateInstall() {
  if (!updateReady.value) return;
  updateBusy.value = true;
  ops.begin("update", "下载并安装更新", "advanced");
  try {
    await updateReady.value.download();
    updateResult.value = "✓ 下载完成，即将重启以完成安装…";
    const { relaunch } = await import("@tauri-apps/plugin-process");
    setTimeout(() => { relaunch().catch(() => {}); }, 1200);
  } catch (e) {
    updateResult.value = "✗ 下载/安装失败：" + e + "。当前安装未受影响，可稍后重试。";
  } finally {
    updateBusy.value = false;
    ops.end("update");
  }
}

// ===== 图标诊断（真实动作 + 内联确认）=====
const iconBusy = ref(false);
const lastIconResult = ref("");
const iconClearConfirmOpen = ref(false);
const restartConfirmOpen = ref(false);

async function doReextractIcons() {
  iconBusy.value = true;
  lastIconResult.value = "正在清空缓存…";
  try {
    const n = await softwareStore.refreshAllIcons({ forceClear: true });
    lastIconResult.value = `✓ 已清空并重新抽图（${n} 个软件）`;
    appStore.showClipToast("success", `已重新抽图 ${n} 个软件`);
  } catch (e) {
    lastIconResult.value = `✗ 失败：${e}`;
  } finally {
    iconBusy.value = false;
  }
}
async function onReextractIcons() {
  if (iconBusy.value) return;
  if (!iconClearConfirmOpen.value) {
    iconClearConfirmOpen.value = true;
    return;
  }
  iconClearConfirmOpen.value = false;
  await doReextractIcons();
}
async function onRestartApp() {
  if (iconBusy.value) return;
  if (!restartConfirmOpen.value) {
    restartConfirmOpen.value = true;
    return;
  }
  restartConfirmOpen.value = false;
  iconBusy.value = true;
  try {
    await restartApp();
  } catch (e) {
    lastIconResult.value = `✗ 重启失败：${e}`;
    iconBusy.value = false;
  }
}
// ===== v0.4.1 任务 B：版本更新历史（随应用打包的单一源快照，离线可查）=====
const expandedNotes = ref<Set<string>>(new Set());
const installedEntry = computed(() => findInstalledEntry(versionInfo.value));
function toggleNotes(version: string) {
  const next = new Set(expandedNotes.value);
  if (next.has(version)) next.delete(version);
  else next.add(version);
  expandedNotes.value = next;
}
async function openExternal(url: string) {
  try {
    const shell = await import("@tauri-apps/plugin-shell");
    await shell.open(url);
  } catch (e) {
    console.error("打开外部链接失败", e);
  }
}
onUnmounted(() => {
  // 该 Tab 不含敏感数据，无清理项；保留钩子位以对齐生命周期约定
});
</script>

<template>
  <div class="tab-pane">
    <SettingsCard
      title="版本与更新"
      subtitle="检查更新通过 GitHub Releases 进行，更新包经签名校验后由你确认安装；Gitee 提供手动下载镜像。"
      :badge="{ text: `v${versionInfo || '…'}`, tone: 'purple' }"
    >
      <div class="s-action-row" style="border-top: none; padding-top: 0;">
        <button class="s-btn-primary" @click="onCheckUpdate" :disabled="updateBusy" data-interactive>
          {{ updateBusy && !updateReady ? "检查中…" : "🔍 检查更新" }}
        </button>
        <button v-if="updateReady && !updateBusy" class="s-btn-primary s-btn-green" @click="onUpdateInstall" data-interactive>
          ⬇️ 下载并安装 v{{ updateReady.version }}
        </button>
      </div>
      <StatusLine v-if="updateProgress" tone="info" :text="updateProgress" />
      <StatusLine
        v-if="updateResult"
        :tone="updateResult.startsWith('✗') ? 'err' : 'ok'"
        :text="updateResult"
      />
    </SettingsCard>

    <SettingsCard
      title="版本更新历史"
      subtitle="随应用打包的历史记录（离线可查）；单一事实源为 GitHub Releases，发布新版本时同步登记。"
    >
      <div class="vh-list">
        <div
          v-for="entry in VERSION_HISTORY"
          :key="entry.version"
          class="vh-item"
          :class="{ installed: installedEntry?.version === entry.version }"
        >
          <div class="vh-row">
            <div class="vh-info">
              <div class="vh-version">
                {{ entry.version }}
                <span v-if="installedEntry?.version === entry.version" class="vh-now">本机安装</span>
              </div>
              <div class="vh-date">{{ entry.date }} · {{ entry.summary }}</div>
            </div>
            <div class="vh-actions">
              <button class="s-btn-mini tap" data-interactive @click="toggleNotes(entry.version)">
                {{ expandedNotes.has(entry.version) ? "收起详情" : "更新内容" }}
              </button>
              <button class="s-btn-mini tap" data-interactive @click="openExternal(entry.url)">
                GitHub ↗
              </button>
            </div>
          </div>
          <pre v-if="expandedNotes.has(entry.version)" class="vh-notes">{{ entry.notes }}</pre>
        </div>
      </div>
      <div class="s-action-row" style="border-top: none; padding-top: 0;">
        <button class="s-btn-secondary" data-interactive @click="openExternal(RELEASES_PAGE_URL)">
          🔗 在 GitHub 查看全部 Releases
        </button>
      </div>
    </SettingsCard>

    <SettingsCard
      title="软件图标诊断"
      subtitle="图标与资源管理器显示不一致时：先清空缓存重抽，再重启应用让它自动重新抽图。"
    >
      <div class="s-action-row" style="border-top: none; padding-top: 0;">
        <button class="s-btn-primary" @click="onReextractIcons" :disabled="iconBusy" data-interactive>
          {{ iconBusy ? "处理中…" : "🗑️ 清空缓存 + 重新抽图" }}
        </button>
        <button class="s-btn-secondary" @click="onRestartApp" :disabled="iconBusy" data-interactive>🔄 重启应用</button>
      </div>
      <ConfirmInline
        v-if="iconClearConfirmOpen"
        title="确定要清空所有图标缓存吗？清空后抽屉柜会自动重新抽图，首次启动会慢 1-2 分钟。"
        confirm-text="确认清空重抽"
        @cancel="iconClearConfirmOpen = false"
        @confirm="onReextractIcons"
      />
      <ConfirmInline
        v-if="restartConfirmOpen"
        title="确定要重启抽屉柜吗？（用于让 webview 重新加载新代码）"
        confirm-text="确认重启"
        @cancel="restartConfirmOpen = false"
        @confirm="onRestartApp"
      />
      <StatusLine v-if="lastIconResult" :tone="lastIconResult.startsWith('✗') ? 'err' : 'ok'" :text="lastIconResult" />
    </SettingsCard>
  </div>
</template>

<style scoped>
/* ===== 版本更新历史 ===== */
.vh-list { display: flex; flex-direction: column; gap: 8px; }
.vh-item {
  border: 1px solid var(--border);
  border-radius: 9px;
  background: var(--bg-secondary);
  padding: 10px 12px;
}
.vh-item.installed { border-color: rgba(16, 185, 129, 0.45); }
.vh-row { display: flex; align-items: center; justify-content: space-between; gap: 10px; flex-wrap: wrap; }
.vh-info { min-width: 0; flex: 1; }
.vh-version { font-size: 13px; font-weight: 700; color: var(--text-primary); display: flex; align-items: center; gap: 8px; }
.vh-now {
  font-size: 10.5px; font-weight: 600; padding: 1px 8px; border-radius: 999px;
  color: #34d399; background: rgba(16, 185, 129, 0.12); border: 1px solid rgba(16, 185, 129, 0.4);
}
.vh-date { font-size: 11.5px; color: var(--text-muted); margin-top: 2px; }
.vh-actions { display: flex; gap: 6px; flex-shrink: 0; }
.vh-notes {
  margin: 10px 0 0;
  padding: 10px 12px;
  background: var(--bg-primary);
  border: 1px solid var(--border);
  border-radius: 8px;
  font-size: 12px;
  line-height: 1.7;
  font-family: inherit;
  white-space: pre-wrap;
  word-break: break-word;
  color: var(--text-secondary);
  max-height: 320px;
  overflow-y: auto;
}
</style>

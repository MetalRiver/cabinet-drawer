<script setup lang="ts">
import { ref, reactive, watch, onMounted, onUnmounted, computed } from "vue";
import { useRouter } from "vue-router";
import { useWidgetStore } from "../stores/widget";
import { useSoftwareStore } from "../stores/software";
import { useAppStore } from "../stores/app";
import { usePasswordStore } from "../stores/passwords";
import { useTempStore } from "../stores/temp";
import { useSnippetsStore } from "../stores/snippets";
import {
  setWindowSize,
  setWindowPosition,
  getDataDir,
  setupBeginMigration,
  getDataRootSummary,
  restartApp,
  getAppVersion,
  pickPath,
  hardPurgeTrash,
  getSetting,
  setSetting,
  changeMasterPassword,
  changeV2MasterPassword,
  getSecurityStatus,
  prepareV2RecoveryRotation,
  confirmV2RecoveryRotation,
  cancelV2RecoveryRotation,
  rescuePasswordsWithMaster,
  hasSecondPassword as apiHasSecondPassword,
  changeSecondPassword,
  exportEncryptedBackup,
  importEncryptedBackup,
  factoryReset,
  setupBeginRestoreDefault,
  deleteRetainedSource,
  openRetainedSourceFolder,
  type ConflictPolicy,
  type ImportStats,
  type ExportResult,
  type RetainedSourceInfo,
} from "../api";
import ConfirmInline from "../components/ConfirmInline.vue";

const widgetStore = useWidgetStore();
const softwareStore = useSoftwareStore();
const appStore = useAppStore();
const passwordStore = usePasswordStore();
const tempStore = useTempStore();
const snippetsStore = useSnippetsStore();
const router = useRouter();

// ========== 方案③：设置页重构 5 tabs ==========
// 原 4 tab (通用/桌面插件框/数据与维护/高级) → 重分为 5 tab，按用户心智分组
const SETTING_TABS = [
  { key: "basic",    label: "基础",         icon: "🛠️" },
  { key: "security", label: "安全与密码",   icon: "🔒" },
  { key: "widget",   label: "桌面插件框",   icon: "🖼️" },
  { key: "data",     label: "数据维护",     icon: "📦" },
  { key: "advanced", label: "高级",         icon: "🔬" },
];
const activeTab = ref<string>("basic");
function selectTab(key: string) {
  activeTab.value = key;
  const el = document.getElementById(`tab-${key}`);
  const tabsBar = document.querySelector<HTMLElement>(".settings-tabs");
  const scrollEl = document.querySelector<HTMLElement>(".content-scroll")
    || (document.scrollingElement as HTMLElement | null);
  if (!el || !scrollEl) return;
  const tabsH = tabsBar?.offsetHeight ?? 62;
  const scrollRect = scrollEl.getBoundingClientRect();
  const elRect = el.getBoundingClientRect();
  const relTop = elRect.top - scrollRect.top;
  const target = scrollEl.scrollTop + relTop - tabsH - 10;
  scrollEl.scrollTo({ top: target, behavior: "smooth" });
}

// ========== 数据目录 ==========
const dataDir = ref<string>("加载中...");
const dataDirCopied = ref(false);
async function loadDataDir() {
  try { dataDir.value = await getDataDir(); }
  catch (e) { dataDir.value = `(加载失败: ${e})`; }
}
async function copyDataDir() {
  try {
    await navigator.clipboard.writeText(dataDir.value);
    dataDirCopied.value = true;
    setTimeout(() => (dataDirCopied.value = false), 1500);
  } catch (e) { console.error("复制失败", e); }
}
onMounted(loadDataDir);

// ========== Phase 2C-3：更改数据存储位置（事务式迁移） ==========
const migrationTarget = ref<string>("");
const migrationBusy = ref(false);
const migrationRunning = ref(false);
const migrationDone = ref<{ source: string; target: string; archive: string } | null>(null);
async function loadMigrationInfo() {
  try {
    const s = await getDataRootSummary();
    if (s.last_migration) migrationDone.value = s.last_migration;
  } catch { /* 摘要不可用时静默 */ }
}
onMounted(loadMigrationInfo);
async function migrationPick() {
  const p = await pickPath({ mode: "folder", title: "选择新的数据存储位置（需为空目录）" });
  if (p) {
    if (p.trim().toLowerCase() === dataDir.value.trim().toLowerCase()) {
      migrationTarget.value = "";
      alert("新位置与当前数据位置相同。");
      return;
    }
    migrationTarget.value = p;
  }
}
async function migrationStart() {
  if (migrationBusy.value || !migrationTarget.value) return;
  migrationBusy.value = true;
  migrationRunning.value = true;
  try {
    // 成功后后端会自行重启应用（迁移收尾在新进程 setup 内完成）
    await setupBeginMigration(migrationTarget.value);
  } catch (e) {
    migrationRunning.value = false;
    migrationBusy.value = false;
    alert(String(e));
  }
}

// ========== Phase 2C-4：恢复默认位置 + 旧数据副本管理 ==========
const restoreConfirmOpen = ref(false);
const restoreBusy = ref(false);
const restoreRunning = ref(false);
const defaultDir = ref<string>("");
const retainedSources = ref<RetainedSourceInfo[]>([]);
const canRestoreDefault = computed(() => {
  // dataDir 与 config_root 均来自 Rust PathBuf 字符串，大小写归一即可比较
  const norm = (p: string) => p.trim().toLowerCase();
  return (
    defaultDir.value !== "" &&
    dataDir.value !== "加载中..." &&
    norm(dataDir.value) !== norm(defaultDir.value)
  );
});
async function loadRestoreInfo() {
  try {
    const s = await getDataRootSummary();
    defaultDir.value = s.config_root ?? "";
    retainedSources.value = (s.retained_sources ?? []).filter((r) => r.status === "retained");
  } catch { /* 摘要不可用时静默 */ }
}
onMounted(loadRestoreInfo);
async function restoreStart() {
  if (restoreBusy.value) return;
  restoreBusy.value = true;
  restoreRunning.value = true;
  try {
    // 成功后后端会自行重启应用（恢复收尾在新进程 setup 内完成）
    await setupBeginRestoreDefault();
  } catch (e) {
    restoreRunning.value = false;
    restoreBusy.value = false;
    alert(String(e));
  }
}
async function openRetained(opId: string) {
  try {
    deleteError.value = "";
    await openRetainedSourceFolder(opId);
  } catch (e) {
    // fail visible（Tauri webview 中 alert 无效，必须内联显示）
    deleteError.value = String(e);
  }
}
// 不可逆操作：两段内联确认（Tauri webview 无原生 confirm，必须用应用内组件）
const pendingDeleteOp = ref<string | null>(null);
const deleteStage = ref<1 | 2>(1);
const deleteError = ref<string>("");
function removeRetained(opId: string) {
  pendingDeleteOp.value = opId;
  deleteStage.value = 1;
  deleteError.value = "";
}
function cancelDelete() {
  pendingDeleteOp.value = null;
  deleteStage.value = 1;
}
async function confirmDelete(opId: string) {
  if (deleteStage.value === 1) { deleteStage.value = 2; return; }
  try {
    await deleteRetainedSource(opId);
  } catch (e) {
    deleteError.value = String(e);
  }
  pendingDeleteOp.value = null;
  deleteStage.value = 1;
  await loadRestoreInfo();
}

// ========== 行为（自动锁定 / 临时内容 / 剪贴板）==========
const autoLockMinutes = ref(widgetStore.autoLockMinutes);
const tempExpireDays = ref(widgetStore.tempExpireDays);
const clipboardSeconds = ref(widgetStore.clipboardClearSeconds);
watch(() => widgetStore.autoLockMinutes, (v) => autoLockMinutes.value = v);
watch(() => widgetStore.tempExpireDays, (v) => tempExpireDays.value = v);
watch(() => widgetStore.clipboardClearSeconds, (v) => clipboardSeconds.value = v);
watch(autoLockMinutes, async (v) => { await widgetStore.setAutoLockMinutes(Number(v)); });
watch(tempExpireDays, async (v) => { await widgetStore.setTempExpireDays(Number(v)); });
watch(clipboardSeconds, async (v) => { await widgetStore.setClipboardClearSeconds(Number(v)); });

// ========== 回收站 ==========
const trashRetentionDays = ref<number>(30);
watch(trashRetentionDays, async (v) => {
  try { await setSetting("trash_retention_days", String(v)); } catch {}
});
async function onOpenTrash() {
  router.push("/");
  window.dispatchEvent(new CustomEvent("open-trash"));
}
async function onClearTrash() {
  if (!confirm("确定要立即清空回收站吗？\n（所有已删除项将被永久删除）")) return;
  try {
    const n = await hardPurgeTrash();
    alert(`已清空 ${n} 项`);
  } catch (e) { alert("清空失败：" + e); }
}

// ========== 主密码 ==========
const currentPassword = ref("");
const newPassword = ref("");
const confirmPassword = ref("");
const pwChanging = ref(false);
const pwChangeResult = ref("");
const securityModel = ref<"legacy_security_model" | "stable_dek_v2" | null>(null);
const isV2Security = computed(() => securityModel.value === "stable_dek_v2");
async function loadSecurityModel() {
  try { securityModel.value = (await getSecurityStatus()).security_model; }
  catch { securityModel.value = null; }
}
onMounted(loadSecurityModel);
function clearMasterPasswordInputs() {
  currentPassword.value = "";
  newPassword.value = "";
  confirmPassword.value = "";
}
onUnmounted(clearMasterPasswordInputs);
async function onChangePassword() {
  if (pwChanging.value) return;
  if (!currentPassword.value) { pwChangeResult.value = "请输入当前密码"; return; }
  if (newPassword.value.length < 6) { pwChangeResult.value = "新密码至少 6 位"; return; }
  if (newPassword.value !== confirmPassword.value) { pwChangeResult.value = "两次输入的新密码不一致"; return; }
  if (newPassword.value === currentPassword.value) { pwChangeResult.value = "新密码不能与当前密码相同"; return; }
  if (securityModel.value === null) { pwChangeResult.value = "无法确认当前安全模式，请重试"; return; }
  if (!isV2Security.value && !confirm("修改主密码将重新加密所有密码和恢复短语（约 1-3 秒），确定吗？")) return;
  pwChanging.value = true;
  pwChangeResult.value = isV2Security.value ? "正在保存…" : "正在重加密…";
  try {
    if (isV2Security.value) {
      await changeV2MasterPassword(currentPassword.value, newPassword.value);
      pwChangeResult.value = "✓ 主密码已更新，恢复短语和密码数据保持不变";
    } else {
      const stats = await changeMasterPassword(currentPassword.value, newPassword.value);
      void passwordStore.refresh();
      const lines = [`✓ 已重加密 ${stats.passwords_reencrypted} 条密码`];
      if (stats.recovery_phrase_reencrypted) lines.push("✓ 恢复短语已重加密");
      if (stats.passwords_failed > 0) lines.push(`⚠ ${stats.passwords_failed} 条无法解密（已跳过）`);
      pwChangeResult.value = lines.join(" · ");
    }
    appStore.showClipToast("success", "主密码已更新");
    clearMasterPasswordInputs();
  } catch (e) {
    pwChangeResult.value = "✗ 失败：" + String(e);
  } finally { pwChanging.value = false; }
}

// ========== v2 Recovery Phrase 两阶段轮换 ==========
const recoveryRotationStep = ref<"idle" | "words" | "confirm">("idle");
const recoveryRotationWords = ref<string[]>([]);
const recoveryRotationToken = ref("");
const recoveryRotationIndexes = ref<number[]>([]);
const recoveryRotationInputs = ref<string[]>(["", "", ""]);
const recoveryRotationSaved = ref(false);
const recoveryRotationBusy = ref(false);
const recoveryRotationHint = ref("");
const recoveryRotationMatches = computed(() =>
  recoveryRotationIndexes.value.length === 3 &&
  recoveryRotationIndexes.value.every(
    (wordIndex, inputIndex) =>
      recoveryRotationInputs.value[inputIndex].trim().toLowerCase() ===
      recoveryRotationWords.value[wordIndex]
  )
);
function clearRecoveryRotationLocal() {
  recoveryRotationStep.value = "idle";
  recoveryRotationWords.value = [];
  recoveryRotationToken.value = "";
  recoveryRotationIndexes.value = [];
  recoveryRotationInputs.value = ["", "", ""];
  recoveryRotationSaved.value = false;
}
async function startRecoveryRotation() {
  if (recoveryRotationBusy.value || !isV2Security.value) return;
  recoveryRotationBusy.value = true;
  recoveryRotationHint.value = "";
  try {
    const prepared = await prepareV2RecoveryRotation();
    recoveryRotationToken.value = prepared.rotation_token;
    recoveryRotationWords.value = prepared.recovery_words;
    recoveryRotationIndexes.value = prepared.confirmation_indexes;
    recoveryRotationInputs.value = ["", "", ""];
    recoveryRotationSaved.value = false;
    recoveryRotationStep.value = "words";
  } catch (e) {
    recoveryRotationHint.value = "✗ " + String(e);
  } finally {
    recoveryRotationBusy.value = false;
  }
}
function beginRecoveryConfirmation() {
  if (!recoveryRotationSaved.value) {
    recoveryRotationHint.value = "请先确认已经安全保存新的恢复短语";
    return;
  }
  recoveryRotationInputs.value = ["", "", ""];
  recoveryRotationHint.value = "";
  recoveryRotationStep.value = "confirm";
}
async function finishRecoveryRotation() {
  if (recoveryRotationBusy.value || !recoveryRotationMatches.value) return;
  recoveryRotationBusy.value = true;
  recoveryRotationHint.value = "";
  try {
    await confirmV2RecoveryRotation(
      recoveryRotationToken.value,
      recoveryRotationInputs.value.map((word) => word.trim().toLowerCase())
    );
    clearRecoveryRotationLocal();
    recoveryRotationHint.value = "✓ 新恢复短语已生效，旧恢复短语已失效";
    appStore.showClipToast("success", "恢复短语已更换");
  } catch (e) {
    recoveryRotationHint.value = "✗ " + String(e);
  } finally {
    recoveryRotationBusy.value = false;
  }
}
async function cancelRecoveryRotation() {
  if (recoveryRotationBusy.value) return;
  const token = recoveryRotationToken.value;
  if (!token) {
    clearRecoveryRotationLocal();
    return;
  }
  recoveryRotationBusy.value = true;
  try {
    await cancelV2RecoveryRotation(token);
    clearRecoveryRotationLocal();
    recoveryRotationHint.value = "";
  } catch (e) {
    recoveryRotationHint.value = "✗ " + String(e);
  } finally {
    recoveryRotationBusy.value = false;
  }
}
onUnmounted(() => {
  const token = recoveryRotationToken.value;
  clearRecoveryRotationLocal();
  if (token) void cancelV2RecoveryRotation(token).catch(() => {});
});

// ========== 方案① s1d：🔐 独立二次验证密码（新增 section） ==========
// 状态：true=已启用独立密码（用户已设置）；false=跟随主密码（默认）
const pw2ndEnabled = ref(false);
// 旧密码输入：启用时=旧二次验证密码，未启用时=主密码（校验身份用）
const pw2ndOld = ref("");
const pw2ndNew = ref("");
const pw2ndNewConfirm = ref("");
const pw2ndBusy = ref(false);
const pw2ndHint = ref("");
async function refreshPw2ndEnabled() {
  try { pw2ndEnabled.value = await apiHasSecondPassword(); }
  catch { pw2ndEnabled.value = false; }
}
onMounted(refreshPw2ndEnabled);

async function onSetPw2nd() {
  if (pw2ndBusy.value) return;
  if (!pw2ndOld.value) { pw2ndHint.value = pw2ndEnabled.value ? "请输入旧的二次验证密码" : "请先输入当前主密码（用于验证身份）"; return; }
  if (pw2ndNew.value.length < 6) { pw2ndHint.value = "新二次验证密码至少 6 位"; return; }
  if (pw2ndNew.value !== pw2ndNewConfirm.value) { pw2ndHint.value = "两次新密码不一致"; return; }
  if (!confirm(pw2ndEnabled.value
    ? "将修改独立二次验证密码（≠主密码），确定吗？"
    : "即将启用独立二次验证密码：\n· 启用后，查看/复制任何密码需再输它（主密码无效）\n· 它不能与主密码相同\n确定继续？")) return;
  pw2ndBusy.value = true; pw2ndHint.value = "";
  try {
    await changeSecondPassword(pw2ndOld.value, pw2ndNew.value);
    pw2ndHint.value = pw2ndEnabled.value ? "✓ 独立二次验证密码已更新" : "✓ 已启用独立二次验证密码（查看/复制密码时生效）";
    appStore.showClipToast("success", pw2ndEnabled.value ? "二次验证密码已更新" : "二次验证密码已启用");
    pw2ndOld.value = ""; pw2ndNew.value = ""; pw2ndNewConfirm.value = "";
    void refreshPw2ndEnabled();
  } catch (e) {
    pw2ndHint.value = "✗ " + String(e);
  } finally { pw2ndBusy.value = false; }
}
async function onClearPw2nd() {
  if (!pw2ndBusy.value && pw2ndEnabled.value) {
    if (!pw2ndOld.value) { pw2ndHint.value = "请先输入当前二次验证密码（验证身份后才能关闭）"; return; }
    if (!confirm("即将清空独立二次验证密码，改回「跟随主密码」。确定吗？")) return;
    pw2ndBusy.value = true; pw2ndHint.value = "";
    try {
      await changeSecondPassword(pw2ndOld.value, "");
      pw2ndHint.value = "✓ 已清空，二次验证恢复跟随主密码";
      appStore.showClipToast("success", "已关闭独立二次验证");
      pw2ndOld.value = ""; pw2ndNew.value = ""; pw2ndNewConfirm.value = "";
      void refreshPw2ndEnabled();
    } catch (e) {
      pw2ndHint.value = "✗ " + String(e);
    } finally { pw2ndBusy.value = false; }
  }
}

// ========== 🚨 紧急救援：密码解密失败时一键救回 ==========
const rescuePw = reactive({
  oldMaster: "",
  busy: false,
  hint: "",
  lastStats: null as null | { tried: number; rescued: number; failed: number },
});
async function onRescuePasswords() {
  if (rescuePw.busy) return;
  if (!rescuePw.oldMaster) { rescuePw.hint = "请先输入当初加密这些密码时用的主密码"; return; }
  if (!confirm(
    "⚠️ 紧急救援说明：\n" +
    "如果所有密码都显示「解密失败」（通常是改主密码/启用独立密码/换电脑导入后出现），\n" +
    "请输入「当初创建/最后一次正常显示密码时用的主密码」，系统将尝试用它解密所有密码，\n" +
    "然后用当前主密钥重新加密写回。\n\n" +
    "继续吗？"
  )) return;
  rescuePw.busy = true; rescuePw.hint = ""; rescuePw.lastStats = null;
  try {
    const r = await rescuePasswordsWithMaster(rescuePw.oldMaster);
    rescuePw.lastStats = r;
    if (r.failed === 0) {
      rescuePw.hint = `✓ 救援成功！共检查 ${r.tried} 条，救回 ${r.rescued} 条（其余本来就能解）`;
      appStore.showClipToast("success", `密码救援成功：救回 ${r.rescued} 条`);
      rescuePw.oldMaster = "";
      // 触发密码区刷新
      try { await passwordStore.reloadAll?.(); } catch {}
      try { await passwordStore.refresh?.(); } catch {}
    } else {
      rescuePw.hint = `⚠️ 部分救援失败：救回 ${r.rescued} 条，仍有 ${r.failed} 条无法解密（密码不对？）`;
      appStore.showClipToast("info", `部分失败：${r.failed} 条密码仍无法解密`);
    }
  } catch (e) {
    rescuePw.hint = "✗ " + String(e);
  } finally { rescuePw.busy = false; }
}

// ========== 窗口位置 ==========
async function resetPosition() {
  const screenW = window.screen.width;
  const w = widgetStore.miniMode ? 56 : 380;
  const h = widgetStore.miniMode ? 320 : 560;
  await setWindowPosition(screenW - w - 24, 24);
  await setWindowSize(w, h);
}
async function centerWindow() {
  const screenW = window.screen.width;
  const screenH = window.screen.height;
  const w = widgetStore.miniMode ? 56 : 380;
  const h = widgetStore.miniMode ? 320 : 560;
  await setWindowPosition(Math.round((screenW - w) / 2), Math.round((screenH - h) / 2));
  await setWindowSize(w, h);
}

// ========== 图标诊断 ==========
const iconBusy = ref(false);
const lastIconResult = ref<string>("");
async function onReextractIcons() {
  if (iconBusy.value) return;
  if (!confirm("确定要清空所有图标缓存吗？\n（清空后抽屉柜会自动重新抽图，首次启动会慢 1-2 分钟）")) return;
  iconBusy.value = true; lastIconResult.value = "正在清空缓存…";
  try {
    const n = await softwareStore.refreshAllIcons({ forceClear: true });
    lastIconResult.value = `✓ 已清空并重新抽图（${n} 个软件）`;
    appStore.showClipToast("success", `已重新抽图 ${n} 个软件`);
  } catch (e) {
    lastIconResult.value = `✗ 失败：${e}`;
  } finally { iconBusy.value = false; }
}
async function onRestartApp() {
  if (iconBusy.value) return;
  if (!confirm("确定要重启抽屉柜吗？\n（用于让 webview 重新加载新代码）")) return;
  iconBusy.value = true;
  try { await restartApp(); }
  catch (e) { lastIconResult.value = `✗ 重启失败：${e}`; iconBusy.value = false; }
}

// ========== 数据迁移 ==========
// 已移除（Phase 2A 2026-10）：migrate_data 后端 fail closed（复制运行中 SQLite
// 无一致性保证，且新路径不被启动仲裁采用=迁移无效）。数据目录迁移另行独立设计。

// ========== 应用更新（Phase 2E：官方 Tauri updater，签名校验 + 用户确认安装）==========
const versionInfo = ref<string>("");
const updateBusy = ref(false);
const updateResult = ref<string>("");
const updateReady = ref<null | { version: string; notes: string; download: () => Promise<void> }>(null);
const updateProgress = ref<string>("");
getAppVersion().then((v) => (versionInfo.value = v)).catch(() => {});

async function onCheckUpdate() {
  updateBusy.value = true; updateResult.value = "正在检查更新…"; updateReady.value = null; updateProgress.value = "";
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
  } finally { updateBusy.value = false; }
}

async function onUpdateInstall() {
  if (!updateReady.value) return;
  updateBusy.value = true;
  try {
    await updateReady.value.download();
    updateResult.value = "✓ 下载完成，即将重启以完成安装…";
    const { relaunch } = await import("@tauri-apps/plugin-process");
    setTimeout(() => { relaunch().catch(() => {}); }, 1200);
  } catch (e) {
    updateResult.value = "✗ 下载/安装失败：" + e + "。当前安装未受影响，可稍后重试。";
  } finally { updateBusy.value = false; }
}

// ========== B1：导入 / 导出加密备份 ==========
// 导出
const exportPw = ref("");
const exportBusy = ref(false);
const exportResult = ref("");
async function onExportBackup() {
  if (exportBusy.value) return;
  const pw = exportPw.value;
  if (!pw) { exportResult.value = "✗ 请先输入主密码，用于加密备份文件"; return; }
  if (pw.length < 6) { exportResult.value = "✗ 主密码至少 6 位"; return; }
  exportBusy.value = true; exportResult.value = "正在打包 + 加密…（数据多会稍慢）";
  try {
    const result: ExportResult = await exportEncryptedBackup(pw);
    let msg = `✓ 已导出备份：\n${result.path}\n\n`;
    if (result.security_model === "stable-dek-v2") {
      msg += `安全格式：v${result.backup_version} Stable DEK 完整快照\n`;
      msg += `完整业务记录：${result.business_rows} 条`;
      if (result.trash_passwords > 0) msg += `（含回收站密码 ${result.trash_passwords} 条）`;
      msg += "\n密码密文与 UUID 已原样保存，未产生明文密码。";
      exportResult.value = msg;
      exportPw.value = "";
      return;
    }
    msg += `📊 密码导出状态：\n`;
    msg += `  · 明文解密成功（换电脑可恢复）：${result.passwords_decrypted_ok} 条\n`;
    if (result.trash_passwords > 0) {
      msg += `  · 其中回收站密码：${result.trash_passwords} 条\n`;
    }
    if (result.passwords_decrypted_failed > 0) {
      msg += `\n⚠️ 【警告】有 ${result.passwords_decrypted_failed} 条密码解密失败！\n`;
      msg += `   → 这些密码保留的是旧密文格式，换电脑导入后可能无法正常解密！\n`;
      msg += `   → 建议：先使用「紧急救援」功能修复这些密码后再重新导出备份。`;
    } else {
      msg += `  · 解密失败：0 条（全部正常，换电脑放心用）`;
    }
    exportResult.value = msg;
    exportPw.value = "";
  } catch (e: any) {
    exportResult.value = `✗ 导出失败：${e ?? "未知错误"}`;
  } finally { exportBusy.value = false; }
}

// 导入
const importFile = ref("");
const importPw = ref("");
const importPolicy = ref<ConflictPolicy>("overwrite");
const importBusy = ref(false);
const importResult = ref("");
async function onPickImportFile() {
  try {
    const p = await pickPath({ mode: "file", title: "选择 .drawerbox 备份文件" });
    if (p) importFile.value = p;
  } catch (e) { importResult.value = `✗ 选择文件失败：${e}`; }
}
function fmtStats(s: ImportStats): string {
  if (s.full_restore && s.security_model === "stable-dek-v2") {
    return [
      `安全格式：v${s.backup_version} Stable DEK 完整恢复`,
      `软件：${s.apps_inserted} ｜密码：${s.passwords_inserted}`,
      `代码段：${s.snippets_inserted} ｜便签：${s.temps_inserted}`,
      `分类：${s.categories_inserted} ｜设置项：${s.settings}`,
      `数据库快照：${(s.total_bytes / 1024).toFixed(1)} KB`,
      "原主密码与 Recovery Phrase 均保持有效。",
    ].join("\n");
  }
  const lines: string[] = [];
  lines.push(`软件：新增 ${s.apps_inserted} ｜跳过 ${s.apps_skipped} ｜覆盖 ${s.apps_overwritten}`);
  lines.push(`密码：新增 ${s.passwords_inserted} ｜跳过 ${s.passwords_skipped} ｜覆盖 ${s.passwords_overwritten}`);
  lines.push(`代码段：新增 ${s.snippets_inserted} ｜跳过 ${s.snippets_skipped} ｜覆盖 ${s.snippets_overwritten}`);
  lines.push(`便签：新增 ${s.temps_inserted} ｜跳过 ${s.temps_skipped} ｜覆盖 ${s.temps_overwritten}`);
  lines.push(`分类：新增 ${s.categories_inserted} ｜重名合并 ${s.categories_conflict}`);
  lines.push(`设置项：写入 ${s.settings} ｜置顶项：写入 ${s.pinned_inserted}`);
  lines.push(`解密数据量：${(s.total_bytes / 1024).toFixed(1)} KB`);
  return lines.join("\n");
}
async function onImportBackup() {
  if (importBusy.value) return;
  if (!importFile.value.trim()) { importResult.value = "✗ 请先选择一个 .drawerbox 备份文件"; return; }
  if (!importPw.value) { importResult.value = "✗ 请输入备份时使用的主密码"; return; }
  importBusy.value = true;
  importResult.value = isV2Security.value
    ? "正在校验主密码、完整性与密码密文，然后原子恢复…（请勿关闭窗口）"
    : "正在校验密码 + 解密 + 写入…（请勿关闭窗口）";
  try {
    const stats = await importEncryptedBackup(importFile.value.trim(), importPw.value, importPolicy.value);
    // 导入成功：刷新所有 store 里的列表 → UI 立即同步到「对应区域」
    await Promise.all([
      widgetStore.reloadAll(),
      passwordStore.reloadAll(),
      snippetsStore.reloadAll(),
      tempStore.reloadAll(),
      softwareStore.reloadAll(),
    ]);
    importResult.value = "✓ 导入完成，已同步到各区域：\n" + fmtStats(stats);
    importPw.value = "";
  } catch (e: any) {
    // 常见错误：密码错误 / 非 drawerbox 文件 / 版本不兼容
    let msg = `${e ?? "未知错误"}`;
    if (msg.includes("BadTag") || msg.includes("password") || msg.includes("decrypt")) {
      msg = "解密失败：请检查主密码是否正确，或文件是否完整。";
    } else if (msg.includes("Magic") || msg.includes("header") || msg.includes("magic")) {
      msg = "文件格式无效：请选择 .drawerbox 备份文件。";
    }
    importResult.value = `✗ 导入失败：${msg}`;
  } finally { importBusy.value = false; }
}

function clearBackupSensitiveInputs() {
  exportPw.value = "";
  importPw.value = "";
}
onUnmounted(clearBackupSensitiveInputs);

// ========== B3：危险操作 - 工厂还原 ==========
// B3 防误触：① 勾选确认 → ② 必须输入「确认重置」四个字 → 按钮才可点
const resetConfirmed = ref(false);
const resetTypedText = ref("");
const resetBusy = ref(false);
const resetReady = computed(() => resetConfirmed.value && resetTypedText.value === "确认重置");
async function onResetAllData() {
  if (!resetReady.value || resetBusy.value) return;
  if (!confirm("最后确认：\n\n真的要删除所有数据并重启吗？\n\n操作一旦执行不可恢复！")) return;
  resetBusy.value = true;
  try {
    // Rust 端 factoryReset 会：① 清空内存密钥 ② 写 .factory_reset_pending 标记 ③ 立刻 restart_app(app) 强制重启
    // 下次启动时，setup() 会在 SQLite 打开之前先读到标记 → 100% 无句柄 → 物理删除 DB+WAL/SHM+icons → 进入首次设置向导
    // 【关键】这里绝对不能再弹 alert 让用户点确定！否则等待期间前端会再去拿 SQLite 锁 → Windows os error 32
    await factoryReset();

    // 兜底：如果 Rust 端 restart_app 因为某些原因没生效（极端情况），1 秒后前端强制调 restartApp 保证重启
    setTimeout(() => {
      restartApp().catch(() => {});
    }, 1000);
  } catch (e: any) {
    // 只有写标记文件失败才会走到这里（极少见，比如数据目录无写入权限），用户需要看到错误
    let msg = e?.toString?.() ?? String(e ?? "未知错误");
    if (msg.includes("删除失败") || msg.includes("os error 32")) {
      msg = "重置执行失败：文件被占用。请关闭抽屉柜后，手动删除设置页「📂 数据存储位置」对应文件夹里的 drawer_box.db、drawer_box.db-wal、drawer_box.db-shm 三个文件和 icons/ 文件夹，再重新启动 App 即可进入首次设置向导。";
    }
    alert(`✗ 重置失败：${msg}`);
    resetBusy.value = false;
  }
}

// ========== 启动：加载设置项 ==========
onMounted(async () => {
  try {
    const days = await getSetting("trash_retention_days");
    if (days) trashRetentionDays.value = parseInt(days, 10) || 30;
  } catch {}
});

// ========== IntersectionObserver：滚动自动同步 active tab（绑定到 .content-scroll 容器） ==========
onMounted(() => {
  setTimeout(() => {
    const sections = SETTING_TABS
      .map((t) => document.getElementById(`tab-${t.key}`))
      .filter(Boolean) as HTMLElement[];
    if (sections.length === 0) return;
    const root = document.querySelector<HTMLElement>(".content-scroll") || null;
    const io = new IntersectionObserver(
      (entries) => {
        const visible = entries
          .filter((e) => e.isIntersecting)
          .sort((a, b) => a.boundingClientRect.top - b.boundingClientRect.top);
        if (visible.length > 0) {
          const key = visible[0].target.id.replace(/^tab-/, "");
          if (key && key !== activeTab.value) activeTab.value = key;
        }
      },
      {
        root,
        rootMargin: root ? "-10% 0px -75% 0px" : "-20% 0px -70% 0px",
        threshold: 0,
      }
    );
    sections.forEach((s) => io.observe(s));
  }, 120);
});
</script>

<template>
  <div class="view">
    <!-- ===== 5 Tab 顶部导航（sticky） ===== -->
    <div class="settings-tabs" role="tablist">
      <button
        v-for="t in SETTING_TABS"
        :key="t.key"
        class="settings-tab tap"
        :class="{ active: activeTab === t.key }"
        role="tab"
        :aria-selected="activeTab === t.key"
        data-interactive
        @click="selectTab(t.key)"
      >
        <span class="tab-icon">{{ t.icon }}</span>
        <span class="tab-label">{{ t.label }}</span>
      </button>
    </div>

    <!-- ================================================= -->
    <!-- 🛠️ Tab 1：基础（行为偏好 + 回收站 + 快捷键说明） -->
    <!-- ================================================= -->
    <section id="tab-basic" class="settings-section-group" data-tab="basic">

      <!-- 快捷键（纯展示卡片） -->
      <div class="settings-section card-soft">
        <div class="section-head">
          <h3 class="section-title">⌨️ 快捷键速查</h3>
          <span class="section-badge">3 项</span>
        </div>
        <div class="shortcut-grid">
          <div class="shortcut-item">
            <span class="shortcut-label">呼出 / 收起窗口</span>
            <span class="shortcut-display">Alt + Q</span>
          </div>
          <div class="shortcut-item">
            <span class="shortcut-label">立即锁定</span>
            <span class="shortcut-display">Alt + L</span>
          </div>
          <div class="shortcut-item">
            <span class="shortcut-label">完整 / 迷你切换</span>
            <span class="shortcut-display">Alt + M</span>
          </div>
        </div>
      </div>

      <!-- 行为偏好（横向行：左图标+标签，右控件，视觉类似 iOS 设置） -->
      <div class="settings-section card-soft">
        <div class="section-head">
          <h3 class="section-title">⚙️ 行为偏好</h3>
        </div>
        <div class="pref-list">
          <div class="pref-row">
            <div class="pref-left">
              <span class="pref-icon">🧊</span>
              <div class="pref-text">
                <span class="pref-label">闲置自动锁定</span>
                <span class="pref-sub">无操作后自动锁定密码区</span>
              </div>
            </div>
            <select v-model.number="autoLockMinutes" class="form-select form-select-inline" data-interactive>
              <option :value="1">1 分钟</option>
              <option :value="5">5 分钟</option>
              <option :value="10">10 分钟</option>
              <option :value="15">15 分钟</option>
              <option :value="30">30 分钟</option>
              <option :value="0">永不自动锁定</option>
            </select>
          </div>
          <div class="pref-row">
            <div class="pref-left">
              <span class="pref-icon">📅</span>
              <div class="pref-text">
                <span class="pref-label">便签默认过期</span>
                <span class="pref-sub">新建便签的自动清除时间</span>
              </div>
            </div>
            <select v-model.number="tempExpireDays" class="form-select form-select-inline" data-interactive>
              <option :value="1">1 天</option>
              <option :value="3">3 天</option>
              <option :value="7">7 天</option>
              <option :value="0">永不过期</option>
            </select>
          </div>
          <div class="pref-row">
            <div class="pref-left">
              <span class="pref-icon">📋</span>
              <div class="pref-text">
                <span class="pref-label">剪贴板自动清除</span>
                <span class="pref-sub">复制密码后自动清空剪贴板</span>
              </div>
            </div>
            <select v-model.number="clipboardSeconds" class="form-select form-select-inline" data-interactive>
              <option :value="10">10 秒</option>
              <option :value="15">15 秒</option>
              <option :value="30">30 秒</option>
              <option :value="60">1 分钟</option>
            </select>
          </div>
        </div>
      </div>

      <!-- 回收站（从"数据与维护"挪到基础，更符合用户心智） -->
      <div class="settings-section card-soft">
        <div class="section-head">
          <h3 class="section-title">🗑️ 回收站</h3>
          <span class="section-badge section-badge-blue">已恢复</span>
        </div>
        <div class="pref-list">
          <div class="pref-row">
            <div class="pref-left">
              <span class="pref-icon">🕐</span>
              <div class="pref-text">
                <span class="pref-label">删除项保留天数</span>
                <span class="pref-sub">超过后将在下次启动时永久清理</span>
              </div>
            </div>
            <select v-model.number="trashRetentionDays" class="form-select form-select-inline" data-interactive>
              <option :value="1">1 天</option>
              <option :value="7">7 天</option>
              <option :value="30">30 天（默认）</option>
              <option :value="90">90 天</option>
              <option :value="365">1 年</option>
              <option :value="0">永久保留</option>
            </select>
          </div>
        </div>
        <div class="action-row">
          <button class="btn-secondary tap" @click="onOpenTrash" data-interactive>🗑️ 打开回收站</button>
          <button class="btn-secondary btn-warn tap" @click="onClearTrash" data-interactive>🧹 立即清空回收站</button>
        </div>
      </div>
    </section>

    <!-- ================================================= -->
    <!-- 🔒 Tab 2：安全与密码（所有安全相关集中） -->
    <!-- ================================================= -->
    <section id="tab-security" class="settings-section-group" data-tab="security">

      <!-- 主密码（保持原 section 结构，但用新样式） -->
      <div class="settings-section card-soft">
        <div class="section-head">
          <h3 class="section-title">🔒 主密码</h3>
          <span class="section-badge section-badge-purple">核心</span>
        </div>
        <p class="section-hint section-hint-purple">
          <template v-if="isV2Security">修改后只更新主密码保护，密码数据与 Recovery Phrase 保持不变。</template>
          <template v-else>修改后会立即用新密码重加密所有密码条目和恢复短语（保持解锁状态，无需重新输入）。</template><br>
          <template v-if="!isV2Security"><b>注意：</b> 修改主密码 <b>不会</b> 影响您的「独立二次验证密码」（两套密码独立）。</template>
        </p>
        <div class="form-grid">
          <div class="form-group">
            <label class="form-label">当前主密码</label>
            <input v-model="currentPassword" type="password" class="form-input" placeholder="请输入当前主密码" data-interactive />
          </div>
          <div class="form-group">
            <label class="form-label">新主密码</label>
            <input v-model="newPassword" type="password" class="form-input" placeholder="至少 6 位" data-interactive />
          </div>
          <div class="form-group">
            <label class="form-label">确认新主密码</label>
            <input v-model="confirmPassword" type="password" class="form-input" placeholder="再次输入新主密码" data-interactive />
          </div>
        </div>
        <div class="action-row">
          <button class="btn-primary" :disabled="pwChanging" @click="onChangePassword" data-interactive>
            {{ pwChanging ? (isV2Security ? "保存中…" : "重加密中…") : "🔐 修改主密码" }}
          </button>
        </div>
        <p v-if="pwChangeResult" class="hint-inline" :class="{ 'hint-ok': pwChangeResult.startsWith('✓'), 'hint-err': pwChangeResult.startsWith('✗') }">
          {{ pwChangeResult }}
        </p>
      </div>

      <div v-if="isV2Security" class="settings-section card-soft card-security">
        <div class="section-head">
          <h3 class="section-title">🛟 更换恢复短语</h3>
          <span class="section-badge section-badge-green">v2</span>
        </div>
        <p class="section-hint section-hint-green">
          生成并确认新的 12 个恢复词后，旧恢复短语会立即失效。确认完成前，旧恢复短语仍然有效。
        </p>

        <div v-if="recoveryRotationStep === 'idle'" class="action-row">
          <button class="btn-primary btn-green" :disabled="recoveryRotationBusy" @click="startRecoveryRotation" data-interactive>
            {{ recoveryRotationBusy ? "生成中…" : "更换恢复短语" }}
          </button>
        </div>

        <template v-else-if="recoveryRotationStep === 'words'">
          <div class="recovery-rotate-grid">
            <div v-for="(word, index) in recoveryRotationWords" :key="index" class="recovery-rotate-word">
              <span>{{ index + 1 }}</span><b>{{ word }}</b>
            </div>
          </div>
          <label class="recovery-rotate-check">
            <input v-model="recoveryRotationSaved" type="checkbox" data-interactive />
            <span>我已将新的恢复短语保存到安全位置</span>
          </label>
          <div class="action-row">
            <button class="btn-primary btn-green" :disabled="!recoveryRotationSaved || recoveryRotationBusy" @click="beginRecoveryConfirmation" data-interactive>
              验证已保存
            </button>
            <button class="btn-secondary" :disabled="recoveryRotationBusy" @click="cancelRecoveryRotation" data-interactive>取消</button>
          </div>
        </template>

        <template v-else>
          <p class="section-hint">请按编号输入刚才保存的新恢复词：</p>
          <div class="form-grid">
            <div v-for="(wordIndex, inputIndex) in recoveryRotationIndexes" :key="wordIndex" class="form-group">
              <label class="form-label">第 {{ wordIndex + 1 }} 个词</label>
              <input
                v-model="recoveryRotationInputs[inputIndex]"
                type="text"
                class="form-input"
                autocomplete="off"
                :spellcheck="false"
                placeholder="输入恢复词"
                data-interactive
              />
            </div>
          </div>
          <div class="action-row">
            <button class="btn-primary btn-green" :disabled="!recoveryRotationMatches || recoveryRotationBusy" @click="finishRecoveryRotation" data-interactive>
              {{ recoveryRotationBusy ? "确认中…" : "确认并立即更换" }}
            </button>
            <button class="btn-secondary" :disabled="recoveryRotationBusy" @click="cancelRecoveryRotation" data-interactive>取消</button>
          </div>
        </template>

        <p v-if="recoveryRotationHint" class="hint-inline" :class="{ 'hint-ok': recoveryRotationHint.startsWith('✓'), 'hint-err': recoveryRotationHint.startsWith('✗') }">
          {{ recoveryRotationHint }}
        </p>
      </div>

      <!-- 方案① s1d：🔐 密码区二次验证（NEW）——legacy-only，v2 下无此概念，隐藏 -->
      <div v-if="!isV2Security" class="settings-section card-soft card-security">
        <div class="section-head">
          <h3 class="section-title">🔐 密码区二次验证</h3>
          <span v-if="pw2ndEnabled" class="section-badge section-badge-green">已启用独立密码</span>
          <span v-else class="section-badge section-badge-gray">跟随主密码（默认）</span>
        </div>
        <p class="section-hint section-hint-green">
          <b>默认行为：</b> 每次点击密码的 👁️ 查看 / 📋 复制 按钮，需输入<b>主密码</b>二次确认。<br>
          <b>启用独立密码后（推荐）：</b> 二次验证只认这套独立密码，<b>主密码无效</b>。<br>
          即使您临时离开未锁屏，陌生人也无法查看/复制任何密码——安全再升级。
        </p>

        <div class="form-grid">
          <div class="form-group">
            <label class="form-label">
              {{ pw2ndEnabled ? "旧的二次验证密码（验证身份）" : "当前主密码（验证身份后启用独立密码）" }}
            </label>
            <input v-model="pw2ndOld" type="password" class="form-input" :placeholder="pw2ndEnabled ? '请输入旧二次验证密码' : '请先输入主密码'" data-interactive />
          </div>
          <div class="form-group">
            <label class="form-label">
              {{ pw2ndEnabled ? "新的二次验证密码" : "新二次验证密码（至少 6 位，且 ≠ 主密码）" }}
            </label>
            <input v-model="pw2ndNew" type="password" class="form-input" placeholder="至少 6 位" data-interactive />
          </div>
          <div class="form-group">
            <label class="form-label">再确认一次新二次验证密码</label>
            <input v-model="pw2ndNewConfirm" type="password" class="form-input" placeholder="请再次输入" data-interactive />
          </div>
        </div>

        <div class="action-row">
          <button class="btn-primary btn-green" :disabled="pw2ndBusy" @click="onSetPw2nd" data-interactive>
            {{ pw2ndBusy ? "处理中…" : (pw2ndEnabled ? "✎ 修改独立二次验证密码" : "🚀 启用独立二次验证密码") }}
          </button>
          <button
            v-if="pw2ndEnabled"
            class="btn-secondary btn-warn"
            :disabled="pw2ndBusy"
            @click="onClearPw2nd"
            data-interactive
          >🗑️ 清空，改回跟随主密码</button>
        </div>

        <p v-if="pw2ndHint" class="hint-inline" :class="{ 'hint-ok': pw2ndHint.startsWith('✓'), 'hint-err': pw2ndHint.startsWith('✗') }">
          {{ pw2ndHint }}
        </p>
      </div>

      <!-- 🚨 紧急救援：legacy 专用（DW2/Stable DEK 下不存在该失败场景），v2 隐藏 -->
      <div v-if="!isV2Security" class="settings-section card-soft card-danger">
        <div class="section-head">
          <h3 class="section-title">🚨 紧急救援：密码解密失败？</h3>
          <span class="section-badge section-badge-warn">救命按钮</span>
        </div>
        <p class="section-hint section-hint-warn">
          <b>什么情况用它？</b><br>
          · 改完主密码后，所有密码显示「解密失败」<br>
          · 启用/修改独立密码后，所有密码显示「解密失败」<br>
          · 换电脑导入旧备份后，密码全部解不开<br><br>
          <b>原理：</b> 输入「<u>当初加密这些密码时用的主密码</u>」，系统用它解密所有密码，<br>
          再用<b>当前主密钥</b>重新加密写回——就像给所有密码换一把适配新锁的钥匙。
        </p>

        <div class="form-grid">
          <div class="form-group">
            <label class="form-label">当初加密时用的主密码（123321 等旧密码）</label>
            <input v-model="rescuePw.oldMaster" type="password" class="form-input" placeholder="请输入旧主密码，至少 6 位" data-interactive />
          </div>
        </div>

        <div class="action-row">
          <button class="btn-primary btn-warn" :disabled="rescuePw.busy" @click="onRescuePasswords" data-interactive>
            {{ rescuePw.busy ? "救援中…" : "🚨 立刻救回我的密码" }}
          </button>
        </div>

        <p v-if="rescuePw.lastStats" class="hint-inline" :class="rescuePw.lastStats.failed===0 ? 'hint-ok' : 'hint-err'" style="margin-top:12px;">
          <b>救援结果：</b> 共检查 <b>{{ rescuePw.lastStats.tried }}</b> 条，
          成功救回 <b style="color:var(--c-green)">{{ rescuePw.lastStats.rescued }}</b> 条，
          仍失败 <b style="color:var(--c-red)">{{ rescuePw.lastStats.failed }}</b> 条
          （失败通常是输入的旧主密码不对，或密码本身已损坏）
        </p>
        <p v-if="rescuePw.hint" class="hint-inline" :class="{ 'hint-ok': rescuePw.hint.startsWith('✓'), 'hint-err': rescuePw.hint.startsWith('✗'), 'hint-warn': rescuePw.hint.startsWith('⚠️') }">
          {{ rescuePw.hint }}
        </p>
      </div>
    </section>

    <!-- ================================================= -->
    <!-- 🖼️ Tab 3：桌面插件框（外观/显示/位置，不含开发者模式） -->
    <!-- ================================================= -->
    <section id="tab-widget" class="settings-section-group" data-tab="widget">
      <div class="settings-section card-soft">
        <div class="section-head">
          <h3 class="section-title">🖥️ 显示模式</h3>
        </div>
        <p class="section-hint">抽屉柜支持完整卡片和极简图标列两种形态，可随时切换窗口层级与显示尺寸。</p>
        <div class="seg-group">
          <div class="seg-label"><span>形态</span></div>
          <div class="seg-inner">
            <button
              class="seg-btn tap" :class="{ active: !widgetStore.miniMode }"
              @click="widgetStore.miniMode && widgetStore.toggleMiniMode()" data-interactive
            >
              <span class="seg-icon">🗂️</span><span class="seg-label-inner">完整卡片模式</span>
            </button>
            <button
              class="seg-btn tap" :class="{ active: widgetStore.miniMode }"
              @click="!widgetStore.miniMode && widgetStore.toggleMiniMode()" data-interactive
            >
              <span class="seg-icon">🎛️</span><span class="seg-label-inner">极简图标列</span>
            </button>
          </div>
        </div>
        <div class="seg-group">
          <div class="seg-label"><span>窗口层级</span></div>
          <div class="seg-inner">
            <button
              class="seg-btn tap" :class="{ active: !widgetStore.alwaysOnTop }"
              @click="widgetStore.alwaysOnTop && widgetStore.toggleAlwaysOnTop()" data-interactive
            >
              <span class="seg-icon">⬇️</span><span class="seg-label-inner">不置顶（默认）</span>
            </button>
            <button
              class="seg-btn tap" :class="{ active: widgetStore.alwaysOnTop }"
              @click="!widgetStore.alwaysOnTop && widgetStore.toggleAlwaysOnTop()" data-interactive
            >
              <span class="seg-icon">⬆️</span><span class="seg-label-inner">置顶显示</span>
            </button>
          </div>
        </div>
      </div>

      <div class="settings-section card-soft">
        <div class="section-head">
          <h3 class="section-title">📍 窗口位置</h3>
        </div>
        <div class="action-row">
          <button class="btn-secondary tap" data-interactive @click="resetPosition">↺ 重置到右上角</button>
          <button class="btn-secondary tap" data-interactive @click="centerWindow">⊙ 居中</button>
        </div>
        <div class="preset-grid" title="快速切换预设位置">
          <button class="preset-btn tap" data-interactive @click="widgetStore.applyPresetPosition('top-left')" title="左上角">↖</button>
          <button class="preset-btn tap" data-interactive @click="widgetStore.applyPresetPosition('center')" title="居中">⊙</button>
          <button class="preset-btn tap" data-interactive @click="widgetStore.applyPresetPosition('top-right')" title="右上角">↗</button>
          <button class="preset-btn tap" data-interactive @click="widgetStore.applyPresetPosition('bottom-left')" title="左下角">↙</button>
          <button class="preset-btn tap" data-interactive @click="widgetStore.applyPresetPosition('remember')" title="记忆当前位置">💾</button>
          <button class="preset-btn tap" data-interactive @click="widgetStore.applyPresetPosition('bottom-right')" title="右下角">↘</button>
        </div>
        <p class="hint">点击预设格子 → 抽屉柜会动画滑动到对应位置</p>
      </div>
    </section>

    <!-- ================================================= -->
    <!-- 📦 Tab 4：数据维护（存储位置 / 迁移 / 导入导出） -->
    <!-- ================================================= -->
    <section id="tab-data" class="settings-section-group" data-tab="data">

      <!-- 数据目录：独立展示卡片（不与迁移混） -->
      <div class="settings-section card-soft">
        <div class="section-head">
          <h3 class="section-title">📂 数据存储位置</h3>
        </div>
        <div class="data-dir-row">
          <span class="data-dir-label">当前目录：</span>
          <code class="data-dir-path" :title="dataDir">{{ dataDir }}</code>
          <button class="btn-mini tap" @click="copyDataDir" data-interactive>
            {{ dataDirCopied ? "✓ 已复制" : "📋 复制路径" }}
          </button>
        </div>
        <p class="hint-left">
          这里保存抽屉柜的本地数据（数据库、回收站、图标缓存一并存在此目录）。<br />
          软件安装位置与数据存储位置相互独立：重新安装软件或改变安装位置不会自动移动这些数据。<br />
          建议定期使用加密备份，并将重要备份保存在非系统盘。
        </p>

        <!-- Phase 2C-3：更改数据存储位置（事务式迁移 C→D / D→E）。
             2C-4：移除 !migrationDone 门控——多级迁移/恢复默认（每级产生 retained archive）
             是 Data Root 生命周期的正式能力，横幅仍一次性展示。 -->
        <div class="migration-block" v-if="!migrationRunning">
          <button class="btn-mini tap" @click="migrationPick" data-interactive :disabled="migrationBusy">
            🚚 更改数据存储位置
          </button>
        </div>

        <!-- 迁移确认面板 -->
        <div class="migration-confirm" v-if="migrationTarget && !migrationRunning">
          <p><b>当前位置：</b><code>{{ dataDir }}</code></p>
          <p><b>新位置：</b><code>{{ migrationTarget }}</code></p>
          <p class="hint-left">
            迁移过程中抽屉柜会暂时停止数据操作。<br />
            新位置验证成功前不会删除原数据。<br />
            迁移完成后应用会自动重新启动。<br />
            发生异常时会保留原数据并阻止继续写入。
          </p>
          <div class="action-row">
            <button class="btn-mini tap" @click="migrationTarget = ''" data-interactive>取消</button>
            <button class="btn-primary" @click="migrationStart" data-interactive :disabled="migrationBusy">
              开始迁移
            </button>
          </div>
        </div>

        <!-- 迁移进行中（真实阶段提示，无假进度条） -->
        <div class="migration-running" v-if="migrationRunning">
          <p>⏳ 正在迁移数据：复制数据 → 验证数据一致性 → 完成后自动重启。<br />请勿关闭抽屉柜。</p>
        </div>

        <!-- 迁移成功后的保留提示（一次性展示） -->
        <div class="migration-done" v-if="migrationDone">
          <p>✅ 数据存储位置已更改：{{ migrationDone.target }}<br />
          原位置数据（{{ migrationDone.source }}）已停止使用，并保留作为安全副本（{{ migrationDone.archive }}）。</p>
        </div>

        <!-- Phase 2C-4：恢复到默认位置（仅 external 时显示） -->
        <div class="migration-block" v-if="canRestoreDefault && !restoreRunning">
          <button class="btn-mini tap" @click="restoreConfirmOpen = true" data-interactive :disabled="restoreBusy">
            ↩️ 恢复到默认位置
          </button>
        </div>
        <div class="migration-confirm" v-if="restoreConfirmOpen && !restoreRunning">
          <p><b>当前位置：</b><code>{{ dataDir }}</code></p>
          <p><b>默认位置：</b><code>{{ defaultDir }}</code></p>
          <p class="hint-left">
            抽屉柜会把当前完整数据复制到默认位置。<br />
            验证完成前不会删除当前数据。<br />
            迁移完成后应用会重新启动。
          </p>
          <div class="action-row">
            <button class="btn-mini tap" @click="restoreConfirmOpen = false" data-interactive>取消</button>
            <button class="btn-primary" @click="restoreStart" data-interactive :disabled="restoreBusy">
              恢复到默认位置
            </button>
          </div>
        </div>
        <div class="migration-running" v-if="restoreRunning">
          <p>⏳ 正在恢复到默认位置：复制数据 → 验证数据一致性 → 完成后自动重启。<br />请勿关闭抽屉柜。</p>
        </div>

        <!-- Phase 2C-4：旧数据副本（retained sources）管理 -->
        <div class="retained-block" v-if="retainedSources.length">
          <h4 class="retained-title">🗄️ 旧数据副本</h4>
          <p class="hint-left">这是迁移前保留的安全副本。当前抽屉柜不会再向它写入数据。</p>
          <p class="retained-error" v-if="deleteError">⚠️ {{ deleteError }}</p>
          <div class="retained-row" v-for="r in retainedSources" :key="r.op_id">
            <div class="retained-info">
              <div>{{ r.created_at.slice(0, 10) }}｜原位置：<code>{{ r.original_root }}</code></div>
              <div class="hint-left">状态：已停止使用{{ r.available === false ? "｜⚠️ 当前不可访问" : "" }}</div>
            </div>
            <div class="action-row" v-if="pendingDeleteOp !== r.op_id">
              <button class="btn-mini tap" @click="openRetained(r.op_id)" data-interactive>📂 打开所在文件夹</button>
              <button class="btn-mini tap" @click="removeRetained(r.op_id)" data-interactive>🗑️ 删除旧数据副本</button>
            </div>
            <ConfirmInline
              v-else
              :title="deleteStage === 1
                ? '删除后将无法再通过这个旧数据副本恢复迁移前状态。确认删除？'
                : '再次确认：这个操作不可撤销，删除旧数据副本吗？'"
              confirm-text="删除旧数据副本"
              @cancel="cancelDelete"
              @confirm="confirmDelete(r.op_id)"
            />
          </div>
        </div>
      </div>

      <!-- 迁移卡片已移除（Phase 2A 2026-10）：migrate_data fail closed，
           换盘/防系统盘重装丢数据请使用下方「导入/导出备份」（.drawerbox 加密单文件）。 -->

      <!-- B1：真实导入/导出备份（AES-256-GCM + zstd，文件格式 .drawerbox） -->
      <div class="settings-section card-soft">
        <div class="section-head">
          <h3 class="section-title">💾 导入 / 导出备份</h3>
          <span class="section-badge section-badge-green">跨设备离线</span>
        </div>
        <p class="section-hint section-hint-green">
          导出为 <code>.drawerbox</code> 加密单文件（主密码保护 + AES-256-GCM + zstd 压缩），纯离线、不需要云。v2 会完整保留安全元数据与原始密码密文，换电脑后仍可使用原主密码和 Recovery Phrase。
        </p>

        <!-- ① 导出：主密码输入 + 导出按钮 -->
        <div class="sub-block">
          <h4 class="sub-title">📤 导出备份（把当前所有数据打包加密）</h4>
          <div class="form-group">
            <label class="form-label">当前主密码（至少 6 位；恢复时必须再次输入）</label>
            <input
              v-model="exportPw"
              type="password"
              class="form-input"
              placeholder="请输入主密码…"
              autocomplete="new-password"
              data-interactive
            />
          </div>
          <div class="action-row">
            <button class="btn-primary" @click="onExportBackup" :disabled="exportBusy" data-interactive>
              {{ exportBusy ? "加密导出中…" : "🚀 导出为 .drawerbox 文件" }}
            </button>
          </div>
          <p v-if="exportResult" class="hint-inline" :class="{ 'hint-ok': exportResult.startsWith('✓'), 'hint-err': exportResult.startsWith('✗') }" style="white-space: pre-line;">
            {{ exportResult }}
          </p>
        </div>

        <div class="divider" />

        <!-- ② 导入：选文件 + 密码 + 冲突策略 + 导入按钮 -->
        <div class="sub-block">
          <h4 class="sub-title">📥 导入备份（从 .drawerbox 还原到对应区域）</h4>

          <div class="form-group">
            <label class="form-label">① 备份文件</label>
            <div style="display: flex; gap: 6px; align-items: stretch;">
              <input
                v-model="importFile"
                class="form-input"
                placeholder="例如 D:\Backups\drawerbox-20250101.drawerbox"
                style="flex: 1;"
                data-interactive
              />
              <button class="btn-mini btn-mini-wide tap" @click="onPickImportFile" data-interactive>📂 选择</button>
            </div>
          </div>

          <div class="form-group">
            <label class="form-label">② 备份时的主密码</label>
            <input
              v-model="importPw"
              type="password"
              class="form-input"
              placeholder="输入导出时使用的主密码…"
              autocomplete="new-password"
              data-interactive
            />
          </div>

          <div v-if="!isV2Security" class="form-group">
            <label class="form-label">③ 遇到重名时（按名称/路径判断）</label>
            <div class="radio-row">
              <label class="radio-item tap" data-interactive>
                <input type="radio" v-model="importPolicy" value="overwrite" />
                <span><b>覆盖</b>（推荐还原到旧机器/重置后导入：同名全部替换）</span>
              </label>
              <label class="radio-item tap" data-interactive>
                <input type="radio" v-model="importPolicy" value="skip" />
                <span><b>跳过</b>（保留当前已有数据，只加新的）</span>
              </label>
              <label class="radio-item tap" data-interactive>
                <input type="radio" v-model="importPolicy" value="merge" />
                <span><b>合并</b>（软件路径存在时合并分类与使用计数）</span>
              </label>
            </div>
          </div>

          <div class="action-row">
            <button class="btn-primary" @click="onImportBackup" :disabled="importBusy" data-interactive>
              {{ importBusy ? "校验 + 恢复中…" : (isV2Security ? "🧩 验证并完整恢复" : "🧩 开始导入并同步到各区域") }}
            </button>
          </div>
          <p v-if="importResult" class="hint-inline" :class="{ 'hint-ok': importResult.startsWith('✓'), 'hint-err': importResult.startsWith('✗') }" style="white-space: pre-line;">
            {{ importResult }}
          </p>
        </div>
      </div>

      <!-- B3：⚠️ 危险操作 - 工厂还原（数据维护 tab 的末尾，符合心智；双重校验：勾选 + 输入 4 个字） -->
      <div class="settings-section card-soft card-danger danger-card">
        <div class="section-head">
          <h3 class="section-title">⚠️ 危险操作：重置所有数据</h3>
          <span class="section-badge section-badge-red">永久不可逆</span>
        </div>
        <div class="danger-warn">
          🔥 工厂还原会<b>永久删除</b>：全部密码 / 软件 / 代码段 / 便签 / 回收站 / 图标缓存 / 全局设置 / 二次验证密码。删除后<b>无法恢复</b>，等价于抽屉柜第一次安装的状态！
        </div>
        <div class="danger-warn" style="margin-top: 6px;">
          ℹ️ 重置只影响<b>当前使用的数据位置</b>（含其中的全部数据与数据位置设置）。之前迁移保留的<b>旧数据副本不会被自动删除</b>，重置后如需清理可手动处理。
        </div>
        <label class="danger-check tap" data-interactive>
          <input type="checkbox" v-model="resetConfirmed" />
          <span>第 1 步：我已确认 → 此操作会<b>永久删除所有数据且不可恢复</b>，我已做好充分备份（如已导出 .drawerbox）</span>
        </label>
        <div class="form-group" style="margin-top: 10px;">
          <label class="form-label">第 2 步：请在下方输入 <code style="color: #ef4444; font-weight: 700;">确认重置</code> 四个字（防误触）</label>
          <input
            v-model="resetTypedText"
            class="form-input"
            :class="{ 'input-danger-ok': resetReady }"
            placeholder="请输入：确认重置"
            maxlength="4"
            data-interactive
          />
        </div>
        <div class="action-row">
          <button
            class="btn-danger"
            :disabled="!resetReady || resetBusy"
            :class="{ 'btn-danger-ready': resetReady }"
            @click="onResetAllData"
            data-interactive
          >
            {{ resetBusy ? "重置中…（3秒后自动重启）" : (resetReady ? "🔴 确认：重置所有数据（不可恢复）" : "☑️ 请先勾选确认项并输入 4 个字") }}
          </button>
        </div>
      </div>
    </section>

    <!-- ================================================= -->
    <!-- 🔬 Tab 5：高级（图标诊断 / 更新 / 危险操作） -->
    <!-- ================================================= -->
    <section id="tab-advanced" class="settings-section-group" data-tab="advanced">

      <!-- 图标诊断 -->
      <div class="settings-section card-soft">
        <div class="section-head">
          <h3 class="section-title">🖼️ 软件图标诊断</h3>
        </div>
        <p class="section-hint">
          如果软件区显示的图标与 Windows 资源管理器里看到的不一致（例如之前抽图失败、缓存脏），可按顺序：
          <b>① 清空缓存重抽</b> → <b>② 重启应用</b>，重启后抽屉柜会自动为所有软件重新抽取最新图标。
        </p>
        <div class="action-row">
          <button class="btn-primary" @click="onReextractIcons" :disabled="iconBusy" data-interactive>
            {{ iconBusy ? "处理中…" : "🗑️ 清空缓存 + 重新抽图" }}
          </button>
          <button class="btn-secondary" @click="onRestartApp" :disabled="iconBusy" data-interactive>🔄 重启应用</button>
        </div>
        <p v-if="lastIconResult" class="hint-inline" :class="{ 'hint-ok': lastIconResult.startsWith('✓'), 'hint-err': lastIconResult.startsWith('✗') }">
          {{ lastIconResult }}
        </p>
      </div>

      <!-- 应用更新 -->
      <div class="settings-section card-soft">
        <div class="section-head">
          <h3 class="section-title">🔄 应用更新</h3>
          <span class="section-badge section-badge-purple">v{{ versionInfo || "…" }}</span>
        </div>
        <p class="section-hint section-hint-purple">
          当前版本：<code>{{ versionInfo || "加载中" }}</code>。自动更新服务将在后续版本接入 GitHub Releases API。
        </p>
        <div class="action-row">
          <button class="btn-primary" @click="onCheckUpdate" :disabled="updateBusy" data-interactive>
            {{ updateBusy && !updateReady ? "检查中…" : "🔍 检查更新" }}
          </button>
          <button v-if="updateReady && !updateBusy" class="btn-primary" @click="onUpdateInstall" data-interactive>
            ⬇️ 下载并安装 v{{ updateReady.version }}
          </button>
        </div>
        <p v-if="updateProgress" class="hint-inline">{{ updateProgress }}</p>
        <p v-if="updateResult" class="hint-inline" :class="{ 'hint-ok': !updateResult.startsWith('✗'), 'hint-err': updateResult.startsWith('✗') }">
          {{ updateResult }}
        </p>
      </div>

    </section>
  </div>
</template>

<style scoped>
/* Phase 2C-4：旧数据副本 */
.retained-block {
  margin-top: 12px;
  padding-top: 10px;
  border-top: 1px dashed var(--border-color, rgba(255, 255, 255, 0.12));
}
.retained-title {
  margin: 4px 0 6px;
  font-size: 13px;
  font-weight: 600;
}
.retained-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  padding: 8px 10px;
  margin-bottom: 6px;
  border-radius: 8px;
  background: rgba(255, 255, 255, 0.04);
}
.retained-info {
  min-width: 0;
  font-size: 12px;
  overflow: hidden;
}
.retained-info code {
  font-size: 11px;
  word-break: break-all;
}
.retained-error {
  margin: 6px 0;
  font-size: 12px;
  color: #f87171;
}
.view { padding-bottom: 20px; }

/* ========== 顶部 sticky tabs ========== */
.settings-tabs {
  display: flex;
  gap: 4px;
  padding: 6px;
  margin-bottom: 14px;
  background: var(--bg-primary);
  border: 1px solid var(--border);
  border-radius: var(--radius);
  position: sticky;
  top: 0;
  z-index: 5;
  backdrop-filter: blur(10px);
  -webkit-backdrop-filter: blur(10px);
  box-shadow: 0 2px 8px rgba(0, 0, 0, 0.18);
}
.settings-tab {
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 5px;
  padding: 8px 6px;
  border: none;
  border-radius: 9px;
  background: transparent;
  color: var(--text-muted);
  font-size: 12px;
  font-weight: 500;
  cursor: pointer;
  transition: all 0.18s var(--ease-smooth);
  -webkit-app-region: no-drag;
  white-space: nowrap;
  min-width: 0;
}
.settings-tab:hover { background: var(--bg-tertiary); color: var(--text-primary); }
.settings-tab.active {
  background: linear-gradient(135deg, var(--accent) 0%, var(--highlight) 100%);
  color: white;
  font-weight: 600;
  box-shadow: 0 2px 8px rgba(99, 102, 241, 0.35);
}
.tab-icon { font-size: 14px; line-height: 1; }
.tab-label { letter-spacing: 0.2px; overflow: hidden; text-overflow: ellipsis; }
@media (max-width: 659px) {
  .tab-label { display: none; }
  .settings-tab { padding: 10px 6px; }
}

/* section 层级 */
.settings-section-group { display: flex; flex-direction: column; gap: 0; }
.settings-section-group + .settings-section-group { margin-top: 4px; }

/* ========== 卡片统一：柔和背景 + 更轻边框 + section-head 小工具 ========== */
.settings-section {
  background: var(--bg-primary);
  border-radius: var(--radius);
  padding: 18px 18px 16px;
  margin-bottom: 14px;
  border: 1px solid var(--border);
  transition: border-color 0.2s;
}
.settings-section:hover { border-color: rgba(120, 130, 255, 0.35); }
.card-soft { box-shadow: 0 1px 3px rgba(0, 0, 0, 0.04); }
.card-security { border-left: 3px solid #10b981; }
.card-disabled-card { opacity: 0.88; }
.card-danger { border-color: rgba(244, 63, 94, 0.5) !important; }

.section-head {
  display: flex; align-items: center; justify-content: space-between;
  margin-bottom: 10px; gap: 10px;
}
.section-title {
  font-size: 14px; font-weight: 600; margin: 0;
  color: var(--text-primary);
  display: flex; align-items: center; gap: 6px;
}
.section-badge {
  font-size: 10.5px; font-weight: 600;
  padding: 3px 8px; border-radius: 999px;
  white-space: nowrap; flex-shrink: 0;
  border: 1px solid transparent;
  letter-spacing: 0.2px;
}
.section-badge-gray { background: var(--bg-tertiary); color: var(--text-muted); border-color: var(--border); }
.section-badge-blue { background: rgba(59,130,246,0.12); color: #60a5fa; border-color: rgba(59,130,246,0.35); }
.section-badge-purple { background: rgba(168,85,247,0.12); color: #c084fc; border-color: rgba(168,85,247,0.35); }
.section-badge-green { background: rgba(16,185,129,0.12); color: #34d399; border-color: rgba(16,185,129,0.4); }
.section-badge-red { background: rgba(244,63,94,0.12); color: #fb7185; border-color: rgba(244,63,94,0.45); }
.section-badge-soon { background: rgba(120,120,120,0.12) !important; }

.section-hint {
  font-size: 11.5px; color: var(--text-muted);
  margin: 0 0 14px; padding: 9px 12px;
  background: var(--bg-secondary); border-radius: 9px;
  border-left: 3px solid var(--accent);
  line-height: 1.65;
}
.section-hint-purple { border-left-color: #a855f7; background: rgba(168, 85, 247, 0.07); }
.section-hint-blue { border-left-color: #3b82f6; background: rgba(59, 130, 246, 0.06); }
.section-hint-green { border-left-color: #10b981; background: rgba(16, 185, 129, 0.07); }

.form-sub {
  display: block;
  font-size: 10.5px;
  color: var(--text-faint);
  font-weight: normal;
  margin-top: 4px;
  line-height: 1.5;
}

/* ========== 表单 ========== */
.form-group { margin-bottom: 12px; }
.form-group:last-child { margin-bottom: 0; }
.form-grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(200px, 1fr));
  gap: 10px 14px;
  margin-bottom: 10px;
}
.form-label {
  font-size: 11.5px; color: var(--text-muted);
  margin-bottom: 5px; display: block; font-weight: 500;
}
.form-input,
.form-select {
  width: 100%;
  background: var(--bg-secondary);
  border: 1px solid var(--border);
  border-radius: 8px;
  padding: 9px 11px;
  color: var(--text-primary);
  font-size: 12.5px;
  outline: none;
  transition: border-color 0.2s, box-shadow 0.2s;
  -webkit-app-region: no-drag;
  box-sizing: border-box;
}
.form-input:focus,
.form-select:focus {
  border-color: var(--accent);
  box-shadow: 0 0 0 3px rgba(99, 102, 241, 0.15);
}
.form-select { cursor: pointer; }
.form-select-inline {
  width: auto; min-width: 128px;
  font-size: 11.5px; padding: 7px 10px;
  flex-shrink: 0;
}

/* ========== 行为偏好 / 开关：横向行（iOS 风格） ========== */
.pref-list {
  display: flex; flex-direction: column;
  border: 1px solid var(--border); border-radius: 10px;
  background: var(--bg-secondary); overflow: hidden;
  margin-bottom: 4px;
}
.pref-row {
  display: flex; align-items: center; justify-content: space-between;
  padding: 10px 12px; gap: 10px;
  border-bottom: 1px solid var(--border);
  min-height: 44px;
  box-sizing: border-box;
}
.pref-row:last-child { border-bottom: none; }
.pref-left { display: flex; align-items: center; gap: 10px; min-width: 0; flex: 1; }
.pref-icon {
  width: 28px; height: 28px; border-radius: 7px;
  display: inline-flex; align-items: center; justify-content: center;
  background: linear-gradient(135deg, rgba(99,102,241,0.18), rgba(59,130,246,0.1));
  font-size: 14px; flex-shrink: 0;
}
.pref-text { display: flex; flex-direction: column; min-width: 0; }
.pref-label { font-size: 12.5px; font-weight: 600; color: var(--text-primary); line-height: 1.3; }
.pref-sub { font-size: 10.5px; color: var(--text-faint); line-height: 1.4; margin-top: 2px; }

/* ========== 分段控件（Seg-group：桌面插件框用） ========== */
.seg-group {
  display: flex; align-items: center; justify-content: space-between;
  gap: 10px;
  padding: 10px 0;
  border-bottom: 1px dashed var(--border);
}
.seg-group:last-of-type { border-bottom: none; }
.seg-group > .seg-label {
  font-size: 12px; color: var(--text-muted); font-weight: 500;
  min-width: 68px; flex-shrink: 0;
  display: inline-flex; align-items: center;
}
.seg-inner {
  display: flex; gap: 4px; flex: 1;
  background: var(--bg-secondary);
  border: 1px solid var(--border);
  border-radius: 9px;
  padding: 4px;
}
.seg-btn {
  flex: 1;
  background: transparent;
  border: none;
  border-radius: 7px;
  padding: 8px 6px;
  color: var(--text-muted);
  cursor: pointer;
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 5px;
  font-size: 11px;
  transition: all 0.2s var(--ease-smooth);
}
.seg-btn:hover { color: var(--text-secondary); background: var(--bg-tertiary); }
.seg-btn.active {
  background: linear-gradient(135deg, var(--accent) 0%, var(--highlight) 100%);
  color: white;
  box-shadow: 0 2px 6px rgba(79, 124, 255, 0.32);
  font-weight: 600;
}
.seg-icon { font-size: 13px; }
.seg-label-inner { white-space: nowrap; }

/* ========== 开关（iOS 风） ========== */
.switch {
  position: relative;
  width: 44px; height: 26px;
  background: var(--bg-tertiary);
  border: 1px solid var(--border);
  border-radius: 14px;
  cursor: pointer;
  transition: all 0.25s var(--ease-spring);
  padding: 0;
  flex-shrink: 0;
}
.switch.on {
  background: linear-gradient(135deg, var(--accent) 0%, var(--highlight) 100%);
  border-color: transparent;
}
.switch-knob {
  position: absolute;
  top: 2px; left: 2px;
  width: 20px; height: 20px;
  background: white;
  border-radius: 50%;
  transition: all 0.25s var(--ease-spring);
  box-shadow: 0 2px 4px rgba(0,0,0,0.2);
}
.switch.on .switch-knob { left: 22px; }

/* ========== 动作区（独立一行，底部，视觉不与表单混） ========== */
.action-row {
  display: flex; gap: 10px;
  padding-top: 12px;
  margin-top: 4px;
  border-top: 1px solid var(--border);
}
.action-row .btn-primary { flex: 2; }
.action-row .btn-secondary,
.action-row .btn-danger { flex: 1; }
.action-row .btn-warn { flex: 1.2; }

/* ========== 按钮 ========== */
.btn-primary {
  padding: 10px 18px; border-radius: 9px;
  font-size: 12.5px; font-weight: 600; cursor: pointer;
  background: linear-gradient(135deg, var(--accent) 0%, var(--highlight) 100%);
  color: white; border: none;
  transition: all 0.18s;
  box-shadow: 0 4px 12px rgba(79, 124, 255, 0.3);
}
.btn-primary:hover:not(:disabled) {
  transform: translateY(-1px);
  box-shadow: 0 6px 16px rgba(79, 124, 255, 0.42);
}
.btn-primary:disabled { opacity: 0.55; cursor: not-allowed; box-shadow: none; }

.btn-green {
  background: linear-gradient(135deg, #10b981 0%, #059669 100%) !important;
  box-shadow: 0 4px 12px rgba(16, 185, 129, 0.3) !important;
}
.btn-green:hover:not(:disabled) { box-shadow: 0 6px 16px rgba(16, 185, 129, 0.4) !important; }

.btn-secondary {
  padding: 10px 18px; border-radius: 9px;
  font-size: 12.5px; cursor: pointer;
  background: var(--bg-secondary);
  color: var(--text-secondary);
  border: 1px solid var(--border);
  transition: all 0.18s;
}
.btn-secondary:hover:not(:disabled) { background: var(--bg-tertiary); color: var(--text-primary); border-color: rgba(99,102,241,0.35); }
.btn-secondary:disabled { opacity: 0.55; cursor: not-allowed; }

.btn-warn {
  color: #f97316 !important;
  border-color: rgba(249, 115, 22, 0.35) !important;
  background: rgba(249, 115, 22, 0.06) !important;
  font-weight: 500;
}
.btn-warn:hover:not(:disabled) {
  color: #fb923c !important;
  background: rgba(249, 115, 22, 0.1) !important;
  border-color: rgba(249, 115, 22, 0.55) !important;
}

/* 占位：即将上线的按钮（灰化 + 角标） */
.btn-soon {
  position: relative;
  opacity: 0.62 !important;
  cursor: not-allowed !important;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 6px;
}
.soon-chip {
  font-size: 10px;
  padding: 1.5px 6px;
  border-radius: 999px;
  background: rgba(120,120,120,0.22);
  color: var(--text-faint);
  border: 1px solid var(--border);
  font-weight: 600;
  letter-spacing: 0.2px;
}

.btn-mini {
  padding: 6px 10px; border-radius: 7px;
  font-size: 11px; cursor: pointer;
  background: var(--bg-secondary);
  color: var(--text-secondary);
  border: 1px solid var(--border);
  flex-shrink: 0;
  transition: all 0.15s;
  white-space: nowrap;
}
.btn-mini:hover { background: var(--bg-tertiary); color: var(--text-primary); border-color: rgba(99,102,241,0.35); }
.btn-mini-wide { padding: 0 14px; min-height: 36px; align-self: stretch; }

/* ========== 危险操作：红色大卡 ========== */
.danger-card {
  background: linear-gradient(135deg, rgba(244,63,94,0.06), rgba(251,146,60,0.04)) !important;
}
.danger-warn {
  font-size: 12px;
  line-height: 1.65;
  padding: 10px 12px;
  border-radius: 9px;
  border: 1px dashed rgba(244, 63, 94, 0.55);
  background: rgba(244, 63, 94, 0.08);
  color: #fecdd3;
  margin-bottom: 12px;
}
.danger-check {
  display: inline-flex; align-items: flex-start; gap: 9px;
  padding: 9px 12px;
  border: 1px solid rgba(244, 63, 94, 0.3);
  border-radius: 8px;
  background: rgba(244, 63, 94, 0.05);
  font-size: 11.5px;
  color: #fda4af;
  line-height: 1.55;
  margin-bottom: 10px;
  cursor: pointer;
}
.danger-check input[type="checkbox"] {
  margin-top: 2px; width: 15px; height: 15px; accent-color: #f43f5e; flex-shrink: 0;
}
.btn-danger {
  padding: 11px 18px;
  border-radius: 9px;
  font-size: 12.5px;
  font-weight: 600;
  cursor: pointer;
  background: #4b5563;
  color: rgba(255,255,255,0.85);
  border: 1px solid rgba(255,255,255,0.08);
  width: 100%;
  transition: all 0.2s;
  letter-spacing: 0.2px;
}
.btn-danger-ready {
  background: linear-gradient(135deg, #ef4444 0%, #dc2626 100%) !important;
  color: white !important;
  border-color: transparent !important;
  box-shadow: 0 4px 14px rgba(239, 68, 68, 0.4);
  animation: dangerPulse 1.4s ease-in-out infinite;
}
.btn-danger-ready:hover {
  transform: translateY(-1px);
  box-shadow: 0 6px 18px rgba(239, 68, 68, 0.55);
}
@keyframes dangerPulse {
  0%, 100% { box-shadow: 0 4px 14px rgba(239, 68, 68, 0.38); }
  50%      { box-shadow: 0 4px 20px rgba(239, 68, 68, 0.62); }
}

/* ========== 快捷键卡片（3 列网格，纯展示） ========== */
.shortcut-grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(200px, 1fr));
  gap: 8px 14px;
}
.shortcut-item {
  display: flex; justify-content: space-between; align-items: center;
  padding: 9px 11px;
  background: var(--bg-secondary);
  border: 1px solid var(--border);
  border-radius: 8px;
  transition: border-color 0.2s;
}
.shortcut-item:hover { border-color: rgba(99,102,241,0.35); }
.shortcut-label { font-size: 12px; color: var(--text-secondary); }
.shortcut-display {
  background: var(--bg-primary);
  border: 1px solid var(--border);
  border-radius: 6px;
  padding: 3px 10px;
  font-size: 11.5px;
  font-family: 'Cascadia Code', 'Consolas', monospace;
  color: var(--text-primary);
  box-shadow: inset 0 -1px 0 rgba(0,0,0,0.06);
  letter-spacing: 0.3px;
}

/* ========== 6 宫格预设位置 ========== */
.preset-grid {
  display: grid;
  grid-template-columns: repeat(3, 1fr);
  gap: 8px;
  margin-top: 12px;
}
.preset-btn {
  padding: 14px 0;
  background: var(--bg-secondary);
  border: 1px solid var(--border);
  border-radius: 9px;
  color: var(--text-secondary);
  font-size: 18px;
  cursor: pointer;
  transition: all 0.18s var(--ease-smooth);
}
.preset-btn:hover {
  background: var(--accent-soft);
  border-color: var(--accent);
  color: var(--accent-bright);
  transform: translateY(-1px);
}
.preset-btn:active { transform: scale(0.94); }

/* ========== 数据目录行 ========== */
.data-dir-row {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 10px 12px;
  background: var(--bg-secondary);
  border: 1px solid var(--border);
  border-radius: 9px;
  margin-bottom: 8px;
}
.data-dir-label { font-size: 11.5px; color: var(--text-muted); flex-shrink: 0; font-weight: 500; }
.data-dir-path {
  flex: 1;
  font-size: 11px;
  color: var(--text-primary);
  background: transparent;
  padding: 2px 6px;
  border-radius: 5px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-family: 'Consolas', 'Monaco', monospace;
  min-width: 0;
}

/* ========== 提示文案（含 OK / ERR 着色） ========== */
.hint,
.hint-left {
  margin: 8px 0 0;
  font-size: 10.5px;
  color: var(--text-faint);
  line-height: 1.55;
}
.hint { text-align: center; }
.hint-left { text-align: left; }
.hint code {
  background: var(--bg-tertiary);
  padding: 1px 5px;
  border-radius: 4px;
  color: var(--accent-bright);
  font-size: 10px;
  margin: 0 1px;
}
.hint-inline {
  margin: 10px 0 0;
  font-size: 11px;
  padding: 8px 12px;
  border-radius: 8px;
  line-height: 1.55;
  border: 1px solid transparent;
}
.hint-ok {
  color: #6ee7b7;
  background: rgba(16, 185, 129, 0.08);
  border-color: rgba(16, 185, 129, 0.35);
}
.hint-err {
  color: #fca5a5;
  background: rgba(239, 68, 68, 0.08);
  border-color: rgba(239, 68, 68, 0.4);
}

/* ========== B1 导入/导出 新增样式 ========== */
.sub-block {
  margin-top: 2px;
}
.sub-title {
  font-size: 13px;
  font-weight: 600;
  color: var(--text-primary);
  margin: 4px 0 10px;
}
.divider {
  height: 1px;
  margin: 18px 0 16px;
  background: var(--border);
  opacity: 0.6;
}
.radio-row {
  display: flex;
  flex-direction: column;
  gap: 8px;
  padding: 6px 2px 2px;
}
.radio-item {
  display: flex;
  align-items: flex-start;
  gap: 8px;
  padding: 8px 10px;
  border-radius: 8px;
  border: 1px solid var(--border);
  background: var(--bg-secondary);
  font-size: 12px;
  line-height: 1.5;
  cursor: pointer;
  transition: all 0.15s ease;
}
.radio-item:hover {
  border-color: rgba(59, 130, 246, 0.45);
  background: rgba(59, 130, 246, 0.08);
}
.radio-item input {
  margin-top: 3px;
  accent-color: #3b82f6;
}
/* 重置：输入 4 字成功后绿色边框 */
.input-danger-ok {
  border-color: rgba(16, 185, 129, 0.55) !important;
  box-shadow: 0 0 0 3px rgba(16, 185, 129, 0.12) !important;
  background: rgba(16, 185, 129, 0.05) !important;
}

.recovery-rotate-grid {
  display: grid;
  grid-template-columns: repeat(3, minmax(0, 1fr));
  gap: 7px;
  margin: 12px 0;
}
.recovery-rotate-word {
  display: flex;
  align-items: center;
  gap: 6px;
  min-width: 0;
  padding: 8px;
  border: 1px solid var(--border);
  border-radius: 8px;
  background: var(--bg-secondary);
  font-size: 11px;
}
.recovery-rotate-word span { color: var(--text-faint); }
.recovery-rotate-word b {
  overflow: hidden;
  color: var(--text-primary);
  text-overflow: ellipsis;
}
.recovery-rotate-check {
  display: flex;
  align-items: center;
  gap: 8px;
  margin: 10px 0;
  color: var(--text-muted);
  font-size: 11px;
}
</style>

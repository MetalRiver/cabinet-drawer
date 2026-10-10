<script setup lang="ts">
// ===== Settings UI V2 · 数据与备份 Tab =====
// 层级：A 当前数据位置 → 回收站 → B 加密备份·导出（折叠） → C 备份恢复（折叠）
//       → D 迁移与历史副本（按实际状态显示） → E 重置所有数据（折叠，页面最末）。
// 全部业务逻辑从旧 SettingsView 逐字迁移（迁移/恢复/副本/备份/重置语义零改动）。
// 长操作通过 ops 总线上报：切换到其他分类时容器顶部显示"后台操作进行中"横幅；
// 离开本分类时清理敏感输入（备份密码），但执行中状态保留（Tab 常驻挂载）。
import { ref, computed, watch } from "vue";
import { useRouter } from "vue-router";
import {
  getDataDir,
  getSecurityStatus,
  setupBeginMigration,
  getDataRootSummary,
  restartApp,
  pickPath,
  hardPurgeTrash,
  getSetting,
  setSetting,
  setupBeginRestoreDefault,
  deleteRetainedSource,
  openRetainedSourceFolder,
  exportEncryptedBackup,
  importEncryptedBackup,
  factoryReset,
  type ConflictPolicy,
  type ImportStats,
  type ExportResult,
  type RetainedSourceInfo,
} from "../../../api";
import ConfirmInline from "../../../components/ConfirmInline.vue";
import SettingsCard from "../../../components/settings/SettingsCard.vue";
import SettingsRow from "../../../components/settings/SettingsRow.vue";
import StatusLine from "../../../components/settings/StatusLine.vue";
import DarkSelect from "../../../components/DarkSelect.vue";
import { useAppStore } from "../../../stores/app";
import { useWidgetStore } from "../../../stores/widget";
import { usePasswordStore } from "../../../stores/passwords";
import { useSnippetsStore } from "../../../stores/snippets";
import { useTempStore } from "../../../stores/temp";
import { useSoftwareStore } from "../../../stores/software";
import { useSettingsOps } from "../ops";

const props = defineProps<{ active: boolean }>();
const router = useRouter();
const appStore = useAppStore();
const widgetStore = useWidgetStore();
const passwordStore = usePasswordStore();
const snippetsStore = useSnippetsStore();
const tempStore = useTempStore();
const softwareStore = useSoftwareStore();
const ops = useSettingsOps();

// ===== A. 当前数据位置 =====
const dataDir = ref<string>("加载中...");
const dataDirCopied = ref(false);
const dataDirExpanded = ref(false);
async function loadDataDir() {
  try {
    dataDir.value = await getDataDir();
  } catch (e) {
    dataDir.value = `(加载失败: ${e})`;
  }
}
loadDataDir();
async function copyDataDir() {
  try {
    await navigator.clipboard.writeText(dataDir.value);
    dataDirCopied.value = true;
    setTimeout(() => (dataDirCopied.value = false), 1500);
  } catch (e) {
    console.error("复制失败", e);
  }
}

// 更改数据存储位置（事务式迁移）
const migrationTarget = ref<string>("");
const migrationBusy = ref(false);
const migrationRunning = ref(false);
const migrationDone = ref<{ source: string; target: string; archive: string } | null>(null);
const migrationError = ref("");
async function loadMigrationInfo() {
  try {
    const s = await getDataRootSummary();
    if (s.last_migration) migrationDone.value = s.last_migration;
  } catch {
    /* 摘要不可用时静默 */
  }
}
loadMigrationInfo();
async function migrationPick() {
  const p = await pickPath({ mode: "folder", title: "选择新的数据存储位置（需为空目录）" });
  if (p) {
    if (p.trim().toLowerCase() === dataDir.value.trim().toLowerCase()) {
      migrationTarget.value = "";
      migrationError.value = "新位置与当前数据位置相同。";
      return;
    }
    migrationError.value = "";
    migrationTarget.value = p;
  }
}
async function migrationStart() {
  if (migrationBusy.value || !migrationTarget.value) return;
  migrationBusy.value = true;
  migrationRunning.value = true;
  ops.begin("migration", "数据迁移到新位置", "data");
  try {
    // 成功后后端会自行重启应用（迁移收尾在新进程 setup 内完成）
    await setupBeginMigration(migrationTarget.value);
  } catch (e) {
    migrationRunning.value = false;
    migrationBusy.value = false;
    migrationError.value = String(e);
  } finally {
    ops.end("migration");
  }
}

// 恢复默认位置
const restoreConfirmOpen = ref(false);
const restoreBusy = ref(false);
const restoreRunning = ref(false);
const defaultDir = ref<string>("");
const retainedSources = ref<RetainedSourceInfo[]>([]);
const restoreError = ref("");
const canRestoreDefault = computed(() => {
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
  } catch {
    /* 摘要不可用时静默 */
  }
}
loadRestoreInfo();
async function restoreStart() {
  if (restoreBusy.value) return;
  restoreBusy.value = true;
  restoreRunning.value = true;
  ops.begin("restore", "恢复到默认数据位置", "data");
  try {
    await setupBeginRestoreDefault();
  } catch (e) {
    restoreRunning.value = false;
    restoreBusy.value = false;
    restoreError.value = String(e);
  } finally {
    ops.end("restore");
  }
}

// 旧数据副本管理（不可逆，两段内联确认）
const pendingDeleteOp = ref<string | null>(null);
const deleteStage = ref<1 | 2>(1);
const deleteError = ref("");
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
  if (deleteStage.value === 1) {
    deleteStage.value = 2;
    return;
  }
  try {
    await deleteRetainedSource(opId);
  } catch (e) {
    deleteError.value = String(e);
  }
  pendingDeleteOp.value = null;
  deleteStage.value = 1;
  await loadRestoreInfo();
}
async function openRetained(opId: string) {
  try {
    deleteError.value = "";
    await openRetainedSourceFolder(opId);
  } catch (e) {
    deleteError.value = String(e);
  }
}

// ===== 回收站（自「基础」迁入：保留期限 / 查看 / 恢复 / 清空 统一模块）=====
const trashRetentionDays = ref<number>(30);
watch(trashRetentionDays, async (v) => {
  try {
    await setSetting("trash_retention_days", String(v));
  } catch (e) {
    appStore.showClipToast("info", `⚠ 回收站保留天数保存失败：${e}`);
  }
});
// 0（永久保留）是合法值，必须原样显示；仅缺失/非法/负数回退默认 30
getSetting("trash_retention_days")
  .then((days) => {
    if (days !== null && days !== undefined && days.trim() !== "") {
      const n = Number(days.trim());
      trashRetentionDays.value = Number.isFinite(n) && n >= 0 ? Math.round(n) : 30;
    }
  })
  .catch(() => {});
const trashRetentionOptions = [
  { value: 1, label: "1 天" },
  { value: 7, label: "7 天" },
  { value: 30, label: "30 天（默认）" },
  { value: 90, label: "90 天" },
  { value: 365, label: "1 年" },
  { value: 0, label: "永久保留" },
];
function onOpenTrash() {
  router.push("/");
  window.dispatchEvent(new CustomEvent("open-trash"));
}
const clearTrashConfirmOpen = ref(false);
const trashActionMsg = ref("");
const trashActionBusy = ref(false);
function onClearTrash() {
  trashActionMsg.value = "";
  clearTrashConfirmOpen.value = true;
}
async function doClearTrash() {
  if (trashActionBusy.value) return;
  clearTrashConfirmOpen.value = false;
  trashActionBusy.value = true;
  try {
    const n = await hardPurgeTrash();
    trashActionMsg.value = `✓ 已清空 ${n} 项`;
  } catch (e) {
    trashActionMsg.value = "✗ 清空失败：" + e;
  } finally {
    trashActionBusy.value = false;
  }
}
// ===== B. 加密备份·导出（折叠）=====
const exportOpen = ref(false);
const exportPw = ref("");
const exportBusy = ref(false);
const exportResult = ref("");
async function onExportBackup() {
  if (exportBusy.value) return;
  const pw = exportPw.value;
  if (!pw) { exportResult.value = "✗ 请先输入主密码，用于加密备份文件"; return; }
  if (pw.length < 6) { exportResult.value = "✗ 主密码至少 6 位"; return; }
  exportBusy.value = true;
  exportResult.value = "正在打包 + 加密…（数据多会稍慢）";
  ops.begin("export", "导出加密备份", "data");
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
      msg += `   → 这些密码保留旧密文格式，换电脑导入后可能无法正常解密！\n`;
      msg += `   → 建议先在「安全与密码 → 紧急救援」修复后再重新导出。`;
    } else {
      msg += `  · 解密失败：0 条（全部正常，换电脑放心用）`;
    }
    exportResult.value = msg;
    exportPw.value = "";
  } catch (e: unknown) {
    exportResult.value = `✗ 导出失败：${e ?? "未知错误"}`;
  } finally {
    exportBusy.value = false;
    ops.end("export");
  }
}

// ===== C. 备份恢复（折叠，独立于导出）=====
const importOpen = ref(false);
const importFile = ref("");
const importPw = ref("");
const importPolicy = ref<ConflictPolicy>("overwrite");
const importBusy = ref(false);
const importResult = ref("");
async function onPickImportFile() {
  try {
    const p = await pickPath({ mode: "file", title: "选择 .drawerbox 备份文件" });
    if (p) importFile.value = p;
  } catch (e) {
    importResult.value = `✗ 选择文件失败：${e}`;
  }
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
  importResult.value = securityModelCache.value === "stable_dek_v2"
    ? "正在校验主密码、完整性与密码密文，然后原子恢复…（请勿关闭窗口）"
    : "正在校验密码 + 解密 + 写入…（请勿关闭窗口）";
  ops.begin("import", "恢复加密备份", "data");
  try {
    const stats = await importEncryptedBackup(importFile.value.trim(), importPw.value, importPolicy.value);
    await Promise.all([
      widgetStore.reloadAll(),
      passwordStore.reloadAll(),
      snippetsStore.reloadAll(),
      tempStore.reloadAll(),
      softwareStore.reloadAll(),
    ]);
    importResult.value = "✓ 导入完成，已同步到各区域：\n" + fmtStats(stats);
    importPw.value = "";
  } catch (e: unknown) {
    let msg = `${e ?? "未知错误"}`;
    if (msg.includes("BadTag") || msg.includes("password") || msg.includes("decrypt")) {
      msg = "解密失败：请检查主密码是否正确，或文件是否完整。";
    } else if (msg.includes("Magic") || msg.includes("header") || msg.includes("magic")) {
      msg = "文件格式无效：请选择 .drawerbox 备份文件。";
    }
    importResult.value = `✗ 导入失败：${msg}`;
  } finally {
    importBusy.value = false;
    ops.end("import");
  }
}
// 与安全页同一判定源：getSecurityStatus 的 security_model
const securityModelCache = ref<string | null>(null);
getSecurityStatus()
  .then((s) => (securityModelCache.value = s.security_model))
  .catch(() => {});

// ===== E. 重置所有数据（折叠，页面最末，三重门原样）=====
const resetOpen = ref(false);
const resetConfirmed = ref(false);
const resetTypedText = ref("");
const resetBusy = ref(false);
const resetFinalConfirmOpen = ref(false);
const resetError = ref("");
const resetReady = computed(() => resetConfirmed.value && resetTypedText.value === "确认重置");
async function onResetAllData() {
  if (!resetReady.value || resetBusy.value) return;
  if (!resetFinalConfirmOpen.value) {
    resetFinalConfirmOpen.value = true;
    return;
  }
  resetFinalConfirmOpen.value = false;
  resetBusy.value = true;
  resetError.value = "";
  ops.begin("reset", "重置所有数据", "data");
  try {
    // Rust 端 factoryReset：清内存密钥 → 写 .factory_reset_pending → restart；
    // 此处绝不能再弹阻塞对话框（等待期拿 SQLite 锁会 os error 32）
    await factoryReset();
    setTimeout(() => {
      restartApp().catch(() => {});
    }, 1000);
  } catch (e: unknown) {
    let msg = (e as { toString?: () => string })?.toString?.() ?? String(e ?? "未知错误");
    if (msg.includes("删除失败") || msg.includes("os error 32")) {
      msg =
        "重置执行失败：文件被占用。请关闭抽屉柜后，手动删除数据目录里的 drawer_box.db、drawer_box.db-wal、drawer_box.db-shm 三个文件和 icons/ 文件夹，再重新启动应用即可进入首次设置向导。";
    }
    resetError.value = "✗ 重置失败：" + msg;
    resetBusy.value = false;
  } finally {
    ops.end("reset");
  }
}

// ===== 生命周期：离开本分类清理敏感输入（执行中状态保留，Tab 常驻挂载）=====
watch(
  () => props.active,
  (active) => {
    if (active) return;
    exportPw.value = "";
    importPw.value = "";
    clearTrashConfirmOpen.value = false;
    resetFinalConfirmOpen.value = false;
    restoreConfirmOpen.value = false;
  }
);
</script>

<template>
  <div class="tab-pane">
    <!-- A. 当前数据位置 -->
    <SettingsCard
      title="当前数据位置"
      subtitle="所有数据的家。重装软件不会移动它；建议定期加密备份到非系统盘。"
      :badge="{ text: '位置', tone: 'blue' }"
    >
      <div class="s-data-dir-row">
        <span class="s-data-dir-label">路径</span>
        <code class="s-data-dir-path" :class="{ expanded: dataDirExpanded }" :title="dataDir">{{ dataDir }}</code>
        <div class="dir-actions">
          <button class="s-btn-mini tap" data-interactive @click="dataDirExpanded = !dataDirExpanded">
            {{ dataDirExpanded ? "收起" : "展开" }}
          </button>
          <button class="s-btn-mini tap" data-interactive @click="copyDataDir">
            {{ dataDirCopied ? "✓ 已复制" : "📋 复制" }}
          </button>
        </div>
      </div>
      <StatusLine v-if="migrationError" tone="err" :text="migrationError" />
      <StatusLine v-if="restoreError && !restoreRunning" tone="err" :text="restoreError" />

      <!-- 更改位置（迁移确认面板，按需展开） -->
      <div v-if="!migrationRunning" class="mig-trigger">
        <button class="s-btn-mini tap" data-interactive :disabled="migrationBusy" @click="migrationPick">
          🚚 更改数据存储位置
        </button>
      </div>
      <div v-if="migrationTarget && !migrationRunning" class="mig-panel">
        <p><b>当前位置：</b><code>{{ dataDir }}</code></p>
        <p><b>新位置：</b><code>{{ migrationTarget }}</code></p>
        <p class="s-hint">
          迁移期间暂停数据操作；验证成功前不会删除原数据；完成后应用自动重启；异常时保留原数据并阻止继续写入。
        </p>
        <div class="s-action-row">
          <button class="s-btn-secondary" data-interactive @click="migrationTarget = ''">取消</button>
          <button class="s-btn-primary" data-interactive :disabled="migrationBusy" @click="migrationStart">开始迁移</button>
        </div>
      </div>
      <div v-if="migrationRunning" class="mig-running">
        ⏳ 正在迁移数据：复制 → 验证一致性 → 完成后自动重启。请勿关闭抽屉柜。
      </div>
      <div v-if="migrationDone" class="mig-done">
        ✅ 数据已迁移到 {{ migrationDone.target }}；原位置（{{ migrationDone.source }}）已保留为安全副本（{{ migrationDone.archive }}）。
      </div>
    </SettingsCard>

    <!-- 回收站（自基础迁入，统一模块） -->
    <SettingsCard
      title="回收站"
      subtitle="删除的软件/密码/命令/便签先进回收站，超过保留期限后下次启动时自动清理。"
      :badge="{ text: '可恢复', tone: 'blue' }"
    >
      <SettingsRow icon="🕐" label="删除项保留期限" sub="0 = 永久保留，绝不自动清理">
        <DarkSelect v-model="trashRetentionDays" :options="trashRetentionOptions" aria-label="删除项保留天数" />
      </SettingsRow>
      <div class="s-action-row">
        <button class="s-btn-secondary" data-interactive @click="onOpenTrash">🗑️ 打开回收站（查看 / 恢复 / 彻底删除）</button>
        <button class="s-btn-secondary s-btn-warn" data-interactive @click="onClearTrash">🧹 立即清空</button>
      </div>
      <ConfirmInline
        v-if="clearTrashConfirmOpen"
        title="确定要立即清空回收站吗？所有已删除项将被永久删除，不可恢复。"
        confirm-text="立即清空"
        :loading="trashActionBusy"
        @cancel="clearTrashConfirmOpen = false"
        @confirm="doClearTrash"
      />
      <StatusLine :tone="trashActionMsg.startsWith('✓') ? 'ok' : trashActionMsg ? 'err' : 'info'" :text="trashActionMsg" />
    </SettingsCard>

    <!-- B. 加密备份·导出（折叠） -->
    <SettingsCard
      title="导出加密备份"
      subtitle="打包为 .drawerbox 单文件（主密码 + AES-256-GCM + zstd），纯离线。"
      :badge="{ text: '跨设备', tone: 'green' }"
      collapsible
      v-model:open="exportOpen"
    >
      <div class="s-form-group">
        <label class="s-form-label">当前主密码（至少 6 位；恢复时必须再次输入）</label>
        <input v-model="exportPw" type="password" class="s-form-input" placeholder="请输入主密码…" autocomplete="new-password" data-interactive />
      </div>
      <div class="s-action-row">
        <button class="s-btn-primary" data-interactive :disabled="exportBusy" @click="onExportBackup">
          {{ exportBusy ? "加密导出中…" : "🚀 导出为 .drawerbox 文件" }}
        </button>
      </div>
      <StatusLine :tone="exportResult.startsWith('✓') ? 'ok' : exportResult ? 'err' : 'info'" :text="exportResult" />
    </SettingsCard>

    <!-- C. 备份恢复（折叠，独立于导出） -->
    <SettingsCard
      title="从备份恢复"
      subtitle="选择 .drawerbox 文件 → 输入备份时主密码 → 确认恢复，完成后自动同步到各区域。"
      :badge="{ text: '还原', tone: 'green' }"
      collapsible
      v-model:open="importOpen"
    >
      <div class="s-form-group">
        <label class="s-form-label">① 备份文件</label>
        <div class="file-row">
          <input v-model="importFile" class="s-form-input" placeholder="例如 D:\Backups\drawerbox-20250101.drawerbox" data-interactive />
          <button class="s-btn-mini s-btn-mini-wide tap" data-interactive @click="onPickImportFile">📂 选择</button>
        </div>
      </div>
      <div class="s-form-group">
        <label class="s-form-label">② 备份时使用的主密码</label>
        <input v-model="importPw" type="password" class="s-form-input" placeholder="输入导出时使用的主密码…" autocomplete="new-password" data-interactive />
      </div>
      <div class="s-form-group">
        <label class="s-form-label">③ 遇到重名时（按名称/路径判断）</label>
        <div class="s-radio-row">
          <label class="s-radio-item tap" data-interactive>
            <input type="radio" :value="'overwrite'" v-model="importPolicy" />
            <span><b>覆盖</b>：同名全部替换（适合还原到旧机器/重置后导入）</span>
          </label>
          <label class="s-radio-item tap" data-interactive>
            <input type="radio" :value="'skip'" v-model="importPolicy" />
            <span><b>跳过</b>：保留当前已有数据，只加新的</span>
          </label>
          <label class="s-radio-item tap" data-interactive>
            <input type="radio" :value="'merge'" v-model="importPolicy" />
            <span><b>合并</b>：软件路径存在时合并分类与使用计数</span>
          </label>
        </div>
      </div>
      <div class="s-action-row">
        <button class="s-btn-primary" data-interactive :disabled="importBusy" @click="onImportBackup">
          {{ importBusy ? "校验 + 恢复中…" : "🧩 开始恢复并同步到各区域" }}
        </button>
      </div>
      <StatusLine :tone="importResult.startsWith('✓') ? 'ok' : importResult ? 'err' : 'info'" :text="importResult" />
    </SettingsCard>

    <!-- D. 迁移与历史副本（按实际状态显示） -->
    <SettingsCard
      v-if="canRestoreDefault || retainedSources.length || restoreRunning"
      title="数据位置迁移与历史副本"
      subtitle="恢复默认位置；迁移前留下的旧数据副本可打开或删除。"
    >
      <div v-if="canRestoreDefault && !restoreRunning" class="mig-trigger">
        <button class="s-btn-mini tap" data-interactive :disabled="restoreBusy" @click="restoreConfirmOpen = true">
          ↩️ 恢复到默认位置
        </button>
      </div>
      <div v-if="restoreConfirmOpen && !restoreRunning" class="mig-panel">
        <p><b>当前位置：</b><code>{{ dataDir }}</code></p>
        <p><b>默认位置：</b><code>{{ defaultDir }}</code></p>
        <p class="s-hint">先把完整数据复制到默认位置并验证；验证完成前不会删除当前数据；完成后应用重启。</p>
        <div class="s-action-row">
          <button class="s-btn-secondary" data-interactive @click="restoreConfirmOpen = false">取消</button>
          <button class="s-btn-primary" data-interactive :disabled="restoreBusy" @click="restoreStart">恢复到默认位置</button>
        </div>
      </div>
      <div v-if="restoreRunning" class="mig-running">
        ⏳ 正在恢复到默认位置：复制 → 验证一致性 → 完成后自动重启。请勿关闭抽屉柜。
      </div>

      <div v-if="retainedSources.length" class="s-retained-block">
        <h4 class="s-retained-title">🗄️ 旧数据副本（{{ retainedSources.length }}）</h4>
        <p class="s-hint">迁移前保留的安全副本，当前抽屉柜不再向其写入数据。</p>
        <StatusLine v-if="deleteError" tone="err" :text="deleteError" />
        <div v-for="r in retainedSources" :key="r.op_id" class="s-retained-row">
          <div class="s-retained-info">
            <div>{{ r.created_at.slice(0, 10) }}｜原位置：<code>{{ r.original_root }}</code></div>
            <div class="s-hint">状态：已停止使用{{ r.available === false ? "｜⚠️ 当前不可访问" : "" }}</div>
          </div>
          <div class="s-action-row retained-actions" v-if="pendingDeleteOp !== r.op_id">
            <button class="s-btn-mini tap" data-interactive @click="openRetained(r.op_id)">📂 打开</button>
            <button class="s-btn-mini tap" data-interactive @click="removeRetained(r.op_id)">🗑️ 删除副本</button>
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
    </SettingsCard>

    <!-- E. 重置所有数据（折叠，页面最末，与常用操作保持距离） -->
    <SettingsCard
      title="重置所有数据"
      subtitle="永久删除全部数据并回到首次安装状态；展开前请确认已有备份。"
      :badge="{ text: '永久不可逆', tone: 'red' }"
      intent="danger"
      collapsible
      v-model:open="resetOpen"
    >
      <div class="s-danger-warn">
        🔥 重置会<b>永久删除</b>：全部密码 / 软件 / 命令 / 便签 / 回收站 / 图标缓存 / 全局设置 / 二次验证密码，等价于第一次安装。<br />
        ℹ️ 只影响当前使用的数据位置；迁移保留的旧数据副本不会被自动删除。
      </div>
      <template v-if="resetOpen">
        <label class="s-danger-check tap" data-interactive>
          <input type="checkbox" v-model="resetConfirmed" />
          <span>第 1 步：我已确认会<b>永久删除所有数据且不可恢复</b>，已做好备份（如已导出 .drawerbox）</span>
        </label>
        <div class="s-form-group">
          <label class="s-form-label">第 2 步：输入 <code style="color:#ef4444;font-weight:700;">确认重置</code> 四个字（防误触）</label>
          <input
            v-model="resetTypedText"
            class="s-form-input"
            :class="{ 's-input-danger-ok': resetReady }"
            placeholder="请输入：确认重置"
            maxlength="4"
            data-interactive
          />
        </div>
        <div class="s-action-row">
          <button
            class="s-btn-danger"
            :disabled="!resetReady || resetBusy"
            :class="{ 's-btn-danger-ready': resetReady }"
            @click="onResetAllData"
            data-interactive
          >
            {{ resetBusy ? "重置中…（即将自动重启）" : resetReady ? "🔴 确认：重置所有数据（不可恢复）" : "☑️ 请先勾选并输入 4 个字" }}
          </button>
        </div>
        <ConfirmInline
          v-if="resetFinalConfirmOpen"
          title="最后确认：真的要删除所有数据并重启吗？操作一旦执行不可恢复！"
          confirm-text="确认重置所有数据"
          @cancel="resetFinalConfirmOpen = false"
          @confirm="onResetAllData"
        />
        <StatusLine v-if="resetError" tone="err" :text="resetError" />
      </template>
      <p v-else class="s-hint">展开后需要完成三重确认（勾选 + 输入四字 + 最终确认）才会执行。</p>
    </SettingsCard>
  </div>
</template>

<style scoped>
.dir-actions { display: flex; gap: 6px; flex-shrink: 0; }
.mig-trigger { margin-top: 10px; }
.mig-panel {
  margin-top: 10px;
  padding: 12px;
  border: 1px solid var(--border);
  border-radius: 9px;
  background: var(--bg-secondary);
}
.mig-panel p { margin: 4px 0; font-size: 12px; }
.mig-panel code { word-break: break-all; font-size: 11.5px; }
.mig-running, .mig-done {
  margin-top: 10px;
  padding: 10px 12px;
  border-radius: 9px;
  font-size: 12px;
  line-height: 1.6;
}
.mig-running { background: rgba(59, 130, 246, 0.08); border-left: 3px solid #3b82f6; color: #93c5fd; }
.mig-done { background: rgba(16, 185, 129, 0.08); border-left: 3px solid #10b981; color: #6ee7b7; word-break: break-all; }
.file-row { display: flex; gap: 6px; align-items: stretch; }
.file-row .s-form-input { flex: 1; }
.s-btn-mini-wide { padding: 0 14px; min-height: 36px; align-self: stretch; }
.retained-actions { border-top: none; padding-top: 0; margin-top: 0; }
</style>

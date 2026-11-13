<script setup lang="ts">
/**
 * Phase 2C-2：首次初始化数据位置选择（Step 0）
 *
 * 三种模式（由后端 get_setup_mode 决定）：
 * - choose：空环境，请选择数据保存位置（绝不自动建库）
 * - resume_init / orphan_init_recover：未完成的初始化 → 继续或放弃
 *
 * 只提供真实可用的动作，无占位按钮。
 * 文案面向普通用户，不暴露 Config Root / Canonical Root / pointer / guard 等术语。
 *
 * Batch 1.5（v0.4.3）：信息架构调整为「一主两折叠」——
 * 「使用推荐位置」为唯一主按钮；自定义位置与恢复数据（已有目录 / 加密备份）
 * 折叠为两个次级入口；错误信息就近显示在触发它的输入区下方；
 * 安全初始化协议（guard/state/迁移/阻断）零改动。
 */
import { ref, computed } from "vue";
import {
  pickPath,
  setupAbandonPending,
  setupAttachExisting,
  setupBeginCustomInit,
  setupCheckCustomRoot,
  setupChooseDefault,
  setupResumeCustomInit,
  setupRestoreBackupBegin,
} from "../api";
import { getAppEdition, type SetupModeInfo } from "../api";

const isTestEdition = ref(false);
void getAppEdition().then((e) => { isTestEdition.value = e === "acceptance"; }).catch(() => {});
const props = defineProps<{ mode: SetupModeInfo }>();

const busy = ref(false);
// Batch 1.5：错误就近原则——每个入口的报错显示在自己的输入区下方
const errRecommended = ref("");
const errCustom = ref("");
const errAttach = ref("");
const errBackup = ref("");
const error = ref(""); // 恢复页（继续/放弃）专用
const notice = ref("");
const customPath = ref("");
const attachPath = ref("");
const showCustom = ref(false);
const showRecover = ref(false);
// R3.1：推荐路径一键复制 + 已复制反馈
const copied = ref(false);
async function copyPath(p: string | undefined) {
  if (!p) return;
  try {
    await navigator.clipboard.writeText(p);
    copied.value = true;
    setTimeout(() => { copied.value = false; }, 1800);
  } catch {
    copied.value = false;
  }
}

const isChoose = computed(() => props.mode.kind === "choose");
const isRecover = computed(() => !isChoose.value);

function reload() {
  // 选择完成后回到正常启动流程（is_first_run → 主密码设置向导）
  window.location.reload();
}

async function onChooseRecommended() {
  if (busy.value) return;
  busy.value = true;
  errRecommended.value = "";
  try {
    await setupChooseDefault();
    reload();
  } catch (e) {
    errRecommended.value = String(e);
  } finally {
    busy.value = false;
  }
}

async function onPickCustom() {
  const p = await pickPath({ mode: "folder", title: "选择抽屉柜数据保存位置" });
  if (p) customPath.value = p;
}

async function onPickAttach() {
  const p = await pickPath({ mode: "folder", title: "选择已有抽屉柜数据目录" });
  if (p) attachPath.value = p;
}

async function onBeginCustom() {
  if (busy.value) return;
  const target = customPath.value.trim();
  if (!target) {
    errCustom.value = "请先选择数据保存位置";
    return;
  }
  busy.value = true;
  errCustom.value = "";
  notice.value = "";
  try {
    await setupCheckCustomRoot(target);
    notice.value = "位置可用，正在创建…";
    await setupBeginCustomInit(target);
    reload();
  } catch (e) {
    error.value = "";
    errCustom.value = String(e);
    notice.value = "";
  } finally {
    busy.value = false;
  }
}

async function onAttachExisting() {
  if (busy.value) return;
  const target = attachPath.value.trim();
  if (!target) {
    errAttach.value = "请先选择已有数据目录";
    return;
  }
  busy.value = true;
  errAttach.value = "";
  notice.value = "";
  try {
    notice.value = "正在验证数据…";
    await setupAttachExisting(target); // 成功后后端重启进程
  } catch (e) {
    notice.value = "";
    errAttach.value = String(e);
    busy.value = false;
  }
}

async function onResume() {
  if (busy.value) return;
  busy.value = true;
  error.value = "";
  try {
    await setupResumeCustomInit();
    reload();
  } catch (e) {
    error.value = String(e);
    busy.value = false;
  }
}

// ===== Phase 2C-5：从加密备份恢复（第四入口）=====
const backupFilePath = ref("");
const backupPassword = ref("");
const restoreTargetMode = ref<"default" | "custom">("default");
const restoreCustomPath = ref("");

async function onPickBackupFile() {
  const p = await pickPath({
    mode: "file",
    title: "选择 .drawerbox 加密备份文件",
  });
  if (p) backupFilePath.value = p;
}

async function onPickRestoreCustom() {
  const p = await pickPath({ mode: "folder", title: "选择恢复数据保存位置" });
  if (p) restoreCustomPath.value = p;
}

async function onBeginBackupRestore() {
  if (busy.value) return;
  if (!backupFilePath.value.trim()) {
    errBackup.value = "请先选择 .drawerbox 备份文件";
    return;
  }
  if (!backupPassword.value) {
    errBackup.value = "请输入备份时使用的主密码";
    return;
  }
  const target = restoreTargetMode.value === "custom" ? restoreCustomPath.value.trim() : null;
  if (restoreTargetMode.value === "custom" && !target) {
    errBackup.value = "请先选择自定义数据保存位置";
    return;
  }
  busy.value = true;
  errBackup.value = "";
  notice.value = "正在验证并恢复备份…";
  try {
    await setupRestoreBackupBegin(backupFilePath.value.trim(), backupPassword.value, target);
    notice.value = "恢复完成，正在启动抽屉柜…";
    // 必须整进程重启：default 目标要重新走 setup（管理 AppState → 锁屏），
    // external 目标要在 Rust 启动路径中提交 active_root。webview reload 不够。
    setTimeout(async () => {
      const { restartApp } = await import("../api");
      await restartApp();
    }, 800);
  } catch (e) {
    error.value = "";
    errBackup.value = String(e);
    notice.value = "";
    busy.value = false;
  }
}

async function onAbandon() {
  if (busy.value) return;
  busy.value = true;
  error.value = "";
  try {
    await setupAbandonPending(); // 成功后后端重启进程
  } catch (e) {
    error.value = String(e);
    busy.value = false;
  }
}
</script>

<template>
  <div class="dr-root">
    <div class="dr-topbar" data-tauri-drag-region></div>

    <!-- ===== 选择数据保存位置 ===== -->
    <div v-if="isChoose" class="dr-body">
      <div v-if="isTestEdition" class="edition-badge edition-badge-float" title="验收便携测试版：数据将保存在本文件夹内，删除文件夹即卸载，与正式版完全隔离">测试版</div>

      <header class="dr-header">
        <h1 class="dr-title">选择数据保存位置</h1>
        <p class="dr-sub">数据会一直保存在你选择的地方。不确定就用推荐位置，之后也可以随时更改。</p>
      </header>

      <p v-if="notice" class="dr-notice">{{ notice }}</p>

      <!-- A. 推荐保存位置：最高视觉权重 -->
      <section class="dr-card dr-card-rec" :class="{ disabled: busy }">
        <div class="dr-card-rec-head">
          <span class="dr-card-rec-icon">💾</span>
          <div class="dr-card-rec-titles">
            <div class="dr-card-rec-title">推荐保存位置
              <span class="dr-badge-rec">推荐</span>
            </div>
            <div class="dr-card-rec-desc">大多数用户直接用这个即可，一键完成初始化。</div>
          </div>
        </div>
        <div class="dr-path-box">
          <span class="dr-path-label">将保存到</span>
          <code class="dr-path-value">{{ mode.config_root }}</code>
          <button class="dr-copy" :disabled="busy" @click="copyPath(mode.config_root)" title="复制路径">
            {{ copied ? "✓ 已复制" : "复制路径" }}
          </button>
        </div>
        <button class="dr-btn-primary" :disabled="busy" @click="onChooseRecommended">
          使用推荐位置
        </button>
        <p v-if="errRecommended" class="dr-error">{{ errRecommended }}</p>
      </section>

      <!-- B. 自定义保存位置 -->
      <section class="dr-card" :class="{ disabled: busy }">
        <div class="dr-card-head-row">
          <span class="dr-card-icon">📂</span>
          <div class="dr-card-titles">
            <div class="dr-card-title">自己选择保存位置</div>
            <div class="dr-card-desc">想把数据放在 D 盘或其他磁盘/文件夹，用这个。</div>
          </div>
          <button class="dr-toggle" :disabled="busy" @click="showCustom = !showCustom">
            {{ showCustom ? "收起" : "展开" }}
          </button>
        </div>
        <div v-if="showCustom" class="dr-card-body">
          <div class="dr-row">
            <input v-model="customPath" class="dr-input" placeholder="例如 D:\抽屉柜数据" :disabled="busy" />
            <button class="dr-mini" :disabled="busy" @click="onPickCustom">浏览…</button>
          </div>
          <p v-if="customPath" class="dr-path-inline">已选择：<code>{{ customPath }}</code></p>
          <button class="dr-btn" :disabled="busy || !customPath.trim()" @click="onBeginCustom">
            {{ busy ? "处理中…" : "使用该位置" }}
          </button>
          <p v-if="errCustom" class="dr-error">{{ errCustom }}</p>
        </div>
      </section>

      <!-- C. 数据恢复 -->
      <section class="dr-card" :class="{ disabled: busy }">
        <div class="dr-card-head-row">
          <span class="dr-card-icon">🔄</span>
          <div class="dr-card-titles">
            <div class="dr-card-title">已有数据？从备份恢复</div>
            <div class="dr-card-desc">重装系统、更换电脑，或手上有加密备份文件时用这个。需要原来的数据目录或备份文件 + 备份主密码。</div>
          </div>
          <button class="dr-toggle" :disabled="busy" @click="showRecover = !showRecover">
            {{ showRecover ? "收起" : "展开" }}
          </button>
        </div>
        <div v-if="showRecover" class="dr-card-body">
          <!-- 已有数据目录 -->
          <div class="dr-block">
            <div class="dr-block-head">🔄 连接原有数据目录</div>
            <p class="dr-sec-desc">重装 Windows 或抽屉柜后，指向原来的数据文件夹即可继续使用，不会覆盖其中数据。</p>
            <div class="dr-row">
              <input v-model="attachPath" class="dr-input" placeholder="选择原数据所在目录" :disabled="busy" />
              <button class="dr-mini" :disabled="busy" @click="onPickAttach">浏览…</button>
            </div>
            <button class="dr-btn" :disabled="busy || !attachPath.trim()" @click="onAttachExisting">
              {{ busy ? "处理中…" : "连接该目录" }}
            </button>
            <p v-if="errAttach" class="dr-error">{{ errAttach }}</p>
          </div>
          <!-- 从加密备份恢复 -->
          <div class="dr-block">
            <div class="dr-block-head">🗄️ 从加密备份文件恢复</div>
            <p class="dr-sec-desc">用之前导出的 .drawerbox 备份文件恢复，需要备份时设置的主密码。</p>
            <div class="dr-row">
              <input v-model="backupFilePath" class="dr-input" placeholder="选择 .drawerbox 备份文件" :disabled="busy" readonly />
              <button class="dr-mini" :disabled="busy" @click="onPickBackupFile">选择文件</button>
            </div>
            <input
              v-model="backupPassword"
              type="password"
              class="dr-input"
              placeholder="备份时使用的主密码"
              :disabled="busy"
              style="margin-top: 0; margin-bottom: 10px"
            />
            <div class="dr-restore-loc">
              <label class="dr-radio">
                <input type="radio" value="default" v-model="restoreTargetMode" :disabled="busy" />
                恢复到推荐位置（本机用户目录）
              </label>
              <label class="dr-radio">
                <input type="radio" value="custom" v-model="restoreTargetMode" :disabled="busy" />
                恢复到自定义位置
              </label>
              <div v-if="restoreTargetMode === 'custom'" class="dr-row">
                <input v-model="restoreCustomPath" class="dr-input" placeholder="例如 D:\抽屉柜数据" :disabled="busy" />
                <button class="dr-mini" :disabled="busy" @click="onPickRestoreCustom">浏览…</button>
              </div>
            </div>
            <button class="dr-btn primary" :disabled="busy || !backupFilePath.trim() || !backupPassword" @click="onBeginBackupRestore">
              {{ busy ? "恢复中…" : "从备份恢复" }}
            </button>
            <p class="dr-hint">恢复会使用备份自身的主密码与恢复短语；数据将恢复到你选择的上述位置，与备份原来的位置无关。恢复前会再次确认，不会未经确认覆盖数据。</p>
            <p v-if="errBackup" class="dr-error">{{ errBackup }}</p>
          </div>
        </div>
      </section>
    </div>

    <!-- ===== 未完成的初始化：继续 / 放弃 ===== -->
    <div v-else class="dr-body">
      <h1 class="dr-title">发现未完成的初始化</h1>
      <p class="dr-sub">
        上次初始化数据位置的过程没有完成。你可以继续初始化，或放弃并清除本次未完成的内容（不会影响其他数据）。
      </p>
      <p v-if="mode.target" class="dr-target">数据位置：<code>{{ mode.target }}</code></p>
      <div class="dr-actions">
        <button class="dr-btn primary" :disabled="busy" @click="onResume">
          {{ busy ? "处理中…" : "继续初始化" }}
        </button>
        <button class="dr-btn danger" :disabled="busy" @click="onAbandon">放弃并清理</button>
      </div>
      <p class="dr-hint">继续初始化会重新生成一组新的恢复词，请准备记录。</p>
      <p v-if="error" class="dr-error">{{ error }}</p>
    </div>
  </div>
</template>

<style scoped>
.dr-root {
  height: 100%;
  width: 100%;
  overflow-y: auto;
  display: flex;
  flex-direction: column;
  background: linear-gradient(160deg, #0b1020 0%, #131a2e 100%);
  color: #e8ecf3;
  box-sizing: border-box;
}
.dr-topbar { height: 28px; width: 100%; flex-shrink: 0; }
/* R3.1：稳定内容容器——顶部对齐 + 居中限宽，消除高窗下的大片无意义留白 */
.dr-body {
  width: 100%;
  max-width: 560px;
  margin: 0 auto;
  padding: 20px 22px 32px;
  box-sizing: border-box;
}
.dr-header { margin-bottom: 18px; }
.dr-title { margin: 0 0 8px; font-size: 20px; font-weight: 700; color: #f2f5fb; }
.dr-sub { margin: 0; font-size: 13px; line-height: 1.7; color: #9aa5bd; }
.dr-notice { margin: 0 0 14px; font-size: 13px; color: #6fd0a0; }

/* ===== 通用卡片 ===== */
.dr-card {
  border: 1px solid #232b42;
  background: rgba(19, 26, 42, 0.82);
  border-radius: 14px;
  padding: 16px;
  margin-bottom: 14px;
  transition: border-color 0.15s var(--ease-smooth, ease), background-color 0.15s var(--ease-smooth, ease);
}
.dr-card:hover { border-color: #34406a; background: rgba(24, 32, 52, 0.86); }
.dr-card.disabled { opacity: 0.6; }
.dr-card-head-row { display: flex; align-items: flex-start; gap: 12px; }
.dr-card-icon { font-size: 20px; line-height: 1.2; flex-shrink: 0; }
.dr-card-titles { flex: 1; min-width: 0; }
.dr-card-title { font-size: 15px; font-weight: 650; color: #eef2f9; margin-bottom: 4px; }
.dr-card-desc { margin: 0; font-size: 12.5px; color: #9aa5bd; line-height: 1.65; }
.dr-toggle {
  flex-shrink: 0;
  border: 1px solid #3a4568;
  background: #1c2438;
  color: #c6cede;
  border-radius: 7px;
  padding: 5px 11px;
  font-size: 12px;
  cursor: pointer;
}
.dr-toggle:hover:not(:disabled) { border-color: #4a5a8a; color: #eef2f9; }
.dr-toggle:disabled { opacity: 0.6; cursor: default; }
.dr-card-body { margin-top: 14px; }

/* ===== A. 推荐位置：最高视觉权重（提亮边框，不用纯色大色块）===== */
.dr-card-rec {
  border: 1px solid #3b6ef5;
  background: linear-gradient(180deg, rgba(59, 110, 245, 0.12) 0%, rgba(22, 28, 44, 0.9) 100%);
  padding: 16px;
}
.dr-card-rec:hover { border-color: #5b8bff; }
.dr-card-rec-head { display: flex; align-items: flex-start; gap: 12px; margin-bottom: 14px; }
.dr-card-rec-icon { font-size: 24px; line-height: 1.1; flex-shrink: 0; }
.dr-card-rec-titles { flex: 1; min-width: 0; }
.dr-card-rec-title {
  font-size: 16px; font-weight: 700; color: #ffffff;
  display: flex; align-items: center; gap: 8px; margin-bottom: 5px;
}
.dr-badge-rec {
  font-size: 11px; font-weight: 600; color: #9dbaff;
  background: rgba(59, 110, 245, 0.22);
  border: 1px solid rgba(91, 139, 255, 0.5);
  border-radius: 6px; padding: 1px 7px;
}
.dr-card-rec-desc { font-size: 12.5px; color: #aab6cf; line-height: 1.65; }
/* 路径展示：放大 + 提对比 + 复制按钮 */
.dr-path-box {
  display: flex; align-items: center; gap: 10px;
  background: #0d1220;
  border: 1px solid #2c3550;
  border-radius: 9px;
  padding: 10px 12px;
  margin-bottom: 14px;
}
.dr-path-label { font-size: 12px; color: #8b97b3; flex-shrink: 0; }
.dr-path-value {
  flex: 1; min-width: 0;
  font-size: 13px; color: #dce4f2;
  font-family: inherit;
  word-break: break-all;
  line-height: 1.5;
}
.dr-copy {
  flex-shrink: 0;
  border: 1px solid #3a4568;
  background: #1c2438;
  color: #c6cede;
  border-radius: 7px;
  padding: 6px 11px;
  font-size: 12px;
  cursor: pointer;
}
.dr-copy:hover:not(:disabled) { border-color: #4a5a8a; color: #eef2f9; }
.dr-btn-primary {
  width: 100%;
  border: 1px solid #3b6ef5;
  background: #3b6ef5;
  color: #ffffff;
  border-radius: 9px;
  padding: 12px 14px;
  font-size: 15px;
  font-weight: 600;
  cursor: pointer;
}
.dr-btn-primary:hover:not(:disabled) { filter: brightness(1.1); }
.dr-btn-primary:disabled { opacity: 0.6; cursor: default; }

/* ===== B/C 共用内部元素 ===== */
.dr-block { margin-top: 14px; }
.dr-block + .dr-block { padding-top: 14px; border-top: 1px dashed #232b42; }
.dr-block-head { font-size: 13.5px; font-weight: 650; margin-bottom: 5px; color: #dce4f2; }
.dr-sec-desc { margin: 0 0 10px; font-size: 12.5px; color: #9aa5bd; line-height: 1.65; }
.dr-path-inline {
  margin: 0 0 10px; font-size: 12.5px; color: #9aa5bd;
}
.dr-path-inline code { color: #dce4f2; word-break: break-all; }
.dr-restore-loc { margin: 10px 0; display: flex; flex-direction: column; gap: 6px; }
.dr-radio { display: flex; align-items: center; gap: 6px; font-size: 12.5px; color: #c6cede; cursor: pointer; }
.dr-row { display: flex; gap: 8px; margin-bottom: 10px; }
.dr-input {
  flex: 1;
  min-width: 0;
  background: #0f1422;
  border: 1px solid #2c3550;
  border-radius: 8px;
  color: #e8ecf3;
  padding: 8px 10px;
  font-size: 12.5px;
}
.dr-mini {
  border: 1px solid #3a4568;
  background: #232b42;
  color: #e8ecf3;
  border-radius: 8px;
  padding: 8px 12px;
  font-size: 12.5px;
  cursor: pointer;
  white-space: nowrap;
}
.dr-mini:hover:not(:disabled) { border-color: #4a5a8a; }
.dr-btn {
  width: 100%;
  border: 1px solid #3a4568;
  background: #232b42;
  color: #e8ecf3;
  border-radius: 8px;
  padding: 10px 14px;
  font-size: 13px;
  cursor: pointer;
}
.dr-btn:hover:not(:disabled) { border-color: #4a5a8a; background: #28304a; }
.dr-btn.primary { background: #3b6ef5; border-color: #3b6ef5; }
.dr-btn.danger { background: #40253a; border-color: #7a3b5c; }
.dr-btn:disabled { opacity: 0.6; cursor: default; }
.dr-target { font-size: 12.5px; color: #9aa5bd; word-break: break-all; }
.dr-hint { font-size: 11.5px; color: #7c8aa8; line-height: 1.65; }
.dr-error { margin: 8px 0 0; font-size: 12.5px; color: #ff8fa3; line-height: 1.6; word-break: break-all; }
.dr-actions { display: flex; gap: 10px; margin-bottom: 10px; }
</style>

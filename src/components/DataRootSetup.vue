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
      <h1 class="dr-title">请选择抽屉柜的数据保存位置</h1>
      <p class="dr-sub">数据丢失很难恢复。大多数用户直接使用推荐位置即可；数据会一直保存在你选择的地方。</p>

      <p v-if="notice" class="dr-notice">{{ notice }}</p>

      <!-- 主入口：唯一主按钮 -->
      <div class="dr-primary" :class="{ disabled: busy }">
        <button class="dr-btn-primary" :disabled="busy" @click="onChooseRecommended">
          💾 使用推荐位置
        </button>
        <code class="dr-card-path">将保存到：{{ mode.config_root }}</code>
        <p v-if="errRecommended" class="dr-error">{{ errRecommended }}</p>
      </div>

      <!-- 次级入口 1：自定义数据位置 -->
      <div class="dr-sec">
        <button class="dr-sec-head" :disabled="busy" @click="showCustom = !showCustom">
          <span class="dr-sec-arrow">{{ showCustom ? "▾" : "▸" }}</span>
          想放到 D 盘等其他位置？
        </button>
        <div v-if="showCustom" class="dr-sec-body">
          <p class="dr-sec-desc">可将核心数据保存到 D 盘等其他本地磁盘。</p>
          <div class="dr-row">
            <input v-model="customPath" class="dr-input" placeholder="例如 D:\抽屉柜数据" :disabled="busy" />
            <button class="dr-mini" :disabled="busy" @click="onPickCustom">选择位置</button>
          </div>
          <button class="dr-btn" :disabled="busy || !customPath.trim()" @click="onBeginCustom">
            {{ busy ? "处理中…" : "使用该位置" }}
          </button>
          <p v-if="errCustom" class="dr-error">{{ errCustom }}</p>
        </div>
      </div>

      <!-- 次级入口 2：恢复数据（已有目录 / 加密备份，保持可发现性） -->
      <div class="dr-sec">
        <button class="dr-sec-head" :disabled="busy" @click="showRecover = !showRecover">
          <span class="dr-sec-arrow">{{ showRecover ? "▾" : "▸" }}</span>
          恢复数据：重装过、换过电脑，或有加密备份文件
        </button>
        <div v-if="showRecover" class="dr-sec-body">
          <!-- 已有数据目录 -->
          <div class="dr-block">
            <div class="dr-block-head">🔄 使用已有数据目录</div>
            <p class="dr-sec-desc">重新安装 Windows 或抽屉柜后，连接原来的数据继续使用。</p>
            <div class="dr-row">
              <input v-model="attachPath" class="dr-input" placeholder="选择原数据所在目录" :disabled="busy" />
              <button class="dr-mini" :disabled="busy" @click="onPickAttach">选择位置</button>
            </div>
            <button class="dr-btn" :disabled="busy || !attachPath.trim()" @click="onAttachExisting">
              {{ busy ? "处理中…" : "连接该目录" }}
            </button>
            <p v-if="errAttach" class="dr-error">{{ errAttach }}</p>
          </div>
          <!-- 从加密备份恢复（Phase 2C-5，第四入口） -->
          <div class="dr-block">
            <div class="dr-block-head">🗄️ 从加密备份恢复</div>
            <p class="dr-sec-desc">更换电脑或丢失数据后，用之前导出的 .drawerbox 备份文件恢复。</p>
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
                <button class="dr-mini" :disabled="busy" @click="onPickRestoreCustom">选择位置</button>
              </div>
            </div>
            <button class="dr-btn primary" :disabled="busy || !backupFilePath.trim() || !backupPassword" @click="onBeginBackupRestore">
              {{ busy ? "恢复中…" : "从备份恢复" }}
            </button>
            <p class="dr-hint">恢复使用备份自身的主密码与恢复短语；数据将恢复到你选择的上述位置，与备份原来的位置无关。</p>
            <p v-if="errBackup" class="dr-error">{{ errBackup }}</p>
          </div>
        </div>
      </div>
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
  background: linear-gradient(160deg, #0b1020 0%, #131a2e 100%);
  color: #e8ecf3;
  box-sizing: border-box;
}
.dr-topbar { height: 28px; width: 100%; }
.dr-body { padding: 8px 22px 24px; }
.dr-title { margin: 0 0 8px; font-size: 17px; }
.dr-sub { margin: 0 0 14px; font-size: 12px; line-height: 1.7; color: #9aa5bd; }
.dr-notice { margin: 0 0 12px; font-size: 12px; color: #6fd0a0; }

/* Batch 1.5：主入口（唯一主按钮 + 路径小字 + 就近错误） */
.dr-primary {
  border: 1px solid #2c3550;
  background: #161c2c;
  border-radius: 12px;
  padding: 14px;
  margin-bottom: 12px;
}
.dr-primary.disabled { opacity: 0.6; }
.dr-btn-primary {
  width: 100%;
  border: 1px solid #3b6ef5;
  background: #3b6ef5;
  color: #ffffff;
  border-radius: 8px;
  padding: 12px 14px;
  font-size: 15px;
  font-weight: 600;
  cursor: pointer;
}
.dr-btn-primary:hover:not(:disabled) { filter: brightness(1.1); }
.dr-btn-primary:disabled { opacity: 0.6; cursor: default; }
.dr-primary .dr-card-path { margin-top: 10px; }

/* Batch 1.5：次级折叠入口 */
.dr-sec {
  border: 1px solid #232b42;
  background: #131a2a;
  border-radius: 10px;
  margin-bottom: 10px;
  overflow: hidden;
}
.dr-sec-head {
  width: 100%;
  display: flex;
  align-items: center;
  gap: 8px;
  background: transparent;
  border: none;
  color: #c6cede;
  font-size: 13px;
  padding: 11px 14px;
  cursor: pointer;
  text-align: left;
}
.dr-sec-head:hover:not(:disabled) { color: #e8ecf3; }
.dr-sec-head:disabled { opacity: 0.6; cursor: default; }
.dr-sec-arrow { color: #7c8aa8; font-size: 11px; width: 12px; }
.dr-sec-body { padding: 2px 14px 14px; border-top: 1px solid #232b42; }
.dr-sec-desc { margin: 10px 0; font-size: 12px; color: #9aa5bd; line-height: 1.6; }
.dr-block { margin-top: 12px; }
.dr-block + .dr-block { padding-top: 12px; border-top: 1px dashed #232b42; }
.dr-block-head { font-size: 13px; font-weight: 600; margin-bottom: 4px; }

.dr-backup-panel {
  margin-top: 10px;
  padding: 10px;
  border: 1px solid #2c3550;
  border-radius: 8px;
  background: #101624;
}
.dr-restore-loc { margin: 10px 0; display: flex; flex-direction: column; gap: 6px; }
.dr-radio { display: flex; align-items: center; gap: 6px; font-size: 12px; color: #c6cede; cursor: pointer; }
.dr-card {
  border: 1px solid #2c3550;
  background: #161c2c;
  border-radius: 12px;
  padding: 14px;
  cursor: pointer;
  transition: border-color 0.15s;
}
.dr-card:not(.disabled):hover { border-color: #3b6ef5; }
.dr-card.disabled { opacity: 0.6; cursor: default; }
.dr-card-head { display: flex; align-items: center; gap: 8px; margin-bottom: 6px; }
.dr-card-icon { font-size: 16px; }
.dr-card-title { font-size: 14px; font-weight: 600; }
.dr-card-desc { margin: 0 0 10px; font-size: 12px; color: #9aa5bd; line-height: 1.6; }
.dr-card-path {
  display: block;
  font-size: 11px;
  color: #7c8aa8;
  background: #0f1422;
  border-radius: 6px;
  padding: 6px 8px;
  word-break: break-all;
}
.dr-row { display: flex; gap: 8px; margin-bottom: 10px; }
.dr-input {
  flex: 1;
  min-width: 0;
  background: #0f1422;
  border: 1px solid #2c3550;
  border-radius: 8px;
  color: #e8ecf3;
  padding: 7px 10px;
  font-size: 12px;
}
.dr-mini {
  border: 1px solid #3a4568;
  background: #232b42;
  color: #e8ecf3;
  border-radius: 8px;
  padding: 7px 12px;
  font-size: 12px;
  cursor: pointer;
  white-space: nowrap;
}
.dr-btn {
  width: 100%;
  border: 1px solid #3a4568;
  background: #232b42;
  color: #e8ecf3;
  border-radius: 8px;
  padding: 9px 14px;
  font-size: 13px;
  cursor: pointer;
}
.dr-btn.primary { background: #3b6ef5; border-color: #3b6ef5; }
.dr-btn.danger { background: #40253a; border-color: #7a3b5c; }
.dr-btn:disabled { opacity: 0.6; cursor: default; }
.dr-actions { display: flex; gap: 10px; margin-bottom: 10px; }
.dr-target { font-size: 12px; color: #9aa5bd; word-break: break-all; }
.dr-hint { font-size: 11px; color: #7c8aa8; }
.dr-error { margin: 8px 0 0; font-size: 12px; color: #ff8fa3; line-height: 1.6; word-break: break-all; }
</style>

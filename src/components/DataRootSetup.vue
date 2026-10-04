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
} from "../api";
import type { SetupModeInfo } from "../api";

const props = defineProps<{ mode: SetupModeInfo }>();

const busy = ref(false);
const error = ref("");
const notice = ref("");
const customPath = ref("");
const attachPath = ref("");

const isChoose = computed(() => props.mode.kind === "choose");
const isRecover = computed(() => !isChoose.value);

function reload() {
  // 选择完成后回到正常启动流程（is_first_run → 主密码设置向导）
  window.location.reload();
}

async function onChooseRecommended() {
  if (busy.value) return;
  busy.value = true;
  error.value = "";
  try {
    await setupChooseDefault();
    reload();
  } catch (e) {
    error.value = String(e);
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
    error.value = "请先选择数据保存位置";
    return;
  }
  busy.value = true;
  error.value = "";
  notice.value = "";
  try {
    await setupCheckCustomRoot(target);
    notice.value = "位置可用，正在创建…";
    await setupBeginCustomInit(target);
    reload();
  } catch (e) {
    error.value = String(e);
  } finally {
    busy.value = false;
  }
}

async function onAttachExisting() {
  if (busy.value) return;
  const target = attachPath.value.trim();
  if (!target) {
    error.value = "请先选择已有数据目录";
    return;
  }
  busy.value = true;
  error.value = "";
  notice.value = "";
  try {
    notice.value = "正在验证数据…";
    await setupAttachExisting(target); // 成功后后端重启进程
  } catch (e) {
    error.value = String(e);
    notice.value = "";
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
      <h1 class="dr-title">请选择抽屉柜的数据保存位置</h1>
      <p class="dr-sub">软件可以重新安装，数据丢失却很难恢复。请选择抽屉柜的数据保存位置。</p>

      <div class="dr-cards">
        <div class="dr-card" :class="{ disabled: busy }" @click="onChooseRecommended">
          <div class="dr-card-head"><span class="dr-card-icon">💾</span><span class="dr-card-title">使用推荐位置</span></div>
          <p class="dr-card-desc">适合大多数用户。数据保存在本机用户目录中。</p>
          <code class="dr-card-path">{{ mode.config_root }}</code>
        </div>

        <div class="dr-card" :class="{ disabled: busy }">
          <div class="dr-card-head"><span class="dr-card-icon">📁</span><span class="dr-card-title">自定义数据位置</span></div>
          <p class="dr-card-desc">可将核心数据保存到 D 盘等其他本地磁盘。</p>
          <div class="dr-row">
            <input v-model="customPath" class="dr-input" placeholder="例如 D:\抽屉柜数据" :disabled="busy" />
            <button class="dr-mini" :disabled="busy" @click="onPickCustom">选择位置</button>
          </div>
          <button class="dr-btn primary" :disabled="busy || !customPath.trim()" @click="onBeginCustom">
            {{ busy ? "处理中…" : "使用该位置" }}
          </button>
        </div>

        <div class="dr-card" :class="{ disabled: busy }">
          <div class="dr-card-head"><span class="dr-card-icon">🔄</span><span class="dr-card-title">使用已有数据目录</span></div>
          <p class="dr-card-desc">适合重新安装 Windows、重新安装抽屉柜后继续使用原来的数据。</p>
          <div class="dr-row">
            <input v-model="attachPath" class="dr-input" placeholder="选择原数据所在目录" :disabled="busy" />
            <button class="dr-mini" :disabled="busy" @click="onPickAttach">选择位置</button>
          </div>
          <button class="dr-btn" :disabled="busy || !attachPath.trim()" @click="onAttachExisting">
            {{ busy ? "处理中…" : "连接该目录" }}
          </button>
        </div>
      </div>

      <p v-if="notice" class="dr-notice">{{ notice }}</p>
      <p v-if="error" class="dr-error">{{ error }}</p>
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
.dr-sub { margin: 0 0 16px; font-size: 12px; line-height: 1.7; color: #9aa5bd; }
.dr-cards { display: flex; flex-direction: column; gap: 12px; }
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
.dr-notice { font-size: 12px; color: #6fd0a0; }
.dr-error { font-size: 12px; color: #ff8fa3; line-height: 1.6; }
</style>

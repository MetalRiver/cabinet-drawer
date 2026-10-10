<script setup lang="ts">
// ===== Settings UI V2 · 安全与密码 Tab =====
// 全部安全逻辑从旧 SettingsView 逐字迁移：主密码（v2/legacy 双路径）、
// 恢复短语两阶段轮换（v2）、独立二次验证密码（legacy）、紧急救援（legacy）。
// 安全语义零改动：确认流、认证条件、敏感输入清理全部保留；
// 差异仅在生命周期——旧版 onUnmounted 清理改为「离开本分类时」清理（Tab 常驻挂载）。
import { ref, reactive, computed, watch } from "vue";
import {
  getSecurityStatus,
  changeV2MasterPassword,
  changeMasterPassword,
  prepareV2RecoveryRotation,
  confirmV2RecoveryRotation,
  cancelV2RecoveryRotation,
  rescuePasswordsWithMaster,
  hasSecondPassword as apiHasSecondPassword,
  changeSecondPassword,
} from "../../../api";
import { useAppStore } from "../../../stores/app";
import { usePasswordStore } from "../../../stores/passwords";
import ConfirmInline from "../../../components/ConfirmInline.vue";
import SettingsCard from "../../../components/settings/SettingsCard.vue";
import StatusLine from "../../../components/settings/StatusLine.vue";

const props = defineProps<{ active: boolean }>();

const appStore = useAppStore();
const passwordStore = usePasswordStore();

const securityModel = ref<"legacy_security_model" | "stable_dek_v2" | null>(null);
const isV2Security = computed(() => securityModel.value === "stable_dek_v2");
async function loadSecurityModel() {
  try {
    securityModel.value = (await getSecurityStatus()).security_model;
  } catch {
    securityModel.value = null;
  }
}
loadSecurityModel();

// ===== 主密码 =====
const currentPassword = ref("");
const newPassword = ref("");
const confirmPassword = ref("");
const pwChanging = ref(false);
const pwChangeResult = ref("");
const pwChangeConfirmOpen = ref(false);
function clearMasterPasswordInputs() {
  currentPassword.value = "";
  newPassword.value = "";
  confirmPassword.value = "";
  pwChangeConfirmOpen.value = false;
}
async function onChangePassword() {
  if (pwChanging.value) return;
  if (!currentPassword.value) { pwChangeResult.value = "请输入当前密码"; return; }
  if (newPassword.value.length < 6) { pwChangeResult.value = "新密码至少 6 位"; return; }
  if (newPassword.value !== confirmPassword.value) { pwChangeResult.value = "两次输入的新密码不一致"; return; }
  if (newPassword.value === currentPassword.value) { pwChangeResult.value = "新密码不能与当前密码相同"; return; }
  if (securityModel.value === null) { pwChangeResult.value = "无法确认当前安全模式，请重试"; return; }
  // legacy 重加密为高危操作 → 应用内确认（v2 路径本身无需确认，保持原逻辑）
  if (!isV2Security.value) {
    if (!pwChangeConfirmOpen.value) {
      pwChangeConfirmOpen.value = true;
      return;
    }
    pwChangeConfirmOpen.value = false;
  }
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
  } finally {
    pwChanging.value = false;
  }
}

// ===== v2 恢复短语两阶段轮换 =====
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

// ===== 独立二次验证密码（legacy-only）=====
const pw2ndEnabled = ref(false);
const pw2ndOld = ref("");
const pw2ndNew = ref("");
const pw2ndNewConfirm = ref("");
const pw2ndBusy = ref(false);
const pw2ndHint = ref("");
const pw2ndConfirm = ref<"" | "set" | "clear">("");
async function refreshPw2ndEnabled() {
  try {
    pw2ndEnabled.value = await apiHasSecondPassword();
  } catch {
    pw2ndEnabled.value = false;
  }
}
refreshPw2ndEnabled();
async function onSetPw2nd() {
  if (pw2ndBusy.value) return;
  if (!pw2ndOld.value) { pw2ndHint.value = pw2ndEnabled.value ? "请输入旧的二次验证密码" : "请先输入当前主密码（用于验证身份）"; return; }
  if (pw2ndNew.value.length < 6) { pw2ndHint.value = "新二次验证密码至少 6 位"; return; }
  if (pw2ndNew.value !== pw2ndNewConfirm.value) { pw2ndHint.value = "两次新密码不一致"; return; }
  if (pw2ndConfirm.value !== "set") {
    pw2ndConfirm.value = "set";
    return;
  }
  pw2ndConfirm.value = "";
  pw2ndBusy.value = true;
  pw2ndHint.value = "";
  try {
    await changeSecondPassword(pw2ndOld.value, pw2ndNew.value);
    pw2ndHint.value = pw2ndEnabled.value ? "✓ 独立二次验证密码已更新" : "✓ 已启用独立二次验证密码（查看/复制密码时生效）";
    appStore.showClipToast("success", pw2ndEnabled.value ? "二次验证密码已更新" : "二次验证密码已启用");
    pw2ndOld.value = ""; pw2ndNew.value = ""; pw2ndNewConfirm.value = "";
    void refreshPw2ndEnabled();
  } catch (e) {
    pw2ndHint.value = "✗ " + String(e);
  } finally {
    pw2ndBusy.value = false;
  }
}
async function onClearPw2nd() {
  if (!pw2ndBusy.value && pw2ndEnabled.value) {
    if (!pw2ndOld.value) { pw2ndHint.value = "请先输入当前二次验证密码（验证身份后才能关闭）"; return; }
    if (pw2ndConfirm.value !== "clear") {
      pw2ndConfirm.value = "clear";
      return;
    }
    pw2ndConfirm.value = "";
    pw2ndBusy.value = true;
    pw2ndHint.value = "";
    try {
      await changeSecondPassword(pw2ndOld.value, "");
      pw2ndHint.value = "✓ 已清空，二次验证恢复跟随主密码";
      appStore.showClipToast("success", "已关闭独立二次验证");
      pw2ndOld.value = ""; pw2ndNew.value = ""; pw2ndNewConfirm.value = "";
      void refreshPw2ndEnabled();
    } catch (e) {
      pw2ndHint.value = "✗ " + String(e);
    } finally {
      pw2ndBusy.value = false;
    }
  }
}

// ===== 紧急救援（legacy-only）=====
const rescuePw = reactive({
  oldMaster: "",
  busy: false,
  hint: "",
  lastStats: null as null | { tried: number; rescued: number; failed: number },
});
const rescueConfirmOpen = ref(false);
async function onRescuePasswords() {
  if (rescuePw.busy) return;
  if (!rescuePw.oldMaster) { rescuePw.hint = "请先输入当初加密这些密码时用的主密码"; return; }
  if (!rescueConfirmOpen.value) {
    rescueConfirmOpen.value = true;
    return;
  }
  rescueConfirmOpen.value = false;
  rescuePw.busy = true;
  rescuePw.hint = "";
  rescuePw.lastStats = null;
  try {
    const r = await rescuePasswordsWithMaster(rescuePw.oldMaster);
    rescuePw.lastStats = r;
    if (r.failed === 0) {
      rescuePw.hint = `✓ 救援成功！共检查 ${r.tried} 条，救回 ${r.rescued} 条（其余本来就能解）`;
      appStore.showClipToast("success", `密码救援成功：救回 ${r.rescued} 条`);
      rescuePw.oldMaster = "";
      try { await passwordStore.reloadAll?.(); } catch { /* store 可能无此方法 */ }
      try { await passwordStore.refresh?.(); } catch { /* 同上 */ }
    } else {
      rescuePw.hint = `⚠️ 部分救援失败：救回 ${r.rescued} 条，仍有 ${r.failed} 条无法解密（密码不对？）`;
      appStore.showClipToast("info", `部分失败：${r.failed} 条密码仍无法解密`);
    }
  } catch (e) {
    rescuePw.hint = "✗ " + String(e);
  } finally {
    rescuePw.busy = false;
  }
}

// ===== 生命周期：离开本分类时清理敏感输入 + 撤销未完成的恢复短语轮换 =====
// （旧版为 onUnmounted 清理；Tab 常驻挂载后改为 active 监听，语义等价）
watch(
  () => props.active,
  (active) => {
    if (active) return;
    clearMasterPasswordInputs();
    pw2ndOld.value = ""; pw2ndNew.value = ""; pw2ndNewConfirm.value = ""; pw2ndConfirm.value = "";
    rescuePw.oldMaster = "";
    rescueConfirmOpen.value = false;
    const token = recoveryRotationToken.value;
    clearRecoveryRotationLocal();
    if (token) void cancelV2RecoveryRotation(token).catch(() => {});
  }
);
</script>

<template>
  <div class="tab-pane">
    <SettingsCard
      title="主密码"
      :badge="{ text: '核心', tone: 'purple' }"
      subtitle="解锁抽屉柜与加密密码数据的钥匙，请务必牢记。"
    >
      <p class="s-hintbox" :class="isV2Security ? 's-hintbox-purple' : ''">
        <template v-if="isV2Security">修改后只更新主密码保护，密码数据与 Recovery Phrase 保持不变。</template>
        <template v-else>修改后会立即用新密码重加密所有密码条目和恢复短语（保持解锁状态）。<br />
          <b>注意：</b>修改主密码<b>不会</b>影响「独立二次验证密码」（两套密码独立）。</template>
      </p>
      <div class="s-form-grid">
        <div class="s-form-group">
          <label class="s-form-label">当前主密码</label>
          <input v-model="currentPassword" type="password" class="s-form-input" placeholder="请输入当前主密码" autocomplete="off" data-interactive />
        </div>
        <div class="s-form-group">
          <label class="s-form-label">新主密码</label>
          <input v-model="newPassword" type="password" class="s-form-input" placeholder="至少 6 位" autocomplete="new-password" data-interactive />
        </div>
        <div class="s-form-group">
          <label class="s-form-label">确认新主密码</label>
          <input v-model="confirmPassword" type="password" class="s-form-input" placeholder="再次输入新主密码" autocomplete="new-password" data-interactive />
        </div>
      </div>
      <div class="s-action-row">
        <button class="s-btn-primary" :disabled="pwChanging" @click="onChangePassword" data-interactive>
          {{ pwChanging ? (isV2Security ? "保存中…" : "重加密中…") : "🔐 修改主密码" }}
        </button>
      </div>
      <ConfirmInline
        v-if="pwChangeConfirmOpen"
        title="修改主密码将重新加密所有密码和恢复短语（约 1-3 秒），确定继续吗？"
        confirm-text="确认重加密"
        @cancel="pwChangeConfirmOpen = false"
        @confirm="onChangePassword"
      />
      <StatusLine :tone="pwChangeResult.startsWith('✓') ? 'ok' : pwChangeResult.startsWith('✗') ? 'err' : 'info'" :text="pwChangeResult" />
    </SettingsCard>

    <SettingsCard
      v-if="isV2Security"
      title="更换恢复短语"
      :badge="{ text: 'v2', tone: 'green' }"
      subtitle="生成并确认新的 12 个恢复词后，旧恢复短语立即失效；确认完成前旧的仍然有效。"
      intent="security"
    >
      <div v-if="recoveryRotationStep === 'idle'" class="s-action-row" style="border-top: none; padding-top: 0;">
        <button class="s-btn-primary s-btn-green" :disabled="recoveryRotationBusy" @click="startRecoveryRotation" data-interactive>
          {{ recoveryRotationBusy ? "生成中…" : "更换恢复短语" }}
        </button>
      </div>

      <template v-else-if="recoveryRotationStep === 'words'">
        <div class="rr-grid">
          <div v-for="(word, index) in recoveryRotationWords" :key="index" class="rr-word">
            <span>{{ index + 1 }}</span><b>{{ word }}</b>
          </div>
        </div>
        <label class="rr-check">
          <input v-model="recoveryRotationSaved" type="checkbox" data-interactive />
          <span>我已将新的恢复短语保存到安全位置</span>
        </label>
        <div class="s-action-row">
          <button class="s-btn-primary s-btn-green" :disabled="!recoveryRotationSaved || recoveryRotationBusy" @click="beginRecoveryConfirmation" data-interactive>
            验证已保存
          </button>
          <button class="s-btn-secondary" :disabled="recoveryRotationBusy" @click="cancelRecoveryRotation" data-interactive>取消</button>
        </div>
      </template>

      <template v-else>
        <p class="s-hint">请按编号输入刚才保存的新恢复词：</p>
        <div class="s-form-grid">
          <div v-for="(wordIndex, inputIndex) in recoveryRotationIndexes" :key="wordIndex" class="s-form-group">
            <label class="s-form-label">第 {{ wordIndex + 1 }} 个词</label>
            <input
              v-model="recoveryRotationInputs[inputIndex]"
              type="text"
              class="s-form-input"
              autocomplete="off"
              :spellcheck="false"
              placeholder="输入恢复词"
              data-interactive
            />
          </div>
        </div>
        <div class="s-action-row">
          <button class="s-btn-primary s-btn-green" :disabled="!recoveryRotationMatches || recoveryRotationBusy" @click="finishRecoveryRotation" data-interactive>
            {{ recoveryRotationBusy ? "确认中…" : "确认并立即更换" }}
          </button>
          <button class="s-btn-secondary" :disabled="recoveryRotationBusy" @click="cancelRecoveryRotation" data-interactive>取消</button>
        </div>
      </template>

      <StatusLine
        v-if="recoveryRotationHint"
        :tone="recoveryRotationHint.startsWith('✓') ? 'ok' : 'err'"
        :text="recoveryRotationHint"
      />
    </SettingsCard>

    <SettingsCard
      v-if="!isV2Security"
      title="密码区二次验证"
      :badge="pw2ndEnabled ? { text: '已启用独立密码', tone: 'green' } : { text: '跟随主密码（默认）', tone: 'gray' }"
      subtitle="查看/复制密码时的第二道验证。"
      intent="security"
    >
      <p class="s-hintbox s-hintbox-green">
        <b>默认行为：</b>查看 / 复制任何密码需输入<b>主密码</b>二次确认。<br />
        <b>启用独立密码后（推荐）：</b>二次验证只认这套独立密码，<b>主密码无效</b>——即使未锁屏，陌生人也无法查看/复制任何密码。
      </p>
      <div class="s-form-grid">
        <div class="s-form-group">
          <label class="s-form-label">
            {{ pw2ndEnabled ? "旧的二次验证密码（验证身份）" : "当前主密码（验证身份后启用独立密码）" }}
          </label>
          <input v-model="pw2ndOld" type="password" class="s-form-input" :placeholder="pw2ndEnabled ? '请输入旧二次验证密码' : '请先输入主密码'" autocomplete="off" data-interactive />
        </div>
        <div class="s-form-group">
          <label class="s-form-label">
            {{ pw2ndEnabled ? "新的二次验证密码" : "新二次验证密码（至少 6 位，且 ≠ 主密码）" }}
          </label>
          <input v-model="pw2ndNew" type="password" class="s-form-input" placeholder="至少 6 位" autocomplete="new-password" data-interactive />
        </div>
        <div class="s-form-group">
          <label class="s-form-label">再确认一次新二次验证密码</label>
          <input v-model="pw2ndNewConfirm" type="password" class="s-form-input" placeholder="请再次输入" autocomplete="new-password" data-interactive />
        </div>
      </div>
      <div class="s-action-row">
        <button class="s-btn-primary s-btn-green" :disabled="pw2ndBusy" @click="onSetPw2nd" data-interactive>
          {{ pw2ndBusy ? "处理中…" : (pw2ndEnabled ? "✎ 修改独立二次验证密码" : "🚀 启用独立二次验证密码") }}
        </button>
        <button
          v-if="pw2ndEnabled"
          class="s-btn-secondary s-btn-warn"
          :disabled="pw2ndBusy"
          @click="onClearPw2nd"
          data-interactive
        >🗑️ 清空，改回跟随主密码</button>
      </div>
      <ConfirmInline
        v-if="pw2ndConfirm === 'set'"
        :title="pw2ndEnabled
          ? '将修改独立二次验证密码（≠主密码），确定继续吗？'
          : '即将启用独立二次验证密码：启用后，查看/复制任何密码需再输它（主密码无效），且它不能与主密码相同。确定继续吗？'"
        confirm-text="确认"
        @cancel="pw2ndConfirm = ''"
        @confirm="onSetPw2nd"
      />
      <ConfirmInline
        v-if="pw2ndConfirm === 'clear'"
        title="即将清空独立二次验证密码，改回「跟随主密码」。确定继续吗？"
        confirm-text="确认清空"
        @cancel="pw2ndConfirm = ''"
        @confirm="onClearPw2nd"
      />
      <StatusLine :tone="pw2ndHint.startsWith('✓') ? 'ok' : pw2ndHint.startsWith('✗') ? 'err' : 'warn'" :text="pw2ndHint" />
    </SettingsCard>

    <SettingsCard
      v-if="!isV2Security"
      title="紧急救援：密码解密失败？"
      :badge="{ text: '救命按钮', tone: 'amber' }"
      subtitle="输入「当初加密这些密码时用的主密码」，用它解密所有密码后以当前主密钥重新加密写回。"
      intent="warning"
    >
      <p class="s-hintbox s-hintbox-green">
        <b>什么情况用它？</b><br />
        改完主密码 / 启用或修改独立密码 / 换电脑导入旧备份后，所有密码显示「解密失败」。
      </p>
      <div class="s-form-group">
        <label class="s-form-label">当初加密时用的主密码</label>
        <input v-model="rescuePw.oldMaster" type="password" class="s-form-input" placeholder="请输入旧主密码" autocomplete="off" data-interactive />
      </div>
      <div class="s-action-row">
        <button class="s-btn-primary s-btn-warn" :disabled="rescuePw.busy" @click="onRescuePasswords" data-interactive>
          {{ rescuePw.busy ? "救援中…" : "🚨 立刻救回我的密码" }}
        </button>
      </div>
      <ConfirmInline
        v-if="rescueConfirmOpen"
        title="将用「当初加密时用的主密码」尝试解密所有密码，并用当前主密钥重新加密写回。确定继续吗？"
        confirm-text="开始救援"
        @cancel="rescueConfirmOpen = false"
        @confirm="onRescuePasswords"
      />
      <template v-if="rescuePw.lastStats">
        <StatusLine
          :tone="rescuePw.lastStats.failed === 0 ? 'ok' : 'err'"
          :text="`救援结果：共检查 ${rescuePw.lastStats.tried} 条，成功救回 ${rescuePw.lastStats.rescued} 条，仍失败 ${rescuePw.lastStats.failed} 条（失败通常是旧主密码不对，或密码本身已损坏）`"
        />
      </template>
      <StatusLine
        v-if="rescuePw.hint"
        :tone="rescuePw.hint.startsWith('✓') ? 'ok' : rescuePw.hint.startsWith('✗') ? 'err' : 'warn'"
        :text="rescuePw.hint"
      />
    </SettingsCard>
  </div>
</template>

<style scoped>
.rr-grid {
  display: grid;
  grid-template-columns: repeat(3, minmax(0, 1fr));
  gap: 7px;
  margin: 4px 0 12px;
}
.rr-word {
  display: flex;
  align-items: center;
  gap: 6px;
  min-width: 0;
  padding: 8px;
  border: 1px solid var(--border);
  border-radius: 8px;
  background: var(--bg-secondary);
  font-size: 11.5px;
}
.rr-word span { color: var(--text-muted); }
.rr-word b { overflow: hidden; color: var(--text-primary); text-overflow: ellipsis; }
.rr-check {
  display: flex;
  align-items: center;
  gap: 8px;
  margin: 10px 0;
  color: var(--text-muted);
  font-size: 12px;
  cursor: pointer;
}
</style>

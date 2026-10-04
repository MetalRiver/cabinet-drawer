<script setup lang="ts">
import { ref, computed, onBeforeUnmount } from "vue";
import { useAppStore } from "../stores/app";
import { finalizeV2Security, initializeV2Security, passwordStrength as apiStrength, restartApp } from "../api";
import { useWindowDrag } from "../composables/useWindowDrag";
// 品牌 Logo
import logo from "@/assets/logo.png";

// 拖动支持：初始设置页也要能拖动窗口
const { onDragHandleMouseDown } = useWindowDrag();

const appStore = useAppStore();
const step = ref(1);
const masterPassword = ref("");
const confirmPassword = ref("");
const showPassword = ref(false);
const strength = ref(0);
const strengthLabel = ref("");
const strengthColor = ref("");
const recoveryWords = ref<string[]>([]);
const recoveryConfirmed = ref(false);
const confirmationIndexes = ref<number[]>([]);
const confirmationInputs = ref<string[]>(["", "", ""]);
const submitting = ref(false);
const restarting = ref(false);
const error = ref("");

// 强度检测
async function checkStrength() {
  if (!masterPassword.value) {
    strength.value = 0;
    strengthLabel.value = "";
    return;
  }
  try {
    strength.value = await apiStrength(masterPassword.value);
  } catch {
    strength.value = 0;
  }
  if (strength.value < 30) {
    strengthLabel.value = "弱";
    strengthColor.value = "#ff5e7e";
  } else if (strength.value < 60) {
    strengthLabel.value = "一般";
    strengthColor.value = "#ffb84d";
  } else if (strength.value < 85) {
    strengthLabel.value = "强";
    strengthColor.value = "#3ddc97";
  } else {
    strengthLabel.value = "非常强";
    strengthColor.value = "#4fc3f7";
  }
}

const canProceed1 = computed(
  () =>
    masterPassword.value.length >= 6 &&
    masterPassword.value === confirmPassword.value &&
    strength.value >= 30
);

async function nextStep() {
  error.value = "";
  if (step.value === 1) {
    if (!canProceed1.value) {
      error.value = "请检查密码要求";
      return;
    }
    submitting.value = true;
    try {
      recoveryWords.value = await initializeV2Security(masterPassword.value);
      masterPassword.value = "";
      confirmPassword.value = "";
      step.value = 2;
    } catch (e) {
      error.value = String(e);
    } finally {
      submitting.value = false;
    }
  } else if (step.value === 2) {
    if (!recoveryConfirmed.value) {
      error.value = "请确认已保存恢复短语";
      return;
    }
    const indexes = new Set<number>();
    while (indexes.size < 3) {
      const random = new Uint32Array(1);
      window.crypto.getRandomValues(random);
      indexes.add(random[0] % 12);
    }
    confirmationIndexes.value = [...indexes].sort((a, b) => a - b);
    confirmationInputs.value = ["", "", ""];
    step.value = 3;
  }
}

const confirmationMatches = computed(() =>
  confirmationIndexes.value.length === 3 &&
  confirmationIndexes.value.every(
    (wordIndex, inputIndex) =>
      confirmationInputs.value[inputIndex].trim().toLowerCase() === recoveryWords.value[wordIndex]
  )
);

async function finishSetup() {
  error.value = "";
  if (!confirmationMatches.value) {
    error.value = "恢复词不匹配，请按编号重新输入";
    return;
  }
  submitting.value = true;
  try {
    await finalizeV2Security();
    recoveryWords.value = [];
    confirmationInputs.value = ["", "", ""];
    confirmationIndexes.value = [];
    appStore.setFirstRun(false);
    appStore.unlock();
    // Phase 2C-2：正式库已就位（推荐位置或自定义位置）→ 重启一次，
    // 让应用以 canonical Data Root 走完整标准启动（托盘 / 全局快捷键 / 启动校验）；
    // 重启后进入锁屏，用刚设置的主密码解锁即可。
    restarting.value = true;
    await restartApp();
  } catch (e) {
    // 正式完成失败时保留当前恢复词与确认输入，允许用户重试，不重新生成。
    error.value = String(e);
    restarting.value = false;
  } finally {
    submitting.value = false;
  }
}

onBeforeUnmount(() => {
  masterPassword.value = "";
  confirmPassword.value = "";
  recoveryWords.value = [];
  confirmationInputs.value = ["", "", ""];
  confirmationIndexes.value = [];
});
</script>

<template>
  <div class="wizard">
    <!-- Phase 2C-2：初始化完成 → 重启进入正式应用（此期间不开放任何操作） -->
    <div v-if="restarting" class="restart-overlay">
      <div class="restart-box">
        <div class="restart-spinner"></div>
        <p class="restart-text">初始化完成，正在启动抽屉柜…</p>
      </div>
    </div>
    <!-- 顶部可拖动条 -->
    <div class="wizard-topbar" data-tauri-drag-region @mousedown="onDragHandleMouseDown">
      <div class="wizard-topbar-left" data-tauri-drag-region>
        <img :src="logo" alt="抽屉柜 Drawer" class="wizard-topbar-logo" />
        <span class="wizard-topbar-title">抽屉柜 Drawer · 初始设置</span>
      </div>
    </div>

    <!-- 背景装饰 -->
    <div class="bg-decoration">
      <div class="bg-orb bg-orb-1"></div>
      <div class="bg-orb bg-orb-2"></div>
    </div>

    <div class="wizard-card">
      <!-- 步骤指示器 -->
      <div class="step-indicator">
        <div class="step" :class="{ active: step >= 1, done: step > 1 }">
          <div class="step-dot">
            <span v-if="step > 1">✓</span>
            <span v-else>1</span>
          </div>
          <div class="step-label">设置主密码</div>
        </div>
        <div class="step-line" :class="{ active: step > 1 }"></div>
        <div class="step" :class="{ active: step >= 2, done: step > 2 }">
          <div class="step-dot">
            <span v-if="step > 2">✓</span>
            <span v-else>2</span>
          </div>
          <div class="step-label">保存恢复短语</div>
        </div>
        <div class="step-line" :class="{ active: step > 2 }"></div>
        <div class="step" :class="{ active: step >= 3 }">
          <div class="step-dot">3</div>
          <div class="step-label">确认恢复词</div>
        </div>
      </div>

      <Transition name="step" mode="out-in">
        <!-- 步骤 1: 设置主密码 -->
        <div v-if="step === 1" key="step1" class="step-content">
          <div class="wizard-icon icon-blue">🔐</div>
          <h1 class="wizard-title">创建主密码</h1>
          <p class="wizard-subtitle">主密码用于加密所有数据。万一忘记，可凭下一步生成的恢复短语重置。</p>

          <div class="form-group">
            <label>主密码</label>
            <div class="input-wrap">
              <input
                v-model="masterPassword"
                :type="showPassword ? 'text' : 'password'"
                class="wizard-input"
                placeholder="至少 6 位"
                @input="checkStrength"
              />
              <button class="input-toggle" @click="showPassword = !showPassword">
                {{ showPassword ? "🙈" : "👁️" }}
              </button>
            </div>
            <!-- 强度条 -->
            <div v-if="masterPassword" class="strength-bar">
              <div class="strength-track">
                <div
                  class="strength-fill"
                  :style="{
                    width: strength + '%',
                    background: strengthColor
                  }"
                ></div>
              </div>
              <div class="strength-label" :style="{ color: strengthColor }">
                {{ strengthLabel }}
              </div>
            </div>
          </div>

          <div class="form-group">
            <label>确认主密码</label>
            <input
              v-model="confirmPassword"
              :type="showPassword ? 'text' : 'password'"
              class="wizard-input"
              placeholder="再次输入"
            />
            <div v-if="confirmPassword && confirmPassword !== masterPassword" class="form-hint error">
              ✗ 两次密码不一致
            </div>
            <div v-else-if="confirmPassword" class="form-hint success">
              ✓ 密码一致
            </div>
          </div>

          <Transition name="error-slide">
            <p v-if="error" class="wizard-error">{{ error }}</p>
          </Transition>

          <button
            class="wizard-btn"
            :disabled="!canProceed1 || submitting"
            @click="nextStep"
          >
            <span v-if="submitting" class="btn-spinner"></span>
            <span v-else>建立安全密码库 →</span>
          </button>
        </div>

        <!-- 步骤 2: 恢复短语 -->
        <div v-else-if="step === 2" key="step2" class="step-content">
          <div class="wizard-icon icon-orange">📜</div>
          <h1 class="wizard-title">保存恢复短语</h1>
          <p class="wizard-subtitle">
            如果忘记主密码，这 12 个词可以帮助你恢复密码库。请把它们保存到安全的地方。
          </p>

          <div class="recovery-grid">
            <div
              v-for="(word, i) in recoveryWords"
              :key="i"
              class="recovery-word"
              :style="{ '--delay': `${i * 0.04}s` }"
            >
              <span class="recovery-num">{{ i + 1 }}</span>
              <span class="recovery-text">{{ word }}</span>
            </div>
          </div>

          <div class="warning-box">
            <span class="warning-icon">⚠️</span>
            <div class="warning-text">
              恢复短语是重置主密码的<strong>唯一方式</strong>。请抄写在纸上，存放在安全的地方。
            </div>
          </div>

          <label class="check-row">
            <input v-model="recoveryConfirmed" type="checkbox" />
            <span>我已抄写下恢复短语</span>
          </label>

          <Transition name="error-slide">
            <p v-if="error" class="wizard-error">{{ error }}</p>
          </Transition>

          <button
            class="wizard-btn"
            :disabled="!recoveryConfirmed"
            @click="nextStep"
          >
            验证已保存 →
          </button>
        </div>

        <!-- 步骤 3: 随机确认 3 个恢复词 -->
        <div v-else key="step3" class="step-content">
          <div class="wizard-icon icon-blue">✓</div>
          <h1 class="wizard-title">确认恢复词</h1>
          <p class="wizard-subtitle">请按编号输入你刚才保存的 3 个恢复词</p>

          <div class="confirmation-list">
            <label v-for="(wordIndex, inputIndex) in confirmationIndexes" :key="wordIndex">
              <span>第 {{ wordIndex + 1 }} 个词</span>
              <input
                v-model="confirmationInputs[inputIndex]"
                class="wizard-input"
                type="text"
                autocomplete="off"
                :spellcheck="false"
                placeholder="输入恢复词"
              />
            </label>
          </div>

          <Transition name="error-slide">
            <p v-if="error" class="wizard-error">{{ error }}</p>
          </Transition>

          <button class="wizard-btn" :disabled="!confirmationMatches || submitting" @click="finishSetup">
            <span v-if="submitting" class="btn-spinner"></span>
            <span v-else>完成并进入抽屉柜</span>
          </button>
        </div>
      </Transition>
    </div>
  </div>
</template>

<style scoped>
/* Phase 2C-2：初始化完成重启遮罩 */
.restart-overlay {
  position: absolute;
  inset: 0;
  z-index: 50;
  display: flex;
  align-items: center;
  justify-content: center;
  background: rgba(10, 14, 22, 0.92);
  backdrop-filter: blur(6px);
}
.restart-box { text-align: center; }
.restart-spinner {
  width: 34px;
  height: 34px;
  margin: 0 auto 14px;
  border: 3px solid rgba(59, 110, 245, 0.25);
  border-top-color: #3b6ef5;
  border-radius: 50%;
  animation: restart-spin 0.9s linear infinite;
}
@keyframes restart-spin { to { transform: rotate(360deg); } }
.restart-text { margin: 0; font-size: 13px; color: #cdd6e6; }
.wizard {
  height: 100%;
  display: flex;
  flex-direction: column;
  position: relative;
  overflow: hidden;
  background:
    radial-gradient(ellipse at 30% 30%, rgba(79, 124, 255, 0.15) 0%, transparent 50%),
    radial-gradient(ellipse at 70% 70%, rgba(255, 94, 126, 0.1) 0%, transparent 50%),
    var(--bg-deep);
}

.bg-decoration { position: absolute; inset: 0; pointer-events: none; overflow: hidden; }

.wizard-topbar {
  height: 32px;
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 0 12px;
  flex-shrink: 0;
  z-index: 10;
  -webkit-app-region: drag;
  app-region: drag;
}
.wizard-topbar-left {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 12px;
  color: var(--text-muted);
  user-select: none;
}
.wizard-topbar-logo {
  width: 16px;
  height: 16px;
  object-fit: contain;
  border-radius: 3px;
  flex-shrink: 0;
}
.wizard-topbar-title { font-weight: 600; }

.bg-orb {
  position: absolute;
  border-radius: 50%;
  filter: blur(60px);
  opacity: 0.4;
  animation: float 20s ease-in-out infinite;
}

.bg-orb-1 { width: 300px; height: 300px; background: var(--accent); top: -100px; left: -100px; }
.bg-orb-2 { width: 250px; height: 250px; background: var(--highlight); bottom: -80px; right: -80px; animation-delay: -10s; }

@keyframes float {
  0%, 100% { transform: translate(0, 0); }
  50% { transform: translate(40px, -30px); }
}

.wizard-card {
  width: 100%;
  max-width: 560px;
  padding: 32px 36px;
  background: var(--bg-glass);
  backdrop-filter: blur(30px) saturate(180%);
  -webkit-backdrop-filter: blur(30px) saturate(180%);
  border: 1px solid var(--border);
  border-radius: var(--radius-xl);
  box-shadow: var(--shadow-lg);
  position: relative;
  z-index: 1;
  max-height: 92%;
  overflow-y: auto;
}

/* 步骤指示器 */
.step-indicator {
  display: flex;
  align-items: center;
  margin-bottom: 28px;
  padding: 0 20px;
}

.step {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 6px;
  flex-shrink: 0;
  transition: all 0.3s;
}

.step-dot {
  width: 32px;
  height: 32px;
  border-radius: 50%;
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: 14px;
  font-weight: 600;
  background: var(--bg-tertiary);
  color: var(--text-muted);
  border: 2px solid var(--border);
  transition: all 0.3s var(--ease-spring);
}

.step.active .step-dot {
  background: linear-gradient(135deg, var(--accent) 0%, var(--highlight) 100%);
  color: white;
  border-color: var(--accent);
  box-shadow: 0 4px 12px rgba(79, 124, 255, 0.4);
}

.step.done .step-dot {
  background: var(--success);
  color: white;
  border-color: var(--success);
}

.step-label {
  font-size: 11px;
  color: var(--text-muted);
  white-space: nowrap;
  transition: color 0.3s;
}

.step.active .step-label { color: var(--text-primary); font-weight: 500; }

.step-line {
  flex: 1;
  height: 2px;
  background: var(--border);
  margin: 0 12px;
  margin-bottom: 22px;
  transition: background 0.4s;
}

.step-line.active { background: linear-gradient(90deg, var(--accent) 0%, var(--success) 100%); }

/* 步骤内容 */
.step-content {
  text-align: center;
}

.step-enter-active {
  animation: stepIn 0.35s var(--ease-out);
}

.step-leave-active {
  animation: stepOut 0.2s var(--ease-smooth);
  position: absolute;
  left: 36px;
  right: 36px;
}

@keyframes stepIn {
  from {
    opacity: 0;
    transform: translateX(20px);
  }
  to {
    opacity: 1;
    transform: translateX(0);
  }
}

@keyframes stepOut {
  to {
    opacity: 0;
    transform: translateX(-20px);
  }
}

.wizard-icon {
  width: 64px;
  height: 64px;
  margin: 0 auto 16px;
  border-radius: 50%;
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: 32px;
  box-shadow: 0 8px 20px rgba(0, 0, 0, 0.3);
}

.icon-blue { background: linear-gradient(135deg, var(--accent) 0%, #4fc3f7 100%); }
.icon-orange { background: linear-gradient(135deg, #ffb84d 0%, #ff8a5e 100%); }

.wizard-title {
  font-size: 22px;
  font-weight: 700;
  margin-bottom: 6px;
}

.wizard-subtitle {
  font-size: 13px;
  color: var(--text-muted);
  margin-bottom: 24px;
}

/* 表单 */
.form-group {
  text-align: left;
  margin-bottom: 18px;
}

.form-group label {
  display: block;
  font-size: 12px;
  color: var(--text-muted);
  margin-bottom: 6px;
  font-weight: 500;
}

.input-wrap {
  position: relative;
}

.wizard-input {
  width: 100%;
  background: rgba(0, 0, 0, 0.3);
  border: 1px solid var(--border);
  border-radius: var(--radius);
  padding: 12px 44px 12px 14px;
  color: var(--text-primary);
  font-size: 14px;
  outline: none;
  transition: all 0.2s;
}

.wizard-input:focus {
  border-color: var(--accent);
  background: rgba(0, 0, 0, 0.4);
  box-shadow: 0 0 0 3px var(--accent-soft);
}

.input-toggle {
  position: absolute;
  right: 8px;
  top: 50%;
  transform: translateY(-50%);
  background: transparent;
  border: none;
  cursor: pointer;
  font-size: 18px;
  padding: 4px 8px;
  opacity: 0.5;
  transition: opacity 0.2s;
}

.input-toggle:hover { opacity: 1; }

.form-hint {
  font-size: 11px;
  margin-top: 4px;
  padding-left: 4px;
}

.form-hint.error { color: var(--highlight); }
.form-hint.success { color: var(--success); }

/* 强度条 */
.strength-bar {
  display: flex;
  align-items: center;
  gap: 10px;
  margin-top: 8px;
}

.strength-track {
  flex: 1;
  height: 4px;
  background: var(--bg-tertiary);
  border-radius: 2px;
  overflow: hidden;
}

.strength-fill {
  height: 100%;
  border-radius: 2px;
  transition: width 0.3s var(--ease-out), background 0.3s;
  box-shadow: 0 0 8px currentColor;
}

.strength-label {
  font-size: 11px;
  font-weight: 600;
  min-width: 50px;
  text-align: right;
}

/* 错误 */
.wizard-error {
  display: inline-block;
  color: var(--highlight);
  font-size: 12px;
  background: var(--highlight-soft);
  padding: 6px 12px;
  border-radius: var(--radius-sm);
  margin: 8px 0 16px;
}

.error-slide-enter-active,
.error-slide-leave-active {
  transition: all 0.3s var(--ease-spring);
}
.error-slide-enter-from,
.error-slide-leave-to {
  opacity: 0;
  transform: translateY(-8px);
}

/* 恢复短语 */
.recovery-grid {
  display: grid;
  grid-template-columns: repeat(4, 1fr);
  gap: 8px;
  margin-bottom: 20px;
  text-align: left;
}

.recovery-word {
  background: var(--bg-tertiary);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  padding: 8px 10px;
  display: flex;
  align-items: center;
  gap: 6px;
  animation: wordIn 0.4s var(--ease-out) var(--delay) backwards;
}

@keyframes wordIn {
  from { opacity: 0; transform: scale(0.8); }
  to { opacity: 1; transform: scale(1); }
}

.recovery-num {
  font-size: 10px;
  color: var(--text-faint);
  font-family: monospace;
}

.recovery-text {
  font-size: 13px;
  font-weight: 500;
  color: var(--text-primary);
  font-family: 'Cascadia Code', 'Consolas', monospace;
}

.warning-box {
  display: flex;
  align-items: flex-start;
  gap: 10px;
  background: rgba(255, 184, 77, 0.1);
  border: 1px solid rgba(255, 184, 77, 0.3);
  border-radius: var(--radius);
  padding: 12px;
  margin-bottom: 16px;
  text-align: left;
}

.warning-icon { font-size: 18px; }

.warning-text {
  font-size: 12px;
  color: var(--text-secondary);
  line-height: 1.5;
}

.warning-text strong { color: var(--warning); }

.check-row {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 13px;
  color: var(--text-secondary);
  cursor: pointer;
  margin-bottom: 16px;
  justify-content: center;
}

.check-row input { cursor: pointer; accent-color: var(--accent); }

.confirmation-list {
  display: grid;
  gap: 14px;
  margin-bottom: 18px;
  text-align: left;
}

.confirmation-list label {
  display: grid;
  gap: 6px;
  color: var(--text-muted);
  font-size: 12px;
}

.wizard-actions {
  display: flex;
  gap: 10px;
}

.wizard-btn {
  flex: 1;
  background: linear-gradient(135deg, var(--accent) 0%, var(--highlight) 100%);
  color: white;
  border: none;
  border-radius: var(--radius);
  padding: 12px;
  font-size: 14px;
  font-weight: 600;
  cursor: pointer;
  box-shadow: 0 4px 12px rgba(79, 124, 255, 0.4);
  transition: all 0.25s var(--ease-smooth);
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 6px;
}

.wizard-btn:hover:not(:disabled) {
  transform: translateY(-1px);
  box-shadow: 0 6px 16px rgba(79, 124, 255, 0.5);
}

.wizard-btn:disabled { opacity: 0.4; cursor: not-allowed; }

.btn-secondary {
  background: var(--bg-tertiary);
  color: var(--text-secondary);
  box-shadow: none;
  border: 1px solid var(--border);
  flex: 0 0 auto;
  padding: 12px 20px;
}

.btn-secondary:hover:not(:disabled) {
  background: var(--bg-glass);
  box-shadow: none;
}

.btn-spinner {
  width: 14px;
  height: 14px;
  border: 2px solid rgba(255, 255, 255, 0.3);
  border-top-color: white;
  border-radius: 50%;
  animation: spin 0.6s linear infinite;
}
</style>

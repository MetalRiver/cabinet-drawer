<script setup lang="ts">
import { ref, onMounted, onUnmounted } from "vue";
import { useAppStore } from "../stores/app";
import {
  unlockApp,
  lockApp,
  hideWindow,
  getSecurityStatus,
  verifyV2RecoveryPhrase,
  recoverV2WithPhrase,
} from "../api";
import { listen } from "@tauri-apps/api/event";
import { useWindowDrag } from "../composables/useWindowDrag";
// 品牌 Logo
import logo from "@/assets/logo.png";

const appStore = useAppStore();
const password = ref("");
const error = ref("");
const isLoading = ref(false);
const isUnlocked = ref(false);
const attempts = ref(0);
// 修复 P1-#16：恢复短语放到组件本地 ref，不再进全局 store
const recoveryPhrase = ref<string[]>([]);
type LockMode = "master" | "recovery-phrase" | "new-master";
const mode = ref<LockMode>("master");
const recoveryInput = ref("");
const newMasterPassword = ref("");
const confirmMasterPassword = ref("");
const v2RecoveryAvailable = ref(false);

// 拖动支持：登录页也要能拖动窗口
const { onDragHandleMouseDown } = useWindowDrag();

// 监听外部锁定事件
let unlisten: (() => void) | null = null;

onMounted(async () => {
  unlisten = await listen("app:lock", () => {
    lockApp();
    appStore.lock();
    password.value = "";
    isUnlocked.value = false;
    clearRecoverySecrets();
  });
  try {
    const status = await getSecurityStatus();
    v2RecoveryAvailable.value = status.security_model === "stable_dek_v2";
  } catch {
    // fail closed：IPC 异常时不开放 Recovery 入口。
    v2RecoveryAvailable.value = false;
  }
});

onUnmounted(() => {
  unlisten?.();
  password.value = "";
  clearRecoverySecrets();
});

function clearRecoverySecrets() {
  recoveryPhrase.value = [];
  recoveryInput.value = "";
  newMasterPassword.value = "";
  confirmMasterPassword.value = "";
  mode.value = "master";
}

function normalizedRecoveryPhrase() {
  return recoveryInput.value.trim().toLowerCase().split(/\s+/).filter(Boolean).join(" ");
}

function beginRecovery() {
  error.value = "";
  if (!v2RecoveryAvailable.value) {
    error.value = "旧版密码库暂不支持恢复短语";
    return;
  }
  password.value = "";
  mode.value = "recovery-phrase";
}

function cancelRecovery() {
  clearRecoverySecrets();
  error.value = "";
  isLoading.value = false;
}

async function handleVerifyRecovery() {
  const phrase = normalizedRecoveryPhrase();
  if (phrase.split(" ").length !== 12) {
    error.value = "请输入 12 个恢复词";
    return;
  }
  isLoading.value = true;
  error.value = "";
  try {
    await verifyV2RecoveryPhrase(phrase);
    recoveryInput.value = phrase;
    mode.value = "new-master";
  } catch {
    error.value = "恢复短语无效";
  } finally {
    isLoading.value = false;
  }
}

async function handleRecoveryComplete() {
  if (newMasterPassword.value.length < 6) {
    error.value = "新主密码长度至少 6 位";
    return;
  }
  if (newMasterPassword.value !== confirmMasterPassword.value) {
    error.value = "两次输入的主密码不一致";
    return;
  }
  isLoading.value = true;
  error.value = "";
  try {
    await recoverV2WithPhrase(normalizedRecoveryPhrase(), newMasterPassword.value);
    isUnlocked.value = true;
    clearRecoverySecrets();
    password.value = "";
    attempts.value = 0;
    setTimeout(() => appStore.unlock(), 200);
  } catch (e) {
    error.value = String(e);
    isLoading.value = false;
  }
}

async function handleUnlock() {
  if (!password.value) {
    error.value = "请输入主密码";
    return;
  }
  isLoading.value = true;
  error.value = "";
  try {
    const recovery = await unlockApp(password.value);
    // 修复 P1-#16：恢复短语仅放本地 ref，不入 store
    recoveryPhrase.value = recovery;
    isUnlocked.value = true;
    setTimeout(() => {
      appStore.unlock();
      password.value = "";
      attempts.value = 0;
    }, 200);
  } catch (e) {
    error.value = String(e);
    attempts.value++;
    isLoading.value = false;
  }
}

// 退出登录界面：直接隐藏抽屉柜
async function handleClose() {
  password.value = "";
  error.value = "";
  clearRecoverySecrets();
  await hideWindow();
}
</script>

<template>
  <div class="lock-screen">
    <!-- 顶部可拖动条 -->
    <div class="lock-topbar" data-tauri-drag-region @mousedown="onDragHandleMouseDown">
      <div class="lock-topbar-left" data-tauri-drag-region>
        <img :src="logo" alt="抽屉柜 Drawer" class="lock-topbar-logo" />
        <span class="lock-topbar-title">抽屉柜 Drawer</span>
      </div>
      <div class="lock-topbar-right" data-no-drag>
        <button class="lock-topbar-btn" title="隐藏 (Alt+Q)" @click="handleClose">✕</button>
      </div>
    </div>

    <div class="bg-decoration">
      <div class="bg-orb bg-orb-1"></div>
      <div class="bg-orb bg-orb-2"></div>
      <div class="bg-orb bg-orb-3"></div>
    </div>

    <div class="lock-content">

    <div class="lock-card" :class="{ success: isUnlocked }">
      <!-- ===== 品牌区：产品 Logo + 名称（80px合适大小，不溢出）===== -->
      <div class="lock-brand">
        <img :src="logo" alt="抽屉柜 Drawer" class="lock-brand-logo" />
        <h1 class="lock-brand-title">抽屉柜 Drawer</h1>
      </div>

      <div class="lock-icon">
        <Transition name="lock-icon" mode="out-in">
          <span v-if="isUnlocked" key="open">🔓</span>
          <span v-else key="close">🔒</span>
        </Transition>
      </div>

      <h2 class="lock-title">
        {{ mode === "master" ? "请输入主密码以解锁" : mode === "recovery-phrase" ? "输入恢复短语" : "设置新主密码" }}
      </h2>
      <p class="lock-subtitle">
        {{ mode === "master" ? "Drawer · 您的私密桌面收纳箱" : mode === "recovery-phrase" ? "请输入保存的 12 个恢复词" : "恢复后将自动进入应用" }}
      </p>

      <div v-if="mode === 'master'" class="lock-input-wrap">
        <input
          v-model="password"
          type="password"
          class="lock-input"
          placeholder="••••••••"
          :disabled="isLoading || isUnlocked"
          @keyup.enter="handleUnlock"
        />
        <div v-if="isLoading" class="lock-spinner"></div>
      </div>

      <div v-else-if="mode === 'recovery-phrase'" class="lock-input-wrap">
        <textarea
          v-model="recoveryInput"
          class="recovery-input"
          rows="4"
          autocomplete="off"
          autocapitalize="none"
          spellcheck="false"
          placeholder="输入 12 个恢复词，以空格分隔"
          :disabled="isLoading"
        ></textarea>
      </div>

      <div v-else class="new-master-fields">
        <input
          v-model="newMasterPassword"
          type="password"
          class="lock-input"
          autocomplete="new-password"
          placeholder="新主密码（至少 6 位）"
          :disabled="isLoading"
        />
        <input
          v-model="confirmMasterPassword"
          type="password"
          class="lock-input"
          autocomplete="new-password"
          placeholder="再次输入新主密码"
          :disabled="isLoading"
          @keyup.enter="handleRecoveryComplete"
        />
      </div>

      <Transition name="error-slide">
        <p v-if="error" class="lock-error">
          <span>⚠️</span>
          {{ error }}
        </p>
      </Transition>

      <button
        v-if="mode === 'master'"
        class="lock-btn"
        :disabled="isLoading || isUnlocked"
        @click="handleUnlock"
      >
        <span v-if="isLoading" class="btn-spinner"></span>
        <span v-else-if="isUnlocked" class="btn-success">✓ 已解锁</span>
        <span v-else>解锁</span>
      </button>

      <button
        v-else-if="mode === 'recovery-phrase'"
        class="lock-btn"
        :disabled="isLoading"
        @click="handleVerifyRecovery"
      >
        <span v-if="isLoading" class="btn-spinner"></span>
        <span v-else>验证恢复短语</span>
      </button>

      <button
        v-else
        class="lock-btn"
        :disabled="isLoading || isUnlocked"
        @click="handleRecoveryComplete"
      >
        <span v-if="isLoading" class="btn-spinner"></span>
        <span v-else-if="isUnlocked" class="btn-success">✓ 已恢复</span>
        <span v-else>设置新主密码并进入</span>
      </button>

      <div v-if="mode === 'master'" class="lock-links">
        <button
          v-if="v2RecoveryAvailable"
          type="button"
          class="lock-link link-button"
          @click="beginRecovery"
        >忘记主密码？</button>
        <span v-else class="legacy-recovery-note">旧版密码库暂不支持恢复</span>
      </div>
      <div v-else class="lock-links">
        <button type="button" class="lock-link link-button" :disabled="isLoading" @click="cancelRecovery">
          返回主密码解锁
        </button>
      </div>
    </div>
    </div>
  </div>
</template>

<style scoped>
.lock-screen {
  height: 100%;
  display: flex;
  flex-direction: column;
  position: relative;
  overflow: hidden;
  background:
    radial-gradient(ellipse at 30% 20%, rgba(79, 124, 255, 0.15) 0%, transparent 50%),
    radial-gradient(ellipse at 70% 80%, rgba(255, 94, 126, 0.1) 0%, transparent 50%),
    var(--bg-deep);
}

.lock-topbar {
  height: 32px;
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 0 8px 0 12px;
  flex-shrink: 0;
  z-index: 10;
  /* 关键：让 Tauri 把整个区域识别为拖动区 */
  -webkit-app-region: drag;
  app-region: drag;
}
.lock-topbar-left {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 12px;
  color: var(--text-muted);
  user-select: none;
}
.lock-topbar-logo {
  /* 左上角小图标：16→18px，不再像文件夹emoji */
  width: 18px;
  height: 18px;
  object-fit: contain;
  border-radius: 4px;
  flex-shrink: 0;
}
.lock-topbar-title { font-weight: 600; }
.lock-topbar-right {
  display: flex;
  align-items: center;
  gap: 2px;
  -webkit-app-region: no-drag;
  app-region: no-drag;
}
.lock-topbar-btn {
  width: 24px;
  height: 24px;
  border: none;
  background: transparent;
  color: var(--text-muted);
  font-size: 14px;
  border-radius: 4px;
  cursor: pointer;
  display: flex;
  align-items: center;
  justify-content: center;
  transition: all 0.15s;
}
.lock-topbar-btn:hover { background: var(--accent-soft); color: var(--text-primary); }
.lock-topbar-btn.close:hover { background: var(--danger); color: white; }

.lock-content {
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
  position: relative;
  min-height: 0;
}

.bg-decoration {
  position: absolute;
  inset: 0;
  pointer-events: none;
  overflow: hidden;
}

.bg-orb {
  position: absolute;
  border-radius: 50%;
  filter: blur(60px);
  opacity: 0.4;
  animation: float 20s ease-in-out infinite;
}

.bg-orb-1 { width: 300px; height: 300px; background: var(--accent); top: -100px; left: -100px; }
.bg-orb-2 { width: 250px; height: 250px; background: var(--highlight); bottom: -80px; right: -80px; animation-delay: -7s; }
.bg-orb-3 { width: 200px; height: 200px; background: #4fc3f7; top: 50%; left: 50%; animation-delay: -14s; }

@keyframes float {
  0%, 100% { transform: translate(0, 0) scale(1); }
  33% { transform: translate(50px, -30px) scale(1.1); }
  66% { transform: translate(-30px, 40px) scale(0.9); }
}

.bg-orb-3 { animation-name: floatCenter; }

@keyframes floatCenter {
  0%, 100% { transform: translate(-50%, -50%) scale(1); }
  50% { transform: translate(-50%, -50%) scale(1.2); }
}

.lock-card {
  width: 100%;
  max-width: 360px; /* 卡片稍微窄一点，更精致 */
  padding: 24px 28px 28px;
  background: var(--bg-glass);
  backdrop-filter: blur(16px) saturate(140%);
  -webkit-backdrop-filter: blur(16px) saturate(140%);
  border: 1px solid var(--border-strong);
  border-radius: var(--radius-xl);
  text-align: center;
  box-shadow:
    var(--shadow-lg),
    inset 0 1px 0 rgba(255, 255, 255, 0.08);
  position: relative;
  z-index: 1;
  transition: all 0.4s var(--ease-spring);
  animation: cardIn 0.6s var(--ease-spring);
}

.lock-card.success {
  transform: scale(0.95);
  opacity: 0;
}

@keyframes cardIn {
  from { opacity: 0; transform: translateY(20px) scale(0.95); }
  to { opacity: 1; transform: translateY(0) scale(1); }
}

/* ===== 品牌区（卡片顶部：产品 Logo + 名称）===== */
.lock-brand {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 8px;
  margin-bottom: 16px;
}
.lock-brand-logo {
  /* 80→64px：合适大小，不抢戏；圆角18→24px：白底不突兀 */
  width: 64px;
  height: 64px;
  object-fit: contain;
  border-radius: 24px;
  box-shadow:
    0 8px 24px rgba(79, 124, 255, 0.32),
    0 0 0 1px rgba(255, 255, 255, 0.08);
  flex-shrink: 0;
}
.lock-brand-title {
  margin: 0;
  /* 20→18px：精致一点 */
  font-size: 18px;
  font-weight: 700;
  background: linear-gradient(135deg, #fff 0%, #b8c1d9 100%);
  -webkit-background-clip: text;
  background-clip: text;
  -webkit-text-fill-color: transparent;
  letter-spacing: 0.5px;
}

.lock-icon {
  /* 64→56px：与品牌Logo比例协调 */
  width: 56px;
  height: 56px;
  margin: 0 auto 16px;
  background: linear-gradient(135deg, var(--accent) 0%, var(--highlight) 100%);
  border-radius: 50%;
  display: flex;
  align-items: center;
  justify-content: center;
  /* 30→26px：对应缩小 */
  font-size: 26px;
  box-shadow: 0 8px 24px rgba(79, 124, 255, 0.4);
  position: relative;
  animation: iconFloat 3s ease-in-out infinite;
}

.lock-icon::after {
  content: "";
  position: absolute;
  inset: -6px;
  border-radius: 50%;
  background: linear-gradient(135deg, var(--accent) 0%, var(--highlight) 100%);
  filter: blur(16px);
  opacity: 0.3;
  z-index: -1;
}

@keyframes iconFloat {
  0%, 100% { transform: translateY(0); }
  50% { transform: translateY(-4px); }
}

.lock-icon-enter-active,
.lock-icon-leave-active { transition: all 0.3s var(--ease-spring); }
.lock-icon-enter-from { opacity: 0; transform: scale(0.5) rotate(-90deg); }
.lock-icon-leave-to { opacity: 0; transform: scale(1.3) rotate(90deg); }

.lock-title {
  /* 现在是小标题：「请输入主密码以解锁」，不要渐变，普通清晰文字即可 */
  font-size: 15px;
  font-weight: 600;
  color: var(--text-primary);
  margin-bottom: 4px;
  background: none;
  -webkit-background-clip: unset;
  background-clip: unset;
  -webkit-text-fill-color: unset;
}

.lock-subtitle {
  /* 12.5→11.5px：与整体比例协调 */
  font-size: 11.5px;
  color: var(--text-muted);
  margin-bottom: 20px;
  margin-top: 0;
  letter-spacing: 0.2px;
}

.lock-input-wrap { position: relative; margin-bottom: 8px; }

.lock-input {
  width: 100%;
  background: rgba(0, 0, 0, 0.3);
  border: 1px solid var(--border);
  border-radius: var(--radius);
  /* 14px 18px → 12px 16px：缩小更精致 */
  padding: 12px 16px;
  color: var(--text-primary);
  /* 16px → 14px：对应整体缩小 */
  font-size: 14px;
  text-align: center;
  outline: none;
  letter-spacing: 4px;
  transition: all 0.2s;
}

.lock-input:focus {
  border-color: var(--accent);
  background: rgba(0, 0, 0, 0.4);
  box-shadow: 0 0 0 3px var(--accent-soft);
}

.lock-input:disabled { opacity: 0.5; cursor: not-allowed; }

.recovery-input {
  width: 100%;
  resize: none;
  background: rgba(0, 0, 0, 0.3);
  border: 1px solid var(--border);
  border-radius: var(--radius);
  padding: 12px 14px;
  color: var(--text-primary);
  font-size: 13px;
  line-height: 1.7;
  outline: none;
  transition: all 0.2s;
}

.recovery-input:focus {
  border-color: var(--accent);
  background: rgba(0, 0, 0, 0.4);
  box-shadow: 0 0 0 3px var(--accent-soft);
}

.recovery-input:disabled { opacity: 0.5; cursor: not-allowed; }

.new-master-fields {
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.new-master-fields .lock-input {
  letter-spacing: 1px;
}

.lock-spinner {
  position: absolute;
  right: 14px;
  top: 50%;
  transform: translateY(-50%);
  width: 16px;
  height: 16px;
  border: 2px solid var(--border);
  border-top-color: var(--accent);
  border-radius: 50%;
  animation: spin 0.6s linear infinite;
}

.lock-error {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 6px;
  color: var(--highlight);
  font-size: 12px;
  margin: 8px 0;
  padding: 8px 12px;
  background: var(--highlight-soft);
  border-radius: var(--radius-sm);
}

.error-slide-enter-active,
.error-slide-leave-active { transition: all 0.3s var(--ease-spring); }
.error-slide-enter-from,
.error-slide-leave-to { opacity: 0; transform: translateY(-8px); }

.lock-btn {
  width: 100%;
  background: linear-gradient(135deg, var(--accent) 0%, var(--highlight) 100%);
  color: white;
  border: none;
  border-radius: var(--radius);
  /* 14px → 12px：缩小更精致 */
  padding: 12px;
  font-size: 14px;
  font-weight: 600;
  cursor: pointer;
  /* 16px → 12px：更紧凑 */
  margin-top: 12px;
  position: relative;
  overflow: hidden;
  box-shadow: 0 4px 16px rgba(79, 124, 255, 0.4);
  transition: all 0.25s var(--ease-smooth);
  display: flex;
  align-items: center;
  justify-content: center;
  /* 46px → 42px：对应整体缩小 */
  min-height: 42px;
}

.lock-btn:hover:not(:disabled) { transform: translateY(-2px); box-shadow: 0 6px 20px rgba(79, 124, 255, 0.5); }
.lock-btn:active:not(:disabled) { transform: translateY(0); }
.lock-btn:disabled { cursor: default; }

.btn-spinner {
  width: 16px;
  height: 16px;
  border: 2px solid rgba(255, 255, 255, 0.3);
  border-top-color: white;
  border-radius: 50%;
  animation: spin 0.6s linear infinite;
}

.btn-success {
  display: flex;
  align-items: center;
  gap: 6px;
  animation: successPop 0.4s var(--ease-spring);
}

@keyframes successPop {
  0% { transform: scale(0); }
  60% { transform: scale(1.2); }
  100% { transform: scale(1); }
}

.lock-links {
  margin-top: 24px;
  display: flex;
  justify-content: center;
  align-items: center;
  gap: 10px;
  font-size: 12px;
}

.lock-link {
  color: var(--text-muted);
  cursor: pointer;
  transition: color 0.2s;
}

.lock-link:hover { color: var(--accent-bright); }
.link-button {
  padding: 0;
  border: 0;
  background: transparent;
  font: inherit;
}
.link-button:disabled { opacity: 0.5; cursor: default; }
.legacy-recovery-note { color: var(--text-faint); }
.lock-divider { color: var(--text-faint); }
</style>

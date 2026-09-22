<script setup lang="ts">
import { ref, onMounted, onUnmounted, computed } from "vue";
import { useAppStore } from "../stores/app";
import {
  prepareLegacyMigration,
  confirmLegacyMigration,
  cancelLegacyMigration,
  getLegacyMigrationStatus,
} from "../api";
import { useWindowDrag } from "../composables/useWindowDrag";
import logo from "@/assets/logo.png";

// ============================================================
// 🔒 两阶段 legacy 安全升级流程（0.3.0）
// Step 1 输入当前主密码 → Step 2 保存 12 个恢复词 → Step 3 确认 3 个随机词
// 恢复短语只保存在组件本地 ref（不入 Pinia / localStorage），
// 组件卸载即清空；确认词由后端验证，前端不做任何判定。
// ============================================================

const appStore = useAppStore();
const { onDragHandleMouseDown } = useWindowDrag();

type MigrationStep = "password" | "words" | "confirm";
const step = ref<MigrationStep>("password");
const isLoading = ref(false);
const error = ref("");

const password = ref("");
const migrationToken = ref("");
const recoveryWords = ref<string[]>([]);
const confirmationIndexes = ref<number[]>([]);
const confirmInputs = ref<string[]>(["", "", ""]);

const wordColumns = computed(() => {
  // 12 词按 3 列 x 4 行展示，序号与后端 confirmation_indexes 的语义一致（0 起）
  const columns: { index: number; word: string }[][] = [[], [], []];
  recoveryWords.value.forEach((word, index) => {
    columns[index % 3].push({ index, word });
  });
  return columns;
});

onMounted(async () => {
  // 组件重新挂载（例如从托盘重新打开窗口）时，若后端仍有进行中的升级：
  // 恢复短语已随上次组件卸载销毁，无法继续确认——安全取消后重新开始。
  try {
    const status = await getLegacyMigrationStatus();
    if (status.active && status.migration_token) {
      await cancelLegacyMigration(status.migration_token);
    }
  } catch {
    // 查询失败不影响开始新流程（prepare 侧有自己的并发闸）
  }
});

onUnmounted(() => {
  clearSecrets();
});

function clearSecrets() {
  password.value = "";
  recoveryWords.value = [];
  confirmInputs.value = ["", "", ""];
}

function resetToPassword() {
  migrationToken.value = "";
  confirmationIndexes.value = [];
  clearSecrets();
  step.value = "password";
}

async function handleCancel() {
  if (isLoading.value) return;
  error.value = "";
  const token = migrationToken.value;
  isLoading.value = true;
  try {
    if (token) {
      await cancelLegacyMigration(token);
    }
  } catch {
    // 取消失败也回到第一步重新发起（后端孤儿 pending 由下次启动清理兜底）
  } finally {
    isLoading.value = false;
    resetToPassword();
  }
}

async function handlePrepare() {
  if (!password.value) {
    error.value = "请输入当前主密码";
    return;
  }
  isLoading.value = true;
  error.value = "";
  try {
    const prepared = await prepareLegacyMigration(password.value);
    migrationToken.value = prepared.migration_token;
    recoveryWords.value = prepared.recovery_words;
    confirmationIndexes.value = prepared.confirmation_indexes;
    confirmInputs.value = ["", "", ""];
    password.value = "";
    step.value = "words";
  } catch (e) {
    error.value = String(e);
  } finally {
    isLoading.value = false;
  }
}

function ordinal(index: number) {
  return `第 ${index + 1} 个词`;
}

async function handleConfirm() {
  const words = confirmInputs.value.map((word) => word.trim().toLowerCase());
  if (words.some((word) => !word)) {
    error.value = "请填写全部 3 个确认词";
    return;
  }
  isLoading.value = true;
  error.value = "";
  try {
    await confirmLegacyMigration(migrationToken.value, words);
    // 升级完成：后端已激活 Stable DEK，本地的短语痕迹即刻清除
    clearSecrets();
    recoveryWords.value = [];
    migrationToken.value = "";
    confirmationIndexes.value = [];
    step.value = "password";
    appStore.completeMigration();
    appStore.unlock();
  } catch (e) {
    error.value = String(e);
    // 源数据变化等 fail closed 场景：后端已清 pending，回到第一步重新开始
    if (String(e).includes("重新开始升级")) {
      resetToPassword();
    }
  } finally {
    isLoading.value = false;
  }
}
</script>

<template>
  <div class="migration-screen">
    <div class="migration-topbar" data-tauri-drag-region @mousedown="onDragHandleMouseDown">
      <div class="migration-topbar-left" data-tauri-drag-region>
        <img :src="logo" alt="抽屉柜 Drawer" class="migration-topbar-logo" />
        <span class="migration-topbar-title">抽屉柜 Drawer</span>
      </div>
    </div>

    <div class="bg-decoration">
      <div class="bg-orb bg-orb-1"></div>
      <div class="bg-orb bg-orb-2"></div>
      <div class="bg-orb bg-orb-3"></div>
    </div>

    <div class="migration-content">
      <div class="migration-card">
        <div class="migration-brand">
          <img :src="logo" alt="抽屉柜 Drawer" class="migration-brand-logo" />
          <h1 class="migration-brand-title">抽屉柜 Drawer</h1>
        </div>

        <!-- ===== Step 1：输入当前主密码 ===== -->
        <template v-if="step === 'password'">
          <h2 class="migration-title">升级密码安全架构</h2>
          <p class="migration-subtitle">你的现有数据会原样保留，升级后需要保存一组恢复短语。</p>
          <div class="migration-input-wrap">
            <input
              v-model="password"
              type="password"
              class="migration-input"
              placeholder="当前主密码"
              :disabled="isLoading"
              @keyup.enter="handlePrepare"
            />
            <div v-if="isLoading" class="migration-spinner"></div>
          </div>
          <Transition name="error-slide">
            <p v-if="error" class="migration-error">
              <span>⚠️</span>
              {{ error }}
            </p>
          </Transition>
          <button class="migration-btn" :disabled="isLoading" @click="handlePrepare">
            <span v-if="isLoading" class="btn-spinner"></span>
            <span v-else>开始升级</span>
          </button>
          <p class="migration-note">升级完成后将使用全新的加密保护，此过程无法跳过。</p>
        </template>

        <!-- ===== Step 2：保存 12 个恢复词 ===== -->
        <template v-else-if="step === 'words'">
          <h2 class="migration-title">保存恢复短语</h2>
          <p class="migration-subtitle">
            请把下面 12 个词按顺序抄写到安全的地方（纸质保存，不要截图或存云端）。
          </p>
          <div class="words-grid">
            <div v-for="(column, c) in wordColumns" :key="c" class="words-column">
              <div v-for="item in column" :key="item.index" class="word-chip">
                <span class="word-index">{{ item.index + 1 }}</span>
                <span class="word-text">{{ item.word }}</span>
              </div>
            </div>
          </div>
          <Transition name="error-slide">
            <p v-if="error" class="migration-error">
              <span>⚠️</span>
              {{ error }}
            </p>
          </Transition>
          <button class="migration-btn" :disabled="isLoading" @click="step = 'confirm'; error = ''">
            <span>我已保存</span>
          </button>
          <button class="migration-btn ghost" :disabled="isLoading" @click="handleCancel">
            取消升级
          </button>
        </template>

        <!-- ===== Step 3：确认 3 个随机位置的词 ===== -->
        <template v-else>
          <h2 class="migration-title">确认恢复短语</h2>
          <p class="migration-subtitle">请输入刚才保存的恢复短语中指定位置的单词。</p>
          <div class="confirm-fields">
            <div v-for="(index, slot) in confirmationIndexes" :key="index" class="confirm-field">
              <label :for="`confirm-word-${slot}`" class="confirm-label">{{ ordinal(index) }}</label>
              <input
                :id="`confirm-word-${slot}`"
                v-model="confirmInputs[slot]"
                type="text"
                class="migration-input word-input"
                autocomplete="off"
                autocapitalize="none"
                spellcheck="false"
                placeholder="单词"
                :disabled="isLoading"
              />
            </div>
          </div>
          <Transition name="error-slide">
            <p v-if="error" class="migration-error">
              <span>⚠️</span>
              {{ error }}
            </p>
          </Transition>
          <button class="migration-btn" :disabled="isLoading" @click="handleConfirm">
            <span v-if="isLoading" class="btn-spinner"></span>
            <span v-else>完成升级</span>
          </button>
          <button class="migration-btn ghost" :disabled="isLoading" @click="handleCancel">
            取消升级
          </button>
        </template>
      </div>
    </div>
  </div>
</template>

<style scoped>
.migration-screen {
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

.migration-topbar {
  height: 32px;
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 0 8px 0 12px;
  flex-shrink: 0;
  z-index: 10;
  -webkit-app-region: drag;
  app-region: drag;
}
.migration-topbar-left {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 12px;
  color: var(--text-muted);
  user-select: none;
}
.migration-topbar-logo {
  width: 18px;
  height: 18px;
  object-fit: contain;
  border-radius: 4px;
  flex-shrink: 0;
}
.migration-topbar-title { font-weight: 600; }

.migration-content {
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
  position: relative;
  min-height: 0;
  padding: 0 20px 20px;
  overflow-y: auto;
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

.migration-card {
  width: 100%;
  max-width: 400px;
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
  animation: cardIn 0.5s var(--ease-spring);
  margin: auto;
}

@keyframes cardIn {
  from { opacity: 0; transform: translateY(20px) scale(0.95); }
  to { opacity: 1; transform: translateY(0) scale(1); }
}

.migration-brand {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 8px;
  margin-bottom: 16px;
}
.migration-brand-logo {
  width: 56px;
  height: 56px;
  object-fit: contain;
  border-radius: 20px;
  box-shadow:
    0 8px 24px rgba(79, 124, 255, 0.32),
    0 0 0 1px rgba(255, 255, 255, 0.08);
  flex-shrink: 0;
}
.migration-brand-title {
  margin: 0;
  font-size: 17px;
  font-weight: 700;
  background: linear-gradient(135deg, #fff 0%, #b8c1d9 100%);
  -webkit-background-clip: text;
  background-clip: text;
  -webkit-text-fill-color: transparent;
  letter-spacing: 0.5px;
}

.migration-title {
  font-size: 15px;
  font-weight: 600;
  color: var(--text-primary);
  margin: 0 0 4px;
}

.migration-subtitle {
  font-size: 11.5px;
  color: var(--text-muted);
  margin: 0 0 18px;
  letter-spacing: 0.2px;
  line-height: 1.6;
}

.migration-input-wrap { position: relative; margin-bottom: 8px; }

.migration-input {
  width: 100%;
  background: rgba(0, 0, 0, 0.3);
  border: 1px solid var(--border);
  border-radius: var(--radius);
  padding: 12px 16px;
  color: var(--text-primary);
  font-size: 14px;
  text-align: center;
  outline: none;
  letter-spacing: 2px;
  transition: all 0.2s;
  box-sizing: border-box;
}

.migration-input:focus {
  border-color: var(--accent);
  background: rgba(0, 0, 0, 0.4);
  box-shadow: 0 0 0 3px var(--accent-soft);
}

.migration-input:disabled { opacity: 0.5; cursor: not-allowed; }

.words-grid {
  display: flex;
  gap: 10px;
  justify-content: center;
  margin-bottom: 14px;
}

.words-column {
  display: flex;
  flex-direction: column;
  gap: 6px;
  flex: 1;
}

.word-chip {
  display: flex;
  align-items: center;
  gap: 8px;
  background: rgba(0, 0, 0, 0.3);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  padding: 7px 10px;
}

.word-index {
  font-size: 10px;
  color: var(--text-faint);
  min-width: 16px;
  text-align: right;
  font-variant-numeric: tabular-nums;
}

.word-text {
  font-size: 13px;
  color: var(--text-primary);
  font-weight: 600;
  letter-spacing: 0.3px;
  user-select: all;
}

.confirm-fields {
  display: flex;
  flex-direction: column;
  gap: 10px;
  margin-bottom: 6px;
}

.confirm-field {
  display: flex;
  align-items: center;
  gap: 10px;
}

.confirm-label {
  font-size: 12px;
  color: var(--text-muted);
  min-width: 64px;
  text-align: right;
}

.word-input {
  letter-spacing: 1px;
  text-align: left;
}

.migration-spinner {
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

@keyframes spin {
  to { transform: translateY(-50%) rotate(360deg); }
}

.migration-error {
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
  word-break: break-all;
}

.error-slide-enter-active,
.error-slide-leave-active { transition: all 0.3s var(--ease-spring); }
.error-slide-enter-from,
.error-slide-leave-to { opacity: 0; transform: translateY(-8px); }

.migration-btn {
  width: 100%;
  background: linear-gradient(135deg, var(--accent) 0%, var(--highlight) 100%);
  color: white;
  border: none;
  border-radius: var(--radius);
  padding: 12px;
  font-size: 14px;
  font-weight: 600;
  cursor: pointer;
  margin-top: 12px;
  position: relative;
  overflow: hidden;
  box-shadow: 0 4px 16px rgba(79, 124, 255, 0.4);
  transition: all 0.25s var(--ease-smooth);
  display: flex;
  align-items: center;
  justify-content: center;
  min-height: 42px;
}

.migration-btn:hover:not(:disabled) { transform: translateY(-2px); box-shadow: 0 6px 20px rgba(79, 124, 255, 0.5); }
.migration-btn:active:not(:disabled) { transform: translateY(0); }
.migration-btn:disabled { cursor: default; opacity: 0.7; }

.migration-btn.ghost {
  background: transparent;
  border: 1px solid var(--border);
  color: var(--text-muted);
  box-shadow: none;
}

.migration-btn.ghost:hover:not(:disabled) {
  color: var(--text-primary);
  border-color: var(--border-strong);
  transform: none;
}

.btn-spinner {
  width: 16px;
  height: 16px;
  border: 2px solid rgba(255, 255, 255, 0.3);
  border-top-color: white;
  border-radius: 50%;
  animation: spinFlat 0.6s linear infinite;
}

@keyframes spinFlat {
  to { transform: rotate(360deg); }
}

.migration-note {
  font-size: 11px;
  color: var(--text-faint);
  margin: 14px 0 0;
  letter-spacing: 0.2px;
}
</style>

<script setup lang="ts">
import { ref, computed, onMounted, reactive } from "vue";
import { useAppStore } from "../stores/app";
import { useWidgetStore } from "../stores/widget";
import { usePasswordStore } from "../stores/passwords";
import {
  copyToClipboardWithTimeout,
  generateRandomPassword,
  passwordStrength as apiStrength,
  verifyPasswordForPwView,
  hasSecondPassword as apiHasSecondPassword,
} from "../api";
import type { PasswordMeta } from "../api";
import { useClipboardCountdown } from "../composables/useClipboardCountdown";
import ConfirmInline from "../components/ConfirmInline.vue";

const appStore = useAppStore();
const widgetStore = useWidgetStore();
const passwordStore = usePasswordStore();
const clipboardCountdown = useClipboardCountdown();

// P0-#LOCK#UI#BUTTON：识别"应用已锁定"错误
// 之前：error 文本是后端直接返回的"应用已锁定: id=1"或"应用已锁定"两种
// 之前不知道错误类型 → 一律显示"重试"按钮 → 用户点了没用
// 现在：isLockedError 判定是否锁定相关错误 → 切换到"去解锁"按钮
const isLockedError = computed(() => {
  const e = passwordStore.error || "";
  return e.includes("应用已锁定") || e.includes("locked") || e.includes("Not unlocked");
});
// P0-#LOCK#UI#BUTTON：去解锁按钮 → 直接跳锁屏页（用户最短恢复路径）
function goUnlock() {
  appStore.lock();
}

// ===== 行内状态 =====
type RowMode = "collapsed" | "expanded" | "editing" | "deleting";

interface RowState {
  mode: RowMode;
  decryptedPassword: string;
  /** 解密失败标记：失败时绝不把提示文案写入 decryptedPassword（P0-B 哨兵清除） */
  decryptFailed: boolean;
  showPassword: boolean;
  isLoading: boolean;
  // 编辑表单
  form: {
    title: string;
    username: string;
    password: string;
    url: string;
    notes: string;
  };
  strength: number;
  strengthLabel: string;
  strengthColor: string;
}

const rowStates = reactive<Record<number, RowState>>({});

// ===== P0-#PW#PER#ITEM#VERIFY / P0-#PW#2ND：单条目二次验证（显示密码/复制密码前必输）=====
// 每次点击「👁️ 显示」或「📋 复制密码」都强制校验，不缓存状态（下次再点再输）
// hasIndependentSecondPassword：true=用户已经启用独立二次验证密码，弹窗文案显示"🔐二次验证密码"；false=用主密码 🔒
type VerifyAction = "reveal" | "copy_pw";
const verifyModal = reactive({
  open: false,
  loading: false,
  password: "",
  error: "",
  pendingAction: null as VerifyAction | null,
  pendingItemId: null as number | null,
});
// P0-#PW#2ND：是否启用独立二次验证密码（决定弹窗文案 / 提示用户用哪套密码）
const hasIndependentSecondPassword = ref(false);
async function refreshHasSecondPassword() {
  try {
    hasIndependentSecondPassword.value = await apiHasSecondPassword();
  } catch {
    hasIndependentSecondPassword.value = false;
  }
}
onMounted(() => {
  void refreshHasSecondPassword();
});

function openVerify(action: VerifyAction, itemId: number) {
  verifyModal.open = true;
  verifyModal.loading = false;
  verifyModal.password = "";
  verifyModal.error = "";
  verifyModal.pendingAction = action;
  verifyModal.pendingItemId = itemId;
  setTimeout(() => {
    const el = document.getElementById("pw-verify-input") as HTMLInputElement | null;
    el?.focus();
  }, 50);
}
function closeVerify() {
  verifyModal.open = false;
  verifyModal.password = "";
  verifyModal.error = "";
  verifyModal.loading = false;
  verifyModal.pendingAction = null;
  verifyModal.pendingItemId = null;
}
async function submitVerify() {
  if (!verifyModal.password) {
    verifyModal.error = hasIndependentSecondPassword.value
      ? "请输入二次验证专用密码"
      : "请输入主密码";
    return;
  }
  verifyModal.loading = true;
  verifyModal.error = "";
  try {
    // 优先独立二次验证密码 → 未启用则回退主密码，后端 verify_password_for_pw_view 统一处理
    const ok = await verifyPasswordForPwView(verifyModal.password);
    if (!ok) {
      verifyModal.error = hasIndependentSecondPassword.value
        ? "二次验证专用密码错误"
        : "主密码错误";
      return;
    }
    const action = verifyModal.pendingAction;
    const id = verifyModal.pendingItemId;
    closeVerify();
    if (action && id != null) {
      const item = passwordStore.items.find((p) => p.id === id);
      if (!item) return;
      const s = getRowState(id);
      if (action === "reveal") {
        s.showPassword = !s.showPassword;
      } else if (action === "copy_pw") {
        await doCopyPassword(item);
      }
    }
  } catch (e) {
    verifyModal.error = String(e) || "验证失败";
  } finally {
    verifyModal.loading = false;
  }
}
// 不弹窗直接执行复制（已通过验证后的内部调用）
async function doCopyPassword(item: PasswordMeta) {
  passwordStore.bumpUseCount(item.id);
  const s = getRowState(item.id);
  let pwd = s.decryptFailed ? "" : s.decryptedPassword;
  if (!pwd) {
    try {
      pwd = await passwordStore.reveal(item.id);
      s.decryptedPassword = pwd;
      s.decryptFailed = false;
    } catch (e) {
      appStore.showClipToast("info", "复制失败：" + String(e));
      return;
    }
  }
  await copyWithTimeout(pwd, "密码");
}

function getRowState(id: number): RowState {
  if (!rowStates[id]) {
    rowStates[id] = {
      mode: "collapsed",
      decryptedPassword: "",
      decryptFailed: false,
      showPassword: false,
      isLoading: false,
      form: { title: "", username: "", password: "", url: "", notes: "" },
      strength: 0,
      strengthLabel: "",
      strengthColor: "",
    };
  }
  return rowStates[id];
}

// ===== 新增表单 =====
const addFormOpen = ref(false);
const addForm = reactive({
  title: "",
  username: "",
  password: "",
  url: "",
  notes: "",
});
const addStrength = ref(0);
const addStrengthLabel = ref("");
const addStrengthColor = ref("");
const addShowPassword = ref(false);
const showGenerator = ref(false);
const genLength = ref(16);
const genUpper = ref(true);
const genLower = ref(true);
const genDigits = ref(true);
const genSymbols = ref(true);

onMounted(() => {
  // 关键修复：fire-and-forget，不再 await
  // 之前 await passwordStore.refresh() 如果 IPC 永不返回，view 完全卡住
  // 现在：view 立即渲染空态/列表，loading 只显示顶部小条
  // + 按钮永远可点，新增操作不依赖 refresh
  void passwordStore.refresh();
});

// ===== 搜索过滤 =====
const filteredItems = computed(() => {
  const q = appStore.searchQuery.trim().toLowerCase();
  if (!q) return passwordStore.items;
  return passwordStore.items.filter((p) =>
    p.title.toLowerCase().includes(q) ||
    p.username.toLowerCase().includes(q) ||
    p.url.toLowerCase().includes(q) ||
    p.notes.toLowerCase().includes(q)
  );
});

// P0-#Z：清掉调试 console.log

// ===== 行展开/折叠 =====
// P0-#Z：点击新 tile 时先收起其他已展开的
async function toggleRow(item: PasswordMeta) {
  const s = getRowState(item.id);
  if (s.mode === "collapsed") {
    // 收起其他展开的（互斥）
    for (const [idStr, st] of Object.entries(rowStates)) {
      const id = Number(idStr);
      if (id !== item.id && st.mode !== "collapsed") {
        st.mode = "collapsed";
        st.showPassword = false;
      }
    }
    s.mode = "expanded";
    s.showPassword = false;
    if (!s.decryptedPassword) {
      s.isLoading = true;
      try {
        s.decryptedPassword = await passwordStore.reveal(item.id);
        s.decryptFailed = false;
      } catch (e) {
        s.decryptFailed = true;
        appStore.showClipToast("info", "解密失败：" + String(e));
      } finally {
        s.isLoading = false;
      }
    }
  } else if (s.mode === "expanded") {
    s.mode = "collapsed";
  }
}

function collapseRow(id: number) {
  const s = getRowState(id);
  if (s.mode === "expanded" || s.mode === "editing" || s.mode === "deleting") {
    s.mode = "collapsed";
  }
}

// ===== 复制 =====
async function copyWithTimeout(text: string, label: string) {
  if (!text) {
    appStore.showClipToast("info", `${label}为空`);
    return;
  }
  try {
    const timeout = widgetStore.clipboardClearSeconds;
    await copyToClipboardWithTimeout(text, timeout);
    clipboardCountdown.start(timeout);
    appStore.showClipToast("success", `已复制${label}`);
  } catch (e) {
    appStore.showClipToast("info", "复制失败：" + String(e));
  }
}

async function copyUsername(item: PasswordMeta) {
  passwordStore.bumpUseCount(item.id);
  await copyWithTimeout(item.username, "用户名");
}

// P0-#PW#PER#ITEM#VERIFY：点「📋 复制密码」先弹主密码二次验证
function copyPasswordFromRow(item: PasswordMeta) {
  openVerify("copy_pw", item.id);
}

async function copyUrl(item: PasswordMeta) {
  passwordStore.bumpUseCount(item.id);
  await copyWithTimeout(item.url, "网址");
}

// ===== 编辑 =====
function enterEdit(item: PasswordMeta) {
  const s = getRowState(item.id);
  s.form = {
    title: item.title,
    username: item.username,
    password: "",
    url: item.url,
    notes: item.notes,
  };
  s.strength = 0;
  s.strengthLabel = "";
  s.strengthColor = "";
  s.mode = "editing";
}

function cancelEdit(id: number) {
  const s = getRowState(id);
  s.mode = "expanded";
}

async function saveEdit(item: PasswordMeta) {
  const s = getRowState(item.id);
  if (!s.form.title.trim()) {
    appStore.showClipToast("info", "请填写标题");
    return;
  }
  try {
    const base = {
      title: s.form.title.trim(),
      username: s.form.username.trim(),
      url: s.form.url.trim(),
      notes: s.form.notes.trim(),
    };
    if (s.form.password) {
      // 明确修改密码
      await passwordStore.edit(item.id, { ...base, password: s.form.password });
    } else {
      // P0-B：仅更新元数据——DB 原密码密文字节级保持不变，绝不回写任何占位文案
      await passwordStore.edit(item.id, base);
    }
    // 更新本地缓存
    item.title = s.form.title.trim();
    item.username = s.form.username.trim();
    item.url = s.form.url.trim();
    item.notes = s.form.notes.trim();
    appStore.showClipToast("success", "已保存");
    s.mode = "expanded";
  } catch (e) {
    appStore.showClipToast("info", "保存失败：" + String(e));
  }
}

async function updateRowStrength(id: number) {
  const s = getRowState(id);
  const pwd = s.form.password;
  if (!pwd) {
    s.strength = 0;
    s.strengthLabel = "";
    return;
  }
  try {
    s.strength = await apiStrength(pwd);
  } catch {
    s.strength = 0;
  }
  if (s.strength < 30) { s.strengthLabel = "弱"; s.strengthColor = "#ff5e7e"; }
  else if (s.strength < 60) { s.strengthLabel = "一般"; s.strengthColor = "#ffb84d"; }
  else if (s.strength < 85) { s.strengthLabel = "强"; s.strengthColor = "#3ddc97"; }
  else { s.strengthLabel = "非常强"; s.strengthColor = "#4fc3f7"; }
}

// ===== 删除 =====
function askDelete(id: number) {
  const s = getRowState(id);
  s.mode = "deleting";
}

function cancelDelete(id: number) {
  const s = getRowState(id);
  s.mode = "expanded";
}

const deletingId = ref<number | null>(null);
async function confirmDelete(id: number) {
  deletingId.value = id;
  try {
    await passwordStore.remove(id);
    delete rowStates[id];
    appStore.showClipToast("success", "已删除");
  } catch (e) {
    appStore.showClipToast("info", "删除失败：" + String(e));
  } finally {
    deletingId.value = null;
  }
}

// ===== 新增 =====
function openAdd() {
  addForm.title = "";
  addForm.username = "";
  addForm.password = "";
  addForm.url = "";
  addForm.notes = "";
  addStrength.value = 0;
  addStrengthLabel.value = "";
  addStrengthColor.value = "";
  addShowPassword.value = false;
  showGenerator.value = false;
  addFormOpen.value = true;
}

function cancelAdd() {
  addFormOpen.value = false;
}

const canAdd = computed(
  () => addForm.title.trim() !== "" && addForm.password.length > 0
);

async function submitAdd() {
  if (!canAdd.value) {
    appStore.showClipToast("info", "请填写标题和密码");
    return;
  }
  try {
    await passwordStore.add({
      title: addForm.title.trim(),
      username: addForm.username.trim(),
      password: addForm.password,
      url: addForm.url.trim(),
      notes: addForm.notes.trim(),
    });
    appStore.showClipToast("success", "已添加");
    addFormOpen.value = false;
  } catch (e) {
    appStore.showClipToast("info", "添加失败：" + String(e));
  }
}

async function updateAddStrength() {
  if (!addForm.password) {
    addStrength.value = 0;
    addStrengthLabel.value = "";
    return;
  }
  try {
    addStrength.value = await apiStrength(addForm.password);
  } catch {
    addStrength.value = 0;
  }
  if (addStrength.value < 30) { addStrengthLabel.value = "弱"; addStrengthColor.value = "#ff5e7e"; }
  else if (addStrength.value < 60) { addStrengthLabel.value = "一般"; addStrengthColor.value = "#ffb84d"; }
  else if (addStrength.value < 85) { addStrengthLabel.value = "强"; addStrengthColor.value = "#3ddc97"; }
  else { addStrengthLabel.value = "非常强"; addStrengthColor.value = "#4fc3f7"; }
}

async function generateForAdd() {
  const pwd = await generateRandomPassword(
    genLength.value,
    genUpper.value,
    genLower.value,
    genDigits.value,
    genSymbols.value
  );
  addForm.password = pwd;
  addShowPassword.value = true;
  await updateAddStrength();
}

function getInitial(title: string) {
  const t = (title || "").trim();
  return t ? t[0].toUpperCase() : "?";
}

// P0-#Z：根据标题生成稳定的颜色索引（0-7）
function colorIndex(title: string) {
  const t = (title || "").trim();
  if (!t) return 0;
  let h = 0;
  for (let i = 0; i < t.length; i++) {
    h = (h * 31 + t.charCodeAt(i)) >>> 0;
  }
  return h % 8;
}

// P0-#Z：缩短 URL 显示
function shortUrl(url: string) {
  if (!url) return "";
  try {
    const u = new URL(url);
    return u.hostname + (u.pathname !== "/" ? u.pathname : "");
  } catch {
    return url.length > 30 ? url.slice(0, 30) + "..." : url;
  }
}

function fmtTime(ts: number) {
  if (!ts) return "";
  const d = new Date(ts);
  const pad = (n: number) => n.toString().padStart(2, "0");
  return `${d.getMonth() + 1}/${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}`;
}
</script>

<template>
  <div class="passwords-view">
    <!-- 头部 -->
    <div class="view-header">
      <div class="view-header-left">
        <div class="view-icon icon-purple">🔑</div>
        <div class="view-header-info">
          <h2 class="view-title">
            <span class="view-title-text">密码区</span>
            <span class="title-count">{{ filteredItems.length }}</span>
          </h2>
          <p class="view-subtitle">加密存储 · 剪贴板 15s 自动清空</p>
        </div>
      </div>
      <div class="view-header-actions">
        <button v-if="!addFormOpen" class="action-btn action-btn-primary tap" title="新增密码" @click="openAdd">+</button>
        <button v-else class="action-btn action-btn-secondary tap" @click="cancelAdd">×</button>
      </div>
    </div>

    <!-- 新增表单（行内） -->
    <Transition name="form">
      <div v-if="addFormOpen" class="add-form">
        <div class="form-row">
          <input
            v-model="addForm.title"
            class="form-input"
            placeholder="标题 *"
            maxlength="64"
          />
        </div>
        <div class="form-row">
          <input
            v-model="addForm.username"
            class="form-input"
            placeholder="用户名 / 账号"
          />
        </div>
        <div class="form-row form-row-pwd">
          <div class="pwd-input-wrap">
            <input
              v-model="addForm.password"
              :type="addShowPassword ? 'text' : 'password'"
              class="form-input"
              placeholder="密码 *"
              @input="updateAddStrength"
            />
            <button
              class="pwd-toggle tap"
              @click="addShowPassword = !addShowPassword"
            >
              {{ addShowPassword ? "🙈" : "👁️" }}
            </button>
          </div>
          <button class="gen-btn tap" @click="showGenerator = !showGenerator" title="生成">
            🎲
          </button>
        </div>

        <Transition name="gen">
          <div v-if="showGenerator" class="generator">
            <div class="gen-row">
              <input
                v-model.number="genLength"
                type="range"
                min="8"
                max="32"
                class="gen-range"
              />
              <span class="gen-length">{{ genLength }}</span>
            </div>
            <div class="gen-options">
              <label><input v-model="genUpper" type="checkbox" /> 大写</label>
              <label><input v-model="genLower" type="checkbox" /> 小写</label>
              <label><input v-model="genDigits" type="checkbox" /> 数字</label>
              <label><input v-model="genSymbols" type="checkbox" /> 符号</label>
            </div>
            <button class="gen-apply tap" @click="generateForAdd">🎲 生成</button>
          </div>
        </Transition>

        <div v-if="addForm.password" class="strength-bar">
          <div class="strength-track">
            <div
              class="strength-fill"
              :style="{ width: addStrength + '%', background: addStrengthColor }"
            ></div>
          </div>
          <span class="strength-label" :style="{ color: addStrengthColor }">
            {{ addStrengthLabel }}
          </span>
        </div>

        <div class="form-row">
          <input v-model="addForm.url" class="form-input" placeholder="网址（可选）" />
        </div>
        <div class="form-row">
          <textarea
            v-model="addForm.notes"
            class="form-input form-textarea"
            placeholder="备注（可选）"
            rows="2"
          ></textarea>
        </div>
        <div class="form-actions">
          <button class="btn-secondary tap" @click="cancelAdd">取消</button>
          <button
            class="btn-primary tap"
            :disabled="!canAdd"
            @click="submitAdd"
          >
            ✓ 添加
          </button>
        </div>
      </div>
    </Transition>

    <!-- 加载中：仅显示顶部小条（不遮盖 + 按钮、列表） -->
    <Transition name="fade">
      <div v-if="passwordStore.loading" class="loading-bar">
        <div class="loading-spinner small"></div>
        <span>正在加载密码…</span>
      </div>
    </Transition>

    <!-- 错误：顶部小条提示 + 操作按钮
         P0-#LOCK#UI#BUTTON#ALWAYS：总是显示"去解锁"按钮（不依赖 isLockedError 检测）
           之前：依赖 string.includes("应用已锁定") 检测
             → 但 build 后用户还是看到"重试"（说明检测失败 / build 没生效）
             → 用户卡在错误条
           现在：error 存在时**总是**显示"去解锁"按钮（必要时也显示"重试"）
             → 1 步恢复（点"去解锁"→ 跳锁屏页 → 解锁 → 复制密码） -->
    <Transition name="fade">
      <div v-if="passwordStore.error && !passwordStore.loading" class="error-bar">
        <span class="error-icon">⚠️</span>
        <span>{{ passwordStore.error }}</span>
        <button class="error-unlock tap" @click="goUnlock">🔓 去解锁</button>
        <button class="error-retry tap" @click="() => void passwordStore.refresh()">↻ 重试</button>
      </div>
    </Transition>

    <!-- 默认骨架屏（首次加载 + 空态共用，保证 UI 永远稳定不闪）
         只有加载完成 + 确实无数据时才显示带"+"的真正空态 -->
    <div
      v-if="passwordStore.loading || !passwordStore.hasLoaded"
      class="skeleton-list"
    >
      <div v-for="i in 3" :key="i" class="skeleton-row">
        <div class="skeleton-icon"></div>
        <div class="skeleton-text">
          <div class="skeleton-line" style="width: 60%"></div>
          <div class="skeleton-line short" style="width: 35%"></div>
        </div>
      </div>
    </div>

    <!-- 真正空态：已加载完 + 确实无数据 -->
    <div
      v-else-if="filteredItems.length === 0"
      class="empty"
    >
      <div class="empty-icon">🔑</div>
      <p v-if="appStore.searchQuery">没有匹配的密码</p>
      <p v-else>还没有密码</p>
      <button v-if="!appStore.searchQuery" class="empty-btn tap" @click="openAdd">
        + 添加第一个
      </button>
    </div>

    <!-- P0-#Z：图标网格（首字母+色块） + 点开展开为详情行 -->
    <div v-else class="pwd-grid">
      <!-- 图标卡片（折叠态） -->
      <template v-for="item in filteredItems" :key="`tile-${item.id}`">
        <div
          v-if="getRowState(item.id).mode === 'collapsed'"
          class="pwd-tile tap"
          :class="`hue-${colorIndex(item.title)}`"
          :title="`${item.title}${item.username ? ' · ' + item.username : ''}`"
          @click="toggleRow(item)"
        >
          <span class="pwd-tile-letter">{{ getInitial(item.title) }}</span>
          <span class="pwd-tile-name">{{ item.title }}</span>
          <!-- P0-#Z：火苗徽章（前端计数，复制密码/用户名/网址时 +1） -->
          <span
            v-if="passwordStore.getUseCount(item.id) > 0"
            class="pwd-tile-flame"
            :title="`已复制 ${passwordStore.getUseCount(item.id)} 次`"
          >🔥 {{ passwordStore.getUseCount(item.id) }}</span>
        </div>
      </template>

      <!-- 展开的详情行（横跨整行） -->
      <template v-for="item in filteredItems" :key="`detail-${item.id}`">
        <div
          v-if="getRowState(item.id).mode !== 'collapsed'"
          class="pwd-detail-row"
        >
          <div class="pwd-detail-card">
            <!-- 详情头部：图标 + 名称 + 关闭 -->
            <div class="pwd-detail-header">
              <div class="pwd-tile small" :class="`hue-${colorIndex(item.title)}`">
                <span class="pwd-tile-letter">{{ getInitial(item.title) }}</span>
              </div>
              <div class="pwd-detail-titles">
                <div class="pwd-detail-title">{{ item.title }}</div>
                <div class="pwd-detail-sub">
                  <span v-if="item.username">👤 {{ item.username }}</span>
                  <span v-if="item.url" class="pwd-detail-url">· {{ shortUrl(item.url) }}</span>
                </div>
              </div>
              <button class="pwd-detail-close tap" title="收起" @click="collapseRow(item.id)">▾</button>
            </div>

            <!-- 展开态：详情 + 操作 -->
            <div v-if="getRowState(item.id).mode === 'expanded'" class="pwd-detail-body">
              <div class="detail-row">
                <span class="detail-label">密码</span>
                <div class="detail-value">
                  <span v-if="getRowState(item.id).isLoading" class="muted">解密中...</span>
                  <span v-else class="password">
                    {{ getRowState(item.id).showPassword
                        ? (getRowState(item.id).decryptFailed
                            ? "（解密失败）"
                            : getRowState(item.id).decryptedPassword)
                        : "••••••••••••" }}
                  </span>
                  <button
                    class="mini-btn tap"
                    :disabled="getRowState(item.id).isLoading"
                    @click="openVerify('reveal', item.id)"
                  >
                    {{ getRowState(item.id).showPassword ? "🙈" : "👁️" }}
                  </button>
                  <button
                    class="mini-btn tap"
                    :disabled="getRowState(item.id).isLoading"
                    @click="copyPasswordFromRow(item)"
                  >
                    📋
                  </button>
                </div>
              </div>

              <div v-if="item.url" class="detail-row">
                <span class="detail-label">网址</span>
                <div class="detail-value">
                  <a class="link" :href="item.url" target="_blank" rel="noopener">{{ item.url }}</a>
                  <button class="mini-btn tap" @click="copyUrl(item)">📋</button>
                </div>
              </div>

              <div v-if="item.notes" class="detail-row">
                <span class="detail-label">备注</span>
                <div class="detail-value notes">{{ item.notes }}</div>
              </div>

              <div class="detail-row meta">
                <span class="meta-text">更新于 {{ fmtTime(item.updated_at) }}</span>
              </div>

              <div class="row-actions">
                <button class="action-btn tap" @click="enterEdit(item)">✏️ 编辑</button>
                <button class="action-btn danger tap" @click="askDelete(item.id)">🗑️ 删除</button>
                <button class="action-btn tap copy" @click="copyUsername(item)">👤 复制账号</button>
              </div>
            </div>

            <!-- 编辑态 -->
            <div v-else-if="getRowState(item.id).mode === 'editing'" class="pwd-edit-body" @click.stop>
              <div class="edit-header">
                <span>✏️ 编辑密码</span>
                <button class="x-btn tap" @click="cancelEdit(item.id)">✕</button>
              </div>
              <div class="form-row">
                <input v-model="getRowState(item.id).form.title" class="form-input" placeholder="标题" />
              </div>
              <div class="form-row">
                <input v-model="getRowState(item.id).form.username" class="form-input" placeholder="用户名" />
              </div>
              <div class="form-row form-row-pwd">
                <div class="pwd-input-wrap">
                  <input
                    v-model="getRowState(item.id).form.password"
                    :type="getRowState(item.id).showPassword ? 'text' : 'password'"
                    class="form-input"
                    placeholder="新密码（留空不修改）"
                    @input="updateRowStrength(item.id)"
                  />
                  <button
                    class="pwd-toggle tap"
                    @click="getRowState(item.id).showPassword = !getRowState(item.id).showPassword"
                  >
                    {{ getRowState(item.id).showPassword ? "🙈" : "👁️" }}
                  </button>
                </div>
              </div>
              <div v-if="getRowState(item.id).form.password" class="strength-bar">
                <div class="strength-track">
                  <div
                    class="strength-fill"
                    :style="{
                      width: getRowState(item.id).strength + '%',
                      background: getRowState(item.id).strengthColor,
                    }"
                  ></div>
                </div>
                <span
                  class="strength-label"
                  :style="{ color: getRowState(item.id).strengthColor }"
                >
                  {{ getRowState(item.id).strengthLabel }}
                </span>
              </div>
              <div class="form-row">
                <input v-model="getRowState(item.id).form.url" class="form-input" placeholder="网址" />
              </div>
              <div class="form-row">
                <textarea
                  v-model="getRowState(item.id).form.notes"
                  class="form-input form-textarea"
                  placeholder="备注"
                  rows="2"
                ></textarea>
              </div>
              <div class="form-actions">
                <button class="btn-secondary tap" @click="cancelEdit(item.id)">取消</button>
                <button class="btn-primary tap" @click="saveEdit(item)">✓ 保存</button>
              </div>
            </div>

            <!-- 删除确认 -->
            <div v-else-if="getRowState(item.id).mode === 'deleting'" class="pwd-deleting-body">
              <ConfirmInline
                :title="`确定删除「${item.title}」？`"
                :loading="deletingId === item.id"
                @cancel="cancelDelete(item.id)"
                @confirm="confirmDelete(item.id)"
              />
            </div>
          </div>
        </div>
      </template>
    </div>

    <!-- P0-#PW#PER#ITEM#VERIFY：单条目主密码二次验证弹窗（显示/复制密码前） -->
    <Transition name="modal">
      <div
        v-if="verifyModal.open"
        class="verify-mask"
        @click.self="closeVerify"
        @keydown.esc="closeVerify"
      >
        <div class="verify-modal" role="dialog" aria-modal="true">
          <div class="verify-header">
            <span class="verify-icon">{{ hasIndependentSecondPassword ? "🔐" : "🔒" }}</span>
            <span class="verify-title">
              {{ hasIndependentSecondPassword ? "请输入二次验证专用密码以继续" : "请输入主密码以继续" }}
            </span>
            <button class="verify-close tap" @click="closeVerify" aria-label="关闭">×</button>
          </div>
          <div class="verify-body">
            <div class="verify-hint">
              {{
                verifyModal.pendingAction === "reveal"
                  ? hasIndependentSecondPassword
                    ? "您即将查看明文密码，需要输入您设置的「二次验证专用密码」（独立于主密码）。"
                    : "您即将查看明文密码，需要二次确认身份（输入主密码）。如需更高级安全可前往「设置 → 安全与密码」单独设置独立二次验证密码。"
                  : hasIndependentSecondPassword
                    ? "您即将复制密码到剪贴板，需要输入「二次验证专用密码」（独立于主密码）。"
                    : "您即将复制密码到剪贴板，需要二次确认身份（输入主密码）。如需更高级安全可前往「设置 → 安全与密码」单独设置独立二次验证密码。"
              }}
            </div>
            <div class="verify-input-row">
              <input
                id="pw-verify-input"
                v-model="verifyModal.password"
                type="password"
                class="form-input verify-input"
                :placeholder="hasIndependentSecondPassword ? '二次验证专用密码（≥6位）' : '主密码（至少 6 位）'"
                autocomplete="new-password"
                @keydown.enter="submitVerify"
              />
              <button
                class="verify-submit tap"
                :disabled="verifyModal.loading || !verifyModal.password"
                @click="submitVerify"
              >
                <template v-if="verifyModal.loading">
                  <span class="loading-spinner small"></span>
                  <span>验证中…</span>
                </template>
                <template v-else>✓ 确认</template>
              </button>
            </div>
            <div v-if="verifyModal.error" class="verify-error">
              ⚠️ {{ verifyModal.error }}
            </div>
            <div class="verify-tip">
              💡 每次查看/复制密码都要重新验证主密码，防止被他人临时窥看。
            </div>
          </div>
        </div>
      </div>
    </Transition>
  </div>
</template>

<style scoped>
.passwords-view { padding-bottom: 8px; }
/* view-header 等用 global 统一样式（这里只补 PasswordView 专属样式） */

.btn-add,
.btn-cancel {
  width: 28px;
  height: 28px;
  border-radius: 50%;
  background: linear-gradient(135deg, var(--accent) 0%, var(--accent-bright) 100%);
  color: white;
  border: none;
  cursor: pointer;
  font-size: 16px;
  display: flex;
  align-items: center;
  justify-content: center;
  box-shadow: 0 2px 8px rgba(79, 124, 255, 0.4);
  transition: all 0.2s;
}
.btn-add:hover { transform: scale(1.1); box-shadow: 0 4px 12px rgba(79, 124, 255, 0.5); }
.btn-cancel {
  background: var(--bg-tertiary);
  color: var(--text-secondary);
  font-size: 11px;
  box-shadow: none;
}

/* 通用表单 */
.add-form,
.row-editing {
  background: var(--bg-primary);
  border: 1px solid var(--border-bright);
  border-radius: var(--radius);
  padding: 12px;
  margin-bottom: 10px;
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.form-row { display: flex; gap: 6px; }
.form-row-pwd { align-items: center; min-width: 0; }

.form-input {
  flex: 1;
  min-width: 0; /* 关键：让 input 在 flex 容器里能缩到 0 宽，长密码才能完整显示在滚动区 */
  background: rgba(0, 0, 0, 0.3);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  padding: 7px 10px;
  color: var(--text-primary);
  font-size: 12px;
  outline: none;
  transition: all 0.2s;
  font-family: inherit;
}

.form-input:focus {
  border-color: var(--accent);
  background: rgba(0, 0, 0, 0.4);
  box-shadow: 0 0 0 2px var(--accent-soft);
}

.form-input::placeholder { color: var(--text-faint); }

.form-textarea {
  resize: vertical;
  min-height: 40px;
  font-family: inherit;
}

.pwd-input-wrap { flex: 1; min-width: 0; position: relative; }

.pwd-toggle {
  position: absolute;
  right: 6px;
  top: 50%;
  transform: translateY(-50%);
  background: transparent;
  border: none;
  cursor: pointer;
  font-size: 13px;
  padding: 2px 6px;
  opacity: 0.6;
}
.pwd-toggle:hover { opacity: 1; }

.gen-btn {
  width: 30px;
  height: 30px;
  border-radius: 6px;
  background: var(--bg-tertiary);
  border: 1px solid var(--border);
  color: var(--text-secondary);
  cursor: pointer;
  font-size: 14px;
  transition: all 0.2s;
}
.gen-btn:hover { background: var(--accent); color: white; border-color: var(--accent); }

.generator {
  background: rgba(0, 0, 0, 0.3);
  border-radius: 6px;
  padding: 8px 10px;
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.gen-row { display: flex; align-items: center; gap: 8px; }
.gen-range { flex: 1; accent-color: var(--accent); }
.gen-length { font-size: 11px; color: var(--accent-bright); min-width: 20px; text-align: right; font-family: monospace; }

.gen-options {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 4px;
  font-size: 11px;
  color: var(--text-secondary);
}
.gen-options label { display: flex; align-items: center; gap: 4px; cursor: pointer; }
.gen-options input { accent-color: var(--accent); cursor: pointer; }

.gen-apply {
  width: 100%;
  padding: 5px;
  background: linear-gradient(135deg, var(--accent) 0%, #4fc3f7 100%);
  color: white;
  border: none;
  border-radius: 5px;
  font-size: 11px;
  cursor: pointer;
}

/* 强度条 */
.strength-bar {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 0 2px;
}
.strength-track {
  flex: 1;
  height: 3px;
  background: var(--bg-tertiary);
  border-radius: 2px;
  overflow: hidden;
}
.strength-fill {
  height: 100%;
  border-radius: 2px;
  transition: width 0.3s, background 0.3s;
  box-shadow: 0 0 4px currentColor;
}
.strength-label {
  font-size: 10px;
  font-weight: 600;
  min-width: 36px;
  text-align: right;
}

.form-actions {
  display: flex;
  gap: 6px;
  margin-top: 4px;
}

.btn-primary,
.btn-secondary,
.btn-danger {
  flex: 1;
  padding: 7px 12px;
  border-radius: 6px;
  font-size: 12px;
  font-weight: 600;
  cursor: pointer;
  border: none;
  transition: all 0.2s;
}
.btn-primary {
  background: linear-gradient(135deg, var(--accent) 0%, var(--highlight) 100%);
  color: white;
  box-shadow: 0 2px 8px rgba(79, 124, 255, 0.3);
}
.btn-primary:disabled { opacity: 0.4; cursor: not-allowed; }
.btn-secondary {
  background: var(--bg-tertiary);
  color: var(--text-secondary);
  border: 1px solid var(--border);
  flex: 0 0 60px;
}
.btn-secondary:hover { background: var(--bg-secondary); color: var(--text-primary); }
.btn-danger {
  background: linear-gradient(135deg, #ff5e7e 0%, #ff8a5e 100%);
  color: white;
  box-shadow: 0 2px 8px rgba(255, 94, 126, 0.3);
  flex: 0 0 60px;
}
.btn-danger:hover { transform: translateY(-1px); }

/* P0-#Z：图标网格 + 点开展开为详情行 */
.pwd-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, 80px);
  gap: 10px;
  align-content: start;
  min-height: 60px;
}
@media (min-width: 560px) {
  .pwd-grid { grid-template-columns: repeat(auto-fill, 88px); gap: 12px; }
}
@media (min-width: 800px) {
  .pwd-grid { grid-template-columns: repeat(auto-fill, 96px); gap: 14px; }
}

.pwd-tile {
  position: relative;
  width: 100%;
  aspect-ratio: 1;
  border-radius: 14px;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 4px;
  padding: 6px 4px;
  cursor: pointer;
  overflow: hidden;
  transition: transform 0.15s var(--ease-smooth), box-shadow 0.15s var(--ease-smooth);
  -webkit-app-region: no-drag;
  border: 1px solid rgba(255, 255, 255, 0.08);
}
.pwd-tile:hover {
  transform: translateY(-2px) scale(1.04);
  box-shadow: 0 8px 20px rgba(0, 0, 0, 0.4);
  border-color: rgba(255, 255, 255, 0.18);
}
.pwd-tile:active {
  transform: scale(0.97);
}
.pwd-tile-letter {
  font-size: 26px;
  font-weight: 700;
  color: rgba(255, 255, 255, 0.96);
  text-shadow: 0 1px 2px rgba(0, 0, 0, 0.3);
  line-height: 1;
}
.pwd-tile-name {
  font-size: 10px;
  font-weight: 500;
  color: rgba(255, 255, 255, 0.78);
  text-align: center;
  max-width: 100%;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  line-height: 1.2;
}
/* P0-#Z：密码卡片火苗徽章（前端计数） */
.pwd-tile-flame {
  position: absolute;
  top: 4px;
  right: 4px;
  display: inline-flex;
  align-items: center;
  gap: 2px;
  padding: 1px 5px;
  border-radius: 7px;
  font-size: 9px;
  font-weight: 700;
  background: linear-gradient(135deg, rgba(255, 138, 94, 0.95) 0%, rgba(255, 78, 110, 0.95) 100%);
  color: #fff;
  box-shadow: 0 1px 3px rgba(255, 78, 110, 0.4);
  line-height: 1.2;
  pointer-events: none;
}
.pwd-tile.small {
  width: 38px;
  height: 38px;
  aspect-ratio: auto;
  border-radius: 10px;
  flex-shrink: 0;
}
.pwd-tile.small .pwd-tile-letter { font-size: 14px; }

/* 8 种 hue（基于标题 hash 取模） */
.hue-0 { background: linear-gradient(135deg, #ff8a5e, #ffb84d); }
.hue-1 { background: linear-gradient(135deg, #ff5e8a, #ff94b8); }
.hue-2 { background: linear-gradient(135deg, #b46aff, #d8a0ff); }
.hue-3 { background: linear-gradient(135deg, #4f7cff, #7eb6ff); }
.hue-4 { background: linear-gradient(135deg, #00b8d9, #5fdce8); }
.hue-5 { background: linear-gradient(135deg, #00d49d, #5ee8c0); }
.hue-6 { background: linear-gradient(135deg, #f59e0b, #fbbf24); }
.hue-7 { background: linear-gradient(135deg, #ec4899, #f472b6); }

/* 展开的详情行：横跨整行 */
.pwd-detail-row {
  grid-column: 1 / -1;
  animation: expandIn 0.25s var(--ease-out);
}
.pwd-detail-card {
  background: var(--bg-primary);
  border: 1px solid var(--border-strong);
  border-radius: 14px;
  padding: 14px 16px;
  box-shadow: 0 4px 16px rgba(0, 0, 0, 0.3);
}
.pwd-detail-header {
  display: flex;
  align-items: center;
  gap: 10px;
  padding-bottom: 12px;
  border-bottom: 1px dashed var(--border);
  margin-bottom: 12px;
}
.pwd-detail-titles { flex: 1; min-width: 0; }
.pwd-detail-title {
  font-size: 14px;
  font-weight: 700;
  color: var(--text-primary);
  margin-bottom: 2px;
}
.pwd-detail-sub {
  font-size: 11px;
  color: var(--text-muted);
  display: flex;
  gap: 6px;
  align-items: center;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.pwd-detail-url {
  color: var(--accent-bright);
  font-family: monospace;
}
.pwd-detail-close {
  width: 28px;
  height: 28px;
  border-radius: 6px;
  background: transparent;
  border: none;
  color: var(--text-muted);
  cursor: pointer;
  font-size: 14px;
  font-weight: 700;
  transition: all 0.15s;
}
.pwd-detail-close:hover {
  background: var(--bg-tertiary);
  color: var(--text-primary);
}

.pwd-detail-body,
.pwd-edit-body,
.pwd-deleting-body { display: flex; flex-direction: column; gap: 8px; }

@keyframes expandIn {
  from { opacity: 0; transform: translateY(-6px); }
  to { opacity: 1; transform: translateY(0); }
}
.x-btn:hover { background: var(--bg-tertiary); color: var(--text-primary); }

.detail-row {
  display: flex;
  align-items: flex-start;
  gap: 8px;
  margin-bottom: 6px;
  font-size: 12px;
}
.detail-row:last-child { margin-bottom: 0; }

.detail-label {
  color: var(--text-muted);
  min-width: 32px;
  font-size: 10px;
  text-transform: uppercase;
  letter-spacing: 0.5px;
  padding-top: 1px;
}

.detail-value {
  flex: 1;
  display: flex;
  align-items: center;
  gap: 6px;
  background: rgba(0, 0, 0, 0.25);
  border: 1px solid var(--border);
  border-radius: 6px;
  padding: 5px 6px 5px 8px;
  font-family: 'Cascadia Code', 'Consolas', monospace;
  word-break: break-all;
}

.detail-value.password {
  color: var(--text-primary);
  letter-spacing: 0.5px;
  flex: 1;
}

.detail-value.notes {
  background: rgba(0, 0, 0, 0.25);
  white-space: pre-wrap;
  font-family: inherit;
  line-height: 1.4;
  padding: 6px 8px;
}

.detail-value .link {
  color: var(--accent-bright);
  text-decoration: none;
  font-family: inherit;
  flex: 1;
  word-break: break-all;
  font-size: 11px;
}
.detail-value .link:hover { text-decoration: underline; }

.mini-btn {
  background: var(--bg-tertiary);
  border: 1px solid var(--border);
  color: var(--text-secondary);
  border-radius: 4px;
  padding: 2px 6px;
  font-size: 11px;
  cursor: pointer;
  flex-shrink: 0;
  transition: all 0.2s;
}
.mini-btn:hover {
  background: var(--accent);
  color: white;
  border-color: var(--accent);
}
.mini-btn:disabled { opacity: 0.4; cursor: not-allowed; }

.detail-row.meta {
  margin-top: 4px;
  font-size: 10px;
  color: var(--text-faint);
}
.meta-text { padding-left: 40px; }

.row-actions {
  display: flex;
  gap: 6px;
  margin-top: 10px;
  padding-top: 10px;
  border-top: 1px dashed var(--border);
}

.action-btn {
  flex: 1;
  padding: 6px 8px;
  background: var(--bg-tertiary);
  border: 1px solid var(--border);
  color: var(--text-secondary);
  border-radius: 6px;
  font-size: 11px;
  cursor: pointer;
  transition: all 0.2s;
  white-space: nowrap;
}
.action-btn:hover {
  background: var(--accent);
  color: white;
  border-color: var(--accent);
  transform: translateY(-1px);
}
.action-btn.danger:hover {
  background: var(--highlight);
  border-color: var(--highlight);
}
.action-btn.copy { flex: 0 0 auto; padding: 6px 10px; }

/* 编辑态 */
.edit-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  font-size: 12px;
  font-weight: 600;
  color: var(--text-secondary);
  margin-bottom: 4px;
}

/* 删除态 */
.row-deleting {
  padding: 14px 12px;
  text-align: center;
  background: rgba(255, 94, 126, 0.08);
  animation: expandIn 0.2s var(--ease-out);
}
.del-icon { font-size: 24px; margin-bottom: 6px; }
.del-text { font-size: 12px; color: var(--text-secondary); margin-bottom: 10px; line-height: 1.5; }
.del-text strong { color: var(--highlight); }

/* 空状态 */
.empty {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  padding: 32px 20px;
  text-align: center;
  color: var(--text-muted);
}
.empty-icon { font-size: 36px; opacity: 0.3; margin-bottom: 8px; }
.empty p { font-size: 12px; margin-bottom: 12px; }
.empty-btn {
  background: linear-gradient(135deg, var(--accent) 0%, var(--highlight) 100%);
  color: white;
  border: none;
  border-radius: 6px;
  padding: 6px 14px;
  font-size: 12px;
  cursor: pointer;
}
.loading-spinner {
  width: 22px;
  height: 22px;
  border: 2px solid var(--border);
  border-top-color: var(--accent);
  border-radius: 50%;
  animation: spin 0.8s linear infinite;
  margin-bottom: 8px;
}
.loading-spinner.small {
  width: 14px;
  height: 14px;
  border-width: 2px;
  margin-bottom: 0;
  flex-shrink: 0;
}

/* 加载小条：顶部 inline 显示，不遮盖其他元素 */
.loading-bar {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 6px 12px;
  margin-bottom: 8px;
  background: var(--accent-soft);
  border: 1px solid var(--accent-soft);
  border-radius: 8px;
  font-size: 12px;
  color: var(--accent-bright);
  -webkit-app-region: no-drag;
}

/* 错误小条：顶部 inline，带重试 */
.error-bar {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 6px 12px;
  margin-bottom: 8px;
  background: rgba(239, 68, 68, 0.15);
  border: 1px solid rgba(239, 68, 68, 0.4);
  border-radius: 8px;
  font-size: 12px;
  color: #fca5a5;
  -webkit-app-region: no-drag;
}
.error-icon { font-size: 14px; flex-shrink: 0; }
.error-bar > span:nth-child(2) { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.error-retry {
  background: rgba(239, 68, 68, 0.25);
  border: 1px solid rgba(239, 68, 68, 0.5);
  color: white;
  border-radius: 6px;
  padding: 2px 8px;
  font-size: 11px;
  cursor: pointer;
  flex-shrink: 0;
}
.error-retry:hover { background: rgba(239, 68, 68, 0.45); }
/* P0-#LOCK#UI#BUTTON："去解锁"按钮（紫色，明显区别于红色"重试"）
   紫色 = 引导用户去解锁页 → 完整闭环 */
.error-unlock {
  background: linear-gradient(135deg, #a855f7 0%, #7c3aed 100%);
  border: 1px solid rgba(168, 85, 247, 0.7);
  color: white;
  border-radius: 6px;
  padding: 2px 10px;
  font-size: 11px;
  font-weight: 600;
  cursor: pointer;
  flex-shrink: 0;
  box-shadow: 0 0 0 0 rgba(168, 85, 247, 0);
  animation: lock-pulse 1.5s ease-in-out infinite;
}
.error-unlock:hover { transform: translateY(-1px); box-shadow: 0 4px 12px rgba(168, 85, 247, 0.45); }
.error-unlock:active { transform: translateY(0) scale(0.96); }
@keyframes lock-pulse {
  0%, 100% { box-shadow: 0 0 0 0 rgba(168, 85, 247, 0); }
  50% { box-shadow: 0 0 0 4px rgba(168, 85, 247, 0.2); }
}

/* 淡入淡出（loading-bar / error-bar 用） */
.fade-enter-active, .fade-leave-active { transition: all 0.2s var(--ease-smooth); }
.fade-enter-from, .fade-leave-to { opacity: 0; transform: translateY(-4px); }

/* ===== 骨架屏（默认固定状态，UI 永远不闪） ===== */
.skeleton-list {
  display: flex;
  flex-direction: column;
  gap: 8px;
  padding: 4px 0;
}
.skeleton-row {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 10px 12px;
  background: var(--bg-secondary);
  border: 1px solid var(--border);
  border-radius: 12px;
  overflow: hidden;
}
.skeleton-icon {
  width: 36px;
  height: 36px;
  border-radius: 9px;
  background: linear-gradient(
    90deg,
    rgba(255, 255, 255, 0.05) 0%,
    rgba(255, 255, 255, 0.12) 50%,
    rgba(255, 255, 255, 0.05) 100%
  );
  background-size: 200% 100%;
  animation: skeletonShimmer 1.6s ease-in-out infinite;
  flex-shrink: 0;
}
.skeleton-text {
  flex: 1;
  display: flex;
  flex-direction: column;
  gap: 5px;
}
.skeleton-line {
  height: 10px;
  border-radius: 5px;
  background: linear-gradient(
    90deg,
    rgba(255, 255, 255, 0.05) 0%,
    rgba(255, 255, 255, 0.12) 50%,
    rgba(255, 255, 255, 0.05) 100%
  );
  background-size: 200% 100%;
  animation: skeletonShimmer 1.6s ease-in-out infinite;
}
.skeleton-line.short { height: 8px; width: 40%; }
@keyframes skeletonShimmer {
  0% { background-position: 200% 0; }
  100% { background-position: -200% 0; }
}

/* 表单动画 */
.form-enter-active,
.form-leave-active { transition: all 0.25s var(--ease-out); overflow: hidden; }
.form-enter-from,
.form-leave-to { opacity: 0; max-height: 0; padding-top: 0; padding-bottom: 0; margin-bottom: 0; }
.form-enter-to,
.form-leave-from { max-height: 600px; }

.gen-enter-active,
.gen-leave-active { transition: all 0.2s var(--ease-out); overflow: hidden; }
.gen-enter-from,
.gen-leave-to { opacity: 0; max-height: 0; padding: 0 10px; }
.gen-enter-to,
.gen-leave-from { max-height: 200px; padding: 8px 10px; }

/* ===== P0-#PW#PER#ITEM#VERIFY：单条目主密码二次验证弹窗 ===== */
.verify-mask {
  position: fixed; inset: 0;
  background: rgba(0, 0, 0, 0.65);
  backdrop-filter: blur(10px);
  display: flex; align-items: center; justify-content: center;
  z-index: 200;
  -webkit-app-region: no-drag;
}
.verify-modal {
  background: var(--bg-secondary);
  border: 1px solid var(--border-strong);
  border-radius: 14px;
  width: 90%; max-width: 380px;
  box-shadow: 0 20px 60px rgba(0, 0, 0, 0.55), 0 0 0 1px rgba(168, 85, 247, 0.25);
  overflow: hidden;
  animation: verifyPop 0.22s var(--ease-out);
}
@keyframes verifyPop {
  from { opacity: 0; transform: translateY(8px) scale(0.96); }
  to   { opacity: 1; transform: translateY(0)    scale(1); }
}
.verify-header {
  display: flex; align-items: center; gap: 8px;
  padding: 12px 14px;
  border-bottom: 1px solid var(--border);
  background: linear-gradient(135deg, rgba(99, 102, 241, 0.14), rgba(168, 85, 247, 0.08));
}
.verify-icon { font-size: 16px; flex-shrink: 0; }
.verify-title {
  flex: 1; font-size: 13px; font-weight: 700; color: var(--text-primary);
}
.verify-close {
  width: 24px; height: 24px; border-radius: 6px; border: none;
  background: transparent; color: var(--text-muted); font-size: 16px;
  cursor: pointer; display: flex; align-items: center; justify-content: center;
  transition: all 0.15s;
}
.verify-close:hover { background: var(--bg-tertiary); color: var(--text-primary); }

.verify-body { padding: 14px; display: flex; flex-direction: column; gap: 10px; }
.verify-hint {
  font-size: 12px; color: var(--text-muted); line-height: 1.5;
  padding: 8px 10px; border-radius: 8px;
  background: rgba(99, 102, 241, 0.08);
  border: 1px solid rgba(99, 102, 241, 0.2);
}
.verify-input-row {
  display: flex; gap: 6px; align-items: stretch;
}
.verify-input {
  flex: 1;
  min-width: 0;
  height: 36px;
  font-size: 13px;
  padding: 0 11px;
  letter-spacing: 1px;
}
.verify-submit {
  display: inline-flex; align-items: center; justify-content: center; gap: 6px;
  height: 36px; padding: 0 16px;
  background: linear-gradient(135deg, var(--accent) 0%, #a855f7 100%);
  color: white; font-size: 12px; font-weight: 600;
  border: none; border-radius: 8px; cursor: pointer;
  box-shadow: 0 2px 8px rgba(99, 102, 241, 0.35);
  transition: all 0.15s;
  white-space: nowrap;
  flex-shrink: 0;
}
.verify-submit:hover:not(:disabled) {
  transform: translateY(-1px);
  box-shadow: 0 4px 12px rgba(168, 85, 247, 0.45);
}
.verify-submit:active:not(:disabled) { transform: translateY(0) scale(0.97); }
.verify-submit:disabled {
  opacity: 0.5; cursor: not-allowed;
  box-shadow: none;
}
.verify-error {
  font-size: 11.5px; color: #fca5a5;
  padding: 6px 10px; border-radius: 6px;
  background: rgba(239, 68, 68, 0.12);
  border: 1px solid rgba(239, 68, 68, 0.4);
  font-weight: 500;
}
.verify-tip {
  font-size: 10.5px; color: var(--text-faint);
  line-height: 1.5;
  padding-top: 2px;
  border-top: 1px dashed var(--border);
}
</style>

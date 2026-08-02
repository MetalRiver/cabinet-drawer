import { defineStore } from "pinia";
import { ref } from "vue";
import {
  loadWidgetConfig,
  saveWidgetConfig,
  setAlwaysOnTop,
  setIgnoreCursorEvents,
  setMiniModeWithPos,
  setWindowPosition,
  getWindowPosition,
  getWindowSize,
  isDockHidden,
} from "../api";

export type DockSide = "top" | "bottom" | "left" | "right";

export const useWidgetStore = defineStore("widget", () => {
  // ===== 状态 =====
  /** 贴边自动收起：已永久停用（Win32钩子存在未响应卡死问题，v2.0重写后回归），状态强制为 false 以兼容旧逻辑短路 */
  const autoHide = ref(false);
  const edgeHidden = ref(false);
  const alwaysOnTop = ref(false); // 默认不置顶：抽屉柜是普通应用，需要时再勾选
  const autoLockMinutes = ref(5);
  const clipboardClearSeconds = ref(15);
  const tempExpireDays = ref(3);
  /** 极简图标列模式（缩略态） */
  const miniMode = ref(false);
  /** 鼠标穿透：true=不接收事件（穿透到桌面） */
  const clickThrough = ref(false);
  const ready = ref(false);

  // ===== 初始化：从后端加载 =====
  async function init() {
    try {
      // 贴边自动收起已永久停用：强制 autoHide = false，不再读取 widget.auto_hide / widget.mode 旧配置
      autoHide.value = false;

      const top = await loadWidgetConfig("always_on_top");
      if (top === "false") {
        alwaysOnTop.value = false;
        await setAlwaysOnTop(false);
      }

      const lock = await loadWidgetConfig("auto_lock_minutes");
      if (lock) autoLockMinutes.value = Math.max(1, parseInt(lock) || 5);

      const clip = await loadWidgetConfig("clipboard_clear_seconds");
      if (clip) clipboardClearSeconds.value = Math.max(5, parseInt(clip) || 15);

      const temp = await loadWidgetConfig("temp_expire_days");
      if (temp) tempExpireDays.value = Math.max(1, parseInt(temp) || 3);

      // 修复 P0：不再持久化 mini_mode 跨启动
      // 原因：用户用 Alt+Q 弹窗时希望看到完整版抽屉柜，而不是上次遗留的迷你模式
      // 迷你模式仍可在本次会话内生效（点 ◂ 切换），但重启后默认完整
      // const mini = await loadWidgetConfig("mini_mode");
      // if (mini === "true") miniMode.value = true;

      const ct = await loadWidgetConfig("click_through");
      if (ct === "true") clickThrough.value = true;
    } catch (e) {
      console.warn("加载 widget 配置失败", e);
    } finally {
      // 不再同步 autoHide 到 Rust（钩子已禁用），只读一次 dock 状态用于兼容旧界面显示
      try {
        const hidden = await isDockHidden();
        edgeHidden.value = hidden;
      } catch (e) {
        console.warn("同步 dock 状态失败", e);
      }
      ready.value = true;
    }
  }

  // ===== 边缘轮询保底 =====
  // 显示时：每 2 秒检查位置是否在边缘，贴边就触发自动隐藏
  // 隐藏时：每 200ms 加速轮询，配合 cursorPosition 监测鼠标是否在触角附近（mousemove 在小触角里不一定触发）
  // 修复 P0-#X：startEdgePolling 默认不启动（旧代码模块加载时自动注册 mousemove，永不清理）
  // 现在仅在 onAutoHideChanged / 实际需要时启动
  let edgePollingTimer: number | null = null;
  function startEdgePolling() {
    if (edgePollingTimer) return;
    // 修复 P0-#X：按需 attach mousemove（仅在轮询启动时注册，停止时移除）
    if (!mouseMoveHandler) {
      mouseMoveHandler = trackMouse;
      document.addEventListener("mousemove", mouseMoveHandler, { passive: true });
    }
    edgePollingTimer = window.setInterval(async () => {
      if (!autoHide.value) return;
      try {
        if (!edgeHidden.value) {
          // === 显示状态：检查是否需要隐藏 ===
          const [x, y] = await getWindowPosition();
          const [w, h] = await getWindowSize();
          const screenW = window.screen.width;
          const screenH = window.screen.height;
          const threshold = 20;
          const right = x + w;
          const bottom = y + h;
          const nearRight = right > screenW - threshold;
          const nearLeft = x < threshold;
          const nearTop = y < threshold;
          const nearBottom = bottom > screenH - threshold;
          const nearEdge = nearRight || nearLeft || nearTop || nearBottom;
          if (nearEdge) {
            scheduleAutoHide();
          } else {
            cancelAutoHide();
          }
        } else {
          // === 隐藏状态：每 200ms 检查鼠标是否在触角附近，触发了就唤回 ===
          const screenW = window.screen.width;
          const screenH = window.screen.height;
          const mouseX = lastMouseX;
          const mouseY = lastMouseY;
          const nearEdge =
            mouseX <= AUTO_REVEAL_DIST ||
            mouseX >= screenW - AUTO_REVEAL_DIST ||
            mouseY <= AUTO_REVEAL_DIST ||
            mouseY >= screenH - AUTO_REVEAL_DIST;
          if (nearEdge) {
            performAutoReveal();
          }
        }
      } catch (e) {
        // ignore
      }
    }, 200);
  }
  function stopEdgePolling() {
    if (edgePollingTimer) {
      clearInterval(edgePollingTimer);
      edgePollingTimer = null;
    }
    // 修复 P0-#X：detach mousemove（轮询停止时一并清理）
    if (mouseMoveHandler) {
      document.removeEventListener("mousemove", mouseMoveHandler);
      mouseMoveHandler = null;
    }
  }
  // 记录鼠标位置（仅在 startEdgePolling 启动后才注册 mousemove）
  // 修复 P0-#X：删除模块加载时的 `document.addEventListener("mousemove", trackMouse)`
  // 原因：那个监听器永远不会被移除，每次 mousemove 都会触发回调，拖动期尤为密集
  // 现在由 startEdgePolling / stopEdgePolling 按需 attach/detach mousemove
  let lastMouseX = 0;
  let lastMouseY = 0;
  let mouseMoveHandler: ((e: MouseEvent) => void) | null = null;
  function trackMouse(e: MouseEvent) {
    lastMouseX = e.clientX;
    lastMouseY = e.clientY;
  }

  // ===== 修改并持久化 =====
  /** 已停用：贴边自动收起 Win32 钩子存在卡死问题，此函数保留仅作向后兼容空操作 */
  async function setAutoHide(_v: boolean) {
    console.warn("[widgetStore] setAutoHide 已停用，功能 v2.0 重写后回归");
    autoHide.value = false;
  }

  async function setEdgeHidden(v: boolean) {
    edgeHidden.value = v;
  }

  async function toggleAlwaysOnTop() {
    alwaysOnTop.value = !alwaysOnTop.value;
    await setAlwaysOnTop(alwaysOnTop.value);
    await saveWidgetConfig("always_on_top", String(alwaysOnTop.value));
  }

  async function setAutoLockMinutes(m: number) {
    autoLockMinutes.value = m;
    await saveWidgetConfig("auto_lock_minutes", String(m));
  }

  async function setClipboardClearSeconds(s: number) {
    clipboardClearSeconds.value = s;
    await saveWidgetConfig("clipboard_clear_seconds", String(s));
  }

  async function setTempExpireDays(d: number) {
    tempExpireDays.value = d;
    await saveWidgetConfig("temp_expire_days", String(d));
  }

  /**
   * 切换迷你模式（极简图标列）
   * - 切到迷你：56×320 垂直条
   * - 切回完整：380×560
   */
  async function toggleMiniMode() {
    const next = !miniMode.value;
    miniMode.value = next;
    await setMiniModeWithPos(next);
    // 注意：mini_mode 已不再持久化（init() 里不读取），启动默认永远是完整版，保证 Alt+Q 弹出就是完整卡片
    // 迷你模式仅在本次会话内生效，重启后归位
    if (next) {
      clickThrough.value = false; // 关闭全局穿透，由前端按区域控制
    }
  }

  /**
   * 切换鼠标穿透（全局模式，仅适用于迷你模式之外的实验性场景）
   */
  async function toggleClickThrough() {
    const next = !clickThrough.value;
    clickThrough.value = next;
    await setIgnoreCursorEvents(next);
    await saveWidgetConfig("click_through", String(next));
  }

  // 导入备份后整体刷新：重新读取 settings widget.* 配置（包括 autoLock/tempExpire/alwaysOnTop 等全局偏好）
  async function reloadAll() {
    ready.value = false;
    await init();
  }

  /**
   * 按区域动态切换穿透：
   * - 修复 P1-#8：之前的判断反了。正确语义：
   *   - 用户开启了"穿透模式"（clickThrough = true）才需要动态切换
   *   - hovering=true（鼠标在 widget 实体上）：必须收事件 → setIgnoreCursorEvents(false)
   *   - hovering=false（鼠标在 widget 透明区域）：事件穿透到桌面 → setIgnoreCursorEvents(true)
   *   - 如果用户没开穿透模式（clickThrough = false），永远不忽略事件
   */
  async function setClickThroughForHover(hovering: boolean) {
    if (!clickThrough.value) return; // 没开穿透模式，啥也不做
    await setIgnoreCursorEvents(!hovering); // hover 实体时关闭穿透，否则开启穿透
  }

  // ===== 窗口位置/大小持久化 =====
  // 修复 P0-#X：保存策略调整
  // 之前：scheduleSavePosition 300ms debounce，连续触发会被合并
  //   问题：在拖动结束时，scheduleSavePosition 可能仍处于 debounce 等待中，
  //   导致关闭应用前最后位置未保存
  // 修复：拆成两个方法
  //   - scheduleSavePosition: 300ms debounce（用于普通场景，如 Alt+Q 等）
  //   - savePositionNow: 立即保存一次（用于拖动结束、关闭前）
  let saveTimer: number | null = null;
  function scheduleSavePosition() {
    if (saveTimer) clearTimeout(saveTimer);
    saveTimer = window.setTimeout(() => {
      saveTimer = null;
      void doSavePosition();
    }, 300);
  }
  /**
   * 立即保存位置/大小（不做 debounce）
   * 使用场景：拖动结束、Alt+L 锁定、关闭应用前
   */
  async function savePositionNow() {
    if (saveTimer) {
      clearTimeout(saveTimer);
      saveTimer = null;
    }
    await doSavePosition();
  }
  async function doSavePosition() {
    try {
      const [x, y] = await getWindowPosition();
      const [w, h] = await getWindowSize();
      await saveWidgetConfig("x", String(x));
      await saveWidgetConfig("y", String(y));
      await saveWidgetConfig("width", String(w));
      await saveWidgetConfig("height", String(h));
    } catch (e) {
      console.warn("保存位置失败", e);
    }
  }

  // 应用启动后自动恢复位置（仅用作手动恢复，setup hook 已处理）
  async function restorePosition() {
    const [x, y] = await getWindowPosition();
    return { x, y };
  }

  async function moveTo(x: number, y: number) {
    await setWindowPosition(x, y);
  }

  /**
   * 边缘吸附：拖拽结束后调用，若窗口靠近屏幕边缘 20px 则吸附到边缘
   */
  async function snapToEdge() {
    try {
      const [x, y] = await getWindowPosition();
      const [w, h] = await getWindowSize();
      const screenW = window.screen.width;
      const screenH = window.screen.height;
      const threshold = 20;
      let nx = x;
      let ny = y;
      let snapped = false;
      // 左
      if (x < threshold) {
        nx = 0;
        snapped = true;
      }
      // 右
      else if (x + w > screenW - threshold) {
        nx = screenW - w;
        snapped = true;
      }
      // 上
      if (y < threshold) {
        ny = 0;
        snapped = true;
      }
      // 下
      else if (y + h > screenH - threshold) {
        ny = screenH - h;
        snapped = true;
      }
      if (snapped) {
        await setWindowPosition(nx, ny);
        // 吸附到边缘后，启动自动隐藏倒计时
        if (autoHide.value) {
          scheduleAutoHide();
        }
      }
    } catch (e) {
      console.warn("边缘吸附失败", e);
    }
  }

  /**
   * 通用边缘检测：每次窗口移动后调用，若在边缘 20px 内且 autoHide 开启，启动倒计时
   * 这是比 snapToEdge 更通用的入口：不论是拖动、预设、还是其他方式移动到边缘都能触发
   */
  let edgeCheckTimer: number | null = null;
  function scheduleCheckEdgeProximity() {
    if (!autoHide.value) return;
    if (edgeCheckTimer) clearTimeout(edgeCheckTimer);
    edgeCheckTimer = window.setTimeout(async () => {
      try {
        const [x, y] = await getWindowPosition();
        const [w, h] = await getWindowSize();
        const screenW = window.screen.width;
        const screenH = window.screen.height;
        const threshold = 20;
        const nearLeft = x < threshold;
        const nearRight = x + w > screenW - threshold;
        const nearTop = y < threshold;
        const nearBottom = y + h > screenH - threshold;
        if (nearLeft || nearRight || nearTop || nearBottom) {
          scheduleAutoHide();
        } else {
          // 离开边缘 → 取消倒计时，并恢复已隐藏的窗口
          cancelAutoHide();
          if (edgeHidden.value) {
            performAutoReveal();
          }
        }
      } catch (e) {
        console.warn("边缘检测失败", e);
      }
    }, 200);
  }

  // ===== 边缘自动隐藏 =====
  // 修复 P0-#X：所有边缘检测 / 隐藏动画 / 触角恢复 **完全交给 Rust 端 win_dock.rs**
  // 之前前端：
  //   - tweenPosition 用 rAF 每帧（16ms）调 setWindowPosition → 拖动到边缘时 IPC 风暴
  //   - startEdgePolling 200ms 轮询 getWindowPosition / getWindowSize → 持续 IPC
  //   - startEdgeProximityWatch 模块加载时永久监听 mousemove → 永不清理
  //   - scheduleCheckEdgeProximity 拖动结束再打 4 个 IPC
  // 叠加 Rust 端 WndProc 自身补间 → "未响应" 根因
  // 现在：前端不启动任何轮询、不做动画、不监听 mousemove（仅 Rust 端 WndProc 工作）
  // 公开 API 保留空实现 / 占位，避免调用方（如 MainLayout）报错

  // ===== 占位实现（修复 P0-#X：之前 return 引用了未定义的函数，导致 store 创建抛 ReferenceError → 黑屏）=====
  // 实际自动隐藏逻辑已完全移交 Rust 端 win_dock.rs；这里只保留空实现，保证调用方不报错
  const AUTO_REVEAL_DIST = 6; // 触角宽度（px），保留常量避免编译报错
  let _autoHideTimer: number | null = null;
  async function scheduleAutoHide() {
    // noop: 实际倒计时由 Rust 端 WndProc 通过 PostMessageW 实现
    if (_autoHideTimer) {
      clearTimeout(_autoHideTimer);
      _autoHideTimer = null;
    }
  }
  async function cancelAutoHide() {
    if (_autoHideTimer) {
      clearTimeout(_autoHideTimer);
      _autoHideTimer = null;
    }
  }
  async function performAutoHide() {
    // noop: 实际隐藏由 Rust 端 WndProc 处理
  }
  async function performAutoReveal() {
    // noop: 实际唤回由 Rust 端 WndProc 处理
  }
  function startEdgeProximityWatch() {
    // noop: 之前此函数会注册永不清理的 mousemove，触发未响应，已禁用
  }
  function stopEdgeProximityWatch() {
    // noop
  }
  // 把内部可能引用到 performAutoReveal 的占位绑定到本作用域（之前是裸引用，会触发 ReferenceError）
  void performAutoReveal;
  void scheduleAutoHide;
  void cancelAutoHide;
  void AUTO_REVEAL_DIST;

  // ===== 预设位置 =====
  type Preset = "center" | "top-left" | "top-right" | "bottom-left" | "bottom-right" | "remember";
  const PRESET_MARGIN = 24;

  async function applyPresetPosition(preset: Preset) {
    try {
      const [w, h] = await getWindowSize();
      const sw = window.screen.width;
      const sh = window.screen.height;
      let targetX = 0;
      let targetY = 0;
      switch (preset) {
        case "center":
          targetX = Math.round((sw - w) / 2);
          targetY = Math.round((sh - h) / 2);
          break;
        case "top-left":
          targetX = PRESET_MARGIN;
          targetY = PRESET_MARGIN;
          break;
        case "top-right":
          targetX = sw - w - PRESET_MARGIN;
          targetY = PRESET_MARGIN;
          break;
        case "bottom-left":
          targetX = PRESET_MARGIN;
          targetY = sh - h - PRESET_MARGIN;
          break;
        case "bottom-right":
          targetX = sw - w - PRESET_MARGIN;
          targetY = sh - h - PRESET_MARGIN;
          break;
        case "remember":
          try {
            const [x, y] = await getWindowPosition();
            const [w, h] = await getWindowSize();
            await saveWidgetConfig("x", String(x));
            await saveWidgetConfig("y", String(y));
            await saveWidgetConfig("width", String(w));
            await saveWidgetConfig("height", String(h));
          } catch (e) {
            console.warn("[applyPresetPosition] remember 持久化失败", e);
          }
          return;
      }
      // 修复 P0-#X：tweenPosition 会 rAF 每帧 IPC。
      // 现在直接 setWindowPosition 一次到位（无动画），让 Rust 端 SetTimer 接管后续贴边自动隐藏
      await setWindowPosition(targetX, targetY);
      await saveWidgetConfig("x", String(targetX));
      await saveWidgetConfig("y", String(targetY));
      // 注意：不再调 scheduleCheckEdgeProximity —— Rust 端 WndProc 自己会监听 WM_WINDOWPOSCHANGING
    } catch (e) {
      console.warn("应用预设位置失败", e);
    }
  }

  /**
   * 强制切回完整模式（380×560）
   * 使用场景：Alt+Q 快捷键弹窗时调用，避免上次的迷你模式被记住
   * 修复：之前 Alt+Q → win.show() 不改尺寸，如果 miniMode=true 就还是 56×320 简单版
   */
  async function forceExpand() {
    if (miniMode.value) {
      miniMode.value = false;
      // 不再持久化 mini_mode，下次启动默认完整
      // 用户在本会话内可自由切换迷你模式，重启后归位
    }
    // 总是强制调后端 setSize 恢复完整模式尺寸（后端尺寸可能因自动隐藏等被改过）
    try {
      await setMiniModeWithPos(false);
    } catch {}
  }

  return {
    autoHide,
    edgeHidden,
    alwaysOnTop,
    autoLockMinutes,
    clipboardClearSeconds,
    tempExpireDays,
    miniMode,
    clickThrough,
    ready,
    init,
    reloadAll,
    setAutoHide,
    setEdgeHidden,
    toggleAlwaysOnTop,
    setAutoLockMinutes,
    setClipboardClearSeconds,
    setTempExpireDays,
    toggleMiniMode,
    toggleClickThrough,
    setClickThroughForHover,
    scheduleSavePosition,
    savePositionNow,
    restorePosition,
    moveTo,
    snapToEdge,
    scheduleCheckEdgeProximity,
    performAutoHide,
    performAutoReveal,
    startEdgeProximityWatch,
    stopEdgeProximityWatch,
    startEdgePolling,
    stopEdgePolling,
    applyPresetPosition,
    forceExpand,
  };
});

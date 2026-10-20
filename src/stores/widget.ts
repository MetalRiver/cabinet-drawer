import { defineStore } from "pinia";
import { ref, computed } from "vue";
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
import { useAppStore } from "./app";

export type DockSide = "top" | "bottom" | "left" | "right";

/**
 * v0.4.1 修复（用户反馈 P0）：设置值解析统一入口。
 * - 0 是合法设置值（永不锁定 / 永不过期 / 永久保留），必须原样保留；
 *   旧实现 `Math.max(1, parseInt(v) || 默认)` 把 0 当 falsy 改写成默认值，
 *   导致"行为偏好修改后重启恢复原设定"。
 * - 缺失 / 非数字 / 负数 → 回退默认值。
 */
function parseSetting(raw: string | null | undefined, fallback: number): number {
  if (raw === null || raw === undefined) return fallback;
  const trimmed = raw.trim();
  if (trimmed === "") return fallback;
  const n = Number(trimmed);
  if (!Number.isFinite(n) || n < 0) return fallback;
  return Math.round(n);
}

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
      // v0.4.1 修复：双向同步。旧实现只在存值 "false" 时同步，
      // 置顶=true 时 UI 停在默认 false → 重启后实际置顶生效但界面显示"不置顶"，
      // 用户误以为设置没有保存。
      if (top !== null && top !== undefined) {
        alwaysOnTop.value = top === "true";
        await setAlwaysOnTop(alwaysOnTop.value);
      }

      const lock = await loadWidgetConfig("auto_lock_minutes");
      // 0 = 永不锁定，是合法值，必须保留（v0.4.1）
      autoLockMinutes.value = parseSetting(lock, 5);

      const clip = await loadWidgetConfig("clipboard_clear_seconds");
      // 剪贴板清除无 0（永不）选项；0/非法值回退默认 15，其余下限 5 秒
      const clipParsed = parseSetting(clip, 15);
      clipboardClearSeconds.value = clipParsed === 0 ? 15 : Math.max(5, clipParsed);

      const temp = await loadWidgetConfig("temp_expire_days");
      // 0 = 永不过期，是合法值，必须保留（v0.4.1）
      tempExpireDays.value = parseSetting(temp, 3);

      // 修复 P0：不再持久化 mini_mode 跨启动
      // 原因：用户用 Alt+Q 弹窗时希望看到完整版抽屉柜，而不是上次遗留的迷你模式
      // 迷你模式仍可在本次会话内生效（点 ◂ 切换），但重启后默认完整
      // const mini = await loadWidgetConfig("mini_mode");
      // if (mini === "true") miniMode.value = true;

      const ct = await loadWidgetConfig("click_through");
      if (ct === "true") clickThrough.value = true;

      // 记忆位置（独立坐标，Settings UI V2）
      await loadMemoryPosition();
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
  /**
   * v0.4.1 修复：保存失败必须可见（用户反馈"不能假装保存成功"）。
   * 旧实现保存异常被静默吞掉，用户以为保存成功。
   */
  function reportSaveError(context: string, e: unknown) {
    console.error(`[widgetStore] 保存设置失败（${context}）`, e);
    try {
      useAppStore().showClipToast("info", `⚠ 设置保存失败（${context}）：${e}`);
    } catch {
      /* toast 通道不可用时至少 console 有记录 */
    }
  }

  // ===== R5：持久化写队列（每 key 串行）=====
  // 不假设 watch 异步回调与 Tauri IPC 天然串行：同步命令可能被线程池乱序执行，
  // 旧值可能在"新值之后"落库。每 key 维护一条 Promise 链——按调用顺序入队，
  // 后选择的值一定最后落盘；旧写入失败只可见报错，不回滚、不阻断新值写入。
  const saveQueues = new Map<string, Promise<void>>();
  function enqueueSave(key: string, value: string, label: string) {
    const prev = saveQueues.get(key) ?? Promise.resolve();
    const next = prev.then(async () => {
      try {
        await saveWidgetConfig(key, value);
      } catch (e) {
        reportSaveError(label, e);
      }
    });
    // rejection 已在上面可见化，这里吞掉防止 unhandled rejection
    saveQueues.set(key, next.catch(() => {}));
  }
  /** 仅供隔离测试使用：清空保存队列 */
  function _testResetSaveQueues() {
    saveQueues.clear();
  }

  /** 已停用：贴边自动收起 Win32 钩子存在卡死问题，此函数保留仅作向后兼容空操作 */
  async function setAutoHide(_v: boolean) {
    console.warn("[widgetStore] setAutoHide 已停用，功能 v2.0 重写后回归");
    autoHide.value = false;
  }

  async function setEdgeHidden(v: boolean) {
    edgeHidden.value = v;
  }

  async function toggleAlwaysOnTop() {
    const next = !alwaysOnTop.value;
    alwaysOnTop.value = next;
    try {
      await setAlwaysOnTop(next);
      enqueueSave("always_on_top", String(next), "窗口置顶");
    } catch (e) {
      // 回滚 UI 与窗口实际状态，不假装成功
      alwaysOnTop.value = !next;
      try { await setAlwaysOnTop(!next); } catch { /* 回滚失败仅留日志 */ }
      reportSaveError("窗口置顶", e);
    }
  }

  async function setAutoLockMinutes(m: number) {
    autoLockMinutes.value = m;
    enqueueSave("auto_lock_minutes", String(m), "闲置自动锁定");
  }

  async function setClipboardClearSeconds(s: number) {
    clipboardClearSeconds.value = s;
    enqueueSave("clipboard_clear_seconds", String(s), "剪贴板自动清除");
  }

  async function setTempExpireDays(d: number) {
    tempExpireDays.value = d;
    enqueueSave("temp_expire_days", String(d), "便签默认过期");
  }

  // ===== R3：几何隔离 =====
  // 迷你/完整两套几何（完整 = widget.x/y/width/height，迷你 = widget.mini_x/mini_y）。
  // 切换期间抑制几何保存：tauri://move、resize、拖拽结束等事件不得污染几何快照。
  const geometryTransition = ref(false);
  let geometryTransitionTimer: number | null = null;
  function beginGeometryTransition() {
    geometryTransition.value = true;
    if (geometryTransitionTimer) clearTimeout(geometryTransitionTimer);
    geometryTransitionTimer = window.setTimeout(() => {
      geometryTransition.value = false;
    }, 800); // > MainLayout 500ms move-debounce 窗口，留余量
  }

  /**
   * 切换迷你模式（极简图标列；R3 几何隔离）
   * - 迷你：后端解除 min 约束 + 逻辑 64×360 + 独立迷你位置；完整几何快照不动
   * - 完整：后端恢复 min 约束 + 用户原有完整几何（保存过才设置，缺省保持当前值）
   * - mini_mode 不持久化：重启/Alt+Q 呼出恒为完整模式（产品约定），完整几何由
   *   apply_widget_config 在启动时恢复
   */
  async function toggleMiniMode() {
    const next = !miniMode.value;
    miniMode.value = next;
    beginGeometryTransition();
    try {
      await setMiniModeWithPos(next);
    } catch (e) {
      console.warn("[widgetStore] 迷你模式切换失败", e);
    } finally {
      if (next) {
        clickThrough.value = false; // 关闭全局穿透，由前端按区域控制
      }
    }
  }

  /**
   * 切换鼠标穿透（全局模式，仅适用于迷你模式之外的实验性场景）
   */
  async function toggleClickThrough() {
    const next = !clickThrough.value;
    clickThrough.value = next;
    await setIgnoreCursorEvents(next);
    enqueueSave("click_through", String(next), "鼠标穿透");
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
    // R3：迷你/完整切换期间不保存——切换过程的 move/resize 事件不得污染几何快照
    if (geometryTransition.value) return;
    try {
      const [x, y] = await getWindowPosition();
      if (miniMode.value) {
        // R3：迷你态只记独立迷你位置（widget.mini_x/mini_y），
        // 尺寸固定 64×360（逻辑px）无需保存，完整几何快照保持不变
        enqueueSave("mini_x", String(x), "迷你位置");
        enqueueSave("mini_y", String(y), "迷你位置");
      } else {
        const [w, h] = await getWindowSize();
        enqueueSave("x", String(x), "窗口位置");
        enqueueSave("y", String(y), "窗口位置");
        enqueueSave("width", String(w), "窗口尺寸");
        enqueueSave("height", String(h), "窗口尺寸");
      }
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

  // ===== 预设位置（Settings UI V2：完整九宫格，全部为纯位置动作，不改尺寸）=====
  type Preset =
    | "top-left" | "top-center" | "top-right"
    | "middle-left" | "center" | "middle-right"
    | "bottom-left" | "bottom-center" | "bottom-right";
  const PRESET_MARGIN = 24;

  /**
   * 应用九宫格预设位置。
   * - 基于 Windows 显示器**工作区域**（availWidth/availHeight，已扣除任务栏）；
   * - CSS 逻辑像素 × devicePixelRatio → 物理像素，与 set_window_position
   *   的 PhysicalPosition 对齐（125%/150% 缩放下不再偏移）；
   * - 绝不修改窗口尺寸（历史缺陷：旧"重置到右上角/居中"会 setWindowSize 重置尺寸）；
   * - 成功后同步持久化 x/y（完整几何快照的一部分，与记忆坐标无关）。
   */
  async function applyPresetPosition(preset: Preset): Promise<{ x: number; y: number }> {
    const [w, h] = await getWindowSize();
    // 工作区域（逻辑 px）：优先 availWidth/availHeight（扣除任务栏），异常回退全屏
    const aw = window.screen.availWidth || window.screen.width;
    const ah = window.screen.availHeight || window.screen.height;
    let lx = 0;
    let ly = 0;
    switch (preset) {
      case "center":
        lx = Math.round((aw - w) / 2);
        ly = Math.round((ah - h) / 2);
        break;
      case "top-left":      lx = PRESET_MARGIN;             ly = PRESET_MARGIN;             break;
      case "top-center":    lx = Math.round((aw - w) / 2);  ly = PRESET_MARGIN;             break;
      case "top-right":     lx = aw - w - PRESET_MARGIN;    ly = PRESET_MARGIN;             break;
      case "middle-left":   lx = PRESET_MARGIN;             ly = Math.round((ah - h) / 2);  break;
      case "middle-right":  lx = aw - w - PRESET_MARGIN;    ly = Math.round((ah - h) / 2);  break;
      case "bottom-left":   lx = PRESET_MARGIN;             ly = ah - h - PRESET_MARGIN;    break;
      case "bottom-center": lx = Math.round((aw - w) / 2);  ly = ah - h - PRESET_MARGIN;    break;
      case "bottom-right":  lx = aw - w - PRESET_MARGIN;    ly = ah - h - PRESET_MARGIN;    break;
    }
    // 边界保护：负值/越界拉回工作区内
    lx = Math.max(0, Math.min(lx, Math.max(0, aw - w)));
    ly = Math.max(0, Math.min(ly, Math.max(0, ah - h)));
    // 逻辑 → 物理（set_window_position 是 PhysicalPosition）
    const dpr = window.devicePixelRatio || 1;
    const px = Math.round(lx * dpr);
    const py = Math.round(ly * dpr);
    await setWindowPosition(px, py);
    enqueueSave("x", String(px), "窗口位置");
    enqueueSave("y", String(py), "窗口位置");
    return { x: px, y: py };
  }

  // ===== 记忆位置（Settings UI V2：独立坐标，不与自动几何快照混用）=====
  // 保存当前位置 = 用户显式动作，写入 widget.mem_x/mem_y；
  // 回到记忆位置 = 仅移动窗口（不改尺寸），未保存过时可见提示。
  const memoryX = ref<number | null>(null);
  const memoryY = ref<number | null>(null);
  const memoryLabel = computed(() =>
    memoryX.value === null || memoryY.value === null ? "未保存" : `已记忆 ${memoryX.value}, ${memoryY.value}`
  );

  async function loadMemoryPosition() {
    try {
      const [x, y] = await Promise.all([
        loadWidgetConfig("mem_x"),
        loadWidgetConfig("mem_y"),
      ]);
      memoryX.value = x === null || x === undefined ? null : Number(x);
      memoryY.value = y === null || y === undefined ? null : Number(y);
      if (!Number.isFinite(memoryX.value!) || !Number.isFinite(memoryY.value!)) {
        memoryX.value = null;
        memoryY.value = null;
      }
    } catch (e) {
      console.warn("读取记忆位置失败", e);
    }
  }

  async function saveMemoryPosition(): Promise<void> {
    const [x, y] = await getWindowPosition();
    enqueueSave("mem_x", String(x), "记忆位置");
    enqueueSave("mem_y", String(y), "记忆位置");
    memoryX.value = x;
    memoryY.value = y;
  }

  async function restoreMemoryPosition(): Promise<void> {
    if (memoryX.value === null || memoryY.value === null) {
      throw new Error("还没有保存过记忆位置：先点「保存当前位置」");
    }
    // 仅移动位置，不改窗口尺寸
    await setWindowPosition(memoryX.value, memoryY.value);
  }

  /**
   * Alt+Q 唤出时的完整模式恢复动作（widget:force-expand 事件驱动）。
   * v0.4.3 产品决策：Alt+Q 只显示/隐藏**当前模式**，不自动切换完整/迷你
   * → 后端 Alt+Q 已不再 emit 本事件，此函数当前**无触发方**（inert）。
   * 保留原因：恢复/迁移流程复用 + 语义已被单测钉死
   * （widget-force-expand.test.ts），删除与否待 Batch 2 一并裁剪。
   * mini_mode 不持久化：重启后恒为完整模式（产品约定）。
   */
  async function forceExpand() {
    if (!miniMode.value) return;
    miniMode.value = false;
    // 恢复 min 约束 + 用户原有完整几何（后端处理），切换期间抑制几何保存
    beginGeometryTransition();
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
    _testResetSaveQueues,
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
    memoryX,
    memoryY,
    memoryLabel,
    loadMemoryPosition,
    saveMemoryPosition,
    restoreMemoryPosition,
  };
});

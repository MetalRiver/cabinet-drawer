// @vitest-environment jsdom
// ===== R5（D-Repair）：widget store 设置持久化写队列竞态隔离测试 =====
//
// 目标：证明"用户快速连续修改设置"时，最终持久化值 = 最后一次有效选择；
// 且较早的写入失败/延迟完成不会错误回滚或覆盖较新的选择。
// 方式：mock Tauri IPC（../api），用可控制的延迟/失败注入验证 enqueueSave 的
// per-key 串行 Promise 链语义。

import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { createPinia, setActivePinia } from "pinia";

// ---- mock ../api：只提供 widget.ts 实际用到的函数 ----
type SaveCall = { key: string; value: string };
const saveCalls: SaveCall[] = [];
/** key → 注入的延迟(ms) / 是否失败（按调用序消费） */
const scripted: Record<
  string,
  { delay?: number; fail?: boolean }[]
> = {};

vi.mock("../../api", () => ({
  loadWidgetConfig: vi.fn(async () => null),
  saveWidgetConfig: vi.fn(async (key: string, value: string) => {
    const script = scripted[key]?.shift() ?? {};
    if (script.delay) await new Promise((r) => setTimeout(r, script.delay));
    saveCalls.push({ key, value });
    if (script.fail) throw new Error(`IPC_FAIL(${key})`);
  }),
  setAlwaysOnTop: vi.fn(async () => {}),
  setIgnoreCursorEvents: vi.fn(async () => {}),
  setMiniModeWithPos: vi.fn(async () => {}),
  setWindowPosition: vi.fn(async () => {}),
  getWindowPosition: vi.fn(async () => [0, 0] as [number, number]),
  getWindowSize: vi.fn(async () => [480, 720] as [number, number]),
  isDockHidden: vi.fn(async () => false),
}));

import { useWidgetStore } from "../widget";

function lastSaved(key: string): string | undefined {
  for (let i = saveCalls.length - 1; i >= 0; i--) {
    if (saveCalls[i].key === key) return saveCalls[i].value;
  }
  return undefined;
}

/** 等待某 key 的写队列清空（队列尾 promise 存在于 store 内部，这里轮询静默） */
async function flushQueue() {
  for (let i = 0; i < 50; i++) {
    await new Promise((r) => setTimeout(r, 10));
  }
}

describe("widget store 设置持久化（R5 写队列）", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    saveCalls.length = 0;
    for (const k of Object.keys(scripted)) delete scripted[k];
    vi.spyOn(console, "error").mockImplementation(() => {});
  });
  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("快速连续修改：两次写入按调用顺序落盘，最终值 = 最后一次选择", async () => {
    const store = useWidgetStore();
    store._testResetSaveQueues();
    // 第一次写入慢（500ms），第二次立即
    scripted["auto_lock_minutes"] = [{ delay: 500 }, {}];
    await store.setAutoLockMinutes(10);
    await store.setAutoLockMinutes(30);
    await flushQueue();
    expect(saveCalls.map((c) => c.value)).toEqual(["10", "30"]);
    expect(lastSaved("auto_lock_minutes")).toBe("30");
  });

  it("较早的写入失败：可见报错但不回滚、不阻断较新的选择", async () => {
    const store = useWidgetStore();
    store._testResetSaveQueues();
    scripted["temp_expire_days"] = [{ fail: true }, {}];
    await store.setTempExpireDays(7);
    await store.setTempExpireDays(1);
    await flushQueue();
    // 两次都尝试落盘，顺序保持；最终持久化值 = 新选择
    expect(saveCalls.map((c) => c.value)).toEqual(["7", "1"]);
    expect(lastSaved("temp_expire_days")).toBe("1");
    // 失败可见（appStore toast），而非静默
    const { useAppStore } = await import("../app");
    const app = useAppStore();
    expect(app.clipToast?.text).toContain("设置保存失败");
    expect(app.clipToast?.text).toContain("IPC_FAIL(temp_expire_days)");
  });

  it("慢的旧写入完成晚也不覆盖新值（串行链保证顺序）", async () => {
    const store = useWidgetStore();
    store._testResetSaveQueues();
    scripted["clipboard_clear_seconds"] = [{ delay: 300 }, { delay: 0 }];
    await store.setClipboardClearSeconds(60);
    await store.setClipboardClearSeconds(15);
    await flushQueue();
    expect(saveCalls.map((c) => c.value)).toEqual(["60", "15"]);
    expect(lastSaved("clipboard_clear_seconds")).toBe("15");
  });

  it("迷你/完整切换窗口内几何保存被抑制，窗口期后恢复（R3 放行检查）", async () => {
    vi.useFakeTimers();
    try {
      const store = useWidgetStore();
      store._testResetSaveQueues();
      await store.toggleMiniMode(); // 进入迷你（后端 mock 立即完成）
      store.savePositionNow(); // 切换窗口期内的 move / resize / 拖拽结束事件
      expect(saveCalls.length).toBe(0); // 必须被抑制：不得污染几何快照
      await store.toggleMiniMode(); // 退出迷你（仍在 800ms 抑制窗口内）
      store.savePositionNow();
      expect(saveCalls.length).toBe(0); // 仍被抑制
      vi.advanceTimersByTime(800);
      await vi.runAllTimersAsync(); // 解除抑制定时器 + 写队列微任务
      // 此时已回到完整模式且抑制解除 → 正常保存完整几何
      store.savePositionNow();
      await vi.runAllTimersAsync();
      const keys = saveCalls.map((c) => c.key);
      expect(keys).toContain("x");
      expect(keys).toContain("y");
      expect(keys).toContain("width");
      expect(keys).toContain("height");
      expect(lastSaved("width")).toBe("480");
    } finally {
      vi.useRealTimers();
    }
  });

  it("0 是合法设置值：setTempExpireDays(0) 原样入队持久化", async () => {
    const store = useWidgetStore();
    store._testResetSaveQueues();
    await store.setTempExpireDays(0);
    await flushQueue();
    expect(lastSaved("temp_expire_days")).toBe("0");
    expect(store.tempExpireDays).toBe(0);
  });
});

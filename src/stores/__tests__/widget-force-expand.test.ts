// @vitest-environment jsdom
// ===== v0.4.3 Batch 1-A：forceExpand 语义回归 =====
// - 完整态：force-expand 事件必须零动作（不再无条件 setMiniModeWithPos(false)，
//   否则会用已保存几何回滚用户刚做的纯 resize，并产生错误尺寸闪跳窗口）
// - 迷你态：清 miniMode 标记 + 恢复完整几何（后端 setMiniModeWithPos(false)）
// - 迷你态切换期间 geometryTransition 生效（doSavePosition 首行守卫依赖）

import { describe, it, expect, vi, beforeEach } from "vitest";
import { createPinia, setActivePinia } from "pinia";

const apiCalls: string[] = [];

vi.mock("../../api", () => ({
  loadWidgetConfig: vi.fn(async () => null),
  saveWidgetConfig: vi.fn(async () => {}),
  setWindowPosition: vi.fn(async () => {}),
  setWindowSize: vi.fn(async () => {}),
  getWindowPosition: vi.fn(async () => [100, 100] as [number, number]),
  getWindowSize: vi.fn(async () => [480, 720] as [number, number]),
  setAlwaysOnTop: vi.fn(async () => {}),
  setIgnoreCursorEvents: vi.fn(async () => {}),
  setMiniModeWithPos: vi.fn(async (_mini: boolean) => {
    apiCalls.push(`setMiniModeWithPos:${_mini}`);
  }),
  isDockHidden: vi.fn(async () => false),
}));

import { useWidgetStore } from "../widget";
import { setMiniModeWithPos } from "../../api";

beforeEach(() => {
  setActivePinia(createPinia());
  apiCalls.length = 0;
  vi.clearAllMocks();
  vi.spyOn(console, "warn").mockImplementation(() => {});
  vi.spyOn(console, "error").mockImplementation(() => {});
});

async function flush() {
  for (let i = 0; i < 5; i++) await Promise.resolve();
}

describe("forceExpand（Alt+Q 唤出语义，v0.4.3 A1）", () => {
  it("完整态：零动作，不触发任何几何 IPC", async () => {
    const store = useWidgetStore();
    expect(store.miniMode).toBe(false);
    await store.forceExpand();
    await flush();
    expect(setMiniModeWithPos).not.toHaveBeenCalled();
    expect(apiCalls).toEqual([]);
  });

  it("迷你态：清标记并恢复完整几何（setMiniModeWithPos(false) 恰好一次）", async () => {
    const store = useWidgetStore();
    // 进入迷你态（走正常 toggle 路径，与真实使用一致）
    await store.toggleMiniMode();
    await flush();
    expect(store.miniMode).toBe(true);
    apiCalls.length = 0;
    vi.mocked(setMiniModeWithPos).mockClear();

    await store.forceExpand();
    await flush();
    expect(store.miniMode).toBe(false);
    expect(setMiniModeWithPos).toHaveBeenCalledTimes(1);
    expect(apiCalls).toEqual(["setMiniModeWithPos:false"]);
  });

  it("迷你态切换期间几何保存被抑制（geometryTransition 守卫生效）", async () => {
    const store = useWidgetStore();
    await store.toggleMiniMode();
    // 切换进行中触发的 move/resize 保存必须被丢弃
    // （toggleMiniMode 内 beginGeometryTransition → 结束后解除）
    await store.forceExpand();
    await flush();
    // 若守卫失效，doSavePosition 会写入 x/y/width/height；此处验证切换路径
    // 没有把迷你几何当完整几何保存（mini_x/mini_y 只在迷你态保存，
    // 完整态守卫下什么都不写）
    const saved = vi.mocked((await import("../../api")).saveWidgetConfig);
    const keys = saved.mock.calls.map((c) => c[0]);
    expect(keys).not.toContain("width");
    expect(keys).not.toContain("height");
  });

  it("完整态下连续多次 force-expand 事件不产生副作用（后端每次唤出都 emit）", async () => {
    const store = useWidgetStore();
    for (let i = 0; i < 5; i++) {
      await store.forceExpand();
    }
    await flush();
    expect(setMiniModeWithPos).not.toHaveBeenCalled();
    expect(store.miniMode).toBe(false);
  });
});

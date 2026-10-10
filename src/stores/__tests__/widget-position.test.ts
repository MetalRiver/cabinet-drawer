// @vitest-environment jsdom
// ===== Settings UI V2（阶段 4）：窗口位置真实性回归 =====
// - 记忆位置使用独立坐标（widget.mem_x/mem_y），与自动几何（widget.x/y）不混用
// - 回到记忆位置：未保存 → 可见错误且不动窗口；已保存 → 仅移动、不改尺寸
// - 九宫格预设：基于工作区计算、只调位置 IPC、绝不触发 setWindowSize

import { describe, it, expect, vi, beforeEach } from "vitest";
import { createPinia, setActivePinia } from "pinia";

const savedSettings = new Map<string, string>();
const positionCalls: Array<[number, number]> = [];
const sizeCalls: Array<[number, number]> = [];
let fakeWindowPos: [number, number] = [123, 456];

vi.mock("../../api", () => ({
  loadWidgetConfig: vi.fn(async (key: string) => savedSettings.get(key) ?? null),
  saveWidgetConfig: vi.fn(async (key: string, value: string) => {
    savedSettings.set(key, value);
  }),
  setWindowPosition: vi.fn(async (x: number, y: number) => {
    positionCalls.push([x, y]);
    fakeWindowPos = [x, y];
  }),
  setWindowSize: vi.fn(async (w: number, h: number) => {
    sizeCalls.push([w, h]);
  }),
  getWindowPosition: vi.fn(async () => fakeWindowPos),
  getWindowSize: vi.fn(async () => [480, 720] as [number, number]),
  setAlwaysOnTop: vi.fn(async () => {}),
  setIgnoreCursorEvents: vi.fn(async () => {}),
  setMiniModeWithPos: vi.fn(async () => {}),
  isDockHidden: vi.fn(async () => false),
}));

import { useWidgetStore } from "../widget";
import { saveWidgetConfig, setWindowSize } from "../../api";

beforeEach(() => {
  setActivePinia(createPinia());
  savedSettings.clear();
  positionCalls.length = 0;
  sizeCalls.length = 0;
  fakeWindowPos = [123, 456];
  Object.defineProperty(window.screen, "availWidth", { value: 1920, configurable: true });
  Object.defineProperty(window.screen, "availHeight", { value: 1080, configurable: true });
  vi.spyOn(console, "error").mockImplementation(() => {});
  vi.spyOn(console, "warn").mockImplementation(() => {});
});

async function flush() {
  for (let i = 0; i < 5; i++) await Promise.resolve();
}

describe("记忆位置（独立坐标）", () => {
  it("保存当前位置写入 mem_x/mem_y，不触碰自动几何 x/y/width/height", async () => {
    const store = useWidgetStore();
    await store.saveMemoryPosition();
    await flush();
    expect(savedSettings.get("mem_x")).toBe("123");
    expect(savedSettings.get("mem_y")).toBe("456");
    expect(savedSettings.has("x")).toBe(false);
    expect(savedSettings.has("width")).toBe(false);
    expect(store.memoryX).toBe(123);
    expect(store.memoryY).toBe(456);
    expect(store.memoryLabel).toBe("已记忆 123, 456");
  });

  it("未保存过时回到记忆位置：可见错误且不调用窗口 IPC", async () => {
    const store = useWidgetStore();
    await store.loadMemoryPosition();
    await expect(store.restoreMemoryPosition()).rejects.toThrow("还没有保存过记忆位置");
    expect(positionCalls.length).toBe(0);
  });

  it("已保存时回到记忆位置：仅移动窗口、绝不改尺寸", async () => {
    savedSettings.set("mem_x", "800");
    savedSettings.set("mem_y", "300");
    const store = useWidgetStore();
    await store.loadMemoryPosition();
    await store.restoreMemoryPosition();
    expect(positionCalls).toEqual([[800, 300]]);
    expect(sizeCalls.length).toBe(0);
  });
});

describe("九宫格预设（真实位置动作）", () => {
  it("居中：按工作区计算逻辑坐标并按 devicePixelRatio 转物理像素", async () => {
    Object.defineProperty(window, "devicePixelRatio", { value: 1, configurable: true });
    const store = useWidgetStore();
    const r = await store.applyPresetPosition("center");
    expect(r).toEqual({ x: 720, y: 180 }); // (1920-480)/2, (1080-720)/2
    expect(positionCalls).toEqual([[720, 180]]);
    expect(sizeCalls.length).toBe(0); // 绝不重置窗口尺寸
    await flush();
    expect(savedSettings.get("x")).toBe("720");
    expect(savedSettings.get("y")).toBe("180");
    expect(savedSettings.has("width")).toBe(false);
  });

  it("DPI 150%：逻辑坐标 × 1.5 转物理像素", async () => {
    Object.defineProperty(window, "devicePixelRatio", { value: 1.5, configurable: true });
    const store = useWidgetStore();
    const r = await store.applyPresetPosition("top-left");
    expect(r).toEqual({ x: 36, y: 36 }); // 24 * 1.5
    expect(positionCalls).toEqual([[36, 36]]);
  });

  it("尺寸 IPC 全程零调用（位置动作不改尺寸的硬约束）", async () => {
    const store = useWidgetStore();
    for (const p of [
      "top-left", "top-center", "top-right",
      "middle-left", "center", "middle-right",
      "bottom-left", "bottom-center", "bottom-right",
    ] as const) {
      await store.applyPresetPosition(p);
    }
    expect(setWindowSize).not.toHaveBeenCalled();
  });
});

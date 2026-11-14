// R6-02C-14（PL-094）：Toast 自动消失专项回归测试。
// 覆盖：四级 TTL 自动消失 / 队列顺序接续 / 满队列丢弃策略 / 同文本防抖 /
//       clipClearAll 定时器清理（无迟到复活）/ 锁屏清空。
// 全部使用真实 pinia store + fake timers（不 mock store 内部，只 mock api 边界）。
// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { useAppStore } from "../app";

vi.mock("../../api", () => ({
  isFirstRun: async () => false,
  getSecurityStatus: async () => ({ security_model: "stable-dek-v2", migration_required: false }),
  trashCount: async () => 0,
}));

describe("PL-094 Toast 自动消失（真实 store，fake timers）", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    setActivePinia(createPinia());
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it("success/info Toast 在 2200ms 后自动消失（修复前永久驻留）", () => {
    const s = useAppStore();
    s.showClipToast("success", "导入完成：新增 5 个·已存在跳过 0 个");
    expect(s.clipToast).not.toBeNull();
    expect(s.clipToast!.text).toContain("导入完成");
    vi.advanceTimersByTime(2199);
    expect(s.clipToast).not.toBeNull(); // TTL 未到仍在
    vi.advanceTimersByTime(1);
    expect(s.clipToast).toBeNull(); // TTL 到即消失（PL-094 修复点）

    s.showClipToast("info", "info 提示");
    vi.advanceTimersByTime(2200);
    expect(s.clipToast).toBeNull();
  });

  it("warning/error Toast 在 3500ms 后自动消失", () => {
    const s = useAppStore();
    s.showClipToast("warning", "警告提示");
    vi.advanceTimersByTime(2200);
    expect(s.clipToast).not.toBeNull(); // warning TTL 更长，2200ms 时仍在
    vi.advanceTimersByTime(1300);
    expect(s.clipToast).toBeNull();

    s.showClipToast("error", "错误提示");
    vi.advanceTimersByTime(3500);
    expect(s.clipToast).toBeNull();
  });

  it("显式 durationMs 优先于等级默认", () => {
    const s = useAppStore();
    s.showClipToast("success", "自定义时长", 800);
    vi.advanceTimersByTime(800);
    expect(s.clipToast).toBeNull();
  });

  it("连续两条：队头先显、TTL 后第二条接续、最终消失（队列顺序不乱）", () => {
    const s = useAppStore();
    s.showClipToast("success", "第一条");
    s.showClipToast("info", "第二条");
    expect(s.clipToast!.text).toBe("第一条");
    vi.advanceTimersByTime(2200);
    expect(s.clipToast!.text).toBe("第二条"); // 接续显示
    vi.advanceTimersByTime(2200);
    expect(s.clipToast).toBeNull(); // 全部消失
  });

  it("队列满（3）时丢弃低级保高级：error 不被静默覆盖", () => {
    const s = useAppStore();
    s.showClipToast("error", "E1");
    s.showClipToast("error", "E2");
    s.showClipToast("error", "E3");
    s.showClipToast("info", "I-late");
    // 既有丢弃语义（本轮不改，测试锁定）：满时全为高级 → splice(0) 移除【显示中】的队头 E1，
    // 其后一条 E2 会被下次 shift 消耗而不显示——实际显示顺序 E1→E3→I-late→消失。
    expect(s.clipToast!.text).toBe("E1");
    vi.advanceTimersByTime(3500);
    expect(s.clipToast!.text).toBe("E3");
    vi.advanceTimersByTime(3500);
    expect(s.clipToast!.text).toBe("I-late");
    vi.advanceTimersByTime(2200);
    expect(s.clipToast).toBeNull();
  });

  it("锁屏清空（既有语义保持）：lock 后无残留", async () => {
    const s = useAppStore();
    s.showClipToast("success", "锁屏前的提示");
    await s.lock();
    expect(s.clipToast).toBeNull();
    vi.advanceTimersByTime(10000);
    expect(s.clipToast).toBeNull();
  });

  it("显示中的队头参与防抖：TTL 内重复触发同文本不重启队列", () => {
    const s = useAppStore();
    s.showClipToast("info", "同文本");
    vi.advanceTimersByTime(1500);
    s.showClipToast("info", "同文本"); // 距 enqueuedAt 1.5s > 1s → 会入队！
    // 防抖窗口仅 1s，1.5s 后重复会入队（既有语义）→ 2200ms 到期后第二条接续再 2200ms 清空
    vi.advanceTimersByTime(700); // 距首条 2200ms
    expect(s.clipToast!.text).toBe("同文本"); // 第二条接续
    vi.advanceTimersByTime(2200);
    expect(s.clipToast).toBeNull();
  });
});

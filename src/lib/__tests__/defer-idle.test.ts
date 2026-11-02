// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { scheduleDeferred } from "../defer-idle";

// O2：启动期非关键扫描的延迟执行器语义。
// jsdom 无 requestIdleCallback → 测试注入可控 mock；fake timers 驱动时序。

describe("scheduleDeferred（O2 启动扫描延迟执行器）", () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });
  afterEach(() => {
    vi.useRealTimers();
    delete (window as unknown as { requestIdleCallback?: unknown }).requestIdleCallback;
    delete (window as unknown as { cancelIdleCallback?: unknown }).cancelIdleCallback;
  });

  it("minDelay 前不执行；minDelay 后经空闲回调执行；started 幂等", () => {
    const holder: { cb: (() => void) | null } = { cb: null };
    (window as unknown as { requestIdleCallback: unknown }).requestIdleCallback = (f: () => void) => {
      holder.cb = f;
      return 1;
    };
    let runs = 0;
    const t = scheduleDeferred(() => { runs++; }, { minDelayMs: 300, timeoutMs: 3000 });
    vi.advanceTimersByTime(299);
    expect(runs).toBe(0);
    vi.advanceTimersByTime(1); // minDelay → scheduleAt → 空闲回调注册（未执行）
    expect(holder.cb).not.toBeNull();
    expect(t.isStarted()).toBe(false);
    holder.cb?.(); // 模拟空闲回调触发
    expect(runs).toBe(1);
    expect(t.isStarted()).toBe(true);
  });

  it("minDelay 前 cancel → 零执行", () => {
    (window as unknown as { requestIdleCallback: unknown }).requestIdleCallback = vi.fn(() => 1);
    let runs = 0;
    const t = scheduleDeferred(() => { runs++; }, { minDelayMs: 300 });
    t.cancel();
    vi.advanceTimersByTime(5000);
    expect(runs).toBe(0);
  });

  it("requestIdleCallback 不可用 → setTimeout(timeoutMs) 兜底执行", () => {
    let runs = 0;
    scheduleDeferred(() => { runs++; }, { minDelayMs: 300, timeoutMs: 3000 });
    vi.advanceTimersByTime(300 + 3000);
    expect(runs).toBe(1);
  });

  it("requestIdleCallback 存在但不回调（浏览器缺陷）→ timeoutMs setTimeout 兜底", () => {
    (window as unknown as { requestIdleCallback: unknown }).requestIdleCallback = vi.fn(() => 1);
    let runs = 0;
    scheduleDeferred(() => { runs++; }, { minDelayMs: 300, timeoutMs: 3000 });
    vi.advanceTimersByTime(300 + 3000);
    expect(runs).toBe(1);
  });

  it("任务同步抛出 → onError 收到，不向外传播", () => {
    const onError = vi.fn();
    scheduleDeferred(() => { throw new Error("boom"); }, { minDelayMs: 100, timeoutMs: 1000, onError });
    vi.advanceTimersByTime(100 + 1000);
    expect(onError).toHaveBeenCalledTimes(1);
  });

  it("任务异步拒绝 → onError 收到，不向外传播", async () => {
    const onError = vi.fn();
    scheduleDeferred(async () => { throw new Error("async boom"); }, { minDelayMs: 100, timeoutMs: 1000, onError });
    vi.advanceTimersByTime(100 + 1000);
    await Promise.resolve();
    await Promise.resolve();
    expect(onError).toHaveBeenCalledTimes(1);
  });
});

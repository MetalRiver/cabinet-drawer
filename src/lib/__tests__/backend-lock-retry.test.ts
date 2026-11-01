import { describe, it, expect, vi } from "vitest";
import { clearBackendKeyWithRetry } from "../backend-lock-retry";

// O1.2：空闲自动锁后端密钥清除的重试语义。
// 立即 resolve 的 delay 注入，避免真实等待。

describe("clearBackendKeyWithRetry", () => {
  it("首次成功：只调用一次，返回 true", async () => {
    const lockFn = vi.fn().mockResolvedValue(undefined);
    const onError = vi.fn();
    const ok = await clearBackendKeyWithRetry(lockFn, { onError, delay: () => Promise.resolve() });
    expect(ok).toBe(true);
    expect(lockFn).toHaveBeenCalledTimes(1);
    expect(onError).not.toHaveBeenCalled();
  });

  it("瞬时失败后重试成功：按次序重试，返回 true", async () => {
    const lockFn = vi
      .fn()
      .mockRejectedValueOnce(new Error("ipc down"))
      .mockResolvedValueOnce(undefined);
    const onError = vi.fn();
    const delay = vi.fn().mockResolvedValue(undefined);
    const ok = await clearBackendKeyWithRetry(lockFn, { attempts: 3, retryDelayMs: 500, onError, delay });
    expect(ok).toBe(true);
    expect(lockFn).toHaveBeenCalledTimes(2);
    expect(delay).toHaveBeenCalledWith(500);
    expect(onError).toHaveBeenCalledTimes(1);
    expect(onError).toHaveBeenNthCalledWith(1, expect.any(Error), 1);
  });

  it("全部尝试失败：返回 false，每次失败都有 onError，不向上抛异常", async () => {
    const lockFn = vi.fn().mockRejectedValue(new Error("backend unreachable"));
    const onError = vi.fn();
    const ok = await clearBackendKeyWithRetry(lockFn, { attempts: 3, onError, delay: () => Promise.resolve() });
    expect(ok).toBe(false);
    expect(lockFn).toHaveBeenCalledTimes(3);
    expect(onError).toHaveBeenCalledTimes(3);
    expect(onError).toHaveBeenNthCalledWith(1, expect.any(Error), 1);
    expect(onError).toHaveBeenNthCalledWith(3, expect.any(Error), 3);
  });

  it("lockFn 同步抛出异常：按失败处理，不向上抛", async () => {
    const lockFn = vi.fn(() => {
      throw new Error("sync throw");
    }) as unknown as () => Promise<void>;
    const onError = vi.fn();
    const ok = await clearBackendKeyWithRetry(lockFn, { attempts: 2, onError, delay: () => Promise.resolve() });
    expect(ok).toBe(false);
    expect(onError).toHaveBeenCalledTimes(2);
  });

  it("attempts 非法值（0/负数）收敛为至少 1 次", async () => {
    const lockFn = vi.fn().mockRejectedValue(new Error("x"));
    const ok = await clearBackendKeyWithRetry(lockFn, { attempts: 0, delay: () => Promise.resolve() });
    expect(ok).toBe(false);
    expect(lockFn).toHaveBeenCalledTimes(1);
  });

  it("O1.3 shouldAbort 首查即真：一次都不尝试，返回 false", async () => {
    const lockFn = vi.fn().mockResolvedValue(undefined);
    const onError = vi.fn();
    const ok = await clearBackendKeyWithRetry(lockFn, {
      shouldAbort: () => true,
      onError,
      delay: () => Promise.resolve(),
    });
    expect(ok).toBe(false);
    expect(lockFn).not.toHaveBeenCalled();
    expect(onError).not.toHaveBeenCalled();
  });

  it("O1.3 shouldAbort 在 delay 后生效：第 2 次尝试前中止（用户已解锁，不清新密钥）", async () => {
    const lockFn = vi.fn().mockRejectedValue(new Error("fail"));
    let unlocked = false;
    const delay = vi.fn(async () => {
      unlocked = true; // 模拟重试等待期间用户完成解锁
    });
    const ok = await clearBackendKeyWithRetry(lockFn, {
      attempts: 3,
      shouldAbort: () => unlocked,
      delay,
    });
    expect(ok).toBe(false);
    expect(lockFn).toHaveBeenCalledTimes(1);
    expect(delay).toHaveBeenCalledTimes(1);
  });
});

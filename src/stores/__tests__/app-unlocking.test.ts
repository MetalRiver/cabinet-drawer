import { describe, it, expect, beforeEach } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { useAppStore } from "../app";

// O1.3：isUnlocking 信号——空闲自动锁的重试中止与耗尽判定依据。
// 背景：前端 isLocked 翻转要等 KDF 验证 + UI 切换（实测 1–3s），期间后端新密钥已就位。
describe("appStore isUnlocking（O1.3）", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
  });

  it("初始为 false；beginUnlock 置位；endUnlock 复位", () => {
    const s = useAppStore();
    expect(s.isUnlocking).toBe(false);
    s.beginUnlock();
    expect(s.isUnlocking).toBe(true);
    s.endUnlock();
    expect(s.isUnlocking).toBe(false);
  });

  it("多次 beginUnlock 幂等；解锁失败路径 endUnlock 同样复位", () => {
    const s = useAppStore();
    s.beginUnlock();
    s.beginUnlock();
    expect(s.isUnlocking).toBe(true);
    s.endUnlock();
    expect(s.isUnlocking).toBe(false);
  });
});

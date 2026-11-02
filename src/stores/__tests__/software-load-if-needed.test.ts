import { describe, it, expect, vi, beforeEach } from "vitest";
import { createPinia, setActivePinia } from "pinia";

// O3：软件列表初始化去重——loadItemsIfNeeded 语义验证。
// mock 只需覆盖被调用到的 api（其余导入名未调用即为 undefined，不报错）。
vi.mock("../../api", () => ({
  listApps: vi.fn(async () => []),
  listAppCategories: vi.fn(async () => []),
}));

import { useSoftwareStore } from "../software";

describe("softwareStore.loadItemsIfNeeded（O3）", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
  });

  it("首次（hasLoaded=false）→ 真正加载并置位 hasLoaded", async () => {
    const s = useSoftwareStore();
    const { listApps } = await import("../../api");
    expect(s.hasLoaded).toBe(false);
    await s.loadItemsIfNeeded();
    expect(listApps).toHaveBeenCalledTimes(1);
    expect(s.hasLoaded).toBe(true);
  });

  it("已加载（hasLoaded=true）→ 跳过（O3 去重：每会话 1 次全量）", async () => {
    const s = useSoftwareStore();
    const { listApps } = await import("../../api");
    await s.loadItemsIfNeeded();
    await s.loadItemsIfNeeded();
    await s.loadItemsIfNeeded();
    expect(listApps).toHaveBeenCalledTimes(1);
  });

  it("验收 E：加载失败 → hasLoaded 保持 false → 下次仍真正加载（可重试，不误标）", async () => {
    const s = useSoftwareStore();
    const { listApps } = await import("../../api");
    vi.mocked(listApps).mockRejectedValueOnce(new Error("ipc fail"));
    await s.loadItemsIfNeeded();
    expect(s.hasLoaded).toBe(false);
    expect(s.error).toContain("ipc fail");
    await s.loadItemsIfNeeded(); // 失败后重试 → 仍会真正加载
    expect(listApps).toHaveBeenCalledTimes(2);
    expect(s.hasLoaded).toBe(true);
  });

  it("强制刷新 loadItems 不受去重影响（O2 扫描/CRUD/reloadAll 路径保障）", async () => {
    const s = useSoftwareStore();
    const { listApps } = await import("../../api");
    await s.loadItemsIfNeeded(); // 初始化
    await s.loadItems(); // 强制刷新：总是加载
    await s.loadItems(); // 再次强制刷新
    expect(listApps).toHaveBeenCalledTimes(3);
  });
});

// @vitest-environment jsdom
// ===== v0.4.1 任务 B 回归：外链动作（真实调用路径，非按钮渲染检查）=====
import { describe, it, expect, vi, beforeEach } from "vitest";

const apiOpenExternalUrl = vi.fn();
vi.mock("../../api", () => ({
  openExternalUrl: (url: string) => apiOpenExternalUrl(url),
}));

import { openExternalLink } from "../external-links";

beforeEach(() => {
  apiOpenExternalUrl.mockReset();
});

describe("openExternalLink（GitHub 跳转失效修复）", () => {
  it("正常路径：把 URL 传给 Rust open_external_url", async () => {
    apiOpenExternalUrl.mockResolvedValue(undefined);
    await openExternalLink("https://github.com/MetalRiver/cabinet-drawer/releases/tag/v0.4.0");
    expect(apiOpenExternalUrl).toHaveBeenCalledWith(
      "https://github.com/MetalRiver/cabinet-drawer/releases/tag/v0.4.0"
    );
  });

  it("后端失败：抛出含 URL 的可见错误（不再静默吞掉——用户实测零反应的根因修复）", async () => {
    apiOpenExternalUrl.mockRejectedValue(new Error("打开系统浏览器失败：EOF"));
    await expect(
      openExternalLink("https://github.com/MetalRiver/cabinet-drawer/releases")
    ).rejects.toThrow(/外部链接打开失败.*releases.*打开系统浏览器失败/);
  });

  it("非 http(s) 链接：前端直接拒绝，不发起 IPC", async () => {
    await expect(openExternalLink("file:///C:/evil.exe")).rejects.toThrow("已阻止打开");
    await expect(openExternalLink("javascript:alert(1)")).rejects.toThrow("已阻止打开");
    expect(apiOpenExternalUrl).not.toHaveBeenCalled();
  });
});

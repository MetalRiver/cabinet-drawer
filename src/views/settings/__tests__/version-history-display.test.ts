// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import { createApp, nextTick, type App } from "vue";
import AdvancedTab from "../tabs/AdvancedTab.vue";
import { VERSION_HISTORY } from "../../../data/version-history";

// Only IPC and side-effect boundaries are mocked; mount the actual history UI.
vi.mock("../../../api", () => ({
  getAppVersion: async () => "0.4.4",
  getAppEdition: async () => "production",
  getSetting: async () => "false",
  setSetting: vi.fn(),
  restartApp: vi.fn(),
}));
vi.mock("../../../stores/app", () => ({
  useAppStore: () => ({ showClipToast: vi.fn() }),
}));
vi.mock("../../../stores/software", () => ({
  useSoftwareStore: () => ({ refreshAllIcons: vi.fn() }),
}));
vi.mock("../ops", () => ({
  useSettingsOps: () => ({ begin: vi.fn(), end: vi.fn() }),
}));
vi.mock("../../../lib/external-links", () => ({ openExternalLink: vi.fn() }));

let app: App | undefined;
afterEach(() => {
  app?.unmount();
  app = undefined;
  document.body.innerHTML = "";
});

describe("v0.4.4 candidate history in the existing settings UI", () => {
  it("shows the unreleased candidate on top with 未发布 badge, expands its full notes, and includes it in the all-records panel", async () => {
    const container = document.createElement("div");
    document.body.appendChild(container);
    app = createApp(AdvancedTab);
    app.mount(container);
    await Promise.resolve();
    await nextTick();

    // 候选条目：unreleased + date 为空，UI 显示"未发布"，不得出现虚构日期
    expect(VERSION_HISTORY[0].version).toBe("v0.4.4");
    expect(VERSION_HISTORY[0].date).toBe("");
    expect(VERSION_HISTORY[0].unreleased).toBe(true);

    const first = container.querySelector(".vh-item")!;
    expect(first.querySelector(".vh-version")?.textContent).toContain("v0.4.4");
    expect(first.querySelector(".vh-date")?.textContent).toContain("未发布");
    expect(first.querySelector(".vh-date")?.textContent).not.toContain("2026-");

    // 已安装的正式版本 v0.4.3 保持完整展示（date + unreleased=false）
    const second = container.querySelectorAll(".vh-item")[1];
    expect(second?.textContent).toContain("v0.4.3");
    expect(second?.textContent).toContain("2026-10-24");

    // 展开候选条目 notes
    const expand = Array.from(first.querySelectorAll("button")).find(b => b.textContent === "更新内容")!;
    expand.click();
    await nextTick();
    expect(first.querySelector(".vh-notes")?.textContent).toBe(VERSION_HISTORY[0].notes);
    expect(first.querySelector(".vh-notes")?.textContent).toContain("密码区");
    expect(first.querySelector(".vh-notes")?.textContent).toContain("软件区");
    expect(first.querySelector(".vh-notes")?.textContent).toContain("数据与兼容");

    // 全部记录面板
    const showAll = Array.from(container.querySelectorAll("button")).find(b => b.textContent?.includes("查看全部历史版本"))!;
    showAll.click();
    await nextTick();
    const modal = document.querySelector(".vh-modal")!;
    expect(modal.querySelectorAll(".vh-item")).toHaveLength(VERSION_HISTORY.length);
    expect(modal.querySelector(".vh-item")?.textContent).toContain("v0.4.4");
    expect(modal.querySelector(".vh-item")?.textContent).toContain("未发布");
    expect(modal.querySelector(".vh-notes")?.textContent).toBe(VERSION_HISTORY[0].notes);
  });
});

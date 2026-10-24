// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import { createApp, nextTick, type App } from "vue";
import AdvancedTab from "../tabs/AdvancedTab.vue";
import { VERSION_HISTORY } from "../../../data/version-history";

// Only IPC and side-effect boundaries are mocked; mount the actual history UI.
vi.mock("../../../api", () => ({
  getAppVersion: async () => "0.4.3",
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

describe("v0.4.3 candidate history in the existing settings UI", () => {
  it("shows the installed released entry, expands its full notes, and includes it in the all-records panel", async () => {
    const container = document.createElement("div");
    document.body.appendChild(container);
    app = createApp(AdvancedTab);
    app.mount(container);
    await Promise.resolve();
    await nextTick();

    const first = container.querySelector(".vh-item")!;
    expect(first.querySelector(".vh-version")?.textContent).toContain("v0.4.3");
    expect(first.querySelector(".vh-now")?.textContent).toBe("本机安装");
    expect(first.querySelector(".vh-date")?.textContent).toContain("2026-10-24");
    expect(VERSION_HISTORY[0].date).toBe("2026-10-24");
    expect(VERSION_HISTORY[0].unreleased).toBe(false);
    expect(container.querySelectorAll(".vh-item")[1].textContent).toContain("2026-10-12");

    const expand = Array.from(first.querySelectorAll("button")).find(b => b.textContent === "更新内容")!;
    expand.click();
    await nextTick();
    expect(first.querySelector(".vh-notes")?.textContent).toBe(VERSION_HISTORY[0].notes);
    expect(first.querySelector(".vh-notes")?.textContent).toContain("软件扫描与整理");
    expect(first.querySelector(".vh-notes")?.textContent).toContain("数据兼容与备份建议");

    const showAll = Array.from(container.querySelectorAll("button")).find(b => b.textContent?.includes("查看全部历史版本"))!;
    showAll.click();
    await nextTick();
    const modal = document.querySelector(".vh-modal")!;
    expect(modal.querySelectorAll(".vh-item")).toHaveLength(VERSION_HISTORY.length);
    expect(modal.querySelector(".vh-modal-head")?.textContent).toContain("条版本记录");
    expect(modal.querySelector(".vh-modal-head")?.textContent).not.toContain("正式版本");
    expect(modal.querySelector(".vh-item")?.textContent).toContain("v0.4.3");
    expect(modal.querySelector(".vh-date")?.textContent).toContain("2026-10-24");
    expect(modal.querySelector(".vh-notes")?.textContent).toBe(VERSION_HISTORY[0].notes);
  });
});

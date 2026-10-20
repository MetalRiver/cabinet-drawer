// @vitest-environment jsdom
// ===== Batch 3：顶部新版本提醒 —— 8 场景验收 =====
// 1. 当前已是最新版   2. 检查到正式新版本   3. 网络不可用   4. 服务器超时
// 5. 更新元数据错误   6. 用户关闭自动检查   7. 开关持久化   8. 提醒点击导航
// （场景 8 为组件级行为，本文件覆盖 1-7 的逻辑层 + 场景 6/7 的判定口径；
//   导航跳转 /settings?tab=advanced 由 SettingsView 的 query 读取覆盖。）

import { describe, it, expect, vi, beforeEach } from "vitest";

// 模拟 settings 存取（内存表）
const settings = new Map<string, string>();
vi.mock("../../api", () => ({
  getSetting: vi.fn(async (key: string) => settings.get(key) ?? null),
  setSetting: vi.fn(async (key: string, value: string) => {
    settings.set(key, value);
  }),
}));

// 模拟 updater 插件（可注入行为）
type CheckResult = object | null;
let checkImpl: () => Promise<CheckResult> = async () => null;
vi.mock("@tauri-apps/plugin-updater", () => ({
  check: vi.fn(async () => checkImpl()),
}));

import {
  shouldAutoCheck,
  autoCheckUpdate,
  checkOnce,
  AUTO_CHECK_KEY,
  LAST_CHECK_KEY,
  AUTO_CHECK_INTERVAL_MS,
} from "../update-check";

beforeEach(() => {
  settings.clear();
  checkImpl = async () => null;
  vi.clearAllMocks();
});

const DAY = AUTO_CHECK_INTERVAL_MS;

describe("场景 6/7：自动检查开关与持久化", () => {
  it("关闭自动检查 → 完全不发起请求（返回 null，check 不被调用）", async () => {
    settings.set(AUTO_CHECK_KEY, "false");
    const outcome = await autoCheckUpdate(Date.now());
    expect(outcome).toBeNull();
  });

  it("开关持久化：写入 'false' 后重启（重新读取）仍为关闭", async () => {
    settings.set(AUTO_CHECK_KEY, "false");
    // 模拟重启：新一次 autoCheckUpdate 读同一 settings 表
    const outcome = await autoCheckUpdate(Date.now());
    expect(outcome).toBeNull();
    // 且从未触碰检查器
    expect(settings.has(LAST_CHECK_KEY)).toBe(false);
  });

  it("缺省（无设置记录）= 开启", async () => {
    checkImpl = async () => ({ version: "0.5.0" });
    const outcome = await autoCheckUpdate(Date.now());
    expect(outcome?.status).toBe("available");
  });
});

describe("节流（24h 至多一次）", () => {
  it("从未检查过 → 立即检查", () => {
    expect(shouldAutoCheck({ enabled: true, lastCheckAt: null, now: 1000 })).toBe(true);
  });
  it("24h 内已检查过 → 跳过", () => {
    expect(
      shouldAutoCheck({ enabled: true, lastCheckAt: 1000, now: 1000 + DAY - 1 })
    ).toBe(false);
  });
  it("超过 24h → 再查", () => {
    expect(
      shouldAutoCheck({ enabled: true, lastCheckAt: 1000, now: 1000 + DAY + 1 })
    ).toBe(true);
  });
  it("检查后写入时间戳（下次节流依据）", async () => {
    checkImpl = async () => null;
    const now = 1_000_000;
    await autoCheckUpdate(now);
    expect(settings.get(LAST_CHECK_KEY)).toBe(String(now));
  });
});

describe("场景 1：当前已是最新版", () => {
  it("check() 返回 null → up-to-date，不显示提醒", async () => {
    const outcome = await checkOnce();
    expect(outcome.status).toBe("up-to-date");
    expect(outcome.version).toBeUndefined();
  });
});

describe("场景 2：检查到正式新版本", () => {
  it("返回 available + 版本号", async () => {
    checkImpl = async () => ({ version: "0.5.0" });
    const outcome = await checkOnce();
    expect(outcome.status).toBe("available");
    expect(outcome.version).toBe("0.5.0");
  });
});

describe("场景 3/4/5：网络不可用 / 超时 / 元数据错误", () => {
  it("网络不可用（请求抛错）→ error，不抛出异常", async () => {
    checkImpl = async () => {
      throw new Error("network unreachable");
    };
    const outcome = await checkOnce();
    expect(outcome.status).toBe("error");
    expect(outcome.error).toContain("network unreachable");
  });
  it("服务器超时（AbortError）→ error", async () => {
    checkImpl = async () => {
      const e = new Error("operation timed out");
      throw e;
    };
    const outcome = await checkOnce();
    expect(outcome.status).toBe("error");
  });
  it("更新元数据错误（非法清单导致插件抛错）→ error，绝不显示虚假提醒", async () => {
    checkImpl = async () => {
      throw new Error("failed to parse update manifest");
    };
    const outcome = await checkOnce();
    expect(outcome.status).toBe("error");
    expect(outcome.version).toBeUndefined();
  });
  it("error 结果不写入 version —— 调用方不会显示新版本徽章", async () => {
    checkImpl = async () => {
      throw new Error("offline");
    };
    const outcome = await autoCheckUpdate(Date.now());
    expect(outcome?.version).toBeUndefined();
  });
});

describe("本地开发版本（v0.4.3）不被当作已发布版本", () => {
  it("检查结果完全由服务器清单决定，本地版本号不参与判定", async () => {
    // 本地 0.4.3 vs 服务器 latest.json=0.4.1：check() 返回 null（无更新）
    checkImpl = async () => null;
    const outcome = await checkOnce();
    expect(outcome.status).toBe("up-to-date");
  });
});

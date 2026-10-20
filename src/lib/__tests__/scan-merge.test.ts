// @vitest-environment jsdom
// ===== Batch 2.4（并发审计 D3/D5）：扫描候选合并与会话门卫 =====
// - 累积合并：按身份去重，新值覆盖旧值（已在库/回收站标记刷新）
// - 不同参数/不同目标不合并（Chrome PWA 场景）
// - 会话门卫：旧任务迟到结果必须被丢弃，不得污染新任务候选

import { describe, it, expect } from "vitest";
import {
  mergeScanCandidates,
  applyScanOutcomeIfCurrent,
  scanCandidateKey,
  isScanSelectable,
} from "../scan-merge";
import type { ScannedApp } from "../../api";

function app(p: Partial<ScannedApp>): ScannedApp {
  return {
    name: "软件",
    path: "C:\\lnk\\x.lnk",
    icon_path: "",
    args: "",
    source: "开始菜单",
    app_type: "app",
    app_subtype: "",
    is_launchable: true,
    target: "C:\\Apps\\x.exe",
    target_args: "",
    workdir: "",
    status: "identified",
    already_imported: false,
    in_trash: false,
    trash_id: null,
    ...p,
  };
}

describe("mergeScanCandidates（累积合并）", () => {
  it("同身份新结果覆盖旧结果（already_imported 标记刷新）", () => {
    const base = [app({ path: "C:\\a.lnk", already_imported: false })];
    const incoming = [app({ path: "C:\\a.lnk", already_imported: true })];
    const merged = mergeScanCandidates(base, incoming);
    expect(merged).toHaveLength(1);
    expect(merged[0].already_imported).toBe(true);
  });

  it("不同参数（Chrome PWA）不合并", () => {
    const base = [app({ target: "C:\\Chrome\\chrome.exe", target_args: "" })];
    const incoming = [
      app({ path: "C:\\pwa.lnk", target: "C:\\Chrome\\chrome.exe", target_args: "--app=https://x" }),
    ];
    expect(mergeScanCandidates(base, incoming)).toHaveLength(2);
  });

  it("累积不覆盖整个列表：第二次结果并入第一次", () => {
    const first = mergeScanCandidates(
      [],
      [app({ path: "C:\\a.lnk", name: "甲", target: "C:\\Apps\\a.exe" })]
    );
    const second = mergeScanCandidates(first, [
      app({ path: "C:\\b.lnk", name: "乙", target: "C:\\Apps\\b.exe" }),
    ]);
    expect(second).toHaveLength(2);
    expect([...second.map((x) => x.name)].sort()).toEqual(["乙", "甲"]); // 两条都在，已排序
  });

  it("身份键对大小写与斜杠不敏感", () => {
    expect(scanCandidateKey(app({ target: "C:\\A\\X.EXE" }))).toBe(
      scanCandidateKey(app({ target: "c:/a/x.exe" }))
    );
  });
});

describe("applyScanOutcomeIfCurrent（会话门卫，D3/D5）", () => {
  it("当前会话的结果放行", () => {
    const o = { apps: [app({})], cancelled: false, truncated: false };
    expect(applyScanOutcomeIfCurrent(o, 3, 3)).toBe(o);
  });

  it("旧任务迟到结果必须丢弃（不得污染新任务候选）", () => {
    const stale = { apps: [app({ name: "旧任务残留" })], cancelled: false, truncated: false };
    expect(applyScanOutcomeIfCurrent(stale, 2, 3)).toBeNull();
  });
});

// ===== P1-B：回收站候选必须走显式恢复，不得混进批量导入 =====
describe("isScanSelectable（回收站门禁）", () => {
  it("普通新候选可勾选", () => {
    expect(isScanSelectable(app({}))).toBe(true);
  });

  it("已在库不可勾选", () => {
    expect(isScanSelectable(app({ already_imported: true }))).toBe(false);
  });

  it("回收站命中不可勾选（走恢复，不重复入库）", () => {
    expect(isScanSelectable(app({ in_trash: true, trash_id: 42 }))).toBe(false);
  });
});

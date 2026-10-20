/**
 * Batch 2.4（并发审计 D3/D5）：扫描候选累积合并 —— 纯函数，独立可测。
 *
 * 语义：
 * - 高级模式多次扫描结果**按身份累积**，不覆盖整个列表；
 * - 同身份以**最新一次**结果为准（已在库/回收站标记刷新）；
 * - 调用方必须传 sessionId：只有"当前会话"的结果才允许合入，
 *   迟到的旧任务结果直接丢弃（防污染新任务候选）。
 */
import type { ScannedApp } from "../api";

/** 入口身份（与后端双层键的展示口径一致：类型 ∥ 规范化目标 ∥ 规范化参数）。
 *  规范化与后端 normalize_path_key 对齐：小写、正斜杠统一、去尾分隔符。 */
export function scanCandidateKey(a: ScannedApp): string {
  const norm = (s: string) => {
    let out = s.trim().toLowerCase().replace(/\//g, "\\");
    while (out.endsWith("\\") && out.length > 1) out = out.slice(0, -1);
    return out;
  };
  return `${a.app_type}|${norm(a.target || a.path)}|${norm(a.target_args)}`;
}

/** 合并：base + incoming，按身份去重、新值覆盖旧值；返回按名称排序的新数组 */
export function mergeScanCandidates(
  base: ScannedApp[],
  incoming: ScannedApp[]
): ScannedApp[] {
  const map = new Map<string, ScannedApp>();
  for (const a of base) map.set(scanCandidateKey(a), a);
  for (const a of incoming) map.set(scanCandidateKey(a), a);
  return Array.from(map.values()).sort((x, y) =>
    x.name.toLowerCase().localeCompare(y.name.toLowerCase())
  );
}

/**
 * P1-B（v0.4.3 验收）：候选是否可勾选导入。
 * - 已在库：不可勾选（导入会跳过）；
 * - 已在回收站：不可勾选，改为走显式「恢复此软件」动作——
 *   否则用户勾选后导入只会得到"回收站命中"，看起来就是"无法重新入库"。
 */
export function isScanSelectable(a: ScannedApp): boolean {
  return !a.already_imported && !a.in_trash;
}

/**
 * 会话门卫：旧任务的迟到结果不得写入新任务的候选。
 * currentSession = 当前会话号；resultSession = 该结果产生时的会话号。
 * 返回 null 表示"丢弃"（非当前会话）。
 */
export function applyScanOutcomeIfCurrent<T>(
  outcome: T,
  resultSession: number,
  currentSession: number
): T | null {
  if (resultSession !== currentSession) return null;
  return outcome;
}

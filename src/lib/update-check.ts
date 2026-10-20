/**
 * Batch 3（v0.4.3）：顶部新版本提醒 —— 检查逻辑与节流（纯逻辑独立可测）。
 *
 * 边界（按验收指令）：
 * - 只"发现"，不下载、不安装（plugin 的 check() 只拉取更新清单）；
 * - 不修改签名验证机制（由 plugin/updater 既有链路处理）；
 * - 离线/超时/清单错误 → 统一收敛为 error，绝不显示虚假的新版本状态；
 * - 节流：自动检查每 24h 至多一次（上次时间持久化在 settings 表）；
 * - 用户关闭自动检查后完全不发起请求；手动检查不受节流限制。
 */
import { getSetting, setSetting } from "../api";

export const AUTO_CHECK_KEY = "auto_check_updates"; // "true"（默认）| "false"
export const LAST_CHECK_KEY = "update_last_check_at"; // epoch ms 字符串
export const AUTO_CHECK_INTERVAL_MS = 24 * 60 * 60 * 1000;

export interface UpdateCheckOutcome {
  status: "up-to-date" | "available" | "error";
  /** 可用的新版本号（status === "available" 时存在） */
  version?: string;
  /** status === "error" 时的原因（离线/超时/元数据错误） */
  error?: string;
}

/** 节流判定（纯函数） */
export function shouldAutoCheck(opts: {
  enabled: boolean;
  lastCheckAt: number | null;
  now: number;
  intervalMs?: number;
}): boolean {
  if (!opts.enabled) return false;
  const interval = opts.intervalMs ?? AUTO_CHECK_INTERVAL_MS;
  if (opts.lastCheckAt == null) return true;
  return opts.now - opts.lastCheckAt >= interval;
}

/** 读取自动检查开关（缺省 = 开启；仅显式 "false" 视为关闭） */
export async function readAutoCheckEnabled(): Promise<boolean> {
  try {
    const v = await getSetting(AUTO_CHECK_KEY);
    return v !== "false";
  } catch {
    return true; // 读不到设置不阻塞启动，按默认开启（后续检查失败静默）
  }
}

/** 读取上次检查时间（无记录/解析失败 → null） */
export async function readLastCheckAt(): Promise<number | null> {
  try {
    const v = await getSetting(LAST_CHECK_KEY);
    if (!v) return null;
    const n = Number(v);
    return Number.isFinite(n) && n > 0 ? n : null;
  } catch {
    return null;
  }
}

export async function persistLastCheckAt(now: number): Promise<void> {
  try {
    await setSetting(LAST_CHECK_KEY, String(now));
  } catch {
    // 持久化失败不影响本次结果展示；下次会多查一次，可接受
  }
}

/** 执行一次更新检查（复用既有 Tauri Updater；只查清单，不下载不安装） */
export async function checkOnce(): Promise<UpdateCheckOutcome> {
  try {
    const { check } = await import("@tauri-apps/plugin-updater");
    const update = await check();
    if (!update) {
      return { status: "up-to-date" };
    }
    return { status: "available", version: update.version };
  } catch (e) {
    // 网络不可用 / 超时 / 服务器端元数据错误 / 签名清单格式错误——全部收敛为 error
    return { status: "error", error: String(e) };
  }
}

/** 自动检查入口：节流判定 → 检查 → 持久化时间戳。
 *  返回 outcome；不需要检查时返回 null（调用方不显示任何提醒）。 */
export async function autoCheckUpdate(
  now: number = Date.now()
): Promise<UpdateCheckOutcome | null> {
  const enabled = await readAutoCheckEnabled();
  const lastCheckAt = await readLastCheckAt();
  if (!shouldAutoCheck({ enabled, lastCheckAt, now })) {
    return null;
  }
  const outcome = await checkOnce();
  await persistLastCheckAt(now);
  return outcome;
}

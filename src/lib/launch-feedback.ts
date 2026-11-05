// ===== 统一启动反馈（方案甲验收 B 项，2026-11-05）=====
// 所有启动入口（软件区左键/双击/右键、常用页卡片、其他 launch_app 调用方）
// 共用同一套冷却与提示逻辑，避免"同一应用两种提示/重复启动"。
export const LAUNCH_COOLDOWN_MS = 2000;

// 模块级共享冷却表：跨页面统一防重复启动（软件区点完 2s 内常用页再点同一应用也会被拦）
const launchedAt = new Map<number, number>();

export function launchCoolingDown(id: number): boolean {
  const last = launchedAt.get(id);
  return last != null && Date.now() - last < LAUNCH_COOLDOWN_MS;
}
export function launchMarkAccepted(id: number): void {
  launchedAt.set(id, Date.now());
}

// ===== 启动序号守卫：旧请求迟到不得覆盖新提示 =====
let launchSeq = 0;
export function nextLaunchSeq(): number {
  return ++launchSeq;
}
export function isLatestLaunch(seq: number): boolean {
  return seq === launchSeq;
}


export interface LaunchTargetInfo {
  name?: string;
  path?: string;
  app_type?: string;
}

type ShowToast = (kind: "success" | "info", msg: string, durationMs?: number) => void;

/** 启动请求成功提交后的提示（区分 no-op 测试目标 / 网址 / 快捷方式 / 文件夹 / 程序） */
export function launchSuccessToast(show: ShowToast, item?: LaunchTargetInfo): void {
  const name = item?.name ?? "";
  const p = (item?.path || "").toLowerCase();
  // no-op 测试目标：瞬启即退、不打开任何真实程序——必须让用户知道，避免误判"点了没反应"
  if (/noop/i.test(p)) {
    show("success", `🧪 已启动测试目标（no-op，不打开真实程序）「${name}」`, 3000);
    return;
  }
  if (
    item?.app_type === "url" ||
    p.startsWith("http://") ||
    p.startsWith("https://") ||
    p.startsWith("steam://") ||
    p.startsWith("com.epicgames.launcher://") ||
    p.endsWith(".url")
  ) {
    show("success", `已在浏览器中打开「${name}」`, 3000);
    return;
  }
  if (p.endsWith(".lnk")) {
    show("success", `已启动「${name}」`, 3000);
    return;
  }
  if (item?.app_type === "folder") {
    show("success", `已打开文件夹「${name}」`, 3000);
    return;
  }
  show("success", `已启动「${name}」`, 3000);
}

/** 启动失败提示（missing = 路径不存在；其余按后端消息） */
export function launchFailureToast(
  show: ShowToast,
  item: LaunchTargetInfo | undefined,
  result: { error?: string; message?: string },
): void {
  if (result.error === "missing") {
    const p = (item?.path || "").toLowerCase();
    const isUrl =
      item && (item.app_type === "url" || p.startsWith("http://") || p.startsWith("https://") || p.startsWith("steam://"));
    if (isUrl) {
      show("info", `网页打开失败：${item?.path}`, 3500);
      return;
    }
    if (item && p.endsWith(".url")) {
      show("info", `「${item?.name ?? ""}」网页快捷方式已失效。已尝试启动 Steam，请在库中找到该游戏`, 3500);
      return;
    }
    show("info", `找不到「${item?.name ?? ""}」的安装位置。请重新拖入 .exe 或 .lnk`, 3500);
    return;
  }
  show("info", result.message || "启动失败", 3500);
}

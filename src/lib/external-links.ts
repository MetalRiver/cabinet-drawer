// ===== v0.4.1 任务 B：外部链接打开（错误必须可见，绕开静默失败路径）=====
import { openExternalUrl as apiOpenExternalUrl } from "../api";

export type ExternalLinkInvoker = (url: string) => Promise<void>;

/** 默认走 Rust open_external_url（系统默认浏览器；失败返回可见错误） */
const defaultInvoker: ExternalLinkInvoker = (url) => apiOpenExternalUrl(url);

/**
 * 打开外部 http(s) 链接。
 * - 非 http(s) 直接拒绝（前端兜底，后端同规则二次防御）；
 * - 任何失败抛出含 URL 的可见错误（调用方展示到 StatusLine/toast，绝不静默）。
 */
export async function openExternalLink(
  url: string,
  invoker: ExternalLinkInvoker = defaultInvoker
): Promise<void> {
  if (!/^https?:\/\//i.test(url)) {
    throw new Error(`已阻止打开非 http(s) 链接：${url}`);
  }
  try {
    await invoker(url);
  } catch (e) {
    throw new Error(`外部链接打开失败：${url}（${e}）`);
  }
}

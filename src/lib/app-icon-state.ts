/**
 * M1-P2（F1+F4）：软件图标统一状态模块——AppView 与 FrequentView 的唯一图标加载/失败/恢复机制。
 *
 * 设计要点（对应 设计-M1图标全链路归因与修复设计-2026-11-07.md F1/F4）：
 *  1. 状态按 (id, 当前 icon_path, attempt 版本) 区分：
 *     - icon_path 变化（重抽/重基/换图标）→ 旧失败状态自动作废；
 *     - attempt 递增 → asset URL 附加 ?v=N → 浏览器视为新资源重新请求
 *       （Tauri asset 协议按 uri().path() 解析、忽略 query——URL 变化不改变目标文件，
 *        确保"只清标记仍显示浏览器缓存旧位图"不会发生）。
 *  2. 失败处理链（两视图一致）：asset URL 失败 → 标记 assetFailed（模板切到 data URL/SVG 分支）
 *     → 受控 data URL 回退（单飞，防旧结果覆盖）→ 回退也失败 → SVG/emoji 兜底
 *     + 安排【单次】30s 延迟恢复重试。
 *  3. 恢复闭环（E1 反演）：延迟重试调用 fill_missing_icons（后端把缺失/失效图标重抽/重基到
 *     当前缓存根）→ attempt++ 使该 icon 的 URL 变化 → img 重新请求 → 文件已恢复则直接显示。
 *     重试每恢复周期最多一次，不成功则停在兜底态等待下一次挂载/手动刷新——无全局轮询、无无限重试。
 *  4. 健康图标不受刷新影响：attempt 只对失败过的 id 递增，其 asset URL 不变，不触发重读。
 *  5. 不扩大文件读取权限：回退仅走 M0 受控接口 get_app_icon_data_url（锁定态拒绝+缓存根校验）。
 */
import { ref } from "vue";
import { convertFileSrc } from "@tauri-apps/api/core";
import { getAppIconDataUrl, fillMissingIcons } from "../api";

interface IconFailState {
  /** 当前 icon_path（路径变化检测——路径变了旧状态即作废） */
  path: string;
  /** asset URL 是否已失败（模板据此从 asset 分支切到 data URL/SVG 分支） */
  assetFailed: boolean;
  /** 受控 data URL 回退是否也失败（true=SVG/emoji 兜底） */
  fallbackFailed: boolean;
  /** 资源版本：随恢复尝试递增，附加到 asset URL 强制重新请求 */
  attempt: number;
  /** 30s 延迟恢复重试：pending=已安排未执行；done=已执行（每恢复周期最多一次） */
  recoveryPending: boolean;
  recoveryDone: boolean;
}

/** 受控回退缓存：id → data URL（G3-1C 语义保留；模板/性能探针经 store 引用） */
export const iconDataUrls = ref<Record<number, string>>({});
/** 失败状态表：id → 状态 */
const failures = ref<Record<number, IconFailState>>({});

const inflight = new Set<number>();
let requestSeq = 0;
const seqByIcon = new Map<number, number>();

function ensure(id: number, path: string): IconFailState {
  const cur = failures.value[id];
  if (!cur || cur.path !== path) {
    // 新条目或路径变化 → 旧失败状态作废（F1：更新图标后旧错误状态必须失效）
    const fresh: IconFailState = {
      path,
      assetFailed: false,
      fallbackFailed: false,
      attempt: cur?.attempt ?? 0,
      recoveryPending: false,
      recoveryDone: false,
    };
    failures.value = { ...failures.value, [id]: fresh };
    return fresh;
  }
  return cur;
}

/** asset URL 主路径：路径变化检测 + attempt 版本参数（健康 icon 的 URL 稳定不变，不触发重读） */
export function assetUrl(id: number, iconPath: string | null | undefined): string {
  if (!iconPath) return "";
  ensure(id, iconPath);
  const base = convertFileSrc(iconPath);
  // attempt=0 时不加参数（URL 与历史一致）；失败恢复后 attempt>0 强制浏览器重新请求
  return failures.value[id] && failures.value[id].attempt > 0 ? `${base}?v=${failures.value[id].attempt}` : base;
}

/** asset 分支是否应让位（模板 v-if：asset img 仅在未失败时渲染） */
export function assetFailed(id: number): boolean {
  const st = failures.value[id];
  return !!st && st.assetFailed;
}

/**
 * asset URL 加载失败入口（AppView/FrequentView @error 统一走这里）。
 * 同一 (id,path,attempt) 只处理一次；受控回退单飞 + 序号防旧结果覆盖新状态。
 */
export function onAssetError(id: number, iconPath: string | null | undefined): void {
  if (!iconPath) return;
  const st = ensure(id, iconPath);
  if (st.assetFailed && st.fallbackFailed) return; // 已进入兜底/恢复流程
  if (!st.assetFailed) {
    failures.value = { ...failures.value, [id]: { ...st, assetFailed: true } };
  }
  if (inflight.has(id)) return;
  inflight.add(id);
  const seq = ++requestSeq;
  seqByIcon.set(id, seq);
  void (async () => {
    try {
      const url = await getAppIconDataUrl(id);
      // 防旧覆盖：仅当该 icon 未发生路径/版本变化时才采纳结果
      const cur = failures.value[id];
      if (!cur || cur.path !== iconPath || seqByIcon.get(id) !== seq) return;
      if (url) {
        iconDataUrls.value = { ...iconDataUrls.value, [id]: url };
      } else {
        markFallbackFailed(id, iconPath);
      }
    } catch {
      const cur = failures.value[id];
      if (cur && cur.path === iconPath && seqByIcon.get(id) === seq) markFallbackFailed(id, iconPath);
    } finally {
      inflight.delete(id);
    }
  })();
}

function markFallbackFailed(id: number, path: string): void {
  const st = ensure(id, path);
  failures.value = { ...failures.value, [id]: { ...st, fallbackFailed: true } };
  if (iconDataUrls.value[id] !== undefined) {
    const { [id]: _drop, ...rest } = iconDataUrls.value;
    iconDataUrls.value = rest;
  }
  scheduleRecoveryOnce(id, path);
}

/**
 * F4 恢复闭环：受控回退也失败后，安排【单次】30s 延迟恢复重试。
 * - 只调一次 fill_missing_icons（事件驱动，非轮询）——后端重抽缺失图标/重基失效路径；
 * - 重试周期结束时**无条件**对该 icon 强制重新请求一次（attempt++ URL 版本化）：
 *   ① fill>0（后端重抽成功）→ 新请求 200 恢复；
 *   ② fill=0 但文件已被外部恢复（U盘插回/重装等——fill 按 icon_path 存在性跳过）→ 新请求 200 恢复；
 *   ③ fill=0 且文件仍缺失 → 新请求 404 → 回到兜底态（recoveryDone=true 不再自动重试，
 *      等待下一次挂载或手动刷新）。
 * 404 响应不被 HTTP 缓存，重建的 img 元素的新请求可命中已恢复的文件。
 */
function scheduleRecoveryOnce(id: number, path: string): void {
  const st = failures.value[id];
  if (!st || st.recoveryPending || st.recoveryDone) return;
  failures.value = { ...failures.value, [id]: { ...st, recoveryPending: true } };
  setTimeout(() => {
    void (async () => {
      const cur = failures.value[id];
      // 重试前校验：仍处于同一失败态（未被路径变化/手动刷新处理过）
      if (!cur || cur.path !== path || !cur.recoveryPending) return;
      failures.value = { ...failures.value, [id]: { ...cur, recoveryPending: false, recoveryDone: true } };
      try {
        await fillMissingIcons();
      } catch {
        return; // 后端不可用 → 停在兜底态
      }
      recoverIcon(id, path);
    })();
  }, 30_000);
}

/** 恢复单个 icon：attempt++（URL 版本变化）+ 清失败态 + 清旧 data URL（重新走 asset 主路径） */
function recoverIcon(id: number, path: string): void {
  const cur = failures.value[id];
  const base: IconFailState =
    cur && cur.path === path
      ? cur
      : { path, assetFailed: false, fallbackFailed: false, attempt: 0, recoveryPending: false, recoveryDone: false };
  failures.value = {
    ...failures.value,
    [id]: { ...base, attempt: base.attempt + 1, assetFailed: false, fallbackFailed: false, recoveryPending: false },
  };
  if (iconDataUrls.value[id] !== undefined) {
    const { [id]: _drop, ...rest } = iconDataUrls.value;
    iconDataUrls.value = rest;
  }
}

/** F4：图标批量变化完成点（fillMissingIcons/refreshAllIcons/forceReextract 返回后调用）。
 *  n>0 表示后端图标有实际变化 → 所有失败过的 icon attempt++（URL 变化重新请求）；
 *  n=0 时仅清失败标记不动 URL——文件未变，无需重读（避免反复读取全部图标）。 */
export function bumpAfterIconRefresh(n: number): void {
  for (const idStr of Object.keys(failures.value)) {
    const id = Number(idStr);
    const st = failures.value[id];
    if (!st) continue;
    if (n > 0) {
      failures.value = {
        ...failures.value,
        [id]: { ...st, attempt: st.attempt + 1, assetFailed: false, fallbackFailed: false, recoveryPending: false, recoveryDone: true },
      };
      if (iconDataUrls.value[id] !== undefined) {
        const { [id]: _drop, ...rest } = iconDataUrls.value;
        iconDataUrls.value = rest;
      }
    } else {
      failures.value = { ...failures.value, [id]: { ...st, assetFailed: false, fallbackFailed: false, recoveryPending: false } };
    }
  }
}

/** 回退 data URL 也加载失败（AppView onFallbackIconError 语义保留） */
export function dropFallback(id: number): void {
  if (iconDataUrls.value[id] === undefined) return;
  const { [id]: _drop, ...rest } = iconDataUrls.value;
  iconDataUrls.value = rest;
  const st = failures.value[id];
  if (st) failures.value = { ...failures.value, [id]: { ...st, fallbackFailed: true } };
}

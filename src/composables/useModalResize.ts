// R6-02C-07（弹窗尺寸统一）：弹窗拖拽调尺寸 + 尺寸记忆。
// 机制：CSS resize:both（.modal-resizable）提供拖拽把柄；ResizeObserver 捕获用户拖拽
// 后的尺寸，按 key 存 localStorage（非敏感 UI 偏好，无用户内容）；下次打开同 key 弹窗时恢复。
// 约束：min/max 由 CSS 控制（不超出应用可视区域）；窄窗（≤419px）由 CSS 禁用恢复，
// 响应式布局接管。尺寸数据不含任何用户内容，无安全敏感面。
const STORAGE_PREFIX = "drawer-modal-size:";

export function applySavedModalSize(el: HTMLElement | null, key: string): void {
  if (!el) return;
  try {
    const raw = localStorage.getItem(STORAGE_PREFIX + key);
    if (!raw) return;
    const saved = JSON.parse(raw) as { w: number; h: number };
    if (saved.w >= 280 && saved.h >= 320) {
      el.style.width = saved.w + "px";
      el.style.height = saved.h + "px";
    }
  } catch {
    /* localStorage 不可用或数据损坏：忽略，走默认尺寸 */
  }
}

export function observeModalSize(el: HTMLElement | null, key: string): () => void {
  if (!el || typeof ResizeObserver === "undefined") return () => {};
  let lastSig = "";
  const observer = new ResizeObserver((entries) => {
    const rect = entries[0].contentRect;
    const w = Math.round(rect.width);
    const h = Math.round(rect.height);
    const sig = w + "x" + h;
    if (sig !== lastSig && w > 0 && h > 0) {
      lastSig = sig;
      try {
        localStorage.setItem(STORAGE_PREFIX + key, JSON.stringify({ w, h }));
      } catch {
        /* 忽略存储失败 */
      }
    }
  });
  observer.observe(el);
  return () => observer.disconnect();
}

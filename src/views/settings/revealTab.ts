// ===== Settings UI V2 P0 修复：分类导航独立滚动 =====
// 根因回归：Element.scrollIntoView({inline:"center"}) 会级联滚动**所有可滚祖先**，
// 包括 MainLayout 的 .content-scroll（overflow-x:hidden 仍是可编程滚动盒）与应用根，
// 导致点击右侧分类时整个应用左移、标题/搜索框/底部导航被裁切。
// 修复原则：只允许导航容器自己改变 scrollLeft——用 offsetLeft 精确计算目标位置，
// 仅对导航元素调用 scrollTo；绝不触碰文档或祖先滚动状态。

export interface NavMetrics {
  clientWidth: number;
  scrollWidth: number;
}

/** 计算把按钮滚到导航视野中央所需的 scrollLeft（含边界钳制）；无可滚空间返回 0 */
export function computeRevealLeft(
  nav: NavMetrics,
  btn: { offsetLeft: number; offsetWidth: number }
): number {
  const max = Math.max(0, nav.scrollWidth - nav.clientWidth);
  if (max === 0) return 0;
  const target = btn.offsetLeft - (nav.clientWidth - btn.offsetWidth) / 2;
  return Math.max(0, Math.min(target, max));
}

/**
 * 把指定分类滚入导航容器视野（横向滚动导航；左导航竖排模式处理纵向）。
 * 只对传入的 nav 元素执行 scrollTo；不使用 scrollIntoView，不影响任何祖先。
 */
export function revealNavTab(nav: HTMLElement, key: string): void {
  const btn = nav.querySelector<HTMLElement>(`[data-tab-key="${key}"]`);
  if (!btn) return;
  const hasHorizontal = nav.scrollWidth - nav.clientWidth > 1;
  const hasVertical = nav.scrollHeight - nav.clientHeight > 1;
  if (!hasHorizontal && !hasVertical) return;
  if (typeof nav.scrollTo !== "function") {
    // jsdom/旧内核兜底：直接赋值同样只影响该元素自身滚动状态
    if (hasHorizontal) nav.scrollLeft = computeRevealLeft(nav, btn);
    return;
  }
  if (hasHorizontal) {
    nav.scrollTo({ left: computeRevealLeft(nav, btn), behavior: "smooth" });
  }
  if (hasVertical) {
    const maxV = Math.max(0, nav.scrollHeight - nav.clientHeight);
    const targetV = btn.offsetTop - (nav.clientHeight - btn.offsetHeight) / 2;
    nav.scrollTo({ top: Math.max(0, Math.min(targetV, maxV)), behavior: "smooth" });
  }
}

/** 更新左右滚动提示（渐隐遮罩）的可见性；仅读取导航自身滚动状态 */
export function updateScrollHints(
  nav: HTMLElement
): { left: boolean; right: boolean } {
  const max = nav.scrollWidth - nav.clientWidth;
  if (max <= 1) return { left: false, right: false };
  return {
    left: nav.scrollLeft > 1,
    right: nav.scrollLeft < max - 1,
  };
}

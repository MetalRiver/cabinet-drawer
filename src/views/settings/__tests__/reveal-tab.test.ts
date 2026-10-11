// @vitest-environment jsdom
// ===== Settings UI V2 P0 回归：分类导航独立滚动，根容器零水平位移 =====
// 用户实机截图证据：scrollIntoView 级联滚动应用根 → 整页左移裁切。
// 本测试锁定：revealNavTab 只对导航元素执行 scrollTo（含祖先 spy 证明），
// 文档/body 水平滚动状态保持 0；窄宽度下选中项进入导航可视范围。

import { describe, it, expect, vi, beforeEach } from "vitest";
import { computeRevealLeft, revealNavTab, updateScrollHints } from "../revealTab";

const KEYS = ["basic", "security", "window", "data", "advanced"];
const LABELS: Record<string, number> = {
  basic: 90, security: 120, window: 120, data: 130, advanced: 130,
};

function buildNav(containerWidth: number): { nav: HTMLElement; buttons: HTMLElement[] } {
  const nav = document.createElement("div");
  Object.defineProperty(nav, "clientWidth", { value: containerWidth, configurable: true });
  Object.defineProperty(nav, "scrollWidth", {
    get: () => Array.from(nav.children).reduce((acc, c) => acc + (c as HTMLElement).offsetWidth, 0),
    configurable: true,
  });
  let x = 0;
  for (const key of KEYS) {
    const b = document.createElement("button");
    b.setAttribute("data-tab-key", key);
    Object.defineProperty(b, "offsetLeft", { value: x, configurable: true });
    Object.defineProperty(b, "offsetWidth", { value: LABELS[key], configurable: true });
    x += LABELS[key];
    nav.appendChild(b);
  }
  return { nav, buttons: Array.from(nav.children) as HTMLElement[] };
}

beforeEach(() => {
  document.body.innerHTML = "";
  document.documentElement.scrollLeft = 0;
  document.body.scrollLeft = 0;
  // jsdom 未实现 Element.scrollTo：先装可覆写桩，供 spyOn 使用
  if (typeof HTMLElement.prototype.scrollTo !== "function") {
    Object.defineProperty(HTMLElement.prototype, "scrollTo", {
      value: function scrollTo() {},
      writable: true,
      configurable: true,
    });
  }
});

/** spy scrollTo（兼容 (options) 与 (x, y) 两种重载），并让选项生效到 scrollLeft */
function spyScrollTo() {
  const spy = vi
    .spyOn(HTMLElement.prototype, "scrollTo")
    .mockImplementation(function (this: HTMLElement, x?: number | ScrollToOptions) {
      const opts = (typeof x === "object" ? x : undefined) as ScrollToOptions | undefined;
      if (opts && typeof opts.left === "number") this.scrollLeft = opts.left;
      return undefined;
    });
  return spy;
}

describe("computeRevealLeft（纯计算）", () => {
  it("无可滚空间返回 0；居中目标越界被钳制", () => {
    expect(computeRevealLeft({ clientWidth: 600, scrollWidth: 580 }, { offsetLeft: 0, offsetWidth: 100 })).toBe(0);
    // 居中目标 = offsetLeft - (clientWidth - w)/2
    expect(
      computeRevealLeft({ clientWidth: 480, scrollWidth: 590 }, { offsetLeft: 460, offsetWidth: 130 })
    ).toBe(Math.max(0, Math.min(460 - (480 - 130) / 2, 590 - 480)));
  });
});

describe("revealNavTab（DOM 行为）", () => {
  it("只对导航元素调用 scrollTo，绝级联滚动文档/祖先（P0 根因回归锁定）", () => {
    const { nav } = buildNav(480); // 总宽 590 > 480 → 横向可滚
    document.body.appendChild(nav);
    const protoSpy = spyScrollTo();
    revealNavTab(nav, "advanced");
    // 所有 scrollTo 调用的 this 都必须是 nav 本身
    expect(protoSpy.mock.instances.length).toBeGreaterThan(0);
    for (const inst of protoSpy.mock.instances) {
      expect(inst).toBe(nav);
    }
    // 文档水平滚动状态保持 0
    expect(document.documentElement.scrollLeft).toBe(0);
    expect(document.body.scrollLeft).toBe(0);
    protoSpy.mockRestore();
  });

  it("点击最右侧分类后，该标签进入导航可视范围（精确计算，不依赖 scrollIntoView）", () => {
    const { nav } = buildNav(480);
    document.body.appendChild(nav);
    spyScrollTo();
    revealNavTab(nav, "advanced");
    const btn = nav.querySelector<HTMLElement>('[data-tab-key="advanced"]')!;
    // 按钮右缘不得超出导航可视右界
    expect(btn.offsetLeft + btn.offsetWidth).toBeLessThanOrEqual(nav.scrollLeft + nav.clientWidth + 0.5);
    // 选中项居中目标：scrollLeft = offsetLeft - (clientWidth - offsetWidth)/2，且被钳制
    expect(nav.scrollLeft).toBe(
      Math.max(0, Math.min(460 - (480 - 130) / 2, 590 - 480))
    );
    vi.restoreAllMocks();
  });

  it("不可滚动时不产生任何滚动调用", () => {
    const { nav } = buildNav(900); // 590 < 900
    document.body.appendChild(nav);
    const spy = spyScrollTo();
    revealNavTab(nav, "advanced");
    expect(spy).not.toHaveBeenCalled();
    spy.mockRestore();
  });
});

describe("根容器不发生水平滚动（文档级断言）", () => {
  it("倒序切换全部分类后，document/body scrollLeft 恒为 0", () => {
    const { nav } = buildNav(320); // 最窄：滚动量最大
    document.body.appendChild(nav);
    spyScrollTo();
    for (const key of [...KEYS].reverse()) {
      revealNavTab(nav, key);
    }
    expect(document.documentElement.scrollLeft).toBe(0);
    expect(document.body.scrollLeft).toBe(0);
    vi.restoreAllMocks();
  });
});

describe("updateScrollHints（左右渐隐提示）", () => {
  it("最左：只亮右侧；滚到最右：只亮左侧；不可滚：全灭", () => {
    const nav = {
      scrollLeft: 0,
      clientWidth: 480,
      scrollWidth: 590,
    } as HTMLElement;
    Object.defineProperty(nav, "scrollLeft", { value: 0, writable: true, configurable: true });
    Object.defineProperty(nav, "clientWidth", { value: 480, configurable: true });
    Object.defineProperty(nav, "scrollWidth", { value: 590, configurable: true });
    expect(updateScrollHints(nav)).toEqual({ left: false, right: true });
    nav.scrollLeft = 110;
    expect(updateScrollHints(nav)).toEqual({ left: true, right: false });
    Object.defineProperty(nav, "scrollWidth", { value: 480, configurable: true });
    expect(updateScrollHints(nav)).toEqual({ left: false, right: false });
  });
});

// @vitest-environment jsdom
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { createApp, defineComponent, h, KeepAlive, nextTick, ref, type App } from "vue";
import { createPinia } from "pinia";
import { usePasswordStore } from "../../stores/passwords";
import PasswordView from "../PasswordView.vue";

// 只 mock IPC/副作用边界（../api）与剪贴板倒计时；stores 用真实 Pinia 实例，
// 挂载真实密码区 UI，驱动真实行内状态机（expanded/editing/showPassword/草稿）。
vi.mock("../../../api", () => ({
  copyToClipboardWithTimeout: vi.fn(async () => {}),
  generateRandomPassword: vi.fn(() => "Abcdef12"),
  passwordStrength: vi.fn(async () => 50),
  verifyPasswordForPwView: vi.fn(async () => true),
  hasSecondPassword: vi.fn(async () => false),
  getSecurityStatus: vi.fn(async () => ({ security_model: "stable_dek_v2" })),
}));
vi.mock("../../../composables/useClipboardCountdown", () => ({
  useClipboardCountdown: () => ({ start: vi.fn(), seconds: vi.fn(() => 0) }),
}));

const seedItems = [
  { id: 1, title: "GitHub", username: "me@example.com", url: "https://github.com", notes: "", created_at: 1700000000000, updated_at: 1700000000000 },
  { id: 2, title: "Bank", username: "banker", url: "https://bank.example", notes: "2FA", created_at: 1700000000001, updated_at: 1700000000001 },
];

let app: App | undefined;
let host: HTMLElement | undefined;
let pwExposed: any; // PasswordView defineExpose 的内容
let showPwd: ReturnType<typeof ref<boolean>>;

// 父组件：KeepAlive 包裹密码区，showPwd 切换「在密码区 / 离开」，
// 切走时 Vue 真实触发 PasswordView 的 onDeactivated（比直接调钩子更贴近真机）。
const Host = defineComponent({
  setup() {
    showPwd = ref(true);
    return () =>
      h("div", [
        h(KeepAlive, () => (showPwd.value ? h(PasswordView) : h("div", { class: "placeholder" }))),
      ]);
  },
});

async function mountView() {
  host = document.createElement("div");
  document.body.appendChild(host);
  app = createApp(Host);
  app.use(createPinia());
  app.mount(host);
  await Promise.resolve();
  await nextTick();
  // 用隔离测试数据填充真实 store（refresh 走 IPC 已被 mock，这里直接播种）
  const passwordStore = usePasswordStore();
  passwordStore.items = seedItems.map((it) => ({ ...it }));
  passwordStore.hasLoaded = true;
  await nextTick();
  // 拿到 PasswordView 实例（defineExpose 内容挂在 instance.exposed）
  const grid = host!.querySelector(".pwd-grid") as any;
  pwExposed = grid?.__vueParentComponent?.exposed;
  return pwExposed;
}

function getRow(id: number) {
  return pwExposed.rowStates[id];
}

async function leaveAndReturn() {
  showPwd.value = false; // 切走 → 真实触发 onDeactivated
  await nextTick();
  showPwd.value = true; // 切回
  await nextTick();
}

beforeAll(() => {
  Element.prototype.scrollIntoView = vi.fn();
});

afterEach(() => {
  app?.unmount();
  app = undefined;
  host?.remove();
  host = undefined;
  document.body.innerHTML = "";
});

describe("R3 密码区：离开自动收起 + 明文隐藏 + 草稿保留", () => {
  it("expanded 行离开密码区后收起并隐藏明文", async () => {
    await mountView();
    (host!.querySelectorAll(".pwd-tile")[0] as HTMLElement).click();
    await nextTick();
    expect(getRow(1).mode).toBe("expanded");
    getRow(1).decryptedPassword = "secret-1";
    getRow(1).showPassword = true;

    await leaveAndReturn();

    expect(getRow(1).mode).toBe("collapsed");
    expect(getRow(1).showPassword).toBe(false);
  });

  it("editing 行离开后保留草稿但强制隐藏明文", async () => {
    await mountView();
    (host!.querySelectorAll(".pwd-tile")[1] as HTMLElement).click();
    await nextTick();
    pwExposed.enterEdit(seedItems[1]);
    await nextTick();
    expect(getRow(2).mode).toBe("editing");
    getRow(2).form.password = "draft-new-pwd";
    getRow(2).showPassword = true;

    await leaveAndReturn();

    expect(getRow(2).mode).toBe("editing");
    expect(getRow(2).form.password).toBe("draft-new-pwd");
    expect(getRow(2).showPassword).toBe(false);
  });

  it("多卡互斥：展开第二张时第一张自动收起", async () => {
    await mountView();
    const tiles = host!.querySelectorAll(".pwd-tile");
    (tiles[0] as HTMLElement).click();
    await nextTick();
    expect(getRow(1).mode).toBe("expanded");
    (tiles[1] as HTMLElement).click();
    await nextTick();
    expect(getRow(1).mode).toBe("collapsed");
    expect(getRow(2).mode).toBe("expanded");
  });
});

describe("R3 密码区：详情卡双击收起 + 内部交互隔离", () => {
  it("双击详情卡背景收起", async () => {
    await mountView();
    (host!.querySelectorAll(".pwd-tile")[0] as HTMLElement).click();
    await nextTick();
    const card = host!.querySelector(".pwd-detail-card") as HTMLElement;
    expect(getRow(1).mode).toBe("expanded");
    card.dispatchEvent(new MouseEvent("dblclick", { bubbles: true }));
    await nextTick();
    expect(getRow(1).mode).toBe("collapsed");
  });

  it("双击详情卡内部按钮不触发收起（复制/编辑/删除/关闭/显示）", async () => {
    await mountView();
    (host!.querySelectorAll(".pwd-tile")[0] as HTMLElement).click();
    await nextTick();
    getRow(1).decryptedPassword = "secret-1";

    const card = host!.querySelector(".pwd-detail-card") as HTMLElement;
    const interactive = Array.from(
      card.querySelectorAll("button, input, textarea, a"),
    ) as HTMLElement[];
    expect(interactive.length).toBeGreaterThan(0);
    for (const el of interactive) {
      el.dispatchEvent(new MouseEvent("dblclick", { bubbles: false }));
    }
    await nextTick();
    expect(getRow(1).mode).toBe("expanded");
  });

  it("编辑态双击详情卡背景不收起（豁免，保护草稿）", async () => {
    await mountView();
    (host!.querySelectorAll(".pwd-tile")[0] as HTMLElement).click();
    await nextTick();
    pwExposed.enterEdit(seedItems[0]);
    await nextTick();
    expect(getRow(1).mode).toBe("editing");
    const card = host!.querySelector(".pwd-detail-card") as HTMLElement;
    card.dispatchEvent(new MouseEvent("dblclick", { bubbles: true }));
    await nextTick();
    expect(getRow(1).mode).toBe("editing");
  });
});
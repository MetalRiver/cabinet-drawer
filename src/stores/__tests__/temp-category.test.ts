// @vitest-environment jsdom
// ===== Batch 4：便签 store —— 分类筛选 / 标题搜索 / CRUD 透传 =====
import { describe, it, expect, vi, beforeEach } from "vitest";
import { createPinia, setActivePinia } from "pinia";

const apiCalls: string[] = [];
const mockTemps = [
  { id: 1, text: "工作正文", created_at: 1, expires_at: 9_999_999_999_999, title: "会议纪要", category_id: 10, category_name: "工作" },
  { id: 2, text: "生活正文", created_at: 2, expires_at: 9_999_999_999_999, title: undefined, category_id: 11, category_name: "生活" },
  { id: 3, text: "未分类正文", created_at: 3, expires_at: 9_999_999_999_999, title: undefined, category_id: undefined, category_name: undefined },
];
const mockCats = [
  { id: 10, name: "工作" },
  { id: 11, name: "生活" },
];

vi.mock("../../api", () => ({
  listTemp: vi.fn(async () => mockTemps),
  listTempCategories: vi.fn(async () => mockCats),
  createTempCategory: vi.fn(async (name: string) => {
    apiCalls.push(`createCat:${name}`);
    return 99;
  }),
  renameTempCategory: vi.fn(async () => {}),
  deleteTempCategory: vi.fn(async (id: number) => {
    apiCalls.push(`deleteCat:${id}`);
    return 1;
  }),
  setTempCategory: vi.fn(async (id: number, cid: number | null) => {
    apiCalls.push(`setCat:${id}:${cid}`);
  }),
  updateTempTitle: vi.fn(async (id: number, title: string | null) => {
    apiCalls.push(`setTitle:${id}:${title}`);
  }),
  createTemp: vi.fn(async (text: string, ttl: number, title?: string) => {
    apiCalls.push(`create:${text}:${title ?? ""}`);
    return 1;
  }),
  deleteTemp: vi.fn(async () => {}),
  cleanupExpiredTemp: vi.fn(async () => 0),
  updateTempText: vi.fn(async () => {}),
  renewTemp: vi.fn(async () => 0),
  isTempPermanent: () => false,
  copyToClipboardWithTimeout: vi.fn(async () => {}),
}));

import { useTempStore } from "../temp";

beforeEach(() => {
  setActivePinia(createPinia());
  apiCalls.length = 0;
  vi.clearAllMocks();
});

describe("Batch 4：便签分类与搜索", () => {
  it("loadCategories 载入分类列表", async () => {
    const store = useTempStore();
    await store.loadCategories();
    expect(store.categories).toHaveLength(2);
    expect(store.categories[0].name).toBe("工作");
  });

  it("分类筛选：activeCategoryId 过滤对应便签", async () => {
    const store = useTempStore();
    await store.loadItems();
    store.activeCategoryId = 10;
    expect(store.filteredItems.map((x) => x.id)).toEqual([1]);
    store.activeCategoryId = 11;
    expect(store.filteredItems.map((x) => x.id)).toEqual([2]);
  });

  it("未分类筛选：showUncategorized 只留无分类便签", async () => {
    const store = useTempStore();
    await store.loadItems();
    store.showUncategorized = true;
    expect(store.filteredItems.map((x) => x.id)).toEqual([3]);
  });

  it("搜索覆盖标题与正文", async () => {
    const store = useTempStore();
    await store.loadItems();
    store.searchQuery = "会议纪要"; // 标题命中
    expect(store.filteredItems.map((x) => x.id)).toEqual([1]);
    store.searchQuery = "生活正文"; // 正文命中
    expect(store.filteredItems.map((x) => x.id)).toEqual([2]);
    store.searchQuery = "不存在";
    expect(store.filteredItems).toHaveLength(0);
  });

  it("create 透传标题（空标题转 undefined 语义由 api 层处理）", async () => {
    const store = useTempStore();
    await store.create("正文", 60, "我的标题");
    expect(apiCalls.some((c) => c === "create:正文:我的标题")).toBe(true);
  });

  it("updateTitle / setCategory / deleteCategory 透传并刷新", async () => {
    const store = useTempStore();
    await store.loadItems();
    await store.updateTitle(1, null);
    await store.setCategory(1, null);
    await store.deleteCategory(10);
    expect(apiCalls).toContain("setTitle:1:null");
    expect(apiCalls).toContain("setCat:1:null");
    expect(apiCalls).toContain("deleteCat:10");
    // 删除当前选中分类后筛选复位
    expect(store.activeCategoryId).toBeNull();
  });
});

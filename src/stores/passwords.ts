import { defineStore } from "pinia";
import { ref } from "vue";
import { useAppStore } from "./app";
import {
  listPasswords,
  createPassword,
  updatePassword,
  deletePassword,
  getPasswordDecrypted,
  bumpPasswordUseCount,
  type PasswordMeta,
} from "../api";

export const usePasswordStore = defineStore("passwords", () => {
  const items = ref<PasswordMeta[]>([]);
  const loading = ref(false);
  const error = ref("");

  // 内部状态：上次是否成功加载过（用于 UI 文案：未加载 vs 已加载但为空）
  const hasLoaded = ref(false);

  // P1-#PW#USE#PERSIST：useCount 来自后端（与 apps / snippets 一致）
  // 之前：纯前端 in-memory 计数 → 刷新页面就丢
  // 现在：bumpUseCount 调后端 IPC 累加，items[i].use_count 自动反映
  // 保留 in-memory useCounts 仅作为乐观更新（立即 +1，等后端返回）
  const useCounts = ref<Record<number, number>>({});

  function bumpUseCount(id: number) {
    // 1. 乐观更新：立即 +1 让 UI 立刻有反馈
    useCounts.value = { ...useCounts.value, [id]: (useCounts.value[id] || 0) + 1 };
    // 2. 后端持久化累加
    bumpPasswordUseCount(id)
      .then((realCount) => {
        // 用后端真实值校正（避免乐观更新偏差）
        useCounts.value = { ...useCounts.value, [id]: realCount };
        // 同步到 items 数组（让刷新页面后还能看到正确的 useCount）
        const it = items.value.find((i) => i.id === id);
        if (it) it.use_count = realCount;
      })
      .catch((e) => {
        console.warn("[passwords] bump use count failed", e);
      });
  }

  function getUseCount(id: number): number {
    // 优先用乐观值（in-memory），否则从 items 里读后端持久化的值
    if (useCounts.value[id] !== undefined) return useCounts.value[id];
    const it = items.value.find((i) => i.id === id);
    return it?.use_count ?? 0;
  }

  async function refresh() {
    // 关键修复：loading=true 不再阻塞 UI，view 始终可交互
    // view 层只看是否为空显示空态，不再被 loading 遮盖
    if (loading.value) return; // 防并发
    loading.value = true;
    error.value = "";
    try {
      // 加超时：10s 后强制结束（避免 IPC 卡死导致 UI 永远转）
      const timeoutPromise = new Promise<never>((_, reject) =>
        setTimeout(() => reject(new Error("加载超时（10s）")), 10000)
      );
      const list = await Promise.race([listPasswords(), timeoutPromise]);
      items.value = list;
      hasLoaded.value = true;
    } catch (e) {
      error.value = String(e);
      console.error("[passwords] refresh failed:", e);
      // P0-#LOCK#AUTO#REDIRECT：后端返回"应用已锁定"时自动跳转到锁屏
      // 兜底：万一 app:lock 事件丢失（理论上不会），用户点密码区时也能自动回到锁屏
      // 否则用户卡在错误条 + 重试按钮没用（永远返回同一个错误）
      const msg = String(e);
      if (msg.includes("应用已锁定") || msg.includes("locked")) {
        useAppStore().lock();
      }
    } finally {
      loading.value = false;
    }
  }

  async function add(params: {
    title: string;
    username: string;
    password: string;
    url: string;
    notes: string;
  }) {
    await createPassword(params);
    await refresh();
  }

  async function edit(
    id: number,
    params: {
      title: string;
      username: string;
      /** P0-B 契约：缺省 = 本次不修改密码（不传 password 字段） */
      password?: string;
      url: string;
      notes: string;
    }
  ) {
    await updatePassword({ id, ...params });
    await refresh();
  }

  async function remove(id: number) {
    await deletePassword(id);
    // P1-#PW#USE#PERSIST：删除时清理 useCounts
    if (useCounts.value[id] !== undefined) {
      const { [id]: _, ...rest } = useCounts.value;
      useCounts.value = rest;
    }
    await refresh();
    useAppStore().refreshTrashCount();
  }

  async function reveal(id: number): Promise<string> {
    try {
      return await getPasswordDecrypted(id);
    } catch (e) {
      // P0-#LOCK#AUTO#REDIRECT：recover / copy 时碰到锁定也要自动跳锁屏
      const msg = String(e);
      if (msg.includes("应用已锁定") || msg.includes("locked")) {
        useAppStore().lock();
      }
      throw e;
    }
  }

  // 导入备份后整体刷新：清空 useCounts 缓存 + 重新拉列表
  async function reloadAll() {
    useCounts.value = {};
    await refresh();
  }

  return {
    items,
    loading,
    error,
    hasLoaded,
    useCounts,
    refresh,
    reloadAll,
    add,
    edit,
    remove,
    reveal,
    bumpUseCount,
    getUseCount,
  };
});
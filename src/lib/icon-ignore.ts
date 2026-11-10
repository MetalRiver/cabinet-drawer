import { ref } from "vue";

/**
 * R5（0.4.7）：图标失败「用户忽略」状态
 * - 关联键 = (id, icon_path)：路径变更（重关联/重抽）自动失效，避免不同记录互相污染
 * - 持久化于 localStorage（UI 状态，非数据库；不改 schema）
 * - 重抽成功（icon_path 变化）由调用方 clearIconIgnore 清理
 */
const KEY = "drawer-icon-ignore-v1";
const ignoreMap = ref<Record<number, string>>(load());

function load(): Record<number, string> {
  try {
    return JSON.parse(localStorage.getItem(KEY) || "{}") as Record<number, string>;
  } catch {
    return {};
  }
}

export function isIconIgnored(id: number, iconPath: string | null | undefined): boolean {
  return ignoreMap.value[id] !== undefined && ignoreMap.value[id] === iconPath;
}

export function ignoreIcon(id: number, iconPath: string | null | undefined): void {
  if (!iconPath) return;
  ignoreMap.value = { ...ignoreMap.value, [id]: iconPath };
  try {
    localStorage.setItem(KEY, JSON.stringify(ignoreMap.value));
  } catch {
    /* 持久化失败不阻断（会话内仍生效） */
  }
}

export function clearIconIgnore(id: number): void {
  if (ignoreMap.value[id] === undefined) return;
  const next = { ...ignoreMap.value };
  delete next[id];
  ignoreMap.value = next;
  try {
    localStorage.setItem(KEY, JSON.stringify(ignoreMap.value));
  } catch {
    /* 同上 */
  }
}
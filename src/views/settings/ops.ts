// ===== Settings UI V2：后台长操作状态总线 =====
// 目的（用户约束）：迁移 / 备份恢复 / 更新等长期操作的执行状态不能因切换分类丢失；
// 已开始的操作必须明确显示执行中、成功或失败——切到其他 Tab 时容器顶部显示横幅。
// Tab 组件在长操作 try/finally 中 begin/end；容器注入并渲染。
import { inject, provide, reactive } from "vue";
import type { InjectionKey } from "vue";

export interface SettingsOpsApi {
  /** 声明一个进行中的长操作（key 唯一；originTab 用于判断是否"后台"） */
  begin(key: string, label: string, originTab: string): void;
  end(key: string): void;
  /** key -> { label, originTab } */
  readonly ops: Record<string, { label: string; originTab: string }>;
}

const KEY: InjectionKey<SettingsOpsApi> = Symbol("settings-ops");

export function provideSettingsOps(): SettingsOpsApi {
  const ops = reactive<Record<string, { label: string; originTab: string }>>({});
  const api: SettingsOpsApi = {
    begin: (key, label, originTab) => {
      ops[key] = { label, originTab };
    },
    end: (key) => {
      delete ops[key];
    },
    ops,
  };
  provide(KEY, api);
  return api;
}

/** 容器外使用时的安全空实现（不应发生，防崩） */
export function useSettingsOps(): SettingsOpsApi {
  const fallback: SettingsOpsApi = {
    begin: () => {},
    end: () => {},
    ops: reactive({}),
  };
  return inject(KEY, fallback);
}

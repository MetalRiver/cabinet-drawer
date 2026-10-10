<script setup lang="ts">
// ===== Settings UI V2 · 基础 Tab：行为偏好 + 快捷键 =====
// 回收站已移至「数据与备份」；本页只保留日常行为偏好与纯展示快捷键卡。
import { ref, watch } from "vue";
import { useWidgetStore } from "../../../stores/widget";
import DarkSelect from "../../../components/DarkSelect.vue";
import SettingsCard from "../../../components/settings/SettingsCard.vue";
import SettingsRow from "../../../components/settings/SettingsRow.vue";
import type { DarkSelectOption } from "../../../components/DarkSelect.vue";

const widgetStore = useWidgetStore();

// 与旧实现一致：本地 ref 双向绑定 + watch 同步 widgetStore（保存失败由 store toast 可见化）
const autoLockMinutes = ref(widgetStore.autoLockMinutes);
const tempExpireDays = ref(widgetStore.tempExpireDays);
const clipboardSeconds = ref(widgetStore.clipboardClearSeconds);
watch(() => widgetStore.autoLockMinutes, (v) => (autoLockMinutes.value = v));
watch(() => widgetStore.tempExpireDays, (v) => (tempExpireDays.value = v));
watch(() => widgetStore.clipboardClearSeconds, (v) => (clipboardSeconds.value = v));
watch(autoLockMinutes, async (v) => {
  await widgetStore.setAutoLockMinutes(Number(v));
});
watch(tempExpireDays, async (v) => {
  await widgetStore.setTempExpireDays(Number(v));
});
watch(clipboardSeconds, async (v) => {
  await widgetStore.setClipboardClearSeconds(Number(v));
});

const autoLockOptions: DarkSelectOption<number>[] = [
  { value: 1, label: "1 分钟" },
  { value: 5, label: "5 分钟" },
  { value: 10, label: "10 分钟" },
  { value: 15, label: "15 分钟" },
  { value: 30, label: "30 分钟" },
  { value: 0, label: "永不自动锁定" },
];
const tempExpireOptions: DarkSelectOption<number>[] = [
  { value: 1, label: "1 天" },
  { value: 3, label: "3 天" },
  { value: 7, label: "7 天" },
  { value: 0, label: "永不过期" },
];
const clipboardOptions: DarkSelectOption<number>[] = [
  { value: 10, label: "10 秒" },
  { value: 15, label: "15 秒" },
  { value: 30, label: "30 秒" },
  { value: 60, label: "1 分钟" },
];
</script>

<template>
  <div class="tab-pane">
    <SettingsCard title="行为偏好" subtitle="改动立即保存并生效，保存失败会弹出提示。">
      <div class="s-rows">
        <SettingsRow icon="🧊" label="闲置自动锁定" sub="无操作超过该时长自动锁定（0 = 永不）">
          <DarkSelect v-model="autoLockMinutes" :options="autoLockOptions" aria-label="闲置自动锁定" />
        </SettingsRow>
        <SettingsRow icon="📅" label="便签默认有效期" sub="新建便签的默认保留时长（0 = 永不过期）">
          <DarkSelect v-model="tempExpireDays" :options="tempExpireOptions" aria-label="便签默认过期" />
        </SettingsRow>
        <SettingsRow icon="📋" label="剪贴板保护" sub="复制密码/片段/便签后自动清空剪贴板">
          <DarkSelect v-model="clipboardSeconds" :options="clipboardOptions" aria-label="剪贴板自动清除" />
        </SettingsRow>
      </div>
    </SettingsCard>

    <SettingsCard title="快捷键" :badge="{ text: '速查', tone: 'gray' }">
      <div class="s-shortcut-grid">
        <div class="s-shortcut-item">
          <span class="s-shortcut-label">呼出 / 收起窗口</span>
          <span class="s-shortcut-key">Alt + Q</span>
        </div>
        <div class="s-shortcut-item">
          <span class="s-shortcut-label">立即锁定</span>
          <span class="s-shortcut-key">Alt + L</span>
        </div>
        <div class="s-shortcut-item">
          <span class="s-shortcut-label">完整 / 迷你切换</span>
          <span class="s-shortcut-key">Alt + M</span>
        </div>
      </div>
    </SettingsCard>
  </div>
</template>

<style scoped>
.s-rows {
  border: 1px solid var(--border);
  border-radius: 10px;
  background: var(--bg-secondary);
  overflow: hidden;
}
</style>

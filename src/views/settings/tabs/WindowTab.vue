<script setup lang="ts">
// ===== Settings UI V2 · 窗口与外观 Tab =====
// 真实性约束（用户批准方案）：
// - 九宫格全部为纯位置动作，走真实 set_window_position（工作区 + DPI 换算），绝不改窗口尺寸；
// - 「保存当前位置 / 回到记忆位置」使用独立记忆坐标（widget.mem_x/mem_y），
//   与自动保存的窗口几何（widget.x/y/width/height）完全分离；
// - 所有动作失败可见（StatusLine），不存在的动画不写在说明里。
import { ref, onMounted } from "vue";
import { useWidgetStore } from "../../../stores/widget";
import { getWindowPosition } from "../../../api";
import SettingsCard from "../../../components/settings/SettingsCard.vue";
import StatusLine from "../../../components/settings/StatusLine.vue";
import PositionPad from "../../../components/settings/PositionPad.vue";
import type { PadPreset } from "../../../components/settings/PositionPad.vue";

const widgetStore = useWidgetStore();

const posBusy = ref(false);
const posStatus = ref<{ tone: "ok" | "err" | "info"; text: string } | null>(null);
const lastPreset = ref<string | null>(null);
/** 当前窗口位置（物理像素，与保存的几何一致） */
const curX = ref<number | null>(null);
const curY = ref<number | null>(null);

async function refreshPosition() {
  try {
    const [x, y] = await getWindowPosition();
    curX.value = x;
    curY.value = y;
  } catch {
    curX.value = null;
    curY.value = null;
  }
}
onMounted(refreshPosition);

async function onPadSelect(preset: PadPreset) {
  if (posBusy.value) return;
  posBusy.value = true;
  posStatus.value = null;
  try {
    const { x, y } = await widgetStore.applyPresetPosition(preset);
    lastPreset.value = preset;
    curX.value = x;
    curY.value = y;
    posStatus.value = { tone: "ok", text: `已移动到预设位置（${x}, ${y}）· 窗口尺寸保持不变` };
  } catch (e) {
    posStatus.value = { tone: "err", text: `移动失败：${e}` };
  } finally {
    posBusy.value = false;
  }
}

async function onSaveMemory() {
  if (posBusy.value) return;
  posBusy.value = true;
  posStatus.value = null;
  try {
    await widgetStore.saveMemoryPosition();
    posStatus.value = { tone: "ok", text: `已记住当前位置（${widgetStore.memoryX}, ${widgetStore.memoryY}）` };
  } catch (e) {
    posStatus.value = { tone: "err", text: `保存记忆位置失败：${e}` };
  } finally {
    posBusy.value = false;
  }
}

async function onRestoreMemory() {
  if (posBusy.value) return;
  posBusy.value = true;
  posStatus.value = null;
  try {
    await widgetStore.restoreMemoryPosition();
    lastPreset.value = null;
    await refreshPosition();
    posStatus.value = { tone: "ok", text: "已回到记忆位置（窗口尺寸不变）" };
  } catch (e) {
    posStatus.value = { tone: "err", text: String(e) };
  } finally {
    posBusy.value = false;
  }
}
</script>

<template>
  <div class="tab-pane">
    <SettingsCard
      title="显示模式"
      subtitle="完整卡片与极简图标列两种形态；切换与置顶立即生效。"
    >
      <div class="s-seg-group">
        <div class="s-seg-label"><span>形态</span></div>
        <div class="s-seg-inner">
          <button
            class="s-seg-btn tap"
            :class="{ active: !widgetStore.miniMode }"
            data-interactive
            @click="!widgetStore.miniMode && widgetStore.toggleMiniMode()"
          >
            <span class="s-seg-label-inner">🗂️ 完整卡片</span>
          </button>
          <button
            class="s-seg-btn tap"
            :class="{ active: widgetStore.miniMode }"
            data-interactive
            @click="!widgetStore.miniMode && widgetStore.toggleMiniMode()"
          >
            <span class="s-seg-label-inner">🎛️ 极简图标列</span>
          </button>
        </div>
      </div>
      <div class="s-seg-group">
        <div class="s-seg-label"><span>窗口层级</span></div>
        <div class="s-seg-inner">
          <button
            class="s-seg-btn tap"
            :class="{ active: !widgetStore.alwaysOnTop }"
            data-interactive
            @click="widgetStore.alwaysOnTop && widgetStore.toggleAlwaysOnTop()"
          >
            <span class="s-seg-label-inner">⬇️ 不置顶</span>
          </button>
          <button
            class="s-seg-btn tap"
            :class="{ active: widgetStore.alwaysOnTop }"
            data-interactive
            @click="!widgetStore.alwaysOnTop && widgetStore.toggleAlwaysOnTop()"
          >
            <span class="s-seg-label-inner">⬆️ 置顶显示</span>
          </button>
        </div>
      </div>
      <p class="s-hint">
        极简图标列仅在本次运行内生效：重启应用或 Alt+Q 呼出时，自动恢复完整卡片模式（有意设计）。
        保存失败会弹出提示。
      </p>
    </SettingsCard>

    <SettingsCard
      title="窗口位置"
      subtitle="九宫格与记忆位置都只移动窗口，绝不改变你调整好的窗口尺寸。"
    >
      <!-- 记忆位置（独立坐标，不与自动几何混用） -->
      <div class="mem-row">
        <div class="mem-info">
          <span class="mem-title">记忆位置</span>
          <span class="mem-value">{{ widgetStore.memoryLabel }}</span>
        </div>
        <div class="mem-actions">
          <button class="s-btn-secondary" :disabled="posBusy" data-interactive @click="onSaveMemory">📍 保存当前位置</button>
          <button class="s-btn-secondary" :disabled="posBusy || widgetStore.memoryX === null" data-interactive @click="onRestoreMemory">🧭 回到记忆位置</button>
        </div>
      </div>

      <!-- 九宫格：全部为位置动作，中心 = 屏幕居中 -->
      <PositionPad :disabled="posBusy" :active-preset="lastPreset" @select="onPadSelect" />

      <p class="s-hint">
        预设位置基于显示器<b>工作区域</b>（自动避开任务栏）计算，适配 100% / 125% / 150% 缩放；
        拖动窗口到边缘 20px 内可吸附。
      </p>
      <p class="s-hint mono">
        当前窗口位置：{{ curX === null ? "读取中…" : `X ${curX}, Y ${curY}` }}（物理像素，与保存值一致）
      </p>
      <StatusLine v-if="posStatus" :tone="posStatus.tone" :text="posStatus.text" />
    </SettingsCard>
  </div>
</template>

<style scoped>
.mem-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
  flex-wrap: wrap;
  padding: 10px 12px;
  background: var(--bg-secondary);
  border: 1px solid var(--border);
  border-radius: 9px;
  margin-bottom: 12px;
}
.mem-info { display: flex; flex-direction: column; gap: 2px; min-width: 0; }
.mem-title { font-size: 13px; font-weight: 600; color: var(--text-primary); }
.mem-value { font-size: 11.5px; color: var(--text-muted); font-family: 'Consolas', monospace; }
.mem-actions { display: flex; gap: 8px; flex-wrap: wrap; }
.mono { font-family: 'Consolas', monospace; }
</style>

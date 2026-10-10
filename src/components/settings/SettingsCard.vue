<script setup lang="ts">
// ===== SettingsCard：设置卡片统一容器（Settings UI V2）=====
// 统一：标题/副题层级、徽章、意图色边框、可折叠（收起用 v-show 保留内部状态，
// 折叠不销毁表单，敏感清理由 Tab 级 active 监听负责）。
withDefaults(
  defineProps<{
    title: string;
    subtitle?: string;
    badge?: { text: string; tone?: "purple" | "green" | "blue" | "red" | "amber" | "gray" };
    intent?: "default" | "security" | "warning" | "danger";
    collapsible?: boolean;
  }>(),
  { subtitle: "", badge: undefined, intent: "default", collapsible: false }
);
const open = defineModel<boolean>("open", { default: true });
function toggle() {
  open.value = !open.value;
}
</script>

<template>
  <section
    class="s-card"
    :class="{
      's-card-security': intent === 'security',
      's-card-warning': intent === 'warning',
      's-card-danger': intent === 'danger',
    }"
  >
    <component
      :is="collapsible ? 'button' : 'div'"
      class="s-card-head"
      :type="collapsible ? 'button' : undefined"
      :data-interactive="collapsible || undefined"
      @click="collapsible ? toggle() : undefined"
    >
      <div class="s-card-head-text">
        <h3 class="s-card-title">
          {{ title }}
          <span v-if="badge" class="s-badge" :class="`s-badge-${badge.tone ?? 'gray'}`">{{ badge.text }}</span>
        </h3>
        <p v-if="subtitle" class="s-card-sub">{{ subtitle }}</p>
      </div>
      <span v-if="collapsible" class="s-card-chevron" :class="{ open }" aria-hidden="true">▾</span>
    </component>
    <div v-show="!collapsible || open" class="s-card-body">
      <slot />
    </div>
  </section>
</template>

<style scoped>
.s-card {
  background: var(--bg-primary);
  border: 1px solid var(--border);
  border-radius: 12px;
  padding: 14px 16px;
  margin-bottom: 12px;
  transition: border-color 0.2s;
}
.s-card:hover { border-color: rgba(120, 130, 255, 0.35); }
.s-card-security { border-left: 3px solid #10b981; }
.s-card-warning { border-left: 3px solid #f59e0b; }
.s-card-danger { border-color: rgba(244, 63, 94, 0.5); border-left: 3px solid #f43f5e; }

.s-card-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
  width: 100%;
  padding: 0;
  margin: 0 0 10px;
  background: transparent;
  border: none;
  text-align: left;
  cursor: default;
}
button.s-card-head { cursor: pointer; border-radius: 8px; }
button.s-card-head:hover .s-card-title { color: #fff; }
.s-card-head-text { min-width: 0; }
.s-card-title {
  font-size: 14px;
  font-weight: 600;
  margin: 0;
  color: var(--text-primary);
  display: flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
}
.s-card-sub {
  font-size: 11.5px;
  color: var(--text-muted);
  margin: 3px 0 0;
  line-height: 1.5;
}
.s-card-chevron {
  flex-shrink: 0;
  color: var(--text-muted);
  font-size: 12px;
  transition: transform 0.18s;
}
.s-card-chevron.open { transform: rotate(180deg); }
.s-card-body { min-width: 0; }

/* 徽章 */
.s-badge {
  font-size: 10.5px;
  font-weight: 600;
  padding: 2px 8px;
  border-radius: 999px;
  white-space: nowrap;
  border: 1px solid transparent;
  letter-spacing: 0.2px;
}
.s-badge-gray { background: var(--bg-tertiary); color: var(--text-muted); border-color: var(--border); }
.s-badge-blue { background: rgba(59, 130, 246, 0.12); color: #60a5fa; border-color: rgba(59, 130, 246, 0.35); }
.s-badge-purple { background: rgba(168, 85, 247, 0.12); color: #c084fc; border-color: rgba(168, 85, 247, 0.35); }
.s-badge-green { background: rgba(16, 185, 129, 0.12); color: #34d399; border-color: rgba(16, 185, 129, 0.4); }
.s-badge-red { background: rgba(244, 63, 94, 0.12); color: #fb7185; border-color: rgba(244, 63, 94, 0.45); }
.s-badge-amber { background: rgba(245, 158, 11, 0.12); color: #fbbf24; border-color: rgba(245, 158, 11, 0.45); }
</style>

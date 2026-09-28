<script setup lang="ts">
type AttributionMode = "default" | "custom" | "disabled";

defineProps<{
  mode: AttributionMode;
  commitAttribution: string;
  prAttribution: string;
}>();

defineEmits<{
  updateMode: [mode: AttributionMode];
  updateCommitAttribution: [value: string];
  updatePrAttribution: [value: string];
}>();

import { computed } from "vue";
import { t } from "../../i18n";
import ElegantSelect from "../common/ElegantSelect.vue";

// 选项文案在 computed 内求值：写成模块级常量会把语言冻结在 import 时。
const modeOptions = computed(() => [
  { value: "default", label: t("api-profile.attribution.useClaudeCodeDefault") },
  { value: "custom", label: t("api-profile.attribution.custom") },
  { value: "disabled", label: t("api-profile.attribution.disabled") }
]);
</script>

<template>
  <section class="settings-section">
    <h3 class="section-title">{{ t("api-profile.attribution.title") }}</h3>
    <p class="section-hint">{{ t("api-profile.attribution.hint") }}</p>
    <div class="field-group">
      <label class="field-label">{{ t("api-profile.attribution.mode") }}</label>
      <div class="elegant-select-wrapper">
        <ElegantSelect
          :model-value="mode"
          :options="modeOptions"
          @update:model-value="$emit('updateMode', $event as AttributionMode)"
        />
      </div>
      <span v-if="mode === 'default'" class="field-hint">
        {{ t("api-profile.attribution.defaultHint") }}
      </span>
      <span v-else-if="mode === 'disabled'" class="field-hint">
        {{ t("api-profile.attribution.disabledHint") }}
      </span>
    </div>
    <div v-if="mode === 'custom'" class="field-row top-gap">
      <div class="field-group">
        <label class="field-label">{{ t("api-profile.attribution.commit") }}</label>
        <input
          class="field-input"
          type="text"
          :value="commitAttribution"
          autocomplete="off"
          autocapitalize="off"
          autocorrect="off"
          spellcheck="false"
          placeholder="Co-Authored-By: Claude <noreply@anthropic.com>"
          @input="$emit('updateCommitAttribution', ($event.target as HTMLInputElement).value)"
        />
        <span class="field-hint">{{ t("api-profile.attribution.commitHint") }}</span>
      </div>
      <div class="field-group">
        <label class="field-label">{{ t("api-profile.attribution.pr") }}</label>
        <input
          class="field-input"
          type="text"
          :value="prAttribution"
          autocomplete="off"
          autocapitalize="off"
          autocorrect="off"
          spellcheck="false"
          placeholder="Generated with Claude Code"
          @input="$emit('updatePrAttribution', ($event.target as HTMLInputElement).value)"
        />
        <span class="field-hint">{{ t("api-profile.attribution.prHint") }}</span>
      </div>
    </div>
  </section>
</template>

<style scoped>
.settings-section {
  margin-bottom: var(--space-4);
  padding: var(--space-4);
  background: var(--color-bg-secondary);
  border: 1px solid var(--color-border);
  border-radius: var(--radius-lg);
  box-shadow: 0 1px 2px rgba(0, 0, 0, 0.02);
}
.section-title {
  font-size: var(--text-sm);
  font-weight: 600;
  margin: 0 0 var(--space-1);
  color: var(--color-text);
}
.section-hint {
  font-size: var(--text-xs);
  color: var(--color-text-muted);
  margin: 0 0 var(--space-3);
}
.field-row {
  display: flex;
  flex-wrap: wrap;
  gap: var(--space-3);
}
.field-group {
  margin-bottom: var(--space-3);
  flex: 1;
  min-width: 0;
}
.field-label {
  display: block;
  font-size: var(--text-xs);
  font-weight: 500;
  color: var(--color-text-secondary);
  margin-bottom: var(--space-1);
}
.field-input {
  width: 100%;
  box-sizing: border-box;
  padding: 7px var(--space-3);
  border: 1px solid var(--color-border);
  border-radius: var(--radius-md);
  font-size: var(--text-sm);
  font-family: var(--font-mono);
  background: var(--color-bg);
  color: var(--color-text);
  outline: none;
  transition: border-color var(--transition-fast);
}
.field-input:focus {
  border-color: var(--color-primary);
  box-shadow: 0 0 0 3px var(--color-primary-ring);
}
.field-select {
}
.field-hint {
  display: block;
  font-size: 11px;
  color: var(--color-text-muted);
  margin-top: 3px;
}
.top-gap {
  margin-top: var(--space-3);
}
</style>

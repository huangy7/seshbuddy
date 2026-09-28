<script setup lang="ts">
defineProps<{
  maxOutputTokens: string;
  disableExperimentalBetas: string;
  disableNonessentialTraffic: string;
}>();

defineEmits<{
  updateMaxOutputTokens: [value: string];
  updateDisableExperimentalBetas: [value: string];
  updateDisableNonessentialTraffic: [value: string];
}>();

import { computed } from "vue";
import { t } from "../../i18n";
import ElegantSelect from "../common/ElegantSelect.vue";

// 选项文案在 computed 内求值：写成模块级常量会把语言冻结在 import 时。
const booleanOptions = computed(() => [
  { value: "", label: t("api-profile.runtime.useClaudeDefault") },
  { value: "1", label: "1" },
  { value: "0", label: "0" }
]);
</script>

<template>
  <section class="settings-section">
    <h3 class="section-title">{{ t("api-profile.runtime.title") }}</h3>
    <p class="section-hint">{{ t("api-profile.runtime.hint") }}</p>
    <div class="field-row">
      <div class="field-group">
        <label class="field-label">{{ t("api-profile.runtime.maxOutputTokens") }}</label>
        <input
          class="field-input"
          type="text"
          :value="maxOutputTokens"
          placeholder="64000"
          @input="$emit('updateMaxOutputTokens', ($event.target as HTMLInputElement).value)"
        />
        <span class="field-hint">{{ t("api-profile.runtime.maxOutputTokensHint") }}</span>
      </div>
      <div class="field-group">
        <label class="field-label">{{ t("api-profile.runtime.disableExperimentalBetas") }}</label>
        <div class="elegant-select-wrapper">
          <ElegantSelect
            :model-value="disableExperimentalBetas"
            :options="booleanOptions"
            @update:model-value="$emit('updateDisableExperimentalBetas', $event as string)"
          />
        </div>
        <span class="field-hint">{{ t("api-profile.runtime.disableExperimentalBetasHint") }}</span>
      </div>
      <div class="field-group">
        <label class="field-label">{{ t("api-profile.runtime.disableNonessentialTraffic") }}</label>
        <div class="elegant-select-wrapper">
          <ElegantSelect
            :model-value="disableNonessentialTraffic"
            :options="booleanOptions"
            @update:model-value="$emit('updateDisableNonessentialTraffic', $event as string)"
          />
        </div>
        <span class="field-hint">{{ t("api-profile.runtime.disableNonessentialTrafficHint") }}</span>
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
  margin: 0 0 2px;
  color: var(--color-text);
}

.section-hint {
  font-size: 11px;
  color: var(--color-text-muted);
  margin: 0 0 var(--space-3);
  line-height: 1.4;
}

.field-row {
  display: grid;
  grid-template-columns: repeat(3, 1fr);
  gap: var(--space-3);
  align-items: flex-start;
}

@media (max-width: 680px) {
  .field-row {
    grid-template-columns: 1fr;
  }
}

.field-group {
  margin-bottom: 0;
  display: flex;
  flex-direction: column;
  min-width: 0;
}

.field-label {
  display: flex;
  align-items: center;
  height: 18px;
  font-size: var(--text-xs);
  font-weight: 500;
  color: var(--color-text-secondary);
  margin-bottom: 6px;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.field-input {
  width: 100%;
  height: 36px;
  box-sizing: border-box;
  padding: 0 var(--space-3);
  border: 1px solid var(--color-border);
  border-radius: var(--radius-md);
  font-size: var(--text-sm);
  font-family: var(--font-mono);
  background: var(--color-bg);
  color: var(--color-text);
  outline: none;
  line-height: 34px;
  transition: border-color var(--transition-fast), box-shadow var(--transition-fast);
}

.field-input:focus {
  border-color: var(--color-primary);
  box-shadow: 0 0 0 2px var(--color-primary-ring);
}

.elegant-select-wrapper {
  height: 36px;
  display: flex;
  align-items: center;
}

.field-hint {
  display: block;
  font-size: 11px;
  color: var(--color-text-muted);
  margin-top: 6px;
  line-height: 1.4;
}
</style>

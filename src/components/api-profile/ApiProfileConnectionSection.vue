<script setup lang="ts">
import { computed } from "vue";
import { t } from "../../i18n";
import SvgIcon from "../icons/SvgIcon.vue";
import ElegantSelect from "../common/ElegantSelect.vue";
import { formatModelTooltip } from "../../utils/modelPricing";

// 选项文案在 computed 内求值：写成模块级常量会把语言冻结在 import 时。
const reasoningOptions = computed(() => [
  { value: "", label: t("api-profile.connection.useCodexDefault") },
  { value: "low", label: "low" },
  { value: "medium", label: "medium" },
  { value: "high", label: "high" }
]);

interface ModelGroup {
  label: string;
  items: string[];
}

const props = defineProps<{
  profileName: string;
  profileNameError?: string;
  isOfficial?: boolean;
  isClaude: boolean;
  showApiKey: boolean;
  apiKey: string;
  apiKeyPlaceholder: string;

  baseUrl: string;
  baseUrlPlaceholder: string;
  connectionSectionHint: string;
  model: string;
  modelPlaceholder: string;
  activeCombobox: string | null;
  groupedModels: ModelGroup[];
  filteredModels: string[];
  priceById?: Map<string, string>;
  claudeEffortLevels: readonly string[];
  claudeEffortLevel: string;
  timeoutMs: string;
  reasoningEffort: string;

  modelsLoading?: boolean;
  modelsError?: string;
}>();

const emit = defineEmits<{
  updateProfileName: [value: string];
  updateApiKey: [value: string];
  toggleApiKey: [];

  updateBaseUrl: [value: string];
  updateModel: [value: string];
  openCombobox: [field: string];
  comboboxBlur: [];
  selectModel: [value: string];
  updateClaudeEffortLevel: [value: string];
  selectClaudeEffortLevel: [value: string];
  updateTimeoutMs: [value: string];
  updateReasoningEffort: [value: string];
  refreshModels: [];
}>();

const profileAvatar = computed(() => {
  const name = props.profileName || "";
  const lowerName = name.toLowerCase();
  const lowerUrl = (props.baseUrl || "").toLowerCase();
  let provider = "default";
  if (lowerName.includes("deepseek") || lowerUrl.includes("deepseek")) {
    provider = "deepseek";
  } else if (lowerName.includes("openai") || lowerName.includes("codex") || lowerUrl.includes("openai")) {
    provider = "openai";
  } else if (lowerName.includes("glm") || lowerName.includes("zhipu") || lowerUrl.includes("bigmodel")) {
    provider = "glm";
  } else if (lowerName.includes("gemini") || lowerUrl.includes("google")) {
    provider = "gemini";
  } else if (lowerName.includes("kimi") || lowerUrl.includes("moonshot")) {
    provider = "kimi";
  } else if (lowerName.includes("claude") || lowerUrl.includes("anthropic")) {
    provider = "claude";
  }
  const initials = name ? name.slice(0, 2).toUpperCase() : t("api-profile.connection.newProfileInitial");
  return { provider, initials };
});

const effortPresets = computed(() => [
  { label: t("api-profile.connection.defaultOption"), value: "" },
  { label: "low", value: "low" },
  { label: "medium", value: "medium" },
  { label: "high", value: "high" },
  { label: "max", value: "max" },
]);

const timeoutFormatted = computed(() => {
  const ms = parseInt(props.timeoutMs, 10);
  if (isNaN(ms) || ms <= 0) return "";
  if (ms < 1000) return t("api-profile.connection.timeoutFormat.ms", { value: ms });
  const sec = Math.round(ms / 1000);
  if (sec < 60) return t("api-profile.connection.timeoutFormat.seconds", { value: sec });
  const min = (ms / (1000 * 60)).toFixed(1).replace(/\.0$/, "");
  if (parseFloat(min) < 60) return t("api-profile.connection.timeoutFormat.minutes", { value: min });
  const hours = (ms / (1000 * 60 * 60)).toFixed(1).replace(/\.0$/, "");
  return t("api-profile.connection.timeoutFormat.hours", { value: hours });
});

const timeoutPresets = [
  { label: "60s", value: "60000" },
  { label: "5m", value: "300000" },
  { label: "30m", value: "1800000" },
  { label: "50m", value: "3000000" },
];

const displayGroupedModels = computed(() => {
  if (!props.model) return props.groupedModels;
  
  const isExactMatch = props.groupedModels.some(g => g.items.includes(props.model));
  if (isExactMatch) return props.groupedModels;

  const q = props.model.toLowerCase();
  return props.groupedModels.map(g => ({
    label: g.label,
    items: g.items.filter(m => m.toLowerCase().includes(q))
  })).filter(g => g.items.length > 0);
});

const displayClaudeEffortLevels = computed(() => {
  if (!props.claudeEffortLevel) return props.claudeEffortLevels;
  
  const isExactMatch = props.claudeEffortLevels.includes(props.claudeEffortLevel);
  if (isExactMatch) return props.claudeEffortLevels;

  const q = props.claudeEffortLevel.toLowerCase();
  return props.claudeEffortLevels.filter(lvl => lvl.toLowerCase().includes(q));
});
</script>

<template>
  <!-- 1. Basic Info Section (Profile Name) -->
  <section class="settings-section">
    <div class="field-group">
      <label class="field-label">
        {{ t("api-profile.connection.name") }}
        <span class="required-star">*</span>
      </label>
      <div class="profile-name-input-wrapper">
        <div class="card-avatar" :class="`provider-${profileAvatar.provider}`">
          {{ profileAvatar.initials }}
        </div>
        <input
          class="field-input name-input"
          :class="{ 'has-error': !!profileNameError }"
          type="text"
          :value="profileName"
          :disabled="isOfficial"
          autocomplete="off"
          autocapitalize="off"
          autocorrect="off"
          spellcheck="false"
          :placeholder="t('api-profile.connection.namePlaceholder')"
          @input="emit('updateProfileName', ($event.target as HTMLInputElement).value)"
        />
      </div>
      <span v-if="profileNameError" class="field-error-hint">{{ profileNameError }}</span>
      <span v-else class="field-hint">{{ t("api-profile.connection.nameHint") }}</span>
    </div>
  </section>

  <!-- 2. Connection Settings Section -->
  <section class="settings-section">
    <h3 class="section-title">{{ t("api-profile.connection.title") }}</h3>

    <!-- API Key -->
    <div class="field-group">
      <label class="field-label">{{ t("api-profile.connection.apiKey") }}</label>
      <div class="field-input-wrapper">
        <input
          class="field-input field-input-with-btn"
          :type="showApiKey ? 'text' : 'password'"
          :value="apiKey"
          :placeholder="apiKeyPlaceholder"
          @input="emit('updateApiKey', ($event.target as HTMLInputElement).value)"
        />
        <button
          class="field-eye-btn"
          type="button"
          :title="showApiKey ? t('api-profile.connection.hide') : t('api-profile.connection.show')"
          @click="emit('toggleApiKey')"
        >
          <SvgIcon :name="showApiKey ? 'eye-off' : 'eye'" :size="14" />
        </button>
      </div>
    </div>

    <!-- Base URL -->
    <div class="field-group">
      <label class="field-label">{{ t("api-profile.connection.baseUrl") }}</label>
      <input
        class="field-input"
        type="text"
        :value="baseUrl"
        autocomplete="off"
        autocapitalize="off"
        autocorrect="off"
        spellcheck="false"
        :placeholder="baseUrlPlaceholder"
        @input="emit('updateBaseUrl', ($event.target as HTMLInputElement).value)"
      />
      <span class="field-hint">{{ connectionSectionHint }}</span>
    </div>

    <!-- Claude: 2 Columns (Effort Level & API Timeout) -->
    <div v-if="isClaude" class="field-row-2col">
      <!-- Col 1: Effort Level -->
      <div class="field-group">
        <label class="field-label">{{ t("api-profile.connection.effortLevel") }}</label>
        <div class="model-combobox">
          <input
            class="field-input"
            type="text"
            :value="claudeEffortLevel"
            :title="claudeEffortLevel"
            autocomplete="off"
            autocapitalize="off"
            autocorrect="off"
            spellcheck="false"
            :placeholder="t('api-profile.connection.effortPlaceholder')"
            @input="emit('updateClaudeEffortLevel', ($event.target as HTMLInputElement).value)"
            @focus="emit('openCombobox', 'effort')"
            @click="emit('openCombobox', 'effort')"
            @blur="emit('comboboxBlur')"
          />
          <div v-if="activeCombobox === 'effort'" class="model-combobox-dropdown">
            <div class="model-combobox-group-label">{{ t("api-profile.connection.commonOptions") }}</div>
            <div
              v-for="level in displayClaudeEffortLevels"
              :key="level"
              class="model-combobox-item"
              :class="{ 'is-current': level === claudeEffortLevel }"
              :title="level"
              @mousedown.prevent="emit('selectClaudeEffortLevel', level)"
            >{{ level }}</div>
            <template v-if="claudeEffortLevel && !claudeEffortLevels.includes(claudeEffortLevel)">
              <div class="model-combobox-group-label">{{ t("api-profile.connection.currentCustom") }}</div>
              <div class="model-combobox-item is-current">{{ claudeEffortLevel }}</div>
            </template>
          </div>
        </div>
        <div class="preset-pills-row">
          <button
            v-for="p in effortPresets"
            :key="p.value"
            type="button"
            class="pill-tag-btn"
            :class="{ active: (claudeEffortLevel || '').toLowerCase() === p.value }"
            @click="emit('updateClaudeEffortLevel', p.value)"
          >
            {{ p.label }}
          </button>
        </div>
      </div>

      <!-- Col 2: API Timeout (ms) -->
      <div class="field-group">
        <div class="model-label-row">
          <label class="field-label">{{ t("api-profile.connection.timeout") }}</label>
          <span v-if="timeoutFormatted" class="timeout-friendly-badge">{{ timeoutFormatted }}</span>
        </div>
        <div class="timeout-input-wrap">
          <input
            class="field-input"
            type="text"
            :value="timeoutMs"
            autocomplete="off"
            autocapitalize="off"
            autocorrect="off"
            spellcheck="false"
            placeholder="600000"
            @input="emit('updateTimeoutMs', ($event.target as HTMLInputElement).value)"
          />
        </div>
        <div class="preset-pills-row">
          <button
            v-for="p in timeoutPresets"
            :key="p.value"
            type="button"
            class="pill-tag-btn"
            :class="{ active: timeoutMs === p.value }"
            @click="emit('updateTimeoutMs', p.value)"
          >
            {{ p.label }}
          </button>
        </div>
      </div>
    </div>

    <!-- Codex: Main Model & Reasoning Effort -->
    <template v-else>
      <div class="field-group">
        <div class="model-label-row">
          <label class="field-label">{{ t("api-profile.field.mainModel") }}</label>
          <button
            v-if="baseUrl"
            type="button"
            class="model-refresh-btn"
            :disabled="modelsLoading"
            @click="emit('refreshModels')"
          >
            <span v-if="modelsLoading" class="model-refresh-spinner" />
            <SvgIcon v-else name="download" :size="12" />
            <span>{{ t("session.projectPanel.refresh") }}</span>
          </button>
        </div>
        <div class="model-combobox">
          <input
            class="field-input"
            type="text"
            :value="model"
            :title="model"
            autocomplete="off"
            autocapitalize="off"
            autocorrect="off"
            spellcheck="false"
            :placeholder="modelPlaceholder"
            @input="emit('updateModel', ($event.target as HTMLInputElement).value)"
            @focus="emit('openCombobox', 'mainModel')"
            @click="emit('openCombobox', 'mainModel')"
            @blur="emit('comboboxBlur')"
          />
          <div v-if="activeCombobox === 'mainModel'" class="model-combobox-dropdown">
            <template v-for="group in displayGroupedModels" :key="group.label">
              <div class="model-combobox-group-label">{{ group.label }}</div>
              <div
                v-for="m in group.items"
                :key="m"
                class="model-combobox-item"
                :class="{ 'is-current': m === model }"
                :title="formatModelTooltip(m, priceById)"
                @mousedown.prevent="emit('selectModel', m)"
              >{{ m }}</div>
            </template>
            <template v-if="model && !filteredModels.includes(model)">
              <div class="model-combobox-group-label">{{ t("api-profile.connection.current") }}</div>
              <div class="model-combobox-item is-current" :title="model">{{ model }}</div>
            </template>
          </div>
        </div>
        <span v-if="modelsError" class="model-error">{{ modelsError }}</span>
      </div>

      <div class="field-group">
        <label class="field-label">{{ t("api-profile.connection.reasoningEffort") }}</label>
        <div class="elegant-select-wrapper">
          <ElegantSelect
            :model-value="reasoningEffort"
            :options="reasoningOptions"
            @update:model-value="emit('updateReasoningEffort', $event as string)"
          />
        </div>
        <span class="field-hint">{{ t("api-profile.connection.reasoningHint") }}</span>
      </div>
    </template>
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
  margin: 0 0 var(--space-3);
  color: var(--color-text);
  display: flex;
  align-items: center;
  gap: 6px;
}

.field-group {
  margin-bottom: var(--space-3);
  flex: 1;
  min-width: 0;
}

.field-group:last-child {
  margin-bottom: 0;
}

.field-label {
  display: flex;
  align-items: center;
  gap: 4px;
  font-size: var(--text-xs);
  font-weight: 500;
  color: var(--color-text-secondary);
  margin-bottom: var(--space-1);
}

.required-star {
  color: var(--color-danger);
  font-weight: 700;
}

.profile-name-input-wrapper {
  display: flex;
  align-items: center;
  gap: 8px;
}

.card-avatar {
  width: 32px;
  height: 32px;
  border-radius: 6px;
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: 11px;
  font-weight: 700;
  flex-shrink: 0;
  letter-spacing: -0.5px;
  transition: all 0.2s ease;
}

.card-avatar.provider-claude {
  background: rgba(217, 119, 87, 0.15);
  color: #d97757;
  border: 1px solid rgba(217, 119, 87, 0.3);
}

.card-avatar.provider-deepseek {
  background: rgba(59, 130, 246, 0.15);
  color: #3b82f6;
  border: 1px solid rgba(59, 130, 246, 0.3);
}

.card-avatar.provider-openai {
  background: rgba(16, 185, 129, 0.15);
  color: #10b981;
  border: 1px solid rgba(16, 185, 129, 0.3);
}

.card-avatar.provider-glm {
  background: rgba(139, 92, 246, 0.15);
  color: #8b5cf6;
  border: 1px solid rgba(139, 92, 246, 0.3);
}

.card-avatar.provider-gemini {
  background: rgba(236, 72, 153, 0.15);
  color: #ec4899;
  border: 1px solid rgba(236, 72, 153, 0.3);
}

.card-avatar.provider-kimi {
  background: rgba(99, 102, 241, 0.15);
  color: #6366f1;
  border: 1px solid rgba(99, 102, 241, 0.3);
}

.card-avatar.provider-default {
  background: var(--color-bg);
  color: var(--color-text-secondary);
  border: 1px solid var(--color-border);
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

.field-input.has-error {
  border-color: var(--color-danger);
}

.field-input.has-error:focus {
  box-shadow: 0 0 0 2px rgba(239, 68, 68, 0.2);
}

.field-input::placeholder {
  color: var(--color-text-muted);
}

.field-input:disabled {
  opacity: 0.6;
  cursor: not-allowed;
}

.field-input-wrapper {
  position: relative;
  display: flex;
  align-items: center;
}

.field-input-with-btn {
  padding-right: 36px;
}

.field-eye-btn {
  position: absolute;
  right: 4px;
  display: flex;
  align-items: center;
  justify-content: center;
  width: 28px;
  height: 28px;
  border-radius: var(--radius-sm);
  color: var(--color-text-muted);
  cursor: pointer;
  transition: all var(--transition-fast);
}

.field-eye-btn:hover {
  color: var(--color-text);
  background: var(--color-bg-hover);
}

.field-hint {
  display: block;
  font-size: 11px;
  color: var(--color-text-muted);
  margin-top: 4px;
}

.field-error-hint {
  display: block;
  font-size: 11px;
  color: var(--color-danger);
  margin-top: 4px;
  font-weight: 500;
}

.expiry-ok {
  color: var(--color-text-muted);
}

.expiry-warn {
  color: #d97706;
}

.expiry-danger {
  color: var(--color-danger);
}

.field-row-2col {
  display: grid;
  grid-template-columns: repeat(2, 1fr);
  gap: var(--space-3);
}

@media (max-width: 560px) {
  .field-row-2col {
    grid-template-columns: 1fr;
  }
}

.model-label-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 4px;
  margin-bottom: var(--space-1);
}

.model-label-row .field-label {
  margin-bottom: 0;
}

.preset-pills-row {
  display: flex;
  align-items: center;
  gap: 4px;
  margin-top: 6px;
  flex-wrap: wrap;
}

.pill-tag-btn {
  padding: 2px 7px;
  font-size: 11px;
  font-weight: 500;
  border-radius: 4px;
  border: 1px solid var(--color-border);
  background: var(--color-bg);
  color: var(--color-text-muted);
  cursor: pointer;
  transition: all 0.15s ease;
  line-height: 1.2;
}

.pill-tag-btn:hover {
  color: var(--color-text);
  border-color: var(--color-border-hover, var(--color-text-secondary));
  background: var(--color-bg-hover);
}

.pill-tag-btn.active {
  color: var(--color-primary);
  border-color: rgba(37, 99, 235, 0.4);
  background: rgba(37, 99, 235, 0.08);
  font-weight: 600;
}

.timeout-friendly-badge {
  font-size: 10px;
  font-weight: 600;
  color: var(--color-primary);
  background: rgba(37, 99, 235, 0.08);
  padding: 1px 5px;
  border-radius: 3px;
}

.timeout-input-wrap {
  position: relative;
}

.model-combobox {
  position: relative;
}

.model-combobox-dropdown {
  position: absolute;
  top: calc(100% + 4px);
  left: 0;
  min-width: 100%;
  width: max-content;
  max-width: 400px;
  background: var(--color-bg);
  border: 1px solid var(--color-border);
  border-radius: var(--radius-md);
  z-index: 100;
  max-height: 260px;
  overflow-y: auto;
  box-shadow: 0 8px 24px rgba(0, 0, 0, 0.15);
}

.model-combobox-group-label {
  padding: 6px 10px 2px;
  font-size: var(--text-xs);
  font-weight: 700;
  text-transform: uppercase;
  letter-spacing: 0.06em;
  color: var(--color-text-muted);
  opacity: 0.6;
}

.model-combobox-item {
  padding: 5px 10px 5px 16px;
  font-size: var(--text-sm);
  color: var(--color-text);
  cursor: pointer;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.model-combobox-item:hover {
  background: var(--color-bg-hover, var(--color-bg));
}

.model-combobox-item.is-current {
  color: var(--color-primary);
}

.model-refresh-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 4px;
  padding: 2px 6px;
  color: var(--color-text-secondary);
  font-size: 11px;
  background: var(--color-bg);
  border: 1px solid var(--color-border);
  border-radius: var(--radius-sm);
  cursor: pointer;
  transition: all var(--transition-fast);
  flex-shrink: 0;
}

.model-refresh-btn:hover:not(:disabled) {
  color: var(--color-text);
  background: var(--color-bg-hover);
  border-color: var(--color-primary);
}

.model-refresh-btn:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.model-refresh-spinner {
  display: inline-block;
  flex-shrink: 0;
  width: 10px;
  height: 10px;
  border-radius: 50%;
  border: 1.5px solid currentColor;
  border-top-color: transparent;
  animation: model-spin 0.8s linear infinite;
}

@keyframes model-spin {
  from { transform: rotate(0deg); }
  to { transform: rotate(360deg); }
}

.model-error {
  display: block;
  margin-top: 3px;
  font-size: 11px;
  color: var(--color-danger);
}
</style>

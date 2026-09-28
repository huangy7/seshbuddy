<script setup lang="ts">
import { computed } from "vue";
import { t } from "../../i18n";
import SvgIcon from "../icons/SvgIcon.vue";
import { has1m, set1m, strip1m } from "../../utils/modelSuffix";
import { formatModelTooltip } from "../../utils/modelPricing";

interface ModelGroup {
  label: string;
  items: string[];
}

const props = defineProps<{
  cliName: string;

  modelsLoading: boolean;
  modelsError: string;
  activeCombobox: string | null;
  groupedModels: ModelGroup[];
  filteredModels: string[];
  priceById?: Map<string, string>;
  haikuModel: string;
  haikuModelName?: string;
  sonnetModel: string;
  sonnetModelName?: string;
  opusModel: string;
  opusModelName?: string;
  fallbackModel: string;
  /** 网关 display，未手填的行会跟随它 */
  autoNames?: Partial<Record<"haiku" | "sonnet" | "opus", string>>;
  /** 该行的显示名称是否已被用户手填过 */
  typedNames?: Partial<Record<"haiku" | "sonnet" | "opus", boolean>>;
}>();

const emit = defineEmits<{
  refreshModels: [];
  updateModel: [field: "haiku" | "sonnet" | "opus", value: string];
  updateModelName: [field: "haiku" | "sonnet" | "opus", value: string];
  restoreAutoName: [field: "haiku" | "sonnet" | "opus"];
  updateFallbackModel: [value: string];
  openCombobox: [field: string];
  comboboxBlur: [];
  selectModel: [field: "haiku" | "sonnet" | "opus" | "fallback", value: string];
}>();

const modelRoleRows = computed(() => [
  {
    role: "sonnet" as const,
    label: "Sonnet",
    model: props.sonnetModel || "",
    displayName: props.sonnetModelName || "",
    autoName: props.autoNames?.sonnet || "",
    typed: props.typedNames?.sonnet === true,
    supports1m: true,
  },
  {
    role: "opus" as const,
    label: "Opus",
    model: props.opusModel || "",
    displayName: props.opusModelName || "",
    autoName: props.autoNames?.opus || "",
    typed: props.typedNames?.opus === true,
    supports1m: true,
  },
  {
    role: "haiku" as const,
    label: "Haiku",
    model: props.haikuModel || "",
    displayName: props.haikuModelName || "",
    autoName: props.autoNames?.haiku || "",
    typed: props.typedNames?.haiku === true,
    supports1m: true,
  },
]);

/** 名字框空着时的灰字提示：手填过又清空 → 明确告知不会写入 */
function displayPlaceholder(row: { model: string; autoName: string; typed: boolean }): string {
  if (row.typed) return t("api-profile.modelOverrides.typedPlaceholder");
  return row.autoName || strip1m(row.model) || t("api-profile.modelOverrides.displayPlaceholder");
}

/** 该行的值确实等于网关 display、且用户没动过 → 切模型会跟着变 */
function isAutoName(row: { displayName: string; autoName: string; typed: boolean }): boolean {
  return !row.typed && row.displayName !== "" && row.displayName === row.autoName;
}

/** 手填过、且当前模型有网关名字 → 可以点「还原」改回跟随 */
function canRestoreAuto(row: { typed: boolean; autoName: string }): boolean {
  return row.typed && row.autoName !== "";
}

function onModelInput(role: "haiku" | "sonnet" | "opus", inputValue: string, currentModel: string) {
  const wants1m = has1m(inputValue) || has1m(currentModel);
  emit("updateModel", role, set1m(inputValue, wants1m));
}

function onFallbackInput(inputValue: string, currentFallback: string) {
  const wants1m = has1m(inputValue) || has1m(currentFallback);
  emit("updateFallbackModel", set1m(inputValue, wants1m));
}

function getDisplayGroupedModels(field: "haiku" | "sonnet" | "opus" | "fallback") {
  let raw = "";
  if (field === "haiku") raw = props.haikuModel;
  else if (field === "sonnet") raw = props.sonnetModel;
  else if (field === "opus") raw = props.opusModel;
  else raw = props.fallbackModel;

  const model = strip1m(raw);
  if (!model) return props.groupedModels;

  const isExactMatch = props.groupedModels.some((g) => g.items.includes(model));
  if (isExactMatch) return props.groupedModels;

  const q = model.toLowerCase();
  return props.groupedModels
    .map((g) => ({
      label: g.label,
      items: g.items.filter((m) => m.toLowerCase().includes(q)),
    }))
    .filter((g) => g.items.length > 0);
}
</script>

<template>
  <section class="settings-section">
    <!-- Header -->
    <div class="section-title-row">
      <div class="title-left">
        <h3 class="section-title">{{ t("api-profile.modelOverrides.title") }}</h3>
        <p class="section-hint">{{ t("api-profile.modelOverrides.hint") }}</p>
      </div>
      <div class="header-actions">
        <button
          type="button"
          class="action-btn"
          :disabled="modelsLoading"
          @click="emit('refreshModels')"
        >
          <span v-if="modelsLoading" class="model-refresh-spinner" />
          <SvgIcon v-else name="download" :size="13" />
          <span>{{ t("api-profile.modelOverrides.fetchModels") }}</span>
        </button>
      </div>
    </div>

    <span v-if="modelsError" class="model-error">{{ modelsError }}</span>

    <!-- Table Column Headers -->
    <div class="model-table-header">
      <span class="th-role">{{ t("api-profile.modelOverrides.role") }}</span>
      <span class="th-display">{{ t("api-profile.modelOverrides.displayName") }}</span>
      <span class="th-actual">{{ t("api-profile.modelOverrides.actualModel") }}</span>
      <span class="th-1m">{{ t("api-profile.modelOverrides.declare1m") }}</span>
    </div>

    <!-- Table Body Rows -->
    <div class="model-table-body">
      <div
        v-for="row in modelRoleRows"
        :key="row.role"
        class="model-table-row"
      >
        <!-- Col 1: Role Badge -->
        <div class="role-badge">
          {{ row.label }}
        </div>

        <!-- Col 2: Display Name Input -->
        <div class="cell-display">
          <input
            class="field-input"
            :class="{ 'is-auto': isAutoName(row), 'has-restore': canRestoreAuto(row) }"
            type="text"
            :value="row.displayName"
            :placeholder="displayPlaceholder(row)"
            autocomplete="off"
            autocapitalize="off"
            autocorrect="off"
            spellcheck="false"
            @input="emit('updateModelName', row.role, ($event.target as HTMLInputElement).value)"
          />
          <span
            v-if="isAutoName(row)"
            class="auto-tag"
            :title="t('api-profile.modelOverrides.autoTagTitle')"
          >{{ t("api-profile.modelOverrides.autoTag") }}</span>
          <button
            v-else-if="canRestoreAuto(row)"
            type="button"
            class="auto-restore"
            :title="t('api-profile.modelOverrides.restoreAutoTitle')"
            :aria-label="t('api-profile.modelOverrides.restoreAuto')"
            @click="emit('restoreAutoName', row.role)"
          >↺</button>
        </div>

        <!-- Col 3: Actual Request Model Combobox -->
        <div class="cell-actual model-combobox">
          <input
            class="field-input"
            type="text"
            :value="strip1m(row.model)"
            :title="row.model"
            :placeholder="t('api-profile.modelOverrides.actualPlaceholder')"
            autocomplete="off"
            autocapitalize="off"
            autocorrect="off"
            spellcheck="false"
            @input="onModelInput(row.role, ($event.target as HTMLInputElement).value, row.model)"
            @focus="emit('openCombobox', row.role)"
            @click="emit('openCombobox', row.role)"
            @blur="emit('comboboxBlur')"
          />
          <div v-if="activeCombobox === row.role" class="model-combobox-dropdown">
            <template v-for="group in getDisplayGroupedModels(row.role)" :key="group.label">
              <div class="model-combobox-group-label">{{ group.label }}</div>
              <div
                v-for="m in group.items"
                :key="m"
                class="model-combobox-item"
                :class="{ 'is-current': m === strip1m(row.model) }"
                :title="formatModelTooltip(m, priceById)"
                @mousedown.prevent="emit('selectModel', row.role, set1m(m, has1m(row.model)))"
              >{{ m }}</div>
            </template>
            <template v-if="strip1m(row.model) && !filteredModels.includes(strip1m(row.model))">
              <div class="model-combobox-group-label">{{ t("api-profile.connection.current") }}</div>
              <div class="model-combobox-item is-current" :title="row.model">{{ strip1m(row.model) }}</div>
            </template>
          </div>
        </div>

        <!-- Col 4: 1M Checkbox -->
        <div class="cell-1m">
          <label v-if="row.supports1m" class="checkbox-1m-label" :class="{ active: has1m(row.model) }">
            <input
              type="checkbox"
              class="checkbox-1m-native"
              :checked="has1m(row.model)"
              @change="emit('updateModel', row.role, set1m(row.model, ($event.target as HTMLInputElement).checked))"
            />
            <span class="checkbox-custom-dot" />
            <span>1M</span>
          </label>
        </div>
      </div>
    </div>

    <!-- Default Fallback Model -->
    <div class="fallback-model-section">
      <label class="field-label">{{ t("api-profile.modelOverrides.fallback") }}</label>
      <div class="fallback-input-row">
        <div class="cell-actual model-combobox">
          <input
            class="field-input"
            type="text"
            :value="strip1m(fallbackModel)"
            :title="fallbackModel"
            :placeholder="t('api-profile.modelOverrides.fallbackPlaceholder')"
            autocomplete="off"
            autocapitalize="off"
            autocorrect="off"
            spellcheck="false"
            @input="onFallbackInput(($event.target as HTMLInputElement).value, fallbackModel)"
            @focus="emit('openCombobox', 'fallback')"
            @click="emit('openCombobox', 'fallback')"
            @blur="emit('comboboxBlur')"
          />
          <div v-if="activeCombobox === 'fallback'" class="model-combobox-dropdown">
            <template v-for="group in getDisplayGroupedModels('fallback')" :key="group.label">
              <div class="model-combobox-group-label">{{ group.label }}</div>
              <div
                v-for="m in group.items"
                :key="m"
                class="model-combobox-item"
                :class="{ 'is-current': m === strip1m(fallbackModel) }"
                :title="formatModelTooltip(m, priceById)"
                @mousedown.prevent="emit('selectModel', 'fallback', set1m(m, has1m(fallbackModel)))"
              >{{ m }}</div>
            </template>
            <template v-if="strip1m(fallbackModel) && !filteredModels.includes(strip1m(fallbackModel))">
              <div class="model-combobox-group-label">{{ t("api-profile.connection.current") }}</div>
              <div class="model-combobox-item is-current" :title="fallbackModel">{{ strip1m(fallbackModel) }}</div>
            </template>
          </div>
        </div>
        <div class="cell-1m">
          <label class="checkbox-1m-label" :class="{ active: has1m(fallbackModel) }">
            <input
              type="checkbox"
              class="checkbox-1m-native"
              :checked="has1m(fallbackModel)"
              @change="emit('updateFallbackModel', set1m(fallbackModel, ($event.target as HTMLInputElement).checked))"
            />
            <span class="checkbox-custom-dot" />
            <span>1M</span>
          </label>
        </div>
      </div>
      <span class="field-hint">{{ t("api-profile.modelOverrides.fallbackHint") }}</span>
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

.section-title-row {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 12px;
  margin-bottom: var(--space-3);
}

.title-left {
  flex: 1;
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
  margin: 0;
  line-height: 1.4;
}

.header-actions {
  display: flex;
  align-items: center;
  gap: 6px;
  flex-shrink: 0;
}

.action-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 4px;
  padding: 4px 8px;
  color: var(--color-text-secondary);
  font-size: 11px;
  font-weight: 500;
  background: var(--color-bg);
  border: 1px solid var(--color-border);
  border-radius: var(--radius-md);
  cursor: pointer;
  transition: all var(--transition-fast);
  white-space: nowrap;
}

.action-btn:hover:not(:disabled) {
  color: var(--color-text);
  background: var(--color-bg-hover);
  border-color: var(--color-primary);
}

.action-btn:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.model-error {
  display: block;
  margin-bottom: var(--space-2);
  font-size: 11px;
  color: var(--color-danger);
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

/* Table Header & Rows */
.model-table-header {
  display: grid;
  grid-template-columns: 88px 1fr 1.2fr 68px;
  gap: 8px;
  padding: 0 4px 6px;
  font-size: 11px;
  font-weight: 500;
  color: var(--color-text-muted);
}

.model-table-body {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.model-table-row {
  display: grid;
  grid-template-columns: 88px 1fr 1.2fr 68px;
  gap: 8px;
  align-items: center;
}

@media (max-width: 580px) {
  .model-table-header {
    display: none;
  }
  .model-table-row {
    grid-template-columns: 1fr;
    gap: 6px;
    padding: 8px;
    background: var(--color-bg);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-md);
  }
}

.role-badge {
  height: 36px;
  display: flex;
  align-items: center;
  justify-content: center;
  background: var(--color-bg);
  border: 1px solid var(--color-border);
  border-radius: var(--radius-md);
  font-size: 12px;
  font-weight: 600;
  color: var(--color-text-secondary);
  user-select: none;
}

.cell-display,
.cell-actual {
  min-width: 0;
  width: 100%;
}

.cell-display {
  position: relative;
}

/* 右侧留出角标位，避免长名字压到「自动」标签或还原按钮下 */
.field-input.is-auto {
  padding-right: 44px;
}

.field-input.has-restore {
  padding-right: 34px;
}

/* 右侧角标：自动态是「自动」标签，手填态换成可点的「还原」按钮 */
.auto-tag,
.auto-restore {
  position: absolute;
  top: 50%;
  right: 8px;
  transform: translateY(-50%);
  border: 1px solid var(--color-border);
  border-radius: var(--radius-sm);
  background: var(--color-bg-secondary);
  color: var(--color-text-muted);
  line-height: 1.5;
}

.auto-tag {
  padding: 1px 5px;
  font-size: 11px;
}

.auto-restore {
  width: 22px;
  height: 22px;
  padding: 0;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  font-family: inherit;
  font-size: 13px;
  line-height: 1;
  cursor: pointer;
  transition: color var(--transition-fast), border-color var(--transition-fast), box-shadow var(--transition-fast);
}

.auto-restore:hover {
  color: var(--color-primary);
  border-color: var(--color-primary);
}

.auto-restore:focus-visible {
  outline: none;
  color: var(--color-primary);
  border-color: var(--color-primary);
  box-shadow: 0 0 0 2px var(--color-primary-ring);
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
  transition: border-color var(--transition-fast), box-shadow var(--transition-fast);
}

.field-input:focus {
  border-color: var(--color-primary);
  box-shadow: 0 0 0 2px var(--color-primary-ring);
}

.field-input::placeholder {
  color: var(--color-text-muted);
  font-family: inherit;
}

.cell-1m {
  display: flex;
  align-items: center;
  justify-content: center;
  height: 36px;
}

.checkbox-1m-label {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  height: 28px;
  padding: 0 8px;
  border-radius: var(--radius-sm);
  border: 1px solid var(--color-border);
  background: var(--color-bg);
  color: var(--color-text-muted);
  font-size: 11px;
  font-weight: 600;
  cursor: pointer;
  transition: all 0.15s ease;
  user-select: none;
}

.checkbox-1m-label:hover {
  border-color: var(--color-primary);
  color: var(--color-text);
}

.checkbox-1m-label.active {
  background: rgba(37, 99, 235, 0.08);
  border-color: rgba(37, 99, 235, 0.4);
  color: var(--color-primary);
}

.checkbox-1m-native {
  display: none;
}

.checkbox-custom-dot {
  width: 12px;
  height: 12px;
  border-radius: 50%;
  border: 1.5px solid var(--color-border);
  background: transparent;
  display: inline-block;
  transition: all 0.15s ease;
  position: relative;
}

.checkbox-1m-label.active .checkbox-custom-dot {
  border-color: var(--color-primary);
  background: var(--color-primary);
  box-shadow: 0 0 0 2px rgba(37, 99, 235, 0.2);
}

/* Fallback Model Section */
.fallback-model-section {
  margin-top: var(--space-4);
  padding-top: var(--space-3);
  border-top: 1px solid var(--color-border);
}

.field-label {
  display: block;
  font-size: var(--text-xs);
  font-weight: 500;
  color: var(--color-text-secondary);
  margin-bottom: var(--space-1);
}

.fallback-input-row {
  display: grid;
  grid-template-columns: 1fr 68px;
  gap: 8px;
  align-items: center;
}

.field-hint {
  display: block;
  font-size: 11px;
  color: var(--color-text-muted);
  margin-top: 4px;
}

/* Combobox */
.model-combobox {
  position: relative;
}

.model-combobox-dropdown {
  position: absolute;
  top: calc(100% + 4px);
  left: 0;
  min-width: 100%;
  width: max-content;
  max-width: 420px;
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
  padding: 6px 12px 6px 16px;
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
  font-weight: 500;
}
</style>

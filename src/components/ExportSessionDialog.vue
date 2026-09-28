<script setup lang="ts">
import { ref } from "vue";
import SvgIcon from "./icons/SvgIcon.vue";
import { t } from "../i18n";

const emit = defineEmits<{
  close: [];
  export: [format: string];
  enterSelectionMode: [];
}>();

type ExportScope = "all" | "selection";
type ExportFormat = "txt" | "markdown" | "json" | "jsonl";

const scope = ref<ExportScope>("all");
const format = ref<ExportFormat>("markdown");

const formats: { value: ExportFormat; label: string }[] = [
  { value: "txt", label: "Text" },
  { value: "markdown", label: "Markdown" },
  { value: "json", label: "JSON" },
  { value: "jsonl", label: "JSONL" },
];

function onConfirm() {
  if (scope.value === "selection") {
    emit("enterSelectionMode");
    emit("close");
  } else {
    emit("export", format.value);
    emit("close");
  }
}
</script>

<template>
  <div class="modal-backdrop" @click.self="emit('close')">
    <div class="dialog">
      <div class="dialog-header">
        <h3>{{ t("dialogs.exportSession.title") }}</h3>
        <button class="dialog-close icon-btn" :title="t('dialogs.common.close')" :aria-label="t('dialogs.common.close')" @click="emit('close')">
          <SvgIcon name="x" :size="16" />
        </button>
      </div>

      <div class="dialog-body">
        <div class="form-group">
          <label class="field-label">{{ t("dialogs.exportSession.scope") }}</label>
          <div class="scope-options">
            <label class="scope-option" :class="{ selected: scope === 'all' }">
              <input type="radio" v-model="scope" value="all" />
              <div class="scope-option-content">
                <SvgIcon name="file-text" :size="16" />
                <div class="scope-option-text">
                  <span class="scope-option-title">{{ t("dialogs.exportSession.scopeAllTitle") }}</span>
                  <span class="scope-option-desc">{{ t("dialogs.exportSession.scopeAllDesc") }}</span>
                </div>
              </div>
            </label>
            <label class="scope-option" :class="{ selected: scope === 'selection' }">
              <input type="radio" v-model="scope" value="selection" />
              <div class="scope-option-content">
                <SvgIcon name="check-square" :size="16" />
                <div class="scope-option-text">
                  <span class="scope-option-title">{{ t("dialogs.exportSession.scopeSelectionTitle") }}</span>
                  <span class="scope-option-desc">{{ t("dialogs.exportSession.scopeSelectionDesc") }}</span>
                </div>
              </div>
            </label>
          </div>
        </div>

        <div v-if="scope === 'all'" class="form-group">
          <label class="field-label">{{ t("dialogs.exportSession.format") }}</label>
          <div class="format-options">
            <label
              v-for="f in formats"
              :key="f.value"
              class="format-option"
              :class="{ selected: format === f.value }"
            >
              <input type="radio" v-model="format" :value="f.value" />
              {{ f.label }}
            </label>
          </div>
        </div>
      </div>

      <div class="dialog-footer">
        <button class="btn-cancel" @click="emit('close')">{{ t("dialogs.common.cancel") }}</button>
        <button class="btn-primary" @click="onConfirm">
          <SvgIcon :name="scope === 'selection' ? 'check-square' : 'download'" :size="14" />
          {{ scope === 'selection' ? t("dialogs.exportSession.enterSelection") : t("app.batch.export") }}
        </button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.dialog {
  width: 400px;
  background: var(--color-bg);
  border: 1px solid var(--color-border);
  border-radius: var(--radius-xl);
  box-shadow: var(--shadow-lg);
}
.dialog-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: var(--space-4);
  border-bottom: 1px solid var(--color-border);
}
.dialog-header h3 {
  font-size: var(--text-base);
  font-weight: 600;
  color: var(--color-text);
}
.dialog-close {
  border-radius: var(--radius-sm);
}
.dialog-body {
  padding: var(--space-4);
  display: flex;
  flex-direction: column;
  gap: var(--space-4);
}
.form-group {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
}
.field-label {
  display: block;
  font-size: var(--text-xs);
  font-weight: 600;
  color: var(--color-text-secondary);
}
.scope-options {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
}
.scope-option {
  display: block;
  padding: var(--space-3);
  border: 1px solid var(--color-border);
  border-radius: var(--radius-md);
  cursor: pointer;
  transition: all var(--transition-fast);
}
.scope-option:hover {
  border-color: var(--color-primary);
  background: var(--color-bg-hover);
}
.scope-option.selected {
  border-color: var(--color-primary);
  background: var(--color-primary-light);
}
.scope-option input {
  display: none;
}
.scope-option-content {
  display: flex;
  align-items: flex-start;
  gap: var(--space-3);
}
.scope-option-content > .svg-icon {
  flex-shrink: 0;
  margin-top: 2px;
  color: var(--color-text-secondary);
}
.scope-option.selected .scope-option-content > .svg-icon {
  color: var(--color-primary);
}
.scope-option-text {
  display: flex;
  flex-direction: column;
  gap: 2px;
}
.scope-option-title {
  font-size: var(--text-sm);
  font-weight: 500;
  color: var(--color-text);
}
.scope-option-desc {
  font-size: var(--text-xs);
  color: var(--color-text-muted);
}
.format-options {
  display: flex;
  gap: var(--space-2);
  margin-top: var(--space-1);
}
.format-option {
  display: flex;
  align-items: center;
  gap: var(--space-1);
  padding: var(--space-1) var(--space-3);
  font-size: var(--text-sm);
  border: 1px solid var(--color-border);
  border-radius: var(--radius-md);
  cursor: pointer;
  transition: all var(--transition-fast);
}
.format-option:hover {
  border-color: var(--color-primary);
}
.format-option.selected {
  border-color: var(--color-primary);
  background: var(--color-primary-light);
  color: var(--color-primary);
}
.format-option input {
  display: none;
}
.dialog-footer {
  display: flex;
  justify-content: flex-end;
  gap: var(--space-2);
  padding: var(--space-3) var(--space-4);
  border-top: 1px solid var(--color-border);
}
.btn-cancel {
  padding: var(--space-1) var(--space-3);
  font-size: var(--text-sm);
  color: var(--color-text-secondary);
  border: 1px solid var(--color-border);
  border-radius: var(--radius-md);
  background: transparent;
  cursor: pointer;
  transition: all var(--transition-fast);
}
.btn-cancel:hover {
  background: var(--color-bg-hover);
}
.btn-primary {
  display: flex;
  align-items: center;
  gap: var(--space-1);
  padding: var(--space-1) var(--space-3);
  font-size: var(--text-sm);
  color: white;
  background: var(--color-primary);
  border: none;
  border-radius: var(--radius-md);
  cursor: pointer;
  transition: all var(--transition-fast);
}
.btn-primary:hover {
  background: var(--color-primary-hover);
}
</style>

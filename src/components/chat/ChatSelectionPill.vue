<script setup lang="ts">
import SvgIcon from "../icons/SvgIcon.vue";
import { t } from "../../i18n";

defineProps<{
  selectedCount: number;
  totalCount: number;
  exportDropdownVisible: boolean;
}>();

defineEmits<{
  toggleExportDropdown: [];
  exportSelected: [format: "txt" | "markdown" | "json" | "jsonl"];
  exportImage: [];
  cancel: [];
  toggleSelectAll: [];
}>();
</script>

<template>
  <div class="selection-pill">
    <div class="pill-badge">
      <SvgIcon name="check" :size="12" class="pill-check-icon" />
      <!-- chat.selectionPill.selected 刻意只是一个前导词，不是整句：计数是 <strong> 包着的独立
           标记，与 .pill-total 的斜杠分母各有样式，并进整句就得拆掉这层标记。代价是语序被固定成
           「词在前、计数在后」，现行四门语言都成立；将来要加一门计数在前的语言，要改的是这段
           结构，只补译文不管用。 -->
      <span class="pill-count">
        {{ t("chat.selectionPill.selected") }} <strong>{{ selectedCount }}</strong><span class="pill-total">/{{ totalCount }}</span>
      </span>
    </div>

    <button
      class="pill-btn pill-btn-ghost"
      @click="$emit('toggleSelectAll')"
      :title="selectedCount === totalCount && totalCount > 0 ? t('chat.selectionPill.clearSelection') : t('chat.selectionPill.selectAllMessages')"
    >
      {{ selectedCount === totalCount && totalCount > 0 ? t("chat.selectionPill.deselectAll") : t("app.batch.selectAll") }}
    </button>

    <div class="pill-separator"></div>

    <div class="pill-export-wrapper" @click.stop="$emit('toggleExportDropdown')">
      <button class="pill-btn pill-btn-primary" :disabled="selectedCount === 0">
        <span>{{ t("app.batch.export") }}</span>
        <SvgIcon name="chevron-up" :size="11" />
      </button>
      <div v-if="exportDropdownVisible && selectedCount > 0" class="pill-dropdown">
        <button @click="$emit('exportSelected', 'txt')">{{ t("chat.selectionPill.plainText") }}</button>
        <button @click="$emit('exportSelected', 'markdown')">Markdown (.md)</button>
        <button @click="$emit('exportSelected', 'json')">{{ t("chat.selectionPill.jsonData") }}</button>
        <button @click="$emit('exportSelected', 'jsonl')">{{ t("chat.selectionPill.rawJsonlStream") }}</button>
        <button @click="$emit('exportImage')">{{ t("chat.selectionPill.pngLongImage") }}</button>
      </div>
    </div>

    <button class="pill-close-btn" @click="$emit('cancel')" :title="t('app.batch.exitSelection')">
      <SvgIcon name="x" :size="13" />
    </button>
  </div>
</template>

<style scoped>
.selection-pill {
  position: absolute;
  bottom: 24px;
  left: 50%;
  transform: translateX(-50%);
  background-color: var(--color-bg-secondary);
  border: 1px solid var(--color-border);
  border-radius: 9999px;
  height: 38px;
  padding: 0 6px 0 14px;
  display: inline-flex;
  align-items: center;
  gap: 8px;
  box-shadow: 0 8px 24px rgba(0, 0, 0, 0.12), 0 1px 3px rgba(0, 0, 0, 0.06);
  z-index: 100;
  backdrop-filter: blur(16px);
  -webkit-backdrop-filter: blur(16px);
  white-space: nowrap;
  user-select: none;
  -webkit-user-select: none;
  animation: pillPop 0.18s cubic-bezier(0.16, 1, 0.3, 1);
}

@keyframes pillPop {
  from {
    opacity: 0;
    transform: translate(-50%, 10px) scale(0.97);
  }
  to {
    opacity: 1;
    transform: translate(-50%, 0) scale(1);
  }
}

.pill-badge {
  display: flex;
  align-items: center;
  gap: 6px;
  flex-shrink: 0;
}

.pill-check-icon {
  color: var(--color-primary);
}

.pill-count {
  font-size: 13px;
  color: var(--color-text);
  font-variant-numeric: tabular-nums;
}

.pill-count strong {
  font-weight: 600;
  color: var(--color-text);
}

.pill-total {
  color: var(--color-text-muted);
  font-size: 12px;
  margin-left: 2px;
}

.pill-btn {
  border: none;
  border-radius: 9999px;
  height: 26px;
  padding: 0 10px;
  font-size: 12px;
  font-weight: 500;
  cursor: pointer;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 4px;
  white-space: nowrap;
  flex-shrink: 0;
  transition: all var(--transition-fast);
}

.pill-btn-ghost {
  background: var(--color-bg-hover);
  color: var(--color-text-secondary);
}

.pill-btn-ghost:hover {
  background: var(--color-surface-hover);
  color: var(--color-primary);
}

.pill-export-wrapper {
  position: relative;
  flex-shrink: 0;
}

.pill-btn-primary {
  background: var(--color-primary);
  color: #ffffff;
  padding: 0 12px;
}

.pill-btn-primary:hover:not(:disabled) {
  opacity: 0.92;
  transform: translateY(-0.5px);
}

.pill-btn-primary:disabled {
  opacity: 0.4;
  cursor: not-allowed;
}

.pill-separator {
  width: 1px;
  height: 14px;
  background-color: var(--color-border);
  flex-shrink: 0;
}

.pill-close-btn {
  background: none;
  border: none;
  color: var(--color-text-muted);
  cursor: pointer;
  border-radius: 50%;
  width: 26px;
  height: 26px;
  display: flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
  transition: all var(--transition-fast);
}

.pill-close-btn:hover {
  background-color: var(--color-bg-hover);
  color: var(--color-text);
}

.pill-dropdown {
  position: absolute;
  bottom: calc(100% + 8px);
  left: 50%;
  transform: translateX(-50%);
  background-color: var(--color-surface-elevated, var(--color-bg));
  border: 1px solid var(--color-border);
  border-radius: var(--radius-lg);
  padding: 5px;
  display: flex;
  flex-direction: column;
  box-shadow: 0 8px 24px rgba(0, 0, 0, 0.16);
  min-width: 136px;
  backdrop-filter: blur(20px);
  -webkit-backdrop-filter: blur(20px);
  white-space: nowrap;
  animation: pillPop 0.14s ease-out;
}

.pill-dropdown button {
  background: none;
  border: none;
  padding: 6px 12px;
  text-align: left;
  border-radius: var(--radius-sm);
  color: var(--color-text-secondary);
  cursor: pointer;
  font-size: 12.5px;
  white-space: nowrap;
  transition: background-color var(--transition-fast), color var(--transition-fast);
}

.pill-dropdown button:hover {
  background-color: var(--color-surface-hover, var(--color-bg-hover));
  color: var(--color-text);
}
</style>

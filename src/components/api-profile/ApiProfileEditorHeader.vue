<script setup lang="ts">
import { t } from "../../i18n";
import SvgIcon from "../icons/SvgIcon.vue";

withDefaults(
  defineProps<{
    title: string | null;
    view: "form" | "json";
    format?: "json" | "toml";
  }>(),
  {
    format: "json",
  }
);

const emit = defineEmits<{
  back: [];
  switchView: [view: "form" | "json"];
}>();
</script>

<template>
  <div class="editor-header-bar">
    <div class="header-left">
      <button class="back-btn" @click="emit('back')">
        <SvgIcon name="chevron-left" :size="15" />
        <span>{{ t("api-profile.editorHeader.back") }}</span>
      </button>

      <span class="header-sep">/</span>

      <div class="editor-title-container">
        <span class="editor-title" :title="title || ''">{{ title || t("api-profile.editorHeader.untitled") }}</span>
      </div>
    </div>

    <div class="header-right">
      <div class="api-view-toggle">
        <button
          type="button"
          class="view-btn"
          :class="{ active: view === 'form' }"
          @click="emit('switchView', 'form')"
        >
          {{ t("api-profile.editorHeader.form") }}
        </button>
        <button
          type="button"
          class="view-btn"
          :class="{ active: view === 'json' }"
          @click="emit('switchView', 'json')"
        >
          {{ format === 'toml' ? t('api-profile.editorHeader.tomlPreview') : t('api-profile.editorHeader.jsonPreview') }}
        </button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.editor-header-bar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  margin-bottom: 16px;
  padding-bottom: 12px;
  border-bottom: 1px solid var(--color-border);
}

.header-left {
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
  flex: 1;
}

.back-btn {
  display: inline-flex;
  align-items: center;
  gap: 3px;
  height: 26px;
  padding: 0 8px;
  font-size: 12px;
  font-weight: 500;
  color: var(--color-text-secondary);
  cursor: pointer;
  border-radius: 4px;
  border: 1px solid var(--color-border);
  background: var(--color-bg);
  transition: all var(--transition-fast);
  white-space: nowrap;
  flex-shrink: 0;
}

.back-btn:hover {
  background: var(--color-bg-hover);
  color: var(--color-text);
  border-color: var(--color-primary);
}

.header-sep {
  color: var(--color-border);
  font-size: 13px;
  flex-shrink: 0;
}

.editor-title-container {
  display: flex;
  align-items: center;
  gap: 6px;
  min-width: 0;
}

.editor-title {
  font-size: 13px;
  font-weight: 600;
  color: var(--color-text);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.editor-title-edit-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 20px;
  height: 20px;
  padding: 0;
  border: none;
  border-radius: 4px;
  color: var(--color-text-muted);
  background: transparent;
  cursor: pointer;
  opacity: 0;
  transition: opacity 0.15s ease, color 0.15s ease, background 0.15s ease;
}

.editor-title-container:hover .editor-title-edit-btn,
.editor-title-container:focus-within .editor-title-edit-btn,
.editor-title-edit-btn:focus-visible {
  opacity: 1;
}

.editor-title-edit-btn:hover,
.editor-title-edit-btn:focus-visible {
  color: var(--color-primary);
  background: var(--color-bg-hover);
}

.editor-title-input {
  font-size: 13px;
  font-weight: 600;
  padding: 2px 8px;
  border: 1px solid var(--color-primary);
  border-radius: 4px;
  background: var(--color-bg);
  color: var(--color-text);
  outline: none;
  width: 160px;
}

.header-right {
  display: flex;
  align-items: center;
  flex-shrink: 0;
}

.api-view-toggle {
  display: inline-flex;
  background: var(--color-bg-secondary);
  border: 1px solid var(--color-border);
  border-radius: 6px;
  padding: 2px;
  gap: 2px;
}

.view-btn {
  padding: 3px 10px;
  font-size: 11px;
  font-weight: 500;
  border: none;
  background: transparent;
  color: var(--color-text-muted);
  border-radius: 4px;
  cursor: pointer;
  transition: all 0.15s ease;
  white-space: nowrap;
}

.view-btn:hover {
  color: var(--color-text);
}

.view-btn.active {
  background: var(--color-bg);
  color: var(--color-text);
  font-weight: 600;
  box-shadow: 0 1px 2px rgba(0, 0, 0, 0.06);
  border: 1px solid var(--color-border);
}
</style>

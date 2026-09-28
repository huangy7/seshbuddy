<script setup lang="ts">
import { ref, computed, watch } from "vue";
import { convertFileSrc } from "@tauri-apps/api/core";
import MonacoEditor from "./MonacoEditor.vue";
import SvgIcon from "./icons/SvgIcon.vue";
import InteractiveBreadcrumb from "./InteractiveBreadcrumb.vue";
import { t } from "../i18n";

const props = defineProps<{
  filePath: string;
  projectRoot: string;
  isDark: boolean;
}>();

const emit = defineEmits<{
  dirty: [isDirty: boolean];
  openFile: [path: string, projectRoot: string];
}>();

type ViewMode = "source" | "preview";
const mode = ref<ViewMode>("preview"); // Default to preview

// asset:// URL for iframe
const previewSrc = computed(() => convertFileSrc(props.filePath));

const editorRef = ref<InstanceType<typeof MonacoEditor> | null>(null);

function onDirty(dirty: boolean) {
  emit("dirty", dirty);
}

async function saveFile() {
  await editorRef.value?.saveFile();
}

defineExpose({ saveFile });

watch(() => props.filePath, () => {
  mode.value = "preview"; // Reset to preview on file change
});
</script>

<template>
  <div class="html-preview-wrapper">
    <div class="html-preview-toolbar">
      <InteractiveBreadcrumb 
        :filePath="filePath" 
        :projectRoot="projectRoot" 
        @openFile="(path, root) => emit('openFile', path, root)"
      />
      
      <div class="mode-btn-group">
        <button
          class="mode-btn"
          :class="{ active: mode === 'source' }"
          @click="mode = 'source'"
          :title="t('common.mdPreview.source')"
        >
          <SvgIcon name="code" :size="16" />
        </button>
        <button
          class="mode-btn"
          :class="{ active: mode === 'preview' }"
          @click="mode = 'preview'"
          :title="t('common.mdPreview.preview')"
        >
          <SvgIcon name="eye" :size="16" />
        </button>
      </div>
    </div>
    <div class="html-preview-body" :data-mode="mode">
      <MonacoEditor
        v-show="mode === 'source'"
        ref="editorRef"
        :filePath="filePath"
        :projectRoot="projectRoot"
        :isDark="isDark"
        :hideBreadcrumb="true"
        @dirty="onDirty"
      />
      <iframe
        v-show="mode === 'preview'"
        :src="previewSrc"
        class="html-iframe"
        sandbox="allow-scripts allow-same-origin"
      />
    </div>
  </div>
</template>

<style scoped>
.html-preview-wrapper {
  display: flex;
  flex-direction: column;
  flex: 1;
  min-height: 0;
}
.html-preview-toolbar {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding: 6px 16px;
  border-bottom: 1px solid var(--color-border);
  background: var(--color-bg);
  flex-shrink: 0;
  gap: 16px; /* Ensure some spacing between breadcrumb and buttons */
}
:deep(.interactive-breadcrumb) {
  flex: 1; /* Let breadcrumb expand and shrink */
  min-width: 0; /* Allow shrinking below content size */
  padding: 0;
  border-bottom: none;
  background: transparent;
}
.mode-btn-group {
  display: flex;
  background: var(--color-bg-secondary);
  border-radius: var(--radius-md);
  padding: 4px;
  gap: 2px;
  flex-shrink: 0; /* Never shrink buttons */
}
.mode-btn {
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 6px;
  border-radius: var(--radius-sm);
  color: var(--color-text-muted);
  cursor: pointer;
  border: none;
  background: transparent;
  transition: all var(--transition-fast);
}
.mode-btn.active {
  background: var(--color-bg);
  color: var(--color-text);
  box-shadow: 0 1px 3px rgba(0,0,0,0.1), 0 1px 2px rgba(0,0,0,0.06);
}
.mode-btn:hover:not(.active) {
  color: var(--color-text);
}
.html-preview-body {
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: row;
}
.html-preview-body > * {
  flex: 1;
  min-width: 0;
  min-height: 0;
}
.html-iframe {
  flex: 1;
  border: none;
  background: white;
}
.html-preview-body[data-mode="split"] .html-iframe {
  border-left: 1px solid var(--color-border);
}
</style>

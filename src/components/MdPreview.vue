<script setup lang="ts">
import { ref, computed, watch, onBeforeUnmount, nextTick } from "vue";
import { monaco } from "../monaco-workers";
import MonacoEditor from "./MonacoEditor.vue";
import { renderMarkdown } from "../utils/markdown";
import { vMermaid } from "../directives/vMermaid";
import { handleMarkdownLinkClick } from "../utils/markdownLinks";
import { invokeApp } from "../utils/invokeApp";
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

type ViewMode = "source" | "preview" | "split";
const mode = ref<ViewMode>("preview"); // Default to preview

const editorRef = ref<InstanceType<typeof MonacoEditor> | null>(null);
const previewRef = ref<HTMLDivElement | null>(null);
const rawContent = ref("");
const reloading = ref(false);

let syncingScroll = false;
let previewScrollTop = 0;

async function loadContent() {
  rawContent.value = await invokeApp<string>("read_file_content", {
    path: props.filePath,
    projectRoot: props.projectRoot,
  });
}

const renderedHtml = computed(() => renderMarkdown(rawContent.value));

function onDirty(dirty: boolean) {
  emit("dirty", dirty);
}

function onSaved() {
  void loadContent();
}

async function onReload() {
  reloading.value = true;
  const minDelay = new Promise((r) => setTimeout(r, 400));
  try {
    await loadContent();
  } finally {
    await minDelay;
    reloading.value = false;
  }
}

function onMarkdownClick(event: MouseEvent) {
  void handleMarkdownLinkClick(event, {
    currentFilePath: props.filePath,
    projectRoot: props.projectRoot,
    onOpenFile: ({ path }) => emit("openFile", path, props.projectRoot),
    onFailure: (failure) => console.warn("Markdown link was not opened:", failure),
  });
}

async function saveFile() {
  await editorRef.value?.saveFile();
}

defineExpose({ saveFile });

function getEditorInstance(): monaco.editor.IStandaloneCodeEditor | null {
  return editorRef.value?.getEditor?.() ?? null;
}

function onEditorScroll() {
  if (syncingScroll || mode.value !== "split") return;
  const editor = getEditorInstance();
  const preview = previewRef.value;
  if (!editor || !preview) return;

  syncingScroll = true;
  const scrollTop = editor.getScrollTop();
  const scrollHeight = editor.getScrollHeight();
  const clientHeight = editor.getLayoutInfo().height;
  const maxScroll = scrollHeight - clientHeight;
  const ratio = maxScroll > 0 ? scrollTop / maxScroll : 0;

  const previewMax = preview.scrollHeight - preview.clientHeight;
  preview.scrollTop = ratio * previewMax;
  requestAnimationFrame(() => { syncingScroll = false; });
}

function onPreviewScroll() {
  const preview = previewRef.value;
  if (preview) {
    previewScrollTop = preview.scrollTop;
  }
  if (syncingScroll || mode.value !== "split") return;
  const editor = getEditorInstance();
  if (!editor || !preview) return;

  syncingScroll = true;
  const previewMax = preview.scrollHeight - preview.clientHeight;
  const ratio = previewMax > 0 ? preview.scrollTop / previewMax : 0;

  const scrollHeight = editor.getScrollHeight();
  const clientHeight = editor.getLayoutInfo().height;
  const maxScroll = scrollHeight - clientHeight;
  editor.setScrollTop(ratio * maxScroll);
  requestAnimationFrame(() => { syncingScroll = false; });
}

function savePreviewScroll() {
  const preview = previewRef.value;
  if (preview) {
    previewScrollTop = preview.scrollTop;
  }
}

async function restorePreviewScroll() {
  await nextTick();
  await new Promise((resolve) => requestAnimationFrame(resolve));
  const preview = previewRef.value;
  if (preview) {
    preview.scrollTop = previewScrollTop;
  }
}

let scrollDisposable: monaco.IDisposable | null = null;

function setupEditorScrollSync() {
  scrollDisposable?.dispose();
  scrollDisposable = null;
  const editor = getEditorInstance();
  if (editor && mode.value === "split") {
    scrollDisposable = editor.onDidScrollChange(onEditorScroll);
  }
}

function onEditorReady() {
  setupEditorScrollSync();
}

watch(mode, (newMode, oldMode) => {
  if (oldMode !== "source") {
    savePreviewScroll();
  }
  nextTick(setupEditorScrollSync);
  if (newMode !== "source") {
    void restorePreviewScroll();
  }
});

watch(() => props.filePath, () => {
  previewScrollTop = 0;
  mode.value = "preview"; // Reset to preview on file change
  void loadContent().then(restorePreviewScroll);
}, { immediate: true });

onBeforeUnmount(() => {
  savePreviewScroll();
  scrollDisposable?.dispose();
});
</script>

<template>
  <div class="md-preview-wrapper">
    <div class="md-preview-toolbar">
      <InteractiveBreadcrumb 
        :filePath="filePath" 
        :projectRoot="projectRoot" 
        @openFile="(path, root) => emit('openFile', path, root)"
      />
      
      <button
        class="refresh-btn"
        :title="t('common.mdPreview.reload')"
        :disabled="reloading"
        @click="onReload"
      >
        <span :class="{ spinning: reloading }" class="refresh-icon-wrap">
          <SvgIcon :name="reloading ? 'loader' : 'refresh-cw'" :size="14" />
        </span>
      </button>

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
          :class="{ active: mode === 'split' }" 
          @click="mode = 'split'"
          :title="t('common.mdPreview.split')"
        >
          <SvgIcon name="columns" :size="16" />
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
    <div class="md-preview-body" :data-mode="mode">
      <MonacoEditor
        v-show="mode !== 'preview'"
        ref="editorRef"
        :filePath="filePath"
        :projectRoot="projectRoot"
        :isDark="isDark"
        :hideBreadcrumb="true"
        @dirty="onDirty"
        @saved="onSaved"
        @ready="onEditorReady"
      />
      <div
        v-show="mode !== 'source'"
        ref="previewRef"
        v-mermaid
        class="md-rendered markdown-body"
        v-html="renderedHtml"
        @click="onMarkdownClick"
        @scroll="onPreviewScroll"
      />
    </div>
  </div>
</template>

<style scoped>
.md-preview-wrapper {
  display: flex;
  flex-direction: column;
  flex: 1;
  min-height: 0;
  background: var(--color-bg);
}
.md-preview-toolbar {
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
.refresh-btn {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 28px;
  height: 28px;
  border-radius: var(--radius-sm);
  color: var(--color-text-muted);
  cursor: pointer;
  transition: all var(--transition-fast);
  flex-shrink: 0;
}
.refresh-btn:hover {
  background: var(--color-bg-hover);
  color: var(--color-text);
}
.refresh-btn:disabled {
  opacity: 0.5;
  cursor: default;
}
.refresh-icon-wrap {
  display: inline-flex;
  align-items: center;
  justify-content: center;
}
.spinning {
  animation: spin 0.8s linear infinite;
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
.md-preview-body {
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: row;
}
.md-preview-body > * {
  flex: 1;
  min-width: 0;
  min-height: 0;
}
.md-rendered {
  flex: 1;
  overflow-y: auto;
  padding: 24px;
  font-size: var(--text-sm);
  line-height: 1.6;
}
.md-preview-body[data-mode="split"] .md-rendered {
  border-left: 1px solid var(--color-border);
}
</style>

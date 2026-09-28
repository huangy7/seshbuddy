<script setup lang="ts">
import { ref, watch, onMounted, onBeforeUnmount, nextTick } from "vue";
import { monaco } from "../monaco-workers";
import { invokeApp, renderAppError, appErrorCode } from "../utils/invokeApp";
import { useFileWatcher } from "../composables/useFileWatcher";
import { useTheme } from "../composables/useTheme";
import { ask } from "@tauri-apps/plugin-dialog";
import { t } from "../i18n";

const props = defineProps<{
  filePath: string;
  projectRoot: string;
  isDark: boolean;
  hideBreadcrumb?: boolean;
}>();

import InteractiveBreadcrumb from "./InteractiveBreadcrumb.vue";
import SvgIcon from "./icons/SvgIcon.vue";

const emit = defineEmits<{
  dirty: [isDirty: boolean];
  saved: [];
  ready: [];
  openFile: [path: string, projectRoot: string];
}>();

const container = ref<HTMLDivElement | null>(null);
let editor: monaco.editor.IStandaloneCodeEditor | null = null;

const localContent = ref("");
const isDirty = ref(false);

const { watchFile, unwatchFile } = useFileWatcher();
const { resolvedTheme } = useTheme();

function getMonacoTheme(dark: boolean) {
  return dark ? "vs-dark" : "vs";
}

// ─── Language detection ───────────────────────────────────────────────────────

function getLang(path: string): string {
  const ext = path.split(".").pop()?.toLowerCase() ?? "";
  const map: Record<string, string> = {
    md: "markdown",
    markdown: "markdown",
    html: "html",
    htm: "html",
    json: "json",
    js: "javascript",
    ts: "typescript",
    css: "css",
    go: "go",
  };
  return map[ext] ?? "plaintext";
}

// 存**原始错误**而不是渲染好的句子：渲染结果随语言变，且从它里面嗅探子串判分支
// 正是 `invokeApp` 明文禁止的形态（「按后端文案子串判协议状态」）。分支判据用
// `appErrorCode`，展示用 `renderAppError`，两者都在模板里现取。
const loadError = ref<unknown>(null);

async function loadFile() {
  try {
    const content = await invokeApp<string>("read_file_content", {
      path: props.filePath,
      projectRoot: props.projectRoot,
    });
    localContent.value = content;
    loadError.value = null;
    if (editor) {
      const model = editor.getModel();
      if (model) {
        model.setValue(content);
      }
    }
    isDirty.value = false;
    emit("dirty", false);
  } catch (err: any) {
    loadError.value = err ?? "unknown";
  }
}

async function saveFile() {
  if (loadError.value) return;
  const content = editor?.getValue() ?? localContent.value;
  await invokeApp("save_file", {
    path: props.filePath,
    content,
    projectRoot: props.projectRoot,
  });
  localContent.value = content;
  isDirty.value = false;
  emit("dirty", false);
  emit("saved");
}

// Called when file-changed event fires for this path
async function onExternalChange(_path: string) {
  if (!isDirty.value) {
    await loadFile();
    return;
  }
  const confirmed = await ask(t("common.monacoEditor.conflictMessage"), {
    title: t("common.monacoEditor.conflictTitle"),
    kind: "warning",
    okLabel: t("common.monacoEditor.loadExternal"),
    cancelLabel: t("common.monacoEditor.keepLocal"),
  });
  if (confirmed) {
    await loadFile();
  }
}

onMounted(async () => {
  await nextTick();
  if (!container.value) return;

  await loadFile();

  // 错误态直接退出，editor 保持 null。前提：filePath 变化会重建组件实例
  // （tab.id 由 filePath 派生），因此不会出现"同实例从错误态恢复"的路径；
  // 若未来改变 tab 模型，需要在 watch(filePath) 成功分支补建 editor。
  if (loadError.value) {
    emit("ready");
    return;
  }

  const dark = resolvedTheme.value === "dark";

  editor = monaco.editor.create(container.value, {
    value: localContent.value,
    language: getLang(props.filePath),
    theme: getMonacoTheme(dark),
    automaticLayout: true,
    minimap: { enabled: false },
    scrollBeyondLastLine: false,
    fontSize: 13,
    lineNumbers: "on",
    wordWrap: "off",
  });

  monaco.editor.setTheme(getMonacoTheme(dark));

  editor.onDidChangeModelContent(() => {
    const dirty = editor!.getValue() !== localContent.value;
    if (dirty !== isDirty.value) {
      isDirty.value = dirty;
      emit("dirty", dirty);
    }
  });

  editor.addCommand(monaco.KeyMod.CtrlCmd | monaco.KeyCode.KeyS, () => {
    void saveFile();
  });

  await watchFile(props.filePath, onExternalChange);

  emit("ready");
});

watch(() => props.isDark, (dark) => {
  monaco.editor.setTheme(getMonacoTheme(dark));
});

watch(() => props.filePath, async (newPath, oldPath) => {
  await unwatchFile(oldPath);
  await loadFile();
  if (editor && !loadError.value) {
    const lang = getLang(newPath);
    const model = editor.getModel();
    if (model) {
      monaco.editor.setModelLanguage(model, lang);
    }
  }
  await watchFile(newPath, onExternalChange);
});

onBeforeUnmount(async () => {
  await unwatchFile(props.filePath);
  editor?.dispose();
  editor = null;
});

function getEditor() { return editor; }

defineExpose({ saveFile, isDirty, getEditor });
</script>

<template>
  <div class="monaco-wrapper">
    <!-- Breadcrumb navigation -->
    <InteractiveBreadcrumb 
      v-if="!hideBreadcrumb" 
      :filePath="filePath" 
      :projectRoot="projectRoot" 
      @openFile="(path, root) => emit('openFile', path, root)"
    />
    <div v-if="loadError" class="editor-error-state">
      <SvgIcon name="alert-circle" :size="32" class="error-icon" />
      <div class="error-title">
        {{ appErrorCode(loadError) === "fs.not_text_file" ? t('common.monacoEditor.binaryPreviewUnsupported') : t('common.monacoEditor.readFileFailed') }}
      </div>
      <div class="error-desc">{{ renderAppError(loadError) }}</div>
    </div>
    <div v-else ref="container" class="monaco-container" />
  </div>
</template>

<style scoped>
.monaco-wrapper {
  display: flex;
  flex-direction: column;
  flex: 1;
  min-height: 0;
  min-width: 0;
  overflow: hidden;
}
:deep(.interactive-breadcrumb) {
  flex-shrink: 0;
  min-width: 0;
  overflow-x: auto;
}
.monaco-container {
  flex: 1;
  min-height: 0;
}
.editor-error-state {
  flex: 1;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 12px;
  color: var(--color-text-muted);
  text-align: center;
  padding: 32px;
}
.error-icon {
  color: var(--color-warning, #f59e0b);
}
.error-title {
  font-size: 15px;
  font-weight: 600;
  color: var(--color-text);
}
.error-desc {
  font-size: 12px;
  font-family: var(--font-mono, monospace);
  word-break: break-all;
  max-width: 480px;
}
</style>

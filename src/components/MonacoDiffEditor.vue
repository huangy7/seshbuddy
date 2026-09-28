<script setup lang="ts">
import { ref, onMounted, onBeforeUnmount, watch, nextTick } from "vue";
import { monaco } from "../monaco-workers";
import { useTheme } from "../composables/useTheme";

const props = defineProps<{
  original: string;
  modified: string;
  filePath: string;
  isDark: boolean;
}>();

const { resolvedTheme } = useTheme();
const container = ref<HTMLDivElement | null>(null);
let diffEditor: monaco.editor.IStandaloneDiffEditor | null = null;

function getLang(path: string): string {
  const ext = path.split(".").pop()?.toLowerCase() ?? "";
  const map: Record<string, string> = {
    md: "markdown", markdown: "markdown", html: "html", htm: "html",
    json: "json", js: "javascript", ts: "typescript", css: "css", go: "go",
    rs: "rust", py: "python", java: "java", yml: "yaml", yaml: "yaml",
    toml: "toml", sh: "shell", bash: "shell",
  };
  return map[ext] ?? "plaintext";
}

function getTheme(dark: boolean) {
  return dark ? "vs-dark" : "vs";
}

onMounted(async () => {
  await nextTick();
  if (!container.value) return;

  const lang = getLang(props.filePath);
  const dark = resolvedTheme.value === "dark";

  monaco.editor.setTheme(getTheme(dark));
  diffEditor = monaco.editor.createDiffEditor(container.value, {
    automaticLayout: true,
    readOnly: true,
    renderSideBySide: true,
    minimap: { enabled: false },
    scrollBeyondLastLine: false,
    fontSize: 13,
  });

  const originalModel = monaco.editor.createModel(props.original, lang);
  const modifiedModel = monaco.editor.createModel(props.modified, lang);
  diffEditor.setModel({ original: originalModel, modified: modifiedModel });
});

watch(() => props.isDark, (dark) => {
  monaco.editor.setTheme(getTheme(dark));
});

watch([() => props.original, () => props.modified], () => {
  if (!diffEditor) return;
  const lang = getLang(props.filePath);
  const originalModel = monaco.editor.createModel(props.original, lang);
  const modifiedModel = monaco.editor.createModel(props.modified, lang);
  diffEditor.setModel({ original: originalModel, modified: modifiedModel });
});

onBeforeUnmount(() => {
  const model = diffEditor?.getModel();
  model?.original?.dispose();
  model?.modified?.dispose();
  diffEditor?.dispose();
  diffEditor = null;
});
</script>

<template>
  <div class="diff-editor-wrapper">
    <div ref="container" class="diff-editor-container" />
  </div>
</template>

<style scoped>
.diff-editor-wrapper {
  display: flex;
  flex-direction: column;
  flex: 1;
  min-height: 0;
  min-width: 0;
  overflow: hidden;
}
.diff-editor-container {
  flex: 1;
  min-height: 0;
}
</style>

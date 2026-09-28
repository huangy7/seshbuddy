<script setup lang="ts">
import { ref, watch, onMounted, onBeforeUnmount, nextTick } from "vue";
import { monaco } from "../../monaco-workers";
import { useTheme } from "../../composables/useTheme";

// 通用只读 Monaco 查看器：自带搜索（Ctrl+F）、折叠、语法高亮与大文件加速模式
const props = withDefaults(defineProps<{
  value: string;
  language?: string;
}>(), {
  language: "plaintext",
});

const container = ref<HTMLDivElement | null>(null);
let editor: monaco.editor.IStandaloneCodeEditor | null = null;

const { resolvedTheme } = useTheme();

function getMonacoTheme(dark: boolean) {
  return dark ? "vs-dark" : "vs";
}

onMounted(async () => {
  await nextTick();
  if (!container.value) return;

  const isLargeFile = (props.value?.length ?? 0) > 200_000;

  editor = monaco.editor.create(container.value, {
    value: props.value,
    language: props.language,
    theme: getMonacoTheme(resolvedTheme.value === "dark"),
    automaticLayout: true,
    minimap: { enabled: false },
    scrollBeyondLastLine: false,
    fontSize: 12,
    lineNumbers: "on",
    wordWrap: isLargeFile ? "off" : "on",
    tabSize: 2,
    lineNumbersMinChars: 3,
    overviewRulerLanes: 0,
    folding: !isLargeFile,
    readOnly: true,
    domReadOnly: true,
    scrollbar: {
      verticalScrollbarSize: 8,
      horizontalScrollbarSize: 8,
    },
  });
});

watch(() => props.value, (newVal) => {
  if (editor && editor.getValue() !== newVal) {
    const isLarge = (newVal?.length ?? 0) > 200_000;
    editor.updateOptions({
      wordWrap: isLarge ? "off" : "on",
      folding: !isLarge,
    });
    editor.setValue(newVal ?? "");
  }
});

watch(() => props.language, (lang) => {
  const model = editor?.getModel();
  if (model) monaco.editor.setModelLanguage(model, lang);
});

watch(resolvedTheme, (newTheme) => {
  monaco.editor.setTheme(getMonacoTheme(newTheme === "dark"));
});

onBeforeUnmount(() => {
  editor?.dispose();
  editor = null;
});
</script>

<template>
  <div ref="container" class="monaco-viewer" />
</template>

<style scoped>
.monaco-viewer {
  width: 100%;
  height: 100%;
  min-height: 0;
  border: 1px solid var(--color-border);
  border-radius: var(--radius-md);
  overflow: hidden;
}
</style>

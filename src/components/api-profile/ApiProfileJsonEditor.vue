<script setup lang="ts">
import { ref, watch, onMounted, onBeforeUnmount, nextTick } from "vue";
import { monaco } from "../../monaco-workers";
import { useTheme } from "../../composables/useTheme";

const props = withDefaults(
  defineProps<{
    errorMessage: string;
    jsonText: string;
    language?: "json" | "toml";
  }>(),
  {
    language: "json",
  }
);

const emit = defineEmits<{
  updateJson: [text: string];
}>();

const container = ref<HTMLDivElement | null>(null);
let editor: monaco.editor.IStandaloneCodeEditor | null = null;
let isUpdatingFromProp = false;

const { resolvedTheme } = useTheme();

function getMonacoTheme(dark: boolean) {
  return dark ? "vs-dark" : "vs";
}

onMounted(async () => {
  await nextTick();
  if (!container.value) return;

  const dark = resolvedTheme.value === "dark";

  editor = monaco.editor.create(container.value, {
    value: props.jsonText,
    language: props.language || "json",
    theme: getMonacoTheme(dark),
    automaticLayout: true,
    minimap: { enabled: false },
    scrollBeyondLastLine: false,
    fontSize: 13,
    lineNumbers: "on",
    wordWrap: "on",
    formatOnPaste: true,
    tabSize: 2,
    lineNumbersMinChars: 3,
    overviewRulerLanes: 0,
    readOnly: true,
    domReadOnly: true,
    scrollbar: {
      verticalScrollbarSize: 8,
      horizontalScrollbarSize: 8,
    },
  });

  editor.onDidChangeModelContent(() => {
    if (isUpdatingFromProp) return;
    const val = editor!.getValue();
    emit("updateJson", val);
  });
});

watch(() => props.jsonText, (newVal) => {
  if (editor && editor.getValue() !== newVal) {
    isUpdatingFromProp = true;
    editor.setValue(newVal);
    isUpdatingFromProp = false;
  }
});

watch(resolvedTheme, (newTheme) => {
  const dark = newTheme === "dark";
  monaco.editor.setTheme(getMonacoTheme(dark));
});

watch(() => props.language, (newLang) => {
  if (editor && newLang) {
    const model = editor.getModel();
    if (model) {
      monaco.editor.setModelLanguage(model, newLang);
    }
  }
});

onBeforeUnmount(() => {
  editor?.dispose();
  editor = null;
});
</script>

<template>
  <section class="settings-section json-section">
    <span v-if="errorMessage" class="json-error">{{ errorMessage }}</span>
    <div ref="container" class="monaco-container"></div>
  </section>
</template>

<style scoped>
.settings-section {
  margin-bottom: var(--space-5);
  display: flex;
  flex-direction: column;
  flex: 1;
  min-height: 0;
}
.json-section {
  display: flex;
  flex-direction: column;
  flex: 1;
}
.json-error {
  font-size: var(--text-xs);
  color: var(--color-danger);
  margin-bottom: var(--space-2);
  flex-shrink: 0;
}
.monaco-container {
  width: 100%;
  flex: 1;
  min-height: 350px;
  border: 1px solid var(--color-border);
  border-radius: var(--radius-md);
  overflow: hidden;
}
</style>

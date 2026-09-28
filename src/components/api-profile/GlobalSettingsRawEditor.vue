<script setup lang="ts">
import { ref, watch, onMounted, onBeforeUnmount, nextTick, computed } from "vue";
import { t } from "../../i18n";
import { invokeApp, renderAppError } from "../../utils/invokeApp";
import { monaco } from "../../monaco-workers";
import { useTheme } from "../../composables/useTheme";
import SvgIcon from "../icons/SvgIcon.vue";

const props = defineProps<{
  cliId: string;
  scope: string;
}>();

const emit = defineEmits<{
  close: [];
  saved: [];
}>();

const container = ref<HTMLDivElement | null>(null);
let editor: monaco.editor.IStandaloneCodeEditor | null = null;
const { resolvedTheme } = useTheme();
const isSaving = ref(false);
const errorMessage = ref("");
const originalContent = ref("");
const hasChanges = ref(false);
const isSuccess = ref(false);

const isCodex = computed(() => props.cliId === 'codex');
const isToml = computed(() => isCodex.value);
const editorLanguage = computed(() => isToml.value ? 'toml' : 'json');

function getMonacoTheme(dark: boolean) {
  return dark ? "vs-dark" : "vs";
}

async function loadSettings() {
  try {
    errorMessage.value = "";
    const content = await invokeApp<string>("read_scope_settings", {
      cliId: props.cliId,
      scope: props.scope === "global" ? null : props.scope,
    });
    originalContent.value = content;
    if (editor) {
      editor.setValue(content);
      hasChanges.value = false;
    }
  } catch (err: any) {
    errorMessage.value = t("api-profile.rawEditor.loadFailed", { error: renderAppError(err) });
  }
}

onMounted(async () => {
  await nextTick();
  if (!container.value) return;

  const dark = resolvedTheme.value === "dark";

  editor = monaco.editor.create(container.value, {
    value: "",
    language: editorLanguage.value,
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
    scrollbar: {
      verticalScrollbarSize: 8,
      horizontalScrollbarSize: 8,
    },
  });

  editor.onDidChangeModelContent(() => {
    hasChanges.value = editor?.getValue() !== originalContent.value;
  });

  await loadSettings();
});

watch(resolvedTheme, (newTheme) => {
  const dark = newTheme === "dark";
  monaco.editor.setTheme(getMonacoTheme(dark));
});

onBeforeUnmount(() => {
  editor?.dispose();
  editor = null;
});

function handleFormat() {
  if (editor && !isToml.value) {
    try {
      const content = editor.getValue();
      const parsed = JSON.parse(content);
      const formatted = JSON.stringify(parsed, null, 2);
      
      editor.pushUndoStop();
      editor.executeEdits("format", [{
        range: editor.getModel()!.getFullModelRange(),
        text: formatted
      }]);
      editor.pushUndoStop();
      
      errorMessage.value = "";
    } catch (e: any) {
      errorMessage.value = t("api-profile.rawEditor.formatInvalid", { error: renderAppError(e) });
    }
  }
}

async function handleSave() {
  if (!editor || isSaving.value) return;
  const content = editor.getValue();
  
  // Basic JSON validation for JSON configs
  if (!isToml.value) {
    try {
      JSON.parse(content);
    } catch (e: any) {
      errorMessage.value = t("api-profile.rawEditor.invalidJson", { error: renderAppError(e) });
      return;
    }
  }

  try {
    isSaving.value = true;
    errorMessage.value = "";
    await invokeApp("write_scope_settings", {
      cliId: props.cliId,
      scope: props.scope === "global" ? null : props.scope,
      content,
    });
    
    // Sync the active profile from the newly updated raw config
    await invokeApp("sync_active_profile_from_cli", {
      cliId: props.cliId,
      scope: props.scope === "global" ? null : props.scope,
    });
    
    originalContent.value = content;
    hasChanges.value = false;
    isSuccess.value = true;
    setTimeout(() => {
      isSuccess.value = false;
    }, 2000);
    emit("saved");
  } catch (err: any) {
    errorMessage.value = t("api-profile.rawEditor.saveFailed", { error: renderAppError(err) });
  } finally {
    isSaving.value = false;
  }
}

const currentFilePath = computed(() => {
  if (props.cliId === 'claude') {
    return props.scope === 'global' ? '~/.claude/settings.json' : t("api-profile.rawEditor.projectDirPath");
  } else if (props.cliId === 'codex') {
    return props.scope === 'global' ? '~/.codex/config.toml' : t("api-profile.rawEditor.projectConfigPath");
  }
  return '';
});
</script>

<template>
  <div class="modal-backdrop" @click.self="emit('close')">
    <div class="raw-editor-modal">
      <div class="modal-header">
        <div class="header-left">
          <div class="title-wrapper">
            <div class="title-row">
              <h2 class="modal-title">{{ isToml ? t("api-profile.rawEditor.tomlTitle") : t("api-profile.rawEditor.jsonTitle") }}</h2>
              <span class="scope-badge" :class="{ 'global': scope === 'global' }">
                {{ scope === 'global' ? t("api-profile.scope.global") : t("api-profile.rawEditor.scopeProject") }}
              </span>
            </div>
            <div class="file-path-hint">{{ currentFilePath }}</div>
          </div>
        </div>
        <div class="header-actions">
          <button v-if="!isToml" class="action-icon-btn format-btn" @click="handleFormat" :title="t('api-profile.rawEditor.formatJsonTitle')">
            <SvgIcon name="align-left" :size="16" />
          </button>
          <button class="primary-btn save-btn" :class="{ 'is-loading': isSaving, 'is-success': isSuccess }" @click="handleSave" :disabled="isSaving || (!hasChanges && !isSuccess)">
            <SvgIcon v-if="isSaving" name="loader" :size="14" class="spin-icon" />
            <SvgIcon v-else-if="isSuccess" name="check" :size="14" />
            <span v-if="!isSaving && !isSuccess">{{ t("api-profile.rawEditor.save") }}</span>
            <span v-else-if="isSuccess">{{ t("api-profile.rawEditor.saved") }}</span>
            <span v-else>{{ t("api-profile.rawEditor.saving") }}</span>
          </button>
          <div class="action-divider"></div>
          <button class="icon-btn close-btn" @click="emit('close')" :title="t('dialogs.common.close')">
            <SvgIcon name="x" :size="16" />
          </button>
        </div>
      </div>
      
      <div class="modal-body">
        <div v-if="errorMessage" class="error-banner">
          <SvgIcon name="alert-circle" :size="14" class="error-icon" />
          <span>{{ errorMessage }}</span>
        </div>
        <div ref="container" class="monaco-container"></div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.modal-backdrop {
  padding: var(--space-6);
}

.raw-editor-modal {
  width: 100%;
  max-width: 900px;
  height: 85vh;
  background: var(--color-bg);
  border-radius: var(--radius-lg);
  box-shadow: 0 25px 50px -12px rgba(0, 0, 0, 0.25);
  display: flex;
  flex-direction: column;
  overflow: hidden;
  border: 1px solid var(--color-border);
}

/* Parent transition hook for inner modal animation */
.fade-enter-active .raw-editor-modal,
.fade-leave-active .raw-editor-modal {
  transition: transform var(--transition-base) cubic-bezier(0.16, 1, 0.3, 1);
}

.fade-enter-from .raw-editor-modal,
.fade-leave-to .raw-editor-modal {
  transform: scale(0.97) translateY(10px);
}

.modal-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: var(--space-4) var(--space-5);
  border-bottom: 1px solid var(--color-border);
  background: var(--color-bg);
}

.header-left {
  display: flex;
  align-items: center;
}

.title-wrapper {
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.title-row {
  display: flex;
  align-items: center;
  gap: var(--space-2);
}

.modal-title {
  font-size: var(--text-base);
  font-weight: 600;
  color: var(--color-text);
  margin: 0;
}

.file-path-hint {
  font-size: 11px;
  color: var(--color-text-muted);
  font-family: var(--font-mono);
}

.scope-badge {
  font-size: 11px;
  font-weight: 500;
  padding: 2px 6px;
  border-radius: var(--radius-sm);
  background: rgba(59, 130, 246, 0.1);
  color: var(--color-primary);
  border: 1px solid rgba(59, 130, 246, 0.2);
}

.scope-badge.global {
  background: rgba(16, 185, 129, 0.1);
  color: var(--color-success, #10b981);
  border-color: rgba(16, 185, 129, 0.2);
}

.header-actions {
  display: flex;
  align-items: center;
  gap: var(--space-3);
}

.action-divider {
  width: 1px;
  height: 20px;
  background: var(--color-border);
  margin: 0 var(--space-1);
}

.action-icon-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 32px;
  height: 32px;
  color: var(--color-text-muted);
  background: var(--color-bg-mute, rgba(0, 0, 0, 0.03));
  border: 1px solid var(--color-border);
  border-radius: var(--radius-md);
  cursor: pointer;
  transition: all var(--transition-fast);
}

.action-icon-btn:hover:not(:disabled) {
  background: var(--color-bg-hover, rgba(0, 0, 0, 0.08));
  color: var(--color-text);
}

.action-icon-btn:active:not(:disabled) {
  transform: scale(0.96);
}

.primary-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 6px;
  padding: 6px 16px;
  font-size: var(--text-sm);
  font-weight: 500;
  color: white;
  background: var(--color-primary);
  border: 1px solid transparent;
  border-radius: var(--radius-md);
  cursor: pointer;
  transition: all var(--transition-fast);
}

.primary-btn.is-success {
  background: var(--color-success, #10b981);
}

.primary-btn:hover:not(:disabled) {
  background: var(--color-primary-hover);
  box-shadow: 0 0 0 1px var(--color-bg), 0 0 0 3px rgba(59, 130, 246, 0.2);
}

.primary-btn.is-success:hover:not(:disabled) {
  background: var(--color-success, #10b981);
  box-shadow: 0 0 0 1px var(--color-bg), 0 0 0 3px rgba(16, 185, 129, 0.2);
}

.primary-btn:disabled {
  opacity: 0.5;
  cursor: not-allowed;
  filter: grayscale(0.2);
}

.icon-btn.close-btn {
  margin-left: var(--space-1);
}

.spin-icon {
  animation: spin 1s linear infinite;
}

.modal-body {
  flex: 1;
  display: flex;
  flex-direction: column;
  min-height: 0;
  padding: var(--space-4);
  gap: var(--space-3);
}

.error-banner {
  display: flex;
  align-items: flex-start;
  gap: var(--space-3);
  padding: var(--space-3) var(--space-4);
  background: linear-gradient(to right, rgba(220, 38, 38, 0.08), rgba(220, 38, 38, 0.02));
  color: var(--color-danger);
  border-left: 4px solid var(--color-danger);
  border-radius: var(--radius-md);
  font-size: var(--text-sm);
  font-weight: 500;
  box-shadow: 0 4px 12px rgba(220, 38, 38, 0.05);
}

.error-icon {
  flex-shrink: 0;
}

.monaco-container {
  flex: 1;
  width: 100%;
  border: 1px solid var(--color-border);
  border-radius: var(--radius-md);
  overflow: hidden;
  box-shadow: 0 2px 10px rgba(0, 0, 0, 0.02);
}
</style>

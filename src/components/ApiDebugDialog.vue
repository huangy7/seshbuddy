<script setup lang="ts">
import { computed, ref, watch } from "vue";
import SvgIcon from "./icons/SvgIcon.vue";
import ApiLogView from "./ApiLogView.vue";
import { useSessions } from "../composables/useSessions";
import { resolveFeatureCliId } from "../composables/cliFilter";
import { getCliDefinition, isCliId, type CliId } from "../types/cli";
import { t } from "../i18n";

const emit = defineEmits<{ close: [] }>();
const props = defineProps<{ initialCliId?: CliId; initialSessionId?: string }>();
const { cliOptions } = useSessions();
const apiLogCliOptions = computed(() => cliOptions.value.filter((cli) => cli.supportsApiLogs));
const persistedCliId = localStorage.getItem("seshbuddy-feature-cli-api-log");
const maximized = ref(false);
const minimized = ref(false);
const selectedCliId = ref<CliId | undefined>(resolveFeatureCliId(
  apiLogCliOptions.value,
  props.initialCliId,
  persistedCliId && isCliId(persistedCliId) ? persistedCliId : undefined,
));

const dialogTitle = computed(() => selectedCliId.value
  ? t("dialogs.apiDebug.titleWithCli", { name: getCliDefinition(selectedCliId.value).name })
  : t("dialogs.apiDebug.title"));

watch(
  () => props.initialCliId,
  (entryCliId) => {
    selectedCliId.value = resolveFeatureCliId(
      apiLogCliOptions.value,
      entryCliId,
      selectedCliId.value,
    );
  },
);

function restoreFromMinimized() {
  minimized.value = false;
}

defineExpose({
  restoreFromMinimized
});
</script>

<template>
  <Transition name="fade">
    <div class="modal-backdrop" :class="{ 'modal-backdrop--maximized': maximized, 'modal-backdrop--minimized': minimized }" @click.self="emit('close')">
      <div v-show="!minimized" class="api-debug-window" :class="{ 'api-debug-window-maximized': maximized }">
        <div class="api-debug-header">
          <h2 class="api-debug-title">
            <SvgIcon name="zap" :size="18" />
            {{ dialogTitle }}
          </h2>
          <div class="api-debug-actions">
            <button class="header-btn" :title="t('dialogs.apiDebug.minimize')" @click="minimized = true">
              <SvgIcon name="minus" :size="16" />
            </button>
            <button
              class="header-btn"
              :title="maximized ? t('dialogs.apiDebug.restore') : t('dialogs.apiDebug.maximize')"
              @click="maximized = !maximized"
            >
              <SvgIcon :name="maximized ? 'minimize-2' : 'maximize-2'" :size="16" />
            </button>
            <button class="header-btn" :title="t('dialogs.common.close')" @click="emit('close')">
              <SvgIcon name="x" :size="18" />
            </button>
          </div>
        </div>

        <div class="api-debug-body">
          <ApiLogView
            :initial-cli-id="selectedCliId"
            :initial-session-id="props.initialSessionId"
            @update:cli-id="selectedCliId = $event"
          />
        </div>
      </div>

    </div>
  </Transition>

  <Teleport v-if="minimized" to="#minimized-widgets">
    <Transition name="fade">
      <div class="minimized-widget dialog-minimized-widget" @click="minimized = false">
        <SvgIcon name="zap" :size="20" class="minimized-icon" />
        <div class="minimized-info">
          <div class="minimized-title">{{ t("dialogs.apiDebug.title") }}</div>
          <div class="minimized-status">{{ t("dialogs.apiDebug.runningInBackground") }}</div>
        </div>
        <button class="header-btn close-btn" @click.stop="emit('close')" :title="t('dialogs.common.close')">
          <SvgIcon name="x" :size="16" />
        </button>
      </div>
    </Transition>
  </Teleport>
</template>

<style scoped>
.modal-backdrop {
  padding: 24px;
}

.modal-backdrop--maximized {
  padding: 0;
}

.api-debug-window {
  width: min(1320px, 96vw);
  height: min(840px, 90vh);
  background: var(--color-bg);
  border-radius: var(--radius-xl);
  box-shadow: var(--shadow-lg);
  display: flex;
  flex-direction: column;
  overflow: hidden;
  pointer-events: auto;
}

.api-debug-window-maximized {
  width: 100vw;
  height: 100vh;
  border-radius: 0;
  box-shadow: none;
  transition: none !important;
}

/* 共享样式见 src/styles/utilities.css 的 .minimized-widget 系列；
   此处仅保留本组件特有的 close-btn 微调。 */
.dialog-minimized-widget .close-btn {
  margin-left: 8px;
  width: 24px;
  height: 24px;
  opacity: 0.5;
  transition: all var(--transition-base);
  background: transparent;
}
.dialog-minimized-widget .close-btn:hover {
  opacity: 1;
  background: rgba(239, 68, 68, 0.1);
  color: #ef4444;
}

.api-debug-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-3);
  padding: var(--space-4) var(--space-5);
  border-bottom: 1px solid var(--color-border);
  background: var(--color-bg);
  flex-shrink: 0;
}

.api-debug-title {
  margin: 0;
  display: inline-flex;
  align-items: center;
  gap: var(--space-2);
  font-size: var(--text-lg);
  font-weight: 600;
}

.api-debug-actions {
  display: flex;
  align-items: center;
  gap: var(--space-1);
}

.header-btn {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 32px;
  height: 32px;
  border-radius: var(--radius-md);
  color: var(--color-text-muted);
  transition: all var(--transition-fast);
}

.header-btn:hover {
  background: var(--color-bg-hover);
  color: var(--color-text);
}

.api-debug-body {
  position: relative;
  flex: 1;
  min-height: 0;
  background: var(--color-bg);
}

.fade-enter-active,
.fade-leave-active {
  transition: opacity 0.15s;
}

.fade-enter-from,
.fade-leave-to {
  opacity: 0;
}

@media (max-width: 900px) {
  .modal-backdrop {
    padding: 12px;
  }

  .api-debug-window {
    width: 100%;
    height: min(860px, 92vh);
  }
}
</style>

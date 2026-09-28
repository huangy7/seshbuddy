<script setup lang="ts">
import { computed } from "vue";
import SvgIcon from "../icons/SvgIcon.vue";
import AgentTerminal from "../AgentTerminal.vue";
import type { TerminalPaneLeaf } from "../../types/terminal";
import { isCliId, getCliDefinition } from "../../types/cli";
import { basename } from "../../utils/projectPath";
import { t } from "../../i18n";

const props = withDefaults(
  defineProps<{
    leaf: TerminalPaneLeaf;
    isActive: boolean;
    canClose: boolean;
    showHeader?: boolean;
    isMaximized?: boolean;
  }>(),
  { showHeader: true, isMaximized: false }
);

const emit = defineEmits<{
  focus: [];
  splitRight: [];
  splitDown: [];
  close: [];
  toggleMaximize: [];
  openFile: [path: string, projectRoot: string];
}>();

const cliLabel = computed(() => {
  if (props.leaf.customTitle) return props.leaf.customTitle;
  if (props.leaf.cliKind === "shell" || props.leaf.cliKind === "terminal" || !props.leaf.cliKind) {
    return t("common.terminal.systemTerminal");
  }
  if (isCliId(props.leaf.cliKind)) {
    return getCliDefinition(props.leaf.cliKind).name;
  }
  return props.leaf.cliKind;
});

const cliIcon = computed(() => {
  if (props.leaf.cliKind === "codex") return "cpu";
  if (props.leaf.cliKind === "gemini") return "sparkles";
  if (props.leaf.cliKind === "claude") return "terminal";
  return "terminal";
});

function onPaneMouseDown() {
  if (!props.isActive) {
    emit("focus");
  }
}
</script>

<template>
  <div
    class="terminal-pane"
    :class="{
      active: isActive,
      maximized: isMaximized
    }"
    @mousedown="onPaneMouseDown"
  >
    <!-- Ultra-slim Sub-Header (Only shown when multiple splits or maximized, 22px, zero occlusion) -->
    <div
      v-if="showHeader || isMaximized"
      class="pane-header"
      :class="{ 'is-maximized': isMaximized }"
      @dblclick="emit('toggleMaximize')"
    >
      <div class="pane-title-group">
        <SvgIcon :name="cliIcon" :size="11" class="pane-cli-icon" />
        <span class="pane-title">{{ cliLabel }}</span>
        <span v-if="leaf.projectPath" class="pane-path" :title="leaf.projectPath">
          {{ basename(leaf.projectPath) }}
        </span>
      </div>

      <div class="pane-actions" @mousedown.stop>
        <button
          type="button"
          class="pane-btn"
          :class="{ active: isMaximized }"
          :title="isMaximized ? t('common.terminal.restoreSplit') : t('common.terminal.tempFullscreen')"
          @click="emit('toggleMaximize')"
        >
          <SvgIcon :name="isMaximized ? 'minimize-2' : 'maximize-2'" :size="11" />
        </button>
        <button
          type="button"
          class="pane-btn"
          :title="t('common.terminal.splitRight')"
          @click="emit('splitRight')"
        >
          <SvgIcon name="columns" :size="11" />
        </button>
        <button
          type="button"
          class="pane-btn"
          :title="t('common.terminal.splitDown')"
          @click="emit('splitDown')"
        >
          <SvgIcon name="rows" :size="11" />
        </button>
        <button
          v-if="canClose"
          type="button"
          class="pane-btn close"
          :title="t('common.terminal.closeSplit')"
          @click="emit('close')"
        >
          <SvgIcon name="x" :size="11" />
        </button>
      </div>
    </div>

    <!-- Terminal Canvas -->
    <div class="pane-body">
      <AgentTerminal
        :sessionId="leaf.sessionId"
        :projectRoot="leaf.projectPath"
        @openFile="(path, root) => emit('openFile', path, root)"
      />
    </div>
  </div>
</template>

<style scoped>
.terminal-pane {
  display: flex;
  flex-direction: column;
  flex: 1;
  height: 100%;
  width: 100%;
  min-width: 0;
  min-height: 0;
  background: var(--color-bg, #1e1e1e);
  position: relative;
  overflow: hidden;
  box-sizing: border-box;
}

.terminal-pane.active {
  box-shadow: inset 0 0 0 1px var(--color-border-subtle, rgba(255, 255, 255, 0.08));
}

.terminal-pane.maximized {
  box-shadow: none;
}

/* 22px Ultra-Slim Sub-Header (Zero Occlusion, seamless transparent) */
.pane-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  height: 22px;
  min-height: 22px;
  padding: 0 6px 0 8px;
  background: transparent;
  border-bottom: 1px solid var(--color-border-subtle, rgba(255, 255, 255, 0.05));
  user-select: none;
  -webkit-user-select: none;
  cursor: pointer;
  box-sizing: border-box;
}

.pane-title-group {
  display: flex;
  align-items: center;
  gap: 5px;
  min-width: 0;
  overflow: hidden;
  user-select: none;
  -webkit-user-select: none;
}

.pane-cli-icon {
  flex-shrink: 0;
  color: var(--color-text-muted, #9ca3af);
  transition: color 120ms ease;
  user-select: none;
  -webkit-user-select: none;
}

.terminal-pane.active .pane-cli-icon {
  color: var(--color-primary, #3b82f6);
}

.pane-title {
  font-size: 11px;
  font-weight: 500;
  color: var(--color-text-muted, #9ca3af);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  user-select: none;
  -webkit-user-select: none;
}

.terminal-pane.active .pane-title {
  color: var(--color-text, #e5e7eb);
}

.pane-path {
  font-size: 10px;
  color: var(--color-text-muted, #6b7280);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  opacity: 0.8;
  user-select: none;
  -webkit-user-select: none;
}

/* Action Buttons (Subtle default opacity, lights up smoothly on hover) */
.pane-actions {
  display: flex;
  align-items: center;
  gap: 2px;
  opacity: 0.45;
  transition: opacity 120ms ease;
  flex-shrink: 0;
}

.terminal-pane:hover .pane-actions,
.pane-header:hover .pane-actions,
.terminal-pane.maximized .pane-actions {
  opacity: 1;
}

.pane-btn {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 18px;
  height: 18px;
  border-radius: 3px;
  color: var(--color-text-muted, #9ca3af);
  background: transparent;
  cursor: pointer;
  transition: all 100ms ease;
}

.pane-btn:hover {
  background: rgba(255, 255, 255, 0.1);
  color: var(--color-text, #ffffff);
}

.pane-btn.active {
  color: var(--color-primary, #3b82f6);
}

.pane-btn.close:hover {
  background: rgba(239, 68, 68, 0.2);
  color: #ef4444;
}

.pane-body {
  flex: 1;
  height: 100%;
  width: 100%;
  min-width: 0;
  min-height: 0;
  display: flex;
  flex-direction: column;
}
</style>

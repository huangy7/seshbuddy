<script setup lang="ts">
import SvgIcon from "../icons/SvgIcon.vue";
import { t } from "../../i18n";
import type { SubagentInfo } from "../../types/session";
import { extractWidgetData, ensureWorkBuddySvgStyles } from "../../utils/svg";
import { vMermaid } from "../../directives/vMermaid";

defineProps<{
  expanded: boolean;
  toolName: string;
  summaryHtml: string;
  markdownHtml: string | null;
  inputHtml: string;
  subagent: SubagentInfo | null;
}>();

defineEmits<{
  toggle: [];
  markdownClick: [event: MouseEvent];
  openSubagent: [filePath: string, label: string];
}>();

function getWidget(inputHtml: string) {
  return extractWidgetData(inputHtml);
}
</script>

<template>
  <div class="tool-use-block">
    <div
      class="tool-use-header"
      role="button"
      tabindex="0"
      :aria-expanded="expanded"
      @click="$emit('toggle')"
      @keydown.enter.prevent="$emit('toggle')"
      @keydown.space.prevent="$emit('toggle')"
    >
      <SvgIcon
        :name="expanded ? 'chevron-down' : 'chevron-right'"
        :size="12"
        class="tool-chevron"
      />
      <SvgIcon name="terminal" :size="12" />
      <span
        class="tool-text"
        v-html="summaryHtml"
        @click="$emit('markdownClick', $event)"
      ></span>
      <div class="header-spacer"></div>
      <button
        v-if="toolName === 'Agent' && subagent"
        class="agent-link-btn"
        :title="t('chat.toolGroup.openSubagentInNewTab')"
        @click.stop="$emit('openSubagent', subagent.file_path, subagent.label)"
      >
        <SvgIcon name="external-link" :size="12" />
      </button>
    </div>
    <div v-if="expanded" class="tool-use-detail">
      <div
        v-if="markdownHtml !== null"
        v-mermaid
        class="markdown-body"
        v-html="markdownHtml"
        @click="$emit('markdownClick', $event)"
      ></div>
      <div v-else>
        <div
          v-if="getWidget(inputHtml)"
          class="tool-widget-preview workbuddy-svg-widget"
          v-html="ensureWorkBuddySvgStyles(getWidget(inputHtml)!.widgetCode)"
        ></div>
        <pre
          class="tool-input"
          v-html="inputHtml"
          @click="$emit('markdownClick', $event)"
        ></pre>
      </div>
    </div>
  </div>
</template>

<style scoped>
.tool-use-block {
  margin: var(--space-1) 0;
  border-radius: var(--radius-sm);
  background: var(--color-role-tool-bg);
  overflow: hidden;
}
.tool-use-header {
  -webkit-user-select: none;
  user-select: none;
  display: flex;
  align-items: center;
  gap: var(--space-1);
  padding: var(--space-1) var(--space-2);
  font-family: var(--font-mono);
  font-size: var(--text-xs);
  color: var(--color-role-tool);
  cursor: pointer;
  transition: background var(--transition-fast);
}
.tool-use-header:hover {
  background: var(--color-bg-hover);
}
.header-spacer {
  flex: 1;
}
.agent-link-btn {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 20px;
  height: 20px;
  border: none;
  background: transparent;
  color: var(--color-text-muted);
  border-radius: var(--radius-sm);
  cursor: pointer;
  transition: all var(--transition-fast);
}
.agent-link-btn:hover {
  background: var(--color-bg-tertiary);
  color: var(--color-primary);
}
.tool-chevron {
  flex-shrink: 0;
  opacity: 0.6;
}
.tool-text {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.tool-use-detail {
  border-top: 1px solid var(--color-border-light);
}
.tool-input {
  padding: var(--space-2) var(--space-3);
  font-family: var(--font-mono);
  font-size: var(--text-xs);
  line-height: 1.5;
  color: var(--color-text-secondary);
  white-space: pre-wrap;
  word-break: break-all;
  max-height: 300px;
  overflow-y: auto;
  margin: 0;
}
.tool-widget-preview {
  padding: var(--space-3);
  background: var(--color-bg);
  border-bottom: 1px solid var(--color-border-light);
  display: flex;
  justify-content: center;
  align-items: center;
  overflow-x: auto;
}
.tool-widget-preview :deep(svg) {
  max-width: 100%;
  height: auto;
  display: block;
  margin: 0 auto;
}
</style>

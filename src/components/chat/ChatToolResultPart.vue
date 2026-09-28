<script setup lang="ts">
import SvgIcon from "../icons/SvgIcon.vue";

defineProps<{
  expanded: boolean;
  isError: boolean;
  summaryHtml: string;
  contentHtml: string;
  /** 懒加载状态提示（加载中 / 失败回退预览），空串不展示 */
  hint?: string;
}>();

defineEmits<{
  toggle: [];
  markdownClick: [event: MouseEvent];
}>();
</script>

<template>
  <div
    class="tool-use-block tool-result-block"
    :class="{ 'tool-result-error': isError }"
  >
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
      <SvgIcon name="file-text" :size="12" />
      <span
        class="tool-text tool-result-summary"
        v-html="summaryHtml"
        @click="$emit('markdownClick', $event)"
      ></span>
    </div>
    <div v-if="expanded" class="tool-use-detail">
      <pre
        class="tool-input"
        v-html="contentHtml"
        @click="$emit('markdownClick', $event)"
      ></pre>
      <div v-if="hint" class="tool-result-lazy-hint">{{ hint }}</div>
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
.tool-result-block {
  opacity: 0.8;
}
.tool-result-error {
  background: color-mix(in srgb, var(--color-danger, #e74c3c) 10%, var(--color-role-tool-bg));
}
.tool-result-summary {
  opacity: 0.75;
  font-style: italic;
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
.tool-result-lazy-hint {
  padding: 0 var(--space-3) var(--space-2);
  font-size: var(--text-2xs);
  color: var(--color-text-muted);
  font-style: italic;
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
</style>

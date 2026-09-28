<script setup lang="ts">
import { computed } from "vue";
import SvgIcon from "../icons/SvgIcon.vue";
import { t } from "../../i18n";

const props = defineProps<{
  expanded: boolean;
  thinking: string;
}>();

defineEmits<{
  toggle: [];
}>();

function countWords(text: string): number {
  let count = 0;
  let inWord = false;
  const len = text.length;
  for (let i = 0; i < len; i++) {
    if (text.charCodeAt(i) > 32) {
      if (!inWord) {
        inWord = true;
        count++;
      }
    } else {
      inWord = false;
    }
  }
  return count;
}

const thinkingSummary = computed(() => {
  if (!props.thinking) return "";
  const words = countWords(props.thinking);
  if (words > 10) {
    return t("chat.thinkingPart.wordCount", { count: words });
  }
  return "";
});
</script>

<template>
  <div class="thinking-block" :class="{ 'is-expanded': expanded }">
    <div
      class="thinking-header"
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
        class="thinking-chevron"
      />
      <div class="thinking-icon-wrapper">
        <SvgIcon name="brain" :size="13" />
      </div>
      <span class="thinking-label">{{ t("chat.thinkingPart.thinkingProcess") }}</span>
      <span v-if="thinkingSummary && !expanded" class="thinking-summary-badge">{{ thinkingSummary }}</span>
      <div class="thinking-spacer" />
      <span class="thinking-hint">{{ expanded ? t('chat.thinkingPart.collapse') : t('chat.thinkingPart.expand') }}</span>
    </div>
    <div v-if="expanded" class="thinking-detail">
      <div class="thinking-text">{{ thinking }}</div>
    </div>
  </div>
</template>

<style scoped>
.thinking-block {
  margin: var(--space-2) 0;
  border-radius: var(--radius-lg);
  background: color-mix(in srgb, var(--color-bg-secondary) 65%, var(--color-bg));
  border: 1px solid var(--color-border);
  border-left: 3px solid color-mix(in srgb, var(--color-primary) 60%, transparent);
  overflow: hidden;
  transition: all var(--transition-fast);
}

.thinking-block.is-expanded {
  background: var(--color-bg-secondary);
}

.thinking-header {
  -webkit-user-select: none;
  user-select: none;
  display: flex;
  align-items: center;
  gap: var(--space-2);
  padding: 7px 12px;
  font-size: var(--text-xs);
  color: var(--color-text-secondary);
  cursor: pointer;
  transition: background var(--transition-fast);
}

.thinking-header:hover {
  background: var(--color-bg-hover);
  color: var(--color-text);
}

.thinking-chevron {
  flex-shrink: 0;
  opacity: 0.6;
  transition: transform var(--transition-fast);
}

.thinking-icon-wrapper {
  display: flex;
  align-items: center;
  color: var(--color-primary);
  opacity: 0.85;
}

.thinking-label {
  font-weight: 500;
  letter-spacing: 0.2px;
}

.thinking-summary-badge {
  font-size: 10.5px;
  font-family: var(--font-mono);
  color: var(--color-text-muted);
  background: var(--color-bg-hover);
  padding: 1px 6px;
  border-radius: var(--radius-full);
}

.thinking-spacer {
  flex: 1;
}

.thinking-hint {
  font-size: 11px;
  color: var(--color-text-muted);
  opacity: 0;
  transition: opacity var(--transition-fast);
}

.thinking-header:hover .thinking-hint {
  opacity: 0.8;
}

.thinking-detail {
  border-top: 1px solid var(--color-border-light);
  padding: var(--space-3) var(--space-4);
  max-height: 380px;
  overflow-y: auto;
}

.thinking-text {
  font-family: var(--font-sans);
  font-size: 12.5px;
  line-height: 1.65;
  color: var(--color-text-secondary);
  white-space: pre-wrap;
  word-break: break-word;
}
</style>

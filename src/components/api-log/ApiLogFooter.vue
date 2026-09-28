<script setup lang="ts">
import { t } from "../../i18n";

defineProps<{
  loading: boolean;
}>();

const emit = defineEmits<{
  clear: [days?: number];
}>();
</script>

<template>
  <div class="log-footer">
    <div class="clear-group">
      <button class="clear-btn" :disabled="loading" @click="emit('clear', 7)">{{ t('api-log.footer.clear7Days') }}</button>
      <button class="clear-btn" :disabled="loading" @click="emit('clear', 30)">{{ t('api-log.footer.clear30Days') }}</button>
      <button class="clear-btn" :disabled="loading" @click="emit('clear')">{{ t('api-log.footer.clearAll') }}</button>
    </div>
  </div>
</template>

<style scoped>
.log-footer {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: var(--space-3) var(--space-5);
  border-top: 1px solid var(--color-border);
  flex-shrink: 0;
}
.clear-group {
  display: flex;
  gap: 0;
}
.clear-btn {
  padding: 5px var(--space-3);
  font-size: var(--text-xs);
  border: 1px solid var(--color-border);
  color: var(--color-text-muted);
  cursor: pointer;
  transition: all var(--transition-fast);
}
.clear-btn:first-child {
  border-radius: var(--radius-md) 0 0 var(--radius-md);
}
.clear-btn:last-child {
  border-radius: 0 var(--radius-md) var(--radius-md) 0;
}
.clear-btn:not(:first-child) {
  border-left: none;
}
.clear-btn:hover:not(:disabled) {
  background: rgba(220, 38, 38, 0.06);
  color: var(--color-danger);
  border-color: var(--color-danger);
}
.clear-btn:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}
</style>

<script setup lang="ts">
defineProps<{
  text: string;
}>();
</script>

<template>
  <div class="proxy-banner">
    {{ text }}
  </div>
</template>

<style scoped>
.proxy-banner {
  padding: var(--space-2) var(--space-3);
  margin-bottom: var(--space-3);
  font-size: var(--text-xs);
  color: var(--color-warning-text, #92400e);
  background: var(--color-warning-bg, rgba(245, 158, 11, 0.08));
  border: 1px solid var(--color-warning-border, rgba(245, 158, 11, 0.2));
  border-radius: var(--radius-md);
}
</style>

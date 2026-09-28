<script setup lang="ts">
// 气泡式会话的时间分隔条：相邻消息间隔较长时居中的沉浸式时间胶囊
defineProps<{
  label: string;
}>();
</script>

<template>
  <div class="time-divider">
    <span class="divider-line left-line" />
    <span class="divider-label">{{ label }}</span>
    <span class="divider-line right-line" />
  </div>
</template>

<style scoped>
.time-divider {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: var(--space-3);
  margin: var(--space-5) 0 var(--space-4) 0;
  -webkit-user-select: none;
  user-select: none;
}

.divider-line {
  flex: 1;
  height: 1px;
  background: var(--color-border);
  opacity: 0.6;
}

.left-line {
  background: linear-gradient(90deg, transparent, var(--color-border));
}

.right-line {
  background: linear-gradient(90deg, var(--color-border), transparent);
}

.divider-label {
  padding: 2px 10px;
  border-radius: var(--radius-full);
  background: color-mix(in srgb, var(--color-bg-secondary) 85%, transparent);
  border: 1px solid var(--color-border);
  color: var(--color-text-muted);
  font-size: 11px;
  font-variant-numeric: tabular-nums;
  font-family: var(--font-mono);
  letter-spacing: 0.3px;
  white-space: nowrap;
  backdrop-filter: blur(6px);
  -webkit-backdrop-filter: blur(6px);
  box-shadow: 0 1px 3px rgba(0, 0, 0, 0.03);
}
</style>

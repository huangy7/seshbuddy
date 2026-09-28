<script setup lang="ts">
defineProps<{
  options: ReadonlyArray<{ value: string; label: string; hint?: string }>;
  modelValue: string;
}>();

const emit = defineEmits<{
  (e: "update:modelValue", value: string): void;
}>();

function select(value: string) {
  emit("update:modelValue", value);
}
</script>

<template>
  <div class="segmented-control">
    <button
      v-for="option in options"
      :key="option.value"
      class="segment-btn"
      :class="{ active: modelValue === option.value }"
      :aria-pressed="modelValue === option.value"
      type="button"
      @click="select(option.value)"
    >
      {{ option.label }}
    </button>
  </div>
</template>

<style scoped>
.segmented-control {
  display: inline-flex;
  background: var(--color-bg-hover);
  padding: 3px;
  border-radius: var(--radius-md);
  gap: 2px;
}
.segment-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  padding: 4px 12px;
  border: none;
  border-radius: calc(var(--radius-md) - 2px);
  background: transparent;
  color: var(--color-text-secondary);
  font-size: var(--text-xs);
  font-weight: 500;
  cursor: pointer;
  transition: all var(--transition-fast);
  white-space: nowrap;
}
.segment-btn:hover {
  color: var(--color-text);
}
.segment-btn.active {
  background: var(--color-bg);
  color: var(--color-text);
  box-shadow: var(--shadow-sm);
}
.segment-btn:focus-visible {
  outline: none;
  box-shadow: 0 0 0 2px var(--color-primary-ring), var(--shadow-sm);
}
</style>

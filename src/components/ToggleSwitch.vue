<script setup lang="ts">
const props = defineProps<{
  modelValue: boolean;
  disabled?: boolean;
  ariaLabel?: string;
}>();

const emit = defineEmits<{
  (e: "update:modelValue", value: boolean): void;
}>();

function toggle() {
  if (props.disabled) return;
  emit("update:modelValue", !props.modelValue);
}
</script>

<template>
  <button
    class="switch-control"
    type="button"
    role="switch"
    :aria-checked="modelValue"
    :aria-label="ariaLabel"
    :class="{ on: modelValue }"
    :disabled="disabled"
    @click="toggle"
  />
</template>

<style scoped>
.switch-control {
  position: relative;
  display: inline-flex;
  flex: 0 0 auto;
  width: 30px;
  height: 18px;
  margin-top: 1px;
  padding: 0;
  border: 0;
  border-radius: var(--radius-full);
  background: var(--color-border);
  cursor: pointer;
  transition: background var(--transition-fast), box-shadow var(--transition-fast);
}
.switch-control::after {
  content: "";
  position: absolute;
  top: 2px;
  left: 2px;
  width: 14px;
  height: 14px;
  border-radius: 50%;
  background: white;
  box-shadow: 0 1px 2px rgba(15, 23, 42, 0.2);
  transition: transform var(--transition-fast);
}
.switch-control.on {
  background: var(--color-primary);
}
.switch-control.on::after {
  transform: translateX(12px);
}
.switch-control:focus-visible {
  outline: none;
  box-shadow: 0 0 0 3px var(--color-primary-ring);
}
.switch-control:disabled {
  cursor: not-allowed;
  opacity: 0.55;
}
</style>

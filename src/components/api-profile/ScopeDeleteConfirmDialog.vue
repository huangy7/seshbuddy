<script setup lang="ts">
import { ref } from "vue";
import { t } from "../../i18n";

defineProps<{
  name: string;
}>();

const emit = defineEmits<{
  cancel: [];
  confirm: [cleanDisk: boolean];
}>();

const cleanDisk = ref(true);
</script>

<template>
  <div class="confirm-overlay" @click.self="emit('cancel')">
    <div class="confirm-dialog">
      <h3 class="confirm-title">{{ t("api-profile.dialog.scopeDelete.title") }}</h3>
      <p class="confirm-message">
        {{ t("api-profile.dialog.scopeDelete.message", { name }) }}
      </p>
      <label class="confirm-checkbox">
        <input v-model="cleanDisk" type="checkbox" />
        <span>{{ t("api-profile.dialog.scopeDelete.cleanDisk") }}</span>
      </label>
      <div class="confirm-actions">
        <button class="btn-secondary" @click="emit('cancel')">{{ t("dialogs.common.cancel") }}</button>
        <button class="btn-danger" @click="emit('confirm', cleanDisk)">{{ t("app.batch.delete") }}</button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.confirm-overlay {
  position: absolute;
  inset: 0;
  z-index: 10;
  background: rgba(0, 0, 0, 0.4);
  display: flex;
  align-items: center;
  justify-content: center;
  border-radius: var(--radius-xl);
}
.confirm-dialog {
  background: var(--color-bg);
  border-radius: var(--radius-lg);
  padding: var(--space-5);
  width: 360px;
  max-width: 90%;
  box-shadow: var(--shadow-lg);
}
.confirm-title {
  font-size: var(--text-base);
  font-weight: 600;
  margin: 0 0 var(--space-2);
  color: var(--color-text);
}
.confirm-message {
  font-size: var(--text-sm);
  color: var(--color-text-secondary);
  margin: 0 0 var(--space-3);
  line-height: 1.5;
}
.confirm-checkbox {
  display: flex;
  align-items: flex-start;
  gap: 6px;
  font-size: var(--text-xs);
  color: var(--color-text-secondary);
  cursor: pointer;
  margin: 0 0 var(--space-4);
  line-height: 1.5;
}
.confirm-checkbox input {
  margin: 2px 0 0;
  cursor: pointer;
  flex-shrink: 0;
}
.confirm-actions {
  display: flex;
  justify-content: flex-end;
  gap: var(--space-2);
}
.btn-secondary {
  padding: 7px var(--space-4);
  font-size: var(--text-sm);
  border: 1px solid var(--color-border);
  border-radius: var(--radius-md);
  color: var(--color-text-secondary);
  cursor: pointer;
  transition: all var(--transition-fast);
}
.btn-secondary:hover {
  background: var(--color-bg-hover);
}
.btn-danger {
  padding: 7px var(--space-4);
  font-size: var(--text-sm);
  background: var(--color-danger, #dc2626);
  color: white;
  border-radius: var(--radius-md);
  cursor: pointer;
  transition: all var(--transition-fast);
}
.btn-danger:hover {
  opacity: 0.9;
}
</style>

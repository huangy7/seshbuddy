<script setup lang="ts">
import { t } from "../../i18n";

defineProps<{
  days?: number;
  loading: boolean;
}>();

const emit = defineEmits<{
  cancel: [];
  confirm: [];
}>();
</script>

<template>
  <div
    class="confirm-overlay"
    :class="{ busy: loading }"
    @click.self="!loading && emit('cancel')"
  >
    <div class="confirm-dialog" :class="{ clearing: loading }">
      <h3 class="confirm-title">{{ t('api-log.clearConfirm.title') }}</h3>
      <p class="confirm-message">
        {{ days ? t('api-log.clearConfirm.messageDays', { days }) : t('api-log.clearConfirm.messageAll') }}
      </p>
      <p class="confirm-hint">
        {{ loading ? t('api-log.clearConfirm.hintBusy') : t('api-log.clearConfirm.hintIdle') }}
      </p>
      <div v-if="loading" class="confirm-progress">
        <span class="proxy-spinner"></span>
        <span>{{ days ? t('api-log.clearConfirm.progressDays') : t('api-log.clearConfirm.progressAll') }}</span>
      </div>
      <div class="confirm-actions">
        <button class="btn-secondary" :disabled="loading" @click="emit('cancel')">{{ t('dialogs.common.cancel') }}</button>
        <button class="btn-danger" :disabled="loading" :class="{ loading }" @click="emit('confirm')">
          <span v-if="loading" class="proxy-loading-inline">
            <span class="proxy-spinner"></span>
            {{ t('api-log.clearConfirm.confirmBusy') }}
          </span>
          <span v-else>{{ t('api-log.clearConfirm.confirm') }}</span>
        </button>
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
.confirm-overlay.busy {
  backdrop-filter: blur(2px);
}
.confirm-dialog {
  background: var(--color-bg);
  border-radius: var(--radius-lg);
  padding: var(--space-5);
  width: 340px;
  max-width: 90%;
  box-shadow: var(--shadow-lg);
  transition: transform 180ms ease, box-shadow 180ms ease;
}
.confirm-dialog.clearing {
  transform: scale(1.01);
  box-shadow: 0 20px 40px rgba(15, 23, 42, 0.18);
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
  margin: 0;
  line-height: 1.5;
}
.confirm-hint {
  margin: var(--space-2) 0 0;
  font-size: var(--text-xs);
  line-height: 1.6;
  color: var(--color-text-muted);
}
.confirm-progress {
  margin-top: var(--space-4);
  display: inline-flex;
  align-items: center;
  gap: var(--space-2);
  padding: var(--space-2) var(--space-3);
  border-radius: var(--radius-md);
  background: rgba(59, 130, 246, 0.08);
  color: var(--color-primary);
  font-size: var(--text-xs);
  font-weight: 500;
}
.confirm-actions {
  display: flex;
  justify-content: flex-end;
  gap: var(--space-2);
  margin-top: var(--space-4);
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
.btn-secondary:hover:not(:disabled) {
  background: var(--color-bg-hover);
}
.btn-secondary:disabled {
  opacity: 0.55;
  cursor: not-allowed;
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
.btn-danger.loading {
  min-width: 116px;
}
.btn-danger:hover:not(:disabled) {
  opacity: 0.9;
}
.btn-danger:disabled {
  opacity: 0.8;
  cursor: wait;
}
.proxy-loading-inline {
  display: inline-flex;
  align-items: center;
  gap: 6px;
}
.proxy-spinner {
  display: inline-block;
  width: 14px;
  height: 14px;
  border: 2px solid var(--color-border);
  border-top-color: var(--color-primary);
  border-radius: 50%;
  animation: proxy-spin 0.8s linear infinite;
}
@keyframes proxy-spin {
  to { transform: rotate(360deg); }
}
</style>

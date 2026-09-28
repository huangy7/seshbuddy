<script setup lang="ts">
import { t } from "../../i18n";

const props = defineProps<{
  active: boolean;
  loading: boolean;
  statusText: string;
  totalRecords: number;
  dbSize: number;
  error: string;
}>();

const emit = defineEmits<{
  toggle: [checked: boolean];
}>();

function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

function handleToggle(event: Event) {
  emit("toggle", (event.target as HTMLInputElement).checked);
}
</script>

<template>
  <section class="proxy-panel">
    <div class="proxy-toolbar" :class="{ running: active }">
      <div class="proxy-toolbar-main">
        <slot />
        <label class="proxy-toggle proxy-toggle-compact">
          <input type="checkbox" :checked="active" :disabled="loading" @change="handleToggle" />
          <span class="proxy-toggle-title">
            <span v-if="loading" class="proxy-loading-inline">
              <span class="proxy-spinner"></span>
              {{ t('api-log.toolbar.updatingStatus') }}
            </span>
            <span v-else>{{ t('api-log.toolbar.enableProxy') }}</span>
          </span>
        </label>

        <div class="proxy-toolbar-status">
          <span v-if="active" class="status-dot dot-ok"></span>
          <span v-else class="status-dot dot-off"></span>
          <span class="proxy-toolbar-status-text">{{ statusText }}</span>
        </div>
      </div>

      <div class="proxy-toolbar-actions">
        <span class="status-meta">{{ t('api-log.toolbar.totalRecords', { count: totalRecords }) }}</span>
        <span class="status-meta">{{ formatSize(props.dbSize) }}</span>
      </div>
    </div>

    <div v-if="error" class="proxy-inline-error">{{ error }}</div>
  </section>
</template>

<style scoped>
.proxy-panel {
  padding: var(--space-3) var(--space-5);
  border-bottom: 1px solid var(--color-border);
  background: linear-gradient(180deg, rgba(59, 130, 246, 0.04), transparent);
}
.proxy-toolbar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-4);
  padding: 10px 12px;
  border: 1px solid rgba(59, 130, 246, 0.1);
  border-radius: var(--radius-lg);
  background: rgba(255, 255, 255, 0.82);
  backdrop-filter: blur(8px);
}
.proxy-toolbar.running {
  border-color: rgba(34, 197, 94, 0.18);
  background: linear-gradient(90deg, rgba(34, 197, 94, 0.08), rgba(255, 255, 255, 0.92));
}
.proxy-toolbar-main,
.proxy-toolbar-actions {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  min-width: 0;
}
.proxy-toolbar-main {
  flex: 1;
}
.proxy-toggle {
  display: inline-flex;
  align-items: center;
  gap: var(--space-2);
  cursor: pointer;
  min-width: 0;
}
.proxy-toggle input {
  margin: 0;
  cursor: pointer;
}
.proxy-toggle-compact {
  padding: 0 var(--space-2);
  border-left: 1px solid var(--color-border);
}
.proxy-toggle-title {
  font-size: var(--text-sm);
  font-weight: 600;
  color: var(--color-text);
  white-space: nowrap;
}
.proxy-toolbar-status {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  min-width: 0;
  flex: 1;
}
.proxy-toolbar-status-text {
  font-size: var(--text-xs);
  color: var(--color-text-secondary);
  font-family: var(--font-mono);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.proxy-toolbar-actions {
  flex-shrink: 0;
}
.proxy-inline-error {
  margin-top: var(--space-3);
  padding: var(--space-2) var(--space-3);
  font-size: var(--text-xs);
  color: var(--color-danger);
  background: rgba(220, 38, 38, 0.06);
  border: 1px solid rgba(220, 38, 38, 0.16);
  border-radius: var(--radius-md);
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
.status-dot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  flex-shrink: 0;
}
.dot-ok {
  background: var(--color-success, #22c55e);
  animation: pulse 2s ease-in-out infinite;
}
.dot-off {
  background: var(--color-text-muted);
}
@keyframes pulse {
  0%, 100% { opacity: 1; }
  50% { opacity: 0.4; }
}
.status-meta {
  font-size: var(--text-xs);
  color: var(--color-text-muted);
}

[data-theme="dark"] .proxy-toolbar {
  background: rgba(0, 0, 0, 0.3);
  border-color: rgba(255, 255, 255, 0.08);
}
[data-theme="dark"] .proxy-toolbar.running {
  background: linear-gradient(90deg, rgba(34, 197, 94, 0.15), rgba(0, 0, 0, 0.4));
  border-color: rgba(34, 197, 94, 0.3);
}

@media (max-width: 900px) {
  .proxy-toolbar,
  .proxy-toolbar-main,
  .proxy-toolbar-actions {
    flex-wrap: wrap;
  }

  .proxy-toolbar {
    align-items: flex-start;
  }

  .proxy-toggle-compact {
    border-left: 0;
    padding-left: 0;
  }

  .proxy-toolbar-status-text {
    white-space: normal;
  }

  .proxy-toolbar-actions {
    width: 100%;
    gap: var(--space-2);
  }
}
</style>

<script setup lang="ts">
import { t } from "../../i18n";

interface TrafficDetail {
  id: string;
  timestamp: string;
  method: string;
  path: string;
  req_headers: string | null;
  req_body: string | null;
  req_size: number;
  status: number | null;
  res_headers: string | null;
  res_body: string | null;
  res_size: number;
  duration_ms: number;
}

defineProps<{
  loading: boolean;
  notFound: boolean;
  detail: TrafficDetail | null;
  detailTab: "request" | "response";
  detailSection: "body" | "headers";
  bodyHtml: string;
  headersHtml: string;
  formatBytes: (bytes: number) => string;
}>();

defineEmits<{
  updateDetailTab: [tab: "request" | "response"];
  updateDetailSection: [section: "body" | "headers"];
}>();
</script>

<template>
  <div class="api-detail-panel">
    <div v-if="loading" class="api-loading">{{ t("common.timelineView.loading") }}</div>
    <div v-else-if="notFound" class="api-not-found">{{ t("chat.apiDetail.notFound") }}</div>
    <template v-else-if="detail">
      <div class="api-detail-header">
        <span class="api-status" :class="detail.status && detail.status < 400 ? 'status-ok' : 'status-err'">
          {{ detail.status ?? '—' }}
        </span>
        <span class="api-meta">{{ detail.duration_ms }}ms</span>
        <span class="api-meta">{{ formatBytes(detail.req_size) }} → {{ formatBytes(detail.res_size) }}</span>
        <div class="header-spacer"></div>
        <div class="api-tabs">
          <button :class="{ active: detailTab === 'request' }" @click="$emit('updateDetailTab', 'request')">{{ t("chat.apiDetail.request") }}</button>
          <button :class="{ active: detailTab === 'response' }" @click="$emit('updateDetailTab', 'response')">{{ t("chat.apiDetail.response") }}</button>
        </div>
        <div class="api-tabs">
          <button :class="{ active: detailSection === 'body' }" @click="$emit('updateDetailSection', 'body')">{{ t("chat.apiDetail.body") }}</button>
          <button :class="{ active: detailSection === 'headers' }" @click="$emit('updateDetailSection', 'headers')">{{ t("chat.apiDetail.headers") }}</button>
        </div>
      </div>
      <pre class="api-detail-body json-highlight" v-html="detailSection === 'body' ? bodyHtml : headersHtml"></pre>
    </template>
  </div>
</template>

<style scoped>
.api-detail-panel {
  margin-top: var(--space-2);
  border: 1px solid var(--color-border-light);
  border-radius: var(--radius-sm);
  background: var(--color-bg-secondary, var(--color-bg));
  overflow: hidden;
}
.api-loading,
.api-not-found {
  padding: var(--space-3);
  font-size: var(--text-xs);
  color: var(--color-text-muted);
  text-align: center;
}
.api-detail-header {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  padding: var(--space-2) var(--space-3);
  border-bottom: 1px solid var(--color-border-light);
  font-size: var(--text-xs);
}
.api-status {
  font-weight: 600;
  font-family: var(--font-mono);
}
.api-status.status-ok {
  color: var(--color-success, #22c55e);
}
.api-status.status-err {
  color: var(--color-error, #ef4444);
}
.api-meta {
  color: var(--color-text-muted);
  font-family: var(--font-mono);
}
.header-spacer {
  flex: 1;
}
.api-tabs {
  display: flex;
  gap: 2px;
  background: var(--color-bg-hover);
  border-radius: var(--radius-sm);
  padding: 1px;
}
.api-tabs button {
  padding: 2px 8px;
  font-size: var(--text-2xs);
  border-radius: var(--radius-sm);
  color: var(--color-text-muted);
  cursor: pointer;
  transition: all var(--transition-fast);
}
.api-tabs button.active {
  background: var(--color-bg);
  color: var(--color-text);
  box-shadow: 0 1px 2px rgba(0, 0, 0, 0.1);
}
.api-detail-body {
  padding: var(--space-3);
  font-family: "JetBrains Mono", "Fira Code", ui-monospace, SFMono-Regular, "SF Mono", Menlo, Consolas, "Liberation Mono", monospace;
  font-size: 13px;
  line-height: 1.55;
  white-space: pre-wrap;
  word-break: break-all;
  max-height: 400px;
  overflow-y: auto;
  margin: 0;
  tab-size: 2;
}
.api-detail-body :deep(.json-key) {
  color: #0451a5;
}
.api-detail-body :deep(.json-str) {
  color: #a31515;
}
.api-detail-body :deep(.json-num) {
  color: #098658;
}
.api-detail-body :deep(.json-bool) {
  color: #0000ff;
}
.api-detail-body :deep(.json-null) {
  color: #0000ff;
  font-style: italic;
}
[data-theme="dark"] .api-detail-body :deep(.json-key) {
  color: #9cdcfe;
}
[data-theme="dark"] .api-detail-body :deep(.json-str) {
  color: #ce9178;
}
[data-theme="dark"] .api-detail-body :deep(.json-num) {
  color: #b5cea8;
}
[data-theme="dark"] .api-detail-body :deep(.json-bool) {
  color: #569cd6;
}
[data-theme="dark"] .api-detail-body :deep(.json-null) {
  color: #569cd6;
  font-style: italic;
}
</style>

<script setup lang="ts">
import { computed, ref } from "vue";
import { t } from "../../i18n";
import { formatNumber } from "../../utils/format";
import SvgIcon from "../icons/SvgIcon.vue";
import { reconstructSse, prettyToolInput, type ReconstructedSse } from "../../utils/sse";
import { renderMarkdown as renderMarkdownHtml } from "../../utils/markdown";

// SSE 响应的结构化视图：Markdown 正文渲染 + Thinking 思考流 + Tool 调用 + 现代 Token 统计卡片
const props = defineProps<{
  raw: string;
}>();

const parsed = computed<ReconstructedSse>(() => reconstructSse(props.raw));

const expandedTools = ref<Set<number>>(new Set());
const thinkingOpen = ref<Record<number, boolean>>({});

function toggleTool(idx: number) {
  const next = new Set(expandedTools.value);
  if (next.has(idx)) next.delete(idx);
  else next.add(idx);
  expandedTools.value = next;
}

function toggleThinking(idx: number) {
  thinkingOpen.value = { ...thinkingOpen.value, [idx]: !thinkingOpen.value[idx] };
}

/** 与聊天视图共用同一条 Markdown 管线（highlight.js 高亮 + KaTeX 公式 + DOMPurify 消毒）。
 *  此前本视图自建了一个裸 MarkdownIt 实例，导致同一段正文在 SSE 视图与聊天视图
 *  渲染出两种结果：代码块无高亮、公式不解析、且未经过消毒。 */
function renderMarkdown(text: string | undefined): string {
  if (!text) return "";
  try {
    return renderMarkdownHtml(text);
  } catch {
    return text;
  }
}
</script>

<template>
  <div class="sse-view">
    <!-- 顶部 Meta 信息栏（Model + Stop Reason） -->
    <div v-if="parsed.model || parsed.stopReason" class="sse-meta-bar">
      <div v-if="parsed.model" class="model-badge">
        <SvgIcon name="cpu" :size="12" />
        <span class="model-name">{{ parsed.model }}</span>
      </div>

      <div v-if="parsed.stopReason" class="stop-badge">
        <span class="stop-dot" />
        <span class="stop-label">stop: {{ parsed.stopReason }}</span>
      </div>
    </div>

    <!-- 响应分块列表 -->
    <div class="sse-blocks-container">
      <template v-for="(block, idx) in parsed.blocks" :key="idx">
        <!-- Text 正文（Markdown 渲染） -->
        <div
          v-if="block.type === 'text'"
          class="sse-text-card markdown-body"
          v-html="renderMarkdown(block.text)"
        />

        <!-- Thinking 思考过程（精致 Accordion） -->
        <div v-else-if="block.type === 'thinking'" class="thinking-accordion" :class="{ open: thinkingOpen[idx] }">
          <button class="thinking-header" type="button" @click="toggleThinking(idx)">
            <div class="thinking-title-wrap">
              <SvgIcon name="brain" :size="13" class="brain-icon" />
              <span class="thinking-label">{{ t('api-log.messagesView.thinkingProcess') }}</span>
              <span class="thinking-chars">{{ t('api-log.messagesView.charCount', { count: formatNumber((block.text ?? '').length) }) }}</span>
            </div>
            <SvgIcon :name="thinkingOpen[idx] ? 'chevron-down' : 'chevron-right'" :size="12" class="chevron-icon" />
          </button>
          <Transition name="expand-fade">
            <div v-if="thinkingOpen[idx]" class="thinking-content">
              <pre class="thinking-text">{{ block.text }}</pre>
            </div>
          </Transition>
        </div>

        <!-- Tool Use 工具调用卡片 -->
        <div v-else-if="block.type === 'tool_use'" class="tool-use-card" :class="{ open: expandedTools.has(idx) }">
          <button class="tool-use-header" type="button" @click="toggleTool(idx)">
            <div class="tool-badge">
              <SvgIcon name="wrench" :size="11" />
              <span>{{ t('api-log.messagesView.toolCall') }}</span>
            </div>
            <span class="tool-name">{{ block.name ?? 'tool_call' }}</span>
            <span class="spacer" />
            <SvgIcon :name="expandedTools.has(idx) ? 'chevron-down' : 'chevron-right'" :size="12" class="chevron-icon" />
          </button>
          <Transition name="expand-fade">
            <div v-if="expandedTools.has(idx)" class="tool-use-body">
              <pre class="tool-code">{{ prettyToolInput(block.inputJson) }}</pre>
            </div>
          </Transition>
        </div>
      </template>

      <div v-if="parsed.blocks.length === 0" class="sse-empty">
        <SvgIcon name="zap" :size="24" class="empty-icon" />
        <span>{{ t('api-log.sse.emptyBlocks') }}</span>
      </div>
    </div>

    <!-- 底部 Token Usage 现代仪表盘卡片 -->
    <div v-if="parsed.usage" class="token-dashboard">
      <div class="dashboard-header">
        <SvgIcon name="sparkles" :size="12" />
        <span>{{ t('api-log.sse.tokenUsageTitle') }}</span>
      </div>

      <div class="metric-grid">
        <div v-if="parsed.usage.input != null" class="metric-card input-metric">
          <div class="metric-dot" />
          <div class="metric-info">
            <span class="metric-label">{{ t('api-log.sse.inputTokens') }}</span>
            <span class="metric-value">{{ formatNumber(parsed.usage.input) }}</span>
          </div>
        </div>

        <div v-if="parsed.usage.output != null" class="metric-card output-metric">
          <div class="metric-dot" />
          <div class="metric-info">
            <span class="metric-label">{{ t('api-log.sse.outputTokens') }}</span>
            <span class="metric-value">{{ formatNumber(parsed.usage.output) }}</span>
          </div>
        </div>

        <div v-if="parsed.usage.cacheRead != null" class="metric-card cache-read-metric">
          <div class="metric-dot" />
          <div class="metric-info">
            <span class="metric-label">{{ t('api-log.sse.cacheRead') }}</span>
            <span class="metric-value">{{ formatNumber(parsed.usage.cacheRead) }}</span>
          </div>
        </div>

        <div v-if="parsed.usage.cacheCreation != null" class="metric-card cache-write-metric">
          <div class="metric-dot" />
          <div class="metric-info">
            <span class="metric-label">{{ t('api-log.sse.cacheWrite') }}</span>
            <span class="metric-value">{{ formatNumber(parsed.usage.cacheCreation) }}</span>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.sse-view {
  display: flex;
  flex-direction: column;
  gap: 12px;
  padding: 12px 16px;
  font-size: var(--text-xs);
}

/* ─── Meta Bar ─── */
.sse-meta-bar {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
}

.model-badge {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  padding: 3px 9px;
  background: var(--color-bg-sidebar);
  border: 1px solid var(--color-border);
  border-radius: var(--radius-sm, 4px);
  color: var(--color-text);
  font-family: var(--font-mono);
  font-size: 11px;
  font-weight: 600;
}

.stop-badge {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  padding: 3px 8px;
  background: color-mix(in srgb, var(--color-success, #22c55e) 8%, transparent);
  border: 1px solid color-mix(in srgb, var(--color-success, #22c55e) 25%, transparent);
  border-radius: var(--radius-sm, 4px);
  color: var(--color-success, #22c55e);
  font-family: var(--font-mono);
  font-size: 10.5px;
  font-weight: 600;
}

.stop-dot {
  width: 5px;
  height: 5px;
  border-radius: 50%;
  background: currentColor;
}

/* ─── Blocks Container ─── */
.sse-blocks-container {
  display: flex;
  flex-direction: column;
  gap: 10px;
}

/* ─── Text / Markdown ─── */
.sse-text-card {
  padding: 12px 14px;
  background: var(--color-bg);
  border: 1px solid var(--color-border);
  border-radius: var(--radius-md);
  color: var(--color-text);
  line-height: 1.65;
  font-size: 12px;
}

/* Markdown Typography */
:deep(.markdown-body) {
  font-family: inherit;
}
:deep(.markdown-body p) {
  margin-top: 0;
  margin-bottom: 8px;
}
:deep(.markdown-body p:last-child) {
  margin-bottom: 0;
}
:deep(.markdown-body h1),
:deep(.markdown-body h2),
:deep(.markdown-body h3),
:deep(.markdown-body h4) {
  margin-top: 14px;
  margin-bottom: 6px;
  font-weight: 700;
  color: var(--color-text);
  line-height: 1.3;
}
:deep(.markdown-body h2) {
  font-size: 13px;
  border-bottom: 1px solid var(--color-border);
  padding-bottom: 4px;
}
:deep(.markdown-body h3) {
  font-size: 12px;
}
:deep(.markdown-body ul),
:deep(.markdown-body ol) {
  margin: 6px 0 10px 18px;
  padding: 0;
}
:deep(.markdown-body li) {
  margin-bottom: 4px;
}
:deep(.markdown-body code) {
  font-family: var(--font-mono);
  font-size: 11px;
  padding: 1px 5px;
  border-radius: 3px;
  background: var(--color-bg-sidebar);
  border: 1px solid var(--color-border);
  color: var(--color-primary);
}
:deep(.markdown-body pre) {
  margin: 8px 0;
  padding: 8px 10px;
  border-radius: var(--radius-sm, 4px);
  background: var(--color-bg-sidebar);
  border: 1px solid var(--color-border);
  overflow-x: auto;
}
:deep(.markdown-body pre code) {
  padding: 0;
  background: transparent;
  border: none;
  color: var(--color-text);
}
:deep(.markdown-body table) {
  width: 100%;
  border-collapse: collapse;
  margin: 10px 0;
  font-size: 11.5px;
}
:deep(.markdown-body th),
:deep(.markdown-body td) {
  padding: 6px 10px;
  border: 1px solid var(--color-border);
  text-align: left;
}
:deep(.markdown-body th) {
  background: var(--color-bg-sidebar);
  font-weight: 600;
}
:deep(.markdown-body tr:nth-child(even)) {
  background: color-mix(in srgb, var(--color-bg-sidebar) 50%, transparent);
}

/* ─── Thinking Accordion ─── */
.thinking-accordion {
  border: 1px dashed var(--color-border);
  border-radius: var(--radius-md);
  background: color-mix(in srgb, var(--color-primary) 3%, var(--color-bg-sidebar));
  overflow: hidden;
  transition: all var(--transition-fast);
}
.thinking-accordion.open {
  border-color: color-mix(in srgb, var(--color-primary) 35%, var(--color-border));
  background: var(--color-bg);
}
.thinking-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  width: 100%;
  padding: 7px 12px;
  background: transparent;
  border: none;
  cursor: pointer;
  user-select: none;
}
.thinking-title-wrap {
  display: flex;
  align-items: center;
  gap: 6px;
}
.brain-icon {
  color: var(--color-primary);
}
.thinking-label {
  font-weight: 600;
  font-size: 11px;
  color: var(--color-text);
}
.thinking-chars {
  font-family: var(--font-mono);
  font-size: 10px;
  color: var(--color-text-muted);
  background: var(--color-bg);
  border: 1px solid var(--color-border);
  padding: 0 5px;
  border-radius: 999px;
  line-height: 16px;
}
.chevron-icon {
  color: var(--color-text-muted);
}
.thinking-content {
  padding: 10px 12px;
  border-top: 1px dashed var(--color-border);
  background: var(--color-bg-sidebar);
  max-height: 360px;
  overflow-y: auto;
}
.thinking-text {
  margin: 0;
  font-family: var(--font-mono);
  font-size: 11px;
  line-height: 1.6;
  color: var(--color-text-secondary);
  white-space: pre-wrap;
  word-break: break-word;
}

/* ─── Tool Use Card ─── */
.tool-use-card {
  border: 1px solid var(--color-border);
  border-radius: var(--radius-md);
  background: var(--color-bg);
  overflow: hidden;
  transition: all var(--transition-fast);
}
.tool-use-card.open {
  border-color: color-mix(in srgb, var(--color-warning, #f59e0b) 40%, var(--color-border));
}
.tool-use-header {
  display: flex;
  align-items: center;
  gap: 8px;
  width: 100%;
  padding: 7px 12px;
  background: var(--color-bg-sidebar);
  border: none;
  cursor: pointer;
}
.tool-badge {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  padding: 2px 7px;
  background: color-mix(in srgb, var(--color-warning, #f59e0b) 12%, transparent);
  border: 1px solid color-mix(in srgb, var(--color-warning, #f59e0b) 30%, transparent);
  border-radius: var(--radius-sm, 4px);
  color: var(--color-warning, #f59e0b);
  font-size: 10px;
  font-weight: 600;
}
.tool-name {
  font-family: var(--font-mono);
  font-weight: 600;
  font-size: 11px;
  color: var(--color-text);
}
.spacer {
  flex: 1;
}
.tool-use-body {
  padding: 8px 12px;
  border-top: 1px solid var(--color-border);
  max-height: 280px;
  overflow-y: auto;
}
.tool-code {
  margin: 0;
  font-family: var(--font-mono);
  font-size: 11px;
  color: var(--color-text);
  white-space: pre-wrap;
  word-break: break-all;
}

/* ─── Token Usage Dashboard ─── */
.token-dashboard {
  margin-top: 4px;
  padding: 10px 12px;
  background: var(--color-bg-sidebar);
  border: 1px solid var(--color-border);
  border-radius: var(--radius-md);
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.dashboard-header {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 11px;
  font-weight: 600;
  color: var(--color-text-secondary);
}

.metric-grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(130px, 1fr));
  gap: 6px;
}

.metric-card {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 6px 10px;
  background: var(--color-bg);
  border: 1px solid var(--color-border);
  border-radius: var(--radius-sm, 4px);
}

.metric-dot {
  width: 6px;
  height: 6px;
  border-radius: 50%;
  flex-shrink: 0;
}

.input-metric .metric-dot { background: #3b82f6; }
.output-metric .metric-dot { background: #22c55e; }
.cache-read-metric .metric-dot { background: #a855f7; }
.cache-write-metric .metric-dot { background: #f59e0b; }

.metric-info {
  display: flex;
  flex-direction: column;
  gap: 1px;
  min-width: 0;
}

.metric-label {
  font-size: 9.5px;
  color: var(--color-text-muted);
  text-transform: uppercase;
  letter-spacing: 0.04em;
}

.metric-value {
  font-family: var(--font-mono);
  font-size: 12px;
  font-weight: 700;
  color: var(--color-text);
}

.input-metric .metric-value { color: #3b82f6; }
.output-metric .metric-value { color: #22c55e; }
.cache-read-metric .metric-value { color: #a855f7; }
.cache-write-metric .metric-value { color: #f59e0b; }

.sse-empty {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 8px;
  padding: 32px;
  color: var(--color-text-muted);
}

.expand-fade-enter-active,
.expand-fade-leave-active {
  transition: all 180ms ease;
  overflow: hidden;
}
.expand-fade-enter-from,
.expand-fade-leave-to {
  opacity: 0;
  max-height: 0;
}
.expand-fade-enter-to,
.expand-fade-leave-from {
  opacity: 1;
  max-height: 600px;
}
</style>

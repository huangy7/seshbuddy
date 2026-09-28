<script setup lang="ts">
import { computed, ref, onMounted, watch, nextTick, onBeforeUnmount } from "vue";
import { t } from "../../i18n";
import { formatNumber } from "../../utils/format";
import type { TrafficDetail, TrafficSummary } from "../../composables/useProxy";
import SvgIcon from "../icons/SvgIcon.vue";
import { monaco } from "../../monaco-workers";
import { useTheme } from "../../composables/useTheme";
import {
  formatMessagesToReadableText,
  computeContextDelta,
  type ContextDeltaResult,
} from "../../utils/reqMessages";
import { copyToClipboard } from "../../utils/clipboard";

const props = withDefaults(defineProps<{
  currentDetail: TrafficDetail;
  trafficList: TrafficSummary[];
  /** 列表是否还有更老的记录未加载（此时找不到"上一轮"不代表当前是首条请求） */
  hasMore?: boolean;
  getDetailFn: (id: string) => Promise<TrafficDetail | null>;
}>(), {
  hasMore: false,
});

const emit = defineEmits<{
  copied: [text: string];
}>();

const { resolvedTheme } = useTheme();

const loading = ref(false);
const prevDetail = ref<TrafficDetail | null>(null);

// 视图模式：默认 "delta"（纯增量透视），可选 "diff"（Monaco 传统对比）
const viewMode = ref<"delta" | "diff">("delta");

// 传统 Diff 模式下的子选项
const diffMode = ref<"messages" | "raw">("messages");
const renderSideBySide = ref(true);

const container = ref<HTMLDivElement | null>(null);
let diffEditor: monaco.editor.IStandaloneDiffEditor | null = null;

// 在按时间倒序排列的列表中，找到时间早于当前项的前一条请求（即物理上的上一轮请求）
const prevSummary = computed(() => {
  const currentIdx = props.trafficList.findIndex((t) => t.id === props.currentDetail.id);
  if (currentIdx < 0 || currentIdx >= props.trafficList.length - 1) {
    return null;
  }
  return props.trafficList[currentIdx + 1];
});

// 已加载窗口外仍有更老记录但未加载：此时找不到 prevSummary 不等于"当前是首条请求"
const prevNotLoaded = computed(() => !prevSummary.value && props.hasMore);

async function loadPrevDetail() {
  if (!prevSummary.value) {
    prevDetail.value = null;
    return;
  }
  loading.value = true;
  try {
    prevDetail.value = await props.getDetailFn(prevSummary.value.id);
  } finally {
    loading.value = false;
  }
}

// 计算本轮纯新增的上下文增量
const deltaResult = computed<ContextDeltaResult>(() => {
  return computeContextDelta(prevDetail.value?.req_body, props.currentDetail.req_body);
});

function formatJson(raw: string | null | undefined): string {
  if (!raw) return "";
  try {
    return JSON.stringify(JSON.parse(raw), null, 2);
  } catch {
    return raw;
  }
}

const currentLanguage = computed(() => (diffMode.value === "messages" ? "markdown" : "json"));

const originalContent = computed(() => {
  if (!prevDetail.value) return "";
  return diffMode.value === "messages"
    ? formatMessagesToReadableText(prevDetail.value.req_body)
    : formatJson(prevDetail.value.req_body);
});

const modifiedContent = computed(() => {
  return diffMode.value === "messages"
    ? formatMessagesToReadableText(props.currentDetail.req_body)
    : formatJson(props.currentDetail.req_body);
});

function getTheme(dark: boolean) {
  return dark ? "vs-dark" : "vs";
}

function updateDiffModels() {
  if (!diffEditor) return;
  const lang = currentLanguage.value;
  const currentModel = diffEditor.getModel();

  if (currentModel?.original && currentModel?.modified) {
    if (currentModel.original.getValue() !== originalContent.value) {
      currentModel.original.setValue(originalContent.value);
    }
    if (currentModel.modified.getValue() !== modifiedContent.value) {
      currentModel.modified.setValue(modifiedContent.value);
    }
    monaco.editor.setModelLanguage(currentModel.original, lang);
    monaco.editor.setModelLanguage(currentModel.modified, lang);
  } else {
    const originalModel = monaco.editor.createModel(originalContent.value, lang);
    const modifiedModel = monaco.editor.createModel(modifiedContent.value, lang);
    diffEditor.setModel({ original: originalModel, modified: modifiedModel });
  }
}

function initEditor() {
  if (viewMode.value !== "diff" || !container.value) return;
  if (diffEditor) {
    updateDiffModels();
    return;
  }

  const dark = resolvedTheme.value === "dark";
  monaco.editor.setTheme(getTheme(dark));

  diffEditor = monaco.editor.createDiffEditor(container.value, {
    automaticLayout: true,
    readOnly: true,
    renderSideBySide: renderSideBySide.value,
    useInlineViewWhenSpaceIsLimited: false,
    minimap: { enabled: false },
    scrollBeyondLastLine: false,
    fontSize: 12,
    wordWrap: "off",
  });

  updateDiffModels();
}

function setSideBySide(val: boolean) {
  if (renderSideBySide.value === val) return;
  renderSideBySide.value = val;
  if (diffEditor) {
    diffEditor.updateOptions({ renderSideBySide: val });
    diffEditor.layout();
  }
}

const copiedKey = ref<string | null>(null);

async function copyText(text: string, key: string, label: string) {
  if (await copyToClipboard(text)) {
    copiedKey.value = key;
    emit("copied", label);
    setTimeout(() => {
      if (copiedKey.value === key) copiedKey.value = null;
    }, 1500);
  }
}

onMounted(async () => {
  await loadPrevDetail();
  if (viewMode.value === "diff") {
    await nextTick();
    initEditor();
  }
});

watch(() => props.currentDetail.id, async () => {
  await loadPrevDetail();
  if (viewMode.value === "diff") {
    await nextTick();
    updateDiffModels();
  }
});

watch(viewMode, async (mode) => {
  if (mode === "diff") {
    await nextTick();
    initEditor();
  }
});

watch([diffMode, originalContent, modifiedContent], () => {
  if (viewMode.value === "diff") {
    updateDiffModels();
  }
});

watch(resolvedTheme, (newTheme) => {
  monaco.editor.setTheme(getTheme(newTheme === "dark"));
});

onBeforeUnmount(() => {
  const model = diffEditor?.getModel();
  model?.original?.dispose();
  model?.modified?.dispose();
  diffEditor?.dispose();
  diffEditor = null;
});
</script>

<template>
  <div class="diff-view-root">
    <!-- 极简精炼 Diff 工具栏 -->
    <div class="diff-toolbar">
      <!-- 一级模式选择：⚡ 新增增量 (Delta) vs 🔀 完整 Diff -->
      <div class="diff-mode-segmented">
        <button
          class="seg-btn"
          :class="{ active: viewMode === 'delta' }"
          type="button"
          @click="viewMode = 'delta'"
        >
          <SvgIcon name="sparkles" :size="11" />
          <span>{{ t('api-log.diff.deltaMode') }}</span>
          <span v-if="deltaResult.addedMessages.length > 0" class="mini-count-badge">
            +{{ deltaResult.addedMessages.length }}
          </span>
        </button>
        <button
          class="seg-btn"
          :class="{ active: viewMode === 'diff' }"
          type="button"
          @click="viewMode = 'diff'"
        >
          <SvgIcon name="git-compare" :size="11" />
          <span>{{ t('api-log.diff.fullDiffMode') }}</span>
        </button>
      </div>

      <div class="toolbar-spacer" />

      <!-- 传统 Diff 模式下的子控制项 -->
      <template v-if="viewMode === 'diff' && prevSummary">
        <div class="diff-mode-segmented">
          <button
            class="seg-btn"
            :class="{ active: diffMode === 'messages' }"
            type="button"
            @click="diffMode = 'messages'"
          >
            <span>{{ t('api-log.diff.messagesMode') }}</span>
          </button>
          <button
            class="seg-btn"
            :class="{ active: diffMode === 'raw' }"
            type="button"
            @click="diffMode = 'raw'"
          >
            <span>{{ t('api-log.requestDetail.rawJson') }}</span>
          </button>
        </div>

        <div class="diff-layout-segmented">
          <button
            class="seg-btn"
            :class="{ active: renderSideBySide }"
            type="button"
            :title="t('api-log.diff.sideBySideTitle')"
            @click="setSideBySide(true)"
          >
            <SvgIcon name="columns" :size="11" />
            <span>{{ t('api-log.diff.sideBySide') }}</span>
          </button>
          <button
            class="seg-btn"
            :class="{ active: !renderSideBySide }"
            type="button"
            :title="t('api-log.diff.inlineTitle')"
            @click="setSideBySide(false)"
          >
            <SvgIcon name="align-left" :size="11" />
            <span>{{ t('api-log.diff.inline') }}</span>
          </button>
        </div>
      </template>
    </div>

    <!-- 视图 1：纯增量透视（Delta View） -->
    <div v-if="viewMode === 'delta'" class="delta-view-wrap">
      <div v-if="loading" class="delta-loading">
        <span class="mini-spinner" />
        <span>{{ t('api-log.diff.loadingDelta') }}</span>
      </div>

      <div v-else class="delta-container">
        <!-- 顶部 Summary 统计卡片 -->
        <div class="delta-summary-bar">
          <div class="summary-left">
            <span class="summary-icon">
              <SvgIcon :name="deltaResult.isFirstRequest ? 'star' : 'sparkles'" :size="13" />
            </span>
            <span class="summary-title">
              {{ deltaResult.isFirstRequest
                ? (prevNotLoaded ? t('api-log.diff.prevNotLoaded') : t('api-log.diff.firstContext'))
                : t('api-log.diff.addedContext', { count: deltaResult.addedMessages.length }) }}
            </span>
            <span class="summary-badge">
              +{{ t('api-log.messagesView.charCount', { count: formatNumber(deltaResult.totalCharsAdded) }) }}
            </span>
          </div>
          <span class="summary-context-tip">
            {{ t('api-log.diff.totalRounds', { count: deltaResult.currentMessagesCount }) }}
          </span>
        </div>

        <!-- System Prompt 发生变更的专属提示 -->
        <div v-if="deltaResult.systemPromptChanged" class="delta-card system-prompt-alert">
          <div class="delta-card-head system-alert-head">
            <SvgIcon name="alert-circle" :size="13" />
            <span class="head-title">{{ t('api-log.diff.systemPromptChanged') }}</span>
            <span class="spacer" />
            <button
              v-if="deltaResult.newSystemPrompt"
              class="mini-copy-btn"
              type="button"
              @click="copyText(deltaResult.newSystemPrompt, 'delta-sys', t('api-log.diff.copiedSystemPrompt'))"
            >
              <SvgIcon :name="copiedKey === 'delta-sys' ? 'check' : 'copy'" :size="11" />
            </button>
          </div>
          <div class="delta-card-body">
            <div class="prompt-diff-preview">
              <pre class="prompt-code">{{ deltaResult.newSystemPrompt }}</pre>
            </div>
          </div>
        </div>

        <!-- 本轮新增的消息列表 -->
        <div v-if="deltaResult.addedMessages.length > 0" class="delta-stream">
          <div
            v-for="(msg, mIdx) in deltaResult.addedMessages"
            :key="msg.id"
            class="delta-msg-card"
            :class="`role-${msg.role}`"
          >
            <div class="delta-card-head">
              <div class="role-badge" :class="`badge-${msg.role}`">
                <SvgIcon v-if="msg.role === 'user'" name="user" :size="11" />
                <SvgIcon v-else-if="msg.role === 'assistant'" name="sparkles" :size="11" />
                <SvgIcon v-else-if="msg.role === 'tool'" name="wrench" :size="11" />
                <SvgIcon v-else name="terminal" :size="11" />
                <span>{{ msg.role.toUpperCase() }}</span>
              </div>
              <span class="added-index-tag">{{ t('api-log.diff.roundAdded', { round: deltaResult.prevMessagesCount + mIdx + 1 }) }}</span>
              <span v-if="msg.name" class="sender-name">({{ msg.name }})</span>

              <span class="spacer" />

              <button
                v-if="msg.rawContent"
                class="mini-copy-btn"
                type="button"
                :title="t('common.copyButton.copy')"
                @click="copyText(msg.rawContent, `delta-${msg.id}`, t('api-log.diff.copiedMessage'))"
              >
                <SvgIcon :name="copiedKey === `delta-${msg.id}` ? 'check' : 'copy'" :size="11" />
              </button>
            </div>

            <div class="delta-card-body">
              <template v-for="(block, bIdx) in msg.contentBlocks" :key="bIdx">
                <!-- Text Block -->
                <div v-if="block.type === 'text'" class="delta-text-content">
                  {{ block.text }}
                </div>

                <!-- Thinking Block -->
                <div v-else-if="block.type === 'thinking'" class="delta-thinking-block">
                  <div class="thinking-title">
                    <SvgIcon name="zap" :size="11" />
                    <span>{{ t('api-log.messagesView.thinkingProcess') }}</span>
                  </div>
                  <pre class="thinking-code">{{ block.text }}</pre>
                </div>

                <!-- Tool Use Block -->
                <div v-else-if="block.type === 'tool_use'" class="delta-tool-use-block">
                  <div class="tool-call-label">
                    <SvgIcon name="wrench" :size="11" />
                    <span>{{ t('api-log.messagesView.toolCall') }}: <strong>{{ block.toolName }}</strong></span>
                  </div>
                  <pre class="tool-param-code">{{ typeof block.toolInput === 'object' ? JSON.stringify(block.toolInput, null, 2) : block.toolInput }}</pre>
                </div>

                <!-- Tool Result Block -->
                <div v-else-if="block.type === 'tool_result'" class="delta-tool-result-block" :class="{ error: block.isError }">
                  <div class="tool-result-label">
                    <SvgIcon :name="block.isError ? 'alert-triangle' : 'check'" :size="11" />
                    <span>{{ t('api-log.messagesView.toolResult') }} {{ block.isError ? t('api-log.diff.toolResultError') : '' }}</span>
                  </div>
                  <pre class="tool-output-code">{{ block.toolOutput }}</pre>
                </div>
              </template>
            </div>
          </div>
        </div>

        <!-- 本轮没有新增任何 Message 的空状态 -->
        <div v-else-if="!deltaResult.systemPromptChanged" class="delta-none-card">
          <SvgIcon name="check" :size="24" class="check-icon" />
          <div class="none-title">{{ t('api-log.diff.noneTitle') }}</div>
          <div class="none-desc">{{ t('api-log.diff.noneDesc') }}</div>
        </div>
      </div>
    </div>

    <!-- 视图 2：传统 Monaco Diff 对比容器 -->
    <div v-show="viewMode === 'diff'" class="diff-container-wrap">
      <div v-if="loading" class="diff-loading">
        <span class="mini-spinner" />
        <span>{{ t('api-log.diff.loadingPrev') }}</span>
      </div>
      <div v-else-if="!prevSummary" class="diff-first-tip">
        <SvgIcon name="git-compare" :size="32" class="tip-icon" />
        <div class="tip-title">{{ t('api-log.diff.firstTitle') }}</div>
        <div class="tip-desc">{{ t('api-log.diff.firstDesc') }}</div>
      </div>
      <div ref="container" class="monaco-diff-container" />
    </div>
  </div>
</template>

<style scoped>
.diff-view-root {
  display: flex;
  flex-direction: column;
  height: 100%;
  width: 100%;
  min-height: 0;
  background: var(--color-bg);
  overflow: hidden;
}

.diff-toolbar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  padding: 6px 12px;
  background: var(--color-bg-sidebar);
  border-bottom: 1px solid var(--color-border);
  flex-shrink: 0;
}

.diff-mode-segmented,
.diff-layout-segmented {
  display: inline-flex;
  padding: 2px;
  gap: 2px;
  background: var(--color-bg);
  border: 1px solid var(--color-border);
  border-radius: var(--radius-sm, 4px);
  flex-shrink: 0;
}

.seg-btn {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  font-size: 11px;
  padding: 2px 9px;
  border: none;
  background: transparent;
  color: var(--color-text-muted);
  border-radius: 3px;
  cursor: pointer;
  transition: all var(--transition-fast);
  white-space: nowrap;
  flex-shrink: 0;
}

.seg-btn:hover {
  color: var(--color-text);
}

.seg-btn.active {
  background: var(--color-bg-sidebar);
  color: var(--color-primary);
  font-weight: 600;
  box-shadow: 0 1px 2px rgba(0, 0, 0, 0.05);
}

.mini-count-badge {
  font-family: var(--font-mono);
  font-size: 9.5px;
  background: color-mix(in srgb, var(--color-primary) 15%, transparent);
  color: var(--color-primary);
  padding: 0 4px;
  border-radius: 999px;
  line-height: 14px;
}

.toolbar-spacer {
  flex: 1;
}

/* ─── Delta View 容器 ─── */
.delta-view-wrap {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  padding: 12px 16px;
  background: var(--color-bg);
}

.delta-container {
  display: flex;
  flex-direction: column;
  gap: 10px;
  max-width: 860px;
  margin: 0 auto;
}

.delta-summary-bar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  padding: 8px 12px;
  background: color-mix(in srgb, var(--color-primary) 6%, var(--color-bg-sidebar));
  border: 1px solid color-mix(in srgb, var(--color-primary) 20%, var(--color-border));
  border-radius: var(--radius-md);
}

.summary-left {
  display: flex;
  align-items: center;
  gap: 8px;
}

.summary-icon {
  color: var(--color-primary);
  display: inline-flex;
}

.summary-title {
  font-weight: 600;
  font-size: 11.5px;
  color: var(--color-text);
}

.summary-badge {
  font-family: var(--font-mono);
  font-size: 10px;
  font-weight: 600;
  color: var(--color-primary);
  background: color-mix(in srgb, var(--color-primary) 12%, transparent);
  padding: 1px 6px;
  border-radius: 999px;
}

.summary-context-tip {
  font-size: 10.5px;
  color: var(--color-text-muted);
}

/* ─── Delta Stream Cards ─── */
.delta-stream {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.delta-msg-card,
.delta-card {
  border: 1px solid var(--color-border);
  border-radius: var(--radius-md);
  background: var(--color-bg);
  overflow: hidden;
  box-shadow: 0 1px 3px rgba(0, 0, 0, 0.02);
}

.delta-card-head {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 6px 12px;
  background: var(--color-bg-sidebar);
  border-bottom: 1px solid var(--color-border);
}

.role-badge {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  font-size: 10px;
  font-weight: 700;
  font-family: var(--font-mono);
  padding: 2px 7px;
  border-radius: var(--radius-sm, 4px);
}

.badge-user {
  background: color-mix(in srgb, var(--color-primary) 15%, transparent);
  color: var(--color-primary);
}
.badge-assistant {
  background: color-mix(in srgb, var(--color-success, #22c55e) 15%, transparent);
  color: var(--color-success, #22c55e);
}
.badge-tool {
  background: color-mix(in srgb, var(--color-warning, #f59e0b) 15%, transparent);
  color: var(--color-warning, #f59e0b);
}
.badge-system {
  background: color-mix(in srgb, var(--color-text-muted) 15%, transparent);
  color: var(--color-text-muted);
}

.added-index-tag {
  font-family: var(--font-mono);
  font-size: 10.5px;
  color: var(--color-text-muted);
}

.sender-name {
  font-size: 10.5px;
  color: var(--color-text-muted);
}

.spacer {
  flex: 1;
}

.mini-copy-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 20px;
  height: 20px;
  background: transparent;
  border: none;
  color: var(--color-text-muted);
  border-radius: var(--radius-sm, 4px);
  cursor: pointer;
}
.mini-copy-btn:hover {
  color: var(--color-text);
  background: var(--color-bg-hover);
}

.delta-card-body {
  padding: 10px 12px;
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.delta-text-content {
  font-size: 12px;
  line-height: 1.65;
  color: var(--color-text);
  white-space: pre-wrap;
  word-break: break-word;
}

.delta-thinking-block {
  border: 1px dashed var(--color-border);
  border-radius: var(--radius-sm, 4px);
  background: var(--color-bg-sidebar);
  padding: 8px 10px;
}

.thinking-title {
  display: flex;
  align-items: center;
  gap: 5px;
  font-size: 10.5px;
  color: var(--color-text-muted);
  margin-bottom: 4px;
}

.thinking-code {
  margin: 0;
  font-family: var(--font-mono);
  font-size: 11px;
  color: var(--color-text-secondary);
  white-space: pre-wrap;
  line-height: 1.5;
}

.delta-tool-use-block,
.delta-tool-result-block {
  border: 1px solid var(--color-border);
  border-radius: var(--radius-sm, 4px);
  padding: 8px 10px;
  background: var(--color-bg-sidebar);
}

.tool-call-label,
.tool-result-label {
  display: flex;
  align-items: center;
  gap: 5px;
  font-size: 10.5px;
  font-weight: 600;
  color: var(--color-warning, #f59e0b);
  margin-bottom: 4px;
}

.tool-result-label {
  color: var(--color-success, #22c55e);
}
.delta-tool-result-block.error .tool-result-label {
  color: var(--color-danger, #ef4444);
}

.tool-param-code,
.tool-output-code {
  margin: 0;
  font-family: var(--font-mono);
  font-size: 11px;
  color: var(--color-text);
  white-space: pre-wrap;
  word-break: break-all;
  max-height: 280px;
  overflow-y: auto;
}

/* System Prompt Alert */
.system-prompt-alert {
  border-color: color-mix(in srgb, var(--color-warning, #f59e0b) 40%, var(--color-border));
}
.system-alert-head {
  background: color-mix(in srgb, var(--color-warning, #f59e0b) 8%, var(--color-bg-sidebar));
  color: var(--color-warning, #f59e0b);
  font-weight: 600;
  font-size: 11px;
}
.prompt-code {
  margin: 0;
  font-family: var(--font-mono);
  font-size: 11px;
  color: var(--color-text);
  white-space: pre-wrap;
  max-height: 240px;
  overflow-y: auto;
}

/* None State */
.delta-none-card {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  padding: 32px 16px;
  background: var(--color-bg-sidebar);
  border: 1px dashed var(--color-border);
  border-radius: var(--radius-md);
  text-align: center;
  gap: 8px;
}
.check-icon {
  color: var(--color-success, #22c55e);
}
.none-title {
  font-size: 12.5px;
  font-weight: 600;
  color: var(--color-text);
}
.none-desc {
  font-size: 11px;
  color: var(--color-text-muted);
}

.delta-loading {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 8px;
  padding: 40px;
  font-size: 11.5px;
  color: var(--color-text-muted);
}

/* ─── Monaco Diff 容器 ─── */
.diff-container-wrap {
  flex: 1;
  min-height: 0;
  width: 100%;
  position: relative;
  overflow: hidden;
}

.monaco-diff-container {
  width: 100%;
  height: 100%;
  min-height: 380px;
}

.diff-loading {
  position: absolute;
  inset: 0;
  z-index: 10;
  background: var(--color-bg);
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 8px;
  font-size: 11.5px;
  color: var(--color-text-muted);
}

.mini-spinner {
  width: 14px;
  height: 14px;
  border: 2px solid var(--color-border);
  border-top-color: var(--color-primary);
  border-radius: 50%;
  animation: spin 0.8s linear infinite;
}

@keyframes spin {
  to { transform: rotate(360deg); }
}

.diff-first-tip {
  position: absolute;
  inset: 0;
  z-index: 5;
  background: var(--color-bg);
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 10px;
  padding: 32px;
  text-align: center;
}

.tip-icon {
  opacity: 0.4;
  color: var(--color-primary);
}

.tip-title {
  font-size: 13.5px;
  font-weight: 600;
  color: var(--color-text);
}

.tip-desc {
  font-size: 11.5px;
  color: var(--color-text-muted);
  max-width: 420px;
  line-height: 1.6;
}
</style>

<script setup lang="ts">
import { computed, ref } from "vue";
import { t } from "../../i18n";
import { formatNumber } from "../../utils/format";
import SvgIcon from "../icons/SvgIcon.vue";
import { parseRequestMessages, type ParsedMessageItem } from "../../utils/reqMessages";
import { copyToClipboard } from "../../utils/clipboard";

const props = defineProps<{
  reqBody: string | null;
}>();

const emit = defineEmits<{
  copied: [text: string];
}>();

const parsed = computed(() => parseRequestMessages(props.reqBody));

// 折叠状态控制：默认全部收起
const systemExpanded = ref(false);
const expandedMessageIds = ref<Set<string>>(new Set());
const collapsedBlocks = ref<Record<string, boolean>>({});

function toggleSystem() {
  systemExpanded.value = !systemExpanded.value;
}

function toggleMessage(id: string) {
  const next = new Set(expandedMessageIds.value);
  if (next.has(id)) {
    next.delete(id);
  } else {
    next.add(id);
  }
  expandedMessageIds.value = next;
}

function toggleAll(expand: boolean) {
  if (expand) {
    expandedMessageIds.value = new Set(parsed.value.messages.map((m) => m.id));
    systemExpanded.value = true;
  } else {
    expandedMessageIds.value = new Set();
    systemExpanded.value = false;
  }
}

function toggleBlock(key: string) {
  collapsedBlocks.value = {
    ...collapsedBlocks.value,
    [key]: !collapsedBlocks.value[key],
  };
}

function prettyJson(val: unknown): string {
  if (!val) return "";
  if (typeof val === "string") {
    if (val.length > 100_000) return val;
    try {
      return JSON.stringify(JSON.parse(val), null, 2);
    } catch {
      return val;
    }
  }
  try {
    return JSON.stringify(val, null, 2);
  } catch {
    return String(val);
  }
}

function getMessagePreview(msg: ParsedMessageItem): string {
  if (msg.rawContent) {
    const text = msg.rawContent.trim().replace(/\s+/g, " ");
    return text.length > 90 ? text.slice(0, 90) + "..." : text;
  }
  for (const block of msg.contentBlocks) {
    if (block.type === "text" && block.text) {
      const text = block.text.trim().replace(/\s+/g, " ");
      return text.length > 90 ? text.slice(0, 90) + "..." : text;
    }
    if (block.type === "tool_use" && block.toolName) {
      return t("api-log.messagesView.toolCallPreview", { name: block.toolName });
    }
    if (block.type === "tool_result") {
      return t("api-log.messagesView.toolResultPreview", {
        output: block.toolOutput ? block.toolOutput.slice(0, 40) : "...",
      });
    }
    if (block.type === "thinking" && block.text) {
      return t("api-log.messagesView.thinkingPreview", { text: block.text.slice(0, 40) });
    }
  }
  return "";
}

function getMessageToolCount(msg: ParsedMessageItem): number {
  return msg.contentBlocks.filter((b) => b.type === "tool_use" || b.type === "tool_result").length;
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
</script>

<template>
  <div class="messages-flow">
    <!-- Meta Summary Bar (Model, Temperature, Max Tokens & 一键展开/收起) -->
    <div class="flow-meta-bar">
      <div v-if="parsed.model" class="meta-pill model-pill">
        <SvgIcon name="cpu" :size="12" />
        <span>{{ parsed.model }}</span>
      </div>
      <div v-if="parsed.temperature != null" class="meta-pill">
        <span>temp: {{ parsed.temperature }}</span>
      </div>
      <div v-if="parsed.maxTokens != null" class="meta-pill">
        <span>max_tokens: {{ parsed.maxTokens }}</span>
      </div>

      <div class="meta-spacer" />

      <span class="meta-count">{{
        t('api-log.messagesView.roundCount', { count: parsed.messages.length })
      }}</span>

      <div class="expand-all-actions">
        <button
          class="meta-action-btn"
          type="button"
          @click="toggleAll(expandedMessageIds.size < parsed.messages.length)"
        >
          {{ expandedMessageIds.size === parsed.messages.length
            ? t('api-log.messagesView.collapseAll')
            : t('api-log.messagesView.expandAll') }}
        </button>
      </div>
    </div>

    <!-- System Prompt Card (默认折叠) -->
    <div v-if="parsed.systemPrompt" class="system-card" :class="{ open: systemExpanded }">
      <div class="system-header" @click="toggleSystem">
        <span class="system-badge">
          <SvgIcon name="terminal" :size="12" />
          SYSTEM PROMPT
        </span>
        <span class="system-len">{{
          t('api-log.messagesView.charCount', { count: formatNumber(parsed.systemPrompt.length) })
        }}</span>
        <span v-if="!systemExpanded" class="system-preview-text">
          {{ parsed.systemPrompt.slice(0, 60).replace(/\s+/g, ' ') }}...
        </span>
        <span class="card-spacer" />
        <button
          class="card-action-btn"
          type="button"
          :title="t('api-log.messagesView.copySystemPromptTitle')"
          @click.stop="copyText(parsed.systemPrompt ?? '', 'sys', t('api-log.messagesView.copiedSystemPrompt'))"
        >
          <SvgIcon :name="copiedKey === 'sys' ? 'check' : 'copy'" :size="12" />
        </button>
        <SvgIcon :name="systemExpanded ? 'chevron-down' : 'chevron-right'" :size="12" class="collapse-icon" />
      </div>
      <Transition name="flow-collapse">
        <div v-if="systemExpanded" class="system-body">
          <pre class="prompt-text">{{ parsed.systemPrompt }}</pre>
        </div>
      </Transition>
    </div>

    <!-- Messages List (默认全部收起，点击才展开) -->
    <div v-if="parsed.messages.length > 0" class="dialog-stream">
      <div
        v-for="(msg, msgIdx) in parsed.messages"
        :key="msg.id"
        class="msg-row"
        :class="[`role-${msg.role}`, { expanded: expandedMessageIds.has(msg.id) }]"
      >
        <div class="msg-avatar" :class="`avatar-${msg.role}`">
          <SvgIcon v-if="msg.role === 'user'" name="user" :size="13" />
          <SvgIcon v-else-if="msg.role === 'assistant'" name="sparkles" :size="13" />
          <SvgIcon v-else-if="msg.role === 'tool'" name="wrench" :size="13" />
          <SvgIcon v-else name="terminal" :size="13" />
        </div>

        <div class="msg-bubble-wrap" :class="{ open: expandedMessageIds.has(msg.id) }">
          <div class="bubble-header" @click="toggleMessage(msg.id)">
            <span class="role-name">{{ msg.role.toUpperCase() }}</span>
            <span v-if="msg.name" class="sender-name">({{ msg.name }})</span>
            <span class="msg-index">#{{ msgIdx + 1 }}</span>

            <!-- 收起时展示的简短预览 -->
            <span v-if="!expandedMessageIds.has(msg.id)" class="collapsed-preview" :title="getMessagePreview(msg)">
              {{ getMessagePreview(msg) }}
            </span>

            <span class="card-spacer" />

            <span v-if="getMessageToolCount(msg) > 0" class="mini-tool-tag">
              <SvgIcon name="wrench" :size="10" />
              {{ t('api-log.messagesView.toolCount', { count: getMessageToolCount(msg) }) }}
            </span>

            <button
              v-if="msg.rawContent"
              class="card-action-btn"
              type="button"
              :title="t('api-log.messagesView.copyMessageTitle')"
              @click.stop="copyText(msg.rawContent ?? '', msg.id, t('api-log.messagesView.copiedMessage', { index: msgIdx + 1 }))"
            >
              <SvgIcon :name="copiedKey === msg.id ? 'check' : 'copy'" :size="11" />
            </button>

            <SvgIcon :name="expandedMessageIds.has(msg.id) ? 'chevron-down' : 'chevron-right'" :size="12" class="collapse-icon" />
          </div>

          <Transition name="flow-collapse">
            <div v-if="expandedMessageIds.has(msg.id)" class="bubble-content">
              <template v-for="(block, bIdx) in msg.contentBlocks" :key="bIdx">
                <!-- Text Block -->
                <div v-if="block.type === 'text'" class="content-text">
                  {{ block.text }}
                </div>

                <!-- Thinking Block -->
                <div v-else-if="block.type === 'thinking'" class="thinking-block">
                  <div class="thinking-header" @click="toggleBlock(`${msg.id}-th-${bIdx}`)">
                    <SvgIcon name="zap" :size="11" />
                    <span>{{ t('api-log.messagesView.thinkingProcess') }}</span>
                    <SvgIcon :name="collapsedBlocks[`${msg.id}-th-${bIdx}`] ? 'chevron-right' : 'chevron-down'" :size="10" />
                  </div>
                  <div v-if="!collapsedBlocks[`${msg.id}-th-${bIdx}`]" class="thinking-body">
                    {{ block.text }}
                  </div>
                </div>

                <!-- Tool Use Block -->
                <div v-else-if="block.type === 'tool_use'" class="tool-call-block">
                  <div class="tool-call-header" @click="toggleBlock(`${msg.id}-tc-${bIdx}`)">
                    <div class="tool-badge">
                      <SvgIcon name="wrench" :size="11" />
                      <span>{{ t('api-log.messagesView.toolCall') }}</span>
                    </div>
                    <span class="tool-fn-name">{{ block.toolName }}</span>
                    <span v-if="block.toolCallId" class="tool-call-id">{{ block.toolCallId.slice(-8) }}</span>
                    <span class="card-spacer" />
                    <button
                      v-if="block.toolInput"
                      class="card-action-btn"
                      type="button"
                      :title="t('api-log.messagesView.copyArgsTitle')"
                      @click.stop="copyText(prettyJson(block.toolInput), `${msg.id}-tc-${bIdx}`, t('api-log.messagesView.copiedToolArgs'))"
                    >
                      <SvgIcon :name="copiedKey === `${msg.id}-tc-${bIdx}` ? 'check' : 'copy'" :size="10" />
                    </button>
                    <SvgIcon :name="collapsedBlocks[`${msg.id}-tc-${bIdx}`] ? 'chevron-right' : 'chevron-down'" :size="11" class="collapse-icon" />
                  </div>
                  <div v-if="!collapsedBlocks[`${msg.id}-tc-${bIdx}`]" class="tool-call-body">
                    <pre class="code-block">{{ prettyJson(block.toolInput) }}</pre>
                  </div>
                </div>

                <!-- Tool Result Block -->
                <div v-else-if="block.type === 'tool_result'" class="tool-result-block" :class="{ error: block.isError }">
                  <div class="tool-result-header" @click="toggleBlock(`${msg.id}-tr-${bIdx}`)">
                    <div class="result-badge" :class="{ error: block.isError }">
                      <SvgIcon :name="block.isError ? 'alert-triangle' : 'check'" :size="11" />
                      <span>{{
                        block.isError
                          ? t('api-log.messagesView.toolError')
                          : t('api-log.messagesView.toolResult')
                      }}</span>
                    </div>
                    <span v-if="block.toolCallId" class="tool-call-id">{{ block.toolCallId.slice(-8) }}</span>
                    <span class="card-spacer" />
                    <button
                      v-if="block.toolOutput"
                      class="card-action-btn"
                      type="button"
                      :title="t('api-log.messagesView.copyResultTitle')"
                      @click.stop="copyText(block.toolOutput ?? '', `${msg.id}-tr-${bIdx}`, t('api-log.messagesView.copiedResult'))"
                    >
                      <SvgIcon :name="copiedKey === `${msg.id}-tr-${bIdx}` ? 'check' : 'copy'" :size="10" />
                    </button>
                    <SvgIcon :name="collapsedBlocks[`${msg.id}-tr-${bIdx}`] ? 'chevron-right' : 'chevron-down'" :size="11" class="collapse-icon" />
                  </div>
                  <div v-if="!collapsedBlocks[`${msg.id}-tr-${bIdx}`]" class="tool-result-body">
                    <pre class="code-block">{{ block.toolOutput }}</pre>
                  </div>
                </div>

                <!-- Fallback unknown -->
                <div v-else class="content-unknown">
                  <pre>{{ block.text }}</pre>
                </div>
              </template>
            </div>
          </Transition>
        </div>
      </div>
    </div>

    <!-- Empty State -->
    <div v-else-if="!parsed.systemPrompt" class="messages-empty">
      <SvgIcon name="message-square" :size="24" class="empty-icon" />
      <div class="empty-title">{{ t('api-log.messagesView.emptyTitle') }}</div>
      <div class="empty-desc">{{ t('api-log.messagesView.emptyDesc') }}</div>
    </div>
  </div>
</template>

<style scoped>
.messages-flow {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
  padding: 10px 14px;
  font-size: var(--text-xs);
}

/* ─── Meta Bar ─── */
.flow-meta-bar {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  padding: 5px 10px;
  background: var(--color-bg-sidebar);
  border: 1px solid var(--color-border);
  border-radius: var(--radius-sm, 4px);
  font-family: var(--font-mono);
  font-size: 11px;
}
.meta-pill {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  color: var(--color-text-secondary);
}
.model-pill {
  font-weight: 600;
  color: var(--color-text);
  background: var(--color-bg);
  border: 1px solid var(--color-border);
  padding: 1px 7px;
  border-radius: var(--radius-sm, 4px);
}
.meta-spacer {
  flex: 1;
}
.meta-count {
  color: var(--color-text-muted);
}
.expand-all-actions {
  display: flex;
  align-items: center;
}
.meta-action-btn {
  font-size: 10.5px;
  color: var(--color-primary);
  background: var(--color-bg);
  border: 1px solid var(--color-border);
  border-radius: var(--radius-sm, 4px);
  padding: 2px 8px;
  cursor: pointer;
  transition: all var(--transition-fast);
}
.meta-action-btn:hover {
  border-color: var(--color-primary);
  background: color-mix(in srgb, var(--color-primary) 8%, var(--color-bg));
}

/* ─── System Prompt Card ─── */
.system-card {
  border: 1px solid var(--color-border);
  background: var(--color-bg);
  border-radius: var(--radius-md);
  overflow: hidden;
  transition: all var(--transition-fast);
}
.system-card.open {
  border-color: color-mix(in srgb, var(--color-primary) 30%, var(--color-border));
}
.system-header {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  padding: 7px 12px;
  background: var(--color-bg-sidebar);
  cursor: pointer;
  user-select: none;
}
.system-badge {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  font-weight: 700;
  font-size: 11px;
  font-family: var(--font-mono);
  color: var(--color-primary);
  letter-spacing: 0.05em;
  flex-shrink: 0;
}
.system-len {
  font-size: 10px;
  color: var(--color-text-muted);
  font-family: var(--font-mono);
  flex-shrink: 0;
}
.system-preview-text {
  font-family: var(--font-mono);
  font-size: 11px;
  color: var(--color-text-muted);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  flex: 1;
  min-width: 0;
}
.system-body {
  padding: 12px 14px;
  max-height: 380px;
  overflow-y: auto;
  border-top: 1px solid var(--color-border);
}
.prompt-text {
  margin: 0;
  font-family: var(--font-mono);
  font-size: 11.5px;
  line-height: 1.65;
  color: var(--color-text);
  white-space: pre-wrap;
  word-break: break-word;
}

/* ─── Dialog Stream ─── */
.dialog-stream {
  display: flex;
  flex-direction: column;
  gap: 6px;
}
.msg-row {
  display: flex;
  gap: 8px;
  align-items: flex-start;
}
.msg-avatar {
  width: 24px;
  height: 24px;
  border-radius: 50%;
  display: flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
  margin-top: 4px;
}
.avatar-user {
  background: color-mix(in srgb, var(--color-primary) 15%, transparent);
  color: var(--color-primary);
}
.avatar-assistant {
  background: color-mix(in srgb, var(--color-success, #22c55e) 15%, transparent);
  color: var(--color-success, #22c55e);
}
.avatar-tool {
  background: color-mix(in srgb, var(--color-warning, #f59e0b) 15%, transparent);
  color: var(--color-warning, #f59e0b);
}
.avatar-system {
  background: color-mix(in srgb, var(--color-text-muted) 15%, transparent);
  color: var(--color-text-muted);
}

.msg-bubble-wrap {
  flex: 1;
  min-width: 0;
  border: 1px solid var(--color-border);
  border-radius: var(--radius-md);
  background: var(--color-bg);
  overflow: hidden;
  transition: all var(--transition-fast);
}
.msg-bubble-wrap:hover {
  border-color: color-mix(in srgb, var(--color-primary) 30%, var(--color-border));
}
.msg-bubble-wrap.open {
  border-color: color-mix(in srgb, var(--color-primary) 35%, var(--color-border));
  box-shadow: 0 1px 3px rgba(0, 0, 0, 0.03);
}

.bubble-header {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 6px 10px;
  background: var(--color-bg-sidebar);
  cursor: pointer;
  user-select: none;
}
.role-name {
  font-family: var(--font-mono);
  font-weight: 700;
  font-size: 11px;
  flex-shrink: 0;
}
.role-user .role-name {
  color: var(--color-primary);
}
.role-assistant .role-name {
  color: var(--color-success, #22c55e);
}
.role-tool .role-name {
  color: var(--color-warning, #f59e0b);
}
.sender-name {
  color: var(--color-text-muted);
  font-size: 10.5px;
  flex-shrink: 0;
}
.msg-index {
  font-family: var(--font-mono);
  font-size: 10px;
  color: var(--color-text-muted);
  flex-shrink: 0;
}

.collapsed-preview {
  font-size: 11px;
  color: var(--color-text-muted);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  flex: 1;
  min-width: 0;
  font-family: var(--font-sans);
}

.mini-tool-tag {
  display: inline-flex;
  align-items: center;
  gap: 3px;
  font-family: var(--font-mono);
  font-size: 9.5px;
  padding: 1px 6px;
  border-radius: var(--radius-sm, 4px);
  background: color-mix(in srgb, var(--color-warning, #f59e0b) 12%, transparent);
  color: var(--color-warning, #f59e0b);
  flex-shrink: 0;
}

.card-spacer {
  flex: 1;
}
.card-action-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 22px;
  height: 22px;
  border: none;
  background: transparent;
  color: var(--color-text-muted);
  border-radius: var(--radius-sm, 4px);
  cursor: pointer;
  transition: all var(--transition-fast);
}
.card-action-btn:hover {
  background: var(--color-bg-hover);
  color: var(--color-text);
}
.collapse-icon {
  color: var(--color-text-muted);
  flex-shrink: 0;
}

.bubble-content {
  padding: 10px 12px;
  display: flex;
  flex-direction: column;
  gap: 8px;
  border-top: 1px solid var(--color-border);
}
.content-text {
  font-size: 12px;
  line-height: 1.6;
  color: var(--color-text);
  white-space: pre-wrap;
  word-break: break-word;
}

/* ─── Thinking Block ─── */
.thinking-block {
  border: 1px dashed var(--color-border);
  border-radius: var(--radius-sm, 4px);
  background: var(--color-bg-sidebar);
  overflow: hidden;
}
.thinking-header {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 5px 10px;
  font-size: 11px;
  color: var(--color-text-muted);
  cursor: pointer;
}
.thinking-body {
  padding: 8px 10px;
  font-family: var(--font-mono);
  font-size: 11px;
  color: var(--color-text-secondary);
  line-height: 1.5;
  white-space: pre-wrap;
  border-top: 1px solid var(--color-border);
}

/* ─── Tool Call & Result ─── */
.tool-call-block,
.tool-result-block {
  border: 1px solid var(--color-border);
  border-radius: var(--radius-md);
  overflow: hidden;
}
.tool-call-header,
.tool-result-header {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 6px 10px;
  background: var(--color-bg-sidebar);
  cursor: pointer;
}
.tool-badge,
.result-badge {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  font-size: 10px;
  font-weight: 600;
  padding: 2px 7px;
  border-radius: var(--radius-sm, 4px);
}
.tool-badge {
  background: color-mix(in srgb, var(--color-warning, #f59e0b) 15%, transparent);
  color: var(--color-warning, #f59e0b);
}
.result-badge {
  background: color-mix(in srgb, var(--color-success, #22c55e) 15%, transparent);
  color: var(--color-success, #22c55e);
}
.result-badge.error {
  background: color-mix(in srgb, var(--color-danger, #ef4444) 15%, transparent);
  color: var(--color-danger, #ef4444);
}
.tool-fn-name {
  font-family: var(--font-mono);
  font-weight: 600;
  color: var(--color-text);
  font-size: 11px;
}
.tool-call-id {
  font-family: var(--font-mono);
  font-size: 10px;
  color: var(--color-text-muted);
}
.tool-call-body,
.tool-result-body {
  padding: 8px 10px;
  border-top: 1px solid var(--color-border);
  background: var(--color-bg);
  max-height: 320px;
  overflow-y: auto;
}
.code-block {
  margin: 0;
  font-family: var(--font-mono);
  font-size: 11px;
  line-height: 1.5;
  color: var(--color-text);
  white-space: pre-wrap;
  word-break: break-all;
}

/* ─── Transitions ─── */
.flow-collapse-enter-active,
.flow-collapse-leave-active {
  transition: all 180ms ease;
  overflow: hidden;
}
.flow-collapse-enter-from,
.flow-collapse-leave-to {
  opacity: 0;
  max-height: 0;
}
.flow-collapse-enter-to,
.flow-collapse-leave-from {
  opacity: 1;
  max-height: 1200px;
}

.messages-empty {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  padding: 40px 20px;
  color: var(--color-text-muted);
  gap: 8px;
  text-align: center;
}
.empty-icon {
  opacity: 0.4;
}
.empty-title {
  font-size: var(--text-sm);
  font-weight: 500;
  color: var(--color-text-secondary);
}
.empty-desc {
  font-size: var(--text-xs);
  color: var(--color-text-muted);
}
</style>

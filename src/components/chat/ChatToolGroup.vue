<script setup lang="ts">
import { ref } from "vue";
import SvgIcon from "../icons/SvgIcon.vue";
import CopyButton from "../CopyButton.vue";
import { t } from "../../i18n";
import type { ToolBadgeItem, ToolGroupSegment } from "../../types/chatTurn";

import { extractWidgetData, ensureWorkBuddySvgStyles } from "../../utils/svg";
import {
  copyableToolResultContent,
  displayToolResultContent,
  ensureToolResultFullContent,
  isLazyToolResult,
  toolResultFetchFailed,
  toolResultFetchPending,
  toolResultTruncationSuffix,
} from "../../composables/useToolResultFullContent";

const props = defineProps<{
  segment: ToolGroupSegment;
  subagentMap?: Record<string, any>;
}>();

defineEmits<{
  openSubagent: [filePath: string, label: string];
}>();

const expandedGroups = ref<Set<string>>(new Set());

function toggleGroup(toolName: string) {
  const newSet = new Set(expandedGroups.value);
  if (newSet.has(toolName)) {
    newSet.delete(toolName);
  } else {
    newSet.add(toolName);
    // 懒加载：展开分组时自动拉取组内所有截断 tool_result 的全文
    const group = props.segment.groups.find((g) => g.tool_name === toolName);
    if (group) {
      for (const item of group.items) {
        if (isLazyItem(item)) {
          void ensureToolResultFullContent(itemKey(item), item);
        }
      }
    }
  }
  expandedGroups.value = newSet;
}

function toolIcon(toolName: string): string {
  const name = toolName.toLowerCase();
  if (name.includes("widget") || name.includes("diagram") || name.includes("svg") || name.includes("show_widget")) return "image";
  if (name.includes("bash") || name.includes("terminal") || name.includes("command") || name.includes("exec")) return "terminal";
  if (name.includes("read") || name.includes("cat") || name.includes("view")) return "file-text";
  if (name.includes("edit") || name.includes("write") || name.includes("patch") || name.includes("replace")) return "pencil";
  if (name.includes("grep") || name.includes("glob") || name.includes("search") || name.includes("find")) return "search";
  if (name.includes("web") || name.includes("fetch") || name.includes("http") || name.includes("curl")) return "globe";
  if (name.includes("agent") || name.includes("subagent")) return "brain";
  if (name.includes("skill")) return "sparkles";
  if (name.includes("ask") || name.includes("question")) return "help-circle";
  return "wrench";
}

function getWidget(input: string) {
  return extractWidgetData(input);
}

function getSubagent(item: { tool_name: string; tool_use_id?: string }) {
  // 不限定工具名（Claude 为 Agent、DSH 为 subagent）：map 命中即视为子代理调用
  if (!item.tool_use_id || !props.subagentMap) return null;
  return props.subagentMap[item.tool_use_id] ?? null;
}

// ─── tool_result 懒加载 ───────────────────────────────
function itemKey(item: ToolBadgeItem): string {
  return `${item.originalIndex}-${item.partIndex ?? 0}`;
}
function isLazyItem(item: ToolBadgeItem): boolean {
  return item.part_type === "tool_result" && isLazyToolResult(item);
}
/** 渲染内容 = 已缓存全文 ?? item.input（预览或全文）；被截断时追加本地化提示 */
function itemDisplayContent(item: ToolBadgeItem): string {
  // 非懒加载项不走全文缓存（与改造前一致），只按标志补提示
  if (!isLazyItem(item)) return `${item.input}${toolResultTruncationSuffix(item.truncated)}`;
  return displayToolResultContent(itemKey(item), item.input, item.truncated);
}
/** 复制语义：await 拉全文再复制；失败回退预览（copyableToolResultContent 已兜底） */
function itemCopyPromise(item: ToolBadgeItem): (() => Promise<string>) | undefined {
  if (!isLazyItem(item)) return undefined;
  return () => copyableToolResultContent(itemKey(item), item, item.input);
}
function itemLazyHint(item: ToolBadgeItem): string {
  if (!isLazyItem(item)) return "";
  const key = itemKey(item);
  if (toolResultFetchFailed(key)) return t("chat.toolGroup.fullContentFailed");
  if (toolResultFetchPending(key)) return t("chat.toolGroup.loadingFullContent");
  return "";
}
</script>

<template>
  <div class="chat-tool-group" :class="{ 'has-error': segment.hasError }">
    <div class="tool-badges-bar">
      <button
        v-for="group in segment.groups"
        :key="group.tool_name"
        type="button"
        class="tool-badge"
        :class="{
          active: expandedGroups.has(group.tool_name),
          'has-error': group.hasError
        }"
        @click="toggleGroup(group.tool_name)"
      >
        <SvgIcon :name="toolIcon(group.tool_name)" :size="12" class="badge-icon" />
        <span class="badge-name">{{ group.tool_name }}</span>
        <span v-if="group.count > 1" class="badge-count">{{ group.count }}</span>
        <SvgIcon
          name="chevron-down"
          :size="10"
          class="badge-arrow"
          :class="{ rotate: expandedGroups.has(group.tool_name) }"
        />
      </button>
    </div>

    <!-- Expanded Tool Details -->
    <div v-for="group in segment.groups" :key="`details-${group.tool_name}`">
      <div v-if="expandedGroups.has(group.tool_name)" class="tool-details-list">
        <div
          v-for="item in group.items"
          :key="item.id"
          class="tool-item-detail"
          :class="{ 'is-error': item.is_error }"
        >
          <div class="tool-item-header">
            <SvgIcon :name="toolIcon(item.tool_name)" :size="12" class="item-icon" />
            <span class="tool-item-summary">{{ item.summary || item.tool_name }}</span>
            <div class="tool-item-spacer"></div>
            <CopyButton
              v-if="item.input"
              :text="itemDisplayContent(item)"
              :text-promise="itemCopyPromise(item)"
              size="sm"
            />
            <button
              v-if="getSubagent(item)"
              class="agent-link-btn"
              :title="t('chat.toolGroup.openSubagentInNewTab')"
              @click.stop="$emit('openSubagent', getSubagent(item)!.file_path, getSubagent(item)!.label)"
            >
              <SvgIcon name="external-link" :size="12" />
            </button>
          </div>
          <div
            v-if="getWidget(item.input)"
            class="tool-widget-preview workbuddy-svg-widget"
            v-html="ensureWorkBuddySvgStyles(getWidget(item.input)!.widgetCode)"
          ></div>
          <pre v-if="item.input" class="tool-item-code"><code>{{ itemDisplayContent(item) }}</code></pre>
          <div v-if="itemLazyHint(item)" class="tool-item-lazy-hint">{{ itemLazyHint(item) }}</div>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.chat-tool-group {
  margin: var(--space-1) 0 var(--space-2) 0;
}

.tool-badges-bar {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
  align-items: center;
}

.tool-badge {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  padding: 3px 9px;
  border-radius: var(--radius-full);
  background: var(--color-bg-hover);
  border: 1px solid var(--color-border);
  font-size: 11.5px;
  font-weight: 500;
  color: var(--color-text-secondary);
  cursor: pointer;
  transition: all var(--transition-fast);
  box-shadow: 0 1px 2px rgba(0, 0, 0, 0.02);
}

.tool-badge:hover {
  background: var(--color-bg-active);
  color: var(--color-text);
  border-color: var(--color-border);
  transform: translateY(-1px);
}

.tool-badge.active {
  background: var(--color-primary-light);
  color: var(--color-primary);
  border-color: color-mix(in srgb, var(--color-primary) 30%, transparent);
}

.tool-badge.has-error {
  border-color: var(--color-danger);
  color: var(--color-danger);
  background: color-mix(in srgb, var(--color-danger) 10%, transparent);
}

.badge-icon {
  opacity: 0.8;
}

.badge-count {
  font-size: 10.5px;
  font-family: var(--font-mono);
  background: color-mix(in srgb, currentColor 12%, transparent);
  padding: 0 4px;
  border-radius: var(--radius-full);
  font-weight: 600;
}

.badge-arrow {
  transition: transform 0.2s ease;
  opacity: 0.6;
}

.badge-arrow.rotate {
  transform: rotate(180deg);
}

.tool-details-list {
  margin-top: 8px;
  display: flex;
  flex-direction: column;
  gap: 6px;
  padding-left: 10px;
  border-left: 2px solid color-mix(in srgb, var(--color-primary) 30%, var(--color-border));
}

.tool-item-detail {
  padding: 8px 10px;
  background: var(--color-bg);
  border: 1px solid var(--color-border);
  border-radius: var(--radius-md);
  font-size: 12px;
  box-shadow: 0 1px 3px rgba(0, 0, 0, 0.02);
}

.tool-item-detail.is-error {
  border-color: color-mix(in srgb, var(--color-danger) 40%, var(--color-border));
  background: color-mix(in srgb, var(--color-danger) 4%, var(--color-bg));
}

.tool-item-header {
  display: flex;
  align-items: center;
  gap: 6px;
  font-weight: 500;
  color: var(--color-text);
}

.item-icon {
  color: var(--color-primary);
  opacity: 0.85;
}

.tool-item-summary {
  font-family: var(--font-mono);
  font-size: 11.5px;
  color: var(--color-text);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.tool-item-spacer {
  flex: 1;
}

.agent-link-btn {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 24px;
  height: 24px;
  border: none;
  background: transparent;
  color: var(--color-text-muted);
  border-radius: var(--radius-sm);
  cursor: pointer;
  transition: all var(--transition-fast);
}

.agent-link-btn:hover {
  background: var(--color-bg-hover);
  color: var(--color-primary);
}

.tool-item-code {
  margin-top: 6px;
  padding: 8px 10px;
  background: var(--color-bg-secondary);
  border: 1px solid var(--color-border-light);
  border-radius: var(--radius-sm);
  overflow-x: auto;
  font-family: var(--font-mono);
  font-size: 11.5px;
  line-height: 1.5;
  color: var(--color-text-secondary);
  max-height: 280px;
}
.tool-item-lazy-hint {
  margin-top: 4px;
  font-size: var(--text-2xs);
  color: var(--color-text-muted);
  font-style: italic;
}

.tool-widget-preview {
  margin-top: 6px;
  padding: var(--space-3);
  background: var(--color-bg);
  border: 1px solid var(--color-border-light);
  border-radius: var(--radius-sm);
  display: flex;
  justify-content: center;
  align-items: center;
  overflow-x: auto;
}

.tool-widget-preview :deep(svg) {
  max-width: 100%;
  height: auto;
  display: block;
  margin: 0 auto;
}
</style>

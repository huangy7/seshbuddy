<script setup lang="ts">
import type { ContentPart, SubagentInfo } from "../../types/session";
import { t } from "../../i18n";
import ChatTextPart from "./ChatTextPart.vue";
import ChatToolUsePart from "./ChatToolUsePart.vue";
import ChatToolResultPart from "./ChatToolResultPart.vue";
import ChatThinkingPart from "./ChatThinkingPart.vue";
import ChatImagePart from "./ChatImagePart.vue";
import {
  displayToolResultContent,
  ensureToolResultFullContent,
  isLazyToolResult,
  toolResultFetchFailed,
  toolResultFetchPending,
} from "../../composables/useToolResultFullContent";

const props = withDefaults(defineProps<{
  parts: ContentPart[];
  messageIndex: number;
  /** parts 是合成子集（如单个 text 段）时，其首个 part 在原始消息
     content_parts 内的下标；渲染缓存回查按 partIndexBase + pidx 定位 */
  partIndexBase?: number;
  filterTool: boolean;
  subagentMap: Record<string, SubagentInfo>;
  expandedTools: Record<string, boolean>;
  renderTextPart: (messageIndex: number, partIndex: number) => string;
  renderToolUseSummary: (messageIndex: number, partIndex: number, text: string) => string;
  renderToolUseInput: (messageIndex: number, partIndex: number, text: string) => string;
  renderToolUseMarkdown: (messageIndex: number, partIndex: number) => string;
  renderToolResultSummary: (messageIndex: number, partIndex: number, text: string) => string;
  renderToolResultContent: (messageIndex: number, partIndex: number, text: string) => string;
  getWriteMarkdownContent: (part: { tool_name: string; input: string }) => string | null;
}>(), { partIndexBase: 0 });

const emit = defineEmits<{
  toggleTool: [key: string];
  markdownClick: [event: MouseEvent];
  openSubagent: [filePath: string, label: string];
  previewImage: [url: string];
  downloadImage: [url: string];
}>();

function isText(part: ContentPart): part is { type: "text"; text: string } {
  return part.type === "text";
}
function isToolUse(part: ContentPart): part is { type: "tool_use"; summary: string; tool_name: string; input: string; tool_use_id?: string } {
  return part.type === "tool_use";
}
function isToolResult(part: ContentPart): part is { type: "tool_result"; summary: string; content: string; is_error: boolean; tool_use_id?: string; full_len?: number; truncated_preview?: boolean; truncated?: boolean; source_offset?: number } {
  return part.type === "tool_result";
}
function isThinking(part: ContentPart): part is { type: "thinking"; thinking: string } {
  return part.type === "thinking";
}
function isImage(part: ContentPart): part is { type: "image"; media_type: string; data: string } {
  return part.type === "image";
}
function isImageRef(part: ContentPart): part is { type: "image_ref"; path: string } {
  return part.type === "image_ref";
}

function toolKey(partIndex: number): string {
  return `${props.messageIndex}-${props.partIndexBase + partIndex}`;
}
function subagentFor(part: { tool_name: string; tool_use_id?: string }): SubagentInfo | null {
  // 不限定工具名（Claude 为 Agent、DSH 为 subagent）：map 由后端按真实关联构建，
  // tool_use_id 命中即视为子代理调用。
  if (!part.tool_use_id) return null;
  return props.subagentMap[part.tool_use_id] ?? null;
}

/** 懒加载：渲染内容 = 已缓存全文 ?? part 自带内容（预览或全文）；被截断时追加本地化提示 */
function toolResultDisplayContent(part: { content: string; truncated?: boolean }, partIndex: number): string {
  return displayToolResultContent(toolKey(partIndex), part.content, part.truncated);
}

/** 展开时自动拉全文（折叠不触发）；拉取完成由响应式缓存驱动重渲染 */
function onToggleToolResult(part: ContentPart, partIndex: number) {
  const key = toolKey(partIndex);
  const expanding = !props.expandedTools[key];
  if (expanding && part.type === "tool_result" && isLazyToolResult(part)) {
    void ensureToolResultFullContent(key, part);
  }
  emit("toggleTool", key);
}

/** 懒加载状态提示：加载中 / 失败（成功或无懒加载返回空串） */
function toolResultHint(part: ContentPart, partIndex: number): string {
  if (part.type !== "tool_result" || !isLazyToolResult(part)) return "";
  const key = toolKey(partIndex);
  if (toolResultFetchFailed(key)) return t("chat.toolGroup.fullContentFailed");
  if (toolResultFetchPending(key)) return t("chat.toolGroup.loadingFullContent");
  return "";
}
</script>

<template>
  <div class="message-body">
    <template v-for="(part, pidx) in parts" :key="pidx">
      <ChatTextPart
        v-if="isText(part)"
        :html="renderTextPart(messageIndex, partIndexBase + pidx)"
        @markdown-click="$emit('markdownClick', $event)"
      />
      <ChatToolUsePart
        v-else-if="isToolUse(part) && filterTool"
        :expanded="!!expandedTools[toolKey(pidx)]"
        :tool-name="part.tool_name"
        :summary-html="renderToolUseSummary(messageIndex, partIndexBase + pidx, part.summary)"
        :markdown-html="expandedTools[toolKey(pidx)] && getWriteMarkdownContent(part) ? renderToolUseMarkdown(messageIndex, partIndexBase + pidx) : null"
        :input-html="expandedTools[toolKey(pidx)] ? renderToolUseInput(messageIndex, partIndexBase + pidx, part.input) : ''"
        :subagent="subagentFor(part)"
        :data-tool-key="toolKey(pidx)"
        @toggle="$emit('toggleTool', toolKey(pidx))"
        @markdown-click="$emit('markdownClick', $event)"
        @open-subagent="(filePath, label) => $emit('openSubagent', filePath, label)"
      />
      <ChatToolResultPart
        v-else-if="isToolResult(part) && filterTool"
        :expanded="!!expandedTools[toolKey(pidx)]"
        :is-error="part.is_error"
        :summary-html="renderToolResultSummary(messageIndex, partIndexBase + pidx, part.summary)"
        :content-html="expandedTools[toolKey(pidx)] ? renderToolResultContent(messageIndex, partIndexBase + pidx, toolResultDisplayContent(part, pidx)) : ''"
        :hint="expandedTools[toolKey(pidx)] ? toolResultHint(part, pidx) : ''"
        :data-tool-key="toolKey(pidx)"
        @toggle="onToggleToolResult(part, pidx)"
        @markdown-click="$emit('markdownClick', $event)"
      />
      <ChatThinkingPart
        v-else-if="isThinking(part)"
        :expanded="!!expandedTools[toolKey(pidx)]"
        :thinking="part.thinking"
        :data-tool-key="toolKey(pidx)"
        @toggle="$emit('toggleTool', toolKey(pidx))"
      />
      <ChatImagePart
        v-else-if="isImage(part)"
        :media-type="part.media_type"
        :data="part.data"
        @preview="$emit('previewImage', $event)"
        @download="$emit('downloadImage', $event)"
      />
      <ChatImagePart
        v-else-if="isImageRef(part)"
        :path="part.path"
        @preview="$emit('previewImage', $event)"
        @download="$emit('downloadImage', $event)"
      />
    </template>
  </div>
</template>

<style scoped>
.message-body {
  font-size: var(--text-base);
  line-height: var(--leading-relaxed);
}
</style>

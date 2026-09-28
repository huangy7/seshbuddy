<script setup lang="ts">
import { computed } from "vue";
import type { ChatMessage, SubagentInfo } from "../types/session";
import { formatTimestamp } from "../utils/format";
import { cleanUserText } from "../utils/textClean";
import { t } from "../i18n";

const props = defineProps<{
  messages: ChatMessage[];
  loading: boolean;
  activeMessageIndex?: number | null;
  subagentMap?: Record<string, SubagentInfo>;
}>();

const emit = defineEmits<{
  jumpToMessage: [messageIndex: number];
}>();

interface TimelineNode {
  messageIndex: number;
  timestamp: string;
  preview: string;
  nodeType: "user" | "agent";
  subagentPath?: string;
  /** 纯图片节点携带的图片张数，合并相邻图片节点时累加 */
  imageCount?: number;
}

const timelineNodes = computed<TimelineNode[]>(() => {
  const nodes: TimelineNode[] = [];
  for (let i = 0; i < props.messages.length; i++) {
    const msg = props.messages[i];
    if (msg.role === "user") {
      // 跳过 is_meta 消息（系统占位行）
      if (msg.is_meta) continue;

      const textParts = msg.content_parts.filter((p) => p.type === "text");
      const raw = textParts.map((p) => (p as { text: string }).text).join(" ").trim();
      const cleaned = cleanUserText(raw);
      const hasImage = msg.content_parts.some(p => p.type === "image" || p.type === "image_ref");

      if (!cleaned) {
        // 纯图片消息：显示图片图标而非原始文本
        if (hasImage) {
          nodes.push({
            messageIndex: i,
            timestamp: msg.timestamp,
            preview: t("common.timelineView.imagePreview"),
            nodeType: "user",
            imageCount: 1,
          });
        }
        continue;
      }

      const firstLine = cleaned.split("\n")[0] || "";
      const preview = firstLine.length > 60 ? firstLine.slice(0, 60) + "…" : firstLine;
      nodes.push({ messageIndex: i, timestamp: msg.timestamp, preview, nodeType: "user" });
    } else if (msg.role === "assistant") {
      const agentParts = msg.content_parts.filter(
        (p) => p.type === "tool_use" && p.tool_name === "Agent"
      ) as { type: "tool_use"; summary: string; tool_name: string; input: string; tool_use_id?: string }[];
      
      for (const part of agentParts) {
        let subagentPath: string | undefined;
        let preview = part.summary || t("common.timelineView.agentCall");
        
        if (part.tool_use_id && props.subagentMap && props.subagentMap[part.tool_use_id]) {
          subagentPath = props.subagentMap[part.tool_use_id].file_path;
          preview = props.subagentMap[part.tool_use_id].label || preview;
        }
        
        nodes.push({ messageIndex: i, timestamp: msg.timestamp, preview, nodeType: "agent", subagentPath });
      }
    }
  }

  // 后处理：合并相邻纯图片节点。
  // 张数走 imageCount 字段而不是回读 preview：preview 是本地化文案，
  // 从译文里正则抠数字在非中文语言下抠不到。
  const merged: TimelineNode[] = [];
  for (const node of nodes) {
    const prev = merged.length > 0 ? merged[merged.length - 1] : null;
    if (prev && prev.imageCount !== undefined && node.imageCount !== undefined && node.nodeType === "user") {
      prev.imageCount += 1;
      prev.preview = t("common.timelineView.imageCount", { count: prev.imageCount });
    } else {
      merged.push({ ...node });
    }
  }
  return merged;
});

function formatTime(ts: string): string {
  const full = formatTimestamp(ts);
  const match = full.match(/(\d{1,2}:\d{2})/);
  return match ? match[1] : full;
}
</script>

<template>
  <div class="timeline-view">
    <div v-if="loading" class="timeline-loading">{{ t("common.timelineView.loading") }}</div>
    <div v-else-if="timelineNodes.length === 0" class="timeline-empty">{{ t("common.timelineView.empty") }}</div>
    <div v-else class="timeline-track">
      <div
        v-for="(node, idx) in timelineNodes"
        :key="`${node.messageIndex}-${node.nodeType}`"
        class="timeline-node"
        :class="{ active: activeMessageIndex === node.messageIndex, 'agent-node': node.nodeType === 'agent' }"
        @click="emit('jumpToMessage', node.messageIndex)"
      >
        <div class="timeline-dot-col">
          <div class="timeline-dot" :class="{ 'agent-dot': node.nodeType === 'agent' }"></div>
          <div v-if="idx < timelineNodes.length - 1" class="timeline-line"></div>
        </div>
        <div class="timeline-content">
          <span class="timeline-time">{{ formatTime(node.timestamp) }}</span>
          <span class="timeline-separator">────</span>
          <span class="timeline-preview">{{ node.preview }}</span>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.timeline-view {
  flex: 1;
  overflow-y: auto;
  padding: var(--space-3);
}
.timeline-loading,
.timeline-empty {
  text-align: center;
  padding: var(--space-6);
  color: var(--color-text-muted);
  font-size: var(--text-sm);
}
.timeline-track {
  max-width: none;
  margin: 0;
}
.timeline-node {
  display: flex;
  gap: var(--space-3);
  cursor: pointer;
  min-height: 40px;
}
.timeline-node:hover .timeline-preview {
  color: var(--color-primary);
}
.timeline-node.agent-node:hover .timeline-preview {
  color: var(--color-purple-500, #a855f7);
}
.timeline-node:hover .timeline-dot,
.timeline-node.active .timeline-dot {
  background: var(--color-primary);
  transform: scale(1.3);
}
.timeline-node.agent-node:hover .timeline-dot,
.timeline-node.agent-node.active .timeline-dot {
  background: var(--color-purple-500, #a855f7);
}
.timeline-node.active .timeline-time,
.timeline-node.active .timeline-preview {
  color: var(--color-primary);
}
.timeline-node.agent-node.active .timeline-time,
.timeline-node.agent-node.active .timeline-preview {
  color: var(--color-purple-500, #a855f7);
}
.timeline-dot-col {
  display: flex;
  flex-direction: column;
  align-items: center;
  width: 12px;
  flex-shrink: 0;
}
.timeline-dot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  background: var(--color-text-muted);
  flex-shrink: 0;
  margin-top: 6px;
  transition: all var(--transition-fast);
}
.timeline-dot.agent-dot {
  background: var(--color-purple-400, #c084fc);
  border-radius: 2px;
}
.timeline-line {
  width: 1px;
  flex: 1;
  background: var(--color-border);
  min-height: 16px;
}
.timeline-content {
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  gap: 2px;
  padding-bottom: var(--space-3);
  min-width: 0;
}
.timeline-time {
  font-family: var(--font-mono);
  font-size: var(--text-xs);
  color: var(--color-text-muted);
  white-space: nowrap;
  flex-shrink: 0;
}
.timeline-separator {
  display: none;
}
.timeline-preview {
  font-size: var(--text-sm);
  color: var(--color-text-secondary);
  transition: color var(--transition-fast);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  max-width: 100%;
}
</style>

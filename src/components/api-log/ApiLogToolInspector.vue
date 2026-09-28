<script setup lang="ts">
import { computed, ref } from "vue";
import { t } from "../../i18n";
import SvgIcon from "../icons/SvgIcon.vue";

// 请求体中的工具定义检查器：卡片式展示名称/描述/参数 schema
const props = defineProps<{
  reqBody: string | null;
}>();

interface ToolInfo {
  name: string;
  description: string;
  schemaPretty: string;
}

const tools = computed<ToolInfo[]>(() => {
  if (!props.reqBody) return [];
  try {
    const body = JSON.parse(props.reqBody);
    const arr = Array.isArray(body?.tools)
      ? body.tools
      : Array.isArray(body?.functions)
        ? body.functions.map((f: any) => ({ type: "function", function: f }))
        : null;
    if (!Array.isArray(arr)) return [];
    return arr.map((tool: any) => {
      // Anthropic: {name, description, input_schema}；OpenAI: {type:'function', function:{name, description, parameters}}
      const fn = tool?.function ?? tool;
      const schema = fn?.input_schema ?? fn?.parameters ?? null;
      return {
        name: String(fn?.name ?? tool?.name ?? t("api-log.toolInspector.unnamed")),
        description: String(fn?.description ?? ""),
        schemaPretty: schema ? JSON.stringify(schema, null, 2) : "",
      };
    });
  } catch {
    return [];
  }
});

const expanded = ref<Set<number>>(new Set());

function toggle(idx: number) {
  const next = new Set(expanded.value);
  if (next.has(idx)) next.delete(idx);
  else next.add(idx);
  expanded.value = next;
}

function toggleAll() {
  if (expanded.value.size === tools.value.length) {
    expanded.value = new Set();
  } else {
    expanded.value = new Set(tools.value.map((_, i) => i));
  }
}
</script>

<template>
  <div class="tool-inspector">
    <div v-if="tools.length === 0" class="inspector-empty">{{ t('api-log.toolInspector.empty') }}</div>
    <template v-else>
      <div class="inspector-header">
        <span class="inspector-icon">
          <SvgIcon name="wrench" :size="13" />
        </span>
        <span class="inspector-label">{{ t('api-log.toolInspector.title') }}</span>
        <span class="inspector-count">{{ tools.length }}</span>
        <span class="inspector-spacer" />
        <button class="inspector-toggle-all" type="button" @click="toggleAll">
          {{ expanded.size === tools.length ? t('api-log.messagesView.collapseAll') : t('api-log.messagesView.expandAll') }}
        </button>
      </div>
      <div class="tool-list">
        <div v-for="(tool, idx) in tools" :key="idx" class="tool-card" :class="{ open: expanded.has(idx) }">
          <button class="tool-header" type="button" @click="toggle(idx)">
            <SvgIcon :name="expanded.has(idx) ? 'chevron-down' : 'chevron-right'" :size="12" class="tool-chevron" />
            <span class="tool-name">{{ tool.name }}</span>
            <span v-if="tool.schemaPretty" class="tool-has-schema">schema</span>
          </button>
          <Transition name="tool-expand">
            <div v-if="expanded.has(idx)" class="tool-body">
              <div v-if="tool.description" class="tool-desc">{{ tool.description }}</div>
              <div v-if="tool.schemaPretty" class="tool-schema-wrap">
                <div class="tool-schema-label">{{ t('api-log.toolInspector.inputSchema') }}</div>
                <pre class="tool-schema">{{ tool.schemaPretty }}</pre>
              </div>
            </div>
          </Transition>
        </div>
      </div>
    </template>
  </div>
</template>

<style scoped>
.tool-inspector {
  display: flex;
  flex-direction: column;
  gap: var(--space-3);
  font-size: var(--text-xs);
  padding: var(--space-2) var(--space-1);
}
.inspector-header {
  display: flex;
  align-items: center;
  gap: 6px;
}
.inspector-icon {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 22px;
  height: 22px;
  border-radius: var(--radius-sm, 4px);
  background: color-mix(in srgb, var(--color-warning, #f59e0b) 12%, transparent);
  color: var(--color-warning, #f59e0b);
  flex-shrink: 0;
}
.inspector-label {
  font-weight: 600;
  color: var(--color-text);
}
.inspector-count {
  font-family: var(--font-mono);
  font-size: 10px;
  font-weight: 600;
  color: var(--color-text-muted);
  background: var(--color-bg-hover);
  padding: 0 6px;
  border-radius: 999px;
  line-height: 18px;
}
.inspector-spacer {
  flex: 1;
}
.inspector-toggle-all {
  font-size: 11px;
  color: var(--color-text-muted);
  background: none;
  border: 1px solid var(--color-border);
  border-radius: var(--radius-md);
  padding: 2px 10px;
  cursor: pointer;
  transition: all var(--transition-fast);
}
.inspector-toggle-all:hover {
  color: var(--color-primary);
  border-color: var(--color-primary);
}
.inspector-empty {
  color: var(--color-text-muted);
  text-align: center;
  padding: var(--space-4);
}

/* ─── Tool List ─── */
.tool-list {
  display: flex;
  flex-direction: column;
  gap: 6px;
}
.tool-card {
  border: 1px solid var(--color-border);
  border-radius: var(--radius-md);
  overflow: hidden;
  transition: all var(--transition-fast);
}
.tool-card.open {
  border-color: color-mix(in srgb, var(--color-warning, #f59e0b) 30%, var(--color-border));
}
.tool-card:hover {
  border-color: color-mix(in srgb, var(--color-warning, #f59e0b) 25%, var(--color-border));
}
.tool-header {
  display: flex;
  align-items: center;
  gap: 6px;
  width: 100%;
  padding: 7px var(--space-3);
  background: var(--color-bg-sidebar);
  border: none;
  cursor: pointer;
  text-align: left;
  transition: background var(--transition-fast);
}
.tool-header:hover {
  background: var(--color-bg-hover);
}
.tool-chevron {
  color: var(--color-text-muted);
  flex-shrink: 0;
  transition: transform var(--transition-fast);
}
.tool-name {
  font-family: var(--font-mono);
  font-size: var(--text-xs);
  color: var(--color-text);
  font-weight: 600;
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.tool-has-schema {
  font-size: 10px;
  font-family: var(--font-mono);
  color: var(--color-text-muted);
  background: var(--color-bg-hover);
  padding: 1px 6px;
  border-radius: 999px;
  flex-shrink: 0;
  opacity: 0.7;
}
.tool-body {
  padding: var(--space-3);
  display: flex;
  flex-direction: column;
  gap: var(--space-3);
  border-top: 1px solid var(--color-border);
}
.tool-desc {
  color: var(--color-text-secondary);
  line-height: 1.6;
  white-space: pre-wrap;
  word-break: break-word;
  max-height: 200px;
  overflow-y: auto;
  padding: var(--space-2) var(--space-3);
  background: var(--color-bg-sidebar);
  border-radius: var(--radius-sm, 4px);
}
.tool-schema-wrap {
  display: flex;
  flex-direction: column;
  gap: var(--space-1);
}
.tool-schema-label {
  font-size: 10px;
  font-weight: 600;
  color: var(--color-text-muted);
  text-transform: uppercase;
  letter-spacing: 0.05em;
}
.tool-schema {
  margin: 0;
  font-family: var(--font-mono);
  font-size: 11px;
  color: var(--color-text);
  white-space: pre-wrap;
  word-break: break-all;
  max-height: 360px;
  overflow-y: auto;
  padding: var(--space-2) var(--space-3);
  background: var(--color-bg-sidebar);
  border-radius: var(--radius-sm, 4px);
  border: 1px solid var(--color-border);
}

/* ─── Transition ─── */
.tool-expand-enter-active,
.tool-expand-leave-active {
  transition: opacity 150ms ease, max-height 200ms ease;
  overflow: hidden;
}
.tool-expand-enter-from,
.tool-expand-leave-to {
  opacity: 0;
  max-height: 0;
}
.tool-expand-enter-to,
.tool-expand-leave-from {
  opacity: 1;
  max-height: 800px;
}
</style>

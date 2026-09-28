<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, defineAsyncComponent } from "vue";
import { t } from "../../i18n";
import type { TrafficDetail, TrafficSummary } from "../../composables/useProxy";
import { formatBytes, formatTokenCount, formatTimestamp } from "../../utils/format";
import SvgIcon from "../icons/SvgIcon.vue";
import ApiLogSseView from "./ApiLogSseView.vue";
import ApiLogSkillPanel from "./ApiLogSkillPanel.vue";
import ApiLogToolInspector from "./ApiLogToolInspector.vue";
import ApiLogMessagesView from "./ApiLogMessagesView.vue";
import ApiLogDiffView from "./ApiLogDiffView.vue";
import { parseRequestMessages } from "../../utils/reqMessages";

// Monaco 体积大且依赖浏览器 API，延迟到首次展示 body/headers 时才加载
const MonacoViewer = defineAsyncComponent(() => import("../common/MonacoViewer.vue"));

export type DetailSection = "messages" | "body" | "headers" | "tools" | "skills" | "parsed" | "diff";

const props = withDefaults(defineProps<{
  detail: TrafficDetail;
  /** 行级摘要（token/耗时只在列表行上有） */
  summary?: TrafficSummary | null;
  trafficList?: TrafficSummary[];
  /** trafficList 之外是否还有未加载的更老记录（Diff 视图据此区分"首条"与"未加载"） */
  trafficHasMore?: boolean;
  getDetailFn?: (id: string) => Promise<TrafficDetail | null>;
  detailTab: "request" | "response";
  detailSection: DetailSection;
  bodyText: string;
  bodyLanguage: string;
  headersText: string;
  headersLanguage: string;
  hasTools: boolean;
  hasSkills: boolean;
  isSse: boolean;
  copyState: "idle" | "copied";
  fullscreen?: boolean;
  isSidebarCollapsed?: boolean;
}>(), {
  summary: null,
  trafficList: () => [],
  fullscreen: false,
  isSidebarCollapsed: false,
});

const emit = defineEmits<{
  "update:detailTab": [tab: "request" | "response"];
  "update:detailSection": [section: DetailSection];
  copyDetail: [];
  resetCopyState: [];
  toggleFullscreen: [];
  toggleSidebar: [];
  copied: [label: string];
}>();

const parsedMessagesInfo = computed(() => {
  return parseRequestMessages(props.detail.req_body);
});

// 缓存读写各自独立可选，四种组合不值得各建一个 key；「 / 」是跨语言一致的符号分隔符，
// 与列表用的「、」不同，故不进语言包
const tokenUsageTitle = computed(() => {
  const summary = props.summary;
  if (!summary) return "";
  const parts = [
    t("api-log.requestDetail.tokenInput", { count: summary.input_tokens ?? "—" }),
    t("api-log.requestDetail.tokenOutput", { count: summary.output_tokens ?? "—" }),
  ];
  if (summary.cache_read_tokens) {
    parts.push(t("api-log.requestDetail.tokenCacheRead", { count: summary.cache_read_tokens }));
  }
  if (summary.cache_creation_tokens) {
    parts.push(t("api-log.requestDetail.tokenCacheWrite", { count: summary.cache_creation_tokens }));
  }
  return parts.join(" / ");
});

const toolCount = computed(() => {
  try {
    const json = JSON.parse(props.detail.req_body ?? "");
    const tools = json?.tools ?? json?.functions;
    return Array.isArray(tools) ? tools.length : 0;
  } catch {
    return 0;
  }
});

function selectTab(tab: "request" | "response") {
  emit("update:detailTab", tab);
  emit("resetCopyState");
}

function selectSection(section: DetailSection) {
  emit("update:detailSection", section);
  emit("resetCopyState");
}

function statusColor(code: number | null): string {
  if (!code) return "var(--color-text-muted)";
  if (code >= 200 && code < 300) return "var(--color-success, #22c55e)";
  if (code >= 400) return "var(--color-danger, #dc2626)";
  return "var(--color-warning, #f59e0b)";
}

function formatDuration(ms: number): string {
  if (ms < 1000) return `${ms}ms`;
  return `${(ms / 1000).toFixed(2)}s`;
}

// Esc 退出全屏（capture 阶段拦截，避免冒泡到 App 关掉整个 dialog）
function onKeydown(e: KeyboardEvent) {
  if (props.fullscreen && e.key === "Escape") {
    e.preventDefault();
    e.stopPropagation();
    emit("toggleFullscreen");
  }
}

onMounted(() => document.addEventListener("keydown", onKeydown, true));
onBeforeUnmount(() => document.removeEventListener("keydown", onKeydown, true));
</script>

<template>
  <div class="request-detail-root" :class="{ fullscreen }">
    <!-- 头部摘要信息栏 -->
    <div class="detail-header-bar">
      <div class="header-left">
        <!-- 侧栏收起时展示的展开按钮 (内联在 Header 中，不悬浮遮挡) -->
        <button
          v-if="isSidebarCollapsed"
          class="inline-sidebar-toggle-btn"
          type="button"
          :title="t('api-log.requestDetail.expandSidebarTitle')"
          @click="emit('toggleSidebar')"
        >
          <SvgIcon name="sidebar" :size="14" />
        </button>

        <span class="method-badge" :data-method="detail.method">{{ detail.method }}</span>
        <span class="path-display" :title="detail.path">{{ detail.path }}</span>
        <span class="status-pill" :style="{ color: statusColor(detail.status), borderColor: statusColor(detail.status) }">
          <span class="status-dot" :style="{ background: statusColor(detail.status) }" />
          {{ detail.status || '200' }}
        </span>
      </div>

      <div class="header-stats" v-if="summary">
        <span class="stat-pill duration-pill" :title="t('api-log.requestDetail.durationTitle')">
          <SvgIcon name="clock" :size="11" />
          {{ formatDuration(summary.duration_ms) }}
        </span>
        <span
          v-if="summary.input_tokens != null || summary.output_tokens != null"
          class="stat-pill token-pill"
          :title="tokenUsageTitle"
        >
          <SvgIcon name="zap" :size="11" />
          ↑{{ formatTokenCount(summary.input_tokens ?? 0) }} ↓{{ formatTokenCount(summary.output_tokens ?? 0) }}
        </span>
        <span class="stat-pill size-pill" :title="t('api-log.requestDetail.sizeTitle')">
          {{ formatBytes(summary.req_size + summary.res_size) }}
        </span>
        <span class="stat-pill time-pill">{{ formatTimestamp(detail.timestamp) }}</span>
      </div>

      <div class="header-actions">
        <button
          class="action-fullscreen-btn"
          type="button"
          :title="fullscreen ? t('common.mermaid.exitFullscreen') : t('api-log.requestDetail.fullscreenTitle')"
          @click.stop="emit('toggleFullscreen')"
        >
          <SvgIcon :name="fullscreen ? 'minimize-2' : 'maximize-2'" :size="13" />
          <span>{{ fullscreen ? t('api-log.requestDetail.exitFullscreenLabel') : t('api-log.requestDetail.fullscreenLabel') }}</span>
        </button>
      </div>
    </div>

    <!-- 导航分段控件 -->
    <div class="detail-navigation-bar">
      <!-- 一级 Tab：Request / Response -->
      <div class="segmented-control main-tabs">
        <button
          class="seg-btn"
          :class="{ active: detailTab === 'request' }"
          type="button"
          @click="selectTab('request')"
        >
          <span>{{ t('api-log.requestDetail.tabRequest') }}</span>
        </button>
        <button
          class="seg-btn"
          :class="{ active: detailTab === 'response' }"
          type="button"
          @click="selectTab('response')"
        >
          <span>{{ t('api-log.requestDetail.tabResponse') }}</span>
        </button>
      </div>

      <div class="nav-divider" />

      <!-- 二级视图切换 -->
      <div class="sub-nav-group">
        <template v-if="detailTab === 'request'">
          <button
            v-if="parsedMessagesInfo.hasMessages"
            class="sub-nav-btn"
            :class="{ active: detailSection === 'messages' }"
            type="button"
            @click="selectSection('messages')"
          >
            <span>{{ t('api-log.requestDetail.tabMessages') }}</span>
            <span v-if="parsedMessagesInfo.messages.length" class="sub-badge">{{ parsedMessagesInfo.messages.length }}</span>
          </button>
          <button
            class="sub-nav-btn"
            :class="{ active: detailSection === 'body' }"
            type="button"
            @click="selectSection('body')"
          >
            <span>{{ t('api-log.requestDetail.rawJson') }}</span>
          </button>
          <button
            v-if="hasTools"
            class="sub-nav-btn"
            :class="{ active: detailSection === 'tools' }"
            type="button"
            @click="selectSection('tools')"
          >
            <span>{{ t('api-log.requestDetail.tabTools') }}</span>
            <span v-if="toolCount" class="sub-badge">{{ toolCount }}</span>
          </button>
          <button
            v-if="hasSkills"
            class="sub-nav-btn"
            :class="{ active: detailSection === 'skills' }"
            type="button"
            @click="selectSection('skills')"
          >
            <span>{{ t('api-log.requestDetail.tabSkills') }}</span>
          </button>
          <button
            class="sub-nav-btn"
            :class="{ active: detailSection === 'headers' }"
            type="button"
            @click="selectSection('headers')"
          >
            <span>{{ t('chat.apiDetail.headers') }}</span>
          </button>
          <button
            class="sub-nav-btn"
            :class="{ active: detailSection === 'diff' }"
            type="button"
            :title="t('api-log.requestDetail.diffTabTitle')"
            @click="selectSection('diff')"
          >
            <span>{{ t('api-log.requestDetail.tabDiff') }}</span>
          </button>
        </template>

        <template v-else>
          <button
            v-if="isSse"
            class="sub-nav-btn"
            :class="{ active: detailSection === 'parsed' }"
            type="button"
            @click="selectSection('parsed')"
          >
            <span>{{ t('api-log.requestDetail.tabSseParsed') }}</span>
          </button>
          <button
            class="sub-nav-btn"
            :class="{ active: detailSection === 'body' }"
            type="button"
            @click="selectSection('body')"
          >
            <span>{{ t('api-log.requestDetail.rawBody') }}</span>
          </button>
          <button
            class="sub-nav-btn"
            :class="{ active: detailSection === 'headers' }"
            type="button"
            @click="selectSection('headers')"
          >
            <span>{{ t('chat.apiDetail.headers') }}</span>
          </button>
        </template>
      </div>

      <div class="nav-spacer" />

      <!-- 复制按钮 -->
      <button class="action-copy-btn" type="button" @click.stop="emit('copyDetail')">
        <SvgIcon :name="copyState === 'copied' ? 'check' : 'copy'" :size="12" />
        <span>{{ copyState === 'copied' ? t('dialogs.common.copied') : t('app.ctxMenu.copy') }}</span>
      </button>
    </div>

    <!-- 核心视图工作台：占满剩余高度 -->
    <div class="detail-viewport">
      <ApiLogDiffView
        v-if="detailSection === 'diff' && detailTab === 'request' && getDetailFn"
        :current-detail="detail"
        :traffic-list="trafficList"
        :has-more="trafficHasMore"
        :get-detail-fn="getDetailFn"
      />
      <ApiLogMessagesView
        v-else-if="detailSection === 'messages' && detailTab === 'request'"
        :req-body="detail.req_body"
        @copied="emit('copied', $event)"
      />
      <ApiLogToolInspector
        v-else-if="detailSection === 'tools' && detailTab === 'request'"
        :req-body="detail.req_body"
      />
      <ApiLogSkillPanel
        v-else-if="detailSection === 'skills' && detailTab === 'request'"
        :req-body="detail.req_body"
      />
      <ApiLogSseView
        v-else-if="detailSection === 'parsed' && detailTab === 'response'"
        :raw="detail.res_body ?? ''"
      />
      <div v-else-if="detailSection === 'headers'" class="editor-container">
        <MonacoViewer
          :value="headersText"
          :language="headersLanguage"
        />
      </div>
      <div v-else class="editor-container">
        <MonacoViewer
          :value="bodyText"
          :language="bodyLanguage"
        />
      </div>
    </div>
  </div>
</template>

<style scoped>
.request-detail-root {
  display: flex;
  flex-direction: column;
  height: 100%;
  width: 100%;
  min-height: 0;
  background: var(--color-bg);
  overflow: hidden;
}

.request-detail-root.fullscreen {
  padding: 0;
}

/* ─── 头部信息栏 ─── */
.detail-header-bar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-2);
  padding: 0 14px;
  height: 42px;
  background: var(--color-bg);
  border-bottom: 1px solid var(--color-border);
  flex-shrink: 0;
}

.header-left {
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
  flex: 1;
}

.inline-sidebar-toggle-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 26px;
  height: 26px;
  border-radius: var(--radius-sm, 4px);
  border: 1px solid var(--color-border);
  background: var(--color-bg-sidebar);
  color: var(--color-text-secondary);
  cursor: pointer;
  flex-shrink: 0;
  transition: all var(--transition-fast);
}

.inline-sidebar-toggle-btn:hover {
  color: var(--color-primary);
  border-color: var(--color-primary);
  background: color-mix(in srgb, var(--color-primary) 8%, var(--color-bg));
}

.method-badge {
  font-family: var(--font-mono);
  font-weight: 700;
  font-size: 10.5px;
  padding: 2px 7px;
  border-radius: var(--radius-sm, 4px);
  background: color-mix(in srgb, var(--color-primary) 12%, transparent);
  color: var(--color-primary);
  flex-shrink: 0;
}

.path-display {
  font-family: var(--font-mono);
  font-size: 11.5px;
  font-weight: 600;
  color: var(--color-text);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  min-width: 0;
}

.status-pill {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  font-family: var(--font-mono);
  font-weight: 700;
  font-size: 10.5px;
  padding: 1px 7px;
  border-radius: 999px;
  border: 1px solid currentColor;
  background: color-mix(in srgb, currentColor 8%, transparent);
  flex-shrink: 0;
}

.status-dot {
  width: 5px;
  height: 5px;
  border-radius: 50%;
}

.header-stats {
  display: flex;
  align-items: center;
  gap: 5px;
  flex-shrink: 0;
}

.stat-pill {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  font-family: var(--font-mono);
  font-size: 10.5px;
  color: var(--color-text-muted);
  background: var(--color-bg-sidebar);
  border: 1px solid var(--color-border);
  padding: 2px 7px;
  border-radius: var(--radius-sm, 4px);
}

.token-pill {
  color: var(--color-primary);
  border-color: color-mix(in srgb, var(--color-primary) 25%, var(--color-border));
  background: color-mix(in srgb, var(--color-primary) 5%, transparent);
  font-weight: 600;
}

.duration-pill {
  color: var(--color-text);
  font-weight: 500;
}

.header-actions {
  display: flex;
  align-items: center;
  gap: 6px;
  flex-shrink: 0;
}

.action-fullscreen-btn {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  padding: 3px 8px;
  font-size: 11px;
  font-weight: 500;
  color: var(--color-primary);
  background: color-mix(in srgb, var(--color-primary) 8%, transparent);
  border: 1px solid color-mix(in srgb, var(--color-primary) 25%, transparent);
  border-radius: var(--radius-sm, 4px);
  cursor: pointer;
  transition: all var(--transition-fast);
}

.action-fullscreen-btn:hover {
  background: color-mix(in srgb, var(--color-primary) 15%, transparent);
  border-color: color-mix(in srgb, var(--color-primary) 45%, transparent);
}

/* ─── 导航栏 ─── */
.detail-navigation-bar {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 0 12px;
  height: 38px;
  background: var(--color-bg-sidebar);
  border-bottom: 1px solid var(--color-border);
  flex-shrink: 0;
  overflow-x: auto;
  overflow-y: hidden;
  scrollbar-width: none;
}
.detail-navigation-bar::-webkit-scrollbar {
  display: none;
}

.segmented-control {
  display: inline-flex;
  padding: 2px;
  gap: 2px;
  background: var(--color-bg);
  border: 1px solid var(--color-border);
  border-radius: var(--radius-md);
  flex-shrink: 0;
}

.seg-btn {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  padding: 3px 9px;
  font-size: 11px;
  font-weight: 500;
  color: var(--color-text-muted);
  background: transparent;
  border: none;
  border-radius: calc(var(--radius-md) - 2px);
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
  color: var(--color-text);
  font-weight: 600;
  box-shadow: 0 1px 2px rgba(0, 0, 0, 0.06);
}

.nav-divider {
  width: 1px;
  height: 16px;
  background: var(--color-border);
  margin: 0 2px;
  flex-shrink: 0;
}

.sub-nav-group {
  display: flex;
  align-items: center;
  gap: 4px;
  flex-shrink: 0;
}

.sub-nav-btn {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  padding: 3px 7px;
  font-size: 11px;
  font-weight: 500;
  color: var(--color-text-muted);
  background: transparent;
  border: 1px solid transparent;
  border-radius: var(--radius-sm, 4px);
  cursor: pointer;
  transition: all var(--transition-fast);
  white-space: nowrap;
  flex-shrink: 0;
}

.sub-nav-btn:hover {
  color: var(--color-text);
  background: var(--color-bg-hover);
}

.sub-nav-btn.active {
  color: var(--color-primary);
  background: var(--color-bg);
  border-color: color-mix(in srgb, var(--color-primary) 30%, var(--color-border));
  font-weight: 600;
}

.sub-badge {
  font-size: 9.5px;
  font-family: var(--font-mono);
  background: var(--color-bg-hover);
  color: var(--color-text-muted);
  padding: 0 5px;
  border-radius: 999px;
  line-height: 14px;
  flex-shrink: 0;
}

.sub-nav-btn.active .sub-badge {
  background: color-mix(in srgb, var(--color-primary) 12%, transparent);
  color: var(--color-primary);
}

.nav-spacer {
  flex: 1;
  min-width: 8px;
}

.action-copy-btn {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  padding: 3px 8px;
  font-size: 11px;
  font-weight: 500;
  color: var(--color-text-muted);
  background: var(--color-bg);
  border: 1px solid var(--color-border);
  border-radius: var(--radius-sm, 4px);
  cursor: pointer;
  transition: all var(--transition-fast);
  white-space: nowrap;
  flex-shrink: 0;
}

.action-copy-btn:hover {
  border-color: var(--color-primary);
  color: var(--color-primary);
}

/* ─── 核心视口工作台 ─── */
.detail-viewport {
  flex: 1;
  min-height: 0;
  height: 100%;
  overflow-y: auto;
  position: relative;
  background: var(--color-bg);
}

.editor-container {
  width: 100%;
  height: 100%;
  min-height: 380px;
}
</style>

<script setup lang="ts">
import { computed, ref, watch, onBeforeUnmount } from "vue";
import { t } from "../../i18n";
import { formatDate as formatLocalized } from "../../utils/format";
import { copyToClipboard } from "../../utils/clipboard";
import type {
  SessionTrafficSummary,
  TrafficDetail,
  TrafficSummary,
} from "../../composables/useProxy";
import { sessionLabel, shortSessionId } from "../../composables/useProxy";
import SvgIcon from "../icons/SvgIcon.vue";
import ApiLogRequestDetail, { type DetailSection } from "./ApiLogRequestDetail.vue";

const props = defineProps<{
  sessions: SessionTrafficSummary[];
  sessionsTotal: number;
  expandedSessionId: string | null | undefined;
  expandedSessionTraffic: TrafficSummary[];
  expandedSessionTotal: number;
  expandedId: string | null;
  expandedDetail: TrafficDetail | null;
  detailLoading: boolean;
  detailTab: "request" | "response";
  detailSection: DetailSection;
  detailBodyText: string;
  detailBodyLanguage: string;
  detailHeadersText: string;
  detailHeadersLanguage: string;
  detailHasTools: boolean;
  detailHasSkills: boolean;
  detailIsSse: boolean;
  copyState: "idle" | "copied";
  sessionLoading: boolean;
  loading: boolean;
  sessionSearch: string;
  getDetailFn?: (id: string) => Promise<TrafficDetail | null>;
}>();

const emit = defineEmits<{
  toggleSession: [session: SessionTrafficSummary];
  toggleExpand: [id: string];
  loadMore: [];
  "update:detailTab": [tab: "request" | "response"];
  "update:detailSection": [section: DetailSection];
  "update:sessionSearch": [value: string];
  copied: [label: string];
  resetCopyState: [];
  copyDetail: [];
  toggleFullscreen: [];
}>();

const hasMoreSessionTraffic = computed(
  () => props.expandedSessionTraffic.length < props.expandedSessionTotal,
);

const activeSummary = computed(() => {
  return props.expandedSessionTraffic.find((t) => t.id === props.expandedId) ?? null;
});

const currentSession = computed(() => {
  if (!props.expandedSessionId) return null;
  return props.sessions.find((s) => s.session_id === props.expandedSessionId) ?? null;
});

// 会话名与「点击复制 ID」提示都在 computed 里求值：写成模块级常量会把语言冻结在 import 那一刻。
// 兜底仍只在「该会话不在已加载的列表里」时生效（列表里一定带得出标签）。
const currentSessionName = computed(() =>
  currentSession.value
    ? sessionLabel(currentSession.value.display_name)
    : t("api-log.trafficPanel.sessionFallbackName"),
);

const sessionNavTitle = computed(() =>
  props.expandedSessionId
    ? t("api-log.trafficPanel.sessionTitleWithId", {
        name: currentSessionName.value,
        id: props.expandedSessionId,
      })
    : currentSessionName.value,
);

// ─── 侧边栏宽度记忆与拖拽逻辑 ───
const SIDEBAR_WIDTH_KEY = "seshbuddy-api-log-sidebar-width";
const SIDEBAR_COLLAPSED_KEY = "seshbuddy-api-log-sidebar-collapsed";

const savedWidth = Number(localStorage.getItem(SIDEBAR_WIDTH_KEY));
const sidebarWidth = ref<number>(savedWidth && savedWidth >= 240 && savedWidth <= 600 ? savedWidth : 360);
const isSidebarCollapsed = ref<boolean>(localStorage.getItem(SIDEBAR_COLLAPSED_KEY) === "true");
const isDragging = ref(false);

function toggleSidebar() {
  isSidebarCollapsed.value = !isSidebarCollapsed.value;
  localStorage.setItem(SIDEBAR_COLLAPSED_KEY, String(isSidebarCollapsed.value));
}

let startX = 0;
let startWidth = 360;

function startResize(e: MouseEvent) {
  if (isSidebarCollapsed.value) return;
  isDragging.value = true;
  startX = e.clientX;
  startWidth = sidebarWidth.value;
  document.addEventListener("mousemove", handleResize);
  document.addEventListener("mouseup", stopResize);
  document.body.style.userSelect = "none";
  document.body.style.cursor = "col-resize";
}

function handleResize(e: MouseEvent) {
  if (!isDragging.value) return;
  const delta = e.clientX - startX;
  const newWidth = Math.min(Math.max(startWidth + delta, 240), 600);
  sidebarWidth.value = newWidth;
}

function stopResize() {
  if (!isDragging.value) return;
  isDragging.value = false;
  localStorage.setItem(SIDEBAR_WIDTH_KEY, String(sidebarWidth.value));
  document.removeEventListener("mousemove", handleResize);
  document.removeEventListener("mouseup", stopResize);
  document.body.style.userSelect = "";
  document.body.style.cursor = "";
}

onBeforeUnmount(() => {
  document.removeEventListener("mousemove", handleResize);
  document.removeEventListener("mouseup", stopResize);
});

function formatTime(ts: string): string {
  try {
    return formatLocalized(ts, { hour: "2-digit", minute: "2-digit", second: "2-digit", hour12: false });
  } catch {
    return ts;
  }
}

function formatDate(ts: string): string {
  try {
    return formatLocalized(ts, {
      month: "short",
      day: "numeric",
      hour: "2-digit",
      minute: "2-digit",
      hour12: false,
    });
  } catch {
    return ts;
  }
}

function formatDuration(ms: number): string {
  if (ms < 1000) return `${ms}ms`;
  return `${(ms / 1000).toFixed(1)}s`;
}

function formatTokens(n: number | null): string {
  if (n == null) return "—";
  if (n < 1000) return `${n}`;
  return `${(n / 1000).toFixed(1)}k`;
}

function statusColor(code: number | null): string {
  if (!code) return "var(--color-text-muted)";
  if (code >= 200 && code < 300) return "var(--color-success, #22c55e)";
  if (code >= 400) return "var(--color-danger, #dc2626)";
  return "var(--color-warning, #f59e0b)";
}

function shortPath(path: string): string {
  if (path.length <= 36) return path;
  return path.slice(0, 16) + "..." + path.slice(-16);
}

/**
 * 工具徽标 tooltip：计数用中性措辞（`Tool names: {count}`），分隔符走共享键，
 * 都在渲染期求值——拼接留在函数体里，语言才不会在 import 时被冻结。
 */
function toolsBadgeTitle(item: TrafficSummary): string {
  return t("api-log.trafficPanel.toolsBadgeTitle", {
    count: item.tool_names.length,
    names: item.tool_names.join(t("session.common.listSeparator")),
  });
}

const copiedSessionId = ref<string | null>(null);

async function copySessionId(sessionId: string | null) {
  if (!sessionId) return;
  if (await copyToClipboard(sessionId)) {
    copiedSessionId.value = sessionId;
    emit(
      "copied",
      t("api-log.trafficPanel.copiedSessionId", { id: shortSessionId(sessionId) }),
    );
    setTimeout(() => {
      if (copiedSessionId.value === sessionId) copiedSessionId.value = null;
    }, 1500);
  }
}

function enterSession(sess: SessionTrafficSummary) {
  emit("toggleSession", sess);
}

function exitSession() {
  if (currentSession.value) {
    emit("toggleSession", currentSession.value);
  }
}

// 切换到某个会话且加载出请求列表后，默认选中第一条请求
watch(
  () => props.expandedSessionTraffic,
  (traffic) => {
    if (traffic.length > 0 && !props.expandedId) {
      emit("toggleExpand", traffic[0].id);
    }
  },
  { immediate: true },
);
</script>

<template>
  <div class="split-workbench" :class="{ 'dragging-active': isDragging }">
    <!-- 左侧可伸缩/可折叠侧边栏 (Master Panel) -->
    <div
      class="master-panel"
      :class="{ collapsed: isSidebarCollapsed }"
      :style="{ width: isSidebarCollapsed ? '0px' : `${sidebarWidth}px` }"
    >
      <div v-show="!isSidebarCollapsed" class="master-panel-inner">
        <!-- 视图 1：已进入某个具体会话 (Requests Stream View) -->
        <template v-if="expandedSessionId">
          <!-- 紧凑型单行面包屑返回导航栏（高度 42px 与右侧 Header 严格对齐） -->
          <div class="master-header session-nav-header">
            <button
              class="breadcrumb-back-btn"
              type="button"
              :title="t('api-log.trafficPanel.backToListTitle')"
              @click="exitSession"
            >
              <svg
                class="btn-svg"
                width="14"
                height="14"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                stroke-width="2.2"
                stroke-linecap="round"
                stroke-linejoin="round"
              >
                <line x1="19" y1="12" x2="5" y2="12"></line>
                <polyline points="12 19 5 12 12 5"></polyline>
              </svg>
            </button>
            <div class="session-nav-title-wrap">
              <span
                class="session-nav-title"
                :title="sessionNavTitle"
                @click="copySessionId(expandedSessionId ?? null)"
              >
                {{ currentSessionName }}
              </span>
            </div>
            <span class="session-total-badge">{{
              t('api-log.trafficPanel.sessionTotalCount', { count: expandedSessionTotal })
            }}</span>
            <button
              class="sidebar-toggle-btn"
              type="button"
              :title="t('api-log.trafficPanel.collapseSidebarTitle')"
              @click="toggleSidebar"
            >
              <SvgIcon name="sidebar" :size="13" />
            </button>
          </div>

          <!-- 当前会话内的搜索过滤栏 -->
          <div class="master-filter-bar">
            <div class="search-input-wrap">
              <SvgIcon name="search" :size="13" class="search-icon" />
              <input
                class="master-search-input"
                type="text"
                :placeholder="t('api-log.trafficPanel.searchPlaceholder')"
                :value="sessionSearch"
                @input="emit('update:sessionSearch', ($event.target as HTMLInputElement).value)"
              />
              <button
                v-if="sessionSearch"
                class="clear-search-btn"
                type="button"
                @click="emit('update:sessionSearch', '')"
              >
                <SvgIcon name="x" :size="11" />
              </button>
            </div>
          </div>

          <!-- 针对该会话的请求流列表 -->
          <div class="master-scroll-area">
            <div v-if="sessionLoading && expandedSessionTraffic.length === 0" class="flow-loading-state">
              <span class="mini-spinner" />
              <span>{{ t('api-log.trafficPanel.loadingRequests') }}</span>
            </div>

            <template v-else-if="expandedSessionTraffic.length > 0">
              <div
                v-for="item in expandedSessionTraffic"
                :key="item.id"
                class="request-stream-card"
                :class="{ selected: expandedId === item.id }"
                @click="emit('toggleExpand', item.id)"
              >
                <div class="stream-line-top">
                  <span class="stream-method" :data-method="item.method">{{ item.method }}</span>
                  <span class="stream-path" :title="item.path">{{ shortPath(item.path) }}</span>
                  <span class="stream-status" :style="{ color: statusColor(item.status) }">
                    <span class="status-indicator-dot" :style="{ background: statusColor(item.status) }" />
                    {{ item.status || '200' }}
                  </span>
                </div>

                <div class="stream-line-bottom">
                  <div class="badges-group">
                    <span
                      v-if="item.tool_names.length > 0"
                      class="mini-feature-badge tools-badge"
                      :title="toolsBadgeTitle(item)"
                    >
                      <SvgIcon name="wrench" :size="9" />
                      {{ item.tool_names.length }}
                    </span>
                    <span
                      v-if="item.has_skill_call"
                      class="mini-feature-badge skills-badge"
                      :title="t('api-log.trafficPanel.skillCallBadgeTitle')"
                    >
                      <SvgIcon name="sparkles" :size="9" />
                    </span>
                  </div>

                  <span class="stream-time">{{ formatTime(item.timestamp) }}</span>
                  <span class="stream-duration">{{ formatDuration(item.duration_ms) }}</span>
                  <span
                    v-if="item.input_tokens != null || item.output_tokens != null"
                    class="stream-tokens"
                    :title="t('api-log.trafficPanel.tokenUsageTitle')"
                  >
                    ↑{{ formatTokens(item.input_tokens) }} ↓{{ formatTokens(item.output_tokens) }}
                  </span>
                </div>
              </div>

              <!-- 加载更多 -->
              <div v-if="hasMoreSessionTraffic" class="stream-load-more">
                <button class="load-more-action" :disabled="sessionLoading" @click.stop="emit('loadMore')">
                  {{ sessionLoading ? t('common.timelineView.loading') : t('api-log.trafficPanel.loadMore') }}
                </button>
              </div>
            </template>

            <div v-else class="master-empty">
              {{ t('api-log.trafficPanel.noMatchingRequests') }}
            </div>
          </div>
        </template>

        <!-- 视图 2：会话列表选择视图 (Sessions List View) -->
        <template v-else>
          <div class="master-header">
            <div class="sessions-header-title">
              <SvgIcon name="message-square" :size="14" />
              <span>{{ t('api-log.trafficPanel.allSessions') }}</span>
              <span class="badge-count">{{ sessionsTotal }}</span>
            </div>
            <button
              class="sidebar-toggle-btn"
              type="button"
              :title="t('api-log.trafficPanel.collapseSidebarTitle')"
              @click="toggleSidebar"
            >
              <SvgIcon name="sidebar" :size="13" />
            </button>
          </div>

          <div class="master-scroll-area">
            <!-- 会话卡片列表 -->
            <div
              v-for="sess in sessions"
              :key="sess.session_id ?? '__ungrouped__'"
              class="session-entry-card"
              @click="enterSession(sess)"
            >
              <div class="session-entry-main">
                <div class="session-entry-top">
                  <span class="session-entry-title" :title="sessionLabel(sess.display_name)">{{ sessionLabel(sess.display_name) }}</span>
                  <button
                    v-if="sess.session_id"
                    class="session-id-copy-btn"
                    :title="copiedSessionId === sess.session_id ? t('dialogs.common.copied') : t('api-log.trafficPanel.copyIdTitle', { id: sess.session_id })"
                    type="button"
                    @click.stop="copySessionId(sess.session_id)"
                  >
                    <SvgIcon :name="copiedSessionId === sess.session_id ? 'check' : 'copy'" :size="11" />
                  </button>
                </div>

                <div v-if="sess.project_path" class="session-entry-project" :title="sess.project_path">
                  {{ sess.project_path }}
                </div>

                <div class="session-entry-meta">
                  <span class="session-count-tag">{{
                    t('api-log.trafficPanel.requestCount', { count: sess.request_count })
                  }}</span>
                  <span v-if="sess.error_count" class="session-err-tag">{{
                    t('api-log.trafficPanel.errorCount', { count: sess.error_count })
                  }}</span>
                  <span class="session-time-tag">{{ formatDate(sess.last_timestamp) }}</span>
                </div>
              </div>

              <div class="session-entry-arrow">
                <SvgIcon name="chevron-right" :size="14" />
              </div>
            </div>

            <div v-if="!loading && sessions.length === 0" class="master-empty">
              {{ t('api-log.trafficPanel.noSessions') }}
            </div>
          </div>
        </template>
      </div>
    </div>

    <!-- 可拖拽分割条 (Resize Split Handle) -->
    <div
      v-if="!isSidebarCollapsed"
      class="resize-split-handle"
      :class="{ active: isDragging }"
      :title="t('api-log.trafficPanel.resizeSidebarTitle')"
      @mousedown.prevent="startResize"
    >
      <div class="split-line" />
    </div>

    <!-- 右侧沉浸式详情工作台 (Detail Panel) -->
    <div class="detail-panel">
      <div v-if="detailLoading" class="detail-loading-state">
        <span class="loading-spinner"></span>
        <span>{{ t('api-log.trafficPanel.loadingDetail') }}</span>
      </div>

      <ApiLogRequestDetail
        v-else-if="expandedDetail"
        :detail="expandedDetail"
        :summary="activeSummary"
        :traffic-list="expandedSessionTraffic"
        :traffic-has-more="hasMoreSessionTraffic"
        :get-detail-fn="getDetailFn"
        :detail-tab="detailTab"
        :detail-section="detailSection"
        :body-text="detailBodyText"
        :body-language="detailBodyLanguage"
        :headers-text="detailHeadersText"
        :headers-language="detailHeadersLanguage"
        :has-tools="detailHasTools"
        :has-skills="detailHasSkills"
        :is-sse="detailIsSse"
        :copy-state="copyState"
        :is-sidebar-collapsed="isSidebarCollapsed"
        @update:detail-tab="emit('update:detailTab', $event)"
        @update:detail-section="emit('update:detailSection', $event)"
        @copy-detail="emit('copyDetail')"
        @reset-copy-state="emit('resetCopyState')"
        @toggle-fullscreen="emit('toggleFullscreen')"
        @toggle-sidebar="toggleSidebar"
        @copied="emit('copied', $event)"
      />

      <!-- 空白引导态 -->
      <div v-else class="detail-empty-state">
        <!-- 侧栏收起时如果右侧没有选中的详情，依然展示内联唤出按钮 -->
        <button
          v-if="isSidebarCollapsed"
          class="empty-state-open-sidebar-btn"
          type="button"
          @click="toggleSidebar"
        >
          <SvgIcon name="sidebar" :size="14" />
          <span>{{ t('api-log.trafficPanel.expandRequestList') }}</span>
        </button>

        <div class="empty-guide-card">
          <div class="empty-icon-wrap">
            <SvgIcon name="zap" :size="32" />
          </div>
          <div class="empty-guide-title">
            {{ expandedSessionId
              ? t('api-log.trafficPanel.emptySelectRequest')
              : t('api-log.trafficPanel.emptySelectSession') }}
          </div>
          <div class="empty-guide-desc">
            {{ expandedSessionId
              ? t('api-log.trafficPanel.emptySelectRequestDesc')
              : t('api-log.trafficPanel.emptySelectSessionDesc') }}
          </div>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.split-workbench {
  display: flex;
  height: 100%;
  width: 100%;
  min-height: 0;
  background: var(--color-bg);
  overflow: hidden;
  border-radius: var(--radius-lg);
  border: 1px solid var(--color-border);
  position: relative;
}

.split-workbench.dragging-active {
  user-select: none;
}

/* ─── 左侧 Master 侧边栏 ─── */
.master-panel {
  height: 100%;
  display: flex;
  flex-direction: column;
  background: var(--color-bg-sidebar);
  flex-shrink: 0;
  overflow: hidden;
  position: relative;
  will-change: width;
}

.master-panel.collapsed {
  width: 0 !important;
  border-right: none;
}

.master-panel-inner {
  width: 100%;
  height: 100%;
  display: flex;
  flex-direction: column;
  min-width: 240px;
}

/* ─── 头部面包屑与标题（统一 42px 高度与右侧精准对齐） ─── */
.master-header {
  padding: 0 12px;
  height: 42px;
  border-bottom: 1px solid var(--color-border);
  background: var(--color-bg);
  flex-shrink: 0;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
}

.sessions-header-title {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 12px;
  font-weight: 600;
  color: var(--color-text);
  flex: 1;
}

.session-nav-header {
  display: flex;
  align-items: center;
  justify-content: flex-start;
  gap: 6px;
}

.breadcrumb-back-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 26px;
  height: 26px;
  border-radius: var(--radius-sm, 4px);
  border: 1px solid var(--color-border);
  background: var(--color-bg-sidebar);
  color: var(--color-text);
  cursor: pointer;
  flex-shrink: 0;
  transition: all var(--transition-fast);
}

.breadcrumb-back-btn:hover {
  color: var(--color-primary);
  border-color: var(--color-primary);
  background: color-mix(in srgb, var(--color-primary) 10%, var(--color-bg));
}

.btn-svg {
  display: block;
}

.session-nav-title-wrap {
  flex: 1;
  min-width: 0;
  display: flex;
  align-items: center;
}

.session-nav-title {
  font-size: 12px;
  font-weight: 600;
  color: var(--color-text);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  cursor: pointer;
  transition: color var(--transition-fast);
}

.session-nav-title:hover {
  color: var(--color-primary);
}

.session-total-badge {
  font-size: 10px;
  font-family: var(--font-mono);
  color: var(--color-text-muted);
  background: var(--color-bg-hover);
  padding: 1px 6px;
  border-radius: 999px;
  flex-shrink: 0;
}

.sidebar-toggle-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 24px;
  height: 24px;
  border-radius: var(--radius-sm, 4px);
  border: none;
  background: transparent;
  color: var(--color-text-muted);
  cursor: pointer;
  flex-shrink: 0;
  transition: all var(--transition-fast);
}

.sidebar-toggle-btn:hover {
  color: var(--color-text);
  background: var(--color-bg-hover);
}

/* ─── 搜索过滤 ─── */
.master-filter-bar {
  padding: 6px 10px;
  border-bottom: 1px solid var(--color-border);
  background: var(--color-bg);
  flex-shrink: 0;
}

.search-input-wrap {
  position: relative;
  display: flex;
  align-items: center;
}

.search-icon {
  position: absolute;
  left: 8px;
  color: var(--color-text-muted);
  pointer-events: none;
}

.master-search-input {
  width: 100%;
  height: 26px;
  padding: 0 24px 0 26px;
  font-size: 11px;
  color: var(--color-text);
  background: var(--color-bg-sidebar);
  border: 1px solid var(--color-border);
  border-radius: var(--radius-sm, 4px);
  outline: none;
  transition: all var(--transition-fast);
}

.master-search-input:focus {
  border-color: var(--color-primary);
  background: var(--color-bg);
}

.clear-search-btn {
  position: absolute;
  right: 5px;
  width: 16px;
  height: 16px;
  border-radius: 50%;
  border: none;
  background: var(--color-bg-hover);
  color: var(--color-text-muted);
  display: flex;
  align-items: center;
  justify-content: center;
  cursor: pointer;
}

.master-scroll-area {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  padding: 8px 10px;
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.section-title {
  display: flex;
  align-items: center;
  justify-content: space-between;
  font-size: 10.5px;
  font-weight: 700;
  color: var(--color-text-muted);
  text-transform: uppercase;
  letter-spacing: 0.06em;
  padding: 4px 6px 2px;
}

.badge-count {
  font-family: var(--font-mono);
  font-size: 10px;
  background: var(--color-bg-hover);
  padding: 1px 6px;
  border-radius: 999px;
}

/* ─── 会话选择卡片 (Session Entry Card) ─── */
.session-entry-card {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 10px 12px;
  border: 1px solid var(--color-border);
  border-radius: var(--radius-md);
  background: var(--color-bg);
  cursor: pointer;
  transition: all var(--transition-fast);
}

.session-entry-card:hover {
  border-color: var(--color-primary);
  background: color-mix(in srgb, var(--color-primary) 3%, var(--color-bg));
  box-shadow: 0 1px 4px rgba(0, 0, 0, 0.04);
}

.session-entry-main {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.session-entry-top {
  display: flex;
  align-items: center;
  gap: 6px;
}

.session-entry-title {
  font-size: 12.5px;
  font-weight: 600;
  color: var(--color-text);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  flex: 1;
  min-width: 0;
}

.session-id-copy-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border: none;
  background: transparent;
  color: var(--color-text-muted);
  padding: 2px;
  border-radius: var(--radius-sm, 4px);
  cursor: pointer;
  opacity: 0.6;
}

.session-id-copy-btn:hover {
  opacity: 1;
  color: var(--color-primary);
  background: var(--color-bg-hover);
}

.session-entry-project {
  font-size: 10.5px;
  font-family: var(--font-mono);
  color: var(--color-text-muted);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.session-entry-meta {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 10.5px;
  font-family: var(--font-mono);
}

.session-count-tag {
  color: var(--color-primary);
  font-weight: 600;
}

.session-err-tag {
  color: var(--color-danger, #ef4444);
  font-weight: 600;
}

.session-time-tag {
  color: var(--color-text-muted);
  margin-left: auto;
}

.session-entry-arrow {
  color: var(--color-text-muted);
  flex-shrink: 0;
  transition: transform var(--transition-fast);
}

.session-entry-card:hover .session-entry-arrow {
  transform: translateX(2px);
  color: var(--color-primary);
}

/* ─── 请求流单项 (Request Stream Card) ─── */
.request-stream-card {
  display: flex;
  flex-direction: column;
  gap: 4px;
  padding: 8px 10px;
  border-radius: var(--radius-md);
  border: 1px solid var(--color-border);
  background: var(--color-bg);
  cursor: pointer;
  transition: all var(--transition-fast);
}

.request-stream-card:hover {
  border-color: color-mix(in srgb, var(--color-primary) 40%, var(--color-border));
  box-shadow: 0 1px 3px rgba(0, 0, 0, 0.04);
}

.request-stream-card.selected {
  border-color: var(--color-primary);
  background: color-mix(in srgb, var(--color-primary) 6%, var(--color-bg));
  box-shadow: 0 0 0 1px var(--color-primary);
}

.stream-line-top {
  display: flex;
  align-items: center;
  gap: 6px;
}

.stream-method {
  font-family: var(--font-mono);
  font-size: 9.5px;
  font-weight: 700;
  padding: 1px 5px;
  border-radius: 3px;
  background: color-mix(in srgb, var(--color-primary) 12%, transparent);
  color: var(--color-primary);
  flex-shrink: 0;
}

.stream-path {
  font-family: var(--font-mono);
  font-size: 11px;
  color: var(--color-text);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  flex: 1;
  min-width: 0;
}

.stream-status {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  font-family: var(--font-mono);
  font-size: 10px;
  font-weight: 700;
  flex-shrink: 0;
}

.status-indicator-dot {
  width: 5px;
  height: 5px;
  border-radius: 50%;
}

.stream-line-bottom {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 10px;
  font-family: var(--font-mono);
  color: var(--color-text-muted);
}

.badges-group {
  display: flex;
  align-items: center;
  gap: 4px;
  flex: 1;
}

.mini-feature-badge {
  display: inline-flex;
  align-items: center;
  gap: 2px;
  font-size: 9px;
  padding: 1px 4px;
  border-radius: 3px;
}

.tools-badge {
  background: color-mix(in srgb, var(--color-warning, #f59e0b) 12%, transparent);
  color: var(--color-warning, #f59e0b);
}

.skills-badge {
  background: color-mix(in srgb, var(--color-primary) 12%, transparent);
  color: var(--color-primary);
}

.stream-tokens {
  color: var(--color-primary);
  font-weight: 600;
}

.stream-load-more {
  text-align: center;
  padding: 4px 0;
}

.load-more-action {
  width: 100%;
  padding: 6px;
  font-size: 11px;
  background: var(--color-bg);
  border: 1px dashed var(--color-border);
  border-radius: var(--radius-sm, 4px);
  color: var(--color-text-muted);
  cursor: pointer;
  transition: all var(--transition-fast);
}

.load-more-action:hover {
  border-color: var(--color-primary);
  color: var(--color-primary);
}

/* ─── 可拖拽分割条 (Resize Split Handle) ─── */
.resize-split-handle {
  width: 6px;
  margin-left: -3px;
  margin-right: -3px;
  z-index: 10;
  cursor: col-resize;
  display: flex;
  align-items: center;
  justify-content: center;
  transition: background var(--transition-fast);
}

.split-line {
  width: 1px;
  height: 100%;
  background: var(--color-border);
  transition: all var(--transition-fast);
}

.resize-split-handle:hover .split-line,
.resize-split-handle.active .split-line {
  width: 2px;
  background: var(--color-primary);
}

/* ─── 右侧 Detail 工作台 ─── */
.detail-panel {
  flex: 1;
  min-width: 0;
  height: 100%;
  display: flex;
  flex-direction: column;
  background: var(--color-bg);
  position: relative;
}

.empty-state-open-sidebar-btn {
  position: absolute;
  top: 10px;
  left: 14px;
  display: inline-flex;
  align-items: center;
  gap: 6px;
  padding: 4px 10px;
  font-size: 11px;
  font-weight: 500;
  color: var(--color-text);
  background: var(--color-bg);
  border: 1px solid var(--color-border);
  border-radius: var(--radius-sm, 4px);
  cursor: pointer;
}

.empty-state-open-sidebar-btn:hover {
  border-color: var(--color-primary);
  color: var(--color-primary);
}

.detail-loading-state {
  display: flex;
  align-items: center;
  justify-content: center;
  height: 100%;
  gap: 8px;
  font-size: var(--text-xs);
  color: var(--color-text-muted);
}

.loading-spinner {
  width: 16px;
  height: 16px;
  border: 2px solid var(--color-border);
  border-top-color: var(--color-primary);
  border-radius: 50%;
  animation: spin 0.8s linear infinite;
}

@keyframes spin {
  to { transform: rotate(360deg); }
}

.detail-empty-state {
  display: flex;
  align-items: center;
  justify-content: center;
  height: 100%;
  padding: 32px;
}

.empty-guide-card {
  max-width: 360px;
  text-align: center;
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 10px;
}

.empty-icon-wrap {
  width: 56px;
  height: 56px;
  border-radius: 50%;
  background: color-mix(in srgb, var(--color-primary) 8%, transparent);
  color: var(--color-primary);
  display: flex;
  align-items: center;
  justify-content: center;
}

.empty-guide-title {
  font-size: 14px;
  font-weight: 600;
  color: var(--color-text);
}

.empty-guide-desc {
  font-size: 12px;
  color: var(--color-text-muted);
  line-height: 1.6;
}

/* ─── Loading / Empty ─── */
.flow-loading-state {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 6px;
  padding: 24px;
  font-size: 11px;
  color: var(--color-text-muted);
}

.mini-spinner {
  width: 12px;
  height: 12px;
  border: 2px solid var(--color-border);
  border-top-color: var(--color-primary);
  border-radius: 50%;
  animation: spin 0.8s linear infinite;
}

.master-empty {
  text-align: center;
  padding: 32px 12px;
  font-size: 12px;
  color: var(--color-text-muted);
}
</style>

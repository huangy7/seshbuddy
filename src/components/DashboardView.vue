<script lang="ts">
import type { CliId } from "../types/cli";

export type DashboardMode = "assistant" | "search";

/** Dashboard 最近对话条目（由 App.vue 按 visibleCliIds 聚合排序后传入） */
export interface DashboardRecentConversation {
  cliId: CliId;
  filePath: string;
  encodedDir: string;
  /** `null` = 后端解析不出项目路径，展示处用 `projectLabel` 的本地化兜底 */
  projectPath: string | null;
  title: string;
  timestamp: string;
}

</script>

<script setup lang="ts">
import { computed, nextTick, onMounted, ref } from "vue";
import { invokeApp, renderAppError, type AppErrorPayload } from "../utils/invokeApp";
import SvgIcon from "./icons/SvgIcon.vue";
import ChatAvatar from "./chat/ChatAvatar.vue";
import { t } from "../i18n";
import type { SearchResult, SessionIdentity } from "../types/session";
import { isCliId, getCliDefinition } from "../types/cli";
import { formatRelativeTime } from "../utils/format";
import { matchedFieldLabel } from "../utils/matchedField";
import { projectLabel } from "../utils/projectPath";

const props = withDefaults(defineProps<{
  bootstrapping?: boolean;
  dataSourcePath?: string;
  showUsage?: boolean;
  showApiDebug?: boolean;
  showNewSession?: boolean;
  recentConversations?: DashboardRecentConversation[];
  activeAgentCount?: number;
  waitingAgentCount?: number;
  /** 已提交的搜索关键词（非空即展示内联结果区）；数据链路在 App.vue 复用 useSessions.globalSearch */
  searchQuery?: string;
  searchResults?: SearchResult[];
  searchLoading?: boolean;
  searchPendingCliIds?: string[];
  searchStaleCliIds?: string[];
  searchCliErrors?: Record<string, AppErrorPayload>;
  buildingCliIds?: string[];
}>(), {
  bootstrapping: false,
  dataSourcePath: "",
  showUsage: false,
  showApiDebug: false,
  showNewSession: true,
  recentConversations: () => [],
  activeAgentCount: 0,
  waitingAgentCount: 0,
  searchQuery: "",
  searchResults: () => [],
  searchLoading: false,
  searchPendingCliIds: () => [],
  searchStaleCliIds: () => [],
  searchCliErrors: () => ({}),
  buildingCliIds: () => [],
});

const emit = defineEmits<{
  submitAssistant: [prompt: string];
  submitSearch: [query: string];
  openSession: [identity: SessionIdentity, encodedDir: string];
  openSearchResult: [result: SearchResult];
  openFullSearch: [query: string];
  clearSearch: [];
  buildPendingSearchIndex: [cliId: string];
  buildAllPendingSearchIndexes: [cliIds: string[]];
  newSession: [];
  openHistory: [];
  openAgent: [];
  openAssistant: [];
  openUsage: [];
  openApiDebug: [];
}>();

const mode = ref<DashboardMode>("assistant");
const draft = ref("");
const promptInputRef = ref<HTMLInputElement | null>(null);
const userName = ref("");

const isMac = typeof navigator !== "undefined" && navigator.platform.toUpperCase().includes("MAC");
const searchShortcutText = isMac ? "⌘ ⇧ F" : "Ctrl + Shift + F";

function focusSearch() {
  mode.value = "search";
  nextTick(() => promptInputRef.value?.focus());
  emit("openFullSearch", draft.value);
}

function onSelectSearchMode() {
  mode.value = "search";
  emit("openFullSearch", draft.value);
}

function onPromptInputClick() {
  if (mode.value === "search") {
    emit("openFullSearch", draft.value);
  }
}

defineExpose({ focusSearch });

const cliNameOf = (id: string) => (isCliId(id) ? getCliDefinition(id).name : id);
const searchCliErrorList = computed(() => Object.entries(props.searchCliErrors || {}));
const isAnyPendingBuilding = computed(() =>
  (props.searchPendingCliIds || []).some((id) => props.buildingCliIds?.includes(id))
);
const isAnyStaleBuilding = computed(() =>
  (props.searchStaleCliIds || []).some((id) => props.buildingCliIds?.includes(id))
);

const placeholder = computed(() =>
  mode.value === "assistant"
    ? t("session.dashboard.placeholderAssistant")
    : t("session.dashboard.placeholderSearch"),
);

function onSubmit() {
  const text = draft.value.trim();
  if (!text) return;
  if (mode.value === "assistant") {
    emit("submitAssistant", text);
  } else {
    emit("submitSearch", text);
  }
  draft.value = "";
}

function onOpenRecent(item: DashboardRecentConversation) {
  emit("openSession", { cliId: item.cliId, filePath: item.filePath }, item.encodedDir);
}

const hasAgentStatus = computed(() => props.activeAgentCount > 0 || props.waitingAgentCount > 0);

// ─── 用户名 ───

async function loadUserName() {
  try {
    const name = await invokeApp<string>("get_current_user_name");
    if (name && name.trim()) {
      userName.value = name.trim();
    }
  } catch {
    // 非 Tauri 环境（单元测试）下静默降级为不带用户名的问候语
  }
}

const greetingPrefix = computed(() => {
  const hour = new Date().getHours();
  if (hour >= 5 && hour < 12) return t("session.dashboard.greetingMorning");
  if (hour >= 12 && hour < 18) return t("session.dashboard.greetingAfternoon");
  return t("session.dashboard.greetingEvening");
});

const greetingText = computed(() => {
  const greeting = greetingPrefix.value;
  const name = userName.value.trim();
  if (name) {
    return t("session.dashboard.greetingWithName", { greeting, name });
  }
  return t("session.dashboard.greeting", { greeting });
});

onMounted(async () => {
  void loadUserName();
});
</script>

<template>
  <div class="dashboard">
    <div class="dashboard-inner">
      <Transition name="banner-slide">
        <div v-if="bootstrapping" class="startup-banner">
          <span class="spinner" />
          <div class="banner-text">
            <strong>{{ t("session.dashboard.bootstrappingTitle") }}</strong>
            <span>{{ t("session.dashboard.bootstrappingHint") }}</span>
          </div>
        </div>
      </Transition>

      <header class="hero">
        <div class="hero-eyebrow">
          <span>{{ greetingText }}</span>
        </div>
        <h1 class="hero-title">{{ t("session.dashboard.heroTitle") }}</h1>
        <p class="hero-sub">{{ t("session.dashboard.heroSub") }}</p>
      </header>

      <div class="mode-switch" role="tablist" :aria-label="t('session.dashboard.inputModeAria')">
        <button
          type="button"
          role="tab"
          class="mode-btn"
          :class="{ active: mode === 'assistant' }"
          :aria-selected="mode === 'assistant'"
          @click="mode = 'assistant'"
        >
          {{ t("session.dashboard.modeAssistant") }}
        </button>
        <button
          type="button"
          role="tab"
          class="mode-btn"
          :class="{ active: mode === 'search' }"
          :aria-selected="mode === 'search'"
          @click="onSelectSearchMode"
        >
          {{ t("session.dashboard.modeSearch") }}
        </button>
      </div>

      <form class="prompt-box" @submit.prevent="onSubmit">
        <input
          ref="promptInputRef"
          v-model="draft"
          class="prompt-input"
          type="text"
          :placeholder="placeholder"
          autocomplete="off"
          autocapitalize="off"
          autocorrect="off"
          spellcheck="false"
          @click="onPromptInputClick"
        />
        <button type="submit" class="prompt-send" :disabled="!draft.trim()" :aria-label="t('session.dashboard.send')">↑</button>
      </form>
      <div class="prompt-hint">
        <span>{{ t("session.dashboard.enterToSend") }}</span>
        <span class="prompt-hint-divider">·</span>
        <span>{{ searchShortcutText }} {{ t("session.dashboard.modeSearch") }}</span>
      </div>

      <section v-if="searchQuery" class="search-inline" :aria-label="t('session.dashboard.searchResultsAria')">
        <div class="search-inline-header">
          <h2 class="section-title">{{ t("session.dashboard.searchResultsTitle", { query: searchQuery }) }}</h2>
          <div class="search-inline-actions">
            <button type="button" class="search-full-link" @click="emit('openFullSearch', searchQuery)">
              {{ t("session.dashboard.viewInFullSearch") }}
            </button>
            <button type="button" class="search-clear-btn" @click="emit('clearSearch')">{{ t("session.projectPanel.clear") }}</button>
          </div>
        </div>
        <div v-if="searchLoading" class="search-inline-status">{{ t("session.dashboard.searching") }}</div>
        <div v-else-if="searchResults.length === 0" class="search-inline-status">
          {{ t("session.dashboard.searchEmpty") }}
        </div>
        <ul v-else class="search-inline-list">
          <li v-for="result in searchResults" :key="`${result.cli_id}:${result.file_path}`">
            <button type="button" class="search-result-row" @click="emit('openSearchResult', result)">
              <ChatAvatar role="assistant" :cliId="result.cli_id" class="search-result-cli" />
              <span class="search-result-main">
                <span class="search-result-title">
                  {{ result.display_name }}
                  <small class="search-result-badge">{{ cliNameOf(result.cli_id) }}</small>
                </span>
                <span class="search-result-meta">
                  {{ projectLabel(result.project_path) }} · {{ t("session.dashboard.matchCount", { count: result.match_count }) }}
                </span>
                <span class="search-result-snippet">{{ matchedFieldLabel(result.matched_field) }}{{ result.snippet }}</span>
              </span>
            </button>
          </li>
        </ul>

        <!-- 未索引 CLI 提示与一键构建 -->
        <div v-if="searchPendingCliIds.length && !searchLoading" class="search-inline-hint pending-cli-hint">
          <div class="hint-text-wrap">
            <SvgIcon name="info" :size="14" class="hint-icon" />
            <span>{{ t("session.dashboard.pendingIndexHint", { names: searchPendingCliIds.map(cliNameOf).join(t("session.common.listSeparator")) }) }}</span>
          </div>
          <div class="pending-cli-actions">
            <button
              v-if="searchPendingCliIds.length > 1"
              type="button"
              class="index-btn primary"
              :disabled="isAnyPendingBuilding"
              @click="emit('buildAllPendingSearchIndexes', searchPendingCliIds)"
            >
              {{ t("session.dashboard.buildAll", { count: searchPendingCliIds.length }) }}
            </button>
            <button
              v-for="id in searchPendingCliIds"
              :key="id"
              type="button"
              class="index-btn secondary"
              :disabled="buildingCliIds?.includes(id)"
              @click="emit('buildPendingSearchIndex', id)"
            >
              {{ buildingCliIds?.includes(id) ? t("session.dashboard.buildingCli", { name: cliNameOf(id) }) : searchPendingCliIds.length > 1 ? t("session.dashboard.onlyCli", { name: cliNameOf(id) }) : t("session.dashboard.buildCliIndex", { name: cliNameOf(id) }) }}
            </button>
          </div>
        </div>

        <!-- 索引过期提示与一键更新 -->
        <div v-else-if="searchStaleCliIds.length && !searchLoading" class="search-inline-hint pending-cli-hint">
          <div class="hint-text-wrap">
            <SvgIcon name="info" :size="14" class="hint-icon" />
            <span>{{ t("session.dashboard.staleIndexHint", { names: searchStaleCliIds.map(cliNameOf).join(t("session.common.listSeparator")) }) }}</span>
          </div>
          <div class="pending-cli-actions">
            <button
              v-if="searchStaleCliIds.length > 1"
              type="button"
              class="index-btn primary"
              :disabled="isAnyStaleBuilding"
              @click="emit('buildAllPendingSearchIndexes', searchStaleCliIds)"
            >
              {{ t("session.dashboard.updateAll", { count: searchStaleCliIds.length }) }}
            </button>
            <button
              v-for="id in searchStaleCliIds"
              :key="id"
              type="button"
              class="index-btn secondary"
              :disabled="buildingCliIds?.includes(id)"
              @click="emit('buildPendingSearchIndex', id)"
            >
              {{ buildingCliIds?.includes(id) ? t("session.dashboard.updatingCli", { name: cliNameOf(id) }) : searchStaleCliIds.length > 1 ? t("session.dashboard.onlyCli", { name: cliNameOf(id) }) : t("session.dashboard.updateCliIndex", { name: cliNameOf(id) }) }}
            </button>
          </div>
        </div>

        <!-- 搜索错误提示 -->
        <div v-if="searchCliErrorList.length && !searchLoading" class="search-inline-hint search-cli-error-hint">
          <span v-for="[id, errorMessage] in searchCliErrorList" :key="id">
            {{ t("session.dashboard.searchFailed", { name: cliNameOf(id), message: renderAppError(errorMessage) }) }}
          </span>
        </div>
      </section>

      <!-- 2. 最近对话全宽列表 -->
      <section v-if="recentConversations.length > 0" class="recent-section" :aria-label="t('session.dashboard.recentConversations')">
        <h2 class="section-title">{{ t("session.dashboard.recentConversations") }}</h2>
        <ul class="recent-list">
          <li v-for="item in recentConversations" :key="`${item.cliId}:${item.filePath}`">
            <button type="button" class="recent-row" @click="onOpenRecent(item)">
              <ChatAvatar role="assistant" :cliId="item.cliId" class="recent-cli" />
              <span class="recent-main">
                <span class="recent-title">{{ item.title }}</span>
                <span class="recent-meta">{{ projectLabel(item.projectPath) }}</span>
              </span>
              <span class="recent-time">{{ formatRelativeTime(item.timestamp) }}</span>
            </button>
          </li>
        </ul>
      </section>

      <div v-if="hasAgentStatus" class="agent-status-line">
        <span>
          {{ t("session.dashboard.agentRunning", { count: activeAgentCount }) }}<template v-if="waitingAgentCount > 0"> · {{ t("session.agentDashboard.waitingCount", { count: waitingAgentCount }) }}</template>
        </span>
        <button type="button" class="agent-status-link" @click="emit('openAgent')">{{ t("session.dashboard.view") }}</button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.dashboard {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  display: flex;
  justify-content: center;
  padding: 40px var(--space-6) 60px;
}
.dashboard-inner {
  width: 100%;
  max-width: 790px;
  min-height: 100%;
  display: flex;
  flex-direction: column;
}

.banner-slide-enter-active,
.banner-slide-leave-active {
  transition: opacity 180ms ease, transform 200ms ease, max-height 200ms ease;
  overflow: hidden;
}
.banner-slide-enter-from,
.banner-slide-leave-to { opacity: 0; transform: translateY(-6px); max-height: 0; }
.banner-slide-enter-to,
.banner-slide-leave-from { opacity: 1; transform: translateY(0); max-height: 80px; }
.startup-banner {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  padding: var(--space-3) var(--space-4);
  border: 1px solid var(--color-border);
  border-radius: var(--radius-xl);
  background: var(--color-bg-secondary);
  margin-bottom: var(--space-4);
}
.spinner {
  width: 15px;
  height: 15px;
  border-radius: var(--radius-full);
  border: 2px solid var(--color-border);
  border-top-color: var(--color-primary);
  animation: spin 0.8s linear infinite;
  flex-shrink: 0;
}
.banner-text {
  display: flex;
  flex-direction: column;
  gap: 2px;
  font-size: var(--text-xs);
  color: var(--color-text-secondary);
}
.banner-text strong { color: var(--color-text); font-size: var(--text-sm); }

.hero {
  margin-top: 30px;
}
.hero-eyebrow {
  display: flex;
  align-items: center;
  font-size: 14px;
  color: var(--color-text-secondary, #65758f);
  margin-bottom: 0;
}
.hero-title {
  margin: 16px 0 8px;
  font-family: var(--font-sans);
  font-size: 34px;
  line-height: 1.2;
  font-weight: 700;
  letter-spacing: -0.03em;
  color: var(--color-text, #172033);
}
.hero-sub {
  margin: 0;
  font-size: 15px;
  color: var(--color-text-muted, #76839a);
  line-height: 1.5;
}

.mode-switch {
  display: flex;
  gap: 5px;
  background: var(--color-bg-secondary, #f3f5f8);
  width: max-content;
  padding: 4px;
  border-radius: 10px;
  margin-top: 28px;
  border: 1px solid var(--color-border-light, #e7ebf1);
}
.mode-btn {
  border: 0;
  background: transparent;
  padding: 7px 15px;
  border-radius: 7px;
  color: var(--color-text-muted, #69768b);
  font-size: 13px;
  font-weight: 500;
  cursor: pointer;
  transition: background var(--transition-fast), color var(--transition-fast), box-shadow var(--transition-fast);
}
.mode-btn:hover {
  color: var(--color-text, #1d2a40);
}
.mode-btn.active {
  background: var(--color-bg, #fff);
  color: var(--color-text, #1d2a40);
  font-weight: 600;
  box-shadow: 0 1px 3px rgba(36, 52, 80, 0.1);
}

.prompt-box {
  margin-top: 10px;
  border: 1px solid var(--color-border, #dbe2ec);
  border-radius: 14px;
  background: var(--color-bg, #fff);
  padding: 10px 10px 10px 16px;
  display: flex;
  align-items: center;
  gap: var(--space-2);
  box-shadow: 0 10px 30px rgba(44, 65, 96, 0.04);
  transition: border-color var(--transition-fast), box-shadow var(--transition-fast);
}
.prompt-box:focus-within {
  border-color: var(--color-primary);
  box-shadow: 0 0 0 3px var(--color-primary-ring);
}
.prompt-input {
  flex: 1;
  min-width: 0;
  border: none;
  outline: none;
  background: transparent;
  font-size: 14px;
  font-family: var(--font-sans);
  color: var(--color-text, #172033);
}
.prompt-input::placeholder {
  color: var(--color-text-muted, #9aa5b5);
}
.prompt-send {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 32px;
  height: 32px;
  border: 0;
  border-radius: 8px;
  background: var(--color-text, #172033);
  color: var(--color-bg, #fff);
  font-size: 14px;
  font-weight: 700;
  cursor: pointer;
  flex-shrink: 0;
  transition: opacity var(--transition-fast);
}
.prompt-send:disabled {
  opacity: 0.35;
  cursor: default;
}
.prompt-hint {
  display: flex;
  align-items: center;
  gap: 6px;
  margin: 8px 4px 0;
  font-size: 11px;
  color: var(--color-text-muted, #a1acbb);
}
.prompt-hint-divider {
  opacity: 0.5;
}

.search-inline {
  margin-top: var(--space-4);
}
.search-inline-header {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: var(--space-3);
}
.search-inline-actions {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  flex-shrink: 0;
}
.search-full-link,
.search-clear-btn {
  border: 0;
  background: transparent;
  padding: 0;
  font-size: var(--text-xs);
  cursor: pointer;
}
.search-full-link {
  color: var(--color-primary);
}
.search-clear-btn {
  color: var(--color-text-muted);
}
.search-clear-btn:hover {
  color: var(--color-text);
}
.search-inline-status {
  padding: var(--space-4) var(--space-1);
  font-size: var(--text-sm);
  color: var(--color-text-muted);
}
.search-inline-list {
  list-style: none;
  margin: 0;
  padding: 0;
  border-top: 1px solid var(--color-border-light, #ebeff4);
}
.search-result-row {
  display: flex;
  align-items: flex-start;
  gap: var(--space-3);
  width: 100%;
  padding: var(--space-3) var(--space-1);
  border: 0;
  border-bottom: 1px solid var(--color-border-light, #ebeff4);
  border-radius: var(--radius-lg);
  background: transparent;
  text-align: left;
  cursor: pointer;
  transition: background var(--transition-fast);
}
.search-result-row:hover {
  background: var(--color-bg-hover, #f5f7fa);
}
.search-result-cli {
  width: 22px;
  height: 22px;
  flex-shrink: 0;
  margin-top: 1px;
}
.search-result-main {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
}
.search-result-title {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  font-size: var(--text-sm);
  color: var(--color-text);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.search-result-badge {
  flex-shrink: 0;
  padding: 0 var(--space-1);
  border-radius: var(--radius-sm);
  font-size: var(--text-2xs);
  line-height: 16px;
  color: var(--color-text-secondary);
  background: var(--color-bg-secondary);
}
.search-result-meta {
  font-size: var(--text-xs);
  color: var(--color-text-muted);
}
.search-result-snippet {
  font-size: var(--text-xs);
  color: var(--color-text-secondary);
  line-height: var(--leading-normal);
  display: -webkit-box;
  -webkit-line-clamp: 2;
  -webkit-box-orient: vertical;
  overflow: hidden;
}
.search-inline-hint {
  margin-top: var(--space-3);
  padding: var(--space-3) var(--space-4);
  border-radius: var(--radius-lg);
  font-size: var(--text-xs);
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
}
.search-inline-hint.pending-cli-hint {
  background: var(--color-bg-secondary);
  border: 1px dashed var(--color-border);
  color: var(--color-text-secondary);
}
.search-inline-hint.search-cli-error-hint {
  background: var(--color-bg-danger-subtle, rgba(239, 68, 68, 0.1));
  color: var(--color-danger);
  border: 1px solid var(--color-danger-border, rgba(239, 68, 68, 0.2));
}
.hint-text-wrap {
  display: flex;
  align-items: center;
  gap: var(--space-2);
}
.hint-icon {
  flex-shrink: 0;
  color: var(--color-primary);
}
.pending-cli-actions {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--space-2);
  margin-top: var(--space-1);
}
.index-btn {
  height: 26px;
  padding: 0 var(--space-3);
  border-radius: var(--radius-md);
  font-size: var(--text-xs);
  font-weight: 500;
  cursor: pointer;
  border: 1px solid transparent;
  transition: all var(--transition-fast);
}
.index-btn.primary {
  background: var(--color-primary);
  color: var(--color-text-inverse, #fff);
}
.index-btn.primary:hover:not(:disabled) {
  background: var(--color-primary-hover);
}
.index-btn.secondary {
  color: var(--color-text-secondary);
  background: var(--color-bg, #fff);
  border-color: var(--color-border);
}
.index-btn.secondary:hover:not(:disabled) {
  color: var(--color-text);
  background: var(--color-bg-hover);
}
.index-btn:disabled {
  opacity: 0.6;
  cursor: not-allowed;
}

/* ─── 1. 今日 Token 概览横向条 ─── */
.kpi-section {
  margin-top: 28px;
}
.kpi-horizontal-card {
  display: flex;
  align-items: center;
  width: 100%;
  padding: 14px 22px;
  border: 1px solid var(--color-border-light, #e5eaf1);
  border-radius: 14px;
  background: var(--color-bg, #fff);
  box-shadow: 0 1px 3px rgba(36, 52, 80, 0.03);
  cursor: pointer;
  text-align: left;
  transition: border-color var(--transition-fast), box-shadow var(--transition-fast);
}
.kpi-horizontal-card:hover {
  border-color: var(--color-border);
  box-shadow: 0 4px 12px rgba(36, 52, 80, 0.06);
}
.kpi-val {
  font-size: 18px;
  font-weight: 700;
  letter-spacing: -0.02em;
  color: var(--color-text, #172033);
  white-space: nowrap;
}
.kpi-divider {
  width: 1px;
  height: 26px;
  background: var(--color-border-light, #edf0f4);
  margin: 0 20px;
  flex-shrink: 0;
}
.kpi-updated {
  font-size: 11px;
  color: var(--color-text-muted, #a1acbb);
  margin-left: auto;
  padding-left: 12px;
  white-space: nowrap;
  flex-shrink: 0;
}

.kpi-login-body {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
}
.kpi-login-title {
  font-size: 13px;
  font-weight: 600;
  color: var(--color-text, #172033);
}
.kpi-login-desc {
  margin: 0;
  font-size: 12px;
  color: var(--color-text-muted, #76839a);
  line-height: 1.4;
}
.kpi-login-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  padding: 7px 18px;
  border: 1px solid var(--color-border, #dce3ee);
  border-radius: 8px;
  background: var(--color-bg, #fff);
  color: var(--color-text, #172033);
  font-size: 12px;
  font-weight: 500;
  cursor: pointer;
  flex-shrink: 0;
  transition: background var(--transition-fast), border-color var(--transition-fast);
}
.kpi-login-btn:hover {
  background: var(--color-bg-hover, #f0f3f7);
  border-color: var(--color-border-hover, #cbd5e1);
}

/* ─── 2. 最近对话全宽列表 ─── */
.recent-section {
  margin-top: 32px;
}
.section-title {
  font-size: 14px;
  font-weight: 650;
  margin: 0 0 12px;
  color: var(--color-text, #172033);
}
.recent-list {
  list-style: none;
  margin: 0;
  padding: 0;
  border-top: 1px solid var(--color-border-light, #ebeff4);
}
.recent-row {
  display: grid;
  grid-template-columns: 24px 1fr auto;
  gap: 12px;
  align-items: center;
  border: 0;
  border-bottom: 1px solid var(--color-border-light, #ebeff4);
  padding: 13px 4px;
  border-radius: 8px;
  width: 100%;
  background: transparent;
  text-align: left;
  cursor: pointer;
  transition: background var(--transition-fast);
}
.recent-row:hover {
  background: var(--color-bg-hover, #f5f7fa);
}
.recent-cli {
  width: 22px;
  height: 22px;
  flex-shrink: 0;
}
.recent-main {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
}
.recent-title {
  font-size: 13px;
  font-weight: 500;
  color: var(--color-text, #27344a);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.recent-meta {
  font-size: 11px;
  color: var(--color-text-muted, #9aa6b6);
}
.recent-time {
  font-size: 11px;
  color: var(--color-text-muted, #9aa6b6);
  flex-shrink: 0;
  white-space: nowrap;
}

.agent-status-line {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  font-size: var(--text-xs);
  color: var(--color-text-secondary);
  margin-top: var(--space-6);
}
.agent-status-link {
  border: 0;
  background: transparent;
  color: var(--color-primary);
  font-size: var(--text-xs);
  cursor: pointer;
  padding: 0;
}

@media (max-width: 600px) {
  .dashboard {
    padding: var(--space-4);
  }
  .kpi-horizontal-card {
    flex-direction: column;
    align-items: flex-start;
    gap: 12px;
    padding: 14px;
  }
  .kpi-divider {
    display: none;
  }
  .kpi-updated {
    margin-left: 0;
    padding-left: 0;
  }
}
</style>

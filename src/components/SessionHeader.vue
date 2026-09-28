<script setup lang="ts">
import { ref, computed, nextTick, onMounted, onBeforeUnmount } from "vue";
import SvgIcon from "./icons/SvgIcon.vue";
import SessionAnalyticsModal from "./SessionAnalyticsModal.vue";
import ResumeSessionDialog from "./ResumeSessionDialog.vue";
import ExportSessionDialog from "./ExportSessionDialog.vue";
import { formatTimestamp } from "../utils/format";
import { copyToClipboard } from "../utils/clipboard";
import type { SessionStats, BookmarkInfo, ChatMessage } from "../types/session";
import { getCliDefinition, isCliId } from "../types/cli";
import { t } from "../i18n";

const props = defineProps<{
  displayName: string;
  projectPath: string;
  timestamp: string;
  messageCount: number;
  stats: SessionStats | null;
  bookmarks: BookmarkInfo[];
  timelineOpen: boolean;
  archivePinned: boolean;
  archivePinLoading: boolean;
  hasParentSession?: boolean;
  isArchived?: boolean;
  hasArchiveSnapshot?: boolean;
  profiles?: string[];
  cliId?: string;
  messages?: ChatMessage[] | (() => ChatMessage[]);
  isRefreshing?: boolean;
  favorite?: boolean;
  proxyTrafficCount?: number;
}>();

const emit = defineEmits<{
  export: [format: string];
  resume: [profileName: string | null, skipPermissions: boolean];
  resumeInTerminal: [profileName: string | null, skipPermissions: boolean];
  openProject: [];
  scrollToBookmark: [messageIndex: number];
  toggleArchivePinned: [];
  enterSelectionMode: [];
  backToParent: [];
  "update:timelineOpen": [value: boolean];
  copyRestoreCommand: [profileName: string | null, skipPermissions: boolean, cb?: (ok: boolean) => void];
  refresh: [];
  toggleFavorite: [];
  openProxyTraffic: [];
}>();

const profiles = computed(() => props.profiles ?? []);
const currentCliDef = computed(() =>
  props.cliId && isCliId(props.cliId) ? getCliDefinition(props.cliId) : null
);
const supportsResume = computed(() => currentCliDef.value?.supportsResumeSession ?? true);

const showMoreMenu = ref(false);
const moreTriggerRef = ref<HTMLButtonElement | null>(null);
const moreMenuRef = ref<HTMLElement | null>(null);

const showBookmarkMenu = ref(false);
const bookmarkTriggerRef = ref<HTMLButtonElement | null>(null);

function toggleBookmarkMenu() {
  showBookmarkMenu.value = !showBookmarkMenu.value;
  if (showBookmarkMenu.value) {
    showMoreMenu.value = false;
  }
}

function closeBookmarkMenu(opts: { focusTrigger?: boolean } = {}) {
  if (!showBookmarkMenu.value) return;
  showBookmarkMenu.value = false;
  if (opts.focusTrigger) {
    bookmarkTriggerRef.value?.focus();
  }
}

function goToBookmark(messageIndex: number) {
  closeBookmarkMenu();
  emit("scrollToBookmark", messageIndex);
}

const showAnalyticsModal = ref(false);
const showResumeDialog = ref(false);
const showExportDialog = ref(false);

const analyticsMessages = computed<ChatMessage[]>(() => {
  if (typeof props.messages === "function") {
    return props.messages();
  }
  return props.messages ?? [];
});

function moreMenuItems(): HTMLButtonElement[] {
  const el = moreMenuRef.value;
  if (!el) return [];
  return Array.from(el.querySelectorAll<HTMLButtonElement>(".menu-item:not(:disabled)"));
}

async function toggleMoreMenu() {
  showMoreMenu.value = !showMoreMenu.value;
  if (showMoreMenu.value) {
    await nextTick();
    moreMenuItems()[0]?.focus();
  }
}

function closeMenus(opts: { focusTrigger?: boolean } = {}) {
  if (!showMoreMenu.value) return;
  showMoreMenu.value = false;
  if (opts.focusTrigger) {
    moreTriggerRef.value?.focus();
  }
}

function onMoreMenuKeydown(e: KeyboardEvent) {
  if (!showMoreMenu.value) return;
  if (e.key === "Escape") {
    if (showBookmarkMenu.value) {
      e.stopPropagation();
      e.preventDefault();
      closeBookmarkMenu({ focusTrigger: true });
      return;
    }
    if (showMoreMenu.value) {
      e.stopPropagation();
      e.preventDefault();
      closeMenus({ focusTrigger: true });
      return;
    }
  }
  if (e.key === "ArrowDown" || e.key === "ArrowUp") {
    e.preventDefault();
    const items = moreMenuItems();
    if (items.length === 0) return;
    const current = items.indexOf(document.activeElement as HTMLButtonElement);
    const next =
      e.key === "ArrowDown"
        ? (current + 1) % items.length
        : (current - 1 + items.length) % items.length;
    items[next]?.focus();
  }
}

function onDocumentClick(e: MouseEvent) {
  const target = e.target as HTMLElement | null;
  if (showMoreMenu.value && !target?.closest(".more-wrapper")) {
    closeMenus();
  }
  if (showBookmarkMenu.value && !target?.closest(".bookmark-wrapper")) {
    closeBookmarkMenu();
  }
}

onMounted(() => {
  document.addEventListener("click", onDocumentClick);
  document.addEventListener("keydown", onMoreMenuKeydown);
});

onBeforeUnmount(() => {
  document.removeEventListener("click", onDocumentClick);
  document.removeEventListener("keydown", onMoreMenuKeydown);
});

function openResumeDialog() {
  closeMenus();
  if (!supportsResume.value) return;
  if (props.cliId === "workbuddy") {
    emit("resumeInTerminal", null, false);
    return;
  }
  showResumeDialog.value = true;
}

function openExportDialog() {
  closeMenus();
  showExportDialog.value = true;
}

function openAnalyticsModal() {
  closeMenus();
  showAnalyticsModal.value = true;
}

function requestArchivePinned() {
  if (props.archivePinLoading) return;
  closeMenus();
  emit("toggleArchivePinned");
}

function handleEnterSelection() {
  closeMenus();
  emit("enterSelectionMode");
}

function handleBackToParent() {
  closeMenus();
  emit("backToParent");
}

function handleDirectCopyRestoreCommand() {
  closeMenus();
  emit("copyRestoreCommand", null, false);
}

function handleResumeFromDialog(launchMode: "agent" | "terminal", profileName: string | null, skipPermissions: boolean) {
  showResumeDialog.value = false;
  if (launchMode === "agent") {
    emit("resume", profileName, skipPermissions);
  } else {
    emit("resumeInTerminal", profileName, skipPermissions);
  }
}

function handleCopyCommandFromDialog(profileName: string | null, skipPermissions: boolean, cb?: (ok: boolean) => void) {
  emit("copyRestoreCommand", profileName, skipPermissions, (ok: boolean) => {
    if (cb) cb(ok);
  });
}

function handleExportFromDialog(format: string) {
  showExportDialog.value = false;
  emit("export", format);
}

function handleEnterSelectionFromDialog() {
  showExportDialog.value = false;
  emit("enterSelectionMode");
}

const hasAnalytics = computed(() => props.messageCount > 0);

const displayProjectPath = computed(() => {
  return compactPath(props.projectPath);
});

function compactPath(path: string, maxSegments = 3): string {
  if (!path) return "";

  const isUnixAbsolute = path.startsWith("/");
  const isUncPath = path.startsWith("\\\\");

  const normalized = path.replace(/\\/g, "/");
  const segments = normalized.split("/").filter(Boolean);

  if (segments.length <= maxSegments) {
    return path;
  }

  const separator = path.includes("\\") ? "\\" : "/";
  const headCount = Math.ceil(maxSegments / 2);
  const tailCount = Math.floor(maxSegments / 2);
  const compacted = [...segments.slice(0, headCount), "…", ...segments.slice(-tailCount)].join(separator);

  if (isUncPath) {
    return `\\\\${compacted}`;
  }
  if (isUnixAbsolute && separator === "/") {
    return `/${compacted}`;
  }
  return compacted;
}

const pathCopied = ref(false);

async function copyProjectPath() {
  if (!props.projectPath) return;
  if (await copyToClipboard(props.projectPath)) {
    pathCopied.value = true;
    setTimeout(() => {
      pathCopied.value = false;
    }, 1500);
  } else {
    console.error("Failed to copy project path");
  }
}

const localRefreshing = ref(false);
const isRefreshingState = computed(() => !!props.isRefreshing || localRefreshing.value);

function handleRefresh() {
  if (isRefreshingState.value) return;
  localRefreshing.value = true;
  try {
    emit("refresh");
  } finally {
    setTimeout(() => {
      localRefreshing.value = false;
    }, 600);
  }
}
</script>

<template>
  <div class="session-header">
    <div class="header-info">
      <h2 class="session-name" :title="displayName">{{ displayName }}</h2>
      <div class="session-subline">
        <button
          v-if="projectPath"
          class="session-path"
          :title="pathCopied ? t('session.sessionHeader.copiedProjectPath') : t('session.sessionHeader.copyProjectPath', { path: projectPath })"
          type="button"
          @click="copyProjectPath"
        >
          <SvgIcon
            :name="pathCopied ? 'check' : 'folder'"
            :size="12"
            class="session-path-icon"
            :class="{ 'text-success': pathCopied }"
          />
          <span class="session-path-text">{{ displayProjectPath }}</span>
          <span v-if="pathCopied" class="copied-badge">{{ t("dialogs.common.copied") }}</span>
          <SvgIcon v-else name="copy" :size="11" class="copy-hint-icon" />
        </button>
        <span class="session-subline-dot">·</span>
        <div class="session-meta">
          <span class="meta-item">
            {{ formatTimestamp(timestamp) }}
          </span>
          <span class="session-subline-dot">·</span>
          <span class="meta-item">
            <SvgIcon name="message-square" :size="11" />
            {{ t("session.common.messageCount", { count: messageCount }) }}
          </span>
          <template v-if="proxyTrafficCount">
            <span class="session-subline-dot">·</span>
            <button
              class="meta-item meta-proxy-traffic"
              :title="t('session.sessionHeader.proxyTrafficTitle', { count: proxyTrafficCount })"
              type="button"
              @click="emit('openProxyTraffic')"
            >
              <SvgIcon name="zap" :size="11" />
              {{ t("session.sessionHeader.proxyRecordCount", { count: proxyTrafficCount }) }}
            </button>
          </template>
          <span
            v-if="isArchived"
            class="header-archived-badge"
            :title="t('session.sessionHeader.archivedBadgeTitle')"
          >
            {{ t("session.sessionTree.archived") }}
          </span>
        </div>
      </div>
    </div>

    <div class="header-actions">
      <!-- 1. ⭐ Favorite / Star -->
      <button
        class="action-btn header-btn-favorite"
        :class="{ active: favorite }"
        :title="favorite ? t('session.sessionHeader.favoriteRemove') : t('session.sessionHeader.favoriteAdd')"
        type="button"
        @click="emit('toggleFavorite')"
      >
        <SvgIcon name="star" :size="16" />
      </button>

      <!-- 2. 🔖 Bookmark Button & Dropdown -->
      <div class="bookmark-wrapper">
        <button
          ref="bookmarkTriggerRef"
          class="action-btn header-btn-bookmark"
          :class="{ 'has-bookmarks': bookmarks && bookmarks.length > 0, 'menu-open': showBookmarkMenu }"
          :title="bookmarks && bookmarks.length > 0 ? t('session.sessionHeader.bookmarkListCount', { count: bookmarks.length }) : t('session.sessionHeader.bookmarkListEmpty')"
          type="button"
          aria-haspopup="menu"
          :aria-expanded="showBookmarkMenu"
          :aria-label="t('session.sessionHeader.bookmarkList')"
          @click="toggleBookmarkMenu"
        >
          <SvgIcon name="bookmark" :size="16" />
        </button>

        <Transition name="fade-scale">
          <div
            v-if="showBookmarkMenu"
            class="bookmark-menu menu-surface"
            role="menu"
            :aria-label="t('session.sessionHeader.bookmarkList')"
          >
            <div class="bookmark-menu-header">{{ t("session.sessionHeader.bookmarkMenuTitle") }}</div>
            <div v-if="!bookmarks || bookmarks.length === 0" class="bookmark-empty">
              {{ t("session.sessionHeader.bookmarkEmpty") }}<br /><span class="empty-sub">{{ t("session.sessionHeader.bookmarkEmptyHint") }}</span>
            </div>
            <div v-else class="bookmark-list">
              <button
                v-for="bm in bookmarks"
                :key="`${bm.cliId}-${bm.sessionId}-${bm.messageIndex}`"
                class="bookmark-item"
                role="menuitem"
                type="button"
                @click="goToBookmark(bm.messageIndex)"
              >
                <div class="bookmark-item-head">
                  <SvgIcon name="bookmark" :size="12" class="bm-icon" />
                  <span class="bookmark-label">{{ t("session.bookmarkList.messageIndex", { index: bm.messageIndex + 1 }) }}</span>
                  <span class="bookmark-time">{{ formatTimestamp(bm.createdAt) }}</span>
                </div>
                <span v-if="bm.note" class="bookmark-note">{{ bm.note }}</span>
              </button>
            </div>
          </div>
        </Transition>
      </div>

      <!-- 3. 📁 Focus Project in drawer -->
      <button
        class="action-btn header-btn-project"
        :title="t('session.sessionHeader.openProject')"
        type="button"
        @click="emit('openProject')"
      >
        <SvgIcon name="folder-open" :size="16" />
      </button>

      <!-- 3. 🔄 Refresh -->
      <button
        class="action-btn header-btn-refresh"
        :title="isRefreshingState ? t('session.common.reloading') : t('session.sessionHeader.refreshTitle')"
        :disabled="isRefreshingState"
        type="button"
        @click="handleRefresh"
      >
        <SvgIcon name="refresh-cw" :size="16" :class="{ spin: isRefreshingState }" />
      </button>

      <!-- 4. ··· More Menu -->
      <div class="more-wrapper">
        <button
          ref="moreTriggerRef"
          class="action-btn header-btn-more"
          :class="{ 'menu-open': showMoreMenu }"
          :title="t('session.sessionHeader.moreActions')"
          type="button"
          aria-haspopup="menu"
          :aria-expanded="showMoreMenu"
          :aria-label="t('session.sessionHeader.moreActions')"
          @click="toggleMoreMenu"
        >
          <SvgIcon name="more-horizontal" :size="17" />
        </button>
        <Transition name="fade-scale">
          <div
            v-if="showMoreMenu"
            ref="moreMenuRef"
            class="more-menu menu-surface"
            role="menu"
            :aria-label="t('session.sessionHeader.moreActions')"
          >
            <!-- 返回父对话 -->
            <button
              v-if="hasParentSession"
              class="menu-item header-menu-back-parent"
              role="menuitem"
              type="button"
              @click="handleBackToParent"
            >
              <SvgIcon name="chevron-left" :size="15" />
              <span>{{ t("session.sessionHeader.backToParent") }}</span>
            </button>

            <!-- 永久保留对话 -->
            <button
              class="menu-item header-menu-pin"
              :class="{ active: archivePinned }"
              :disabled="archivePinLoading"
              role="menuitem"
              type="button"
              @click="requestArchivePinned"
            >
              <SvgIcon v-if="archivePinLoading" class="spin" name="loader" :size="15" />
              <SvgIcon v-else :name="archivePinned ? 'pin-off' : 'pin'" :size="15" />
              <span>{{ archivePinned ? t('session.sessionHeader.unpinArchive') : t('session.sessionHeader.pinArchive') }}</span>
            </button>

            <div class="menu-divider"></div>

            <!-- 对话深度分析 -->
            <button
              v-if="hasAnalytics"
              class="menu-item header-menu-analytics"
              role="menuitem"
              type="button"
              @click="openAnalyticsModal"
            >
              <SvgIcon name="bar-chart-2" :size="15" />
              <span>{{ t("session.sessionHeader.analytics") }}</span>
            </button>

            <!-- 导出对话 -->
            <button
              class="menu-item header-menu-export"
              role="menuitem"
              type="button"
              @click="openExportDialog"
            >
              <SvgIcon name="download" :size="15" />
              <span>{{ t("session.sessionHeader.export") }}</span>
            </button>

            <!-- 多选操作 -->
            <button
              class="menu-item header-menu-select"
              role="menuitem"
              type="button"
              @click="handleEnterSelection"
            >
              <SvgIcon name="check-square" :size="15" />
              <span>{{ t("session.sessionHeader.multiSelect") }}</span>
            </button>

            <div v-if="supportsResume" class="menu-divider"></div>

            <!-- 恢复对话 -->
            <button
              v-if="supportsResume"
              class="menu-item header-menu-resume"
              role="menuitem"
              type="button"
              @click="openResumeDialog"
            >
              <SvgIcon name="terminal" :size="15" />
              <span>{{ t("session.sessionHeader.resume") }}</span>
            </button>

            <!-- 复制恢复命令 -->
            <button
              v-if="supportsResume"
              class="menu-item header-menu-copy-command"
              role="menuitem"
              type="button"
              @click="handleDirectCopyRestoreCommand"
            >
              <SvgIcon name="copy" :size="15" />
              <span>{{ t("session.sessionHeader.copyResumeCommand") }}</span>
            </button>
          </div>
        </Transition>
      </div>
    </div>

    <!-- Session Analytics Modal -->
    <SessionAnalyticsModal
      v-if="showAnalyticsModal"
      :displayName="displayName"
      :messages="analyticsMessages"
      :stats="stats"
      @close="showAnalyticsModal = false"
    />

    <!-- Resume Session Dialog -->
    <ResumeSessionDialog
      v-if="showResumeDialog"
      :profiles="profiles"
      :cliId="cliId ?? 'claude'"
      :isArchived="isArchived"
      @close="showResumeDialog = false"
      @resume="handleResumeFromDialog"
      @copyCommand="handleCopyCommandFromDialog"
    />

    <!-- Export Session Dialog -->
    <ExportSessionDialog
      v-if="showExportDialog"
      @close="showExportDialog = false"
      @export="handleExportFromDialog"
      @enterSelectionMode="handleEnterSelectionFromDialog"
    />
  </div>
</template>

<style scoped>
.session-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-3);
  padding: 8px 14px;
  background: var(--color-bg);
  border-bottom: 1px solid var(--color-border);
  flex-shrink: 0;
  min-width: 0;
}

.header-info {
  display: flex;
  flex-direction: column;
  gap: 2px;
  flex: 1 1 auto;
  min-width: 0;
  overflow: hidden;
}

.session-name {
  margin: 0;
  font-size: 14px;
  font-weight: 600;
  color: var(--color-text);
  line-height: 1.3;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.session-subline {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 11px;
  color: var(--color-text-muted);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  min-width: 0;
}

.session-path {
  display: inline-flex;
  align-items: center;
  gap: 3px;
  min-width: 0;
  flex-shrink: 1;
  overflow: hidden;
}

.session-path-icon {
  flex-shrink: 0;
  opacity: 0.7;
}

.session-path-text {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.session-subline-dot {
  flex-shrink: 0;
  opacity: 0.5;
  font-size: 10px;
}

.session-meta {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  flex-shrink: 0;
  white-space: nowrap;
}

.meta-item {
  display: inline-flex;
  align-items: center;
  gap: 3px;
  white-space: nowrap;
}

.meta-proxy-traffic {
  background: none;
  border: none;
  padding: 0;
  font: inherit;
  color: var(--color-primary);
  cursor: pointer;
}

.meta-proxy-traffic:hover {
  text-decoration: underline;
}

.header-archived-badge {
  display: inline-flex;
  align-items: center;
  font-size: 10px;
  padding: 0 4px;
  border-radius: var(--radius-sm, 4px);
  background: var(--color-bg-hover);
  border: 1px solid var(--color-border);
  color: var(--color-text-muted);
  line-height: 1.4;
  white-space: nowrap;
}

.header-actions {
  flex-shrink: 0;
  display: flex;
  align-items: center;
  gap: 4px;
}

.action-btn {
  width: 28px;
  height: 28px;
  padding: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  background: transparent;
  border: 1px solid transparent;
  border-radius: var(--radius-md, 6px);
  color: var(--color-text-muted);
  cursor: pointer;
  transition: all var(--transition-fast, 0.15s);
  flex-shrink: 0;
}

.action-btn:hover:not(:disabled) {
  background: var(--color-bg-hover);
  color: var(--color-text);
  border-color: var(--color-border);
}

.action-btn:disabled {
  opacity: 0.4;
  cursor: not-allowed;
}

.header-btn-favorite.active {
  color: var(--color-warning, #f59e0b);
}

.header-btn-favorite.active :deep(.svg-icon) {
  fill: currentColor;
}

.more-wrapper {
  position: relative;
}

.menu-surface {
  position: absolute;
  top: calc(100% + 6px);
  right: 0;
  z-index: 100;
  background: var(--color-bg);
  border: 1px solid var(--color-border);
  border-radius: var(--radius-lg, 10px);
  box-shadow: 0 10px 25px -5px rgba(0, 0, 0, 0.18);
  padding: 4px;
  min-width: 180px;
}

.more-menu {
  display: flex;
  flex-direction: column;
  gap: 2px;
  width: 190px;
}

.menu-item {
  width: 100%;
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 7px 10px;
  background: transparent;
  border: none;
  border-radius: var(--radius-sm, 6px);
  font-size: 13px;
  color: var(--color-text);
  cursor: pointer;
  text-align: left;
  transition: background 0.15s;
}

.menu-item:hover:not(:disabled) {
  background: var(--color-bg-hover);
}

.menu-item:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.menu-item.active {
  color: var(--color-primary, #3b82f6);
}

.menu-divider {
  height: 1px;
  background: var(--color-border);
  margin: 4px 0;
}

.menu-overlay {
  position: fixed;
  inset: 0;
  z-index: 90;
}

.spin {
  animation: spin 1s linear infinite;
}

@keyframes spin {
  from { transform: rotate(0deg); }
  to { transform: rotate(360deg); }
}

/* Vue Transitions */
.fade-scale-enter-active,
.fade-scale-leave-active {
  transition: all 0.15s cubic-bezier(0.16, 1, 0.3, 1);
}

.fade-scale-enter-from,
.fade-scale-leave-to {
  opacity: 0;
  transform: scale(0.95) translateY(-4px);
}

.header-btn-bookmark.has-bookmarks {
  color: var(--color-primary);
}

.bookmark-wrapper {
  position: relative;
}

.bookmark-menu {
  position: absolute;
  top: calc(100% + 6px);
  right: 0;
  z-index: 200;
  width: 260px;
  max-height: 320px;
  overflow-y: auto;
  background: var(--color-bg-elevated, var(--color-bg));
  border: 1px solid var(--color-border);
  border-radius: var(--radius-lg, 8px);
  box-shadow: var(--shadow-md, 0 10px 25px -5px rgba(0, 0, 0, 0.18));
  padding: 4px;
}

.bookmark-menu-header {
  font-size: 11px;
  font-weight: 600;
  color: var(--color-text-muted);
  padding: 4px 8px 2px;
}

.bookmark-empty {
  padding: 16px 8px;
  text-align: center;
  font-size: 12px;
  color: var(--color-text-muted);
  line-height: 1.5;
}

.bookmark-empty .empty-sub {
  font-size: 11px;
  opacity: 0.75;
}

.bookmark-list {
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.bookmark-item {
  display: flex;
  flex-direction: column;
  gap: 2px;
  width: 100%;
  padding: 6px 8px;
  border: none;
  background: transparent;
  border-radius: var(--radius-sm, 4px);
  text-align: left;
  cursor: pointer;
  transition: background var(--transition-fast);
}

.bookmark-item:hover {
  background: var(--color-bg-hover);
}

.bookmark-item-head {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 12px;
}

.bm-icon {
  color: var(--color-primary);
  flex-shrink: 0;
}

.bookmark-label {
  font-weight: 500;
  color: var(--color-text);
  flex: 1;
}

.bookmark-time {
  font-size: 10px;
  color: var(--color-text-muted);
}

.bookmark-note {
  font-size: 11px;
  color: var(--color-text-secondary);
  padding-left: 18px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

</style>


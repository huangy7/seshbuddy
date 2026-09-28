<script setup lang="ts">
import { ref, computed, onMounted, watch } from "vue";
import { invokeApp } from "../utils/invokeApp";
import SvgIcon from "./icons/SvgIcon.vue";
import { formatTimestamp } from "../utils/format";
import { t } from "../i18n";
import { useSessions } from "../composables/useSessions";
import { renderMarkdown } from "../utils/markdown";
import { vMermaid } from "../directives/vMermaid";
import type { CliId } from "../types/cli";
import type { SessionIdentity } from "../types/session";

interface BookmarkWithContext {
  cliId: CliId;
  sessionId: string;
  sessionPath: string;
  sessionDisplayName: string;
  messageIndex: number;
  messagePreview: string;
  note?: string;
  createdAt: string;
  sourceDeleted: boolean;
  messageRole?: string;
  messageText?: string;
  messageTimestamp?: string;
}

interface SessionGroup {
  groupKey: string;
  sessionPath: string;
  displayName: string;
  bookmarks: BookmarkWithContext[];
  latestCreatedAt: string;
  sourceDeleted: boolean;
}

const emit = defineEmits<{
  jumpToBookmark: [identity: SessionIdentity, encodedDir: string, messageIndex: number];
  bookmarksChanged: [];
}>();

const { visibleCliIds, cliOptions, projects } = useSessions();

const bookmarks = ref<BookmarkWithContext[]>([]);
const loading = ref(false);
const collapsedGroups = ref<Set<string>>(new Set());
const selectedBookmark = ref<BookmarkWithContext | null>(null);
const showDetailDialog = ref(false);

const cliNameById = computed(() => new Map(cliOptions.value.map((cli) => [cli.id, cli.name])));
// 多来源时给书签项加 CLI 标识，避免同名条目分不清来源
const showCliBadge = computed(() => new Set(bookmarks.value.map((bm) => bm.cliId)).size > 1);

function resolveSessionDisplayName(bm: BookmarkWithContext): string {
  if (bm.sessionPath && bm.cliId && projects.value) {
    for (const p of projects.value) {
      const s = p.sessions.find(
        (sess) => sess.file_path === bm.sessionPath && sess.cli_id === bm.cliId,
      );
      if (s?.display_name) {
        return s.display_name;
      }
    }
  }
  return bm.sessionDisplayName || bm.sessionId;
}


const groupedBookmarks = computed<SessionGroup[]>(() => {
  const map = new Map<string, SessionGroup>();
  for (const bm of bookmarks.value) {
    // 分组键带 cliId：不同 CLI 可能共享同一 sessionPath/sessionId
    const groupKey = `${bm.cliId}:${bm.sourceDeleted ? `deleted:${bm.sessionId}` : bm.sessionPath}`;
    let group = map.get(groupKey);
    if (!group) {
      group = {
        groupKey,
        sessionPath: bm.sessionPath,
        displayName: resolveSessionDisplayName(bm),
        bookmarks: [],
        latestCreatedAt: bm.createdAt,
        sourceDeleted: bm.sourceDeleted,
      };
      map.set(groupKey, group);
    }
    group.bookmarks.push(bm);
    if (bm.createdAt > group.latestCreatedAt) {
      group.latestCreatedAt = bm.createdAt;
    }
  }
  const groups = Array.from(map.values());
  groups.sort((a, b) => b.latestCreatedAt.localeCompare(a.latestCreatedAt));
  for (const g of groups) {
    g.bookmarks.sort((a, b) => a.createdAt.localeCompare(b.createdAt));
  }
  return groups;
});

function toggleGroup(groupKey: string) {
  const next = new Set(collapsedGroups.value);
  if (next.has(groupKey)) {
    next.delete(groupKey);
  } else {
    next.add(groupKey);
  }
  collapsedGroups.value = next;
}

function formatTime(ts: string): string {
  const full = formatTimestamp(ts);
  const match = full.match(/(\d{1,2}:\d{2})/);
  return match ? match[1] : full;
}

function encodedDirFromPath(sessionPath: string): string {
  const parts = sessionPath.replace(/\\/g, "/").split("/");
  const projectsIdx = parts.indexOf("projects");
  if (projectsIdx >= 0 && projectsIdx + 1 < parts.length) {
    return parts[projectsIdx + 1];
  }
  return "";
}

function onClickBookmark(bm: BookmarkWithContext) {
  if (bm.sourceDeleted) {
    selectedBookmark.value = bm;
    showDetailDialog.value = true;
    return;
  }
  const encodedDir = encodedDirFromPath(bm.sessionPath);
  emit("jumpToBookmark", { cliId: bm.cliId, filePath: bm.sessionPath }, encodedDir, bm.messageIndex);
}

async function deleteBookmark(bm: BookmarkWithContext) {
  try {
    await invokeApp("toggle_bookmark", {
      cliId: bm.cliId,
      sessionId: bm.sessionId,
      messageIndex: bm.messageIndex,
    });
    await loadBookmarks();
    emit("bookmarksChanged");
  } catch (e) {
    console.error("Failed to delete bookmark:", e);
  }
}

let loadSeq = 0;

async function loadBookmarks() {
  const seq = ++loadSeq;
  loading.value = true;
  // list_bookmarks_with_context 是单 CLI 接口：按可见 CLI fan-out 后合并，
  // 与 App.vue loadBookmarks 同一合并语义——单 CLI 失败保留其旧切片
  const cliIds = new Set(visibleCliIds.value);
  const outcomes = await Promise.all(
    [...cliIds].map(async (cliId) => {
      try {
        const result = await invokeApp<BookmarkWithContext[]>(
          "list_bookmarks_with_context",
          { cliId }
        );
        return { cliId, result } as const;
      } catch (e) {
        console.error(`list_bookmarks_with_context failed (${cliId}):`, e);
        return { cliId, result: null } as const;
      }
    })
  );
  if (seq !== loadSeq) return;
  const succeeded = new Set(
    outcomes.filter((outcome) => outcome.result !== null).map((outcome) => outcome.cliId)
  );
  bookmarks.value = [
    ...bookmarks.value.filter((bm) => cliIds.has(bm.cliId) && !succeeded.has(bm.cliId)),
    ...outcomes.flatMap((outcome) => outcome.result ?? []),
  ];
  loading.value = false;
}

onMounted(loadBookmarks);
watch(visibleCliIds, loadBookmarks);

defineExpose({ refresh: loadBookmarks });
</script>

<template>
  <div class="bookmark-list-view">
    <div class="bookmark-header">
      <SvgIcon name="bookmark" :size="14" class="header-icon" />
      <span class="bookmark-title">{{ t("session.bookmarkList.title") }}</span>
    </div>

    <div class="bookmark-body">
      <div v-if="loading" class="bookmark-loading">{{ t("common.timelineView.loading") }}</div>
      <div v-else-if="groupedBookmarks.length === 0" class="bookmark-empty">
        <SvgIcon name="bookmark" :size="24" />
        <span>{{ t("session.bookmarkList.empty") }}</span>
        <span class="bookmark-empty-hint">{{ t("session.bookmarkList.emptyHint") }}</span>
      </div>
      <div v-else class="bookmark-groups">
        <div
          v-for="group in groupedBookmarks"
          :key="group.groupKey"
          class="bookmark-group"
        >
          <div class="group-header" @click="toggleGroup(group.groupKey)">
            <SvgIcon
              :name="collapsedGroups.has(group.groupKey) ? 'chevron-right' : 'chevron-down'"
              :size="12"
            />
            <span class="group-name" :class="{ 'group-deleted': group.sourceDeleted }">
              {{ group.displayName }}
              <span v-if="group.sourceDeleted" class="deleted-tag">{{ t("session.bookmarkList.deletedTag") }}</span>
            </span>
            <span class="group-count">{{ group.bookmarks.length }}</span>
          </div>
          <div v-if="!collapsedGroups.has(group.groupKey)" class="group-items">
            <div
              v-for="bm in group.bookmarks"
              :key="`${bm.cliId}-${bm.sessionId}-${bm.messageIndex}`"
              class="bookmark-item"
              :class="{ 'bookmark-deleted': bm.sourceDeleted }"
              @click="onClickBookmark(bm)"
            >
              <SvgIcon name="bookmark" :size="12" class="bookmark-icon" />
              <span v-if="showCliBadge" class="bookmark-cli-tag">{{ cliNameById.get(bm.cliId) ?? bm.cliId }}</span>
              <span class="bookmark-time">{{ formatTime(bm.createdAt) }}</span>
              <span class="bookmark-preview">{{ bm.messagePreview || t("session.bookmarkList.messageIndex", { index: bm.messageIndex + 1 }) }}</span>
              <div v-if="bm.note" class="bookmark-note">{{ bm.note }}</div>
              <button class="bookmark-delete-btn" :title="t('session.bookmarkList.remove')" @click.stop="deleteBookmark(bm)">
                <SvgIcon name="trash-2" :size="12" />
              </button>
            </div>
          </div>
        </div>
      </div>
    </div>

    <!-- 详情弹窗 -->
    <div v-if="showDetailDialog && selectedBookmark" class="modal-backdrop" @click.self="showDetailDialog = false">
      <div class="dialog">
        <div class="dialog-header">
          <h3>
            {{ selectedBookmark.sourceDeleted ? t("session.bookmarkList.deletedSession") : '' }}
            {{ resolveSessionDisplayName(selectedBookmark) }}
          </h3>
          <button class="dialog-close icon-btn" :title="t('dialogs.common.close')" :aria-label="t('dialogs.common.close')" @click="showDetailDialog = false">
            <SvgIcon name="x" :size="16" />
          </button>
        </div>
        <div class="dialog-body">
          <div class="message-info">
            <span class="message-role" :class="selectedBookmark.messageRole">
              {{ selectedBookmark.messageRole === 'user' ? t('app.filter.user') : t('app.filter.assistant') }}
            </span>
            <span class="message-time">{{ formatTimestamp(selectedBookmark.createdAt) }}</span>
          </div>
          <div v-if="selectedBookmark.note" class="detail-note">
            <strong>{{ t("session.bookmarkList.note") }}</strong> {{ selectedBookmark.note }}
          </div>
          <div 
            v-if="selectedBookmark.messageText" 
            v-mermaid
            class="message-content markdown-body" 
            v-html="renderMarkdown(selectedBookmark.messageText)"
          ></div>
          <div v-else class="message-no-content">
            {{ t("session.bookmarkList.noContent") }}
          </div>
        </div>
        <div class="dialog-footer">
          <button class="btn-close" @click="showDetailDialog = false">{{ t("dialogs.common.close") }}</button>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.bookmark-list-view {
  flex: 1;
  display: flex;
  flex-direction: column;
  overflow: hidden;
}
.bookmark-header {
  display: flex;
  align-items: center;
  gap: var(--space-1);
  height: 35px;
  min-height: 35px;
  padding: 0 var(--space-3);
  font-size: var(--text-sm);
  font-weight: 600;
  color: var(--color-text);
  border-bottom: 1px solid var(--color-border);
  flex-shrink: 0;
}
.header-icon {
  color: var(--color-warning, var(--color-primary));
}
.bookmark-body {
  flex: 1;
  overflow-y: auto;
  padding: var(--space-1) 0;
}
.bookmark-loading {
  text-align: center;
  padding: var(--space-6);
  color: var(--color-text-muted);
  font-size: var(--text-sm);
}
.bookmark-empty {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: var(--space-2);
  padding: var(--space-6);
  color: var(--color-text-muted);
  font-size: var(--text-sm);
}
.bookmark-empty-hint {
  font-size: var(--text-xs);
  color: var(--color-text-muted);
}
.bookmark-groups {
  display: flex;
  flex-direction: column;
}
.group-header {
  display: flex;
  align-items: center;
  gap: var(--space-1);
  min-height: 30px;
  height: 30px;
  box-sizing: border-box;
  padding: 0 8px;
  margin: 1px 6px;
  cursor: pointer;
  border-radius: 7px;
  font-size: var(--text-sm);
  font-weight: 500;
  color: var(--color-text);
  transition: background var(--transition-fast);
}
.group-header:hover {
  background: var(--color-surface-selected);
}
.group-name {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.group-count {
  font-size: var(--text-2xs);
  font-variant-numeric: tabular-nums;
  font-weight: 500;
  color: var(--color-text-muted);
  background: var(--color-surface-selected);
  border: 1px solid var(--color-border-light);
  border-radius: var(--radius-full);
  padding: 1px 6px;
  min-width: 18px;
  height: 18px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  margin-left: auto;
  line-height: 1;
}
.group-items {
  display: flex;
  flex-direction: column;
}
.bookmark-item {
  display: flex;
  align-items: baseline;
  gap: var(--space-2);
  min-height: 30px;
  box-sizing: border-box;
  padding: 4px 8px 4px 16px;
  margin: 1px 6px;
  border-radius: 7px;
  cursor: pointer;
  transition: background var(--transition-fast);
  flex-wrap: wrap;
}
.bookmark-item:hover {
  background: var(--color-bg-hover);
}
.bookmark-icon {
  color: var(--color-warning, var(--color-primary));
  flex-shrink: 0;
  position: relative;
  top: 2px;
}
.bookmark-cli-tag {
  flex-shrink: 0;
  padding: 0 4px;
  border: 1px solid var(--color-border);
  border-radius: var(--radius-sm);
  color: var(--color-text-muted);
  font-size: var(--text-2xs);
  line-height: 1.5;
}
.bookmark-time {
  font-family: var(--font-mono);
  font-size: var(--text-2xs);
  color: var(--color-text-muted);
  white-space: nowrap;
  flex-shrink: 0;
}
.bookmark-preview {
  font-size: var(--text-xs);
  color: var(--color-text-secondary);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  flex: 1;
  min-width: 0;
}
.bookmark-note {
  width: 100%;
  font-size: var(--text-2xs);
  color: var(--color-text-muted);
  padding-left: calc(12px + var(--space-2));
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.group-deleted {
  color: var(--color-text-muted);
}
.deleted-tag {
  font-size: var(--text-2xs);
  color: var(--color-text-muted);
  font-weight: 400;
}
.bookmark-deleted {
  /* No longer cursor: default; */
}
/* .bookmark-deleted:hover {
  background: transparent;
} */
.bookmark-delete-btn {
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 4px;
  margin-left: auto;
  color: var(--color-text-muted);
  background: transparent;
  border: none;
  border-radius: var(--radius-sm);
  cursor: pointer;
  opacity: 0;
  transition: all var(--transition-fast);
}
.bookmark-item:hover .bookmark-delete-btn {
  opacity: 1;
}
.bookmark-delete-btn:hover {
  color: var(--color-danger, #dc2626);
  background: var(--color-bg-secondary);
}

/* Dialog styles */
.modal-backdrop {
  padding: var(--space-4);
}
.dialog {
  width: 100%;
  max-width: 600px;
  max-height: 80vh;
  display: flex;
  flex-direction: column;
  background: var(--color-bg);
  border: 1px solid var(--color-border);
  border-radius: var(--radius-xl);
  box-shadow: var(--shadow-lg);
}
.dialog-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: var(--space-4);
  border-bottom: 1px solid var(--color-border);
}
.dialog-header h3 {
  font-size: var(--text-base);
  font-weight: 600;
  color: var(--color-text);
  margin: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.dialog-close {
  border-radius: var(--radius-sm);
}
.dialog-body {
  padding: var(--space-4);
  overflow-y: auto;
  flex: 1;
}
.message-info {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  margin-bottom: var(--space-3);
}
.message-role {
  font-size: var(--text-xs);
  font-weight: 600;
  padding: 2px 6px;
  border-radius: var(--radius-sm);
  background: var(--color-bg-secondary);
  color: var(--color-text-secondary);
}
.message-role.user {
  background: rgba(var(--color-primary-rgb, 59, 130, 246), 0.1);
  color: var(--color-primary);
}
.message-role.assistant {
  background: rgba(var(--color-success-rgb, 16, 185, 129), 0.1);
  color: var(--color-success);
}
.message-time {
  font-size: var(--text-xs);
  color: var(--color-text-muted);
}
.detail-note {
  font-size: var(--text-xs);
  color: var(--color-text-secondary);
  background: var(--color-bg-secondary);
  padding: var(--space-2);
  border-radius: var(--radius-md);
  margin-bottom: var(--space-4);
  border-left: 3px solid var(--color-warning, var(--color-primary));
}
.message-content {
  font-size: var(--text-sm);
  line-height: 1.6;
  color: var(--color-text);
}
.message-no-content {
  text-align: center;
  padding: var(--space-6);
  color: var(--color-text-muted);
}
.message-no-content p {
  font-size: var(--text-sm);
  margin-bottom: var(--space-4);
}
.message-preview-fallback {
  text-align: left;
  padding: var(--space-3);
  background: var(--color-bg-secondary);
  border-radius: var(--radius-md);
  font-size: var(--text-xs);
}
.dialog-footer {
  display: flex;
  justify-content: flex-end;
  padding: var(--space-3) var(--space-4);
  border-top: 1px solid var(--color-border);
}
.btn-close {
  padding: var(--space-1) var(--space-4);
  font-size: var(--text-sm);
  color: white;
  background: var(--color-primary);
  border: none;
  border-radius: var(--radius-md);
  cursor: pointer;
  transition: all var(--transition-fast);
}
.btn-close:hover {
  background: var(--color-primary-hover);
}
</style>

<script setup lang="ts">
import { ref, reactive, computed, nextTick, watch } from "vue";
import SvgIcon from "./icons/SvgIcon.vue";
import ChatAvatar from "./chat/ChatAvatar.vue";
import { formatTimestamp, formatRelativeTime } from "../utils/format";
import { projectLabel } from "../utils/projectPath";
import { sessionIdentityKey, type ProjectInfo, type AggregatedProjectInfo, type SessionIdentity, type SessionInfo } from "../types/session";
import { isCliId, getCliDefinition, type CliId } from "../types/cli";
import { useBlockedFolders } from "../composables/useBlockedFolders";
import { useFavorites } from "../composables/useFavorites";
import { groupProjectsByProvider, type ProviderGroup, type TreeGrouping } from "../composables/useTreeGrouping";
import { t } from "../i18n";
import { shouldShowTreeLoading } from "./sessionTreeLoading";

const props = withDefaults(
  defineProps<{
    projects: ProjectInfo[];
    selectedSessionIdentity: SessionIdentity | null;
    selectedSessionIdentities: SessionIdentity[];
    contextMenuSessionIdentity?: SessionIdentity | null;
    searchQuery: string;
    isStreamingProjects: boolean;
    totalLoadedSessions: number;
    isRefreshing: boolean;
    showLoadingIndicator: boolean;
    bootstrapping?: boolean;
    hasCompletedInitialScan?: boolean;
    projectAllSelected: (projectKey: string) => boolean;
    grouping?: TreeGrouping;
  }>(),
  {
    grouping: "directory",
    hasCompletedInitialScan: true,
  }
);

const emit = defineEmits<{
  selectSession: [identity: SessionIdentity, encodedDir: string];
  pinSession: [identity: SessionIdentity, encodedDir: string];
  contextMenuSession: [event: MouseEvent, identity: SessionIdentity, sessionId: string, displayName: string, projectPath: string];
  contextMenuProject: [event: MouseEvent, project: ProjectInfo];
  renameSession: [identity: SessionIdentity, newName: string];
  toggleSelect: [identity: SessionIdentity, event: MouseEvent];
  rangeSelect: [startIdentity: SessionIdentity, endIdentity: SessionIdentity];
  clearSelection: [];
  selectProjectSessions: [projectKey: string];
}>();

// Inline rename state
const renamingSessionKey = ref<string | null>(null);
const renameInput = ref("");

function startRename(identity: SessionIdentity, currentName: string) {
  renamingSessionKey.value = sessionIdentityKey(identity);
  renameInput.value = currentName;
  setTimeout(() => {
    const el = document.querySelector(".session-rename-input") as HTMLInputElement | null;
    if (el) {
      el.focus();
      el.select();
    }
  }, 50);
}

function confirmRename(identity: SessionIdentity) {
  const newName = renameInput.value.trim();
  renamingSessionKey.value = null;
  if (newName) {
    emit("renameSession", identity, newName);
  }
}

function confirmRenameForSession(session: SessionInfo) {
  const identity = identityOf(session);
  if (identity && renamingSessionKey.value === sessionIdentityKey(identity)) {
    confirmRename(identity);
  }
}

function cancelRename() {
  renamingSessionKey.value = null;
}

const isMultiSelectMode = computed(() => props.selectedSessionIdentities.length > 0);

const collapsed: Record<string, boolean> = reactive({});
const treeRef = ref<HTMLElement | null>(null);
const { blockFolder, isBlocked: isFolderBlocked } = useBlockedFolders();

const visibleProjects = computed(() => {
  return props.projects.filter((p) => !isFolderBlocked(p.original_path ?? ""));
});

const showTreeSkeleton = computed(() => shouldShowTreeLoading({
  projectCount: props.projects.length,
  bootstrapping: props.bootstrapping ?? false,
  hasCompletedInitialScan: props.hasCompletedInitialScan ?? true,
  isStreamingProjects: props.isStreamingProjects,
  isRefreshing: props.isRefreshing,
}));

const showEmptyHint = computed(() => {
  if (showTreeSkeleton.value) return false;
  return visibleProjects.value.length === 0;
});

const providerGroups = computed<ProviderGroup[]>(() => {
  if (props.grouping !== "provider") return [];
  return groupProjectsByProvider(visibleProjects.value as AggregatedProjectInfo[]);
});

function providerKey(cliId: string): string {
  return `provider:${cliId}`;
}

function providerProjectKey(cliId: string, project: ProjectInfo | string): string {
  const pk = typeof project === "string" ? project : projectKey(project);
  return `provider:${cliId}:${pk}`;
}

function isProviderCollapsed(cliId: string): boolean {
  if (props.searchQuery.trim()) return false;
  return collapsed[providerKey(cliId)] ?? false;
}

function toggleProvider(cliId: string) {
  const key = providerKey(cliId);
  collapsed[key] = !isProviderCollapsed(cliId);
}

function toggleProviderProject(cliId: string, pk: string) {
  const key = providerProjectKey(cliId, pk);
  collapsed[key] = !isCollapsed(key);
}

function cliName(cliId: CliId): string {
  return getCliDefinition(cliId)?.name || cliId;
}

function toggleProject(projectKey: string) {
  collapsed[projectKey] = !(collapsed[projectKey] ?? true);
}

function isCollapsed(projectKey: string): boolean {
  if (props.searchQuery.trim()) return false;
  return collapsed[projectKey] ?? true;
}

function projectKey(project: ProjectInfo): string {
  return (
    (project as ProjectInfo & { project_key?: string }).project_key ?? project.encoded_dir ?? ""
  );
}

function collapseAll() {
  if (props.grouping === "provider") {
    for (const group of providerGroups.value) {
      collapsed[providerKey(group.cliId)] = true;
      for (const p of group.projects) {
        collapsed[providerProjectKey(group.cliId, p)] = true;
      }
    }
    return;
  }
  for (const p of visibleProjects.value) {
    collapsed[projectKey(p)] = true;
  }
}

function expandAll() {
  if (props.grouping === "provider") {
    for (const group of providerGroups.value) {
      collapsed[providerKey(group.cliId)] = false;
      for (const p of group.projects) {
        collapsed[providerProjectKey(group.cliId, p)] = false;
      }
    }
    return;
  }
  for (const p of visibleProjects.value) {
    collapsed[projectKey(p)] = false;
  }
}

const allCollapsed = computed(() => {
  if (visibleProjects.value.length === 0) return true;
  if (props.grouping === "provider") {
    if (providerGroups.value.length === 0) return true;
    return providerGroups.value.every(
      (g) => isProviderCollapsed(g.cliId) && g.projects.every((p) => isCollapsed(providerProjectKey(g.cliId, p)))
    );
  }
  return visibleProjects.value.every((p) => isCollapsed(projectKey(p)));
});

function toggleCollapseAll() {
  if (allCollapsed.value) {
    expandAll();
  } else {
    collapseAll();
  }
}

function lastPathSegment(path: string | null): string {
  return projectLabel(path);
}

function formatFileSize(bytes: number): string {
  if (bytes < 1024) return bytes + " B";
  if (bytes < 1024 * 1024) return (bytes / 1024).toFixed(0) + " KB";
  return (bytes / (1024 * 1024)).toFixed(1) + " MB";
}

// Resolve a file path to its session info and encoded dir
function identityOf(session: SessionInfo): SessionIdentity | null {
  return isCliId(session.cli_id)
    ? { cliId: session.cli_id, filePath: session.file_path }
    : null;
}

const { isFavorite } = useFavorites();

function isFavoriteSession(session: SessionInfo): boolean {
  const identity = identityOf(session);
  return !!identity && isFavorite(identity);
}

function sessionKey(session: SessionInfo): string {
  const identity = identityOf(session);
  return identity ? sessionIdentityKey(identity) : `${session.cli_id}\u0000${session.file_path}`;
}

function resolveSession(identity: SessionIdentity): {
  session: SessionInfo;
  encodedDir: string | null;
  projectKey: string;
  projectPath: string | null;
  cliId?: CliId;
} | null {
  if (props.grouping === "provider") {
    for (const group of providerGroups.value) {
      if (group.cliId === identity.cliId) {
        for (const p of group.projects) {
          for (const s of p.sessions) {
            if (s.file_path === identity.filePath && s.cli_id === identity.cliId) {
              return {
                session: s,
                encodedDir: p.encoded_dir,
                projectKey: projectKey(p),
                projectPath: p.original_path,
                cliId: group.cliId,
              };
            }
          }
        }
      }
    }
    return null;
  }

  for (const p of visibleProjects.value) {
    for (const s of p.sessions) {
      if (s.file_path === identity.filePath && s.cli_id === identity.cliId) {
        return {
          session: s,
          encodedDir: p.encoded_dir,
          projectKey: projectKey(p),
          projectPath: p.original_path,
        };
      }
    }
  }
  return null;
}

watch(
  () => props.selectedSessionIdentity,
  async (identity) => {
    if (!identity) return;
    const target = resolveSession(identity);
    if (!target) return;
    if (props.grouping === "provider" && target.cliId) {
      collapsed[providerKey(target.cliId)] = false;
      collapsed[providerProjectKey(target.cliId, target.projectKey)] = false;
    } else {
      collapsed[target.projectKey] = false;
    }
    await nextTick();
    const selectedEl = treeRef.value?.querySelector(".session-item.selected") as HTMLElement | null;
    selectedEl?.scrollIntoView({ behavior: "smooth", block: "center" });
  },
  { immediate: true }
);

const canLocateSession = computed(() => {
  if (!props.selectedSessionIdentity) return false;
  return resolveSession(props.selectedSessionIdentity) !== null;
});

async function locateCurrentSession() {
  const identity = props.selectedSessionIdentity;
  if (!identity) return;
  const target = resolveSession(identity);
  if (!target) return;
  if (props.grouping === "provider" && target.cliId) {
    collapsed[providerKey(target.cliId)] = false;
    collapsed[providerProjectKey(target.cliId, target.projectKey)] = false;
  } else {
    collapsed[target.projectKey] = false;
  }
  await nextTick();
  const selectedEl = treeRef.value?.querySelector(".session-item.selected") as HTMLElement | null;
  if (!selectedEl) return;
  selectedEl.scrollIntoView({ behavior: "smooth", block: "center" });
  selectedEl.classList.add("locate-highlight");
  selectedEl.addEventListener("animationend", () => {
    selectedEl.classList.remove("locate-highlight");
  }, { once: true });
}

defineExpose({
  startRename,
  allCollapsed,
  canLocateSession,
  locateCurrentSession,
  toggleCollapseAll,
});

type HighlightSegment = {
  text: string;
  highlighted: boolean;
};

function getHighlightedSegments(text: string): HighlightSegment[] {
  if (!text) return [];

  const q = props.searchQuery.trim();
  if (!q) {
    return [{ text, highlighted: false }];
  }

  const escaped = q.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  return text
    .split(new RegExp(`(${escaped})`, "gi"))
    .filter((segment) => segment.length > 0)
    .map((segment) => ({
      text: segment,
      highlighted: segment.toLowerCase() === q.toLowerCase(),
    }));
}

// All visible sessions in visual top-to-bottom order (for keyboard navigation)
const visibleSessions = computed<Array<{ identity: SessionIdentity; encodedDir: string }>>(() => {
  const list: Array<{ identity: SessionIdentity; encodedDir: string }> = [];
  if (props.grouping === "provider") {
    for (const group of providerGroups.value) {
      if (!isProviderCollapsed(group.cliId)) {
        for (const p of group.projects) {
          if (!isCollapsed(providerProjectKey(group.cliId, p))) {
            for (const s of p.sessions) {
              const identity = identityOf(s);
              if (identity) list.push({ identity, encodedDir: p.encoded_dir ?? "" });
            }
          }
        }
      }
    }
    return list;
  }

  for (const p of visibleProjects.value) {
    if (!isCollapsed(projectKey(p))) {
      for (const s of p.sessions) {
        const identity = identityOf(s);
        if (identity) list.push({ identity, encodedDir: p.encoded_dir ?? "" });
      }
    }
  }
  return list;
});

function onKeyDown(e: KeyboardEvent) {
  if (e.key !== "ArrowUp" && e.key !== "ArrowDown") return;
  e.preventDefault();

  const list = visibleSessions.value;
  if (list.length === 0) return;

  const currentIdx = list.findIndex(
    (item) => props.selectedSessionIdentity
      && sessionIdentityKey(item.identity) === sessionIdentityKey(props.selectedSessionIdentity)
  );

  let nextIdx: number;
  if (e.key === "ArrowDown") {
    nextIdx = currentIdx < 0 ? 0 : Math.min(currentIdx + 1, list.length - 1);
  } else {
    nextIdx = currentIdx < 0 ? 0 : Math.max(currentIdx - 1, 0);
  }

  const next = list[nextIdx];
  emit("selectSession", next.identity, next.encodedDir);
}

function onSessionClick(event: MouseEvent, session: SessionInfo, encodedDir: string) {
  const identity = identityOf(session);
  if (!identity) return;
  const isMac = navigator.platform.toUpperCase().includes("MAC");
  const modKey = isMac ? event.metaKey : event.ctrlKey;

  if (props.selectedSessionIdentities.length > 0) {
    if (event.shiftKey) {
      // Range select: from last selected to clicked
      const lastSelected = props.selectedSessionIdentities[props.selectedSessionIdentities.length - 1];
      emit("rangeSelect", lastSelected, identity);
    } else {
      emit("toggleSelect", identity, event);
    }
  } else if (modKey || event.shiftKey) {
    emit("toggleSelect", identity, event);
  } else {
    emit("selectSession", identity, encodedDir);
  }
}

function onSessionDoubleClick(event: MouseEvent, session: SessionInfo, encodedDir: string) {
  if (event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) return;
  const identity = identityOf(session);
  if (identity) emit("pinSession", identity, encodedDir);
}

function isMultiSelected(session: SessionInfo): boolean {
  const identity = identityOf(session);
  if (!identity) return false;
  const key = sessionIdentityKey(identity);
  return props.selectedSessionIdentities.some((item) => sessionIdentityKey(item) === key);
}

function isSelectedSession(session: SessionInfo): boolean {
  const identity = identityOf(session);
  return !!identity && !!props.selectedSessionIdentity
    && sessionIdentityKey(identity) === sessionIdentityKey(props.selectedSessionIdentity);
}

function isContextMenuSession(session: SessionInfo): boolean {
  const identity = identityOf(session);
  return !!identity && !!props.contextMenuSessionIdentity
    && sessionIdentityKey(identity) === sessionIdentityKey(props.contextMenuSessionIdentity);
}

function onContextMenuSession(event: MouseEvent, session: SessionInfo, projectPath: string) {
  const identity = identityOf(session);
  if (identity) {
    emit("contextMenuSession", event, identity, session.session_id, session.display_name, projectPath);
  }
}

function onToggleSession(session: SessionInfo, event: MouseEvent) {
  const identity = identityOf(session);
  if (identity) emit("toggleSelect", identity, event);
}


</script>

<template>
  <div class="session-tree" :class="{ 'multi-select-mode': isMultiSelectMode }" ref="treeRef" role="tree" tabindex="0" @keydown="onKeyDown">
    <div v-if="showLoadingIndicator && !showTreeSkeleton" class="session-tree-streaming-indicator">
      <span v-if="totalLoadedSessions === 0">{{ t("session.sessionTree.loadingList") }}</span>
      <span v-else>{{ t("session.sessionTree.loadingListCount", { count: totalLoadedSessions }) }}</span>
    </div>
    <Transition name="tree-crossfade" mode="out-in">
      <div v-if="showTreeSkeleton" key="skeleton" class="tree-loading-state" role="status" aria-live="polite">
        <span class="tree-loading-icon"><SvgIcon name="loader" :size="18" /></span>
        <strong>{{ t("session.sessionTree.loadingList") }}</strong>
        <span>{{ t("session.sessionTree.scanningHint") }}</span>
      </div>

      <div v-else-if="showEmptyHint" key="empty" class="empty-hint">
        {{ projects.length > 0 ? t('session.sessionTree.emptyAllBlocked') : t('session.sessionTree.emptyNoData') }}
      </div>

      <!-- Provider Grouping Mode -->
      <div v-else-if="grouping === 'provider'" key="provider-list" class="tree-content-container">
        <div
          v-for="group in providerGroups"
          :key="providerKey(group.cliId)"
          class="provider-node"
          role="treeitem"
        >
          <div
            class="provider-header project-header"
            @click="toggleProvider(group.cliId)"
          >
            <SvgIcon
              :name="isProviderCollapsed(group.cliId) ? 'chevron-right' : 'chevron-down'"
              :size="14"
              class="chevron-icon"
              :class="{ rotated: !isProviderCollapsed(group.cliId) }"
            />
            <ChatAvatar role="assistant" :cliId="group.cliId" class="provider-avatar" />
            <span class="provider-name truncate">{{ cliName(group.cliId) }}</span>
            <span class="provider-count-capsule">{{ group.count }}</span>
          </div>

          <div v-if="!isProviderCollapsed(group.cliId)" class="provider-projects-list" role="group">
            <div
              v-for="project in group.projects"
              :key="providerProjectKey(group.cliId, project)"
              class="project-node nested-project-node"
              role="treeitem"
            >
              <div
                class="project-header nested-project-header"
                @click="toggleProviderProject(group.cliId, projectKey(project))"
                @mousedown.right.prevent.stop
                @contextmenu.prevent.stop="emit('contextMenuProject', $event, project)"
              >
                <span
                  v-if="isMultiSelectMode"
                  class="project-checkbox"
                  :class="{ checked: projectAllSelected(projectKey(project)) }"
                  @click.stop="emit('selectProjectSessions', projectKey(project))"
                ><span v-if="projectAllSelected(projectKey(project))" class="checkbox-mark">&#10003;</span></span>
                <SvgIcon
                  :name="isCollapsed(providerProjectKey(group.cliId, project)) ? 'chevron-right' : 'chevron-down'"
                  :size="14"
                  class="chevron-icon"
                  :class="{ rotated: !isCollapsed(providerProjectKey(group.cliId, project)) }"
                />
                <SvgIcon name="folder" :size="14" class="folder-icon" />
                <span class="project-path truncate" :title="project.original_path || t('session.unknownProject')">
                  <template v-for="(segment, index) in getHighlightedSegments(lastPathSegment(project.original_path))" :key="`${providerProjectKey(group.cliId, project)}-path-${index}`">
                    <mark v-if="segment.highlighted" class="search-highlight">{{ segment.text }}</mark>
                    <template v-else>{{ segment.text }}</template>
                  </template>
                </span>
                <button
                  class="hide-project-btn"
                  :title="t('session.sessionTree.hideFolder')"
                  @click.stop="blockFolder(project.original_path ?? '')"
                >
                  <SvgIcon name="eye-off" :size="12" />
                </button>
              </div>

              <div v-if="!isCollapsed(providerProjectKey(group.cliId, project))" class="session-list nested-session-list" role="group">
                <div
                  v-for="session in project.sessions"
                  :key="sessionKey(session)"
                  class="session-item"
                  :class="{ selected: isSelectedSession(session), 'multi-selected': isMultiSelected(session), 'context-target': isContextMenuSession(session) }"
                  role="treeitem"
                  @click="onSessionClick($event, session, project.encoded_dir ?? '')"
                  @dblclick="onSessionDoubleClick($event, session, project.encoded_dir ?? '')"
                  @mousedown.right.prevent.stop
                  @contextmenu.prevent.stop="onContextMenuSession($event, session, project.original_path ?? '')"
                >
                  <span
                    class="session-checkbox"
                    :class="{ checked: isMultiSelected(session) }"
                    @click.stop="onToggleSession(session, $event)"
                  ><span v-if="isMultiSelected(session)" class="checkbox-mark">&#10003;</span></span>
                  <ChatAvatar role="assistant" :cliId="session.cli_id" class="session-cli-avatar" />
                  <SvgIcon name="file-text" :size="13" class="file-icon" />
                  <div class="session-content">
                    <template v-if="renamingSessionKey === sessionKey(session)">
                      <input
                        class="session-rename-input"
                        v-model="renameInput"
                        autocomplete="off"
                        autocapitalize="off"
                        autocorrect="off"
                        spellcheck="false"
                        @keyup.enter="confirmRenameForSession(session)"
                        @keyup.escape="cancelRename"
                        @blur="confirmRenameForSession(session)"
                        @click.stop
                      />
                    </template>
                    <template v-else>
                      <span class="session-text truncate" :title="session.display_name">
                        <template v-for="(segment, index) in getHighlightedSegments(session.display_name)" :key="`${session.file_path}-label-${index}`">
                          <mark v-if="segment.highlighted" class="search-highlight">{{ segment.text }}</mark>
                          <template v-else>{{ segment.text }}</template>
                        </template>
                      </span>
                      <span class="session-meta">
                        <template v-if="session.timestamp">
                          <span class="session-time" :title="formatTimestamp(session.timestamp)">{{ formatRelativeTime(session.timestamp) }}</span>
                          <span class="meta-sep">·</span>
                        </template>
                        <template v-if="session.git_branch">
                          <span class="session-branch" :title="session.git_branch">{{ session.git_branch }}</span>
                          <span class="meta-sep">·</span>
                        </template>
                        <span class="session-size">{{ formatFileSize(session.file_size) }}</span>
                        <SvgIcon
                          v-if="isFavoriteSession(session)"
                          name="star"
                          :size="10"
                          class="favorite-star"
                          :title="t('session.sessionTree.favoriteStar')"
                        />
                        <span v-if="session.is_archived" class="archived-badge">{{ t("session.sessionTree.archived") }}</span>
                        <span v-else-if="session.has_archive_snapshot" class="snapshot-badge">{{ t("session.sessionTree.snapshot") }}</span>
                      </span>
                    </template>
                  </div>
                </div>
              </div>
            </div>
          </div>
        </div>
      </div>

      <!-- Directory Grouping Mode (Default) -->
      <div v-else key="directory-list" class="tree-content-container">
        <div v-for="project in visibleProjects" :key="projectKey(project)" class="project-node" role="treeitem">
          <div
            class="project-header"
            @click="toggleProject(projectKey(project))"
            @mousedown.right.prevent.stop
            @contextmenu.prevent.stop="emit('contextMenuProject', $event, project)"
          >
            <span
              v-if="isMultiSelectMode"
              class="project-checkbox"
              :class="{ checked: projectAllSelected(projectKey(project)) }"
              @click.stop="emit('selectProjectSessions', projectKey(project))"
            ><span v-if="projectAllSelected(projectKey(project))" class="checkbox-mark">&#10003;</span></span>
            <SvgIcon
              :name="isCollapsed(projectKey(project)) ? 'chevron-right' : 'chevron-down'"
              :size="14"
              class="chevron-icon"
              :class="{ rotated: !isCollapsed(projectKey(project)) }"
            />
            <SvgIcon name="folder" :size="14" class="folder-icon" />
            <span class="project-path truncate" :title="project.original_path || t('session.unknownProject')">
              <template v-for="(segment, index) in getHighlightedSegments(lastPathSegment(project.original_path))" :key="`${projectKey(project)}-path-${index}`">
                <mark v-if="segment.highlighted" class="search-highlight">{{ segment.text }}</mark>
                <template v-else>{{ segment.text }}</template>
              </template>
            </span>
            <button
              class="hide-project-btn"
              :title="t('session.sessionTree.hideFolder')"
              @click.stop="blockFolder(project.original_path ?? '')"
            >
              <SvgIcon name="eye-off" :size="12" />
            </button>
          </div>
          <div v-if="!isCollapsed(projectKey(project))" class="session-list" role="group">
            <div
              v-for="session in project.sessions"
              :key="sessionKey(session)"
              class="session-item"
              :class="{ selected: isSelectedSession(session), 'multi-selected': isMultiSelected(session), 'context-target': isContextMenuSession(session) }"
              role="treeitem"
              @click="onSessionClick($event, session, project.encoded_dir ?? '')"
              @dblclick="onSessionDoubleClick($event, session, project.encoded_dir ?? '')"
              @mousedown.right.prevent.stop
              @contextmenu.prevent.stop="onContextMenuSession($event, session, project.original_path ?? '')"
            >
              <span
                class="session-checkbox"
                :class="{ checked: isMultiSelected(session) }"
                @click.stop="onToggleSession(session, $event)"
              ><span v-if="isMultiSelected(session)" class="checkbox-mark">&#10003;</span></span>
              <ChatAvatar role="assistant" :cliId="session.cli_id" class="session-cli-avatar" />
              <SvgIcon name="file-text" :size="13" class="file-icon" />
              <div class="session-content">
                <template v-if="renamingSessionKey === sessionKey(session)">
                  <input
                    class="session-rename-input"
                    v-model="renameInput"
                    autocomplete="off"
                    autocapitalize="off"
                    autocorrect="off"
                    spellcheck="false"
                    @keyup.enter="confirmRenameForSession(session)"
                    @keyup.escape="cancelRename"
                    @blur="confirmRenameForSession(session)"
                    @click.stop
                  />
                </template>
                <template v-else>
                  <span class="session-text truncate" :title="session.display_name">
                    <template v-for="(segment, index) in getHighlightedSegments(session.display_name)" :key="`${session.file_path}-label-${index}`">
                      <mark v-if="segment.highlighted" class="search-highlight">{{ segment.text }}</mark>
                      <template v-else>{{ segment.text }}</template>
                    </template>
                  </span>
                  <span class="session-meta">
                    <template v-if="session.timestamp">
                      <span class="session-time" :title="formatTimestamp(session.timestamp)">{{ formatRelativeTime(session.timestamp) }}</span>
                      <span class="meta-sep">·</span>
                    </template>
                    <template v-if="session.git_branch">
                      <span class="session-branch" :title="session.git_branch">{{ session.git_branch }}</span>
                      <span class="meta-sep">·</span>
                    </template>
                    <span class="session-size">{{ formatFileSize(session.file_size) }}</span>
                    <SvgIcon
                      v-if="isFavoriteSession(session)"
                      name="star"
                      :size="10"
                      class="favorite-star"
                      :title="t('session.sessionTree.favoriteStar')"
                    />
                    <span v-if="session.is_archived" class="archived-badge">{{ t("session.sessionTree.archived") }}</span>
                    <span v-else-if="session.has_archive_snapshot" class="snapshot-badge">{{ t("session.sessionTree.snapshot") }}</span>
                  </span>
                </template>
              </div>
            </div>
          </div>
        </div>
      </div>
    </Transition>

  </div>
</template>

<style scoped>
.tree-loading-state {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 8px;
  margin: 15px 9px;
  padding: 22px 15px;
  border: 1px solid var(--color-border);
  border-radius: 14px;
  background: var(--color-bg-secondary);
  color: var(--color-text-secondary);
  text-align: center;
}
.tree-loading-state strong { color: var(--color-text); font-size: 13px; }
.tree-loading-state > span:last-child { font-size: 11px; line-height: 1.55; }
.tree-loading-icon {
  display: grid;
  place-items: center;
  width: 38px;
  height: 38px;
  margin-bottom: 4px;
  border-radius: 12px;
  background: color-mix(in srgb, var(--color-primary) 11%, var(--color-bg));
  color: var(--color-primary);
}
.tree-loading-icon :deep(svg) { animation: spin 1.2s linear infinite; }
.session-tree {
  flex: 1;
  overflow-y: auto;
  padding: 0 var(--space-1);
  -webkit-user-select: none;
  user-select: none;
  outline: none;
  display: flex;
  flex-direction: column;
}
.empty-hint {
  padding: var(--space-5);
  text-align: center;
  color: var(--color-text-muted);
  font-size: var(--text-sm);
}
.tree-content-container {
  display: flex;
  flex-direction: column;
  width: 100%;
}
.tree-crossfade-enter-active,
.tree-crossfade-leave-active {
  transition: opacity 0.24s cubic-bezier(0.16, 1, 0.3, 1), transform 0.24s cubic-bezier(0.16, 1, 0.3, 1);
}
.tree-crossfade-enter-from {
  opacity: 0;
  transform: translateY(4px);
}
.tree-crossfade-leave-to {
  opacity: 0;
  transform: translateY(-4px);
}
.session-tree:focus-visible {
  outline: none;
  box-shadow: inset 0 0 0 2px var(--color-primary-ring);
}
.project-node {
  margin-bottom: 2px;
}
.provider-node {
  margin-bottom: 2px;
}
.provider-avatar {
  width: 16px;
  height: 16px;
  min-width: 16px;
  flex-shrink: 0;
  margin-top: 1px;
}
.provider-avatar :deep(.avatar-svg) {
  width: 13px;
  height: 13px;
}
.provider-avatar :deep(.brand-svg) {
  width: 14px;
  height: 14px;
}
.provider-avatar :deep(.brand-svg-fill) {
  width: 16px;
  height: 16px;
}
.provider-name {
  min-width: 0;
  flex: 1;
  font-weight: 600;
}
.provider-count-capsule {
  display: inline-flex;
  align-items: center;
  justify-content: center;
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
  margin-left: auto;
  line-height: 1;
}
.provider-projects-list {
  padding-left: var(--space-3);
}
.nested-session-list {
  padding-left: var(--space-3);
}
.project-header {
  display: flex;
  align-items: center;
  min-height: 30px;
  height: 30px;
  box-sizing: border-box;
  padding: 0 8px;
  margin: 1px 6px;
  cursor: pointer;
  border-radius: 7px;
  font-size: var(--text-sm);
  font-weight: 500;
  gap: var(--space-1);
  color: var(--color-text);
  transition: background var(--transition-fast);
}
.project-header:hover {
  background: var(--color-surface-selected);
}
.chevron-icon {
  color: var(--color-text-muted);
  transition: transform var(--transition-fast);
}
.folder-icon {
  color: var(--color-text-secondary);
  flex-shrink: 0;
}
.project-path {
  min-width: 0;
  flex: 1;
}
.session-list {
  padding-left: var(--space-4);
  overflow: hidden;
}
.tree-expand-enter-active,
.tree-expand-leave-active {
  transition: max-height 180ms ease, opacity 150ms ease, transform 180ms ease;
}
.tree-expand-enter-from,
.tree-expand-leave-to {
  max-height: 0;
  opacity: 0;
  transform: translateY(-4px);
}
.tree-expand-enter-to,
.tree-expand-leave-from {
  max-height: 1200px;
  opacity: 1;
  transform: translateY(0);
}
.session-item {
  display: flex;
  align-items: flex-start;
  min-height: 30px;
  box-sizing: border-box;
  padding: 4px 8px;
  margin: 1px 6px;
  cursor: pointer;
  border-radius: 7px;
  font-size: var(--text-xs);
  gap: var(--space-1);
  color: var(--color-text-secondary);
  transition: background var(--transition-fast), color var(--transition-fast);
}
.session-content {
  display: flex;
  flex-direction: column;
  min-width: 0;
  flex: 1;
  gap: 1px;
}
.session-meta {
  font-size: 10px;
  color: var(--color-text-secondary);
  display: flex;
  align-items: center;
  gap: 4px;
  /* 元信息行永不换行：分支名让位截断，时间/大小/徽章固定 */
  white-space: nowrap;
  overflow: hidden;
}
.meta-sep {
  flex-shrink: 0;
  color: var(--color-text-muted);
}
.session-branch {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  /* 分支前缀多为 feature/ bugfix/ 等样板词，从左侧截断保留有区分度的尾部 */
  direction: rtl;
  text-align: left;
  flex-shrink: 1;
}
.session-size {
  flex-shrink: 0;
  font-variant-numeric: tabular-nums;
}
.archived-badge,
.snapshot-badge {
  background: var(--color-bg-hover);
  border: 1px solid var(--color-border);
  border-radius: 2px;
  padding: 0 3px;
  font-size: 9px;
  color: var(--color-text-muted);
  flex-shrink: 0;
}
.favorite-star {
  color: var(--color-warning);
  fill: currentColor;
  flex-shrink: 0;
}
.snapshot-badge {
  color: var(--color-text-secondary);
}
.session-item:hover {
  background: var(--color-bg-hover);
  color: var(--color-text);
}
.session-item.selected {
  background: var(--color-primary-light);
  color: var(--color-primary);
}
.session-item.context-target:not(.selected):not(.multi-selected) {
  background: var(--color-bg-hover);
}
.session-item.multi-selected {
  background: var(--color-primary-light);
  color: var(--color-primary);
}
.session-item.multi-selected .file-icon {
  color: var(--color-primary);
}
.file-icon {
  flex-shrink: 0;
  color: var(--color-text-secondary);
  margin-top: 2px;
}
.session-cli-avatar {
  width: 14px;
  height: 14px;
  margin-top: 1px;
  flex-shrink: 0;
}
.session-cli-avatar :deep(.avatar-svg) {
  width: 11px;
  height: 11px;
}
.session-cli-avatar :deep(.brand-svg-fill) {
  width: 14px;
  height: 14px;
}
.session-checkbox {
  display: none;
  align-items: center;
  justify-content: center;
  width: 14px;
  height: 14px;
  min-width: 14px;
  border: 1.5px solid var(--color-text-muted);
  border-radius: 3px;
  margin-top: 2px;
  cursor: pointer;
  transition: all var(--transition-fast);
  flex-shrink: 0;
}
.session-checkbox:hover {
  border-color: var(--color-primary);
}
.session-checkbox.checked {
  display: flex;
  background: var(--color-primary);
  border-color: var(--color-primary);
}
.checkbox-mark {
  font-size: 10px;
  line-height: 1;
  color: white;
  font-weight: 700;
}
.session-item.multi-selected .session-checkbox {
  display: flex;
}
.multi-select-mode .session-checkbox {
  display: flex;
}
.project-checkbox {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 14px;
  height: 14px;
  min-width: 14px;
  border: 1.5px solid var(--color-text-muted);
  border-radius: 3px;
  cursor: pointer;
  transition: all var(--transition-fast);
  flex-shrink: 0;
}
.project-checkbox:hover {
  border-color: var(--color-primary);
}
.project-checkbox.checked {
  background: var(--color-primary);
  border-color: var(--color-primary);
}
.session-item.selected .file-icon {
  color: var(--color-primary);
}
.session-item.context-target .session-meta,
.session-item.context-target .flat-item-project {
  color: var(--color-text-secondary);
}
.hide-project-btn {
  display: flex;
  align-items: center;
  padding: 4px;
  color: var(--color-text-muted);
  border-radius: var(--radius-sm);
  opacity: 0;
  transition: opacity var(--transition-fast), background var(--transition-fast), color var(--transition-fast);
  flex-shrink: 0;
}
.project-header:hover .hide-project-btn {
  opacity: 1;
}
.hide-project-btn:hover {
  color: var(--color-warning);
  background: var(--color-bg-hover);
}
.session-text {
  min-width: 0;
  user-select: none;
  -webkit-user-select: none;
  /* 标题独占首行：加重字重与主色，与时间/分支等元信息拉开层级 */
  font-weight: 500;
  color: var(--color-text);
  letter-spacing: 0.01em;
}
.session-time {
  flex-shrink: 0;
  font-variant-numeric: tabular-nums;
}
/* 选中/多选行标题跟随整行 primary 色 */
.session-item.selected .session-text,
.session-item.multi-selected .session-text {
  color: var(--color-primary);
}
.search-highlight {
  background: var(--color-warning);
  color: var(--color-bg);
  border-radius: 2px;
  padding: 0 1px;
}
.session-rename-input {
  flex: 1;
  min-width: 0;
  font-size: var(--text-xs);
  font-weight: 500;
  padding: 1px 4px;
  border: 1px solid var(--color-primary);
  border-radius: var(--radius-sm);
  background: var(--color-bg);
  color: var(--color-text);
  outline: none;
}
@keyframes locate-flash {
  0% { background-color: transparent; }
  20% { background-color: var(--color-primary-light); }
  100% { background-color: transparent; }
}
.locate-highlight {
  animation: locate-flash 1.5s ease-out;
}
.session-tree-streaming-indicator {
  padding: var(--space-2) var(--space-3);
  margin-bottom: var(--space-2);
  font-size: var(--text-xs);
  color: var(--color-text-secondary);
  background: var(--color-bg-secondary);
  border: 1px solid var(--color-border);
  border-radius: var(--radius-lg);
  -webkit-user-select: none;
  user-select: none;
}
</style>

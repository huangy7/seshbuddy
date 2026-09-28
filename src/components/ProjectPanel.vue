<script setup lang="ts">
import { ref, computed, watch, nextTick, onMounted, onBeforeUnmount } from "vue";
import { invokeApp } from "../utils/invokeApp";
import { copyToClipboard } from "../utils/clipboard";
import { ask, open as openDialog } from "@tauri-apps/plugin-dialog";
import SvgIcon from "./icons/SvgIcon.vue";
import FileTreeNode from "./FileTreeNode.vue";
import type { ProjectInfo } from "../types/session";
import { useWorkspaces } from "../composables/useWorkspaces";
import { useBlockedFolders } from "../composables/useBlockedFolders";
import { findBestProjectPath, basename } from "../utils/projectPath";
import { t } from "../i18n";

interface FileEntry {
  name: string;
  path: string;
  isDir: boolean;
  children?: FileEntry[];
}

const props = defineProps<{
  projects: ProjectInfo[];
  manualPaths: string[];
  activeProjectPath?: string | null;
  activeFilePath?: string | null;
}>();

const emit = defineEmits<{
  openFile: [path: string, projectRoot: string];
  pinFile: [path: string, projectRoot: string];
  newSession: [projectPath: string];
}>();

// ─── State ───────────────────────────────────────────────────────────────────

const filterText = ref("");
const selectedProjectOverride = ref<string | null>(null);
const showProjectPicker = ref(false);
const pickerTriggerRef = ref<HTMLButtonElement | null>(null);

const expandedDirs = ref<Set<string>>(new Set());
const fileTree = ref<Map<string, FileEntry[]>>(new Map());
const loadingProjects = ref<Set<string>>(new Set());
const refreshing = ref(false);

const ctxMenu = ref<{ x: number; y: number; entry: FileEntry | null; projectRoot: string } | null>(null);

const { addWorkspace, mergedProjectPaths } = useWorkspaces();
const { isBlocked: isFolderBlocked, blockFolder } = useBlockedFolders();

// ─── Computed project paths & current active project root ─────────────────────

const projectPaths = computed(() => {
  const paths = mergedProjectPaths(props.projects).filter((p) => !isFolderBlocked(p));
  if (props.activeProjectPath && !paths.includes(props.activeProjectPath)) {
    return [props.activeProjectPath, ...paths];
  }
  return paths;
});

const activeProjectRoot = computed<string>(() => {
  if (selectedProjectOverride.value) {
    return selectedProjectOverride.value;
  }
  if (props.activeProjectPath) {
    return props.activeProjectPath;
  }
  if (props.activeFilePath) {
    const matched = findBestProjectPath(props.activeFilePath, projectPaths.value);
    if (matched) return matched;
  }
  return projectPaths.value[0] || "";
});

watch(() => props.activeProjectPath, () => {
  selectedProjectOverride.value = null;
});

function projectName(path: string): string {
  if (!path) return "";
  return basename(path) || path;
}

// ─── Load files for active project ────────────────────────────────────────────

async function loadFiles(projectPath: string) {
  if (!projectPath) return;
  loadingProjects.value.add(projectPath);
  try {
    const entries = await invokeApp<FileEntry[]>("list_project_files", {
      projectPath,
      projectRoot: projectPath,
      filter: null,
    });
    fileTree.value.set(projectPath, entries);
  } catch (e) {
    console.error("list_project_files failed", e);
  } finally {
    loadingProjects.value.delete(projectPath);
  }
}

watch(activeProjectRoot, async (newRoot) => {
  if (newRoot && !fileTree.value.has(newRoot)) {
    await loadFiles(newRoot);
  }
}, { immediate: true });

function filterEntries(entries: FileEntry[], query: string): FileEntry[] {
  if (!query) return entries;
  const q = query.toLowerCase();
  const result: FileEntry[] = [];
  for (const entry of entries) {
    if (entry.isDir) {
      if (entry.name.toLowerCase().includes(q)) {
        result.push(entry);
      } else {
        const children = filterEntries(entry.children || [], query);
        if (children.length > 0) {
          result.push({ ...entry, children });
        }
      }
    } else {
      if (entry.name.toLowerCase().includes(q)) {
        result.push(entry);
      }
    }
  }
  return result;
}

const currentProjectEntries = computed(() => {
  const root = activeProjectRoot.value;
  if (!root) return [];
  const entries = fileTree.value.get(root) ?? [];
  return filterEntries(entries, filterText.value.trim());
});

function toggleDir(path: string) {
  if (expandedDirs.value.has(path)) {
    expandedDirs.value.delete(path);
  } else {
    expandedDirs.value.add(path);
  }
}

function collectAllDirPaths(entries: FileEntry[]): string[] {
  const dirs: string[] = [];
  for (const entry of entries) {
    if (entry.isDir) {
      dirs.push(entry.path);
      if (entry.children) {
        dirs.push(...collectAllDirPaths(entry.children));
      }
    }
  }
  return dirs;
}

const allExpanded = computed(() => {
  return expandedDirs.value.size > 0;
});

function expandAllDirs() {
  const dirs = collectAllDirPaths(currentProjectEntries.value);
  for (const d of dirs) {
    expandedDirs.value.add(d);
  }
}

function collapseAllDirs() {
  expandedDirs.value.clear();
}

async function onRefresh() {
  const root = activeProjectRoot.value;
  if (!root) return;
  refreshing.value = true;
  const minDelay = new Promise((r) => setTimeout(r, 400));
  try {
    await loadFiles(root);
  } finally {
    await minDelay;
    refreshing.value = false;
  }
}

// ─── Project Picker Dropdown ──────────────────────────────────────────────────

function toggleProjectPicker() {
  showProjectPicker.value = !showProjectPicker.value;
}

function selectProjectRoot(path: string) {
  selectedProjectOverride.value = path;
  showProjectPicker.value = false;
  if (!fileTree.value.has(path)) {
    loadFiles(path);
  }
}

function onDocumentClick(e: MouseEvent) {
  const target = e.target as HTMLElement | null;
  if (showProjectPicker.value && !target?.closest(".project-breadcrumb-wrapper")) {
    showProjectPicker.value = false;
  }
}

onMounted(() => {
  document.addEventListener("click", onDocumentClick);
});

onBeforeUnmount(() => {
  document.removeEventListener("click", onDocumentClick);
});

// ─── Locate current file ─────────────────────────────────────────────────────

const canLocateFile = computed(() => {
  if (!props.activeFilePath || !activeProjectRoot.value) return false;
  return props.activeFilePath.startsWith(activeProjectRoot.value);
});

async function locateFile() {
  const filePath = props.activeFilePath;
  const root = activeProjectRoot.value;
  if (!filePath || !root || !filePath.startsWith(root)) return;

  const relativePath = filePath.slice(root.length + 1);
  const segments = relativePath.split(/[/\\]/);
  const sep = filePath.includes("\\") ? "\\" : "/";
  let current = root;
  for (let i = 0; i < segments.length - 1; i++) {
    current += sep + segments[i];
    expandedDirs.value.add(current);
  }

  await nextTick();

  const fileEl = document.querySelector(`[data-path="${CSS.escape(filePath)}"]`) as HTMLElement | null;
  if (!fileEl) return;
  fileEl.scrollIntoView({ behavior: "smooth", block: "center" });
  fileEl.classList.add("locate-highlight");
  fileEl.addEventListener("animationend", () => {
    fileEl.classList.remove("locate-highlight");
  }, { once: true });
}

// ─── Context menu ─────────────────────────────────────────────────────────────

function openCtxMenu(e: MouseEvent, entry: FileEntry | null, projectRoot: string) {
  e.preventDefault();
  e.stopPropagation();
  window.getSelection()?.removeAllRanges();
  ctxMenu.value = { x: e.clientX, y: e.clientY, entry, projectRoot };
}

function closeCtxMenu() {
  ctxMenu.value = null;
}

watch(ctxMenu, (newVal) => {
  if (newVal) {
    document.addEventListener("mousedown", handleCtxClickOutside);
  } else {
    document.removeEventListener("mousedown", handleCtxClickOutside);
  }
});

function handleCtxClickOutside(e: MouseEvent) {
  const target = e.target as HTMLElement | null;
  if (target && !target.closest(".ctx-menu")) {
    closeCtxMenu();
  }
}

onBeforeUnmount(() => {
  document.removeEventListener("mousedown", handleCtxClickOutside);
});

async function ctxDelete() {
  if (!ctxMenu.value?.entry) return;
  const { entry, projectRoot } = ctxMenu.value;
  closeCtxMenu();
  const confirmed = await ask(t("session.projectPanel.deleteConfirm", { name: entry.name }), {
    title: t("session.projectPanel.deleteTitle"),
    kind: "warning",
    okLabel: t("session.projectPanel.moveToTrash"),
    cancelLabel: t("dialogs.common.cancel"),
  });
  if (!confirmed) return;
  await invokeApp("delete_file", { path: entry.path, projectRoot });
  await loadFiles(projectRoot);
}

async function ctxCopyAbsPath() {
  if (!ctxMenu.value) return;
  const path = ctxMenu.value.entry?.path ?? ctxMenu.value.projectRoot;
  await copyToClipboard(path);
  closeCtxMenu();
}

async function ctxReveal() {
  if (!ctxMenu.value) return;
  const path = ctxMenu.value.entry?.path ?? ctxMenu.value.projectRoot;
  closeCtxMenu();
  try {
    await invokeApp("open_path_in_file_manager", { path });
  } catch {
    await copyToClipboard(path);
  }
}

async function ctxBlockFolder() {
  if (!ctxMenu.value) return;
  const path = ctxMenu.value.entry?.path ?? ctxMenu.value.projectRoot;
  closeCtxMenu();
  await blockFolder(path);
}

// ─── Add Workspace ────────────────────────────────────────────────────────────

async function onAddWorkspace() {
  showProjectPicker.value = false;
  const selected = await openDialog({ directory: true, multiple: false, title: t("session.projectPanel.chooseWorkspaceDir") });
  if (selected && typeof selected === "string") {
    addWorkspace(selected);
    selectedProjectOverride.value = selected;
  }
}
</script>

<template>
  <div class="project-panel" @click="closeCtxMenu">
    <!-- Clean Sub-Header: Project Breadcrumb + Actions -->
    <div class="project-toolbar">
      <div class="project-breadcrumb-wrapper">
        <button
          ref="pickerTriggerRef"
          class="project-breadcrumb-trigger"
          type="button"
          aria-haspopup="menu"
          :aria-expanded="showProjectPicker"
          :title="t('session.projectPanel.switchWorkspace')"
          @click.stop="toggleProjectPicker"
        >
          <SvgIcon name="folder" :size="13" class="project-folder-icon" />
          <span class="project-current-name">{{ projectName(activeProjectRoot) || t("session.projectPanel.noProject") }}</span>
          <SvgIcon name="chevron-down" :size="11" class="picker-chevron" />
        </button>

        <Transition name="fade-scale">
          <div
            v-if="showProjectPicker"
            class="project-picker-menu menu-surface"
            role="menu"
            :aria-label="t('session.projectPanel.workspaceMenuAria')"
          >
            <div class="picker-menu-header">{{ t("session.projectPanel.workspaceProjects") }}</div>
            <div class="picker-menu-list">
              <button
                v-for="p in projectPaths"
                :key="p"
                class="project-picker-item"
                :class="{ active: p === activeProjectRoot }"
                role="menuitem"
                type="button"
                @click="selectProjectRoot(p)"
              >
                <SvgIcon name="folder" :size="13" />
                <span class="picker-item-name" :title="p">{{ projectName(p) }}</span>
                <span v-if="p === activeProjectRoot" class="active-check">✓</span>
              </button>
            </div>
            <div class="picker-menu-divider"></div>
            <button class="picker-add-btn" role="menuitem" type="button" @click="onAddWorkspace">
              <SvgIcon name="plus" :size="13" />
              <span>{{ t("session.projectPanel.addWorkspaceDir") }}</span>
            </button>
          </div>
        </Transition>
      </div>

      <div class="toolbar-icon-group">
        <button
          class="panel-icon-btn"
          :title="t('session.projectPanel.newSessionHere')"
          :disabled="!activeProjectRoot"
          type="button"
          @click="emit('newSession', activeProjectRoot)"
        >
          <SvgIcon name="plus" :size="14" />
        </button>
        <button
          class="panel-icon-btn"
          :title="canLocateFile ? t('session.historyPanel.locateCurrent') : t('session.projectPanel.noLinkedFile')"
          :disabled="!canLocateFile"
          type="button"
          @click="locateFile"
        >
          <SvgIcon name="crosshair" :size="14" />
        </button>
        <button
          class="panel-icon-btn"
          :title="allExpanded ? t('session.projectPanel.collapseAllDirs') : t('session.projectPanel.expandAllDirs')"
          :disabled="!activeProjectRoot || currentProjectEntries.length === 0"
          type="button"
          @click="allExpanded ? collapseAllDirs() : expandAllDirs()"
        >
          <SvgIcon :name="allExpanded ? 'collapse-all' : 'expand-all'" :size="14" />
        </button>
        <button
          class="panel-icon-btn"
          :title="t('session.projectPanel.refresh')"
          :disabled="refreshing || !activeProjectRoot"
          type="button"
          @click="onRefresh"
        >
          <span :class="{ spinning: refreshing }" class="refresh-icon-wrap">
            <SvgIcon :name="refreshing ? 'loader' : 'refresh-cw'" :size="14" />
          </span>
        </button>
      </div>
    </div>

    <!-- Search Input (Clean & Full Width, ChatGPT Style) -->
    <div class="project-search-container">
      <div class="project-search-box">
        <SvgIcon name="search" :size="13" class="search-input-icon" />
        <input
          v-model="filterText"
          type="text"
          class="project-filter-input"
          :placeholder="t('session.projectPanel.searchPlaceholder')"
          :aria-label="t('session.projectPanel.searchAria')"
          autocomplete="off"
          autocorrect="off"
          autocapitalize="off"
          spellcheck="false"
        />
        <button
          v-if="filterText"
          type="button"
          class="search-clear-btn"
          :title="t('session.projectPanel.clear')"
          @click="filterText = ''"
        >
          <SvgIcon name="x" :size="11" />
        </button>
      </div>
    </div>

    <!-- Single Project File Tree -->
    <div class="project-tree-body">
      <div v-if="loadingProjects.has(activeProjectRoot)" class="tree-loading">
        {{ t("session.projectPanel.loadingFiles") }}
      </div>
      <div v-else-if="!activeProjectRoot" class="empty-hint">
        {{ t("session.projectPanel.noProjectDir") }}<br />{{ t("session.projectPanel.noProjectDirHint") }}
      </div>
      <div v-else-if="currentProjectEntries.length === 0 && !filterText" class="empty-hint">
        {{ t("session.projectPanel.emptyProject") }}
      </div>
      <div v-else-if="currentProjectEntries.length === 0 && filterText" class="tree-empty">
        {{ t("session.projectPanel.noMatchingFiles") }}
      </div>
      <div v-else class="file-tree-root">
        <FileTreeNode
          v-for="entry in currentProjectEntries"
          :key="entry.path"
          :entry="entry"
          :projectRoot="activeProjectRoot"
          :expandedDirs="expandedDirs"
          :depth="0"
          :filterText="filterText"
          @openFile="(path: string, root: string) => emit('openFile', path, root)"
          @pinFile="(path: string, root: string) => emit('pinFile', path, root)"
          @toggleDir="toggleDir"
          @contextMenu="(e, en, root) => openCtxMenu(e, en, root)"
        />
      </div>
    </div>

    <!-- Context menu -->
    <Teleport to="body">
      <div
        v-if="ctxMenu"
        class="ctx-menu"
        :style="{ left: ctxMenu.x + 'px', top: ctxMenu.y + 'px' }"
        @click.stop
      >
        <button
          v-if="ctxMenu.entry && !ctxMenu.entry.isDir"
          class="ctx-item"
          type="button"
          @click="emit('openFile', ctxMenu.entry!.path, ctxMenu.projectRoot); closeCtxMenu()"
        >
          {{ t("session.projectPanel.openFile") }}
        </button>
        <button class="ctx-item" type="button" @click="ctxReveal">{{ t("session.projectPanel.revealInFileManager") }}</button>
        <button class="ctx-item" type="button" @click="ctxCopyAbsPath">{{ t("session.projectPanel.copyAbsPath") }}</button>
        <template v-if="ctxMenu.entry">
          <button
            v-if="ctxMenu.entry.isDir"
            class="ctx-item"
            type="button"
            @click="ctxBlockFolder"
          >
            {{ t("session.projectPanel.blockFolder") }}
          </button>
          <div class="ctx-sep" />
          <button class="ctx-item ctx-danger" type="button" @click="ctxDelete">{{ t("app.batch.delete") }}</button>
        </template>
      </div>
    </Teleport>
  </div>
</template>

<style scoped>
.project-panel {
  display: flex;
  flex-direction: column;
  height: 100%;
  overflow: hidden;
  -webkit-user-select: none;
  user-select: none;
  background: var(--color-bg-sidebar);
}

.project-toolbar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 8px 10px 6px;
  gap: 8px;
  flex-shrink: 0;
}

.project-breadcrumb-wrapper {
  position: relative;
  min-width: 0;
  flex: 1;
}

.project-breadcrumb-trigger {
  display: flex;
  align-items: center;
  gap: 6px;
  max-width: 100%;
  padding: 3px 6px;
  background: transparent;
  border: 1px solid transparent;
  border-radius: var(--radius-md, 6px);
  color: var(--color-text);
  font-size: var(--text-xs, 12px);
  font-weight: 600;
  cursor: pointer;
  transition: all var(--transition-fast);
}

.project-breadcrumb-trigger:hover {
  background: var(--color-bg-hover);
  border-color: var(--color-border);
}

.project-folder-icon {
  color: var(--color-primary);
  flex-shrink: 0;
}

.project-current-name {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  flex: 1;
  text-align: left;
}

.picker-chevron {
  color: var(--color-text-muted);
  flex-shrink: 0;
}

.toolbar-icon-group {
  display: flex;
  align-items: center;
  gap: 2px;
  flex-shrink: 0;
}

.panel-icon-btn {
  width: 24px;
  height: 24px;
  padding: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  background: transparent;
  border: none;
  border-radius: var(--radius-sm, 4px);
  color: var(--color-text-muted);
  cursor: pointer;
  transition: all var(--transition-fast);
}

.panel-icon-btn:hover:not(:disabled) {
  background: var(--color-bg-hover);
  color: var(--color-text);
}

.panel-icon-btn:disabled {
  opacity: 0.35;
  cursor: not-allowed;
}

.project-search-container {
  padding: 2px 10px 8px;
  border-bottom: 1px solid var(--color-border-light, var(--color-border));
  flex-shrink: 0;
}

.project-search-box {
  display: flex;
  align-items: center;
  background: var(--color-bg, rgba(0, 0, 0, 0.03));
  border: 1px solid var(--color-border);
  border-radius: var(--radius-md, 6px);
  padding: 3px 8px;
  gap: 6px;
  transition: border-color var(--transition-fast);
}

.project-search-box:focus-within {
  border-color: var(--color-primary);
}

.search-input-icon {
  color: var(--color-text-muted);
  flex-shrink: 0;
}

.project-filter-input {
  flex: 1;
  border: none;
  background: transparent;
  outline: none;
  font-size: 11px;
  color: var(--color-text);
  min-width: 0;
}

.project-filter-input::placeholder {
  color: var(--color-text-muted);
}

.search-clear-btn {
  padding: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  background: transparent;
  border: none;
  color: var(--color-text-muted);
  cursor: pointer;
}

.search-clear-btn:hover {
  color: var(--color-text);
}

.project-tree-body {
  flex: 1;
  overflow-y: auto;
  padding: 4px 6px;
}

.file-tree-root {
  display: flex;
  flex-direction: column;
}

.tree-loading,
.tree-empty {
  font-size: var(--text-xs, 12px);
  color: var(--color-text-muted);
  padding: var(--space-3) var(--space-2);
  text-align: center;
}

.empty-hint {
  font-size: var(--text-xs, 12px);
  color: var(--color-text-muted);
  padding: var(--space-6) var(--space-3);
  text-align: center;
  line-height: 1.6;
}

/* Project Picker Menu */
.project-picker-menu {
  position: absolute;
  top: calc(100% + 4px);
  left: 0;
  z-index: 200;
  width: 240px;
  max-height: 300px;
  overflow-y: auto;
  background: var(--color-bg-elevated, var(--color-bg));
  border: 1px solid var(--color-border);
  border-radius: var(--radius-lg, 8px);
  box-shadow: var(--shadow-md);
  padding: 4px;
}

.picker-menu-header {
  font-size: 11px;
  font-weight: 600;
  color: var(--color-text-muted);
  padding: 4px 8px 2px;
}

.picker-menu-list {
  display: flex;
  flex-direction: column;
  gap: 1px;
}

.project-picker-item {
  display: flex;
  align-items: center;
  gap: 6px;
  width: 100%;
  padding: 6px 8px;
  border: none;
  background: transparent;
  border-radius: var(--radius-sm, 4px);
  color: var(--color-text);
  font-size: var(--text-xs, 12px);
  text-align: left;
  cursor: pointer;
  transition: background var(--transition-fast);
}

.project-picker-item:hover {
  background: var(--color-bg-hover);
}

.project-picker-item.active {
  color: var(--color-primary);
  font-weight: 500;
  background: var(--color-primary-light, rgba(59, 130, 246, 0.1));
}

.picker-item-name {
  flex: 1;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.active-check {
  color: var(--color-primary);
  font-weight: bold;
}

.picker-menu-divider {
  height: 1px;
  background: var(--color-border);
  margin: 4px 0;
  opacity: 0.6;
}

.picker-add-btn {
  display: flex;
  align-items: center;
  gap: 6px;
  width: 100%;
  padding: 6px 8px;
  border: none;
  background: transparent;
  border-radius: var(--radius-sm, 4px);
  color: var(--color-text-muted);
  font-size: var(--text-xs, 12px);
  cursor: pointer;
  transition: all var(--transition-fast);
}

.picker-add-btn:hover {
  color: var(--color-primary);
  background: var(--color-bg-hover);
}

/* Context menu */
.ctx-menu {
  position: fixed;
  z-index: 9999;
  background: var(--color-bg-elevated, var(--color-bg));
  border: 1px solid var(--color-border);
  border-radius: var(--radius-md);
  box-shadow: var(--shadow-context);
  min-width: 160px;
  padding: var(--space-1) 0;
}

.ctx-item {
  display: flex;
  align-items: center;
  width: 100%;
  padding: var(--space-1) var(--space-3);
  font-size: var(--text-xs);
  color: var(--color-text);
  background: transparent;
  border: none;
  text-align: left;
  cursor: pointer;
  transition: background var(--transition-fast);
}

.ctx-item:hover {
  background: var(--color-bg-hover);
}

.ctx-danger {
  color: var(--color-danger);
}

.ctx-danger:hover {
  background: rgba(220, 38, 38, 0.1);
}

.ctx-sep {
  height: 1px;
  background: var(--color-border);
  margin: var(--space-1) 0;
}

.refresh-icon-wrap.spinning {
  display: inline-flex;
  animation: spin 0.8s linear infinite;
}

@keyframes spin {
  from { transform: rotate(0deg); }
  to { transform: rotate(360deg); }
}
</style>
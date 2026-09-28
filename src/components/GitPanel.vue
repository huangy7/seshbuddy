<script setup lang="ts">
import { ref, computed } from "vue";
import { invokeApp } from "../utils/invokeApp";
import { copyToClipboard } from "../utils/clipboard";
import { ask } from "@tauri-apps/plugin-dialog";
import SvgIcon from "./icons/SvgIcon.vue";
import { currentLocale, t } from "../i18n";
import { formatRelativeTime } from "../utils/format";
import { useGitState, type GitStatusEntry, type CommitDiffFile, type CommitDiffResult } from "../composables/useGitState";

const emit = defineEmits<{
  openDiff: [filePath: string, projectRoot: string, original: string, modified: string, label: string];
  openCommitDiff: [filePath: string, projectRoot: string, hash: string, label: string];
}>();

const { contextPath, isGitRepo, statusEntries, commits, branches, loading, includeIgnored, refreshAll } = useGitState();

type SubTab = "changes" | "commits" | "branches";
const activeSubTab = ref<SubTab>("changes");

// ─── Changes view options ───────────────────────────────────────────

const groupByDir = ref(false);

function toggleShowIgnored() {
  includeIgnored.value = !includeIgnored.value;
  refreshAll();
}

// ─── Flat renderable list for directory tree ────────────────────────

interface FlatRow {
  type: "dir" | "file";
  depth: number;
  dirPath?: string;
  dirName?: string;
  fileCount?: number;
  entry?: GitStatusEntry;
  fileName?: string;
}

const expandedDirs = ref(new Set<string>());

function toggleDir(path: string) {
  const next = new Set(expandedDirs.value);
  if (next.has(path)) {
    next.delete(path);
  } else {
    next.add(path);
  }
  expandedDirs.value = next;
}

const flatRows = computed<FlatRow[]>(() => {
  if (!groupByDir.value) return [];

  interface DirNode {
    name: string;
    fullPath: string;
    children: Map<string, DirNode>;
    files: GitStatusEntry[];
  }

  const root: DirNode = { name: "", fullPath: "", children: new Map(), files: [] };
  for (const entry of statusEntries.value) {
    const parts = entry.path.split("/");
    let node = root;
    for (let i = 0; i < parts.length - 1; i++) {
      const dirName = parts[i];
      if (!node.children.has(dirName)) {
        node.children.set(dirName, {
          name: dirName,
          fullPath: parts.slice(0, i + 1).join("/"),
          children: new Map(),
          files: [],
        });
      }
      node = node.children.get(dirName)!;
    }
    node.files.push(entry);
  }

  function countFiles(node: DirNode): number {
    let count = node.files.length;
    for (const child of node.children.values()) count += countFiles(child);
    return count;
  }

  const rows: FlatRow[] = [];

  function walk(node: DirNode, depth: number) {
    const sortedChildren = [...node.children.values()].sort((a, b) => a.name.localeCompare(b.name, currentLocale.value));
    const sortedFiles = [...node.files].sort((a, b) => a.path.localeCompare(b.path, currentLocale.value));

    for (const child of sortedChildren) {
      rows.push({
        type: "dir",
        depth,
        dirPath: child.fullPath,
        dirName: child.name,
        fileCount: countFiles(child),
      });
      if (expandedDirs.value.has(child.fullPath)) {
        walk(child, depth + 1);
      }
    }
    for (const entry of sortedFiles) {
      rows.push({
        type: "file",
        depth,
        entry,
        fileName: entry.path.split("/").pop() ?? entry.path,
      });
    }
  }

  walk(root, 0);
  return rows;
});

// ─── Changes ────────────────────────────────────────────────────────

async function openFileDiff(filePath: string) {
  if (!contextPath.value) return;
  try {
    const result = await invokeApp<{ original: string; modified: string; too_large: boolean }>(
      "git_diff_file_content",
      { path: contextPath.value, file: filePath }
    );
    if (result.too_large) {
      await ask(t("session.gitPanel.diffTooLarge"), {
        title: t("app.dialog.notice"),
        kind: "info",
        okLabel: t("dialogs.input.confirm"),
        cancelLabel: "",
      });
      return;
    }
    const label = t("session.gitPanel.diffLabel", { name: filePath.split("/").pop() });
    emit("openDiff", filePath, contextPath.value, result.original, result.modified, label);
  } catch (e: any) {
    console.error("openFileDiff failed:", e);
  }
}

// ─── Commits ────────────────────────────────────────────────────────

const expandedCommit = ref<string | null>(null);
const commitFiles = ref<CommitDiffFile[]>([]);
const commitSearchQuery = ref("");

const filteredCommits = computed(() => {
  const q = commitSearchQuery.value.trim().toLowerCase();
  if (!q) return commits.value;
  return commits.value.filter((commit) => {
    return (
      commit.message.toLowerCase().includes(q) ||
      commit.hash.toLowerCase().includes(q) ||
      commit.short_hash.toLowerCase().includes(q) ||
      commit.author.toLowerCase().includes(q) ||
      commit.email.toLowerCase().includes(q)
    );
  });
});

async function toggleCommit(hash: string) {
  if (expandedCommit.value === hash) {
    expandedCommit.value = null;
    commitFiles.value = [];
    return;
  }
  if (!contextPath.value) return;
  try {
    const result = await invokeApp<CommitDiffResult>("git_commit_diff", { path: contextPath.value, hash });
    expandedCommit.value = hash;
    commitFiles.value = result.files;
  } catch (e) {
    console.error("commit diff failed:", e);
  }
}

async function openCommitFileDiff(hash: string, filePath: string) {
  if (!contextPath.value) return;
  const shortHash = hash.substring(0, 7);
  const label = `${filePath.split("/").pop()} ← ${shortHash}`;
  emit("openCommitDiff", filePath, contextPath.value, hash, label);
}

function parseCommitMessage(message: string): { summary: string; body: string | null } {
  const lines = message.split("\n");
  const summary = lines[0]?.trim() || "";
  const rest = lines.slice(1).join("\n").trim();
  return {
    summary,
    body: rest.length > 0 ? rest : null,
  };
}

const copiedHash = ref<string | null>(null);
const copiedMsg = ref<string | null>(null);

async function copyCommitHash(hash: string, e?: Event) {
  e?.stopPropagation();
  if (await copyToClipboard(hash)) {
    copiedHash.value = hash;
    setTimeout(() => {
      if (copiedHash.value === hash) copiedHash.value = null;
    }, 1500);
  } else {
    console.error("copy hash failed");
  }
}

async function copyCommitMessage(message: string, e?: Event) {
  e?.stopPropagation();
  if (await copyToClipboard(message)) {
    copiedMsg.value = message;
    setTimeout(() => {
      if (copiedMsg.value === message) copiedMsg.value = null;
    }, 1500);
  } else {
    console.error("copy message failed");
  }
}

// ─── Branches ───────────────────────────────────────────────────────

const localBranches = computed(() => branches.value.filter((b) => !b.is_remote));
const remoteBranches = computed(() => branches.value.filter((b) => b.is_remote));

function statusIcon(status: string) {
  switch (status) {
    case "M": return { color: "var(--color-warning)", label: "M" };
    case "A": return { color: "var(--color-success)", label: "A" };
    case "D": return { color: "var(--color-danger)", label: "D" };
    case "R": return { color: "var(--color-primary)", label: "R" };
    case "I": return { color: "var(--color-text-muted)", label: "I" };
    default: return { color: "var(--color-text-muted)", label: "?" };
  }
}

function shortTime(ts: number): string {
  // commit.timestamp 是秒，formatRelativeTime 收 ISO 串：此处只做单位换算，
  // 分档与文案统一由它决定（原先本组件自带一套分档，超过 30 天还会落到
  // 宿主默认 locale 的日期串）。
  return formatRelativeTime(new Date(ts * 1000).toISOString());
}
</script>

<template>
  <div class="git-panel">
    <!-- Sub-tab bar -->
    <div class="sub-tabs">
      <button
        class="sub-tab"
        :class="{ active: activeSubTab === 'changes' }"
        @click="activeSubTab = 'changes'"
      >{{ t("session.gitPanel.tabChanges") }}</button>
      <button
        class="sub-tab"
        :class="{ active: activeSubTab === 'commits' }"
        @click="activeSubTab = 'commits'"
      >{{ t("session.gitPanel.tabCommits") }}</button>
      <button
        class="sub-tab"
        :class="{ active: activeSubTab === 'branches' }"
        @click="activeSubTab = 'branches'"
      >{{ t("session.gitPanel.tabBranches") }}</button>
    </div>

    <!-- Not a git repo -->
    <div v-if="!isGitRepo" class="empty-hint">
      {{ t("session.gitPanel.notRepo") }}
    </div>

    <!-- Loading -->
    <div v-else-if="loading" class="empty-hint">
      {{ t("common.breadcrumb.loading") }}
    </div>

    <!-- ─── Changes Tab ─────────────────────────────────────────────── -->
    <template v-else-if="activeSubTab === 'changes'">
      <!-- Toolbar -->
      <div class="changes-toolbar">
        <button
          class="toolbar-btn"
          :class="{ active: groupByDir }"
          :title="groupByDir ? t('session.gitPanel.switchToFlatView') : t('session.gitPanel.groupByFolder')"
          @click="groupByDir = !groupByDir"
        >
          <SvgIcon name="folder-open" :size="14" />
        </button>
        <button
          class="toolbar-btn"
          :class="{ active: includeIgnored }"
          :title="includeIgnored ? t('session.gitPanel.hideIgnored') : t('session.gitPanel.showIgnored')"
          @click="toggleShowIgnored"
        >
          <SvgIcon :name="includeIgnored ? 'eye' : 'eye-off'" :size="14" />
        </button>
        <span class="toolbar-count">{{ t("session.gitPanel.fileCount", { count: statusEntries.length }) }}</span>
      </div>

      <div class="panel-content">
        <div v-if="statusEntries.length === 0" class="empty-section">{{ t("session.gitPanel.noChanges") }}</div>

        <!-- Flat view -->
        <template v-if="!groupByDir">
          <div
            v-for="entry in statusEntries"
            :key="entry.path"
            class="file-entry"
            @click="openFileDiff(entry.path)"
          >
            <span
              class="status-badge"
              :style="{ color: statusIcon(entry.status).color }"
            >{{ statusIcon(entry.status).label }}</span>
            <span class="file-name" :title="entry.path">{{ entry.path }}</span>
            <span v-if="entry.staged" class="staged-badge">{{ t("session.gitPanel.staged") }}</span>
          </div>
        </template>

        <!-- Directory tree view (flattened) -->
        <template v-else>
          <template v-for="row in flatRows" :key="row.dirPath ?? row.entry?.path">
            <!-- Directory row -->
            <div
              v-if="row.type === 'dir'"
              class="dir-entry"
              :style="{ paddingLeft: (row.depth * 16 + 12) + 'px' }"
              @click="toggleDir(row.dirPath!)"
            >
              <SvgIcon
                :name="expandedDirs.has(row.dirPath!) ? 'chevron-down' : 'chevron-right'"
                :size="12"
                class="chevron"
              />
              <SvgIcon name="folder-open" :size="12" class="dir-icon" />
              <span class="dir-name">{{ row.dirName }}</span>
              <span class="dir-count">{{ row.fileCount }}</span>
            </div>
            <!-- File row -->
            <div
              v-else
              class="file-entry"
              :style="{ paddingLeft: (row.depth * 16 + 12) + 'px' }"
              @click="openFileDiff(row.entry!.path)"
            >
              <span
                class="status-badge"
                :style="{ color: statusIcon(row.entry!.status).color }"
              >{{ statusIcon(row.entry!.status).label }}</span>
              <span class="file-name" :title="row.entry!.path">{{ row.fileName }}</span>
              <span v-if="row.entry!.staged" class="staged-badge">{{ t("session.gitPanel.staged") }}</span>
            </div>
          </template>
        </template>
      </div>
    </template>

    <!-- ─── Commits Tab ─────────────────────────────────────────────── -->
    <template v-else-if="activeSubTab === 'commits'">
      <!-- Commits Toolbar with Search -->
      <div class="commits-toolbar">
        <div class="commit-search-wrapper">
          <SvgIcon name="search" :size="13" class="commit-search-icon" />
          <input
            v-model="commitSearchQuery"
            type="text"
            class="commit-search-input"
            :placeholder="t('session.gitPanel.searchPlaceholder')"
            :aria-label="t('session.gitPanel.searchAria')"
            spellcheck="false"
            autocomplete="off"
            autocorrect="off"
            autocapitalize="off"
          />
          <button
            v-if="commitSearchQuery"
            class="commit-search-clear icon-btn"
            type="button"
            :title="t('session.gitPanel.clearSearch')"
            :aria-label="t('session.gitPanel.clearSearch')"
            @click="commitSearchQuery = ''"
          >
            <SvgIcon name="x" :size="11" />
          </button>
        </div>
        <span class="toolbar-count">
          <template v-if="commitSearchQuery">
            {{ filteredCommits.length }}/{{ commits.length }}
          </template>
          <template v-else>
            {{ commits.length }}
          </template>
        </span>
      </div>

      <div class="panel-content">
        <div v-if="commits.length === 0" class="empty-section">{{ t("session.gitPanel.noCommits") }}</div>
        <div v-else-if="filteredCommits.length === 0" class="empty-section">
          {{ t("session.gitPanel.noMatchingCommits") }}
        </div>
        <div
          v-for="commit in filteredCommits"
          :key="commit.hash"
          class="commit-entry"
          :class="{ expanded: expandedCommit === commit.hash }"
        >
          <div
            class="commit-row"
            :title="commit.message"
            @click="toggleCommit(commit.hash)"
          >
            <SvgIcon
              :name="expandedCommit === commit.hash ? 'chevron-down' : 'chevron-right'"
              :size="12"
              class="chevron"
            />
            <div class="commit-info">
              <div class="commit-summary-line">
                <span class="commit-summary">{{ parseCommitMessage(commit.message).summary }}</span>
              </div>
              <div class="commit-meta-line">
                <span
                  class="commit-hash"
                  :title="t('session.gitPanel.copyHash')"
                  @click.stop="copyCommitHash(commit.short_hash, $event)"
                >
                  {{ commit.short_hash }}
                  <SvgIcon v-if="copiedHash === commit.short_hash" name="check" :size="10" class="copied-icon" />
                </span>
                <span class="meta-dot">·</span>
                <span class="commit-author" :title="`${commit.author} <${commit.email}>`">{{ commit.author }}</span>
                <span class="meta-dot">·</span>
                <span class="commit-time">{{ shortTime(commit.timestamp) }}</span>
              </div>
            </div>
          </div>
          <div v-if="expandedCommit === commit.hash" class="commit-detail">
            <!-- Full commit body card if present -->
            <div v-if="parseCommitMessage(commit.message).body" class="commit-body-card">
              <div class="commit-body-header">
                <span class="commit-body-title">{{ t("session.gitPanel.commitMessage") }}</span>
                <button
                  class="btn-copy-icon"
                  :class="{ copied: copiedMsg === commit.message }"
                  type="button"
                  :title="copiedMsg === commit.message ? t('session.gitPanel.copiedMessage') : t('session.gitPanel.copyMessage')"
                  :aria-label="t('session.gitPanel.copyMessage')"
                  @click.stop="copyCommitMessage(commit.message, $event)"
                >
                  <SvgIcon :name="copiedMsg === commit.message ? 'check' : 'copy'" :size="12" />
                </button>
              </div>
              <div class="commit-body-text">{{ parseCommitMessage(commit.message).body }}</div>
            </div>

            <!-- Changed files header -->
            <div class="commit-section-header">
              <span class="section-label">{{ t("session.gitPanel.changedFiles", { count: commitFiles.length }) }}</span>
            </div>
            <div
              v-for="file in commitFiles"
              :key="file.path"
              class="file-entry nested"
              @click="openCommitFileDiff(commit.hash, file.path)"
            >
              <span
                class="status-badge"
                :style="{ color: statusIcon(file.status).color }"
              >{{ statusIcon(file.status).label }}</span>
              <span class="file-name" :title="file.path">{{ file.path }}</span>
            </div>
          </div>
        </div>
      </div>
    </template>

    <!-- ─── Branches Tab ────────────────────────────────────────────── -->
    <template v-else-if="activeSubTab === 'branches'">
      <div class="panel-content">
        <!-- Local Branches -->
        <div class="section-header">
          <SvgIcon name="git-branch" :size="13" class="section-icon" />
          <span class="section-title">{{ t("session.gitPanel.localBranches") }}</span>
          <span class="section-count">{{ localBranches.length }}</span>
        </div>
        <div
          v-for="branch in localBranches"
          :key="branch.name"
          class="branch-entry"
          :class="{ current: branch.is_current }"
        >
          <SvgIcon name="git-branch" :size="12" class="branch-icon" />
          <span class="branch-name">{{ branch.name }}</span>
          <span v-if="branch.is_current" class="current-badge">HEAD</span>
        </div>

        <!-- Remote Branches -->
        <div class="section-header">
          <SvgIcon name="git-compare" :size="13" class="section-icon" />
          <span class="section-title">{{ t("session.gitPanel.remoteBranches") }}</span>
          <span class="section-count">{{ remoteBranches.length }}</span>
        </div>
        <div
          v-for="branch in remoteBranches"
          :key="branch.name"
          class="branch-entry"
        >
          <SvgIcon name="git-branch" :size="12" class="branch-icon" />
          <span class="branch-name">{{ branch.name }}</span>
        </div>
      </div>
    </template>
  </div>
</template>

<style scoped>
.git-panel {
  display: flex;
  flex-direction: column;
  height: 100%;
  overflow: hidden;
  -webkit-user-select: none;
  user-select: none;
}

/* ─── Sub-tabs ──────────────────────────────────────────────────── */

.sub-tabs {
  display: flex;
  gap: var(--space-1);
  padding: var(--space-2) var(--space-3);
  border-bottom: 1px solid var(--color-border);
  flex-shrink: 0;
}
.sub-tab {
  padding: var(--space-1) var(--space-2);
  font-size: var(--text-xs);
  font-weight: 500;
  color: var(--color-text-muted);
  border-radius: var(--radius-sm);
  cursor: pointer;
  transition: all var(--transition-fast);
}
.sub-tab:hover {
  color: var(--color-text);
  background: var(--color-bg-hover);
}
.sub-tab.active {
  color: var(--color-primary);
  background: var(--color-primary-light);
}

/* ─── Changes toolbar ──────────────────────────────────────────── */

.changes-toolbar {
  display: flex;
  align-items: center;
  gap: var(--space-1);
  padding: var(--space-1) var(--space-3);
  border-bottom: 1px solid var(--color-border);
  flex-shrink: 0;
}
.toolbar-btn {
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 3px;
  color: var(--color-text-muted);
  border-radius: var(--radius-sm);
  cursor: pointer;
  transition: all var(--transition-fast);
}
.toolbar-btn:hover {
  color: var(--color-text);
  background: var(--color-bg-hover);
}
.toolbar-btn.active {
  color: var(--color-primary);
  background: var(--color-primary-light);
}
.toolbar-count {
  margin-left: auto;
  font-size: var(--text-xs);
  color: var(--color-text-muted);
  opacity: 0.7;
}

/* ─── Commits toolbar ──────────────────────────────────────────── */

.commits-toolbar {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  padding: var(--space-1) var(--space-2);
  border-bottom: 1px solid var(--color-border);
  flex-shrink: 0;
}
.commit-search-wrapper {
  flex: 1;
  display: flex;
  align-items: center;
  gap: 5px;
  background: var(--color-bg-subtle, rgba(125, 125, 125, 0.06));
  border: 1px solid var(--color-border);
  border-radius: var(--radius-sm, 4px);
  padding: 2px 6px;
  min-height: 24px;
  transition: border-color var(--transition-fast), background var(--transition-fast);
}
.commit-search-wrapper:focus-within {
  border-color: var(--color-primary);
  background: var(--color-bg);
}
.commit-search-icon {
  flex-shrink: 0;
  color: var(--color-text-muted);
}
.commit-search-input {
  flex: 1;
  min-width: 0;
  border: none;
  background: transparent;
  padding: 0;
  font-size: var(--text-xs);
  color: var(--color-text);
  outline: none;
}
.commit-search-input::placeholder {
  color: var(--color-text-muted);
  opacity: 0.75;
}
.commit-search-clear {
  flex-shrink: 0;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 16px;
  height: 16px;
  padding: 0;
  border: none;
  background: transparent;
  color: var(--color-text-muted);
  border-radius: 50%;
  cursor: pointer;
  transition: all var(--transition-fast);
}
.commit-search-clear:hover {
  background: var(--color-bg-hover);
  color: var(--color-text);
}

/* ─── Panel content ─────────────────────────────────────────────── */

.panel-content {
  flex: 1;
  overflow-y: auto;
  padding-bottom: var(--space-2);
}

.empty-hint {
  font-size: var(--text-xs);
  color: var(--color-text-muted);
  padding: var(--space-4) var(--space-3);
  text-align: center;
  line-height: 1.5;
}

.empty-section {
  font-size: var(--text-xs);
  color: var(--color-text-muted);
  padding: var(--space-1) var(--space-3);
}

/* ─── Section header ────────────────────────────────────────────── */

.section-header {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: var(--space-1) var(--space-3);
  font-size: var(--text-xs);
  font-weight: 600;
  color: var(--color-text-muted);
  margin-top: var(--space-1);
}
.section-title {
  flex: 1;
}
.section-count {
  font-weight: 400;
  color: var(--color-text-muted);
  opacity: 0.7;
}
.section-icon {
  color: var(--color-text-muted);
  flex-shrink: 0;
}

/* ─── File entry ────────────────────────────────────────────────── */

.file-entry {
  display: flex;
  align-items: center;
  min-height: 28px;
  height: 28px;
  box-sizing: border-box;
  gap: 6px;
  padding: 0 8px;
  font-size: var(--text-xs);
  color: var(--color-text-secondary);
  cursor: pointer;
  transition: background var(--transition-fast), color var(--transition-fast);
  border-radius: 6px;
  margin: 1px 6px;
}
.file-entry:hover {
  background: var(--color-bg-hover);
  color: var(--color-text);
}
.file-entry.nested {
  padding-left: 20px;
}

.status-badge {
  flex-shrink: 0;
  font-weight: 600;
  font-size: var(--text-xs);
  width: 14px;
  text-align: center;
  font-variant-numeric: tabular-nums;
}

.file-name {
  flex: 1;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.file-parent-dir {
  color: var(--color-text-muted);
  font-weight: 400;
}

.staged-badge {
  flex-shrink: 0;
  font-size: 10px;
  color: var(--color-success);
  opacity: 0.7;
}

/* ─── Directory entry (tree view) ──────────────────────────────── */

.dir-entry {
  display: flex;
  align-items: center;
  min-height: 28px;
  height: 28px;
  box-sizing: border-box;
  gap: 6px;
  padding: 0 8px;
  font-size: var(--text-xs);
  color: var(--color-text);
  cursor: pointer;
  transition: background var(--transition-fast);
  border-radius: 6px;
  margin: 1px 6px;
}
.dir-entry:hover {
  background: var(--color-bg-hover);
}
.dir-icon {
  flex-shrink: 0;
  color: var(--color-warning);
}
.dir-name {
  flex: 1;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-weight: 500;
}
.dir-count {
  flex-shrink: 0;
  font-size: var(--text-2xs);
  font-variant-numeric: tabular-nums;
  color: var(--color-text-muted);
  opacity: 0.7;
}

/* ─── Commit entry ──────────────────────────────────────────────── */

.commit-entry {
  border-bottom: 1px solid var(--color-border);
}
.commit-entry:last-child {
  border-bottom: none;
}
.commit-entry.expanded {
  background: var(--color-surface-hover, rgba(0, 0, 0, 0.02));
}
.commit-row {
  display: flex;
  align-items: flex-start;
  min-height: 40px;
  box-sizing: border-box;
  gap: 7px;
  padding: 6px 8px;
  margin: 1px 4px;
  border-radius: 6px;
  font-size: var(--text-xs);
  cursor: pointer;
  transition: background var(--transition-fast);
}
.commit-row:hover {
  background: var(--color-surface-selected);
}
.chevron {
  flex-shrink: 0;
  margin-top: 3px;
  color: var(--color-text-muted);
}
.commit-info {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 3px;
}
.commit-summary-line {
  display: flex;
  align-items: baseline;
  min-width: 0;
}
.commit-summary {
  font-size: var(--text-xs);
  font-weight: 500;
  color: var(--color-text);
  line-height: 1.35;
  word-break: break-word;
  display: -webkit-box;
  -webkit-line-clamp: 2;
  -webkit-box-orient: vertical;
  overflow: hidden;
  text-overflow: ellipsis;
}
.commit-meta-line {
  display: flex;
  align-items: center;
  gap: 4px;
  font-size: var(--text-2xs);
  color: var(--color-text-muted);
  line-height: 1;
  min-width: 0;
}
.commit-hash {
  flex-shrink: 0;
  font-family: var(--font-mono, monospace);
  color: var(--color-primary);
  font-weight: 600;
  padding: 1px 3px;
  border-radius: 3px;
  background: rgba(var(--color-primary-rgb, 59, 130, 246), 0.08);
  display: inline-flex;
  align-items: center;
  gap: 2px;
  transition: background var(--transition-fast);
}
.commit-hash:hover {
  background: rgba(var(--color-primary-rgb, 59, 130, 246), 0.18);
}
.copied-icon {
  color: var(--color-success);
}
.meta-dot {
  opacity: 0.4;
  flex-shrink: 0;
}
.commit-author {
  max-width: 90px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  opacity: 0.85;
}
.commit-time {
  margin-left: auto;
  flex-shrink: 0;
  opacity: 0.8;
  font-variant-numeric: tabular-nums;
}
.commit-detail {
  padding: 2px 8px 8px 24px;
}
.commit-body-card {
  margin: 2px 0 8px;
  padding: 8px 10px;
  border-radius: var(--radius-md, 6px);
  background: var(--color-bg-subtle, rgba(125, 125, 125, 0.06));
  border: 1px solid var(--color-border);
}
.commit-body-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: 4px;
}
.commit-body-title {
  font-size: var(--text-2xs);
  font-weight: 600;
  color: var(--color-text-muted);
  letter-spacing: 0.3px;
}
.btn-copy-icon {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 20px;
  height: 20px;
  border-radius: var(--radius-sm, 4px);
  border: 1px solid transparent;
  background: transparent;
  color: var(--color-text-muted);
  cursor: pointer;
  padding: 0;
  transition: all var(--transition-fast);
}
.btn-copy-icon:hover {
  background: var(--color-surface-hover, rgba(125, 125, 125, 0.12));
  color: var(--color-text);
  border-color: var(--color-border);
}
.btn-copy-icon.copied {
  color: var(--color-success);
}
.commit-body-text {
  font-size: var(--text-xs);
  line-height: 1.55;
  color: var(--color-text);
  white-space: pre-wrap;
  word-break: break-word;
  max-height: 220px;
  overflow-y: auto;
}
.commit-section-header {
  display: flex;
  align-items: center;
  font-size: var(--text-2xs);
  font-weight: 600;
  color: var(--color-text-muted);
  margin: 6px 0 4px;
}

/* ─── Branch entry ──────────────────────────────────────────────── */

.branch-entry {
  display: flex;
  align-items: center;
  min-height: 28px;
  height: 28px;
  box-sizing: border-box;
  gap: 6px;
  padding: 0 8px;
  font-size: var(--text-xs);
  color: var(--color-text-secondary);
  border-radius: 6px;
  margin: 1px 6px;
  transition: background var(--transition-fast), color var(--transition-fast);
}
.branch-entry:hover {
  background: var(--color-surface-selected);
  color: var(--color-text);
}
.branch-entry.current {
  background: var(--color-surface-selected);
  color: var(--color-text);
}
.branch-icon {
  flex-shrink: 0;
  color: var(--color-text-muted);
}
.branch-name {
  flex: 1;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.current-badge {
  flex-shrink: 0;
  font-size: 10px;
  font-weight: 600;
  color: var(--color-primary);
  background: var(--color-primary-light);
  padding: 0 4px;
  border-radius: var(--radius-sm);
  line-height: 1.6;
}
</style>

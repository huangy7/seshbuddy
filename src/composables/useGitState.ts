import { ref } from "vue";
import { invokeApp } from "../utils/invokeApp";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export interface GitStatusEntry {
  path: string;
  status: string;
  staged: boolean;
}

export interface CommitInfo {
  hash: string;
  short_hash: string;
  message: string;
  author: string;
  email: string;
  timestamp: number;
}

export interface BranchInfo {
  name: string;
  is_current: boolean;
  is_remote: boolean;
  upstream: string | null;
}

export interface CommitDiffFile {
  path: string;
  status: string;
}

export interface CommitDiffResult {
  info: CommitInfo;
  files: CommitDiffFile[];
}

// Module-level singleton state
const contextPath = ref<string | null>(null);
const isGitRepo = ref(false);
const statusEntries = ref<GitStatusEntry[]>([]);
const commits = ref<CommitInfo[]>([]);
const branches = ref<BranchInfo[]>([]);
const loading = ref(false);
const includeIgnored = ref(false);

let unlisten: UnlistenFn | null = null;
let watchedPath: string | null = null;
let debounceTimer: ReturnType<typeof setTimeout> | null = null;

async function ensureListener() {
  if (unlisten) return;
  unlisten = await listen<string>("git-state-changed", (event) => {
    const changedPath = event.payload;
    if (changedPath === contextPath.value) {
      refreshAll();
    }
  });
}

async function refreshAll() {
  const path = contextPath.value;
  if (!path || !isGitRepo.value) return;
  loading.value = true;
  try {
    const [status, log, branchList] = await Promise.all([
      invokeApp<GitStatusEntry[]>("git_status", { path, includeIgnored: includeIgnored.value }),
      invokeApp<CommitInfo[]>("git_log", { path, file: null, limit: 100 }),
      invokeApp<BranchInfo[]>("git_branches", { path }),
    ]);
    statusEntries.value = status;
    commits.value = log;
    branches.value = branchList;
  } catch (e) {
    console.error("Git refresh failed:", e);
  } finally {
    loading.value = false;
  }
}

async function setContextPath(path: string | null) {
  if (path === contextPath.value) return;

  // Debounce: 200ms
  if (debounceTimer) clearTimeout(debounceTimer);
  debounceTimer = setTimeout(async () => {
    debounceTimer = null;
    // Unwatch old
    if (watchedPath) {
      await invokeApp("unwatch_git_state", { path: watchedPath }).catch(() => {});
      watchedPath = null;
    }

    contextPath.value = path;
    if (!path) {
      isGitRepo.value = false;
      statusEntries.value = [];
      commits.value = [];
      branches.value = [];
      return;
    }

    // Check if git repo
    const isRepo = await invokeApp<boolean>("git_is_repo", { path });
    isGitRepo.value = isRepo;
    if (!isRepo) {
      statusEntries.value = [];
      commits.value = [];
      branches.value = [];
      return;
    }

    await ensureListener();
    await invokeApp("watch_git_state", { path }).catch(() => {});
    watchedPath = path;
    await refreshAll();
  }, 200);
}

function cleanup() {
  if (debounceTimer) {
    clearTimeout(debounceTimer);
    debounceTimer = null;
  }
}

export function useGitState() {
  return {
    contextPath,
    isGitRepo,
    statusEntries,
    commits,
    branches,
    loading,
    includeIgnored,
    setContextPath,
    refreshAll,
    cleanup,
  };
}

import { ref } from "vue";
import type { ProjectInfo } from "../types/session";
import { useBlockedFolders } from "./useBlockedFolders";

const STORAGE_KEY = "seshbuddy-manual-workspaces";

const manualPaths = ref<string[]>(loadFromStorage());

function loadFromStorage(): string[] {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    return raw ? JSON.parse(raw) : [];
  } catch {
    return [];
  }
}

function saveToStorage() {
  localStorage.setItem(STORAGE_KEY, JSON.stringify(manualPaths.value));
}

export function useWorkspaces() {
  const { isBlocked } = useBlockedFolders();

  function addWorkspace(path: string) {
    if (!manualPaths.value.includes(path)) {
      manualPaths.value.push(path);
      saveToStorage();
    }
  }

  function removeWorkspace(path: string) {
    manualPaths.value = manualPaths.value.filter((p) => p !== path);
    saveToStorage();
  }

  function mergedProjectPaths(projects: ProjectInfo[]): string[] {
    const seen = new Set<string>();
    const result: string[] = [];
    for (const p of projects) {
      // 解析不出项目路径的会话不贡献工作区路径。**不能拿空串顶替**：空串经 `isSubPath`
      // 会命中任何绝对路径（`startsWith("/")`），把「没有项目」变成「匹配一切」。
      const path = p.original_path;
      if (!path) continue;
      if (!seen.has(path) && !isBlocked(path)) {
        seen.add(path);
        result.push(path);
      }
    }
    for (const p of manualPaths.value) {
      if (!seen.has(p) && !isBlocked(p)) {
        seen.add(p);
        result.push(p);
      }
    }
    return result;
  }

  return { manualPaths, addWorkspace, removeWorkspace, mergedProjectPaths };
}

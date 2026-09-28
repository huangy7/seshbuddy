import { ref, type Ref } from "vue";
import { invokeApp } from "../utils/invokeApp";

// 单例：全应用共享同一份屏蔽文件夹列表
const blockedFolders = ref<string[]>([]);

function normalizePath(p: string): string {
  return p.trim().replace(/[/\\]+$/, "");
}

export function isPathBlocked(projectPath: string, blockedList: string[]): boolean {
  if (!projectPath) return false;
  const normalizedProj = normalizePath(projectPath);
  if (!normalizedProj) return false;

  return blockedList.some((blocked) => {
    const normalizedBlocked = normalizePath(blocked);
    if (!normalizedBlocked) return false;
    return (
      normalizedProj === normalizedBlocked ||
      normalizedProj.startsWith(normalizedBlocked + "/") ||
      normalizedProj.startsWith(normalizedBlocked + "\\")
    );
  });
}

export function useBlockedFolders(): {
  blockedFolders: Readonly<Ref<string[]>>;
  loadBlockedFolders: () => Promise<void>;
  isBlocked: (projectPath: string) => boolean;
  blockFolder: (path: string) => Promise<void>;
  unblockFolder: (path: string) => Promise<void>;
} {
  async function loadBlockedFolders(): Promise<void> {
    const list = await invokeApp<string[]>("read_blocked_folders");
    blockedFolders.value = [...(list ?? [])];
  }

  function isBlocked(projectPath: string): boolean {
    return isPathBlocked(projectPath, blockedFolders.value);
  }

  async function blockFolder(path: string): Promise<void> {
    const trimmed = path.trim();
    if (!trimmed) return;
    if (blockedFolders.value.includes(trimmed)) return;
    blockedFolders.value = [...blockedFolders.value, trimmed];
    await invokeApp("write_blocked_folders", { paths: blockedFolders.value });
  }

  async function unblockFolder(path: string): Promise<void> {
    const trimmed = path.trim();
    const normalizedTrimmed = normalizePath(trimmed);
    blockedFolders.value = blockedFolders.value.filter(
      (p) => p !== trimmed && normalizePath(p) !== normalizedTrimmed,
    );
    await invokeApp("write_blocked_folders", { paths: blockedFolders.value });
  }

  return {
    blockedFolders,
    loadBlockedFolders,
    isBlocked,
    blockFolder,
    unblockFolder,
  };
}

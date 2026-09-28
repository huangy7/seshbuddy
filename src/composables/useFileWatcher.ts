import { onUnmounted } from "vue";
import { invokeApp } from "../utils/invokeApp";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

type ConflictHandler = (path: string) => void;

let unlisten: UnlistenFn | null = null;
const handlers = new Map<string, ConflictHandler>();

async function ensureListener() {
  if (unlisten) return;
  unlisten = await listen<string>("file-changed", (event) => {
    const path = event.payload;
    const handler = handlers.get(path);
    if (handler) handler(path);
  });
}

export function useFileWatcher() {
  const watchedPaths = new Set<string>();

  async function watchFile(path: string, onChanged: ConflictHandler) {
    await ensureListener();
    handlers.set(path, onChanged);
    watchedPaths.add(path);
    await invokeApp("watch_file", { path });
  }

  async function unwatchFile(path: string) {
    handlers.delete(path);
    watchedPaths.delete(path);
    await invokeApp("unwatch_file", { path }).catch(() => {});
  }

  onUnmounted(() => {
    for (const path of watchedPaths) {
      handlers.delete(path);
      invokeApp("unwatch_file", { path }).catch(() => {});
    }
    watchedPaths.clear();
  });

  return { watchFile, unwatchFile };
}

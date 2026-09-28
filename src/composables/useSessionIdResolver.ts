import { computed, reactive, watch, type Ref } from "vue";
import { invokeApp } from "../utils/invokeApp";
import { useSessions } from "./useSessions";
import type { OpenTab } from "./useTabs";
import { isCliId, type CliId } from "../types/cli";
import { sessionIdentityKey } from "../types/session";

// 注意：本组合式函数持有每次调用独立的状态（sessionIdByPath 等），
// 与 useSessions 的模块级单例不同。当前仅 App.vue 调用一次；
// 若未来出现第二个调用方，缓存会互相隔离，需先改为单例。
export function useSessionIdResolver(openTabs: Ref<OpenTab[]>) {
  const { projects } = useSessions();

  const sessionIdByFilePath = computed(() => {
    const m = new Map<string, string>();
    for (const p of projects.value) {
      for (const s of p.sessions ?? []) {
        if (s.session_id && isCliId(s.cli_id)) {
          m.set(sessionIdentityKey({ cliId: s.cli_id, filePath: s.file_path }), s.session_id);
        }
      }
    }
    return m;
  });

  const sessionIdByPath = reactive(new Map<string, string>());
  const sessionIdResolutionFailed = reactive(new Set<string>());
  const pendingSessionIdPaths = new Set<string>();

  function syncSessionIdFromProjects(sessionPath: string, cliId: CliId): string | null {
    const key = sessionIdentityKey({ cliId, filePath: sessionPath });
    const sessionId = sessionIdByFilePath.value.get(key)?.trim();
    if (sessionId) {
      sessionIdByPath.set(key, sessionId);
      sessionIdResolutionFailed.delete(key);
      return sessionId;
    }
    return null;
  }

  async function ensureHistorySessionId(sessionPath: string, cliId?: CliId): Promise<string | null> {
    if (!cliId) return null;
    const key = sessionIdentityKey({ cliId, filePath: sessionPath });

    const fromProjects = syncSessionIdFromProjects(sessionPath, cliId);
    if (fromProjects) return fromProjects;

    const cached = sessionIdByPath.get(key);
    if (cached) return cached;
    if (sessionIdResolutionFailed.has(key) || pendingSessionIdPaths.has(key)) {
      return null;
    }

    pendingSessionIdPaths.add(key);
    try {
      const resolved = await invokeApp<string | null>("resolve_session_id", {
        cliId,
        filePath: sessionPath,
      });
      const sessionId = resolved?.trim();
      if (sessionId) {
        sessionIdByPath.set(key, sessionId);
        sessionIdResolutionFailed.delete(key);
        return sessionId;
      }
      sessionIdResolutionFailed.add(key);
      return null;
    } catch (e) {
      console.error("resolve_session_id failed:", e);
      sessionIdResolutionFailed.add(key);
      return null;
    } finally {
      pendingSessionIdPaths.delete(key);
    }
  }

  function historySessionId(tab: OpenTab | null | undefined): string | null {
    if (!tab || tab.type !== "history" || !tab.sessionPath || !tab.cliId) return null;
    const sessionId = sessionIdByPath.get(
      sessionIdentityKey({ cliId: tab.cliId, filePath: tab.sessionPath }),
    );
    return sessionId && sessionId.trim() ? sessionId : null;
  }

  watch(
    projects,
    () => {
      for (const tab of openTabs.value) {
        if (tab.type === "history" && tab.sessionPath && tab.cliId) {
          syncSessionIdFromProjects(tab.sessionPath, tab.cliId);
        }
      }
    }
  );

  watch(
    openTabs,
    (tabs) => {
      for (const tab of tabs) {
        if (tab.type === "history" && tab.sessionPath && tab.cliId) {
          void ensureHistorySessionId(tab.sessionPath, tab.cliId);
        }
      }
    },
    { deep: true, immediate: true }
  );

  return { ensureHistorySessionId, historySessionId };
}

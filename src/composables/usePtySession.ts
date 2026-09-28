import { ref, computed } from "vue";
import { invokeApp } from "../utils/invokeApp";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { PtySessionInfo, PtyStatusChangedPayload, AgentStatusMode } from "../types/pty";
import { basename } from "../utils/projectPath";

// Module-scope singleton state
const sessions = ref<PtySessionInfo[]>([]);
const customLabels = ref<Record<string, string>>({});
let statusListener: UnlistenFn | null = null;

function getSessionLabel(sessionId: string, projectPath: string): string {
  return customLabels.value[sessionId] ?? basename(projectPath) ?? projectPath;
}

function setSessionLabel(sessionId: string, label: string) {
  customLabels.value[sessionId] = label;
}

/**
 * 撤销自定义名，回到 `getSessionLabel` 的回退链（项目目录名）。
 *
 * 存在的理由：调用方回到默认标签时**不能**把渲染后的默认文案写进来——那是个界面词，
 * 写进 `customLabels` 就按写入那一刻的语言定死了。删掉条目才让它跟随语言。
 */
function clearSessionLabel(sessionId: string) {
  delete customLabels.value[sessionId];
}

function ensureStatusListener() {
  if (statusListener) return;
  listen<PtyStatusChangedPayload>("pty-status-changed", (event) => {
    const { sessionId, status } = event.payload;
    const session = sessions.value.find((s) => s.sessionId === sessionId);
    if (session) {
      session.status = status;
    }
  }).then((unlisten) => {
    statusListener = unlisten;
  });
}

function closeStatusListener() {
  if (statusListener) {
    statusListener();
    statusListener = null;
  }
}

const waitingInputCount = computed(() =>
  sessions.value.filter((s) => s.status === "waiting_input").length
);

async function refreshSessions() {
  const list = await invokeApp<PtySessionInfo[]>("list_pty_sessions");
  sessions.value = list;
}

async function createSession(
  projectPath: string,
  cliKind: string,
  sessionId?: string,
  resumeSessionId?: string,
  skipPermissions?: boolean,
  profileName?: string | null,
  agentStatusMode?: AgentStatusMode,
): Promise<PtySessionInfo> {
  const info = await invokeApp<PtySessionInfo>("create_pty_session", {
    projectPath,
    cliKind,
    sessionId,
    resumeSessionId,
    skipPermissions,
    profileName: profileName || undefined,
    agentStatusMode,
  });
  sessions.value.push(info);
  return info;
}

async function closeSession(sessionId: string): Promise<void> {
  try {
    await invokeApp("close_pty_session", { sessionId });
  } catch {
    // Guardian thread may have already removed the session after process exit
  }
  sessions.value = sessions.value.filter((s) => s.sessionId !== sessionId);
}

function removeExitedSession(sessionId: string) {
  sessions.value = sessions.value.filter((s) => s.sessionId !== sessionId);
}

export function usePtySession() {
  ensureStatusListener();
  return {
    sessions,
    customLabels,
    getSessionLabel,
    setSessionLabel,
    clearSessionLabel,
    waitingInputCount,
    refreshSessions,
    createSession,
    closeSession,
    removeExitedSession,
    closeStatusListener,
  };
}

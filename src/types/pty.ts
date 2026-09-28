export type PtyStatus = "active" | "idle" | "waiting_input" | "exited";
export type PtyStatusSource = "hook" | "frontend_fallback";
export type AgentStatusMode = "osc" | "hook-relay";
export type PtyFileAction = "read" | "write" | "edit";

export interface PtySessionInfo {
  sessionId: string;
  projectPath: string;
  cliKind: string;
  createdAt: string;
  status: PtyStatus;
  statusSource?: PtyStatusSource;
}

export interface PtyStatusChangedPayload {
  sessionId: string;
  status: PtyStatus;
}

export interface PtyFileLinkPayload {
  sessionId: string;
  path: string;
  displayPath: string;
  action: PtyFileAction;
}

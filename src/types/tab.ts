export interface TabItem {
  id: string;
  label: string;
  icon: string;
  type: "terminal" | "history" | "file" | "diff" | "agent-dashboard";
  closable?: boolean;
  isPreview?: boolean;
  statusColor?: string;
  cliId?: string;
  sessionPath?: string;
  encodedDir?: string;
  sessionId?: string;
  filePath?: string;
  isDirty?: boolean;
  parentSessionPath?: string;
  parentMessageIndex?: number;
  // Diff-specific fields
  diffOldContent?: string;
  diffNewContent?: string;
  splitCount?: number;
}

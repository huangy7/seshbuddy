export type PaneId = string;
export type SplitDir = "row" | "col";

export interface TerminalPaneLeaf {
  kind: "leaf";
  id: PaneId;
  sessionId: string;
  cliKind: string; // "shell" | "claude" | "codex" | "gemini" | etc.
  projectPath: string;
  customTitle?: string;
  createdAt: number;
}

export interface TerminalPaneSplit {
  kind: "split";
  id: PaneId;
  dir: SplitDir; // "row" = left/right horizontal split, "col" = top/bottom vertical split
  ratios?: number[]; // array of normalized fractional sizes, e.g. [0.5, 0.5]
  children: TerminalPaneNode[];
}

export type TerminalPaneNode = TerminalPaneLeaf | TerminalPaneSplit;

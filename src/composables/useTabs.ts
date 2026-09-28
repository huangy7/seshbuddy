import { ref, reactive, computed } from "vue";
import { t } from "../i18n";
import { invokeApp } from "../utils/invokeApp";
import { ask } from "@tauri-apps/plugin-dialog";
import { nanoid } from "nanoid";
import type ChatView from "../components/ChatView.vue";
import type MonacoEditorComponent from "../components/MonacoEditor.vue";
import type { TabItem } from "../types/tab";
import type { CliId } from "../types/cli";
import type { SplitDir, TerminalPaneLeaf, TerminalPaneNode } from "../types/terminal";
import {
  findLeaf,
  firstLeafId,
  leafIds,
  nextLeafId,
  removeLeaf,
  siblingLeafOf,
  splitLeaf,
} from "../utils/panes";
import { sessionIdentityKey, type SessionIdentity } from "../types/session";
import { basename } from "../utils/projectPath";
import { useSessions } from "./useSessions";
import { usePtySession } from "./usePtySession";

// 注意：本组合式函数持有每次调用独立的状态（openTabs 等），
// 与 useSessions 的模块级单例不同。当前仅 App.vue 调用一次；
// 若未来出现第二个调用方，两份 Tab 状态会互相隔离，需先改为单例。

/**
 * Tab 标签。两种形态泾渭分明，避免把界面文案烘进长期状态：
 * - `key`：界面文案，渲染时用 t() 现取，切语言后跟着变；
 * - `text`：用户数据或派生自数据（重命名、文件名、会话 display_name），不翻译。
 */
export type TabLabel = { kind: "key"; key: string } | { kind: "text"; text: string };

/**
 * 标签的显示文案。
 *
 * 必须在使用处现取：t() 读的是响应式 locale，写在渲染/计算上下文里才会随语言切换重算。
 * 把结果存进 OpenTab 就会把语言冻结在创建的那一刻。
 */
export function tabLabelText(label: TabLabel): string {
  return label.kind === "key" ? t(label.key) : label.text;
}

export interface OpenTab {
  id: string;
  type: "terminal" | "history" | "file" | "diff" | "agent-dashboard";
  label: TabLabel;
  isCustomLabel?: boolean;
  sessionId?: string;
  cliId?: CliId;
  sessionPath?: string;
  encodedDir?: string;
  filePath?: string;
  projectRoot?: string;
  isDirty?: boolean;
  isPreview?: boolean;
  parentSessionPath?: string;
  parentMessageIndex?: number;
  diffOriginal?: string;
  diffModified?: string;
  rootPane?: TerminalPaneNode;
  activePaneId?: string;
  maximizedPaneId?: string | null;
}

export interface OpenTerminalTabOptions {
  cliKind?: string;
  projectPath?: string;
  label?: string;
  skipPermissions?: boolean;
  profileName?: string | null;
}

export const AGENT_DASHBOARD_TAB_ID = "agent-dashboard";

export function useTabs() {
  const {
    projects,
    clearActiveSession,
    setActiveSession,
    selectedSessionIdentities,
    batchDeleteSessions,
  } = useSessions();
  const {
    sessions: ptySessions,
    closeSession: closePtySession,
    createSession: createPtySession,
    setSessionLabel,
    clearSessionLabel,
  } = usePtySession();

  const openTabs = ref<OpenTab[]>([]);
  const activeTabId = ref<string | null>(null);

  const fileEditorRefs = new Map<string, InstanceType<typeof MonacoEditorComponent>>();

  function registerEditorRef(tabId: string, ref: InstanceType<typeof MonacoEditorComponent> | null) {
    if (ref) {
      fileEditorRefs.set(tabId, ref);
    } else {
      fileEditorRefs.delete(tabId);
    }
  }

  const historyChatViewRefs = reactive(new Map<string, InstanceType<typeof ChatView>>());

  function registerHistoryChatViewRef(identity: SessionIdentity, el: InstanceType<typeof ChatView> | null) {
    const key = sessionIdentityKey(identity);
    if (el) historyChatViewRefs.set(key, el);
    else historyChatViewRefs.delete(key);
  }

  function onFileDirty(tabId: string, dirty: boolean) {
    const tab = openTabs.value.find((t) => t.id === tabId);
    if (!tab) return;
    tab.isDirty = dirty;
  }

  const tabItems = computed<TabItem[]>(() =>
    openTabs.value.map((tab) => {
      // 标签在这里才解析成文案：t() 读响应式 locale，故本 computed 随语言切换重算
      const label = tabLabelText(tab.label);
      if (tab.type === "agent-dashboard") {
        return { id: tab.id, label, icon: "terminal", type: "agent-dashboard" as const };
      }
      if (tab.type === "terminal") {
        const primarySessionId = tab.sessionId || (tab.rootPane ? findLeaf(tab.rootPane, tab.activePaneId || firstLeafId(tab.rootPane))?.sessionId : undefined);
        const ptySession = primarySessionId ? ptySessions.value.find((s) => s.sessionId === primarySessionId) : undefined;
        const statusColor = ptySession
          ? { active: "var(--color-success)", idle: "var(--color-warning)", waiting_input: "var(--color-danger)", exited: "var(--color-text-muted)" }[ptySession.status]
          : undefined;
        const splitCount = tab.rootPane ? leafIds(tab.rootPane).length : 1;
        return {
          id: tab.id,
          label,
          icon: "terminal",
          type: "terminal" as const,
          statusColor,
          cliId: ptySession?.cliKind,
          splitCount: splitCount > 1 ? splitCount : undefined,
        };
      }
      if (tab.type === "file") {
        const ext = tab.filePath?.split(".").pop()?.toLowerCase();
        const icon = ext === "html" || ext === "htm" ? "globe" : "file-text";
        const fileLabel = tab.isDirty ? `● ${label}` : label;
        return { id: tab.id, label: fileLabel, icon, type: "file" as const, filePath: tab.filePath, isPreview: tab.isPreview };
      }
      if (tab.type === "diff") {
        return { id: tab.id, label, icon: "git-compare", type: "diff" as const, closable: true };
      }
      return { id: tab.id, label, icon: "message-square", type: "history" as const, isPreview: tab.isPreview, cliId: tab.cliId };
    })
  );

  const activeOpenTab = computed(() => openTabs.value.find((t) => t.id === activeTabId.value));
  // Dashboard 不再是 Tab：零工作 Tab 时作为主区域根页面渲染
  const shouldShowDashboard = computed(() => openTabs.value.length === 0);
  const terminalTabs = computed(() => openTabs.value.filter((t) => t.type === "terminal"));
  const fileTabs = computed(() => openTabs.value.filter((t) => t.type === "file" && t.filePath));
  const diffTabs = computed(() => openTabs.value.filter((t) => t.type === "diff"));
  const historyTabs = computed(() => openTabs.value.filter((t) => t.type === "history" && t.sessionPath));
  const showHistoryChat = computed(() => activeOpenTab.value?.type === "history");

  function sanitizeDomId(raw: string) {
    return raw.replace(/[^a-zA-Z0-9_-]/g, "_");
  }

  function historyTabId(identity: SessionIdentity) {
    return `hist-${identity.cliId}-${identity.filePath}`;
  }

  function fileTabId(filePath: string) {
    return `file-${filePath}`;
  }

  function findSessionByIdentity(identity: SessionIdentity) {
    for (const p of projects.value) {
      const s = p.sessions.find(
        (session) => session.file_path === identity.filePath && session.cli_id === identity.cliId,
      );
      if (s) return { project: p, session: s };
    }
    return null;
  }

  function selectHistorySessionFromTab(tab: OpenTab | undefined) {
    if (tab?.type === "history" && tab.sessionPath && tab.cliId && tab.encodedDir != null) {
      setActiveSession(
        { cliId: tab.cliId, filePath: tab.sessionPath },
        tab.encodedDir,
      );
    }
  }

  function pinHistoryPreviewTab(tabId = activeTabId.value) {
    if (!tabId) return;
    const tab = openTabs.value.find((t) => t.id === tabId);
    if (!tab || tab.type !== "history" || !tab.isPreview || !tab.sessionPath) return;

    if (!tab.cliId) return;
    const identity = { cliId: tab.cliId, filePath: tab.sessionPath };
    const existingPinned = openTabs.value.find((candidate) =>
      candidate !== tab
      && candidate.type === "history"
      && !candidate.isPreview
      && candidate.cliId === identity.cliId
      && candidate.sessionPath === identity.filePath
    );
    if (existingPinned) {
      openTabs.value = openTabs.value.filter((t) => t !== tab);
      activeTabId.value = existingPinned.id;
      selectHistorySessionFromTab(existingPinned);
      return;
    }

    const idx = openTabs.value.indexOf(tab);
    if (idx !== -1) {
      openTabs.value.splice(idx, 1, { ...tab, isPreview: false });
    }
  }

  function pinFilePreviewTab(tabId = activeTabId.value) {
    if (!tabId) return;
    const tab = openTabs.value.find((t) => t.id === tabId);
    if (!tab || tab.type !== "file" || !tab.isPreview || !tab.filePath) return;

    const existingPinned = openTabs.value.find(
      (t) => t !== tab && t.type === "file" && !t.isPreview && t.filePath === tab.filePath,
    );
    if (existingPinned) {
      openTabs.value = openTabs.value.filter((t) => t !== tab);
      activeTabId.value = existingPinned.id;
      return;
    }

    const idx = openTabs.value.indexOf(tab);
    if (idx !== -1) {
      openTabs.value.splice(idx, 1, { ...tab, isPreview: false });
    }
  }

  function pinPreviewTab(tabId = activeTabId.value) {
    const tab = openTabs.value.find((t) => t.id === tabId);
    if (tab?.type === "history") {
      pinHistoryPreviewTab(tabId);
    } else if (tab?.type === "file") {
      pinFilePreviewTab(tabId);
    }
  }

  function openAgentDashboardTab() {
    const existing = openTabs.value.find((t) => t.id === AGENT_DASHBOARD_TAB_ID);
    if (!existing) {
      openTabs.value.unshift({
        id: AGENT_DASHBOARD_TAB_ID,
        type: "agent-dashboard",
        label: { kind: "key", key: "dialogs.common.agent" },
      });
    }
    activeTabId.value = AGENT_DASHBOARD_TAB_ID;
  }

  async function openTerminalTab(
    ptySessionId?: string,
    options: OpenTerminalTabOptions = {},
  ): Promise<OpenTab> {
    if (ptySessionId) {
      let existingTab = openTabs.value.find((t) => {
        if (t.type !== "terminal") return false;
        if (t.sessionId === ptySessionId) return true;
        if (t.rootPane) {
          const allLeaves = leafIds(t.rootPane);
          return allLeaves.some((lid) => findLeaf(t.rootPane!, lid)?.sessionId === ptySessionId);
        }
        return false;
      });

      const ptySession = ptySessions.value.find((s) => s.sessionId === ptySessionId);

      if (!existingTab) {
        const leafId = `pane-${nanoid(8)}`;
        const cliKind = ptySession?.cliKind || options.cliKind || "shell";
        const projectPath = ptySession?.projectPath || options.projectPath || "";
        const defaultLabel: TabLabel = cliKind === "shell"
          ? { kind: "key", key: "dialogs.common.terminal" }
          : { kind: "text", text: projectPath ? `${basename(projectPath)} · ${cliKind}` : cliKind };

        const leaf: TerminalPaneLeaf = {
          kind: "leaf",
          id: leafId,
          sessionId: ptySessionId,
          cliKind,
          projectPath,
          createdAt: Date.now(),
        };

        const newTab: OpenTab = {
          id: `term-${ptySessionId}`,
          type: "terminal",
          label: options.label ? { kind: "text", text: options.label } : defaultLabel,
          sessionId: ptySessionId,
          projectRoot: projectPath,
          rootPane: leaf,
          activePaneId: leafId,
        };
        openTabs.value.push(newTab);
        activeTabId.value = newTab.id;
        return newTab;
      } else {
        if (!existingTab.rootPane) {
          const leafId = `pane-${nanoid(8)}`;
          existingTab.rootPane = {
            kind: "leaf",
            id: leafId,
            sessionId: ptySessionId,
            cliKind: ptySession?.cliKind || "shell",
            projectPath: existingTab.projectRoot || "",
            createdAt: Date.now(),
          };
          existingTab.activePaneId = leafId;
        } else {
          for (const lid of leafIds(existingTab.rootPane)) {
            const leaf = findLeaf(existingTab.rootPane, lid);
            if (leaf?.sessionId === ptySessionId) {
              existingTab.activePaneId = lid;
              break;
            }
          }
        }
        activeTabId.value = existingTab.id;
        return existingTab;
      }
    }

    // Create a new PTY session
    const cliKind = options.cliKind || "shell";
    const projectPath = options.projectPath || "";
    const ptyInfo = await createPtySession(
      projectPath,
      cliKind,
      undefined,
      undefined,
      options.skipPermissions,
      options.profileName,
    );

    const leafId = `pane-${nanoid(8)}`;
    const defaultLabel: TabLabel = cliKind === "shell"
      ? { kind: "key", key: "dialogs.common.terminal" }
      : { kind: "text", text: ptyInfo.projectPath ? `${basename(ptyInfo.projectPath)} · ${cliKind}` : cliKind };

    const leaf: TerminalPaneLeaf = {
      kind: "leaf",
      id: leafId,
      sessionId: ptyInfo.sessionId,
      cliKind,
      projectPath: ptyInfo.projectPath,
      createdAt: Date.now(),
    };

    const newTab: OpenTab = {
      id: `term-${ptyInfo.sessionId}`,
      type: "terminal",
      label: options.label ? { kind: "text", text: options.label } : defaultLabel,
      sessionId: ptyInfo.sessionId,
      projectRoot: ptyInfo.projectPath,
      rootPane: leaf,
      activePaneId: leafId,
    };
    openTabs.value.push(newTab);
    activeTabId.value = newTab.id;
    return newTab;
  }

  async function splitTerminalTab(
    tabId: string,
    dir: SplitDir,
    options: OpenTerminalTabOptions = {},
  ): Promise<TerminalPaneLeaf | null> {
    const tab = openTabs.value.find((t) => t.id === tabId);
    if (!tab || tab.type !== "terminal") return null;

    if (!tab.rootPane && tab.sessionId) {
      const initialId = `pane-${nanoid(8)}`;
      const pty = ptySessions.value.find((s) => s.sessionId === tab.sessionId);
      tab.rootPane = {
        kind: "leaf",
        id: initialId,
        sessionId: tab.sessionId,
        cliKind: pty?.cliKind || "shell",
        projectPath: tab.projectRoot || "",
        createdAt: Date.now(),
      };
      tab.activePaneId = initialId;
    }

    if (!tab.rootPane) return null;

    const currentLeaf = findLeaf(tab.rootPane, tab.activePaneId || firstLeafId(tab.rootPane));
    const cliKind = options.cliKind || "shell";
    const projectPath = options.projectPath || currentLeaf?.projectPath || tab.projectRoot || "";

    const ptyInfo = await createPtySession(
      projectPath,
      cliKind,
      undefined,
      undefined,
      options.skipPermissions,
      options.profileName,
    );

    const newLeafId = `pane-${nanoid(8)}`;
    const newLeaf: TerminalPaneLeaf = {
      kind: "leaf",
      id: newLeafId,
      sessionId: ptyInfo.sessionId,
      cliKind,
      projectPath: ptyInfo.projectPath,
      createdAt: Date.now(),
    };

    const splitId = `split-${nanoid(8)}`;
    tab.rootPane = splitLeaf(tab.rootPane, tab.activePaneId || firstLeafId(tab.rootPane), splitId, newLeaf, dir);
    tab.activePaneId = newLeafId;
    activeTabId.value = tab.id;
    return newLeaf;
  }

  async function splitActiveTerminalTab(
    dir: SplitDir,
    options: OpenTerminalTabOptions = {},
  ): Promise<TerminalPaneLeaf | null> {
    if (activeOpenTab.value?.type === "terminal") {
      return splitTerminalTab(activeOpenTab.value.id, dir, options);
    }
    const tab = await openTerminalTab(undefined, options);
    return tab.rootPane ? findLeaf(tab.rootPane, tab.activePaneId || firstLeafId(tab.rootPane)) : null;
  }

  async function closeTerminalPane(tabId: string, paneId: string): Promise<void> {
    const tab = openTabs.value.find((t) => t.id === tabId);
    if (!tab || tab.type !== "terminal" || !tab.rootPane) {
      await closeTab(tabId);
      return;
    }

    const leaf = findLeaf(tab.rootPane, paneId);
    if (leaf?.sessionId) {
      await confirmAndClosePtySession(leaf.sessionId);
    }

    const siblingId = siblingLeafOf(tab.rootPane, paneId);
    const newRoot = removeLeaf(tab.rootPane, paneId);

    if (!newRoot) {
      openTabs.value = openTabs.value.filter((t) => t.id !== tabId);
      if (activeTabId.value === tabId) {
        const nextTab = openTabs.value[openTabs.value.length - 1] ?? null;
        activeTabId.value = nextTab?.id ?? null;
        if (nextTab?.type === "history" && nextTab.sessionPath && nextTab.encodedDir != null) {
          selectHistorySessionFromTab(nextTab);
        } else {
          clearActiveSession();
        }
      }
      return;
    }

    tab.rootPane = newRoot;
    if (tab.activePaneId === paneId) {
      tab.activePaneId = siblingId ?? firstLeafId(newRoot);
    }
  }

  async function closeActiveTerminalPaneOrTab(): Promise<void> {
    const tab = activeOpenTab.value;
    if (!tab) return;
    if (tab.type === "terminal" && tab.rootPane && leafIds(tab.rootPane).length > 1) {
      await closeTerminalPane(tab.id, tab.activePaneId || firstLeafId(tab.rootPane));
    } else {
      await closeTab(tab.id);
    }
  }

  function focusNextTerminalPane() {
    const tab = activeOpenTab.value;
    if (tab?.type === "terminal" && tab.rootPane) {
      tab.activePaneId = nextLeafId(tab.rootPane, tab.activePaneId || firstLeafId(tab.rootPane), 1);
    }
  }

  function focusPrevTerminalPane() {
    const tab = activeOpenTab.value;
    if (tab?.type === "terminal" && tab.rootPane) {
      tab.activePaneId = nextLeafId(tab.rootPane, tab.activePaneId || firstLeafId(tab.rootPane), -1);
    }
  }

  function renameTab(tabId: string, newLabel: string) {
    const tab = openTabs.value.find((t) => t.id === tabId);
    if (!tab) return;
    const trimmed = newLabel.trim();
    if (trimmed && trimmed !== tabLabelText(tab.label)) {
      // 用户自己起的名字是用户数据，不翻译、也不换 key
      tab.label = { kind: "text", text: trimmed };
      tab.isCustomLabel = true;
    } else if (!trimmed) {
      tab.isCustomLabel = false;
      if (tab.type === "terminal") {
        tab.label = { kind: "key", key: "dialogs.common.terminal" };
      }
    }
    // `trimmed === 当前显示文案` 时**什么都不做**：重命名对话框是用当前语言的文案预填的
    // （`AgentPanel` 的 `startRename` 传 `tabLabelText`），用户原样确认传回来的就是它。
    // 若在这里写 `{kind:"text"}`，标签就冻结在确认那一刻的语言——正是 `tabLabelText`
    // 的注释禁止的那件事。
    if (tab.type === "terminal" && tab.sessionId) {
      // 自定义名是用户数据，可以快照；回到默认时**删掉**条目而不是写入渲染后的默认文案，
      // 否则 `customLabels` 会把「终端」这个界面词按当时的语言定死。
      if (tab.isCustomLabel) setSessionLabel(tab.sessionId, tabLabelText(tab.label));
      else clearSessionLabel(tab.sessionId);
    }
  }

  function setSplitRatiosInTab(tab: OpenTab, splitId: string, ratios: number[]) {
    if (!tab.rootPane) return;

    function updateInNode(node: TerminalPaneNode): boolean {
      if (node.kind === "split") {
        if (node.id === splitId) {
          node.ratios = ratios;
          return true;
        }
        for (const c of node.children) {
          if (updateInNode(c)) return true;
        }
      }
      return false;
    }

    updateInNode(tab.rootPane);
  }

  function toggleMaximizePane(tabId?: string, paneId?: string) {
    const targetTabId = tabId || activeTabId.value;
    if (!targetTabId) return;
    const tab = openTabs.value.find((t) => t.id === targetTabId);
    if (!tab || tab.type !== "terminal" || !tab.rootPane) return;

    const targetPaneId = paneId || tab.activePaneId || firstLeafId(tab.rootPane);
    if (tab.maximizedPaneId === targetPaneId) {
      tab.maximizedPaneId = null;
    } else {
      tab.maximizedPaneId = targetPaneId;
    }
  }

  function openHistoryTab(sessionPath: string, encodedDir: string, options: { pinned?: boolean; parentSessionPath?: string; parentMessageIndex?: number; label?: string; projectRoot?: string; cliId: CliId }): OpenTab {
    const identity = { cliId: options.cliId, filePath: sessionPath };
    const session = findSessionByIdentity(identity);
    const cliId = options.cliId;
    // 优先用会话自己的 display_name（用户数据）；都没有才是界面文案
    const displayName = options.label || session?.session.display_name;
    const label: TabLabel = displayName
      ? { kind: "text", text: displayName }
      : { kind: "key", key: "session.tabBar.defaultHistoryLabel" };
    const sessionId = session?.session.session_id;
    const projectRoot = options.projectRoot || session?.project.original_path || undefined;
    const existingPinned = openTabs.value.find(
      (tab) => tab.type === "history"
        && !tab.isPreview
        && tab.cliId === cliId
        && tab.sessionPath === sessionPath,
    );
    if (existingPinned) {
      const idx = openTabs.value.indexOf(existingPinned);
      const updated: OpenTab = {
        ...existingPinned,
        sessionId: sessionId ?? existingPinned.sessionId,
        cliId,
        encodedDir,
        projectRoot: projectRoot ?? existingPinned.projectRoot,
        label: options.label ? { kind: "text", text: options.label } : existingPinned.label,
        parentSessionPath: options.parentSessionPath || existingPinned.parentSessionPath,
        parentMessageIndex: options.parentMessageIndex !== undefined ? options.parentMessageIndex : existingPinned.parentMessageIndex,
      };
      openTabs.value.splice(idx, 1, updated);
      activeTabId.value = updated.id;
      selectHistorySessionFromTab(updated);
      return updated;
    }

    const targetId = historyTabId(identity);
    let tab = openTabs.value.find((t) => t.type === "history" && t.isPreview);
    if (options.pinned) {
      if (tab && tab.cliId === cliId && tab.sessionPath === sessionPath) {
        const idx = openTabs.value.indexOf(tab);
        const updated: OpenTab = { ...tab, id: targetId, label, sessionId, sessionPath, encodedDir, projectRoot, cliId, isPreview: false, parentSessionPath: options.parentSessionPath, parentMessageIndex: options.parentMessageIndex };
        openTabs.value.splice(idx, 1, updated);
        tab = updated;
      } else {
        tab = { id: targetId, type: "history", label, sessionId, sessionPath, encodedDir, projectRoot, cliId, isPreview: false, parentSessionPath: options.parentSessionPath, parentMessageIndex: options.parentMessageIndex };
        openTabs.value.push(tab);
      }
    } else if (tab) {
      const idx = openTabs.value.indexOf(tab);
      const updated: OpenTab = { ...tab, id: targetId, label, sessionId, sessionPath, encodedDir, projectRoot, cliId, parentSessionPath: options.parentSessionPath, parentMessageIndex: options.parentMessageIndex };
      openTabs.value.splice(idx, 1, updated);
      tab = updated;
    } else {
      tab = { id: targetId, type: "history", label, sessionId, sessionPath, encodedDir, projectRoot, cliId, isPreview: true, parentSessionPath: options.parentSessionPath, parentMessageIndex: options.parentMessageIndex };
      openTabs.value.push(tab);
    }
    activeTabId.value = tab.id;
    selectHistorySessionFromTab(tab);
    return tab;
  }

  function openFileTab(filePath: string, projectRoot: string, options: { pinned?: boolean } = {}) {
    const existingPinned = openTabs.value.find((t) => t.type === "file" && !t.isPreview && t.filePath === filePath);
    if (existingPinned) {
      activeTabId.value = existingPinned.id;
      return;
    }

    const label: TabLabel = { kind: "text", text: basename(filePath) || filePath };
    const targetId = fileTabId(filePath);
    let tab = openTabs.value.find((t) => t.type === "file" && t.isPreview);
    if (options.pinned) {
      if (tab && tab.filePath === filePath) {
        const idx = openTabs.value.indexOf(tab);
        const updated: OpenTab = { ...tab, id: targetId, label, filePath, projectRoot, isPreview: false };
        openTabs.value.splice(idx, 1, updated);
        tab = updated;
      } else {
        tab = { id: targetId, type: "file", label, filePath, projectRoot, isDirty: false, isPreview: false };
        openTabs.value.push(tab);
      }
    } else if (tab) {
      if (tab.isDirty && tab.filePath) {
        pinFilePreviewTab(tab.id);
        tab = { id: targetId, type: "file", label, filePath, projectRoot, isDirty: false, isPreview: true };
        openTabs.value.push(tab);
      } else {
        fileEditorRefs.delete(tab.id);
        const idx = openTabs.value.indexOf(tab);
        const updated: OpenTab = { ...tab, id: targetId, label, filePath, projectRoot, isDirty: false };
        openTabs.value.splice(idx, 1, updated);
        tab = updated;
      }
    } else {
      tab = { id: targetId, type: "file", label, filePath, projectRoot, isDirty: false, isPreview: true };
      openTabs.value.push(tab);
    }
    activeTabId.value = tab.id;
  }

  function openDiffTab(filePath: string, projectRoot: string, original: string, modified: string, label: string) {
    const id = `diff-${filePath}-${Date.now()}`;
    const tab: OpenTab = { id, type: "diff", label: { kind: "text", text: label }, filePath, projectRoot, diffOriginal: original, diffModified: modified };
    openTabs.value.push(tab);
    activeTabId.value = id;
  }

  async function openCommitDiffTab(filePath: string, projectRoot: string, hash: string, label: string) {
    try {
      const relPath = filePath.startsWith(projectRoot)
        ? filePath.slice(projectRoot.length).replace(/^\//, "")
        : filePath;
      const result = await invokeApp<{ original: string; modified: string; too_large: boolean }>(
        "git_commit_file_diff",
        { path: projectRoot, hash, file: relPath }
      );
      if (result.too_large) {
        await ask(t("dialogs.diffTooLarge.message"), {
          title: t("app.dialog.notice"),
          kind: "info",
          okLabel: t("dialogs.input.confirm"),
          cancelLabel: "",
        });
        return;
      }
      openDiffTab(filePath, projectRoot, result.original, result.modified, label);
    } catch (e: any) {
      console.error("openCommitDiffTab failed:", e);
    }
  }

  function onSelectTab(tabId: string) {
    activeTabId.value = tabId;
    const tab = openTabs.value.find((t) => t.id === tabId);
    selectHistorySessionFromTab(tab);
  }

  async function confirmAndClosePtySession(sessionId: string): Promise<boolean> {
    const ptySession = ptySessions.value.find((s) => s.sessionId === sessionId);
    if (!ptySession) return true;
    if (ptySession.status !== "exited") {
      const confirmed = await ask(t("dialogs.closeTerminal.message"), {
        title: t("dialogs.closeTerminal.title"),
        kind: "warning",
        okLabel: t("dialogs.common.close"),
        cancelLabel: t("dialogs.common.cancel"),
      });
      if (!confirmed) return false;
    }
    await closePtySession(sessionId);
    return true;
  }

  function closeTerminalTabsForSession(sessionId: string) {
    const ids = new Set(
      openTabs.value
        .filter((tab) => tab.type === "terminal" && tab.sessionId === sessionId)
        .map((tab) => tab.id),
    );
    if (ids.size === 0) return;

    const activeWillClose = activeTabId.value != null && ids.has(activeTabId.value);
    openTabs.value = openTabs.value.filter((tab) => !ids.has(tab.id));
    if (activeWillClose) {
      const nextTab = openTabs.value[openTabs.value.length - 1] ?? null;
      activeTabId.value = nextTab?.id ?? null;
      if (nextTab?.type === "history" && nextTab.sessionPath && nextTab.encodedDir != null) {
        selectHistorySessionFromTab(nextTab);
      } else {
        clearActiveSession();
      }
    }
  }

  async function closePtySessionFromPanel(sessionId: string) {
    const closed = await confirmAndClosePtySession(sessionId);
    if (closed) {
      closeTerminalTabsForSession(sessionId);
    }
  }

  async function closeTab(tabId: string) {
    const tab = openTabs.value.find((t) => t.id === tabId);
    if (!tab) return;

    if (tab.type === "terminal") {
      if (tab.rootPane) {
        for (const lid of leafIds(tab.rootPane)) {
          const l = findLeaf(tab.rootPane, lid);
          if (l?.sessionId) {
            const closed = await confirmAndClosePtySession(l.sessionId);
            if (!closed) return;
          }
        }
      } else if (tab.sessionId) {
        const closed = await confirmAndClosePtySession(tab.sessionId);
        if (!closed) return;
      }
    }

    // Handle dirty file tabs
    if (tab.type === "file" && tab.isDirty) {
      const save = await ask(t("dialogs.closeFile.message"), {
        title: t("dialogs.closeFile.title"),
        kind: "warning",
        okLabel: t("dialogs.closeFile.confirm"),
        cancelLabel: t("dialogs.closeFile.discard"),
      });
      if (save) {
        const editorRef = fileEditorRefs.get(tabId);
        if (editorRef) await editorRef.saveFile();
      }
    }
    fileEditorRefs.delete(tabId);
    if (tab.type === "history" && tab.sessionPath && tab.cliId) {
      historyChatViewRefs.delete(sessionIdentityKey({ cliId: tab.cliId, filePath: tab.sessionPath }));
    }

    openTabs.value = openTabs.value.filter((t) => t.id !== tabId);
    if (activeTabId.value === tabId) {
      const nextTab = openTabs.value[openTabs.value.length - 1] ?? null;
      activeTabId.value = nextTab?.id ?? null;
      if (nextTab?.type === "history" && nextTab.sessionPath && nextTab.encodedDir != null) {
        selectHistorySessionFromTab(nextTab);
      } else {
        clearActiveSession();
      }
    }
  }

  function closeTabsBySessionIdentity(identity: SessionIdentity) {
    const tabs = openTabs.value.filter(
      (tab) => tab.type === "history"
        && tab.cliId === identity.cliId
        && tab.sessionPath === identity.filePath,
    );
    for (const tab of tabs) closeTab(tab.id);
  }

  async function batchDeleteAndCloseTabs() {
    const identities = [...selectedSessionIdentities.value];
    const deleted = await batchDeleteSessions();
    if (deleted) {
      for (const identity of identities) closeTabsBySessionIdentity(identity);
    }
  }

  async function closeOtherTabs(tabId: string) {
    for (const tab of [...openTabs.value]) {
      if (tab.id !== tabId) {
        await closeTab(tab.id);
      }
    }
  }

  async function closeRightTabs(tabId: string) {
    const index = openTabs.value.findIndex((t) => t.id === tabId);
    if (index === -1) return;
    const tabsToClose = openTabs.value.slice(index + 1);
    for (const tab of tabsToClose) {
      await closeTab(tab.id);
    }
  }

  async function closeSavedTabs() {
    const savedTabs = openTabs.value.filter(
      (t) => (t.type === "file" && !t.isDirty) || t.type === "history" || t.type === "diff"
    );
    for (const tab of savedTabs) {
      await closeTab(tab.id);
    }
  }

  async function closeAllTabs() {
    // closeTab 内部已处理终端/脏文件逐个确认与 activeTabId 回退
    for (const tab of [...openTabs.value].reverse()) {
      await closeTab(tab.id);
    }
  }

  return {
    openTabs,
    activeTabId,
    activeOpenTab,
    shouldShowDashboard,
    tabItems,
    terminalTabs,
    fileTabs,
    diffTabs,
    historyTabs,
    showHistoryChat,
    historyChatViewRefs,
    registerEditorRef,
    registerHistoryChatViewRef,
    onFileDirty,
    sanitizeDomId,
    pinHistoryPreviewTab,
    pinFilePreviewTab,
    pinPreviewTab,
    openAgentDashboardTab,
    openTerminalTab,
    splitTerminalTab,
    splitActiveTerminalTab,
    closeTerminalPane,
    closeActiveTerminalPaneOrTab,
    focusNextTerminalPane,
    focusPrevTerminalPane,
    renameTab,
    setSplitRatiosInTab,
    toggleMaximizePane,
    openHistoryTab,
    openFileTab,
    openDiffTab,
    openCommitDiffTab,
    onSelectTab,
    closeTab,
    closeTabsBySessionIdentity,
    closePtySessionFromPanel,
    batchDeleteAndCloseTabs,
    closeOtherTabs,
    closeRightTabs,
    closeSavedTabs,
    closeAllTabs,
    findSessionByIdentity,
  };
}

<script setup lang="ts">
import { onMounted, onBeforeUnmount, ref, reactive, computed, nextTick, watch, defineAsyncComponent } from "vue";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { invokeApp, renderAppError } from "./utils/invokeApp";
import { listen } from "@tauri-apps/api/event";
import type { TauriUpdateInfo } from "./types/updater";
import { getVersion } from "@tauri-apps/api/app";
import ChatView from "./components/ChatView.vue";
import ActivityBar from "./components/ActivityBar.vue";
import type { ActivityTab } from "./types/activity";
import AgentPanel from "./components/AgentPanel.vue";
import HistoryPanel from "./components/HistoryPanel.vue";
import FavoritesPanel from "./components/FavoritesPanel.vue";
import BookmarkListView from "./components/BookmarkListView.vue";
import TabBar from "./components/TabBar.vue";
import DashboardView, { type DashboardRecentConversation } from "./components/DashboardView.vue";
import AgentDashboardView from "./components/AgentDashboardView.vue";
import ProjectPanel from "./components/ProjectPanel.vue";
import { useTheme } from "./composables/useTheme";
import GitPanel from "./components/GitPanel.vue";
import { useGitState } from "./composables/useGitState";

const MonacoEditor = defineAsyncComponent(() => import("./components/MonacoEditor.vue"));
const MonacoDiffEditor = defineAsyncComponent(() => import("./components/MonacoDiffEditor.vue"));
const HtmlPreview = defineAsyncComponent(() => import("./components/HtmlPreview.vue"));
const MdPreview = defineAsyncComponent(() => import("./components/MdPreview.vue"));
const ImagePreview = defineAsyncComponent(() => import("./components/ImagePreview.vue"));
const AgentTerminal = defineAsyncComponent(() => import("./components/AgentTerminal.vue"));
const TerminalPaneTree = defineAsyncComponent(() => import("./components/terminal/TerminalPaneTree.vue"));
import { leafIds, firstLeafId } from "./utils/panes";
import NewSessionDialog from "./components/NewSessionDialog.vue";
import ResumeSessionDialog from "./components/ResumeSessionDialog.vue";
import ExportSessionDialog from "./components/ExportSessionDialog.vue";
import { usePtySession } from "./composables/usePtySession";
import { useAgentStatusMode } from "./composables/useAgentStatusMode";
import { ask, message } from "@tauri-apps/plugin-dialog";
import SessionHeader from "./components/SessionHeader.vue";
import TimelineView from "./components/TimelineView.vue";
import GlobalSearch from "./components/GlobalSearch.vue";
import StatusBar from "./components/StatusBar.vue";
import ContextMenu from "./components/ContextMenu.vue";
import SettingsView from "./components/SettingsView.vue";
import UsageDashboard from "./components/UsageDashboard.vue";
import ApiDebugDialog from "./components/ApiDebugDialog.vue";
import InputDialog from "./components/InputDialog.vue";
import WhatsNewDialog from "./components/WhatsNewDialog.vue";
import FeedbackDialog from "./components/FeedbackDialog.vue";
import FirstRunOnboarding from "./components/FirstRunOnboarding.vue";

import SvgIcon from "./components/icons/SvgIcon.vue";
import mascot from "./assets/mascot.png";
import { changelog } from "./changelog";
import type { ChangelogEntry } from "./changelog";
import { useSessions } from "./composables/useSessions";
import { useUpdater } from "./composables/useUpdater";
import { useWorkspaces } from "./composables/useWorkspaces";
import { findBestProjectPath } from "./utils/projectPath";
import { useSessionIdResolver } from "./composables/useSessionIdResolver";
import { useFavorites } from "./composables/useFavorites";
import { useTerminalApp } from "./composables/useTerminalApp";
import { useAutoIndex } from "./composables/useAutoIndex";
import { useBlockedFolders } from "./composables/useBlockedFolders";
import { renameSession } from "./composables/renameSession";
import { useRightSidebar } from "./composables/useRightSidebar";
import { useTabs, AGENT_DASHBOARD_TAB_ID, tabLabelText, type OpenTab } from "./composables/useTabs";
import { sessionIdentityKey, type ChatMessage, type ProjectInfo, type SessionIdentity, type SessionStats, type BookmarkInfo, type SearchResult, type ContextMenuItem } from "./types/session";
import { getCliDefinition, isCliId, type CliId } from "./types/cli";
import "./styles/transitions.css";
import { copyToClipboard, copyPromiseToClipboard } from "./utils/clipboard";
import { t } from "./i18n";
import { changeLocale } from "./i18n";
import { completeFirstRun, loadStartupMode, runStartup, type FirstRunSelection, type StartupMode } from "./onboarding/startup";
import { shouldDismissStartupSplash } from "./onboarding/startupSplash";

const props = withDefaults(defineProps<{ initialStartupMode?: StartupMode }>(), {
  initialStartupMode: "ready",
});
const startupMode = ref<StartupMode>(props.initialStartupMode);
const hasShownOnboarding = props.initialStartupMode !== "ready";
const onboardingDetectionDone = ref(false);
const onboardingSaving = ref(false);
const onboardingError = ref("");


const {
  projects,
  searchQuery,
  selectedSessionPath,
  filteredProjects,
  totalSessionCount,
  currentCli,
  currentCliId,
  sortMode,
  setSortMode,
  cliOptions,
  cliSessionCounts,
  setCliFilter,
  cliFilter,
  saveCliFilter,
  setSessionUpdatesEnabled,
  visibleCliIds,
  launchCliId,
  recordSuccessfulLaunch,
  refresh,
  loadCliStatuses,
  clearActiveSession,
  ensureSessionVisible,
  deleteSession,
  deleteProject,
  exportSession,
  newSession,
  resumeSession,
  forkSession,
  ensureSessionReadyOnDisk,
  registerContextMenu,
  unregisterContextMenu,
  selectedSessionIdentities,
  toggleSessionSelect,
  clearSessionSelection,
  selectAllSessions,
  isAllSelected,
  selectProjectSessions,
  isProjectAllSelected,
  batchExportSessions,
  batchDeleteProgressText,
  deleteProgress,
  isStreamingProjects,
  totalLoadedSessions,
  isRefreshing,
  showLoadingIndicator,
  bootstrapping,
  hasCompletedInitialScan,
  initialScanSettled,
  hasDetectedCliWithSessions,
  startupPhase,
  startupSlow,
  skipPermissions,
  cliBinaryStatuses,
  globalSearch,
  globalSearchResults,
  globalSearchLoading,
  scanCliErrors,
  retryCliScan,
  searchPendingCliIds,
  searchStaleCliIds,
  searchCliErrors,
  buildingCliIds,
  buildPendingSearchIndex,
  buildAllPendingSearchIndexes,
  searchIndexProgress,
} = useSessions();
if (hasShownOnboarding) setSessionUpdatesEnabled(false);
const { terminalAppForLaunch } = useTerminalApp();
const { isFavorite, toggleFavorite, loadFavorites } = useFavorites();
const { blockFolder, loadBlockedFolders } = useBlockedFolders();
const { maybeAutoCheck, ensureUpdateEventListeners, settings: updaterSettings } = useUpdater();
const { resolvedTheme } = useTheme();
const isDark = computed(() => resolvedTheme.value === "dark");
const { setContextPath } = useGitState();
const { manualPaths, mergedProjectPaths } = useWorkspaces();

const { sessions: ptySessions, waitingInputCount, createSession: createPtySession } = usePtySession();
const { agentStatusMode } = useAgentStatusMode();

const activeActivityTab = ref<ActivityTab>("history");
const sidebarCollapsed = ref(false);

const {
  rightSidebarOpen,
  rightSidebarMode,
  rightSidebarWidth,
  isRightResizing,
  toggleRightSidebar,
  openRightSidebar,
  closeRightSidebar,
  setRightSidebarMode,
  startRightResize,
} = useRightSidebar();

function toggleTimelineSidebar(open?: boolean) {
  if (open === false) {
    if (rightSidebarMode.value === "timeline") {
      closeRightSidebar();
    }
  } else if (rightSidebarOpen.value && rightSidebarMode.value === "timeline") {
    closeRightSidebar();
  } else {
    openRightSidebar("timeline");
  }
}

const {
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
  batchDeleteAndCloseTabs,
  closeOtherTabs,
  closeRightTabs,
  closeAllTabs,
  closeSavedTabs,
  findSessionByIdentity,
} = useTabs();
const { ensureHistorySessionId, historySessionId } = useSessionIdResolver(openTabs);
const agentTerminalRefs = new Map<string, InstanceType<typeof AgentTerminal>>();

const activeProjectDir = computed(() => {
  if (activeOpenTab.value?.projectRoot) return activeOpenTab.value.projectRoot;
  if (activeOpenTab.value?.rootPane?.kind === "leaf" && activeOpenTab.value.rootPane.projectPath) {
    return activeOpenTab.value.rootPane.projectPath;
  }
  if (selectedSession.value?.project.original_path) return selectedSession.value.project.original_path;
  return currentCli.value.dataSourcePath || "";
});

function registerTerminalRef(sessionId: string, el: InstanceType<typeof AgentTerminal> | null) {
  if (el) agentTerminalRefs.set(sessionId, el);
  else agentTerminalRefs.delete(sessionId);
}

const showNewSessionDialog = ref(false);
const newSessionInitialPath = ref<string | null>(null);
const claudeProfiles = ref<string[]>([]);
const contextMenuTargetSession = ref<{
  filePath: string;
  sessionId: string;
  projectPath: string;
  cliId: CliId;
} | null>(null);
const showContextMenuResumeDialog = ref(false);
const showContextMenuExportDialog = ref(false);
// Fork 目标：待分叉的消息锚点（uuid）与其所属会话上下文
const showForkDialog = ref(false);
const forkTarget = ref<{
  filePath: string;
  projectPath: string;
  uuid: string;
  cliId: CliId;
} | null>(null);
const contextMenuTargetSessionIsArchived = computed(() => {
  if (!contextMenuTargetSession.value) return false;
  const { filePath, cliId } = contextMenuTargetSession.value;
  const match = projects.value
    .flatMap((p) => p.sessions)
    .find((s) => s.file_path === filePath && s.cli_id === cliId);
  return match?.is_archived ?? false;
});

async function loadClaudeProfiles() {
  try {
    const names = await invokeApp<string[]>("list_profiles", { cliId: "claude" });
    claudeProfiles.value = names;
  } catch {
    claudeProfiles.value = [];
  }
}

function openNewSessionDialog(projectPath?: string) {
  const launchCli = getCliDefinition(launchCliId.value);
  // 默认启动 CLI 不支持新建对话（如 WorkBuddy）时提示并拦截所有入口
  if (!launchCli.supportsNewSession) {
    message(t("app.dialog.newSessionUnsupported", { cli: launchCli.name }), {
      title: t("app.dialog.notice"),
      kind: "info",
    });
    return;
  }
  newSessionInitialPath.value = projectPath ?? null;
  showNewSessionDialog.value = true;
}

// Derive project root from active tab for Git context, and sync sidebar panel
watch(activeOpenTab, (tab) => {
  if (!tab) {
    setContextPath(null);
    return;
  }
  // Auto-switch sidebar to match tab type
  if (tab.type === "history") activeActivityTab.value = "history";
  else if (tab.type === "terminal") activeActivityTab.value = "agent";
  else if (tab.type === "file") openRightSidebar("project");
  else if (tab.type === "diff") openRightSidebar("git");

  if (tab.type === "terminal") {
    const pty = ptySessions.value.find((s) => s.sessionId === tab.sessionId);
    setContextPath(pty?.projectPath ?? null);
    nextTick(() => {
      const termRef = agentTerminalRefs.get(tab.sessionId!);
      termRef?.fit();
      termRef?.focus();
    });
  } else if (tab.type === "file" || tab.type === "diff") {
    setContextPath(tab.projectRoot ?? null);
  } else if (tab.type === "history") {
    const session = tab.sessionPath && tab.cliId
      ? findSessionByIdentity({ cliId: tab.cliId, filePath: tab.sessionPath })
      : null;
    setContextPath(session?.project.original_path ?? null);
  }
});

function isHtmlFile(filePath: string): boolean {
  const ext = filePath.split(".").pop()?.toLowerCase();
  return ext === "html" || ext === "htm";
}

function isMdFile(filePath: string): boolean {
  const ext = filePath.split(".").pop()?.toLowerCase();
  return ext === "md" || ext === "markdown";
}

function isImageFile(filePath: string): boolean {
  const ext = filePath.split(".").pop()?.toLowerCase();
  // svg 是文本格式（XML），保持走 MonacoEditor 源码编辑，不进入只读图片预览
  return ["png", "jpg", "jpeg", "gif", "webp", "bmp", "ico", "avif"].includes(ext ?? "");
}

// Periodic update check
let updateCheckTimer: ReturnType<typeof setInterval> | undefined;
function setupPeriodicUpdateCheck() {
  clearInterval(updateCheckTimer);
  // Check every hour
  updateCheckTimer = setInterval(() => {
    if (updaterSettings.value.autoCheck) {
      invokeApp("check_tauri_update_bg").catch((err: unknown) => {
        console.error("[update-check] periodic check failed:", err);
      });
    }
  }, 60 * 60 * 1000);
}

// Settings modal
const showSettings = ref(false);
const showUsage = ref(false);
const showApiDebug = ref(false);
const apiDebugDialogRef = ref<any>(null);
const settingsInitialTab = ref<"general" | "api" | "about">("general");
const settingsDialogRef = ref<any>(null);
const settingsInitialCliId = ref<CliId>();
const usageInitialCliId = ref<CliId>();
const apiDebugInitialCliId = ref<CliId>();
const apiDebugInitialSessionId = ref<string>();
const hasUsageCli = computed(() => cliOptions.value.some((cli) => cli.supportsUsageStats));
const hasApiLogCli = computed(() => cliOptions.value.some((cli) => cli.supportsApiLogs));
const canLaunchNewSession = computed(() => getCliDefinition(launchCliId.value).supportsNewSession);

function activeFeatureEntryCliId(): CliId | undefined {
  return activeOpenTab.value?.cliId;
}

function openSettings(
  tab: "general" | "api" | "about" = "general",
  entryCliId: CliId | undefined = activeFeatureEntryCliId(),
) {
  settingsInitialTab.value = tab;
  settingsInitialCliId.value = entryCliId;
  if (showSettings.value && settingsDialogRef.value) {
    settingsDialogRef.value.restoreFromMinimized();
  }
  showSettings.value = true;
}

function openUsageDashboard(entryCliId: CliId | undefined = activeFeatureEntryCliId()) {
  if (!hasUsageCli.value) return;
  usageInitialCliId.value = entryCliId;
  showUsage.value = true;
}

const showFeedback = ref(false);

function openFeedback() {
  showFeedback.value = true;
}

function openAssistant() {
  invokeApp("assistant_open_window").catch((err) => console.error("assistant_open_window failed:", err));
}

function openApiDebug(entryCliId: CliId | undefined = activeFeatureEntryCliId(), sessionId?: string) {
  if (!hasApiLogCli.value) return;
  apiDebugInitialCliId.value = entryCliId;
  apiDebugInitialSessionId.value = sessionId;
  if (showApiDebug.value && apiDebugDialogRef.value) {
    apiDebugDialogRef.value.restoreFromMinimized();
  }
  showApiDebug.value = true;
}


// What's New dialog
const whatsNewEntry = ref<ChangelogEntry | null>(null);

// Refresh state
const STARTUP_SPLASH_MIN_MS = 1400;
const STARTUP_SPLASH_EXIT_DELAY_MS = 420;

const refreshing = ref(false);
const showStartupSplash = ref(true);
let startupSplashTimer: ReturnType<typeof setTimeout> | null = null;
let startupSplashShownAt = Date.now();

const isStartupReadyToDismiss = computed(() => {
  return shouldDismissStartupSplash({
    hasShownOnboarding,
    startupMode: startupMode.value,
    onboardingDetectionDone: onboardingDetectionDone.value,
    initialScanSettled: initialScanSettled.value,
  });
});

const startupStatusText = computed(() => {
  if (startupSlow.value) {
    return t("app.boot.deepIndexing");
  }
  switch (startupPhase.value) {
    case "detecting":
      return t("app.boot.probingCli");
    case "synchronizing":
      return t("app.boot.syncingConfig");
    case "scanning":
      return t("app.boot.scanning");
    case "ready":
      return t("app.boot.ready");
    default:
      return t("app.boot.starting");
  }
});

watch(
  isStartupReadyToDismiss,
  (ready) => {
    if (startupSplashTimer) {
      clearTimeout(startupSplashTimer);
      startupSplashTimer = null;
    }

    if (!ready) {
      startupSplashShownAt = Date.now();
      showStartupSplash.value = true;
      return;
    }

    const elapsed = Date.now() - startupSplashShownAt;
    const remainingMinDisplay = Math.max(0, STARTUP_SPLASH_MIN_MS - elapsed);
    const exitDelay = Math.max(STARTUP_SPLASH_EXIT_DELAY_MS, remainingMinDisplay);

    startupSplashTimer = setTimeout(() => {
      showStartupSplash.value = false;
      startupSplashTimer = null;
    }, exitDelay);
  },
  { immediate: true }
);

let lastSessionListRefreshTime = 0;

async function refreshSessionList() {
  const now = Date.now();
  if (refreshing.value || now - lastSessionListRefreshTime < 800) return;
  lastSessionListRefreshTime = now;
  refreshing.value = true;
  try {
    // 旋转反馈最少 300ms 避免闪烁，最多 8s 超时兜底，绝不让刷新按钮陷入永久旋转
    await Promise.race([
      Promise.all([
        refresh(),
        new Promise((r) => setTimeout(r, 300)),
      ]),
      new Promise((r) => setTimeout(r, 8000)),
    ]);
  } finally {
    refreshing.value = false;
  }
}

// Session stats
const sessionStats = ref<SessionStats | null>(null);
const activeTimelineMessageIndex = ref<number | null>(null);
// 当前活跃会话的代理请求记录数（由 active 的 ChatView 上报），驱动 SessionHeader 的入口 chip
const activeProxyTrafficCount = ref(0);

const messageFilterUser = ref(true);
const messageFilterAssistant = ref(true);
// 工具与思考块默认隐藏：会话打开默认只看用户与助手的纯文本内容
const messageFilterTool = ref(false);
const messageFilterThinking = ref(false);

function resetMessageFilters() {
  messageFilterUser.value = true;
  messageFilterAssistant.value = true;
  messageFilterTool.value = false;
  messageFilterThinking.value = false;
}

const filteredMessageCount = computed(() => activeHistoryChatView()?.filteredMessageCount ?? 0);

const sessionArchivePinned = ref(false);
const sessionArchivePinLoading = ref(false);
let archiveSnapshotSyncTimer: ReturnType<typeof setTimeout> | null = null;

interface SessionArchiveStatus {
  pinned: boolean;
  sourceExists: boolean;
  expiredIfUnpinned: boolean;
  archivedAt?: string | null;
  retentionDays: number;
}

function clearArchiveSnapshotSyncTimer() {
  if (archiveSnapshotSyncTimer) {
    clearTimeout(archiveSnapshotSyncTimer);
    archiveSnapshotSyncTimer = null;
  }
}

// Tab 级操作必须使用 Tab 自身绑定的 CLI；缺失身份时显式失败，禁止回退全局状态。
function cliIdOfTab(tab?: OpenTab | null): CliId {
  if (!tab?.cliId) throw new Error("history tab is missing a CLI identity");
  return tab.cliId;
}

// 恢复/复制命令只使用 Tab 自身保存或按复合身份解析出的上下文。
function resumeContextOfTab(tab: OpenTab): { projectPath: string; sessionId: string } | null {
  const projectPath = tab.projectRoot;
  const sessionId = historySessionId(tab) ?? tab.sessionId;
  return projectPath && sessionId ? { projectPath, sessionId } : null;
}

// ─── Fork：从任意消息分叉会话 ─────────────────────────
function onForkFromHere(tab: OpenTab, uuid: string) {
  const ctx = resumeContextOfTab(tab);
  if (!ctx || !tab.sessionPath) return;
  const cliId = cliIdOfTab(tab);
  forkTarget.value = {
    filePath: tab.sessionPath,
    projectPath: ctx.projectPath,
    uuid,
    cliId,
  };
  if (cliId === "workbuddy") {
    createForkFromTarget().then((newId) => {
      if (newId) {
        resumeSession(ctx.projectPath, newId, cliId);
      }
    });
    return;
  }
  showForkDialog.value = true;
}

// 截断复制转录并返回新 sessionId；失败时弹错并返回 null
async function createForkFromTarget(): Promise<string | null> {
  const ctx = forkTarget.value;
  if (!ctx) return null;
  try {
    // 归档会话需先解压回磁盘，fork 才能读到源文件
    await ensureSessionReadyOnDisk(ctx.filePath, undefined, ctx.cliId);
    const newSessionId = await forkSession(
      { cliId: ctx.cliId, filePath: ctx.filePath },
      ctx.uuid,
    );
    await refresh("snapshot");
    return newSessionId;
  } catch (e) {
    await message(renderAppError(e), { title: t("app.dialog.forkFailed"), kind: "error" });
    return null;
  }
}

function schedulePinnedArchiveSnapshotSync() {
  clearArchiveSnapshotSyncTimer();
  if (!sessionArchivePinned.value || !selectedSessionPath.value) return;

  const filePath = selectedSessionPath.value;
  const cliId = cliIdOfTab(activeOpenTab.value);
  archiveSnapshotSyncTimer = setTimeout(async () => {
    archiveSnapshotSyncTimer = null;
    if (
      selectedSessionPath.value !== filePath ||
      cliIdOfTab(activeOpenTab.value) !== cliId ||
      !sessionArchivePinned.value
    ) {
      return;
    }
    try {
      await invokeApp<boolean>("set_session_archive_pinned", {
        cliId,
        filePath,
        pinned: true,
      });
    } catch (e) {
      console.error("sync pinned archive snapshot failed:", e);
    }
  }, 10_000);
}

// Bookmarks
const bookmarks = ref<BookmarkInfo[]>([]);
const loadedBookmarkCliIds = new Set<CliId>();
function activeHistoryChatView() {
  const tab = activeOpenTab.value;
  if (tab?.type !== "history" || !tab.sessionPath || !tab.cliId) return null;
  return historyChatViewRefs.get(
    sessionIdentityKey({ cliId: tab.cliId, filePath: tab.sessionPath }),
  ) ?? null;
}

function getActiveMessages(): ChatMessage[] {
  return activeHistoryChatView()?.messages ?? [];
}
function getActiveSubagentMap(): Record<string, import("./types/session").SubagentInfo> {
  return activeHistoryChatView()?.subagentMap ?? {};
}
const activeLoading = computed(() =>
  activeHistoryChatView()?.loading ?? false
);
const activeMessageCount = computed(() => getActiveMessages().length);

const sessionRefreshingKey = ref<string | null>(null);

function activeHistorySessionKey() {
  const tab = activeOpenTab.value;
  if (tab?.type !== "history" || !tab.sessionPath || !tab.cliId) return null;
  return sessionIdentityKey({ cliId: tab.cliId, filePath: tab.sessionPath });
}

let lastSessionRefreshTime = 0;

async function refreshActiveSession() {
  const now = Date.now();
  const key = activeHistorySessionKey();
  // 非会话 tab 无对象可刷；避免并发重复加载同一会话；设置 600ms 最小冷却间隔
  if (!key || sessionRefreshingKey.value !== null || now - lastSessionRefreshTime < 600) return;
  lastSessionRefreshTime = now;
  sessionRefreshingKey.value = key;
  try {
    // 保证至少有短促自然的旋转过渡反馈（250ms），避免后台耗时极短导致图标闪现；
    // 流完成即可平稳收尾，并设置 6s 兜底防悬挂超时，彻底杜绝按钮一直转转很久
    await Promise.race([
      Promise.all([
        historyChatViewRefs.get(key)?.refresh(),
        new Promise((r) => setTimeout(r, 250)),
      ]),
      new Promise((r) => setTimeout(r, 6000)),
    ]);
  } finally {
    sessionRefreshingKey.value = null;
  }
}

// 刷新状态按会话隔离：其他 tab 的加载/刷新不影响当前 tab 的刷新控件；
// 不混入 activeLoading，初次加载期间仍可通过刷新按钮重启卡住的流式加载
const isCurrentSessionRefreshing = computed(() => {
  const key = activeHistorySessionKey();
  return key !== null && sessionRefreshingKey.value === key;
});

// 非会话 tab（如终端）不显示消息计数，避免胶囊按钮空转
const statusBarMessageCount = computed(() =>
  showChat.value && activeHistorySessionKey() !== null ? activeMessageCount.value : null
);

async function loadBookmarks(cliId: CliId = currentCliId.value) {
  try {
    const raw = await invokeApp<string>("read_bookmarks", { cliId });
    const list = JSON.parse(raw) as BookmarkInfo[];
    // 合并而非整体替换：保留其他 CLI 已加载的书签，避免跨 CLI Tab 书签丢失
    bookmarks.value = [...bookmarks.value.filter((b) => b.cliId !== cliId), ...list];
    loadedBookmarkCliIds.add(cliId);
  } catch (e) {
    console.error("read_bookmarks failed:", e);
  }
}

// 收藏 Tab 的变更可能发生在任意可见 CLI 上：全量刷新各 CLI 切片
function refreshVisibleBookmarks() {
  for (const cliId of visibleCliIds.value) void loadBookmarks(cliId);
}

function sessionBookmarksFor(sessionId: string | undefined, cliId: CliId) {
  if (!sessionId) return [];
  return bookmarks.value.filter((b) => b.cliId === cliId && b.sessionId === sessionId);
}

async function onToggleBookmark(messageIndex: number) {
  const tab = activeOpenTab.value;
  if (!tab || tab.type !== "history" || !tab.sessionPath) return;

  const sessionId = await ensureHistorySessionId(tab.sessionPath, cliIdOfTab(tab));
  if (!sessionId) return;

  const msg = getActiveMessages()[messageIndex];
  if (!msg) return;

  const messageRole = msg.role ?? null;
  const messageText = msg.content_parts
    .filter((p): p is { type: "text"; text: string } => p.type === "text")
    .map((p) => p.text)
    .join("\n");
  const messageTimestamp = msg.timestamp ?? null;
  const sessionDisplayName = findSessionByIdentity({
    cliId: cliIdOfTab(tab),
    filePath: tab.sessionPath,
  })?.session.display_name || tabLabelText(tab.label) || null;

  try {
    const tabCliId = cliIdOfTab(tab);
    const raw = await invokeApp<string>("toggle_bookmark", {
      cliId: tabCliId,
      sessionId,
      messageIndex,
      messageRole,
      messageText,
      messageTimestamp,
      sessionDisplayName,
    });
    // 与 loadBookmarks 相同的合并语义：仅替换该 CLI 的书签切片
    const list = JSON.parse(raw) as BookmarkInfo[];
    bookmarks.value = [...bookmarks.value.filter((b) => b.cliId !== tabCliId), ...list];
    bookmarkPanelRef.value?.refresh();
  } catch (e) {
    console.error("toggle_bookmark failed:", e);
  }
}

function onScrollToBookmark(messageIndex: number) {
  activeHistoryChatView()?.scrollToMessage(messageIndex);
}

function onJumpToBookmark(identity: SessionIdentity, encodedDir: string, messageIndex: number) {
  openHistoryTab(identity.filePath, encodedDir, { cliId: identity.cliId });
  nextTick(() => {
    const tryJump = (attempts = 8) => {
      const view = activeHistoryChatView();
      if (view) {
        view.scrollToMessage(messageIndex);
      } else if (attempts > 0) {
        setTimeout(() => tryJump(attempts - 1), 50);
      }
    };
    tryJump();
  });
}

async function onOpenSessionFromSettings(identity: SessionIdentity) {
  showSettings.value = false;
  const encodedDir = await ensureSessionVisible(identity.filePath);
  openHistoryTab(identity.filePath, encodedDir ?? "", { cliId: identity.cliId });
}

async function loadSessionArchivePinned(filePath: string | null) {
  sessionArchivePinned.value = false;
  if (!filePath) return;
  try {
    sessionArchivePinned.value = await invokeApp<boolean>("get_session_archive_pinned", {
      cliId: cliIdOfTab(activeOpenTab.value),
      filePath,
    });
  } catch (e) {
    console.error("get_session_archive_pinned failed:", e);
  }
}

async function toggleSessionArchivePinned(targetFilePath?: string) {
  const filePath = (typeof targetFilePath === "string" && targetFilePath ? targetFilePath : null) || selectedSessionPath.value || selectedSession.value?.session.file_path;
  if (!filePath || sessionArchivePinLoading.value) return;
  pinHistoryPreviewTab();
  const nextPinned = !sessionArchivePinned.value;
  if (!nextPinned) {
    try {
      const status = await invokeApp<SessionArchiveStatus>("get_session_archive_status", {
        cliId: cliIdOfTab(activeOpenTab.value),
        filePath,
      });
      if (status.pinned && !status.sourceExists && status.expiredIfUnpinned) {
        const retention =
          status.retentionDays <= 0
            ? t("app.dialog.retentionImmediate")
            : t("app.dialog.retentionDays", { days: status.retentionDays });
        const confirmed = await ask(t("app.dialog.retentionWarning", { retention }), {
          title: t("app.dialog.cancelPermanent"),
          kind: "warning",
        });
        if (!confirmed) return;
      }
    } catch (e) {
      console.error("get_session_archive_status failed:", e);
    }
  }

  sessionArchivePinLoading.value = true;
  try {
    sessionArchivePinned.value = await invokeApp<boolean>("set_session_archive_pinned", {
      cliId: cliIdOfTab(activeOpenTab.value),
      filePath,
      pinned: nextPinned,
    });
    await refresh("snapshot");
  } catch (e) {
    console.error("set_session_archive_pinned failed:", e);
  } finally {
    sessionArchivePinLoading.value = false;
  }
}

async function openProjectInFileManager(projectPath: string) {
  try {
    await invokeApp("open_path_in_file_manager", { path: projectPath });
  } catch (e) {
    console.error("open_path_in_file_manager failed:", e);
  }
}

watch(selectedSessionPath, async (filePath) => {
  sessionStats.value = null;
  if (!filePath) return;
  const cliId = cliIdOfTab(activeOpenTab.value);
  if (!getCliDefinition(cliId).supportsUsageStats) return;

  try {
    sessionStats.value = await invokeApp<SessionStats>("get_session_stats", {
      cliId,
      filePath,
    });
  } catch (e) {
    console.error("get_session_stats failed:", e);
  }
});

// Sidebar resizing
const sidebarWidth = ref(280);
const isResizing = ref(false);

function startResize(e: MouseEvent) {
  isResizing.value = true;
  document.body.style.userSelect = "none";
  const startX = e.clientX;
  const startWidth = sidebarWidth.value;

  const onMove = (ev: MouseEvent) => {
    const newWidth = startWidth + (ev.clientX - startX);
    sidebarWidth.value = Math.max(180, Math.min(newWidth, 600));
  };
  const onUp = () => {
    isResizing.value = false;
    document.body.style.userSelect = "";
    document.removeEventListener("mousemove", onMove);
    document.removeEventListener("mouseup", onUp);
  };
  document.addEventListener("mousemove", onMove);
  document.addEventListener("mouseup", onUp);
}

// Context menu
const ctxMenu = ref({
  visible: false,
  x: 0,
  y: 0,
  items: [] as ContextMenuItem[],
});
const contextMenuSessionIdentity = ref<SessionIdentity | null>(null);
let sessionIdCopyTimer: ReturnType<typeof setTimeout> | undefined;

// HistoryPanel ref for inline rename
const historyPanelRef = ref<InstanceType<typeof HistoryPanel> | null>(null);
const bookmarkPanelRef = ref<InstanceType<typeof BookmarkListView> | null>(null);

// title 与 description 的初值留空：两条打开路径都在 open = true 之前把两者覆写，
// 初值不会被渲染，而 reactive() 的初值在声明处求值一次，写成 t(…) 只会在切换语言后
// 留下一份不再重算的旧语言文案。
const renameDialog = reactive<{
  open: boolean;
  title: string;
  description: string;
  currentName: string;
  identity: SessionIdentity | null;
  tabId: string | null;
}>({
  open: false,
  title: "",
  description: "",
  currentName: "",
  identity: null,
  tabId: null,
});

function openRenameDialog(identity: SessionIdentity, displayName: string) {
  renameDialog.identity = identity;
  renameDialog.tabId = null;
  renameDialog.title = t("app.dialog.renameSession");
  renameDialog.description = t("app.dialog.renameSessionDescription");
  renameDialog.currentName = displayName;
  renameDialog.open = true;
}

function openTabRenameDialog(tabId: string, currentName: string) {
  renameDialog.tabId = tabId;
  renameDialog.identity = null;
  renameDialog.title = t("app.dialog.renameTab");
  renameDialog.description = t("app.dialog.renameTabDescription");
  renameDialog.currentName = currentName;
  renameDialog.open = true;
}

async function handleConfirmRename(newName: string) {
  if (renameDialog.tabId) {
    renameTab(renameDialog.tabId, newName);
    renameDialog.open = false;
    return;
  }
  if (renameDialog.identity) {
    const target = renameDialog.identity;
    renameDialog.open = false;
    await onRenameSession(target, newName);
  }
}

async function onRenameSession(identity: SessionIdentity, newName: string) {
  try {
    await renameSession(identity, newName);
    await refresh("snapshot");
  } catch (e) {
    console.error("rename_session failed:", e);
  }
}

function clearTextSelection() {
  if (typeof window === "undefined") return;
  window.getSelection()?.removeAllRanges();
  requestAnimationFrame(() => {
    window.getSelection()?.removeAllRanges();
  });
}

function closeContextMenu() {
  ctxMenu.value.visible = false;
  contextMenuSessionIdentity.value = null;
  clearTextSelection();
}

function onContextMenuSession(
  event: MouseEvent,
  identity: SessionIdentity,
  sessionId: string,
  displayName: string,
  projectPath: string,
) {
  const { cliId, filePath } = identity;
  clearTextSelection();
  contextMenuSessionIdentity.value = identity;
  // 星标批量语义：右键目标在多选集合内则作用于全部选中项，否则仅作用于自身；
  // 全部已星标则显示"取消星标"，否则"星标"（只补齐未星标项，不翻转已星标项）。
  const favoriteTargets = selectedSessionIdentities.value.some(
    (item) => sessionIdentityKey(item) === sessionIdentityKey(identity),
  )
    ? [...selectedSessionIdentities.value]
    : [identity];
  const allFavorited = favoriteTargets.every((item) => isFavorite(item));
  ctxMenu.value = {
    visible: true,
    x: event.clientX,
    y: event.clientY,
    items: [
      {
        id: "select",
        label: selectedSessionIdentities.value.some(
          (item) => sessionIdentityKey(item) === sessionIdentityKey(identity),
        ) ? t("app.ctxMenu.deselect") : t("app.ctxMenu.select"),
        icon: "check-square",
        action: () => toggleSessionSelect(identity),
      },
      {
        id: "favorite",
        label: allFavorited ? t("app.ctxMenu.unstar") : t("app.ctxMenu.star"),
        icon: "star",
        action: async () => {
          for (const target of favoriteTargets) {
            if (allFavorited || !isFavorite(target)) {
              await toggleFavorite(target);
            }
          }
        },
      },
      {
        id: "rename",
        label: t("app.ctxMenu.rename"),
        icon: "pencil",
        action: () => openRenameDialog(identity, displayName),
      },
      {
        id: "copy",
        label: t("app.ctxMenu.copy"),
        icon: "copy",
        children: [
          {
            id: "copySessionId",
            label: t("app.ctxMenu.copySessionId"),
            icon: "copy",
            closeOnClick: false,
            action: async () => {
              // 按 id 回查而非按文案：文案随语言变化，用文案当键在翻译后必然失效
              const parent = ctxMenu.value.items.find((entry) => entry.id === "copy");
              const item = parent?.children?.find((entry) => entry.id === "copySessionId");
              const ok = await copyToClipboard(sessionId);
              if (item) {
                item.label = ok ? t("dialogs.common.copied") : t("dialogs.common.copyFailed");
                item.icon = ok ? "check" : "alert-circle";
              }
              clearTimeout(sessionIdCopyTimer);
              sessionIdCopyTimer = setTimeout(() => {
                closeContextMenu();
              }, ok ? 650 : 1100);
            },
          },
          {
            id: "copySessionPath",
            label: t("app.ctxMenu.copySessionPath"),
            icon: "copy",
            closeOnClick: false,
            action: async () => {
              const parent = ctxMenu.value.items.find((entry) => entry.id === "copy");
              const item = parent?.children?.find((entry) => entry.id === "copySessionPath");
              const ok = await copyToClipboard(filePath);
              if (item) {
                item.label = ok ? t("dialogs.common.copied") : t("dialogs.common.copyFailed");
                item.icon = ok ? "check" : "alert-circle";
              }
              clearTimeout(sessionIdCopyTimer);
              sessionIdCopyTimer = setTimeout(() => {
                closeContextMenu();
              }, ok ? 650 : 1100);
            },
          },
        ],
      },
      ...(getCliDefinition(cliId).supportsResumeSession
        ? [
            {
              id: "resume",
              label: t("app.ctxMenu.resume"),
              icon: "terminal",
              action: () => {
                // WorkBuddy 直接深链跳转 WorkBuddy 应用，不弹恢复对话框
                if (cliId === "workbuddy") {
                  resumeSession(projectPath, sessionId, cliId);
                  return;
                }
                contextMenuTargetSession.value = { filePath, sessionId, projectPath, cliId };
                showContextMenuResumeDialog.value = true;
              },
            },
          ]
        : []),
      {
        id: "export",
        label: t("app.batch.export"),
        icon: "download",
        action: () => {
          contextMenuTargetSession.value = { filePath, sessionId, projectPath, cliId };
          showContextMenuExportDialog.value = true;
        },
      },
      {
        id: "blockFolder",
        label: t("app.ctxMenu.blockFolder"),
        icon: "folder-x",
        action: () => blockFolder(projectPath),
      },
      // 库型源（OpenCode）没有可删的文件：删除入口连同其分隔线一并隐藏，
      // 与 `supportsResumeSession` 同一处能力门。后端也会把这类请求判失败而非谎报成功。
      ...(getCliDefinition(cliId).supportsDelete
        ? [
            { label: "", action: () => {}, separator: true },
            {
              id: "deleteSession",
              label: t("app.ctxMenu.deleteSession"),
              icon: "trash-2",
              danger: true,
              action: async () => {
                const deleted = await deleteSession({ cliId, filePath }, displayName);
                if (deleted) closeTabsBySessionIdentity(identity);
              },
            },
          ]
        : []),
    ],
  };
}

function onContextMenuProject(event: MouseEvent, project: ProjectInfo) {
  clearTextSelection();
  const projectKey =
    (project as ProjectInfo & { project_key?: string }).project_key ?? project.encoded_dir ?? "";
  const allSelected = isProjectAllSelected(projectKey);
  contextMenuSessionIdentity.value = null;
  ctxMenu.value = {
    visible: true,
    x: event.clientX,
    y: event.clientY,
    items: [
      {
        id: "newSession",
        label: t("app.ctxMenu.newSession"),
        icon: "plus",
        // 没有项目路径时「在这里新建会话」没有落点：与对话框的「未指定初始目录」同义
        action: () => openNewSessionDialog(project.original_path ?? undefined),
      },
      { label: "", action: () => {}, separator: true },
      {
        id: "selectProjectSessions",
        label: allSelected ? t("app.ctxMenu.deselectProject") : t("app.ctxMenu.selectProject"),
        icon: "check-square",
        action: () => selectProjectSessions(projectKey),
      },
      { label: "", action: () => {}, separator: true },
      {
        id: "blockProjectFolder",
        label: t("app.ctxMenu.blockProjectFolder"),
        icon: "folder-x",
        // `blockFolder` 对空串是安全空操作（它自己 trim 后判空），不在这里另造分支
        action: () => blockFolder(project.original_path ?? ""),
      },
      {
        id: "deleteProjectSessions",
        label: t("app.ctxMenu.deleteProjectSessions"),
        icon: "trash-2",
        danger: true,
        action: async () => {
          const identities = project.sessions
            .filter((session) => isCliId(session.cli_id))
            .map((session) => ({ cliId: session.cli_id as CliId, filePath: session.file_path }));
          const deleted = await deleteProject(project);
          if (deleted) {
            for (const identity of identities) closeTabsBySessionIdentity(identity);
          }
        },
      },
    ],
  };
}

// Global search
const showGlobalSearch = ref(false);
const globalSearchRef = ref<InstanceType<typeof GlobalSearch> | null>(null);
const globalSearchInitialQuery = ref("");
const showChatSearch = ref(true);

// Dashboard：内联搜索状态（数据链路复用 useSessions.globalSearch，范围 visibleCliIds）
const dashboardRef = ref<InstanceType<typeof DashboardView> | null>(null);
const dashboardSearchQuery = ref("");

function openGlobalSearch(initialQuery = "") {
  globalSearchInitialQuery.value = initialQuery;
  showGlobalSearch.value = true;
  nextTick(() => {
    globalSearchRef.value?.focusInput();
  });
}

function closeGlobalSearch() {
  showGlobalSearch.value = false;
  globalSearchInitialQuery.value = "";
}

// Dashboard：visibleCliIds 范围内跨项目聚合的最近对话（按时间倒序取前 6 条）
const recentConversations = computed<DashboardRecentConversation[]>(() => {
  const visible = new Set(visibleCliIds.value);
  const items: DashboardRecentConversation[] = [];
  for (const project of projects.value) {
    for (const session of project.sessions) {
      if (!isCliId(session.cli_id) || !visible.has(session.cli_id)) continue;
      items.push({
        cliId: session.cli_id,
        filePath: session.file_path,
        encodedDir: project.encoded_dir ?? "",
        projectPath: project.original_path,
        title: session.display_name || t("app.session.untitled"),
        timestamp: session.timestamp,
      });
    }
  }
  items.sort((a, b) => b.timestamp.localeCompare(a.timestamp));
  return items.slice(0, 6);
});

const activeAgentCount = computed(() => ptySessions.value.filter((s) => s.status === "active").length);

function onDashboardAskAssistant(prompt: string) {
  invokeApp("assistant_open_with_prompt", { prompt }).catch((err) =>
    console.error("assistant_open_with_prompt failed:", err),
  );
}

function onDashboardSearch(query: string) {
  // 搜索模式提交：结果内联展示在 Dashboard，不再直接弹 GlobalSearch
  dashboardSearchQuery.value = query;
  globalSearch(query);
}

async function onDashboardSearchResult(result: SearchResult) {
  await onGlobalSearchSelect(result, dashboardSearchQuery.value);
}

function onDashboardClearSearch() {
  dashboardSearchQuery.value = "";
  globalSearch("");
}



async function onCrossCliSessionSelect(payload: { cliId: string; filePath: string }) {
  if (!isCliId(payload.cliId)) return;
  const encodedDir = await ensureSessionVisible(payload.filePath);
  const projectRoot = projects.value.find((project) =>
    project.sessions.some(
      (session) => session.file_path === payload.filePath && session.cli_id === payload.cliId,
    )
  )?.original_path;
  openHistoryTab(payload.filePath, encodedDir ?? "", {
    cliId: payload.cliId,
    projectRoot: projectRoot ?? undefined,
  });
}

async function onGlobalSearchSelect(result: SearchResult, query: string) {
  if (!isCliId(result.cli_id)) return;
  const encodedDir = await ensureSessionVisible(result.file_path);
  openHistoryTab(result.file_path, encodedDir ?? "", {
    cliId: result.cli_id,
    projectRoot: result.project_path ?? undefined,
  });

  if (query.trim()) {
    showChatSearch.value = true;
    await nextTick();
    activeHistoryChatView()?.openSearchAt(query.trim());
  }
}

function onOpenSearchFromSettings() {
  showSettings.value = false;
  openGlobalSearch();
}

// Current selected session info
const selectedSession = computed(() => {
  const tab = activeOpenTab.value;
  if (tab?.type !== "history" || !tab.sessionPath || !tab.cliId) return null;
  return findSessionByIdentity({ cliId: tab.cliId, filePath: tab.sessionPath });
});

const selectedSessionIdentity = computed<SessionIdentity | null>(() => {
  const tab = activeOpenTab.value;
  return tab?.type === "history" && tab.sessionPath && tab.cliId
    ? { cliId: tab.cliId, filePath: tab.sessionPath }
    : null;
});

const activeHistoryProjectRoot = computed(() => {
  const tab = activeOpenTab.value;
  if (!tab) return null;
  if (tab.type === "history") {
    if (selectedSession.value?.project.original_path) {
      return selectedSession.value.project.original_path;
    }
    if (tab.projectRoot) {
      return tab.projectRoot;
    }
    if (tab.sessionPath) {
      for (const p of projects.value) {
        if (p.sessions.some((s) => s.file_path === tab.sessionPath)) {
          return p.original_path;
        }
      }
    }
  }
  if (tab.projectRoot) {
    return tab.projectRoot;
  }
  if (tab.filePath) {
    return findBestProjectPath(tab.filePath, mergedProjectPaths(projects.value)) || null;
  }
  return null;
});

// Batch operations
const showBatchExportMenu = ref(false);
const batchExportRef = ref<HTMLElement | null>(null);

function onDocumentClick(e: MouseEvent) {
  if (showBatchExportMenu.value && batchExportRef.value && !batchExportRef.value.contains(e.target as Node)) {
    showBatchExportMenu.value = false;
  }
}

function onToggleSelect(identity: SessionIdentity, _event: MouseEvent) {
  toggleSessionSelect(identity);
}

function onRangeSelect(startIdentity: SessionIdentity, endIdentity: SessionIdentity) {
  // Build flat visible session list from filteredProjects
  const list: SessionIdentity[] = [];
  for (const p of filteredProjects.value) {
    for (const s of p.sessions) {
      if (isCliId(s.cli_id)) {
        list.push({ cliId: s.cli_id, filePath: s.file_path });
      }
    }
  }

  const startKey = sessionIdentityKey(startIdentity);
  const endKey = sessionIdentityKey(endIdentity);
  const startIdx = list.findIndex((identity) => sessionIdentityKey(identity) === startKey);
  const endIdx = list.findIndex((identity) => sessionIdentityKey(identity) === endKey);
  if (startIdx === -1 || endIdx === -1) return;

  const lo = Math.min(startIdx, endIdx);
  const hi = Math.max(startIdx, endIdx);
  const range = list.slice(lo, hi + 1);

  // Add all items in range that aren't already selected
  const current = new Map(
    selectedSessionIdentities.value.map((identity) => [sessionIdentityKey(identity), identity]),
  );
  for (const identity of range) {
    current.set(sessionIdentityKey(identity), identity);
  }
  selectedSessionIdentities.value = [...current.values()];
}

async function onCreateNewSession(projectPath: string, cliKind: string, launchMode: "agent" | "terminal", profileName: string | null, skipOverride?: boolean) {
  if (!isCliId(cliKind)) return;
  if (launchMode === "terminal") {
    showNewSessionDialog.value = false;
    await newSession(projectPath, profileName, skipOverride, cliKind);
  } else {
    try {
      const info = await createPtySession(projectPath, cliKind, undefined, undefined, skipOverride ?? skipPermissions.value, profileName, agentStatusMode.value);
      recordSuccessfulLaunch(cliKind);
      showNewSessionDialog.value = false;
      openTerminalTab(info.sessionId);
      activeActivityTab.value = "agent";
    } catch (e: any) {
      await ask(renderAppError(e), { title: t("app.dialog.launchFailed"), kind: "error" });
    }
  }
}

async function resumeSessionInAgent(
  projectPath: string,
  sessionId: string,
  cliId: CliId,
  tabId = activeTabId.value,
  profileName?: string | null,
  skipOverride?: boolean,
) {
  if (!projectPath || !sessionId) return;
  // WorkBuddy 不支持应用内 PTY 恢复，改为深链跳转 WorkBuddy 应用
  if (cliId === "workbuddy") {
    await resumeSession(projectPath, sessionId, cliId);
    return;
  }
  await ensureSessionReadyOnDisk(undefined, sessionId, cliId);
  try {
    pinHistoryPreviewTab(tabId);
    const info = await createPtySession(projectPath, cliId, undefined, sessionId, skipOverride ?? skipPermissions.value, profileName, agentStatusMode.value);
    openTerminalTab(info.sessionId);
    activeActivityTab.value = "agent";
    sidebarCollapsed.value = false;
  } catch (e: any) {
    await ask(renderAppError(e), { title: t("app.dialog.resumeFailed"), kind: "error" });
  }
}

async function copyRestoreCommand(projectPath: string, sessionId: string, cliId: CliId, targetOrCb: any, profileName?: string | null, skipOverride?: boolean) {
  // 立即反馈 UI，避免卡顿感
  if (targetOrCb && typeof targetOrCb !== "function") {
    if (targetOrCb instanceof HTMLElement) {
      const span = targetOrCb.querySelector("span");
      if (span) span.innerText = t("dialogs.common.copied");
    } else {
      targetOrCb.label = t("dialogs.common.copied");
      targetOrCb.icon = "check";
    }
  }

  // 不要先 await invoke 再复制：旧版 WKWebView 中 await 会消耗用户手势激活，
  // 导致剪贴板写入被拒。把 Promise 交给 copyPromiseToClipboard 同步发起写入。
  // 同时异步解压恢复归档会话至本地磁盘，确保用户在外部终端运行复制的命令时源文件已存在。
  ensureSessionReadyOnDisk(undefined, sessionId, cliId).catch((e) => console.warn(e));

  let invokeError: any = null;
  const cmdPromise = invokeApp<string>("get_launch_command", {
    cliId,
    projectPath,
    sessionId,
    skipPermissions: skipOverride ?? skipPermissions.value,
    profileName: profileName || undefined,
    terminalApp: terminalAppForLaunch.value ?? undefined,
  });
  cmdPromise.catch((e) => {
    invokeError = e;
  });
  const ok = await copyPromiseToClipboard(cmdPromise);

  if (typeof targetOrCb === "function") {
    targetOrCb(ok);
  } else if (targetOrCb instanceof HTMLElement) {
    const span = targetOrCb.querySelector("span");
    if (!ok && span) span.innerText = t("dialogs.common.copyFailed");
    setTimeout(() => {
      if (span) span.innerText = profileName ? profileName : t("session.sessionHeader.copyResumeCommand");
    }, ok ? 650 : 1100);
  } else if (targetOrCb) {
    targetOrCb.label = ok ? t("dialogs.common.copied") : t("dialogs.common.copyFailed");
    targetOrCb.icon = ok ? "check" : "alert-circle";
    clearTimeout(sessionIdCopyTimer);
    sessionIdCopyTimer = setTimeout(() => {
      closeContextMenu();
    }, ok ? 650 : 1100);
  } else if (ok) {
    await message(t("common.copyButton.copied"), { kind: "info" });
  } else {
    await message(invokeError ? renderAppError(invokeError) : t("dialogs.common.copyFailed"), { title: t("dialogs.common.copyFailed"), kind: "error" });
  }

  if (!ok && invokeError && targetOrCb) {
    await message(renderAppError(invokeError), { title: t("dialogs.common.copyFailed"), kind: "error" });
  }
}

function exportActiveSession(format: "txt" | "markdown" | "json" | "jsonl") {
  if (!selectedSession.value) return;
  pinHistoryPreviewTab();
  exportSession(
    {
      cliId: cliIdOfTab(activeOpenTab.value),
      filePath: selectedSession.value.session.file_path,
    },
    format,
    selectedSession.value.project.original_path ?? undefined,
    selectedSession.value.session.session_id,
  );
}

function handleOpenSubagent(path: string, label: string) {
  const currentTab = activeOpenTab.value;
  if (!currentTab || currentTab.type !== "history" || !currentTab.sessionPath) return;

  // 不限定工具名（Claude 为 Agent、DSH 为 subagent）：tool_use_id 命中 subagentMap 即视为委托调用
  const parentMessageIndex = activeHistoryChatView()?.messages.findIndex(m =>
    m.role === 'assistant' &&
    m.content_parts.some(p => p.type === 'tool_use' && p.tool_use_id && getActiveSubagentMap()[p.tool_use_id]?.file_path === path)
  );

  openHistoryTab(path, currentTab.encodedDir || "", {
    pinned: true,
    parentSessionPath: currentTab.sessionPath,
    parentMessageIndex: parentMessageIndex !== -1 ? parentMessageIndex : undefined,
    label,
    projectRoot: currentTab.projectRoot,
    cliId: cliIdOfTab(currentTab),
  });
}

function handleBackToParent(tab: OpenTab) {
  if (tab.type === "history" && tab.parentSessionPath) {
    const parentTab = openHistoryTab(tab.parentSessionPath, tab.encodedDir || "", { cliId: cliIdOfTab(tab) });
    if (tab.parentMessageIndex !== undefined && parentTab.sessionPath && parentTab.cliId) {
      setTimeout(() => {
        historyChatViewRefs.get(sessionIdentityKey({
          cliId: parentTab.cliId!,
          filePath: parentTab.sessionPath!,
        }))?.scrollToMessage(tab.parentMessageIndex!);
      }, 100);
    }
  }
}

function toggleSidebar() {
  sidebarCollapsed.value = !sidebarCollapsed.value;
}

function onActivityTabChange(tab: ActivityTab) {
  activeActivityTab.value = tab;
  sidebarCollapsed.value = false;
}

function openHistoryFromDashboard() {
  activeActivityTab.value = "history";
  sidebarCollapsed.value = false;
  // 不改 activeTabId：Dashboard 不是工作 Tab，仅展开侧边栏即可
}

function openAgentFromDashboard() {
  activeActivityTab.value = "agent";
  sidebarCollapsed.value = false;
  openAgentDashboardTab();
}

// Keyboard shortcuts
function onKeyDown(e: KeyboardEvent) {
  if (startupMode.value !== "ready") return;
  const isMac = navigator.platform.toUpperCase().includes("MAC");
  const mod = isMac ? e.metaKey : e.ctrlKey;

  if (e.key === "Escape") {
    e.preventDefault();
    if (showSettings.value) {
      showSettings.value = false;
    } else if (showNewSessionDialog.value) {
      showNewSessionDialog.value = false;
    } else if (showUsage.value) {
      showUsage.value = false;
    } else if (showApiDebug.value) {
      showApiDebug.value = false;
    } else if (showGlobalSearch.value) {
      closeGlobalSearch();
    } else if (selectedSessionIdentities.value.length > 0) {
      clearSessionSelection();
    } else {
      closeContextMenu();
    }
    return;
  }

  // When modal dialogs are open, don't process other shortcuts
  if (showSettings.value || showGlobalSearch.value || showUsage.value || showApiDebug.value || showNewSessionDialog.value) return;

  if (mod && e.shiftKey && e.key.toLowerCase() === "f") {
    e.preventDefault();
    openGlobalSearch();
    return;
  }

  if ((mod && e.key.toLowerCase() === "r") || e.key === "F5") {
    e.preventDefault();
    if (activeHistorySessionKey()) {
      void refreshActiveSession();
    } else {
      void refreshSessionList();
    }
    return;
  }
  if (mod && !e.shiftKey && !e.altKey && e.key.toLowerCase() === "t") {
    e.preventDefault();
    void openTerminalTab(undefined, { cliKind: "shell", projectPath: activeProjectDir.value });
    return;
  }
  if (mod && e.shiftKey && !e.altKey && e.key.toLowerCase() === "d") {
    if (activeOpenTab.value?.type === "terminal") {
      e.preventDefault();
      void splitActiveTerminalTab("col", { cliKind: "shell" });
      return;
    }
  }
  if (mod && !e.shiftKey && !e.altKey && e.key.toLowerCase() === "d") {
    if (activeOpenTab.value?.type === "terminal") {
      e.preventDefault();
      void splitActiveTerminalTab("row", { cliKind: "shell" });
      return;
    }
  }
  if (mod && !e.shiftKey && !e.altKey && e.key.toLowerCase() === "w") {
    if (activeOpenTab.value) {
      e.preventDefault();
      void closeActiveTerminalPaneOrTab();
      return;
    }
  }
  if (mod && !e.shiftKey && !e.altKey && e.key === "]") {
    if (activeOpenTab.value?.type === "terminal") {
      e.preventDefault();
      focusNextTerminalPane();
      return;
    }
  }
  if (mod && !e.shiftKey && !e.altKey && e.key === "[") {
    if (activeOpenTab.value?.type === "terminal") {
      e.preventDefault();
      focusPrevTerminalPane();
      return;
    }
  }
  if (mod && e.shiftKey && !e.altKey && e.key === "Enter") {
    if (activeOpenTab.value?.type === "terminal") {
      e.preventDefault();
      toggleMaximizePane();
      return;
    }
  }
  if (e.key === "Escape" && activeOpenTab.value?.type === "terminal" && activeOpenTab.value?.maximizedPaneId) {
    e.preventDefault();
    toggleMaximizePane();
    return;
  }
  if (mod && e.key.toLowerCase() === "n") {
    e.preventDefault();
    openNewSessionDialog();
    return;
  }
  if (mod && e.key.toLowerCase() === "f") {
    if ((e.target as HTMLElement)?.closest?.(".monaco-editor")) {
      return;
    }
    e.preventDefault();
    if (showChat.value) {
      showChatSearch.value = true;
      nextTick(() => activeHistoryChatView()?.focusSearch());
    } else {
      const input = document.querySelector(".search-input") as HTMLInputElement;
      input?.focus();
    }
    return;
  }
  if (e.key === "Delete" || e.key === "Backspace") {
    const tab = activeOpenTab.value;
    if (
      showHistoryChat.value &&
      tab?.type === "history" &&
      tab.sessionPath &&
      tab.cliId &&
      getCliDefinition(tab.cliId).supportsDelete &&
      !(e.target instanceof HTMLInputElement) &&
      !(e.target instanceof HTMLTextAreaElement)
    ) {
      e.preventDefault();
      const identity = { cliId: tab.cliId, filePath: tab.sessionPath };
      const match = findSessionByIdentity(identity);
      if (!match) return;
      deleteSession(identity, match.session.display_name).then((deleted) => {
        if (deleted) closeTabsBySessionIdentity(identity);
      });
    }
  }
}

// Drag and drop
async function setupDragDrop() {
  const webview = getCurrentWebviewWindow();
  await webview.onDragDropEvent(async (event) => {
    if (event.payload.type === "drop") {
      const paths = event.payload.paths;
      if (paths && paths.length > 0) {
        newSession(paths[0]);
      }
    }
  });
}

const showChat = computed(
  () => selectedSessionPath.value !== null
);

watch(selectedSessionPath, () => {
  showChatSearch.value = true;
  activeTimelineMessageIndex.value = null;
  resetMessageFilters();
});

watch(rightSidebarOpen, () => {
  const chatView = activeHistoryChatView();
  if (!chatView) return;
  chatView.saveScrollAnchor();
  setTimeout(() => {
    chatView.restoreScrollAnchor();
  }, 100);
});

watch([selectedSessionPath, () => activeOpenTab.value?.cliId], ([filePath]) => {
  void loadSessionArchivePinned(filePath);
});

watch(
  () => [selectedSessionPath.value, activeOpenTab.value?.cliId, sessionArchivePinned.value, activeMessageCount.value],
  schedulePinnedArchiveSnapshotSync
);

// 激活 Tab 所属 CLI 的书签尚未加载时按需加载（跨 CLI Tab 书签展示）
watch(
  () => activeOpenTab.value?.cliId,
  (cliId) => {
    if (cliId && !loadedBookmarkCliIds.has(cliId)) void loadBookmarks(cliId);
  }
);

function closeWhatsNew() {
  if (whatsNewEntry.value) {
    localStorage.setItem("seshbuddy-last-seen-version", whatsNewEntry.value.version);
  }
  whatsNewEntry.value = null;
}

// Global update toast
const showGlobalUpdateToast = ref(false);
const updateToastMessage = ref("");

// Global crash recovery toast
const showCrashRecoveryToast = ref(false);

function onGlobalUpdateToastClick() {
  showGlobalUpdateToast.value = false;
  openSettings("about");
}

let sessionWorkStarted = false;

async function startSessionWork() {
  if (sessionWorkStarted) return;
  sessionWorkStarted = true;
  try {
    await invokeApp("set_session_index_scan_clis", { cliIds: cliFilter.value.mode === "all" ? cliOptions.value.map((cli) => cli.id) : cliFilter.value.cliIds });
  } catch (error) {
    console.error("set_session_index_scan_clis failed:", error);
  }
  void invokeApp("refresh_tray_menu", { cliId: currentCliId.value }).catch((error) => {
    console.error("refresh_tray_menu failed:", error);
  });
  void loadClaudeProfiles();
  void loadBookmarks();
  void loadFavorites().catch((error) => {
    console.error("read_favorite_entries failed:", error);
  });
  void loadBlockedFolders().catch((error) => {
    console.error("read_blocked_folders failed:", error);
  });

  const { autoIndexInterval, startAutoIndexScheduler, runSilentIncrementalSync } = useAutoIndex();
  startAutoIndexScheduler(
    () => visibleCliIds.value,
    () => refreshing.value || isRefreshing.value || Boolean(searchIndexProgress.value),
  );
  if (autoIndexInterval.value !== "off") {
    setTimeout(() => {
      void runSilentIncrementalSync(visibleCliIds.value);
    }, 10000);
  }
  try {
    await refresh();
  } finally {
    setSessionUpdatesEnabled(true);
  }
}

async function retryOnboardingStatus() {
  const mode = await loadStartupMode();
  if (mode === "onboarding") {
    onboardingDetectionDone.value = false;
    try {
      await loadCliStatuses();
    } finally {
      startupMode.value = mode;
      onboardingDetectionDone.value = true;
    }
  } else {
    startupMode.value = mode;
  }
  if (mode === "ready") {
    void startSessionWork();
  }
}

function continueWithDefaults() {
  startupMode.value = "ready";
  void startSessionWork();
}

async function finishOnboarding(selection: FirstRunSelection) {
  if (onboardingSaving.value) return;
  onboardingSaving.value = true;
  onboardingError.value = "";
  try {
    await completeFirstRun(selection, {
      saveLocale: changeLocale,
      saveFilter: (cliIds) => saveCliFilter({ mode: "custom", cliIds }),
      markComplete: () => invokeApp<void>("complete_onboarding"),
    });
    startupMode.value = "ready";
    void startSessionWork();
  } catch (error) {
    console.error("complete_onboarding failed:", error);
    onboardingError.value = t("app.onboarding.saveError");
  } finally {
    onboardingSaving.value = false;
  }
}

onMounted(async () => {
  void setupDragDrop();
  document.addEventListener("keydown", onKeyDown);
  document.addEventListener("click", onDocumentClick);

  if (startupMode.value === "status-error") onboardingDetectionDone.value = true;
  void runStartup(startupMode.value, {
    detectClis: async () => {
      try {
        await loadCliStatuses();
      } finally {
        onboardingDetectionDone.value = true;
      }
    },
    startSessions: startSessionWork,
  });

  void listen<TauriUpdateInfo>("update-available", (event) => {
    updateToastMessage.value = t("app.update.available", { version: event.payload.version });
    showGlobalUpdateToast.value = true;
    setTimeout(() => { showGlobalUpdateToast.value = false; }, 10000);
  });

  window.addEventListener("app-crash-recovery", () => {
    showCrashRecoveryToast.value = true;
    setTimeout(() => { showCrashRecoveryToast.value = false; }, 8000);
  });

  void listen("open-settings-about", () => openSettings("about"));
  void listen<{ cliId: string; filePath: string }>("open-session-from-assistant", (e) => {
    void onCrossCliSessionSelect(e.payload);
  });
  void ensureUpdateEventListeners();
  void maybeAutoCheck();

  // Check for version update
  try {
    const currentVersion = await getVersion();
    const lastSeen = localStorage.getItem("seshbuddy-last-seen-version");
    if (lastSeen !== currentVersion) {
      const entry = changelog.find((e) => e.version === currentVersion);
      if (entry) {
        whatsNewEntry.value = entry;
      }
    }
  } catch {
    // Ignore version check errors
  }
  setupPeriodicUpdateCheck();

});

onBeforeUnmount(() => {
  const { stopAutoIndexScheduler } = useAutoIndex();
  stopAutoIndexScheduler();
  clearInterval(updateCheckTimer);
  document.removeEventListener("keydown", onKeyDown);
  document.removeEventListener("click", onDocumentClick);
  if (startupSplashTimer) {
    clearTimeout(startupSplashTimer);
    startupSplashTimer = null;
  }
  clearTimeout(sessionIdCopyTimer);
  clearArchiveSnapshotSyncTimer();
});
</script>

<template>
  <div class="app-layout" :class="{ 'is-startup-active': showStartupSplash }" :inert="startupMode !== 'ready'" :aria-hidden="startupMode !== 'ready' ? 'true' : undefined">
    <div id="minimized-widgets" class="minimized-widgets-container"></div>
    <Transition name="startup-overlay">
      <div v-if="showStartupSplash" class="startup-splash" :class="{ 'is-slow': startupSlow }" role="dialog" :aria-label="t('app.boot.splashAria')">
        <div class="startup-noise"></div>
        <div class="startup-ambient-backdrop"></div>
        <div class="startup-center-stage">
          <div class="startup-beacon-tile" aria-hidden="true">
            <div class="startup-beacon-glow"></div>
            <img class="startup-beacon-image" :src="mascot" alt="" />
          </div>

          <!-- SeshBuddy Brand Typography & Status Lockup -->
          <div class="startup-brand-lockup">
            <div class="startup-brand-title">
              <span class="brand-sesh">Sesh</span><span class="brand-buddy">Buddy</span>
            </div>
            <div class="startup-brand-subtitle">{{ t("app.boot.brandSubtitle") }}</div>

            <!-- Sleek Minimalist Hairline Progress Bar -->
            <div class="startup-progress-track">
              <div class="startup-progress-line"></div>
            </div>

            <div class="startup-status-wrap">
              <span class="startup-status-text">{{ startupStatusText }}</span>
            </div>
          </div>
        </div>
      </div>
    </Transition>

    <Teleport to="body"><FirstRunOnboarding
      v-if="!showStartupSplash && startupMode !== 'ready'"
      :key="startupMode"
      :cli-options="cliOptions"
      :saving="onboardingSaving"
      :error="onboardingError"
      :status-error="startupMode === 'status-error'"
      @complete="finishOnboarding"
      @retry="retryOnboardingStatus"
      @continue-default="continueWithDefaults"
    /></Teleport>


    <div class="content-area">
      <ActivityBar
        :activeMode="activeActivityTab"
        :sidebarCollapsed="sidebarCollapsed"
        :agentBadge="waitingInputCount"
        :showUsage="hasUsageCli"
        :showApiDebug="hasApiLogCli"
        @update:activeMode="onActivityTabChange"
        @toggleSidebar="toggleSidebar"
        @openSettings="openSettings()"
        @openUsage="openUsageDashboard"
        @openApiDebug="openApiDebug"
        @openAssistant="openAssistant"
        @openFeedback="openFeedback"
      />
      <div class="sidebar" v-show="!sidebarCollapsed" :style="{ width: sidebarWidth + 'px' }">
        <AgentPanel
          v-if="activeActivityTab === 'agent'"
          :tabs="terminalTabs"
          :activeTabId="activeTabId"
          @selectTab="onSelectTab"
          @openRenameDialog="openTabRenameDialog"
          @renameTab="renameTab"
          @closeTab="closeTab"
          @newSession="openNewSessionDialog($event)"
          @openProject="openProjectInFileManager"
          @openDashboard="openAgentDashboardTab"
          @splitRight="(tabId) => splitTerminalTab(tabId, 'row', { cliKind: 'shell' })"
          @splitDown="(tabId) => splitTerminalTab(tabId, 'col', { cliKind: 'shell' })"
        />
        <HistoryPanel
          v-else-if="activeActivityTab === 'history'"
          ref="historyPanelRef"
          :projects="filteredProjects"
          :selectedSessionIdentity="selectedSessionIdentity"
          :selectedSessionIdentities="selectedSessionIdentities"
          :contextMenuSessionIdentity="contextMenuSessionIdentity"
          :searchQuery="searchQuery"
          :sortMode="sortMode"
          :isStreamingProjects="isStreamingProjects"
          :totalLoadedSessions="totalLoadedSessions"
          :isRefreshing="isRefreshing"
          :showLoadingIndicator="showLoadingIndicator"
          :bootstrapping="bootstrapping"
          :hasCompletedInitialScan="hasCompletedInitialScan"
          :refreshing="refreshing"
          :visibleCliIds="visibleCliIds"
          :cliOptions="cliOptions"
          :cliSessionCounts="cliSessionCounts"
          :scanCliErrors="scanCliErrors"
          :projectAllSelected="(projectKey: string) => isProjectAllSelected(projectKey)"
          @openFullSearch="openGlobalSearch"
          @update:searchQuery="searchQuery = $event"
          @update:cliFilter="setCliFilter"
          @retryCli="retryCliScan"
          @selectSession="(identity: SessionIdentity, encodedDir: string) => openHistoryTab(identity.filePath, encodedDir, { cliId: identity.cliId })"
          @pinSession="(identity: SessionIdentity, encodedDir: string) => openHistoryTab(identity.filePath, encodedDir, { pinned: true, cliId: identity.cliId })"
          @contextMenuSession="onContextMenuSession"
          @contextMenuProject="onContextMenuProject"
          @toggleSort="setSortMode(sortMode === 'time' ? 'name' : 'time')"
          @refresh="refreshSessionList"
          @renameSession="onRenameSession"
          @toggleSelect="onToggleSelect"
          @rangeSelect="onRangeSelect"
          @clearSelection="clearSessionSelection"
          @selectProjectSessions="selectProjectSessions"
        />
        <FavoritesPanel
          v-else-if="activeActivityTab === 'favorites'"
          :projects="projects"
          @openSession="(identity: SessionIdentity, encodedDir: string) => openHistoryTab(identity.filePath, encodedDir, { cliId: identity.cliId })"
        />
        <BookmarkListView
          v-else-if="activeActivityTab === 'bookmarks'"
          ref="bookmarkPanelRef"
          @jumpToBookmark="onJumpToBookmark"
          @bookmarksChanged="refreshVisibleBookmarks"
        />
      </div>
      <div v-show="!sidebarCollapsed" class="divider" @mousedown="startResize" :class="{ resizing: isResizing }"></div>
      <div
        class="main-panel"
        :class="{
          'no-right-sidebar': !rightSidebarOpen,
          'no-left-sidebar': sidebarCollapsed
        }"
      >
        <TabBar
          v-if="openTabs.length > 0"
          :tabs="tabItems"
          :activeTabId="activeTabId"
          :rightSidebarOpen="rightSidebarOpen"
          @selectTab="onSelectTab"
          @closeTab="closeTab"
          @pinTab="pinPreviewTab"
          @closeOtherTabs="closeOtherTabs"
          @closeRightTabs="closeRightTabs"
          @closeAllTabs="closeAllTabs"
          @closeSavedTabs="closeSavedTabs"
          @toggleRightSidebar="toggleRightSidebar()"
          @openRenameDialog="openTabRenameDialog"
          @renameTab="renameTab"
          @newTerminalTab="() => openTerminalTab(undefined, { cliKind: 'shell', projectPath: activeProjectDir })"
          @newCliTab="(cli) => openTerminalTab(undefined, { cliKind: cli, projectPath: activeProjectDir })"
          @openNewSessionDialog="openNewSessionDialog()"
          @splitRight="(tabId) => splitTerminalTab(tabId, 'row', { cliKind: 'shell' })"
          @splitDown="(tabId) => splitTerminalTab(tabId, 'col', { cliKind: 'shell' })"
        />
        <!-- Terminal tabs: always mounted, shown/hidden to preserve xterm state -->
        <div
          v-for="tab in terminalTabs"
          :key="tab.id"
          v-show="activeTabId === tab.id && selectedSessionIdentities.length === 0"
          class="terminal-stage"
        >
          <TerminalPaneTree
            v-if="tab.rootPane"
            :node="tab.rootPane"
            :activePaneId="tab.activePaneId || firstLeafId(tab.rootPane)"
            :totalLeafCount="leafIds(tab.rootPane).length"
            :maximizedPaneId="tab.maximizedPaneId"
            @focus="(paneId) => { tab.activePaneId = paneId; }"
            @splitRight="() => splitTerminalTab(tab.id, 'row', { cliKind: 'shell' })"
            @splitDown="() => splitTerminalTab(tab.id, 'col', { cliKind: 'shell' })"
            @close="(paneId) => closeTerminalPane(tab.id, paneId)"
            @toggleMaximize="(paneId) => toggleMaximizePane(tab.id, paneId)"
            @openFile="(path, root) => openFileTab(path, root)"
            @updateRatios="(splitId, ratios) => setSplitRatiosInTab(tab, splitId, ratios)"
          />
          <AgentTerminal
            v-else-if="tab.sessionId"
            :ref="(el: any) => registerTerminalRef(tab.sessionId!, el)"
            :sessionId="tab.sessionId!"
            :projectRoot="tab.projectRoot ?? ptySessions.find(s => s.sessionId === tab.sessionId)?.projectPath"
            @openFile="(path, root) => openFileTab(path, root)"
          />
        </div>
        <!-- File tabs: always mounted, shown/hidden to preserve editor/iframe/preview state -->
        <div
          v-for="tab in fileTabs"
          :key="tab.id"
          v-show="activeTabId === tab.id && selectedSessionIdentities.length === 0"
          class="editor-with-toolbar"
        >
          <HtmlPreview
            v-if="isHtmlFile(tab.filePath!)"
            :ref="(el: any) => registerEditorRef(tab.id, el)"
            :filePath="tab.filePath!"
            :projectRoot="tab.projectRoot!"
            :isDark="isDark"
            @dirty="onFileDirty(tab.id, $event)"
            @openFile="(path, root) => openFileTab(path, root)"
          />
          <MdPreview
            v-else-if="isMdFile(tab.filePath!)"
            :ref="(el: any) => registerEditorRef(tab.id, el)"
            :filePath="tab.filePath!"
            :projectRoot="tab.projectRoot!"
            :isDark="isDark"
            @dirty="onFileDirty(tab.id, $event)"
            @openFile="(path, root) => openFileTab(path, root)"
          />
          <ImagePreview
            v-else-if="isImageFile(tab.filePath!)"
            :filePath="tab.filePath!"
            :projectRoot="tab.projectRoot!"
            @openFile="(path, root) => openFileTab(path, root)"
          />
          <MonacoEditor
            v-else
            :ref="(el: any) => registerEditorRef(tab.id, el)"
            :filePath="tab.filePath!"
            :projectRoot="tab.projectRoot!"
            :isDark="isDark"
            @dirty="onFileDirty(tab.id, $event)"
            @openFile="(path, root) => openFileTab(path, root)"
          />
        </div>
        <!-- Diff tabs: always mounted, shown/hidden -->
        <div
          v-for="tab in diffTabs"
          :key="tab.id"
          v-show="activeTabId === tab.id && selectedSessionIdentities.length === 0"
          class="editor-with-toolbar"
        >
          <MonacoDiffEditor
            :original="tab.diffOriginal ?? ''"
            :modified="tab.diffModified ?? ''"
            :filePath="tab.filePath ?? ''"
            :isDark="isDark"
          />
        </div>
        <div v-if="selectedSessionIdentities.length > 0" class="batch-toolbar">
          <div class="batch-toolbar-left">
            <span class="batch-count">{{ t('app.batch.selected', { count: selectedSessionIdentities.length }) }}</span>
            <button class="batch-select-all-btn" @click="isAllSelected ? clearSessionSelection() : selectAllSessions()">
              {{ isAllSelected ? t('app.batch.deselectAll') : t('app.batch.selectAll') }}
            </button>
          </div>
          <div class="batch-toolbar-right">
            <div class="batch-export-wrapper" ref="batchExportRef">
              <button class="batch-btn" @click="showBatchExportMenu = !showBatchExportMenu" :title="t('app.batch.exportTooltip')">
                <SvgIcon name="download" :size="13" />
                {{ t('app.batch.export') }}
                <SvgIcon name="chevron-down" :size="10" />
              </button>
              <div v-if="showBatchExportMenu" class="batch-export-menu">
                <div class="batch-export-group-label">{{ t('app.batch.mergedExport') }}</div>
                <button @click="batchExportSessions('merged', 'txt'); showBatchExportMenu = false">Text</button>
                <button @click="batchExportSessions('merged', 'markdown'); showBatchExportMenu = false">Markdown</button>
                <button @click="batchExportSessions('merged', 'json'); showBatchExportMenu = false">JSON</button>
                <button @click="batchExportSessions('merged', 'jsonl'); showBatchExportMenu = false">JSONL</button>
                <div class="batch-export-group-label">{{ t('app.batch.separateExport') }}</div>
                <button @click="batchExportSessions('separate', 'txt'); showBatchExportMenu = false">Text</button>
                <button @click="batchExportSessions('separate', 'markdown'); showBatchExportMenu = false">Markdown</button>
                <button @click="batchExportSessions('separate', 'json'); showBatchExportMenu = false">JSON</button>
                <button @click="batchExportSessions('separate', 'jsonl'); showBatchExportMenu = false">JSONL</button>
              </div>
            </div>
            <button class="batch-btn batch-btn-danger" @click="batchDeleteAndCloseTabs" :title="t('app.batch.deleteTooltip')">
              <SvgIcon name="trash-2" :size="13" />
              {{ t('app.batch.delete') }}
            </button>
            <button
              v-if="showChat"
              class="batch-close-btn icon-btn"
              :title="t('app.batch.backHome')"
              @click="clearSessionSelection(); clearActiveSession()"
            >
              <SvgIcon name="house" :size="14" />
            </button>
            <button class="batch-close-btn icon-btn" @click="clearSessionSelection" :title="t('app.batch.exitSelection')">
              <SvgIcon name="x" :size="14" />
            </button>
          </div>
        </div>
        <div v-if="deleteProgress" class="modal-backdrop">
          <div class="batch-delete-card">
            <div class="batch-delete-text">{{ batchDeleteProgressText(deleteProgress) }}</div>
            <div class="batch-delete-bar">
              <div
                class="batch-delete-bar-fill"
                :style="{ width: (deleteProgress.total ? (deleteProgress.done / deleteProgress.total) * 100 : 0) + '%' }"
              ></div>
            </div>
          </div>
        </div>
        <!-- History tabs: always mounted, shown/hidden to preserve ChatView scroll -->
        <template
          v-for="tab in historyTabs"
          :key="tab.id"
        >
          <template v-if="activeTabId === tab.id && selectedSessionIdentities.length === 0 && showHistoryChat">
            <SessionHeader
              :displayName="selectedSession?.session.display_name ?? tabLabelText(tab.label)"
              :projectPath="selectedSession?.project.original_path ?? tab.projectRoot ?? ''"
              :timestamp="selectedSession?.session.timestamp ?? ''"
              :messageCount="activeMessageCount"
              :messages="getActiveMessages"
              :stats="sessionStats"
              :bookmarks="sessionBookmarksFor(historySessionId(tab) ?? undefined, cliIdOfTab(tab))"
              :timelineOpen="rightSidebarOpen && rightSidebarMode === 'timeline'"
              :archivePinned="sessionArchivePinned"
              :archivePinLoading="sessionArchivePinLoading"
              :hasParentSession="!!tab.parentSessionPath"
              :isArchived="selectedSession?.session.is_archived"
              :hasArchiveSnapshot="selectedSession?.session.has_archive_snapshot"
              :profiles="claudeProfiles"
              :cliId="cliIdOfTab(tab)"
              :isRefreshing="isCurrentSessionRefreshing"
              :favorite="isFavorite({ cliId: cliIdOfTab(tab), filePath: tab.sessionPath! })"
              :proxyTrafficCount="activeProxyTrafficCount"
              @export="(fmt: string) => exportActiveSession(fmt as 'txt' | 'markdown' | 'json' | 'jsonl')"
              @resume="(profileName: string | null, skip: boolean) => { const ctx = resumeContextOfTab(tab); ctx && resumeSessionInAgent(ctx.projectPath, ctx.sessionId, cliIdOfTab(tab), activeTabId, profileName, skip); }"
              @resumeInTerminal="(profileName: string | null, skip: boolean) => { const ctx = resumeContextOfTab(tab); ctx && resumeSession(ctx.projectPath, ctx.sessionId, cliIdOfTab(tab), profileName, skip); }"
              @copyRestoreCommand="(profileName: string | null, skip: boolean, target: any) => { const ctx = resumeContextOfTab(tab); ctx && copyRestoreCommand(ctx.projectPath, ctx.sessionId, cliIdOfTab(tab), target, profileName, skip); }"
              @openProject="() => openRightSidebar('project')"
              @scrollToBookmark="onScrollToBookmark"
              @toggleArchivePinned="toggleSessionArchivePinned(tab.sessionPath)"
              @update:timelineOpen="toggleTimelineSidebar"
              @enterSelectionMode="activeHistoryChatView()?.enterSelectionMode()"
              @backToParent="handleBackToParent(tab)"
              @refresh="refreshActiveSession"
              @toggleFavorite="toggleFavorite({ cliId: cliIdOfTab(tab), filePath: tab.sessionPath! })"
              @openProxyTraffic="openApiDebug(cliIdOfTab(tab), historySessionId(tab) ?? undefined)"
            />
            <div class="session-toolbar">
              <div :id="`session-toolbar-search-${sanitizeDomId(sessionIdentityKey({ cliId: cliIdOfTab(tab), filePath: tab.sessionPath! }))}`" class="session-toolbar-search" />
              <div class="message-filter-segment" :aria-label="t('app.filter.ariaLabel')">
                <span class="filter-count" :title="t('app.filter.count', { count: filteredMessageCount })">{{ filteredMessageCount }}</span>
                <button class="filter-segment-btn" :class="{ active: messageFilterUser }" @click="messageFilterUser = !messageFilterUser">{{ t('app.filter.user') }}</button>
                <button class="filter-segment-btn" :class="{ active: messageFilterAssistant }" @click="messageFilterAssistant = !messageFilterAssistant">{{ t('app.filter.assistant') }}</button>
                <button class="filter-segment-btn" :class="{ active: messageFilterThinking }" @click="messageFilterThinking = !messageFilterThinking">{{ t('app.filter.thinking') }}</button>
                <button class="filter-segment-btn" :class="{ active: messageFilterTool }" @click="messageFilterTool = !messageFilterTool">{{ t('common.sessionAnalytics.toolCalls') }}</button>
              </div>
            </div>
          </template>
          <div
            v-show="activeTabId === tab.id && selectedSessionIdentities.length === 0 && showHistoryChat"
            class="session-body-shell"
          >
            <ChatView
              :ref="(el: any) => registerHistoryChatViewRef({ cliId: cliIdOfTab(tab), filePath: tab.sessionPath! }, el)"
              :sessionPath="tab.sessionPath!"
              :encodedDir="tab.encodedDir!"
              :cliId="cliIdOfTab(tab)"
              :active="activeTabId === tab.id"
              :sessionId="historySessionId(tab) ?? ''"
              :projectRoot="tab.projectRoot"
              :showSearch="showChatSearch"
              :searchHostId="`session-toolbar-search-${sanitizeDomId(sessionIdentityKey({ cliId: cliIdOfTab(tab), filePath: tab.sessionPath! }))}`"
              :bookmarks="bookmarks"
              :filterUser="messageFilterUser"
              :filterAssistant="messageFilterAssistant"
              :filterTool="messageFilterTool"
              :filterThinking="messageFilterThinking"
              :isSubagent="!!tab.parentSessionPath"
              @toggleBookmark="onToggleBookmark"
              @forkFromHere="(uuid: string) => onForkFromHere(tab, uuid)"
              @active-message-change="activeTimelineMessageIndex = $event"
              @openFile="(path: string, projectRoot: string) => openFileTab(path, projectRoot)"
              @openSubagent="handleOpenSubagent"
              @proxyTrafficCountChange="activeProxyTrafficCount = $event"
            />
          </div>
        </template>
        <!-- Dashboard：零工作 Tab 时的根页面 -->
        <div
          v-if="shouldShowDashboard"
          class="dashboard-stage"
        >
          <DashboardView
            ref="dashboardRef"
            :bootstrapping="bootstrapping"
            :dataSourcePath="currentCli.dataSourcePath"
            :showUsage="hasUsageCli"
            :showApiDebug="hasApiLogCli"
            :showNewSession="canLaunchNewSession"
            :recentConversations="recentConversations"
            :activeAgentCount="activeAgentCount"
            :waitingAgentCount="waitingInputCount"
            :searchQuery="dashboardSearchQuery"
            :searchResults="globalSearchResults"
            :searchLoading="globalSearchLoading"
            :searchPendingCliIds="searchPendingCliIds"
            :searchStaleCliIds="searchStaleCliIds"
            :searchCliErrors="searchCliErrors"
            :buildingCliIds="buildingCliIds"
            @submitAssistant="onDashboardAskAssistant"
            @submitSearch="onDashboardSearch"
            @openSearchResult="onDashboardSearchResult"
            @openFullSearch="openGlobalSearch"
            @clearSearch="onDashboardClearSearch"
            @buildPendingSearchIndex="buildPendingSearchIndex"
            @buildAllPendingSearchIndexes="buildAllPendingSearchIndexes"
            @openSession="(identity: SessionIdentity, encodedDir: string) => openHistoryTab(identity.filePath, encodedDir, { cliId: identity.cliId })"
            @newSession="openNewSessionDialog()"
            @openHistory="openHistoryFromDashboard"
            @openAgent="openAgentFromDashboard"
            @openAssistant="openAssistant"
            @openUsage="openUsageDashboard"
            @openApiDebug="openApiDebug"
          />
        </div>

        <!-- AgentDashboard tab -->
        <div
          v-if="openTabs.some((t) => t.id === AGENT_DASHBOARD_TAB_ID)"
          v-show="activeTabId === AGENT_DASHBOARD_TAB_ID"
          class="agent-dashboard-stage"
        >
          <AgentDashboardView
            :bootstrapping="bootstrapping"
            :showNewSession="canLaunchNewSession"
            @newSession="openNewSessionDialog()"
            @selectTerminal="openTerminalTab"
          />
        </div>
      </div>
      <div
        v-if="rightSidebarOpen"
        class="divider right-divider"
        @mousedown="startRightResize"
        :class="{ resizing: isRightResizing }"
      ></div>
      <aside
        v-if="rightSidebarOpen"
        class="right-sidebar"
        :style="{ width: rightSidebarWidth + 'px' }"
        :aria-label="t('app.sidebar.ariaLabel')"
      >
        <div class="right-sidebar-header">
          <div class="right-sidebar-tabs" role="radiogroup" :aria-label="t('app.sidebar.modeAriaLabel')">
            <button
              type="button"
              role="radio"
              class="right-sidebar-tab-btn"
              :class="{ active: rightSidebarMode === 'project' }"
              :aria-checked="rightSidebarMode === 'project'"
              :title="t('app.sidebar.project')"
              :aria-label="t('app.sidebar.project')"
              @click="setRightSidebarMode('project')"
            >
              <SvgIcon name="folder" :size="16" />
            </button>
            <button
              type="button"
              role="radio"
              class="right-sidebar-tab-btn"
              :class="{ active: rightSidebarMode === 'git' }"
              :aria-checked="rightSidebarMode === 'git'"
              title="Git"
              aria-label="Git"
              @click="setRightSidebarMode('git')"
            >
              <SvgIcon name="git-branch" :size="16" />
            </button>
            <button
              type="button"
              role="radio"
              class="right-sidebar-tab-btn"
              :class="{ active: rightSidebarMode === 'timeline' }"
              :aria-checked="rightSidebarMode === 'timeline'"
              :title="t('app.sidebar.timeline')"
              :aria-label="t('app.sidebar.timeline')"
              @click="setRightSidebarMode('timeline')"
            >
              <SvgIcon name="clock" :size="16" />
            </button>
          </div>
          <div class="right-sidebar-actions">
            <button
              type="button"
              class="right-sidebar-close-btn"
              :title="t('app.sidebar.collapse')"
              :aria-label="t('app.sidebar.collapse')"
              @click="closeRightSidebar"
            >
              <SvgIcon name="panel-right-close" :size="16" />
            </button>
          </div>
        </div>
        <div class="right-sidebar-body">
          <ProjectPanel
            v-if="rightSidebarMode === 'project'"
            :projects="projects"
            :manualPaths="manualPaths"
            :activeProjectPath="activeHistoryProjectRoot"
            :activeFilePath="activeOpenTab?.filePath ?? null"
            @openFile="(path: string, projectRoot: string) => openFileTab(path, projectRoot)"
            @pinFile="(path: string, projectRoot: string) => openFileTab(path, projectRoot, { pinned: true })"
            @newSession="openNewSessionDialog($event)"
          />
          <GitPanel
            v-else-if="rightSidebarMode === 'git'"
            @openDiff="openDiffTab"
            @openCommitDiff="openCommitDiffTab"
          />
          <template v-else-if="rightSidebarMode === 'timeline'">
            <TimelineView
              v-if="activeOpenTab?.type === 'history'"
              :messages="getActiveMessages()"
              :loading="activeLoading"
              :activeMessageIndex="activeTimelineMessageIndex"
              :subagentMap="getActiveSubagentMap()"
              @jumpToMessage="(idx: number) => onScrollToBookmark(idx)"
            />
            <div v-else class="right-sidebar-empty-state">
              <SvgIcon name="clock" :size="32" class="empty-icon" />
              <p class="empty-text">{{ t('app.sidebar.timelineEmpty') }}</p>
            </div>
          </template>
        </div>
      </aside>
    </div>
    <StatusBar
      :projectCount="projects.length"
      :sessionCount="totalSessionCount"
      :messageCount="statusBarMessageCount"
      :cliId="currentCli.id"
      :rightSidebarOpen="rightSidebarOpen"
      :refreshing="isCurrentSessionRefreshing"
      :loading="bootstrapping || (!hasCompletedInitialScan && hasDetectedCliWithSessions && projects.length === 0)"
      @toggleRightSidebar="toggleRightSidebar()"
      @refreshSession="refreshActiveSession"
    />
    <ContextMenu
      :visible="ctxMenu.visible"
      :x="ctxMenu.x"
      :y="ctxMenu.y"
      :items="ctxMenu.items"
      @close="closeContextMenu"
    />
    <GlobalSearch
      v-if="showGlobalSearch"
      ref="globalSearchRef"
      :initialQuery="globalSearchInitialQuery"
      @close="closeGlobalSearch"
      @selectSession="onGlobalSearchSelect"
    />
    <SettingsView
      v-if="showSettings"
      ref="settingsDialogRef"
      :initial-tab="settingsInitialTab"
      :initial-cli-id="settingsInitialCliId"
      @close="showSettings = false"
      @registerMenu="registerContextMenu"
      @unregisterMenu="unregisterContextMenu"
      @openSession="onOpenSessionFromSettings"
      @openSearch="onOpenSearchFromSettings"
      @openFeedback="openFeedback"
    />
    <UsageDashboard
      v-if="showUsage"
      :initial-cli-id="usageInitialCliId"
      @close="showUsage = false"
    />
    <ApiDebugDialog
      v-if="showApiDebug"
      ref="apiDebugDialogRef"
      :initial-cli-id="apiDebugInitialCliId"
      :initial-session-id="apiDebugInitialSessionId"
      @close="showApiDebug = false"
    />
    <InputDialog
      :open="renameDialog.open"
      :title="renameDialog.title"
      :description="renameDialog.description"
      :default-value="renameDialog.currentName"
      :placeholder="t('app.dialog.renamePlaceholder')"
      :confirm-label="t('app.dialog.save')"
      @confirm="handleConfirmRename"
      @cancel="renameDialog.open = false"
    />
    <WhatsNewDialog
      v-if="whatsNewEntry && startupMode === 'ready'"
      :entry="whatsNewEntry"
      @close="closeWhatsNew"
    />
    <FeedbackDialog
      v-if="showFeedback"
      @close="showFeedback = false"
    />
    <NewSessionDialog
      v-if="showNewSessionDialog"
      :initialProjectPath="newSessionInitialPath"
      :initialCliId="launchCliId"
      :cliBinaryStatuses="cliBinaryStatuses"
      @close="showNewSessionDialog = false"
      @create="onCreateNewSession"
    />
    <ResumeSessionDialog
      v-if="showContextMenuResumeDialog && contextMenuTargetSession"
      :profiles="claudeProfiles"
      :cliId="contextMenuTargetSession.cliId"
      :isArchived="contextMenuTargetSessionIsArchived"
      @close="showContextMenuResumeDialog = false"
      @resume="(launchMode, profileName, skip) => {
        showContextMenuResumeDialog = false;
        if (launchMode === 'agent') {
          resumeSessionInAgent(contextMenuTargetSession!.projectPath, contextMenuTargetSession!.sessionId, contextMenuTargetSession!.cliId, activeTabId, profileName, skip);
        } else {
          resumeSession(contextMenuTargetSession!.projectPath, contextMenuTargetSession!.sessionId, contextMenuTargetSession!.cliId, profileName, skip);
        }
      }"
      @copyCommand="(profileName, skip, cb) => {
        copyRestoreCommand(contextMenuTargetSession!.projectPath, contextMenuTargetSession!.sessionId, contextMenuTargetSession!.cliId, cb, profileName, skip);
      }"
    />
    <ResumeSessionDialog
      v-if="showForkDialog && forkTarget"
      :profiles="claudeProfiles"
      :cliId="forkTarget.cliId"
      :title="t('app.dialog.forkSession')"
      @close="showForkDialog = false"
      @resume="async (launchMode, profileName, skip) => {
        showForkDialog = false;
        const ctx = forkTarget!;
        const newId = await createForkFromTarget();
        if (!newId) return;
        if (launchMode === 'agent') {
          resumeSessionInAgent(ctx.projectPath, newId, ctx.cliId, activeTabId, profileName, skip);
        } else {
          resumeSession(ctx.projectPath, newId, ctx.cliId, profileName, skip);
        }
      }"
      @copyCommand="async (profileName, skip, cb) => {
        const ctx = forkTarget!;
        const newId = await createForkFromTarget();
        if (!newId) { cb?.(false); return; }
        copyRestoreCommand(ctx.projectPath, newId, ctx.cliId, cb, profileName, skip);
      }"
    />
    <ExportSessionDialog
      v-if="showContextMenuExportDialog && contextMenuTargetSession"
      @close="showContextMenuExportDialog = false"
      @export="(format) => {
        showContextMenuExportDialog = false;
        exportSession(
          { cliId: contextMenuTargetSession!.cliId, filePath: contextMenuTargetSession!.filePath },
          format as any,
          contextMenuTargetSession!.projectPath,
          contextMenuTargetSession!.sessionId,
        );
      }"
      @enterSelectionMode="() => {
        showContextMenuExportDialog = false;
        onOpenSessionFromSettings({
          cliId: contextMenuTargetSession!.cliId,
          filePath: contextMenuTargetSession!.filePath,
        });
        nextTick(() => {
          activeHistoryChatView()?.enterSelectionMode();
        });
      }"
    />

    <Transition name="update-toast-fade">
      <div v-if="showCrashRecoveryToast" class="global-update-toast crash-toast">
        <SvgIcon name="zap" :size="14" />
        <span>{{ t('app.toast.crashRecovery') }}</span>
      </div>
    </Transition>

    <Transition name="update-toast-fade">
      <div v-if="showGlobalUpdateToast" class="global-update-toast" @click="onGlobalUpdateToastClick">
        <SvgIcon name="zap" :size="14" />
        <span>{{ updateToastMessage }}</span>
      </div>
    </Transition>

  </div>
</template>

<style scoped>
.app-layout {
  position: relative;
  display: flex;
  flex-direction: column;
  height: 100%;
  width: 100%;
  overflow: hidden;
}

.minimized-widgets-container {
  position: fixed;
  bottom: 32px;
  right: 32px;
  display: flex;
  flex-direction: column-reverse;
  align-items: flex-end;
  gap: 16px;
  z-index: 1000;
  pointer-events: none;
}
.app-layout.is-startup-active > :not(.startup-splash) {
  opacity: 0;
  visibility: hidden;
  pointer-events: none;
}
.content-area {
  display: flex;
  flex: 1;
  min-height: 0;
  background: var(--color-bg);
}
.sidebar {
  display: flex;
  flex-direction: column;
  min-width: 180px;
  max-width: 600px;
  background: var(--color-bg-sidebar);
  position: relative;
  margin: 6px 4px 6px 0;
  border: 1px solid var(--color-border);
  border-radius: var(--radius-xl);
  box-shadow: var(--shadow-sm);
  overflow: hidden;
  user-select: none;
  -webkit-user-select: none;
}
.divider {
  width: 4px;
  margin: 0;
  position: relative;
  z-index: 10;
  cursor: col-resize;
  background: transparent;
  flex-shrink: 0;
  transition: background var(--transition-fast);
}
.divider::before {
  content: "";
  position: absolute;
  top: 0;
  bottom: 0;
  left: -2px;
  right: -2px;
  z-index: 1;
}
.divider:hover,
.divider.resizing {
  background: var(--color-primary);
  border-radius: var(--radius-full);
}
.main-panel {
  flex: 1;
  display: flex;
  flex-direction: column;
  min-width: 200px;
  margin: 6px 3px;
  border: 1px solid var(--color-border);
  border-radius: var(--radius-xl);
  box-shadow: var(--shadow-sm);
  overflow: hidden;
  background: var(--color-bg);
}
.main-panel.no-right-sidebar {
  margin-right: 6px;
}
.main-panel.no-left-sidebar {
  margin-left: 2px;
}
.terminal-stage,
.terminal-workspace-stage {
  flex: 1;
  display: flex;
  flex-direction: column;
  min-height: 0;
  overflow: hidden;
}
.session-toolbar {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  min-height: 44px;
  padding: 6px var(--space-4);
  border-bottom: 1px solid var(--color-border-light);
  background: var(--color-bg);
  flex-shrink: 0;
}
.session-toolbar-search {
  flex: 1;
  min-width: 180px;
}
.message-filter-segment {
  display: inline-flex;
  align-items: center;
  flex-shrink: 0;
  height: 28px;
  padding: 2px;
  border: 1px solid var(--color-border-light);
  border-radius: var(--radius-md);
  background: var(--color-bg-secondary);
}
.filter-count {
  min-width: 34px;
  padding: 0 8px;
  font-family: var(--font-mono);
  font-size: var(--text-xs);
  color: var(--color-text-muted);
  text-align: center;
}
.filter-segment-btn {
  height: 22px;
  padding: 0 9px;
  font-size: var(--text-xs);
  font-weight: 500;
  color: var(--color-text-muted);
  background: transparent;
  border: 0;
  border-radius: calc(var(--radius-sm) - 1px);
  cursor: pointer;
  transition: background var(--transition-fast), color var(--transition-fast), box-shadow var(--transition-fast);
}
.filter-segment-btn:hover {
  color: var(--color-text);
}
.filter-segment-btn.active {
  color: var(--color-primary);
  background: var(--color-bg);
  box-shadow: var(--shadow-sm);
}
@media (max-width: 900px) {
  .session-toolbar {
    flex-wrap: wrap;
    gap: var(--space-2);
  }
  .session-toolbar-search {
    flex-basis: 100%;
  }
}
.session-body-shell {
  flex: 1;
  display: flex;
  min-height: 0;
  overflow: hidden;
}
.session-body-shell > :first-child {
  min-width: 0;
}
.right-sidebar {
  display: flex;
  flex-direction: column;
  min-width: 200px;
  max-width: 600px;
  background: var(--color-bg-sidebar);
  position: relative;
  margin: 6px 6px 6px 4px;
  border: 1px solid var(--color-border);
  border-radius: var(--radius-xl);
  box-shadow: var(--shadow-sm);
  overflow: hidden;
}
.right-sidebar-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  height: 38px;
  min-height: 38px;
  padding: 0 10px;
  border-bottom: 1px solid var(--color-border);
  background: var(--color-bg-sidebar);
  user-select: none;
  -webkit-user-select: none;
  flex-shrink: 0;
}
.right-sidebar-tabs {
  display: flex;
  align-items: center;
  gap: 4px;
}
.right-sidebar-tab-btn {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 26px;
  height: 26px;
  border-radius: var(--radius-md, 6px);
  border: 1px solid transparent;
  background: transparent;
  color: var(--color-text-muted);
  cursor: pointer;
  transition: all var(--transition-fast);
}
.right-sidebar-tab-btn:hover {
  color: var(--color-text);
  background: var(--color-surface-selected);
}
.right-sidebar-tab-btn.active {
  color: var(--color-primary);
  background: var(--color-surface);
  border-color: var(--color-border);
  box-shadow: var(--shadow-sm);
}
.right-sidebar-actions {
  display: flex;
  align-items: center;
}
.right-sidebar-close-btn {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 26px;
  height: 26px;
  border-radius: var(--radius-md, 6px);
  border: none;
  background: transparent;
  color: var(--color-text-muted);
  cursor: pointer;
  transition: all var(--transition-fast);
}
.right-sidebar-close-btn:hover {
  color: var(--color-text);
  background: var(--color-surface-selected);
}
.right-sidebar-body {
  display: flex;
  flex-direction: column;
  flex: 1;
  min-height: 0;
  overflow: hidden;
}
.right-sidebar-empty-state {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  flex: 1;
  padding: var(--space-6);
  text-align: center;
  color: var(--color-text-muted);
  gap: var(--space-3);
  user-select: none;
}
.right-sidebar-empty-state .empty-icon {
  opacity: 0.4;
}
.right-sidebar-empty-state .empty-text {
  font-size: var(--text-sm);
  margin: 0;
}
.main-panel-swap-enter-active,
.main-panel-swap-leave-active {
  transition: opacity 180ms ease, transform 220ms ease;
}
.main-panel-swap-enter-from,
.main-panel-swap-leave-to {
  opacity: 0;
  transform: translateY(8px);
}
.overlay-fade-enter-active,
.overlay-fade-leave-active {
  transition: opacity 180ms ease;
}
.overlay-fade-enter-from,
.overlay-fade-leave-to {
  opacity: 0;
}
.batch-toolbar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 0 16px;
  height: 42px;
  min-height: 42px;
  background: var(--color-primary-light);
  border-bottom: 1px solid var(--color-border-light);
  flex-shrink: 0;
}
.batch-toolbar-left {
  display: flex;
  align-items: center;
  gap: 10px;
}
.batch-toolbar-right {
  display: flex;
  align-items: center;
  gap: 6px;
}
.batch-count {
  font-size: var(--text-sm);
  font-weight: 600;
  color: var(--color-primary);
}
.batch-select-all-btn {
  padding: 2px 8px;
  font-size: var(--text-xs);
  font-weight: 500;
  color: var(--color-primary);
  background: transparent;
  border: 1px solid var(--color-primary);
  border-radius: var(--radius-sm);
  cursor: pointer;
  transition: all var(--transition-fast);
}
.batch-select-all-btn:hover {
  background: var(--color-primary);
  color: white;
}
.batch-btn {
  display: flex;
  align-items: center;
  gap: 4px;
  padding: 4px 10px;
  font-size: var(--text-xs);
  font-weight: 500;
  border-radius: var(--radius-sm);
  cursor: pointer;
  color: var(--color-text);
  background: var(--color-bg-hover);
  border: 1px solid var(--color-border-light);
  transition: all var(--transition-fast);
}
.batch-btn:hover {
  background: var(--color-bg-active);
}
.batch-btn-danger {
  color: var(--color-danger);
}
.batch-btn-danger:hover {
  background: var(--color-danger);
  color: white;
}
.batch-delete-card {
  width: 280px;
  padding: var(--space-4);
  border-radius: var(--radius-lg);
  background: var(--color-bg);
  box-shadow: var(--shadow-lg);
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
}
.batch-delete-text {
  font-size: var(--text-sm);
  color: var(--color-text);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.batch-delete-bar {
  height: 6px;
  border-radius: 3px;
  background: var(--color-bg-hover);
  overflow: hidden;
}
.batch-delete-bar-fill {
  height: 100%;
  border-radius: 3px;
  background: var(--color-primary);
  transition: width 0.15s ease;
}
.batch-close-btn {
  width: 26px;
  height: 26px;
  border-radius: var(--radius-sm);
}
.batch-export-wrapper {
  position: relative;
}
.batch-export-menu {
  position: absolute;
  top: 100%;
  right: 0;
  margin-top: 4px;
  min-width: 150px;
  padding: 4px;
  background: var(--color-bg-elevated);
  border: 1px solid var(--color-border-light);
  border-radius: var(--radius-md);
  box-shadow: 0 4px 12px rgba(0, 0, 0, 0.15);
  z-index: 20;
  display: flex;
  flex-direction: column;
}
.batch-export-menu button {
  display: block;
  width: 100%;
  text-align: left;
  padding: 5px 10px;
  font-size: var(--text-xs);
  color: var(--color-text);
  border-radius: var(--radius-sm);
  cursor: pointer;
  transition: background var(--transition-fast);
}
.batch-export-menu button:hover {
  background: var(--color-bg-hover);
}
.batch-export-group-label {
  padding: 4px 10px 2px;
  font-size: 10px;
  font-weight: 600;
  color: var(--color-text-muted);
  text-transform: uppercase;
  letter-spacing: 0.5px;
}
.startup-splash {
  position: absolute;
  inset: 0;
  z-index: calc(var(--z-overlay) + 10);
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  padding: clamp(24px, 5vw, 56px);
  background:
    radial-gradient(circle at 18% 18%, color-mix(in srgb, var(--color-primary) 18%, transparent), transparent 32%),
    radial-gradient(circle at 82% 14%, color-mix(in srgb, var(--color-info) 16%, transparent), transparent 28%),
    linear-gradient(160deg, color-mix(in srgb, var(--color-bg) 90%, #dbeafe 10%), var(--color-bg));
  backdrop-filter: blur(22px) saturate(1.08);
  overflow: hidden;
}
.startup-splash.is-slow {
  background:
    radial-gradient(circle at 18% 18%, color-mix(in srgb, var(--color-warning) 18%, transparent), transparent 34%),
    radial-gradient(circle at 82% 14%, color-mix(in srgb, var(--color-primary) 16%, transparent), transparent 30%),
    linear-gradient(160deg, color-mix(in srgb, var(--color-bg) 88%, #f8fafc 12%), var(--color-bg));
}
.startup-noise {
  position: absolute;
  inset: -20%;
  background-image:
    linear-gradient(rgba(148, 163, 184, 0.08) 1px, transparent 1px),
    linear-gradient(90deg, rgba(148, 163, 184, 0.08) 1px, transparent 1px);
  background-size: 28px 28px;
  mask-image: radial-gradient(circle at center, black 42%, transparent 88%);
  opacity: 0.45;
  transform: perspective(800px) rotateX(65deg) scale(1.5);
}
.startup-ambient-backdrop {
  position: absolute;
  inset: 0;
  background:
    radial-gradient(circle at 50% 45%, color-mix(in srgb, var(--color-primary) 15%, transparent) 0%, transparent 60%),
    radial-gradient(circle at 50% 55%, color-mix(in srgb, #6366f1 12%, transparent) 0%, transparent 65%);
  pointer-events: none;
}
.startup-center-stage {
  position: relative;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  z-index: 1;
}

.startup-beacon-tile {
  position: relative;
  width: 92px;
  height: 92px;
  display: flex;
  align-items: center;
  justify-content: center;
  animation: beacon-float 3.6s ease-in-out infinite;
}
.startup-beacon-glow {
  position: absolute;
  inset: -14px;
  border-radius: 38px;
  background: radial-gradient(circle, color-mix(in srgb, #f3786c 24%, transparent) 0%, transparent 70%);
  pointer-events: none;
  animation: beacon-ambient-glow 3.6s ease-in-out infinite;
}
.startup-beacon-image {
  width: 100%;
  height: 100%;
  display: block;
  overflow: visible;
  filter: drop-shadow(0 14px 28px rgba(243, 120, 108, 0.24));
}

/* Brand Typography Lockup */
.startup-brand-lockup {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  text-align: center;
  margin-top: 24px;
  z-index: 2;
}
.startup-brand-title {
  font-size: clamp(28px, 4.8vw, 36px);
  font-weight: 800;
  letter-spacing: -0.035em;
  line-height: 1.1;
  display: flex;
  align-items: center;
  gap: 1px;
  animation: startup-title-fade 0.7s cubic-bezier(0.16, 1, 0.3, 1) backwards;
  animation-delay: 0.12s;
}
.brand-sesh {
  background: linear-gradient(135deg, #38bdf8 0%, #6366f1 100%);
  -webkit-background-clip: text;
  background-clip: text;
  -webkit-text-fill-color: transparent;
  filter: drop-shadow(0 2px 12px rgba(56, 189, 248, 0.32));
}
.brand-buddy {
  background: linear-gradient(135deg, #f8fafc 0%, #cbd5e1 100%);
  -webkit-background-clip: text;
  background-clip: text;
  -webkit-text-fill-color: transparent;
}
:root[data-theme="light"] .brand-buddy {
  background: linear-gradient(135deg, #0f172a 0%, #334155 100%);
  -webkit-background-clip: text;
  background-clip: text;
  -webkit-text-fill-color: transparent;
}
.startup-brand-subtitle {
  font-size: 10px;
  font-weight: 600;
  letter-spacing: 0.22em;
  text-transform: uppercase;
  color: var(--color-text-muted);
  margin-top: 7px;
  opacity: 0.75;
  animation: startup-subtitle-fade 0.7s cubic-bezier(0.16, 1, 0.3, 1) backwards;
  animation-delay: 0.22s;
}

/* Minimalist Hairline Progress Bar */
.startup-progress-track {
  position: relative;
  width: 140px;
  height: 2px;
  border-radius: 999px;
  background: color-mix(in srgb, var(--color-border) 45%, transparent);
  margin-top: 22px;
  overflow: hidden;
  animation: startup-track-fade 0.7s cubic-bezier(0.16, 1, 0.3, 1) backwards;
  animation-delay: 0.3s;
}
.startup-progress-line {
  position: absolute;
  top: 0;
  left: 0;
  width: 48px;
  height: 100%;
  border-radius: 999px;
  background: linear-gradient(90deg, transparent, #38bdf8, #818cf8, transparent);
  box-shadow: 0 0 10px rgba(56, 189, 248, 0.8);
  animation: startup-line-glide 1.6s cubic-bezier(0.4, 0, 0.2, 1) infinite;
}

.startup-status-wrap {
  margin-top: 10px;
  display: flex;
  align-items: center;
  justify-content: center;
  min-height: 18px;
  animation: startup-status-fade 0.7s cubic-bezier(0.16, 1, 0.3, 1) backwards;
  animation-delay: 0.36s;
}
.startup-status-text {
  font-size: 11px;
  font-weight: 500;
  color: var(--color-text-muted);
  letter-spacing: 0.02em;
  opacity: 0.88;
  transition: opacity 0.2s ease;
}

/* Transitions & Animations */
.startup-overlay-enter-active,
.startup-overlay-leave-active {
  transition: opacity 450ms cubic-bezier(0.16, 1, 0.3, 1), transform 450ms cubic-bezier(0.16, 1, 0.3, 1), filter 450ms cubic-bezier(0.16, 1, 0.3, 1);
}
.startup-overlay-enter-from {
  opacity: 0;
  transform: scale(0.96);
}
.startup-overlay-leave-to {
  opacity: 0;
  transform: scale(1.03);
  filter: blur(8px);
}

@keyframes beacon-float {
  0%, 100% { transform: translateY(0); }
  50% { transform: translateY(-4px); }
}
@keyframes beacon-ambient-glow {
  0%, 100% { opacity: 0.6; transform: scale(0.95); }
  50% { opacity: 1; transform: scale(1.08); }
}
@keyframes startup-title-fade {
  from { opacity: 0; transform: translateY(8px) scale(0.97); }
  to { opacity: 1; transform: translateY(0) scale(1); }
}
@keyframes startup-subtitle-fade {
  from { opacity: 0; transform: translateY(6px); }
  to { opacity: 0.75; transform: translateY(0); }
}
@keyframes startup-track-fade {
  from { opacity: 0; transform: scaleX(0.8); }
  to { opacity: 1; transform: scaleX(1); }
}
@keyframes startup-line-glide {
  0% { transform: translateX(-140%); }
  100% { transform: translateX(280%); }
}
@keyframes startup-status-fade {
  from { opacity: 0; transform: translateY(4px); }
  to { opacity: 0.88; transform: translateY(0); }
}
@media (max-width: 720px) {
  .startup-beacon-tile {
    width: 76px;
    height: 76px;
    border-radius: 22px;
  }
  .startup-brand-title {
    font-size: 26px;
  }
}
.global-update-toast {
  position: fixed;
  top: 24px;
  left: 50%;
  transform: translateX(-50%);
  background: var(--color-primary);
  color: #fff;
  padding: 8px 16px;
  border-radius: 20px;
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: var(--text-sm);
  font-weight: 500;
  box-shadow: 0 4px 12px var(--color-primary-ring);
  cursor: pointer;
  z-index: 9999;
  transition: all 0.2s ease;
}

.global-update-toast:hover {
  transform: translateX(-50%) translateY(-2px);
  box-shadow: 0 6px 16px color-mix(in srgb, var(--color-primary) 40%, transparent);
}

.update-toast-fade-enter-active,
.update-toast-fade-leave-active {
  transition: opacity 0.3s ease, transform 0.3s cubic-bezier(0.175, 0.885, 0.32, 1.275);
}
.update-toast-fade-enter-from,
.update-toast-fade-leave-to {
  opacity: 0;
  transform: translate(-50%, -20px);
}

.crash-toast {
  background: linear-gradient(135deg, #fffbeb 0%, #fef3c7 100%) !important;
  color: #b45309 !important;
  border: 1px solid #fde68a !important;
  border-radius: 10px !important;
  padding: 10px 18px !important;
  box-shadow: 0 8px 24px rgba(217, 119, 6, 0.12) !important;
  backdrop-filter: none !important;
  -webkit-backdrop-filter: none !important;
}
.crash-toast svg {
  color: #d97706 !important;
}

@media (prefers-color-scheme: dark) {
  .crash-toast {
    background: linear-gradient(135deg, rgba(245, 158, 11, 0.2), rgba(245, 158, 11, 0.05)) !important;
    color: #fcd34d !important;
    border: 1px solid rgba(245, 158, 11, 0.3) !important;
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.2) !important;
  }
  .crash-toast svg {
    color: #fbbf24 !important;
  }
}

/* ─── Editor / history wrapper ─────────────────────────────────────────────────────── */
.editor-with-toolbar {
  display: flex;
  flex-direction: column;
  flex: 1;
  min-height: 0;
  overflow: hidden;
}

.dashboard-stage,
.agent-dashboard-stage {
  flex: 1;
  display: flex;
  flex-direction: column;
  min-height: 0;
  overflow: hidden;
}
</style>

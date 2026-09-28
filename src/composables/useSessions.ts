import { ref, computed, watch, triggerRef } from "vue";
import { t } from "../i18n";
import { invokeApp, renderAppError, type AppErrorPayload } from "../utils/invokeApp";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { ask, message, open, save } from "@tauri-apps/plugin-dialog";
import { useBlockedFolders } from "./useBlockedFolders";
import { useTerminalApp } from "./useTerminalApp";
import { batchDeleteProgressText, type BatchDeleteProgress } from "./batchDeleteProgress";
import { useStreamingCollection } from "./useStreamingCollection";
import { sessionMatchesQuery } from "../utils/sessionFilter";
import { formatDialogSessionTitle } from "../utils/textClean";
import type {
  AggregatedProjectInfo,
  ExportOutcome,
  ProjectInfo,
  SessionIdentity,
  SessionInfo,
  SearchResult,
} from "../types/session";
import { sessionIdentityKey } from "../types/session";
import {
  resolveVisibleCliIds,
  type CliFilterState,
} from "./cliFilter";
import {
  IS_WINDOWS,
  SUPPORTED_CLIS,
  cliInstallHint,
  getCliDefinition,
  isCliId,
  resolveCliDefinition,
  type CliId,
  type CliOption,
  type CliPathConfig,
  type CliRuntime,
  type CliStatus,
  type ContextMenuOutcome,
  type ResolvedCliDefinition,
} from "../types/cli";

/**
 * 系统右键菜单的措辞按平台分叉：macOS 是 Finder 的「快速操作/服务」，Windows 是资源
 * 管理器的右键菜单。平台由前端判断，后端只回「注册了 / 注销了」这个动作。
 * 判据复用 `types/cli.ts` 导出的那一份——两处各自声明会让界面说错平台而无从发现。
 */

/**
 * 导出结果 → 语言包键。`Record` 的键集来自 `ExportOutcome["kind"]`，
 * 后端新增变体而这里漏配时是**编译错误**，不是运行时的 undefined。
 *
 * ⚠️ 导出是为了让测试能遍历**取值**：闸门规则 5 只认直接的 `t("…")` 字面量、
 * 规则 6 只扫 `src/changelog.ts` 的 `*Keys` 数组，**两份静态检查都看不见 `Record` 的取值**，
 * 而 vue-i18n 缺键时原样返回键名（`i18n/index.ts` 的 `missingWarn: false`）——
 * 一个拼错的键会安静地上屏成 `app.batch.exportOutcome.singel`。
 */
export const EXPORT_OUTCOME_KEY: Record<ExportOutcome["kind"], string> = {
  single: "app.batch.exportOutcome.single",
  merged: "app.batch.exportOutcome.merged",
  separate: "app.batch.exportOutcome.separate",
};

/**
 * 渲染导出结果。`count` 只存在于 `merged` / `separate` 上，故按 `kind` 判别后取值——
 * 判别式联合让 `single` 分支碰不到 `count`，而漏写某个变体时本函数会因
 * 「缺少返回语句」编译不过。
 */
function exportOutcomeText(outcome: ExportOutcome): string {
  switch (outcome.kind) {
    case "merged":
    case "separate":
      return t(EXPORT_OUTCOME_KEY[outcome.kind], { count: outcome.count });
    case "single":
      return t(EXPORT_OUTCOME_KEY.single);
  }
}

/**
 * 注册成功的两种平台措辞。**两种都要能被枚举**：只有被选中的那条会进
 * `CONTEXT_MENU_OUTCOME_KEY`，若另一种不可枚举，它在语言包里是否存在就永远没人校验
 * （jsdom 的 UA 不是 Windows，`registeredWindows` 到不了）。
 */
export const CONTEXT_MENU_REGISTERED_KEY: Record<"macos" | "windows", string> = {
  macos: "app.systemContextMenu.registeredMacos",
  windows: "app.systemContextMenu.registeredWindows",
};

/**
 * 右键菜单结果 → 语言包键，同上由 `Record` 保证键集完整。
 * 导出理由同 `EXPORT_OUTCOME_KEY`（`Record` 的取值不被任何静态检查覆盖）。
 */
export const CONTEXT_MENU_OUTCOME_KEY: Record<ContextMenuOutcome["kind"], string> = {
  registered: CONTEXT_MENU_REGISTERED_KEY[IS_WINDOWS ? "windows" : "macos"],
  unregistered: "app.systemContextMenu.unregistered",
};

export interface ProjectSessionChunkItem {
  session: SessionInfo;
  /** `null` = 后端解析不出项目路径（见 `ProjectInfo::encoded_dir`），不是空串。 */
  encoded_dir: string | null;
  /** `null` = 后端解析不出项目路径，不是空串。聚合时按「没有这个值」处理。 */
  original_path: string | null;
}

interface ScanProjectsStreamDone {
  total_sessions: number;
  cli_results: Array<{
    cli_id: string;
    total_sessions: number;
    // 后端自 B4 起原样携带 `AppError`（`Coded` → `{code, params}`，`Business` → 字符串），
    // 不再发渲染好的字符串。渲染在下面的赋值处过一次 `renderAppError`。
    error: AppErrorPayload | null;
  }>;
}

export function normalizeProjectPath(path: string): string {
  const trimmed = path.trim();
  const windowsPath = /^[a-zA-Z]:[\\/]/.test(trimmed) || /^[\\/]{2}[^\\/]/.test(trimmed);
  const uncPath = /^[\\/]{2}[^\\/]/.test(trimmed);
  let normalized = trimmed.replace(/\\/g, "/");
  normalized = uncPath
    ? `//${normalized.replace(/^\/+/, "").replace(/\/{2,}/g, "/")}`
    : normalized.replace(/\/{2,}/g, "/");

  const isPosixRoot = normalized === "/";
  const isDriveRoot = /^[a-zA-Z]:\/$/.test(normalized);
  if (!isPosixRoot && !isDriveRoot) normalized = normalized.replace(/\/+$/, "");
  return windowsPath ? normalized.toLocaleLowerCase("en-US") : normalized;
}

export function aggregateProjectItems(
  items: ProjectSessionChunkItem[],
): AggregatedProjectInfo[] {
  const grouped = new Map<string, AggregatedProjectInfo>();

  for (const item of items) {
    // 缺席的项目路径不能进 `normalizeProjectPath`（它一上来就 `path.trim()`）；
    // 空串是这一层的「缺席」分组键——真实路径归一后永远不会是空串（`/` 归一成 `/`），
    // 所以它不会和任何真项目撞组。
    const projectKey = item.original_path ? normalizeProjectPath(item.original_path) : "";
    const cliId = isCliId(item.session.cli_id) ? item.session.cli_id : null;
    const existing = grouped.get(projectKey);

    if (existing) {
      existing.sessions.push(item.session);
      if (cliId && !existing.cli_ids.includes(cliId)) existing.cli_ids.push(cliId);
      continue;
    }

    grouped.set(projectKey, {
      project_key: projectKey,
      cli_ids: cliId ? [cliId] : [],
      encoded_dir: item.encoded_dir,
      original_path: item.original_path,
      sessions: [item.session],
    });
  }

  return [...grouped.values()].map((project) => ({
    ...project,
    sessions: [...project.sessions].sort((a, b) => {
      const aTime = Date.parse(a.timestamp);
      const bTime = Date.parse(b.timestamp);
      if (Number.isNaN(aTime) && Number.isNaN(bTime)) return 0;
      if (Number.isNaN(aTime)) return 1;
      if (Number.isNaN(bTime)) return -1;
      return bTime - aTime;
    }),
  }));
}

export function shouldRefreshForCli(
  visibleCliIds: CliId[],
  eventCliId?: string,
): boolean {
  return !eventCliId || (isCliId(eventCliId) && visibleCliIds.includes(eventCliId));
}

export function groupSessionIdentities(
  items: SessionIdentity[],
): Map<CliId, string[]> {
  const groups = new Map<CliId, string[]>();
  for (const item of items) {
    const paths = groups.get(item.cliId);
    if (paths) paths.push(item.filePath);
    else groups.set(item.cliId, [item.filePath]);
  }
  return groups;
}

export function resolveLaunchCliId(
  visibleIds: CliId[],
  options: CliOption[],
  lastSuccessful?: CliId,
): CliId | undefined {
  const installedCreatable = options.filter(
    (option) => option.hasBinary && option.supportsNewSession,
  );
  const visibleCreatable = installedCreatable.filter((option) =>
    visibleIds.includes(option.id),
  );

  if (visibleCreatable.length === 1) return visibleCreatable[0].id;
  if (lastSuccessful && installedCreatable.some((option) => option.id === lastSuccessful)) {
    return lastSuccessful;
  }
  return installedCreatable[0]?.id;
}

// 复用流式加载 + 列表通用编排
const projectsStream = useStreamingCollection<ProjectSessionChunkItem, ScanProjectsStreamDone>({
  command: 'scan_projects',
  topic: 'scan_projects',
});

// projects 从 stream.items 按规范化工作区路径跨 CLI 聚合派生
const projects = computed<AggregatedProjectInfo[]>(() =>
  aggregateProjectItems(projectsStream.items.value)
);
const searchQuery = ref("");
const selectedSessionPath = ref<string | null>(null);
const selectedProjectDir = ref<string | null>(null);
const activeSessionIdentity = ref<SessionIdentity | null>(null);
const skipPermissions = ref(
  localStorage.getItem("seshbuddy-skip-permissions") !== "false"
);
const { terminalAppForLaunch } = useTerminalApp();
const currentCliId = ref<CliId>((() => {
  const saved = localStorage.getItem("seshbuddy-current-cli");
  return saved && isCliId(saved) ? saved : "claude";
})());
const cliFilter = ref<CliFilterState>((() => {
  const saved = localStorage.getItem("seshbuddy-cli-filter");
  if (!saved) return { mode: "all" };
  try {
    const parsed = JSON.parse(saved) as Partial<CliFilterState>;
    if (parsed.mode === "all") return { mode: "all" };
    if (parsed.mode === "custom" && Array.isArray(parsed.cliIds)) {
      return {
        mode: "custom",
        cliIds: parsed.cliIds.filter(
          (cliId): cliId is CliId => typeof cliId === "string" && isCliId(cliId),
        ),
      };
    }
  } catch {
    // Invalid persisted state falls back to the safe default below.
  }
  return { mode: "all" };
})());
const persistedLaunchCliId = (() => {
  const saved = localStorage.getItem("seshbuddy-launch-cli");
  return saved && isCliId(saved) ? saved : undefined;
})();
const lastSuccessfulLaunchCliId = ref<CliId | undefined>(persistedLaunchCliId);
const cliStatuses = ref<Record<CliId, CliRuntime>>({
  claude: { hasSessions: false, hasBinary: false },
  codex: { hasSessions: false, hasBinary: false },
  gemini: { hasSessions: false, hasBinary: false },
  workbuddy: { hasSessions: false, hasBinary: false },
  dsh: { hasSessions: false, hasBinary: false },
  antigravity: { hasSessions: false, hasBinary: false },
});
const cliSessionCounts = ref<Partial<Record<CliId, number>>>({});
const cliPathConfigs = ref<Record<CliId, CliPathConfig | null>>({
  claude: null,
  codex: null,
  gemini: null,
  workbuddy: null,
  dsh: null,
  antigravity: null,
});

const currentCli = computed<ResolvedCliDefinition>(() =>
  resolveCliDefinition(currentCliId.value, cliPathConfigs.value[currentCliId.value])
);
const cliOptions = computed<CliOption[]>(() =>
  SUPPORTED_CLIS.map((cli) => {
    const resolved = resolveCliDefinition(cli.id, cliPathConfigs.value[cli.id]);
    return {
      ...resolved,
      hasSessions: cliStatuses.value[cli.id]?.hasSessions ?? false,
      hasBinary: cliStatuses.value[cli.id]?.hasBinary ?? false,
    };
  })
);
const installedCliOptions = computed<CliOption[]>(() =>
  cliOptions.value.filter((cli) => cli.hasSessions)
);
const hasDetectedCliWithSessions = computed<boolean>(() =>
  installedCliOptions.value.length > 0
);
const availableCliIds = computed<CliId[]>(() =>
  installedCliOptions.value.map((cli) => cli.id)
);
const visibleCliIds = computed<CliId[]>(() =>
  resolveVisibleCliIds(cliFilter.value, availableCliIds.value)
);
const selectedCliIds = computed<CliId[]>(() =>
  cliFilter.value.mode === "all" ? cliOptions.value.map((cli) => cli.id) : cliFilter.value.cliIds
);
const launchCliId = computed<CliId>(() =>
  resolveLaunchCliId(
    visibleCliIds.value,
    cliOptions.value,
    lastSuccessfulLaunchCliId.value,
  ) ?? lastSuccessfulLaunchCliId.value ?? currentCliId.value
);
// 二进制门布尔映射：新建会话等需要可执行文件的动作按此 gate
const cliBinaryStatuses = computed<Record<string, boolean>>(() =>
  Object.fromEntries(
    (Object.entries(cliStatuses.value) as [CliId, CliRuntime][]).map(
      ([id, runtime]) => [id, runtime?.hasBinary ?? false]
    )
  )
);

async function syncCurrentCliWithInstalled() {
  if (visibleCliIds.value.includes(currentCliId.value)) return;

  const fallbackCliId = visibleCliIds.value[0];
  if (!fallbackCliId) return;

  currentCliId.value = fallbackCliId;
  localStorage.setItem("seshbuddy-current-cli", fallbackCliId);
  await invokeApp("refresh_tray_menu", { cliId: fallbackCliId }).catch((error) => {
    console.error("refresh_tray_menu failed:", error);
  });
}

function saveCliFilter(next: CliFilterState) {
  cliFilter.value = next;
  localStorage.setItem("seshbuddy-cli-filter", JSON.stringify(next));
  clearSessionSelection();
}

async function setCliFilter(next: CliFilterState) {
  await invokeApp("set_session_index_scan_clis", {
    cliIds: next.mode === "all" ? cliOptions.value.map((cli) => cli.id) : next.cliIds,
  });
  saveCliFilter(next);
  await refresh();
  if (globalSearchQuery.value.trim()) {
    await globalSearch(globalSearchQuery.value);
  }
}

// Live session watching
const autoFollow = ref(false);

// Sort mode: 'time' (default, most recent first) or 'name' (alphabetical)
type SortMode = "time" | "name";
const sortMode = ref<SortMode>(
  (localStorage.getItem("seshbuddy-sort-mode") as SortMode) || "time"
);

function setSortMode(mode: SortMode) {
  sortMode.value = mode;
  localStorage.setItem("seshbuddy-sort-mode", mode);
}

// Global search state
const globalSearchQuery = ref("");
const searchIndexProgress = ref<SearchIndexProgressPayload | null>(null);
const searchIndexBuildRequest = ref<SearchIndexBuildRequest | null>(null);
const searchIndexBuildSkipped = ref(false);
let searchDebounceTimer: ReturnType<typeof setTimeout> | null = null;

// search_sessions 流式化：复用 useStreamingCollection 的抢占语义替代手写 requestSeq。
// 模块级使用：见 useStreamingCollection JSDoc"生命周期使用规范"——单例 store 场景安全。
interface SearchSessionsStreamDone {
  total: number;
  query: string;
  // 后端 done payload 无 serde 重命名，线上为 snake_case
  pending_cli_ids: string[];
  // 已建索引但有增量未同步的 CLI(本次仍搜索了其已有内容)
  stale_cli_ids: string[];
  // 单来源搜索失败的分项错误(其余来源结果仍正常返回)。
  // 与 scan 通道同形：后端原样携带 `AppError`，不是渲染好的字符串。
  cli_errors?: Array<{ cli_id: string; error: AppErrorPayload }>;
}
const searchStream = useStreamingCollection<SearchResult, SearchSessionsStreamDone>({
  command: 'search_sessions',
  topic: 'search_sessions',
  slowThresholdMs: 0,  // 用户主动搜索：loading 即时显示，无 300ms 沉默期
});
const globalSearchResults = searchStream.items;
const globalSearchLoading = searchStream.isRefreshing;

// all 模式下 done 携带尚未建索引的 CLI 列表，供结果区底部按需构建引导
const searchPendingCliIds = ref<string[]>([]);
// all 模式下已建索引但有增量未同步的 CLI(结果可能缺少最新内容)
const searchStaleCliIds = ref<string[]>([]);
// 扫描/搜索的分项失败：仅记录本次结果中出现的 CLI，成功即清除其错误项。
// 存的是**结构化错误**（`AppErrorPayload`）而不是渲染好的句子：渲染结果随语言变，
// 存进 ref 就把语言冻结在事件发生的那一刻——切语言后外层句子变了、内插的这句没变，
// 一行里两种语言。渲染交给各消费点（`renderAppError`）。
const scanCliErrors = ref<Partial<Record<CliId, AppErrorPayload>>>({});
const searchCliErrors = ref<Partial<Record<CliId, AppErrorPayload>>>({});
// 按需构建在飞的 CLI：进度监听放行非当前 CLI 的事件 + 按钮 loading/防重入
const buildingCliIds = ref<string[]>([]);
watch(() => searchStream.done.value, (done) => {
  searchPendingCliIds.value = done?.pending_cli_ids ?? [];
  searchStaleCliIds.value = done?.stale_cli_ids ?? [];
  // 搜索范围恒为 visibleCliIds 全集，done 的 cli_errors 即当前全部失败项，整体替换
  const errors: Partial<Record<CliId, AppErrorPayload>> = {};
  for (const entry of done?.cli_errors ?? []) {
    if (isCliId(entry.cli_id)) errors[entry.cli_id] = entry.error;
  }
  searchCliErrors.value = errors;
});

// Pagination state
type StartupPhase = "detecting" | "synchronizing" | "scanning" | "ready";
type RefreshMode = "bootstrap" | "foreground" | "snapshot";
type SessionListIndexUpdatedPayload = {
  cliId?: string;
};
type SearchIndexProgressPayload = {
  cliId: string;
  phase?: string;
  current: number;
  total: number;
  startedAtMs?: number;
  processedBytes?: number;
  totalBytes?: number;
  currentPath?: string | null;
  currentFileBytes?: number;
  currentFileSize?: number;
};
type SearchIndexStatusPayload = {
  ready: boolean;
  requiresConfirmation: boolean;
  missingSessions: number;
  totalSessions: number;
  missingBytes: number;
  totalBytes: number;
  physicalIndexReady: boolean;
};
/** 待确认构建里的一项：一个 CLI 的缺口统计。 */
type SearchIndexBuildItem = SearchIndexStatusPayload & { cliId: string };

/**
 * 一次待确认的索引构建请求，承载**一组** CLI。
 *
 * ⚠️ 必须是列表而不是单个：`buildAllPendingSearchIndexes` 是按顺序把每个需要确认的 CLI
 * 入队的，单槽会被后一个覆盖，于是「一键全部」只弹最后一个、其余静默丢失，用户得反复点。
 */
type SearchIndexBuildRequest = {
  items: SearchIndexBuildItem[];
  /** 入队时的关键词；确认时若已变，不再拿它去刷新（结果会落到旧查询上） */
  query: string;
};

const STARTUP_SLOW_MS = 2600;
let sessionIndexListenerPromise: Promise<UnlistenFn> | null = null;
let searchIndexListenerPromise: Promise<UnlistenFn> | null = null;
let sessionUpdatesEnabled = true;

function setSessionUpdatesEnabled(enabled: boolean) {
  sessionUpdatesEnabled = enabled;
}

let isHandlingSessionIndexUpdate = false;
let pendingSessionIndexUpdate = false;

async function handleSessionIndexUpdated(cliId?: string) {
  if (!sessionUpdatesEnabled) return;
  if (!shouldRefreshForCli(selectedCliIds.value, cliId)) {
    return;
  }
  if (cliId && isCliId(cliId) && !visibleCliIds.value.includes(cliId)) {
    await loadCliStatuses();
    if (!visibleCliIds.value.includes(cliId)) return;
  }
  // 若正在前台流式刷新或正在处理快照更新，排队等待，避免打断当前流
  if (projectsStream.isRefreshing.value || isHandlingSessionIndexUpdate) {
    pendingSessionIndexUpdate = true;
    return;
  }
  isHandlingSessionIndexUpdate = true;
  try {
    await refresh("snapshot");
    if (globalSearchQuery.value.trim()) {
      await globalSearch(globalSearchQuery.value);
    }
  } catch (e) {
    console.error("session-list-index-updated refresh failed:", e);
  } finally {
    isHandlingSessionIndexUpdate = false;
    if (pendingSessionIndexUpdate) {
      pendingSessionIndexUpdate = false;
      void handleSessionIndexUpdated();
    }
  }
}

function ensureSessionIndexListener() {
  if (sessionIndexListenerPromise) return;

  sessionIndexListenerPromise = listen<SessionListIndexUpdatedPayload>(
    "session-list-index-updated",
    async (event) => {
      const cliId = event.payload?.cliId;
      await handleSessionIndexUpdated(cliId);
    }
  );
}

function ensureSearchIndexListener() {
  if (searchIndexListenerPromise) return;

  searchIndexListenerPromise = listen<SearchIndexProgressPayload>(
    "search-index-progress",
    (event) => {
      const { cliId, current, total, phase } = event.payload;
      if (!visibleCliIds.value.includes(cliId as CliId) && !buildingCliIds.value.includes(cliId)) return;
      
      if (phase === "done" || (!phase && current >= total)) {
        searchIndexProgress.value = null;
      } else if (phase === "error") {
        // Terminal error frame: clear progress like "done" so the UI is not
        // stuck showing progress and the search debounce is unblocked.
        console.error("search-index-progress error:", event.payload);
        searchIndexProgress.value = null;
      } else {
        searchIndexBuildRequest.value = null;
        searchIndexBuildSkipped.value = false;
        searchIndexProgress.value = {
          ...event.payload,
          startedAtMs: searchIndexProgress.value?.startedAtMs ?? Date.now(),
        };
      }
    }
  );
}

const deleteProgress = ref<BatchDeleteProgress | null>(null);

let batchDeleteProgressListenerPromise: Promise<void> | null = null;
function ensureBatchDeleteProgressListener(): Promise<void> {
  if (batchDeleteProgressListenerPromise) return batchDeleteProgressListenerPromise;
  batchDeleteProgressListenerPromise = listen<BatchDeleteProgress>(
    "batch-delete-progress",
    (event) => {
      if (deleteProgress.value != null) deleteProgress.value = event.payload;
    }
  ).then(() => {});
  return batchDeleteProgressListenerPromise;
}

const totalSessionCount = ref(0);
// 暴露给 UI 的入口（保留旧名字以减少调用点扰动）
const isRefreshing = projectsStream.isRefreshing;
const showLoadingIndicator = projectsStream.showLoadingIndicator;
const totalLoadedSessions = projectsStream.totalItems;
// isStreamingProjects 历史调用方语义，现等价 isRefreshing
const isStreamingProjects = isRefreshing;
const bootstrapping = ref(true);
const hasCompletedInitialScan = ref(false);
const initialScanSettled = ref(false);
const startupPhase = ref<StartupPhase>("detecting");
const startupSlow = ref(false);

watch(
  [projectsStream.done, projectsStream.error],
  ([done, error]) => {
    if (initialScanSettled.value || (!done && !error)) return;
    initialScanSettled.value = true;
    hasCompletedInitialScan.value = true;
    bootstrapping.value = false;
    startupSlow.value = false;
    if (done) startupPhase.value = "ready";
  },
  { flush: "sync" },
);

const { isBlocked } = useBlockedFolders();

const filteredProjects = computed(() => {
  const q = searchQuery.value.toLowerCase().trim();
  let result = projects.value;

  // Filter out blocked projects
  result = result.filter((p) => !isBlocked(p.original_path ?? ""));

  if (q) {
    result = result
      .map((p) => {
        const projectMatches = (p.original_path ?? "").toLowerCase().includes(q);
        return {
          ...p,
          sessions: projectMatches
            ? p.sessions
            : p.sessions.filter((s) => sessionMatchesQuery(s, q)),
        };
      })
      .filter((p) => p.sessions.length > 0);
  }

  if (sortMode.value === "name") {
    result = [...result].sort((a, b) =>
      (a.original_path ?? "").localeCompare(b.original_path ?? "")
    );
  } else {
    result = [...result].sort((a, b) => {
      const aTime = Date.parse(a.sessions[0]?.timestamp ?? "");
      const bTime = Date.parse(b.sessions[0]?.timestamp ?? "");
      return (Number.isNaN(bTime) ? 0 : bTime) - (Number.isNaN(aTime) ? 0 : aTime);
    });
  }

  return result;
});

async function loadCliStatuses() {
  try {
    const statuses = await invokeApp<CliStatus[]>("list_cli_statuses");
    const next: Record<CliId, CliRuntime> = {
      claude: { hasSessions: false, hasBinary: false },
      codex: { hasSessions: false, hasBinary: false },
      gemini: { hasSessions: false, hasBinary: false },
      workbuddy: { hasSessions: false, hasBinary: false },
      dsh: { hasSessions: false, hasBinary: false },
      antigravity: { hasSessions: false, hasBinary: false },
    };
    for (const status of statuses) {
      if (isCliId(status.id)) {
        next[status.id] = {
          hasSessions: status.has_sessions,
          hasBinary: status.has_binary,
        };
      }
    }
    cliStatuses.value = next;
  } catch (e) {
    console.error("list_cli_statuses failed:", e);
  }
}

async function loadCliPathConfigs() {
  try {
    const configs = await invokeApp<CliPathConfig[]>("list_cli_path_configs");
    const next: Record<CliId, CliPathConfig | null> = {
      claude: null,
      codex: null,
      gemini: null,
      workbuddy: null,
      dsh: null,
      antigravity: null,
    };
    for (const config of configs) {
      if (isCliId(config.id)) {
        next[config.id] = config;
      }
    }
    cliPathConfigs.value = next;
  } catch (e) {
    console.error("list_cli_path_configs failed:", e);
  }
}

function clearActiveSession() {
  autoFollow.value = false;
  activeSessionIdentity.value = null;
  selectedSessionPath.value = null;
  selectedProjectDir.value = null;
}

function setActiveSession(identity: SessionIdentity, projectDir: string) {
  activeSessionIdentity.value = identity;
  selectedSessionPath.value = identity.filePath;
  selectedProjectDir.value = projectDir;
}

async function setCurrentCli(cliId: CliId) {
  if (!cliStatuses.value[cliId]?.hasSessions) return;
  if (currentCliId.value === cliId) return;
  currentCliId.value = cliId;
  localStorage.setItem("seshbuddy-current-cli", cliId);
  invokeApp("refresh_tray_menu", { cliId }).catch((error) => {
    console.error("refresh_tray_menu failed:", error);
  });
  clearActiveSession();
  selectedSessionIdentities.value = [];
  globalSearchQuery.value = '';
  void searchStream.refresh({ cliIds: visibleCliIds.value, query: '' });
  await refresh();
}

async function ensureCliInstalled(cliId: CliId): Promise<boolean> {
  try {
    const installed = await invokeApp<boolean>("detect_cli", {
      cliId,
    });
    cliStatuses.value = {
      ...cliStatuses.value,
      [cliId]: {
        ...cliStatuses.value[cliId],
        hasBinary: installed,
      },
    };
    if (installed) return true;
  } catch (e) {
    console.error("detect_cli failed:", e);
  }

  await ask(cliInstallHint(getCliDefinition(cliId)), {
    title: t("app.dialog.notice"),
    kind: "info",
  });
  return false;
}

async function ensureCurrentCliInstalled(): Promise<boolean> {
  return ensureCliInstalled(currentCliId.value);
}

function recordSuccessfulLaunch(cliId: CliId) {
  lastSuccessfulLaunchCliId.value = cliId;
  localStorage.setItem("seshbuddy-launch-cli", cliId);
}

let isRefreshRunning = false;

async function refresh(
  mode: RefreshMode = bootstrapping.value ? "bootstrap" : "foreground"
) {
  if (mode === "foreground" && isRefreshRunning) {
    return;
  }
  isRefreshRunning = true;
  try {
    ensureSessionIndexListener();
    ensureSearchIndexListener();
    await refreshInner(mode);
  } finally {
    isRefreshRunning = false;
  }
}

async function refreshInner(mode: RefreshMode) {
  const isInitialRefresh = bootstrapping.value;
  let slowTimer: ReturnType<typeof setTimeout> | null = null;

  if (isInitialRefresh) {
    startupPhase.value = "detecting";
    startupSlow.value = false;
    slowTimer = setTimeout(() => {
      startupSlow.value = true;
    }, STARTUP_SLOW_MS);
  }

  const cliTasks = (async () => {
    await Promise.all([
      loadCliStatuses(),
      loadCliPathConfigs(),
    ]);
    if (isInitialRefresh) {
      startupPhase.value = "synchronizing";
    }
    await syncCurrentCliWithInstalled();
  })();

  const scanTask = (async () => {
    if (mode === "foreground") {
      // 后台触发索引刷新，不阻塞流式 start；notify: false 避免向前端广播打断正在展示的前台流
      refreshVisibleSessionIndexes(false);
    }

    if (isInitialRefresh) {
      await cliTasks; // Ensure CLI is synced before scanning if initial
      startupPhase.value = "scanning";
    }

    const previousActiveIdentity = activeSessionIdentity.value;

    // composable 内部已处理：原子替换 / 300ms 阈值 / done with 0 真清空 / error 隐式回滚。
    // snapshot（文件监听定时刷新）静默执行：不清空列表、不显示 loading pill，避免周期性闪屏。
    await projectsStream.refresh(
      { cliIds: visibleCliIds.value },
      {
        // 首次启动允许渐进展示；已有目录的前台/监听刷新必须等完整扫描
        // 结果到齐后再一次性替换，避免目录树随 chunk 半份半份地重排。
        silent: mode === "snapshot",
        atomicSwap: mode !== "bootstrap",
      }
    );

    // 流式刷新期间若有被按住的增量索引变更，在此统一补扫一次
    if (pendingSessionIndexUpdate) {
      pendingSessionIndexUpdate = false;
      void handleSessionIndexUpdated();
    }

    // done 时同步全量计数（done payload 才有总数）
    if (projectsStream.done.value) {
      totalSessionCount.value = projectsStream.done.value.total_sessions;
      const nextScanErrors = { ...scanCliErrors.value };
      for (const result of projectsStream.done.value.cli_results ?? []) {
        if (!isCliId(result.cli_id)) continue;
        if (result.error) {
          // 分项失败：记录错误，保留该 CLI 旧计数。与搜索通道存同一种形状（结构化），
          // 渲染交给消费点。
          nextScanErrors[result.cli_id] = result.error;
          continue;
        }
        delete nextScanErrors[result.cli_id];
        cliSessionCounts.value = {
          ...cliSessionCounts.value,
          [result.cli_id]: result.total_sessions,
        };
      }
      scanCliErrors.value = nextScanErrors;
    }

    if (previousActiveIdentity) {
      const found = projects.value.some((p) =>
        p.sessions.some(
          (session) => session.file_path === previousActiveIdentity.filePath
            && session.cli_id === previousActiveIdentity.cliId,
        )
      );
      if (!found) {
        clearActiveSession();
      }
    }

    if (isInitialRefresh && (projectsStream.done.value || projectsStream.error.value)) {
      startupPhase.value = "ready";
    }
  })();

  if (mode === "bootstrap") {
    void cliTasks.then(() => refreshVisibleSessionIndexes(false));
  }

  // The stream's promise may return at its timeout while its done event is still pending.
  // Initial loading only settles when the stream reports done or error.
  await Promise.all([cliTasks, scanTask]);
  if (slowTimer && initialScanSettled.value) clearTimeout(slowTimer);
}

let isRefreshingIndexes = false;
function refreshVisibleSessionIndexes(notify = true) {
  if (isRefreshingIndexes) return;
  isRefreshingIndexes = true;
  const tasks = visibleCliIds.value.map((cliId) =>
    invokeApp<boolean>("refresh_session_list_index", {
      cliId,
      notify,
    }).catch((e) => {
      console.error(`background refresh_session_list_index failed (${cliId}):`, e);
      return false;
    })
  );
  void Promise.all(tasks).finally(() => {
    isRefreshingIndexes = false;
  });
}

// 单个 CLI 扫描失败后的手动重试：强制重建其列表索引并重新扫描
async function retryCliScan(cliId: CliId) {
  try {
    await invokeApp<boolean>("refresh_session_list_index", {
      cliId,
      notify: true,
      force: true,
    });
  } catch (e) {
    console.error(`retryCliScan refresh_session_list_index failed (${cliId}):`, e);
  }
  await refresh();
}

function findSessionProjectDir(filePath: string): string | null {
  for (const project of projects.value) {
    if (project.sessions.some((session) => session.file_path === filePath)) {
      return project.encoded_dir;
    }
  }
  return null;
}

async function ensureSessionVisible(filePath: string): Promise<string | null> {
  // 1. 快路径：已在 projects 里
  let encodedDir = findSessionProjectDir(filePath);
  if (encodedDir) return encodedDir;

  // 2. 流式加载中 → 等当前 stream 跑完再重查
  if (projectsStream.isRefreshing.value) {
    await waitForStreamToSettle();
    encodedDir = findSessionProjectDir(filePath);
    if (encodedDir) return encodedDir;
  }

  // 3. 没在加载也没找到 → 触发一次 refresh 再查
  await refresh();
  return findSessionProjectDir(filePath);
}

function waitForStreamToSettle(): Promise<void> {
  return new Promise<void>((resolve) => {
    if (!projectsStream.isRefreshing.value) {
      resolve();
      return;
    }
    const stop = watch(
      () => projectsStream.isRefreshing.value,
      (loading) => {
        if (!loading) {
          stop();
          resolve();
        }
      }
    );
  });
}

function setAutoFollow(val: boolean) {
  autoFollow.value = val;
}

/**
 * 内存级瞬时剔除已删会话（0ms 响应体感）。
 *
 * 业务背景：
 * 在用户确认删除后，若等待全盘扫描或流式加载完成（耗时可达数秒），已删会话会在界面停滞显示
 * 造成卡顿与未响应感。此函数直接同步从内存集合中移除已删项，并触发响应式更新与计数同步，
 * 使得侧边栏与搜索面板在下一帧瞬间更新，随后由后台静默同步确保磁盘与内存最终一致。
 */
function removeSessionIdentitiesFromMemory(identities: SessionIdentity[]) {
  if (identities.length === 0) return;
  const removedKeys = new Set(identities.map(sessionIdentityKey));
  const prevLen = projectsStream.items.value.length;
  const nextItems = projectsStream.items.value.filter((item) => {
    const key = sessionIdentityKey({
      cliId: item.session.cli_id as CliId,
      filePath: item.session.file_path,
    });
    return !removedKeys.has(key);
  });

  if (nextItems.length !== prevLen) {
    projectsStream.items.value = nextItems;
    triggerRef(projectsStream.items);
    totalLoadedSessions.value = nextItems.length;

    const removedCount = prevLen - nextItems.length;
    if (totalSessionCount.value >= removedCount) {
      totalSessionCount.value -= removedCount;
    } else {
      totalSessionCount.value = 0;
    }

    const countsByCli: Partial<Record<CliId, number>> = {};
    for (const identity of identities) {
      countsByCli[identity.cliId] = (countsByCli[identity.cliId] ?? 0) + 1;
    }
    const nextCliSessionCounts = { ...cliSessionCounts.value };
    for (const [cliIdStr, count] of Object.entries(countsByCli)) {
      const cliId = cliIdStr as CliId;
      const current = nextCliSessionCounts[cliId];
      if (typeof current === "number") {
        nextCliSessionCounts[cliId] = Math.max(0, current - (count ?? 0));
      }
    }
    cliSessionCounts.value = nextCliSessionCounts;
  }

  // 同步剔除全局搜索结果中的匹配项
  if (searchStream.items.value.length > 0) {
    const nextSearchResults = searchStream.items.value.filter((item) => {
      const key = sessionIdentityKey({
        cliId: item.cli_id as CliId,
        filePath: item.file_path,
      });
      return !removedKeys.has(key);
    });
    if (nextSearchResults.length !== searchStream.items.value.length) {
      searchStream.items.value = nextSearchResults;
      triggerRef(searchStream.items);
    }
  }
}

/**
 * 把 `delete_sessions_to_trash` 的失败原因渲染成一句可插值的文案。
 *
 * **必须经 `renderAppError`**：后端的 `failed` 第二个元素是 `AppError` 的线格式，
 * `Coded` 发的是 `{ code, params }` **对象**，原样插进 `{error}` / `{reasons}` 占位符会渲染成
 * `[object Object]`（对象会被 `JSON.stringify` 成多行 JSON 上屏）——**不报错、不崩，只是文案错了**，
 * 所以漏掉这一步没有任何编译期或运行时症状。
 *
 * **收成一处**是为了让这件事**没有第二次做错的机会**：三个渲染点
 * （单会话删除的 `{error}`、项目删除与批量删除的 `{reasons}`）都走它，
 * 谁都不能再自己拼一遍。去重与分隔符与改前 `deleteProject` 的口径一致。
 */
function renderDeleteReasons(failed: [string, AppErrorPayload][]): string {
  const reasons = [...new Set(failed.map(([, reason]) => renderAppError(reason)))];
  return reasons.join(t("session.common.errorSeparator"));
}

async function deleteSession(
  identity: SessionIdentity,
  displayName: string,
): Promise<boolean> {
  const previewTitle = formatDialogSessionTitle(displayName);
  const confirmed = await ask(
    t("dialogs.deleteConfirm.sessionMessage", { title: previewTitle }),
    { title: t("dialogs.deleteConfirm.title"), kind: "warning" }
  );
  if (!confirmed) return false;

  try {
    const outcome = await invokeApp<{ failed: [string, AppErrorPayload][]; succeeded: number }>(
      "delete_sessions_to_trash",
      { cliId: identity.cliId, paths: [identity.filePath] }
    );
    if (outcome.failed.length > 0) {
      await message(t("session.dialog.deleteFailedMessage", { error: renderDeleteReasons(outcome.failed) }), {
        title: t("settings.archivedSessions.deleteFailed"),
        kind: "error",
      });
      return false;
    }

    // 内存级瞬时剔除：侧边栏与计数器下一帧瞬间消失，杜绝等待全盘扫描导致的卡滞感
    removeSessionIdentitiesFromMemory([identity]);

    if (
      activeSessionIdentity.value
      && sessionIdentityKey(activeSessionIdentity.value) === sessionIdentityKey(identity)
    ) {
      clearActiveSession();
    }
    // 后台静默同步校验磁盘最新状态，不阻塞当前删除操作响应
    void refresh("snapshot");
    return true;
  } catch (e) {
    await message(renderAppError(e), { title: t("app.dialog.error"), kind: "error" });
    return false;
  }
}

interface GroupedDeleteResult {
  failed: [string, AppErrorPayload][];
  succeeded: number;
  succeededKeys: Set<string>;
}

async function deleteIdentityGroups(identities: SessionIdentity[]): Promise<GroupedDeleteResult> {
  const result: GroupedDeleteResult = {
    failed: [],
    succeeded: 0,
    succeededKeys: new Set<string>(),
  };

  for (const [cliId, paths] of groupSessionIdentities(identities)) {
    try {
      const outcome = await invokeApp<{ failed: [string, AppErrorPayload][]; succeeded: number }>(
        "delete_sessions_to_trash",
        { cliId, paths },
      );
      result.failed.push(...outcome.failed);
      result.succeeded += outcome.succeeded;
      const failedPaths = new Set(outcome.failed.map(([path]) => path));
      for (const filePath of paths) {
        if (!failedPaths.has(filePath)) {
          result.succeededKeys.add(sessionIdentityKey({ cliId, filePath }));
        }
      }
    } catch (error) {
      const reason = renderAppError(error);
      result.failed.push(...paths.map((path): [string, AppErrorPayload] => [path, reason]));
    }
  }

  return result;
}

async function deleteProject(project: ProjectInfo): Promise<boolean> {
  const count = project.sessions.length;
  const confirmed = await ask(
    t("dialogs.deleteConfirm.projectMessage", {
      // 缺席时给本地化兜底句：vue-i18n 会把 `null` 渲染成空串，对话框上就只剩一个空位
      path: project.original_path || t("session.unknownProject"),
      count,
    }),
    { title: t("dialogs.deleteConfirm.title"), kind: "warning" }
  );
  if (!confirmed) return false;

  try {
    const identities = project.sessions
      .filter((session) => isCliId(session.cli_id))
      .map((session) => ({ cliId: session.cli_id as CliId, filePath: session.file_path }));
    const outcome = await deleteIdentityGroups(identities);
    selectedSessionIdentities.value = selectedSessionIdentities.value.filter(
      (item) => !outcome.succeededKeys.has(sessionIdentityKey(item)),
    );

    // 内存级瞬时剔除所有成功删除的会话项
    const succeededIdentities = identities.filter((item) =>
      outcome.succeededKeys.has(sessionIdentityKey(item))
    );
    removeSessionIdentitiesFromMemory(succeededIdentities);

    if (
      activeSessionIdentity.value
      && outcome.succeededKeys.has(sessionIdentityKey(activeSessionIdentity.value))
    ) {
      clearActiveSession();
    }
    // 后台静默校验磁盘最新状态
    void refresh("snapshot");
    if (outcome.failed.length > 0) {
      await message(
        t("session.dialog.deletePartial", {
          succeeded: outcome.succeeded,
          failed: outcome.failed.length,
          reasons: renderDeleteReasons(outcome.failed),
        }),
        { title: t("session.dialog.deleteCompletedTitle"), kind: "warning" },
      );
      return false;
    }
    return true;
  } catch (e) {
    await message(renderAppError(e), { title: t("app.dialog.error"), kind: "error" });
    return false;
  }
}

async function exportSession(
  identity: SessionIdentity,
  format: "txt" | "markdown" | "json" | "jsonl" = "txt",
  projectPath?: string,
  _sessionId?: string,
  selectedIndexes?: number[]
) {
  const extMap = { txt: "txt", markdown: "md", json: "json", jsonl: "jsonl" } as const;
  const nameMap = { txt: "Text", markdown: "Markdown", json: "JSON", jsonl: "JSON Lines" } as const;
  const ext = extMap[format];

  const baseName = projectPath
    ? projectPath.split(/[\\/]/).filter(Boolean).pop() ?? "session"
    : "session";

  const safeName = baseName.replace(/[<>:"/\\|?* ]/g, "_");
  const now = new Date();
  const dateStr = `${now.getFullYear()}${(now.getMonth() + 1).toString().padStart(2, "0")}${now.getDate().toString().padStart(2, "0")}_${now.getHours().toString().padStart(2, "0")}${now.getMinutes().toString().padStart(2, "0")}${now.getSeconds().toString().padStart(2, "0")}`;
  const suffix = selectedIndexes && selectedIndexes.length > 0 ? "_selected" : "";

  const defaultPath = `${safeName}_${dateStr}${suffix}.${ext}`;

  const savePath = await save({
    filters: [{ name: nameMap[format], extensions: [ext] }],
    defaultPath,
  });
  if (!savePath) return;

  try {
    const outcome = await invokeApp<ExportOutcome>("export_session", {
      cliId: identity.cliId,
      filePath: identity.filePath,
      savePath,
      format,
      selectedIndexes: selectedIndexes || null,
    });
    await message(exportOutcomeText(outcome), { title: t("app.batch.export"), kind: "info" });
  } catch (e) {
    await message(renderAppError(e), { title: t("app.dialog.error"), kind: "error" });
  }
}

async function newSession(
  projectPath?: string,
  profileName?: string | null,
  skipOverride?: boolean,
  cliId: CliId = launchCliId.value,
) {
  const launchCli = getCliDefinition(cliId);
  if (!launchCli.supportsNewSession) {
    await message(t("session.dialog.newSessionUnsupported", { cli: launchCli.name }), {
      title: t("app.dialog.notice"),
      kind: "info",
    });
    return;
  }
  if (!(await ensureCliInstalled(cliId))) {
    return;
  }

  let targetPath = projectPath;
  if (!targetPath) {
    const selected = await open({ directory: true });
    if (!selected) return;
    targetPath = selected as string;
  }

  try {
    await invokeApp("open_in_terminal", {
      cliId,
      projectPath: targetPath,
      sessionId: null,
      skipPermissions: skipOverride ?? skipPermissions.value,
      profileName: profileName || undefined,
      terminalApp: terminalAppForLaunch.value ?? undefined,
    });
    recordSuccessfulLaunch(cliId);
  } catch (e) {
    await message(renderAppError(e), { title: t("app.dialog.error"), kind: "error" });
  }
}

async function ensureSessionReadyOnDisk(
  filePath: string | undefined,
  sessionId: string | undefined,
  cliId: CliId,
): Promise<boolean> {
  let targetPath = filePath;
  if (!targetPath && sessionId) {
    const item = projectsStream.items.value.find(
      (i) => i.session.session_id === sessionId && i.session.cli_id === cliId,
    );
    if (item) {
      targetPath = item.session.file_path;
    }
  }
  if (!targetPath) return true;

  const item = projectsStream.items.value.find(
    (i) => i.session.file_path === targetPath && i.session.cli_id === cliId,
  );
  if (item?.session.is_archived) {
    try {
      await invokeApp("restore_session_to_disk", {
        cliId,
        filePath: targetPath,
      });
      item.session.is_archived = false;
      await refresh("snapshot");
    } catch (e: any) {
      console.warn("restore_session_to_disk error:", e);
    }
  }
  return true;
}

async function resumeSession(
  projectPath: string,
  sessionId: string,
  cliId: CliId,
  profileName?: string | null,
  skipOverride?: boolean,
) {
  const targetCli = getCliDefinition(cliId);
  if (!targetCli.supportsResumeSession) {
    return;
  }
  if (!(await ensureCliInstalled(cliId))) {
    return;
  }

  await ensureSessionReadyOnDisk(undefined, sessionId, cliId);

  try {
    await invokeApp("open_in_terminal", {
      cliId,
      projectPath,
      sessionId,
      skipPermissions: skipOverride ?? skipPermissions.value,
      profileName: profileName || undefined,
      terminalApp: terminalAppForLaunch.value ?? undefined,
    });
  } catch (e) {
    await message(renderAppError(e), { title: t("app.dialog.error"), kind: "error" });
  }
}

/// Fork 会话：以 anchorUuid 对应消息为终点复制截断转录，返回新 sessionId。
/// 只负责产生新会话文件，启动/复制命令由调用方编排。
async function forkSession(identity: SessionIdentity, anchorUuid: string): Promise<string> {
  return invokeApp<string>("fork_session", {
    cliId: identity.cliId,
    filePath: identity.filePath,
    anchorUuid,
  });
}

async function setCliDataDirOverride(cliId: CliId, dataDir: string | null) {
  await invokeApp("set_cli_data_dir", {
    cliId,
    dataDir,
  });
  await loadCliPathConfigs();
  if (visibleCliIds.value.includes(cliId)) {
    await refresh();
  }
}

async function registerContextMenu(cliId: CliId = currentCliId.value) {
  if (!getCliDefinition(cliId).supportsContextMenu) {
    await message(t("settings.integration.contextMenuUnsupported"), {
      title: t("app.dialog.notice"),
      kind: "info",
    });
    return;
  }

  try {
    const outcome = await invokeApp<ContextMenuOutcome>("register_context_menu", {
      skipPermissions: skipPermissions.value,
      terminalApp: terminalAppForLaunch.value ?? undefined,
    });
    await message(t(CONTEXT_MENU_OUTCOME_KEY[outcome.kind]), {
      title: t("app.dialog.notice"),
      kind: "info",
    });
  } catch (e) {
    await message(renderAppError(e), { title: t("app.dialog.error"), kind: "error" });
  }
}

async function unregisterContextMenu() {
  try {
    const outcome = await invokeApp<ContextMenuOutcome>("unregister_context_menu");
    await message(t(CONTEXT_MENU_OUTCOME_KEY[outcome.kind]), {
      title: t("app.dialog.notice"),
      kind: "info",
    });
  } catch (e) {
    await message(renderAppError(e), { title: t("app.dialog.error"), kind: "error" });
  }
}

async function globalSearch(query: string) {
  globalSearchQuery.value = query;
  const trimmedQuery = query.trim();

  if (searchDebounceTimer) {
    clearTimeout(searchDebounceTimer);
    searchDebounceTimer = null;
  }

  if (!trimmedQuery) {
    searchIndexBuildRequest.value = null;
    searchIndexBuildSkipped.value = false;
    // 空 query：refresh 触发后端快路径（不查 DB，立即 emit done(0)），
    // composable 的 done-with-0-chunk 分支清空 items
    void searchStream.refresh({ cliIds: visibleCliIds.value, query: '' });
    return;
  }

  const frontCliId = visibleCliIds.value.includes(currentCliId.value)
    ? currentCliId.value
    : visibleCliIds.value[0];
  searchDebounceTimer = setTimeout(async () => {
    searchDebounceTimer = null;
    try {
      if (searchIndexProgress.value) {
        return;
      }
      // 抢占检查：debounce 期间 query 可能已改
      if (globalSearchQuery.value.trim() !== trimmedQuery) {
        return;
      }
      await searchStream.refresh({
        cliIds: visibleCliIds.value,
        frontCliId,
        query: trimmedQuery,
      });
    } catch (e) {
      console.error('search_sessions_stream failed:', e);
    }
  }, 300);
}

async function startSearchIndexBuild() {
  const request = searchIndexBuildRequest.value;
  if (!request) return;
  searchIndexBuildRequest.value = null;
  searchIndexBuildSkipped.value = false;
  const targetIds = request.items
    .map((item) => item.cliId)
    .filter((id) => !buildingCliIds.value.includes(id));
  if (targetIds.length === 0) return;
  buildingCliIds.value = [...buildingCliIds.value, ...targetIds];
  const built: string[] = [];
  try {
    // 逐个构建、**各自兜错**：一个 CLI 失败不该把「一键全部」退化成「点一次建一个」。
    for (const id of targetIds) {
      try {
        await invokeApp('ensure_search_index_ready', { cliId: id });
        built.push(id);
      } catch (e) {
        console.error('search index build failed:', id, e);
      }
    }
    if (globalSearchQuery.value.trim() !== request.query) {
      return;
    }
    await searchStream.refresh({
      cliIds: visibleCliIds.value,
      frontCliId: visibleCliIds.value.includes(currentCliId.value)
        ? currentCliId.value
        : visibleCliIds.value[0],
      query: request.query,
    });
  } finally {
    buildingCliIds.value = buildingCliIds.value.filter((id) => !targetIds.includes(id));
    // 兜底清一次：`searchStream.refresh` 的 done 会重算这两个列表，但关键词在构建期间
    // 变了时会走上面的早退分支、根本不刷新，状态就残留下来逼用户再点一次。
    // 只清**构建成功**的——失败的缺口应当留着让用户看到并重试。
    searchPendingCliIds.value = searchPendingCliIds.value.filter((id) => !built.includes(id));
    searchStaleCliIds.value = searchStaleCliIds.value.filter((id) => !built.includes(id));
  }
}

// all 模式结果区底部「未索引 CLI」的构建入口；大索引复用确认弹窗流程
async function buildPendingSearchIndex(cliId: string) {
  if (buildingCliIds.value.includes(cliId)) return;
  try {
    const status = await invokeApp<SearchIndexStatusPayload>('get_search_index_status', { cliId });
    if (!status.ready && status.requiresConfirmation) {
      searchIndexBuildSkipped.value = false;
      const query = globalSearchQuery.value.trim();
      const prev = searchIndexBuildRequest.value;
      // 关键词变了就重开一次请求：旧请求的 items 是针对旧查询收集的
      const kept = prev && prev.query === query ? prev.items : [];
      searchIndexBuildRequest.value = {
        // 同一 CLI 重复入队只记一次（连点「一键全部」或与单个入口混用时会发生）
        items: [...kept.filter((it) => it.cliId !== cliId), { ...status, cliId }],
        query,
      };
      return;
    }
    buildingCliIds.value = [...buildingCliIds.value, cliId];
    try {
      await invokeApp('ensure_search_index_ready', { cliId });
      await searchStream.refresh({
        cliIds: visibleCliIds.value,
        frontCliId: visibleCliIds.value.includes(currentCliId.value)
          ? currentCliId.value
          : visibleCliIds.value[0],
        query: globalSearchQuery.value.trim(),
      });
      searchPendingCliIds.value = searchPendingCliIds.value.filter((id) => id !== cliId);
      searchStaleCliIds.value = searchStaleCliIds.value.filter((id) => id !== cliId);
    } finally {
      buildingCliIds.value = buildingCliIds.value.filter((id) => id !== cliId);
    }
  } catch (e) {
    console.error('build pending search index failed:', e);
  }
}

async function buildAllPendingSearchIndexes(cliIds?: string[]) {
  // 去重：同一个 CLI 入队两次会让它在确认框里出现两遍（`buildPendingSearchIndex` 按 cliId
  // 去重，这里再去一次是为了让「已入队几个」与用户看到的按钮数一致）
  const targetIds = [...new Set(cliIds ?? searchPendingCliIds.value)].filter(
    (id) => !buildingCliIds.value.includes(id)
  );
  if (targetIds.length === 0) return;
  for (const id of targetIds) {
    await buildPendingSearchIndex(id);
  }
}

function dismissSearchIndexBuild() {
  searchIndexBuildRequest.value = null;
  searchIndexBuildSkipped.value = true;
}

// 组件挂载时清除过期状态用：不设置 'skipped'，
// 与 dismissSearchIndexBuild（用户主动关闭）区分。
function resetSearchIndexBuildState() {
  searchIndexBuildRequest.value = null;
  searchIndexBuildSkipped.value = false;
}

function setSkipPermissions(val: boolean) {
  skipPermissions.value = val;
  localStorage.setItem("seshbuddy-skip-permissions", String(val));
}

const selectedSessionIdentities = ref<SessionIdentity[]>([]);

function toggleSessionSelect(identity: SessionIdentity) {
  const key = sessionIdentityKey(identity);
  const idx = selectedSessionIdentities.value.findIndex(
    (item) => sessionIdentityKey(item) === key,
  );
  if (idx !== -1) {
    selectedSessionIdentities.value = [
      ...selectedSessionIdentities.value.slice(0, idx),
      ...selectedSessionIdentities.value.slice(idx + 1),
    ];
  } else {
    selectedSessionIdentities.value = [...selectedSessionIdentities.value, identity];
  }
}

function clearSessionSelection() {
  selectedSessionIdentities.value = [];
}

function selectAllSessions() {
  const allSessions: SessionIdentity[] = [];
  for (const p of filteredProjects.value) {
    for (const s of p.sessions) {
      if (isCliId(s.cli_id)) {
        allSessions.push({ cliId: s.cli_id, filePath: s.file_path });
      }
    }
  }
  selectedSessionIdentities.value = allSessions;
}

function selectProjectSessions(projectKey: string) {
  const project = filteredProjects.value.find(
    (p) => p.project_key === projectKey || p.encoded_dir === projectKey,
  );
  if (!project) return;
  const projectIdentities = project.sessions
    .filter((session) => isCliId(session.cli_id))
    .map((session) => ({ cliId: session.cli_id as CliId, filePath: session.file_path }));
  const current = new Map(
    selectedSessionIdentities.value.map((item) => [sessionIdentityKey(item), item]),
  );
  const allSelected = projectIdentities.every((item) => current.has(sessionIdentityKey(item)));
  if (allSelected) {
    for (const item of projectIdentities) {
      current.delete(sessionIdentityKey(item));
    }
  } else {
    for (const item of projectIdentities) {
      current.set(sessionIdentityKey(item), item);
    }
  }
  selectedSessionIdentities.value = [...current.values()];
}

function isProjectAllSelected(projectKey: string): boolean {
  const project = filteredProjects.value.find(
    (p) => p.project_key === projectKey || p.encoded_dir === projectKey,
  );
  if (!project || project.sessions.length === 0) return false;
  const selected = new Set(
    selectedSessionIdentities.value.map((item) => sessionIdentityKey(item)),
  );
  return project.sessions.every(
    (session) => isCliId(session.cli_id)
      && selected.has(sessionIdentityKey({ cliId: session.cli_id, filePath: session.file_path })),
  );
}

const isAllSelected = computed(() => {
  if (selectedSessionIdentities.value.length === 0) return false;
  const allIdentities = new Set<string>();
  for (const p of filteredProjects.value) {
    for (const s of p.sessions) {
      if (isCliId(s.cli_id)) {
        allIdentities.add(sessionIdentityKey({ cliId: s.cli_id, filePath: s.file_path }));
      }
    }
  }
  if (allIdentities.size === 0) return false;
  const selected = new Set(
    selectedSessionIdentities.value.map((item) => sessionIdentityKey(item)),
  );
  return (
    allIdentities.size === selected.size &&
    [...allIdentities].every((key) => selected.has(key))
  );
});

async function batchDeleteSessions(): Promise<boolean> {
  const count = selectedSessionIdentities.value.length;
  if (count === 0) return false;

  const confirmed = await ask(
    t("dialogs.deleteConfirm.batchMessage", { count }),
    { title: t("dialogs.deleteConfirm.batchTitle"), kind: "warning" }
  );
  if (!confirmed) return false;

  deleteProgress.value = { done: 0, total: count, currentPath: "" };

  try {
    await ensureBatchDeleteProgressListener();
    const attempted = [...selectedSessionIdentities.value];
    const outcome = await deleteIdentityGroups(attempted);
    deleteProgress.value = null;

    selectedSessionIdentities.value = attempted.filter(
      (item) => !outcome.succeededKeys.has(sessionIdentityKey(item)),
    );

    // 内存级瞬时剔除所有成功删除的会话项
    const succeededIdentities = attempted.filter((item) =>
      outcome.succeededKeys.has(sessionIdentityKey(item))
    );
    removeSessionIdentitiesFromMemory(succeededIdentities);

    if (
      activeSessionIdentity.value
      && outcome.succeededKeys.has(sessionIdentityKey(activeSessionIdentity.value))
    ) {
      clearActiveSession();
    }
    // 后台静默校验磁盘最新状态
    void refresh("snapshot");
    if (outcome.failed.length > 0) {
      await message(
        t("session.dialog.deletePartial", {
          succeeded: outcome.succeeded,
          failed: outcome.failed.length,
          reasons: renderDeleteReasons(outcome.failed),
        }),
        { title: t("session.dialog.deleteCompletedTitle"), kind: "warning" }
      );
      return false;
    }
    return true;
  } catch (e) {
    deleteProgress.value = null;
    await message(renderAppError(e), { title: t("app.dialog.error"), kind: "error" });
    return false;
  }
}

async function batchExportSessions(
  mode: "merged" | "separate",
  format: "txt" | "markdown" | "json" | "jsonl" = "txt"
) {
  const count = selectedSessionIdentities.value.length;
  if (count === 0) return;

  const extMap = { txt: "txt", markdown: "md", json: "json", jsonl: "jsonl" } as const;
  const nameMap = { txt: "Text", markdown: "Markdown", json: "JSON", jsonl: "JSON Lines" } as const;
  const ext = extMap[format];

  const defaultPath = `batch_export.${ext}`;

  const savePath = await save({
    filters: [{ name: nameMap[format], extensions: [ext] }],
    defaultPath,
  });
  if (!savePath) return;

  try {
    const outcome = await invokeApp<ExportOutcome>("batch_export_sessions", {
      sessions: [...selectedSessionIdentities.value],
      savePath,
      format,
      mode,
    });
    await message(exportOutcomeText(outcome), { title: t("app.batch.exportTooltip"), kind: "info" });
  } catch (e) {
    await message(renderAppError(e), { title: t("app.dialog.error"), kind: "error" });
  }
}

export function useSessions() {
  ensureSessionIndexListener();
  ensureSearchIndexListener();
  return {
    projects,
    searchQuery,
    activeSessionIdentity,
    selectedSessionPath,
    selectedProjectDir,
    filteredProjects,
    totalSessionCount,
    skipPermissions,
    setSkipPermissions,
    cliFilter,
    setCliFilter,
    saveCliFilter,
    setSessionUpdatesEnabled,
    visibleCliIds,
    launchCliId,
    recordSuccessfulLaunch,
    currentCliId,
    currentCli,
    cliStatuses,
    cliSessionCounts,
    cliPathConfigs,
    cliOptions,
    cliBinaryStatuses,
    installedCliOptions,
    setCurrentCli,
    loadCliStatuses,
    loadCliPathConfigs,
    setCliDataDirOverride,
    sortMode,
    setSortMode,
    globalSearchQuery,
    globalSearchResults,
    globalSearchLoading,
    searchIndexProgress,
    searchIndexBuildRequest,
    searchIndexBuildSkipped,
    refresh,
    clearActiveSession,
    setActiveSession,
    ensureSessionVisible,
    deleteSession,
    deleteProject,
    removeSessionIdentitiesFromMemory,
    exportSession,
    newSession,
    resumeSession,
    forkSession,
    ensureSessionReadyOnDisk,
    ensureCurrentCliInstalled,
    registerContextMenu,
    unregisterContextMenu,
    globalSearch,
    searchPendingCliIds,
    searchStaleCliIds,
    scanCliErrors,
    searchCliErrors,
    retryCliScan,
    buildingCliIds,
    startSearchIndexBuild,
    buildPendingSearchIndex,
    buildAllPendingSearchIndexes,
    dismissSearchIndexBuild,
    resetSearchIndexBuildState,
    selectedSessionIdentities,
    toggleSessionSelect,
    clearSessionSelection,
    selectAllSessions,
    isAllSelected,
    selectProjectSessions,
    isProjectAllSelected,
    batchDeleteSessions,
    batchDeleteProgressText,
    deleteProgress,
    batchExportSessions,
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
    autoFollow,
    setAutoFollow,
  };
}

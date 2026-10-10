import { nextTick, ref, shallowRef } from "vue";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { SessionIdentity, SessionInfo } from "../types/session";
import { setLocale, t } from "../i18n";
import { SUPPORTED_LOCALES } from "../i18n/types";

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  listen: vi.fn(),
  ask: vi.fn(async () => true),
  message: vi.fn(async () => undefined),
  save: vi.fn(async () => "/tmp/export.txt"),
  listeners: new Map<string, (event: { payload?: any }) => unknown>(),
  deleteFailures: new Set<string>(),
  projectRefresh: vi.fn(async () => undefined),
  searchRefresh: vi.fn(async () => undefined),
  projectItems: undefined as ReturnType<typeof shallowRef<any[]>> | undefined,
  projectDone: undefined as ReturnType<typeof ref<any>> | undefined,
  searchDone: undefined as ReturnType<typeof ref<any>> | undefined,
}));

const storage = new Map<string, string>();
vi.stubGlobal("localStorage", {
  getItem: (key: string) => storage.get(key) ?? null,
  setItem: (key: string, value: string) => storage.set(key, value),
  removeItem: (key: string) => storage.delete(key),
  clear: () => storage.clear(),
  key: (index: number) => [...storage.keys()][index] ?? null,
  get length() { return storage.size; },
});

vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: mocks.listen.mockImplementation(async (topic, callback) => {
    mocks.listeners.set(topic, callback);
    return () => {};
  }),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({
  ask: mocks.ask,
  message: mocks.message,
  open: vi.fn(),
  save: mocks.save,
}));
vi.mock("./useStreamingCollection", () => ({
  useStreamingCollection: ({ command }: { command: string }) => {
    const items = shallowRef<any[]>([]);
    const done = ref(command === "scan_projects" ? { total_sessions: 0, cli_results: [] } : null);
    const stream = {
      items,
      done,
      error: ref(null),
      isRefreshing: ref(false),
      showLoadingIndicator: ref(false),
      totalItems: ref(0),
      refresh: command === "scan_projects" ? mocks.projectRefresh : mocks.searchRefresh,
      cancel: vi.fn(),
    };
    if (command === "scan_projects") {
      mocks.projectItems = items;
      mocks.projectDone = done;
    }
    if (command === "search_sessions") {
      mocks.searchDone = done;
    }
    return stream;
  },
}));

const {
  useSessions,
  EXPORT_OUTCOME_KEY,
  CONTEXT_MENU_REGISTERED_KEY,
  CONTEXT_MENU_OUTCOME_KEY,
} = await import("./useSessions");
const { mount } = await import("@vue/test-utils");
const DataIndexSettings = (await import("../components/DataIndexSettings.vue")).default;
const store = useSessions();

function session(cliId: "claude" | "codex", filePath = "/same/session.jsonl"): SessionInfo {
  return {
    session_id: `${cliId}-session`,
    file_path: filePath,
    display_name: cliId,
    timestamp: "2026-08-27T08:00:00Z",
    file_size: 1,
    git_branch: "main",
    has_archive_snapshot: false,
    is_archived: false,
    cli_id: cliId,
  };
}

const activeCodexIdentity: SessionIdentity = {
  cliId: "codex",
  filePath: "/same/session.jsonl",
};

function activateCodexSession() {
  const identityStore = store as typeof store & {
    activeSessionIdentity?: ReturnType<typeof ref<SessionIdentity | null>>;
    setActiveSession?: (identity: SessionIdentity, projectDir: string) => void;
  };
  expect(identityStore.setActiveSession).toBeTypeOf("function");
  expect(identityStore.activeSessionIdentity).toBeDefined();
  identityStore.setActiveSession!(activeCodexIdentity, "codex");
  store.autoFollow.value = true;
  return identityStore;
}

beforeEach(() => {
  vi.useRealTimers();
  mocks.invoke.mockReset();
  mocks.ask.mockClear();
  mocks.message.mockClear();
  mocks.save.mockClear();
  mocks.projectRefresh.mockClear();
  mocks.searchRefresh.mockClear();
  mocks.deleteFailures.clear();
  mocks.projectItems!.value = [
    { encoded_dir: "claude", original_path: "/workspace", session: session("claude") },
    { encoded_dir: "codex", original_path: "/workspace", session: session("codex") },
  ];
  mocks.projectDone!.value = {
    total_sessions: 2,
    cli_results: [
      { cli_id: "claude", total_sessions: 1, error: null },
      { cli_id: "codex", total_sessions: 1, error: null },
    ],
  };
  store.cliStatuses.value = {
    claude: { hasSessions: true, hasBinary: true },
    codex: { hasSessions: true, hasBinary: true },
    gemini: { hasSessions: false, hasBinary: false },
    workbuddy: { hasSessions: false, hasBinary: false },
    dsh: { hasSessions: false, hasBinary: false },
    antigravity: { hasSessions: false, hasBinary: false },
  };
  store.cliFilter.value = { mode: "custom", cliIds: ["claude"] };
  store.selectedSessionIdentities.value = [];
  store.clearActiveSession();
  store.searchQuery.value = "";
  store.showArchivedSessions.value = true;
  store.showSnapshotSessions.value = true;
  storage.clear();

  mocks.invoke.mockImplementation(async (command: string, args?: Record<string, any>) => {
    if (command === "list_cli_statuses") {
      return [
        { id: "claude", has_sessions: true, has_binary: true },
        { id: "codex", has_sessions: true, has_binary: true },
      ];
    }
    if (command === "list_cli_path_configs") return [];
    if (command === "delete_sessions_to_trash") {
      if (mocks.deleteFailures.has(args?.cliId)) throw new Error(`${args?.cliId} failed`);
      return { failed: [], succeeded: args?.paths.length ?? 0 };
    }
    // 后端 B 类改造后导出回结构化结果（不是渲染好的句子），前端按 kind 取语言包键。
    if (command === "batch_export_sessions") {
      return {
        kind: args?.mode === "separate" ? "separate" : "merged",
        count: args?.sessions.length ?? 0,
      };
    }
    if (command === "register_context_menu") return { kind: "registered" };
    if (command === "unregister_context_menu") return { kind: "unregistered" };
    if (command === "fork_session") return "forked-session";
    if (command === "detect_cli") return true;
    return undefined;
  });
});

describe("useSessions store wiring", () => {
  it("offers visibility recovery only when hidden sessions match the current list search", () => {
    const archived = { ...session("claude", "/archive/old.jsonl"), is_archived: true };
    mocks.projectItems!.value = [
      { encoded_dir: "archive", original_path: "/archive", session: archived },
    ];
    store.setShowArchivedSessions(false);

    expect(store.filteredProjects.value).toEqual([]);
    expect(store.hasSessionsHiddenByVisibility.value).toBe(true);

    store.searchQuery.value = "unrelated";
    expect(store.hasSessionsHiddenByVisibility.value).toBe(false);

    store.searchQuery.value = "archive";
    expect(store.hasSessionsHiddenByVisibility.value).toBe(true);
  });

  it("changes history visibility immediately from the two settings switches", async () => {
    const archived = { ...session("claude", "/archive/old.jsonl"), is_archived: true };
    const snapshot = { ...session("claude", "/workspace/snapshot.jsonl"), has_archive_snapshot: true };
    mocks.projectItems!.value = [
      { encoded_dir: "archive", original_path: "/archive", session: archived },
      { encoded_dir: "workspace", original_path: "/workspace", session: snapshot },
    ];
    const wrapper = mount(DataIndexSettings, {
      global: {
        stubs: {
          IndexSettings: true,
          ArchiveRetentionSettings: true,
          ArchivedSessionsSettings: true,
          CacheMaintenanceSettings: true,
          BlockedFoldersSettings: true,
        },
      },
    });

    const archivedSwitch = wrapper.get(`[role="switch"][aria-label="${t("settings.dataIndex.showArchivedSessions")}"]`);
    const snapshotSwitch = wrapper.get(`[role="switch"][aria-label="${t("settings.dataIndex.showSnapshotSessions")}"]`);
    await archivedSwitch.trigger("click");
    expect(store.filteredProjects.value.map((p) => p.original_path)).toEqual(["/workspace"]);
    await snapshotSwitch.trigger("click");
    expect(store.filteredProjects.value).toEqual([]);
    expect(localStorage.getItem("seshbuddy-show-archived-sessions")).toBe("false");
    expect(localStorage.getItem("seshbuddy-show-snapshot-sessions")).toBe("false");
  });

  it("independently hides archived and snapshot sessions from the history list", () => {
    const live = session("claude", "/workspace/live.jsonl");
    const snapshot = { ...session("claude", "/workspace/snapshot.jsonl"), has_archive_snapshot: true };
    const archived = { ...session("codex", "/archive/old.jsonl"), has_archive_snapshot: true, is_archived: true };
    mocks.projectItems!.value = [
      { encoded_dir: "workspace", original_path: "/workspace", session: live },
      { encoded_dir: "workspace", original_path: "/workspace", session: snapshot },
      { encoded_dir: "archive", original_path: "/archive", session: archived },
    ];

    expect(store.filteredProjects.value.flatMap((p) => p.sessions.map((s) => s.file_path))).toEqual([
      "/workspace/live.jsonl", "/workspace/snapshot.jsonl", "/archive/old.jsonl",
    ]);

    store.setShowArchivedSessions(false);
    expect(store.filteredProjects.value.map((p) => p.original_path)).toEqual(["/workspace"]);
    expect(store.filteredProjects.value[0].sessions.map((s) => s.file_path)).toEqual([
      "/workspace/live.jsonl", "/workspace/snapshot.jsonl",
    ]);

    store.setShowSnapshotSessions(false);
    expect(store.filteredProjects.value[0].sessions.map((s) => s.file_path)).toEqual([
      "/workspace/live.jsonl",
    ]);

    store.setShowArchivedSessions(true);
    expect(store.filteredProjects.value.map((p) => p.original_path)).toEqual(["/workspace", "/archive"]);
    expect(store.filteredProjects.value[1].sessions.map((s) => s.file_path)).toEqual(["/archive/old.jsonl"]);
  });

  it("persists list visibility and clears batch selection when hiding sessions", () => {
    store.selectedSessionIdentities.value = [{ cliId: "claude", filePath: "/same/session.jsonl" }];

    store.setShowArchivedSessions(false);
    store.setShowSnapshotSessions(false);

    expect(store.selectedSessionIdentities.value).toEqual([]);
    expect(localStorage.getItem("seshbuddy-show-archived-sessions")).toBe("false");
    expect(localStorage.getItem("seshbuddy-show-snapshot-sessions")).toBe("false");
  });

  it("passes visible CLI IDs to project refresh and ignores hidden CLI update events", async () => {
    await store.refresh("snapshot");
    expect(mocks.projectRefresh).toHaveBeenLastCalledWith(
      { cliIds: ["claude"] },
      { silent: true, atomicSwap: true },
    );

    mocks.projectRefresh.mockClear();
    await mocks.listeners.get("session-list-index-updated")?.({ payload: { cliId: "codex" } });
    expect(mocks.projectRefresh).not.toHaveBeenCalled();

    await mocks.listeners.get("session-list-index-updated")?.({ payload: { cliId: "claude" } });
    expect(mocks.projectRefresh).toHaveBeenCalledTimes(1);
  });

  it("refreshes a selected source when its first session appears", async () => {
    store.cliFilter.value = { mode: "custom", cliIds: ["claude", "gemini"] };
    mocks.invoke.mockImplementation(async (command: string) => {
      if (command === "list_cli_statuses") return [
        { id: "claude", has_sessions: true, has_binary: true },
        { id: "gemini", has_sessions: true, has_binary: true },
      ];
      if (command === "list_cli_path_configs") return [];
      return undefined;
    });

    await mocks.listeners.get("session-list-index-updated")?.({ payload: { cliId: "gemini" } });

    expect(mocks.projectRefresh).toHaveBeenCalledWith(
      { cliIds: ["claude", "gemini"] },
      { silent: true, atomicSwap: true },
    );
  });

  it("does not start a watcher-driven scan while onboarding is active", async () => {
    store.setSessionUpdatesEnabled(false);
    try {
      await mocks.listeners.get("session-list-index-updated")?.({ payload: { cliId: "claude" } });
      expect(mocks.projectRefresh).not.toHaveBeenCalled();
    } finally {
      store.setSessionUpdatesEnabled(true);
    }
  });

  it("preserves counts for hidden CLI sources while refreshing the visible tree", async () => {
    store.cliSessionCounts.value = { claude: 3, codex: 8 };
    mocks.projectDone!.value = {
      total_sessions: 2,
      cli_results: [
        { cli_id: "claude", total_sessions: 2, error: null },
      ],
    };

    await store.refresh("snapshot");

    expect(store.cliSessionCounts.value).toEqual({ claude: 2, codex: 8 });
  });

  it("does not issue a supplemental title search outside the visible project data", async () => {
    vi.useFakeTimers();
    store.searchQuery.value = "hidden";
    await nextTick();
    await vi.advanceTimersByTimeAsync(350);

    expect(mocks.invoke.mock.calls.some(([command]) => command === "search_cross_cli_titles")).toBe(false);
  });

  it("uses the explicit session identity for deletion when paths collide", async () => {
    await store.deleteSession(
      { cliId: "codex", filePath: "/same/session.jsonl" },
      "Codex session",
    );

    expect(mocks.invoke).toHaveBeenCalledWith("delete_sessions_to_trash", {
      cliId: "codex",
      paths: ["/same/session.jsonl"],
    });
  });

  /// R-F：`BatchDeleteOutcome.failed` 的第二个元素是 `AppError` 的线格式
  /// （`Coded` 发 `{code, params}` 对象），渲染前必须过 `renderAppError`。
  /// 漏了这一步不会报错——`{error}` 占位符会把对象渲染成 `[object Object]` 上屏。
  it("renders a structured delete failure reason instead of [object Object]", async () => {
    mocks.invoke.mockImplementation(async (command: string) => {
      if (command === "delete_sessions_to_trash") {
        return {
          failed: [
            ["/gone.jsonl", { code: "system_ops.trash_failed", params: { detail: "boom" } }],
          ],
          succeeded: 0,
        };
      }
      return undefined;
    });

    await store.deleteSession({ cliId: "claude", filePath: "/gone.jsonl" }, "Claude session");

    const expected = t("session.dialog.deleteFailedMessage", {
      error: t("errors.system_ops.trash_failed", { detail: "boom" }),
    });
    expect(mocks.message).toHaveBeenCalledWith(expected, expect.anything());
    // 阳性对照的反面：候选文案确实带上了渲染后的句子，而不是对象本身。
    expect(String(mocks.message.mock.calls[0]?.[0])).not.toContain("[object Object]");
  });

  /// 同一个缺陷的**第二个**渲染点（`batchDeleteSessions` → `deletePartial` 的 `{reasons}`）。
  /// 单独立一条，因为它是被复核者抓到的：改完 `deleteSession` 与 `deleteProject` 之后，
  /// 这一处仍然原样插值，用户看到的是错误信封的 JSON dump。**三个渲染点都要有回归。**
  it("renders structured batch-delete reasons instead of [object Object]", async () => {
    store.selectedSessionIdentities.value = [{ cliId: "claude", filePath: "/gone/a.jsonl" }];
    mocks.invoke.mockImplementation(async (command: string) => {
      if (command === "delete_sessions_to_trash") {
        return {
          failed: [
            ["/gone/a.jsonl", { code: "system_ops.trash_failed", params: { detail: "boom" } }],
          ],
          succeeded: 0,
        };
      }
      return undefined;
    });

    await store.batchDeleteSessions();

    const expected = t("session.dialog.deletePartial", {
      succeeded: 0,
      failed: 1,
      reasons: t("errors.system_ops.trash_failed", { detail: "boom" }),
    });
    expect(mocks.message).toHaveBeenCalledWith(expected, expect.anything());
    expect(String(mocks.message.mock.calls[0]?.[0])).not.toContain("[object Object]");
  });

  it("keeps the active Codex session when deleting Claude at the same path", async () => {
    const identityStore = activateCodexSession();

    await store.deleteSession(
      { cliId: "claude", filePath: "/same/session.jsonl" },
      "Claude session",
    );

    expect(identityStore.activeSessionIdentity!.value).toEqual(activeCodexIdentity);
    expect(store.selectedSessionPath.value).toBe(activeCodexIdentity.filePath);
    expect(store.autoFollow.value).toBe(true);
  });

  it("keeps the active Codex session when deleting a Claude-only project with the same path", async () => {
    const identityStore = activateCodexSession();
    const claudeProject = {
      encoded_dir: "claude",
      original_path: "/workspace",
      sessions: [session("claude")],
    };

    await store.deleteProject(claudeProject);

    expect(identityStore.activeSessionIdentity!.value).toEqual(activeCodexIdentity);
    expect(store.selectedSessionPath.value).toBe(activeCodexIdentity.filePath);
    expect(store.autoFollow.value).toBe(true);
  });

  it("keeps the active Codex session when batch-deleting Claude at the same path", async () => {
    const identityStore = activateCodexSession();
    store.selectedSessionIdentities.value = [
      { cliId: "claude", filePath: "/same/session.jsonl" },
    ];

    await store.batchDeleteSessions();

    expect(identityStore.activeSessionIdentity!.value).toEqual(activeCodexIdentity);
    expect(store.selectedSessionPath.value).toBe(activeCodexIdentity.filePath);
    expect(store.autoFollow.value).toBe(true);
  });

  it("uses explicit CLI context for restore, resume, and fork operations", async () => {
    mocks.projectItems!.value = [
      {
        encoded_dir: "codex",
        original_path: "/workspace",
        session: { ...session("codex"), is_archived: true },
      },
    ];

    await store.ensureSessionReadyOnDisk("/same/session.jsonl", undefined, "codex");
    await store.resumeSession("/workspace", "codex-session", "codex");
    await store.forkSession(
      { cliId: "codex", filePath: "/same/session.jsonl" },
      "anchor",
    );

    expect(mocks.invoke).toHaveBeenCalledWith("restore_session_to_disk", {
      cliId: "codex",
      filePath: "/same/session.jsonl",
    });
    expect(mocks.invoke).toHaveBeenCalledWith("open_in_terminal", expect.objectContaining({
      cliId: "codex",
      sessionId: "codex-session",
    }));
    expect(mocks.invoke).toHaveBeenCalledWith("fork_session", {
      cliId: "codex",
      filePath: "/same/session.jsonl",
      anchorUuid: "anchor",
    });
  });

  it("preserves composite identities in the batch export request", async () => {
    const identities: SessionIdentity[] = [
      { cliId: "claude", filePath: "/same/session.jsonl" },
      { cliId: "codex", filePath: "/same/session.jsonl" },
    ];
    store.selectedSessionIdentities.value = identities;

    await store.batchExportSessions("merged", "json");

    expect(mocks.invoke).toHaveBeenCalledWith("batch_export_sessions", {
      sessions: identities,
      savePath: "/tmp/export.txt",
      format: "json",
      mode: "merged",
    });
  });

  // 后端只回 `{ kind, count }`，上屏的句子必须由前端语言包渲染。
  // 断言用基准语言 en（test-setup 已固定），故钉的是**渲染链**不是译文：
  // 把 message 改回直接显示后端值、或键配错、或 count 没插进去，都会变红。
  it("renders the merged export notice from the language pack", async () => {
    store.selectedSessionIdentities.value = [
      { cliId: "claude", filePath: "/a.jsonl" },
      { cliId: "codex", filePath: "/b.jsonl" },
    ];

    await store.batchExportSessions("merged", "json");

    expect(mocks.message.mock.calls.at(-1)?.[0]).toBe("Merged 2 sessions into one file.");
  });

  it("renders the separate export notice from the language pack", async () => {
    store.selectedSessionIdentities.value = [
      { cliId: "claude", filePath: "/a.jsonl" },
      { cliId: "codex", filePath: "/b.jsonl" },
    ];

    await store.batchExportSessions("separate", "json");

    expect(mocks.message.mock.calls.at(-1)?.[0]).toBe("Exported 2 sessions separately.");
  });

  // 右键菜单同形：后端只回动作，措辞（含平台分叉）由前端语言包渲染。
  // jsdom 的 userAgent 不是 Windows，故这里覆盖的是 macOS 措辞那支。
  it("renders the context menu notices from the language pack", async () => {
    await store.registerContextMenu("claude");
    expect(mocks.message.mock.calls.at(-1)?.[0]).toBe(
      "Context menu registered. Right-click a folder in Finder and look for Claude Code under Quick Actions or Services."
    );

    await store.unregisterContextMenu();
    expect(mocks.message.mock.calls.at(-1)?.[0]).toBe("Context menu unregistered.");
  });

  /**
   * 两个 `Record` 的**全部取值** × 全部语言都要解析得到。
   *
   * 上面三条用例只走到 `merged` / `separate` / `registered`(macOS) / `unregistered`；
   * `single` 与 `registeredWindows` 到不了（前者 `exportSession` 无人调用，
   * 后者 jsdom 的 UA 不是 Windows）。而这两个键是 `Record` 对象字面量里的**字符串取值**：
   * 闸门规则 5 只认直接的 `t("…")` 字面量（`findTranslationKeyRefs`），规则 6 只扫
   * `src/changelog.ts` 的 `*Keys` 数组——**两份静态检查都看不见 `Record` 的取值**。
   *
   * 判据用 vue-i18n 自己的缺键行为：`missingWarn: false` 时它**原样返回键名**，
   * 故「渲染结果 ≠ 键名」等价于「键在（回退链能到的）语言包里存在」。
   * 遍历取值而不是逐条断言站点，是为了关掉**这一类**：将来往任一个 `Record` 里加一行，
   * 本用例自动覆盖。
   *
   * `CONTEXT_MENU_REGISTERED_KEY` 必须一起遍历：平台选择在模块加载时就落定，
   * 只遍历 `CONTEXT_MENU_OUTCOME_KEY` 会漏掉**另一个平台**那条（本机是 macOS 时漏
   * `registeredWindows`，反之漏 `registeredMacos`）。
   */
  it("resolves every outcome key in every supported language", () => {
    const keys = new Set([
      ...Object.values(EXPORT_OUTCOME_KEY),
      ...Object.values(CONTEXT_MENU_REGISTERED_KEY),
      ...Object.values(CONTEXT_MENU_OUTCOME_KEY),
    ]);

    // 阳性对照：判据在缺键时确实报红，证明下面那轮不是恒真
    expect(t("app.batch.exportOutcome.__missing__")).toBe("app.batch.exportOutcome.__missing__");
    // 阳性对照：确实取到了本任务新增的全部 6 个包键，而不是遍历了一个空集合
    expect(keys.size).toBe(6);

    try {
      for (const locale of SUPPORTED_LOCALES) {
        setLocale(locale);
        for (const key of keys) {
          // `count` 是 merged/separate 的占位符，对无占位符的键无副作用
          expect(t(key, { count: 1 }), `${locale} 下 ${key} 未解析`).not.toBe(key);
        }
      }
    } finally {
      setLocale("en");
    }
  });

  it("continues after one CLI deletion rejects, refreshes, and retains only failed identities", async () => {
    const identities: SessionIdentity[] = [
      { cliId: "claude", filePath: "/claude/a.jsonl" },
      { cliId: "codex", filePath: "/codex/b.jsonl" },
      { cliId: "gemini", filePath: "/gemini/c.jsonl" },
    ];
    store.selectedSessionIdentities.value = identities;
    mocks.deleteFailures.add("codex");

    const allSucceeded = await store.batchDeleteSessions();

    const deleteCalls = mocks.invoke.mock.calls.filter(
      ([command]) => command === "delete_sessions_to_trash",
    );
    expect(deleteCalls.map(([, args]) => args.cliId)).toEqual(["claude", "codex", "gemini"]);
    expect(mocks.projectRefresh).toHaveBeenCalled();
    expect(store.selectedSessionIdentities.value).toEqual([identities[1]]);
    expect(allSucceeded).toBe(false);
  });

  it("continues deleting the other project sources and refreshes after a partial failure", async () => {
    store.cliFilter.value = { mode: "all" };
    mocks.deleteFailures.add("claude");

    const allSucceeded = await store.deleteProject(store.projects.value[0]);

    const deleteCalls = mocks.invoke.mock.calls.filter(
      ([command]) => command === "delete_sessions_to_trash",
    );
    expect(deleteCalls.map(([, args]) => args.cliId)).toEqual(["claude", "codex"]);
    expect(mocks.projectRefresh).toHaveBeenCalled();
    expect(allSucceeded).toBe(false);
  });

  it("exposes per-CLI scan errors and clears them once that CLI scans successfully", async () => {
    mocks.projectDone!.value = {
      total_sessions: 1,
      cli_results: [
        // 存的是**结构化错误**不是渲染好的文案：渲染结果随语言变，存进 ref 就会把语言
        // 冻结在事件发生的那一刻。渲染由消费点（DashboardView / GlobalSearch / HistoryPanel）
        // 各自做。读错字段（把 `error` 当字符串拼、或去读旧的 `message`）会立刻失败。
        {
          cli_id: "claude",
          total_sessions: 0,
          error: { code: "session.dir_not_found", params: { path: "/gone" } },
        },
        // `Business` 变体仍是字符串，原样透出、不查语言包
        { cli_id: "codex", total_sessions: 0, error: "legacy plain message" },
      ],
    };

    await store.refresh("snapshot");
    expect(store.scanCliErrors.value).toEqual({
      claude: { code: "session.dir_not_found", params: { path: "/gone" } },
      codex: "legacy plain message",
    });

    mocks.projectDone!.value = {
      total_sessions: 1,
      cli_results: [
        { cli_id: "claude", total_sessions: 1, error: null },
        { cli_id: "codex", total_sessions: 1, error: null },
      ],
    };

    await store.refresh("snapshot");
    expect(store.scanCliErrors.value).toEqual({});
  });

  it("keeps scan errors of CLIs absent from the latest scan results", async () => {
    store.scanCliErrors.value = { codex: "旧错误" };
    mocks.projectDone!.value = {
      total_sessions: 1,
      cli_results: [
        { cli_id: "claude", total_sessions: 1, error: null },
      ],
    };

    await store.refresh("snapshot");

    expect(store.scanCliErrors.value).toEqual({ codex: "旧错误" });
  });

  it("retryCliScan forces an index rebuild for the CLI and refreshes", async () => {
    await store.retryCliScan("claude");

    expect(mocks.invoke).toHaveBeenCalledWith("refresh_session_list_index", {
      cliId: "claude",
      notify: true,
      force: true,
    });
    expect(mocks.projectRefresh).toHaveBeenCalled();
  });

  it("surfaces per-CLI search errors from the search done payload and clears them on the next clean run", async () => {
    mocks.searchDone!.value = {
      total: 0,
      query: "q",
      pending_cli_ids: [],
      stale_cli_ids: [],
      // 与 scan 通道同形：`{code, params}`，**原样存**（渲染交给消费点）
      cli_errors: [
        {
          cli_id: "codex",
          error: { code: "session.load_failed", params: { path: "/p.jsonl" } },
        },
      ],
    };
    await nextTick();
    expect(store.searchCliErrors.value).toEqual({
      codex: { code: "session.load_failed", params: { path: "/p.jsonl" } },
    });

    mocks.searchDone!.value = {
      total: 1,
      query: "q",
      pending_cli_ids: [],
      stale_cli_ids: [],
      cli_errors: [],
    };
    await nextTick();
    expect(store.searchCliErrors.value).toEqual({});
  });

  it("optimistically removes deleted session from memory and updates session counts immediately", async () => {
    const claudeItem = {
      session: {
        session_id: "claude-1",
        file_path: "/workspace/claude-1.jsonl",
        display_name: "Claude 会话 1",
        timestamp: "2026-09-17T08:00:00Z",
        file_size: 100,
        git_branch: "main",
        cli_id: "claude",
      },
      encoded_dir: "workspace",
      original_path: "/workspace",
    };
    const codexItem = {
      session: {
        session_id: "codex-1",
        file_path: "/workspace/codex-1.jsonl",
        display_name: "Codex 会话 1",
        timestamp: "2026-09-17T08:00:00Z",
        file_size: 100,
        git_branch: "main",
        cli_id: "codex",
      },
      encoded_dir: "workspace",
      original_path: "/workspace",
    };
    mocks.projectItems!.value = [claudeItem, codexItem];
    mocks.projectDone!.value = {
      total_sessions: 1,
      cli_results: [
        { cli_id: "claude", total_sessions: 0, error: null },
        { cli_id: "codex", total_sessions: 1, error: null },
      ],
    };
    store.totalSessionCount.value = 2;
    store.cliSessionCounts.value = { claude: 1, codex: 1 };

    const deleted = await store.deleteSession(
      { cliId: "claude", filePath: "/workspace/claude-1.jsonl" },
      "Claude 会话 1",
    );

    expect(deleted).toBe(true);
    // 内存瞬时剔除，无需等待全盘扫描
    expect(mocks.projectItems!.value).toEqual([codexItem]);
    expect(store.totalSessionCount.value).toBe(1);
    expect(store.cliSessionCounts.value.claude).toBe(0);
    expect(store.cliSessionCounts.value.codex).toBe(1);
    // 静默后台同步被触发
    expect(mocks.projectRefresh).toHaveBeenCalled();
  });

  it("batchDeleteSessions optimistically removes only succeeded sessions from memory", async () => {
    const claudeItem = {
      session: {
        session_id: "claude-1",
        file_path: "/claude/success.jsonl",
        display_name: "Claude 成功会话",
        timestamp: "2026-09-17T08:00:00Z",
        file_size: 100,
        git_branch: "main",
        cli_id: "claude",
      },
      encoded_dir: "workspace",
      original_path: "/workspace",
    };
    const codexItem = {
      session: {
        session_id: "codex-1",
        file_path: "/codex/fail.jsonl",
        display_name: "Codex 失败会话",
        timestamp: "2026-09-17T08:00:00Z",
        file_size: 100,
        git_branch: "main",
        cli_id: "codex",
      },
      encoded_dir: "workspace",
      original_path: "/workspace",
    };
    mocks.projectItems!.value = [claudeItem, codexItem];
    mocks.projectDone!.value = {
      total_sessions: 1,
      cli_results: [
        { cli_id: "claude", total_sessions: 0, error: null },
        { cli_id: "codex", total_sessions: 1, error: null },
      ],
    };
    store.totalSessionCount.value = 2;
    store.cliSessionCounts.value = { claude: 1, codex: 1 };

    store.selectedSessionIdentities.value = [
      { cliId: "claude", filePath: "/claude/success.jsonl" },
      { cliId: "codex", filePath: "/codex/fail.jsonl" },
    ];
    mocks.deleteFailures.add("codex");

    const allSucceeded = await store.batchDeleteSessions();

    expect(allSucceeded).toBe(false);
    // 仅成功项被从内存剔除，失败项保留
    expect(mocks.projectItems!.value).toEqual([codexItem]);
    expect(store.totalSessionCount.value).toBe(1);
    expect(store.cliSessionCounts.value.claude).toBe(0);
    expect(store.cliSessionCounts.value.codex).toBe(1);
  });

  describe("索引构建请求承载一组 CLI", () => {
    const confirmable = (missingSessions: number, totalSessions: number) => ({
      ready: false,
      requiresConfirmation: true,
      missingSessions,
      totalSessions,
      missingBytes: 10,
      totalBytes: 100,
      physicalIndexReady: false,
    });

    it("「一键构建全部」把每个待确认的 CLI 都累积进同一个请求", async () => {
      mocks.invoke.mockImplementation(async (command: string, args?: Record<string, any>) => {
        if (command === "get_search_index_status") {
          return args?.cliId === "claude" ? confirmable(3, 3) : confirmable(2, 9);
        }
        return undefined;
      });

      store.resetSearchIndexBuildState();
      await store.buildAllPendingSearchIndexes(["claude", "codex"]);

      // 判据是**条数**：请求若是单槽，这里只剩最后入队的那个 CLI，
      // 用户看到「一键全部」却只被问了最后一个，其余静默丢失、得反复点。
      expect(store.searchIndexBuildRequest.value?.items.map((it) => it.cliId)).toEqual([
        "claude",
        "codex",
      ]);
    });

    it("同一个 CLI 重复入队只记一项", async () => {
      mocks.invoke.mockImplementation(async (command: string) => {
        if (command === "get_search_index_status") return confirmable(3, 3);
        return undefined;
      });

      store.resetSearchIndexBuildState();
      await store.buildPendingSearchIndex("claude");
      await store.buildPendingSearchIndex("claude");

      expect(store.searchIndexBuildRequest.value?.items).toHaveLength(1);
    });

    it("确认后把请求里的每个 CLI 都建了，且单个失败不中断其余", async () => {
      const ensured: string[] = [];
      mocks.invoke.mockImplementation(async (command: string, args?: Record<string, any>) => {
        if (command === "get_search_index_status") return confirmable(3, 3);
        if (command === "ensure_search_index_ready") {
          ensured.push(args?.cliId);
          if (args?.cliId === "claude") throw new Error("boom");
        }
        return undefined;
      });

      store.resetSearchIndexBuildState();
      await store.buildAllPendingSearchIndexes(["claude", "codex"]);
      await store.startSearchIndexBuild();

      // 两个都要被尝试。只建最后一个（旧实现）或第一个失败就中断（未做失败隔离）都会红。
      expect(ensured).toEqual(["claude", "codex"]);
    });
  });
});

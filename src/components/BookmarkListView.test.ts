import { ref } from "vue";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { CliId, CliOption } from "../types/cli";
import { CLI_DEFINITIONS } from "../types/cli";

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  visibleCliIds: undefined as ReturnType<typeof ref<CliId[]>> | undefined,
  failures: new Set<CliId>(),
}));

vi.stubGlobal("localStorage", {
  getItem: () => null,
  setItem: () => {},
  removeItem: () => {},
  clear: () => {},
  key: () => null,
  length: 0,
});

vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("../composables/useSessions", async () => {
  const { ref } = await vi.importActual<typeof import("vue")>("vue");
  mocks.visibleCliIds = ref<CliId[]>(["claude", "codex"]);
  const cliOptions: CliOption[] = (["claude", "codex"] as CliId[]).map((id) => ({
    ...CLI_DEFINITIONS[id],
    hasSessions: true,
    hasBinary: true,
  }));
  return {
    useSessions: () => ({
      visibleCliIds: mocks.visibleCliIds,
      cliOptions: ref(cliOptions),
      projects: ref([]),
    }),
  };
});

const { mount, flushPromises } = await import("@vue/test-utils");
const BookmarkListView = (await import("./BookmarkListView.vue")).default;

function bookmark(cliId: CliId, sessionId: string, messageIndex = 0) {
  return {
    cliId,
    sessionId,
    sessionPath: `/projects/shared/${sessionId}.jsonl`,
    sessionDisplayName: `${cliId}-${sessionId}`,
    messageIndex,
    messagePreview: `preview-${cliId}-${sessionId}`,
    createdAt: "2026-08-27T08:00:00Z",
    sourceDeleted: false,
  };
}

const results: Record<string, any[]> = {
  claude: [bookmark("claude", "a")],
  codex: [bookmark("codex", "b")],
};

beforeEach(() => {
  mocks.invoke.mockReset();
  mocks.failures.clear();
  mocks.visibleCliIds!.value = ["claude", "codex"];
  results.claude = [bookmark("claude", "a")];
  results.codex = [bookmark("codex", "b")];
  mocks.invoke.mockImplementation(async (command: string, args?: Record<string, any>) => {
    if (command === "list_bookmarks_with_context") {
      const cliId = args?.cliId as CliId;
      if (mocks.failures.has(cliId)) throw new Error(`${cliId} 读取失败`);
      return results[cliId] ?? [];
    }
    return undefined;
  });
});

function mountView() {
  return mount(BookmarkListView, {
    global: { stubs: { SvgIcon: true } },
  });
}

describe("BookmarkListView cross-CLI fan-out", () => {
  it("loads bookmarks for every visible CLI and merges them", async () => {
    const wrapper = mountView();
    await flushPromises();

    const calledCliIds = mocks.invoke.mock.calls
      .filter(([command]) => command === "list_bookmarks_with_context")
      .map(([, args]) => args.cliId);
    expect(calledCliIds.sort()).toEqual(["claude", "codex"]);

    const items = wrapper.findAll(".bookmark-item");
    expect(items).toHaveLength(2);
    expect(wrapper.text()).toContain("preview-claude-a");
    expect(wrapper.text()).toContain("preview-codex-b");
  });

  it("keeps the failed CLI's previous slice when reloading", async () => {
    const wrapper = mountView();
    await flushPromises();
    expect(wrapper.findAll(".bookmark-item")).toHaveLength(2);

    mocks.failures.add("codex");
    results.claude = [bookmark("claude", "a2")];

    await wrapper.vm.refresh();
    await flushPromises();

    expect(wrapper.text()).toContain("preview-claude-a2");
    expect(wrapper.text()).toContain("preview-codex-b");
    expect(wrapper.findAll(".bookmark-item")).toHaveLength(2);
  });

  it("tags bookmarks with their CLI name when multiple sources are present", async () => {
    const wrapper = mountView();
    await flushPromises();

    const tags = wrapper.findAll(".bookmark-cli-tag");
    expect(tags).toHaveLength(2);
    expect(tags.map((tag) => tag.text()).sort()).toEqual(["Claude Code", "Codex"]);
  });

  it("keeps groups from different CLIs separate when session paths collide", async () => {
    results.codex = [{ ...bookmark("codex", "a"), sessionPath: "/projects/shared/a.jsonl" }];

    const wrapper = mountView();
    await flushPromises();

    expect(wrapper.findAll(".bookmark-group")).toHaveLength(2);
  });
});

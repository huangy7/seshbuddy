import { nextTick, ref } from "vue";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { CLI_DEFINITIONS, type CliId, type CliOption } from "../types/cli";

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  visibleCliIds: undefined as ReturnType<typeof ref<CliId[]>> | undefined,
  currentCliId: undefined as ReturnType<typeof ref<CliId>> | undefined,
  launchCliId: undefined as ReturnType<typeof ref<CliId>> | undefined,
}));

const storage = new Map<string, string>();
if (typeof document !== "undefined" && !document.queryCommandSupported) {
  document.queryCommandSupported = () => false;
}
vi.stubGlobal("localStorage", {
  getItem: (key: string) => storage.get(key) ?? null,
  setItem: (key: string, value: string) => storage.set(key, value),
  removeItem: (key: string) => storage.delete(key),
  clear: () => storage.clear(),
  key: (index: number) => [...storage.keys()][index] ?? null,
  get length() { return storage.size; },
});
vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => {
  callback(0);
  return 0;
});

const cliOptions: CliOption[] = (Object.keys(CLI_DEFINITIONS) as CliId[]).map((id) => ({
  ...CLI_DEFINITIONS[id],
  hasSessions: id === "claude" || id === "codex",
  hasBinary: id === "claude" || id === "codex",
}));

vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("@tauri-apps/api/event", () => ({
  emit: vi.fn(),
  listen: vi.fn(async () => () => {}),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn(), message: vi.fn() }));
vi.mock("../composables/useSessions", async () => {
  const { ref } = await vi.importActual<typeof import("vue")>("vue");
  mocks.visibleCliIds = ref<CliId[]>(["claude"]);
  mocks.currentCliId = ref<CliId>("claude");
  mocks.launchCliId = ref<CliId>("codex");
  return {
    useSessions: () => ({
      projects: ref([]),
      currentCliId: mocks.currentCliId,
      currentCli: ref(CLI_DEFINITIONS.claude),
      launchCliId: mocks.launchCliId,
      visibleCliIds: mocks.visibleCliIds,
      cliOptions: ref(cliOptions),
      cliSessionCounts: ref({ claude: 1, codex: 1, gemini: 0, dsh: 0 }),
      skipPermissions: ref(true),
    }),
  };
});

const { mount } = await import("@vue/test-utils");
const { t } = await import("../i18n");
const UsageDashboard = (await import("./UsageDashboard.vue")).default;
const NewSessionDialog = (await import("./NewSessionDialog.vue")).default;
const ApiDebugDialog = (await import("./ApiDebugDialog.vue")).default;

beforeEach(() => {
  storage.clear();
  mocks.invoke.mockReset();
  mocks.invoke.mockResolvedValue([]);
  mocks.visibleCliIds!.value = ["claude"];
  mocks.currentCliId!.value = "claude";
  mocks.launchCliId!.value = "codex";
});

describe("feature CLI context", () => {
  it("switches the usage CLI locally without changing the history filter", async () => {
    const wrapper = mount(UsageDashboard, {
      global: { stubs: { SvgIcon: true, ChatAvatar: true } },
    });
    await nextTick();

    const chips = wrapper.findAll(".cli-chip[data-cli-id]");
    expect(chips.map((chip) => chip.attributes("data-cli-id"))).toEqual([
      "claude",
      "codex",
      "gemini",
      "dsh",
      "antigravity",
      "opencode",
    ]);

    // Click claude chip to deselect it locally
    await wrapper.get('[data-cli-id="claude"]').trigger("click");
    await nextTick();

    expect(mocks.visibleCliIds!.value).toEqual(["claude"]);
    expect(mocks.currentCliId!.value).toBe("claude");
  });

  it("initializes new conversations from launchCliId and emits that explicit CLI", async () => {
    const wrapper = mount(NewSessionDialog, {
      props: {
        initialProjectPath: "/workspace/project",
        initialCliId: mocks.launchCliId!.value,
        cliBinaryStatuses: { claude: true, codex: true },
      },
      global: {
        stubs: {
          SvgIcon: true,
          ElegantSelect: true,
        },
      },
    });

    expect(wrapper.get<HTMLInputElement>('input[type="radio"][value="codex"]').element.checked).toBe(true);
    await wrapper.get(".btn-create").trigger("click");

    expect(wrapper.emitted("create")?.[0]).toEqual([
      "/workspace/project",
      "codex",
      "agent",
      null,
      true,
    ]);
  });

  it("resolves an unsupported API debug entry before rendering its title and log page", () => {
    const wrapper = mount(ApiDebugDialog, {
      props: { initialCliId: "gemini" },
      global: {
        stubs: {
          SvgIcon: true,
          ApiLogView: true,
          Teleport: true,
        },
      },
    });

    expect(wrapper.get(".api-debug-title").text()).toContain(
      t("dialogs.apiDebug.titleWithCli", { name: "Claude Code" }),
    );
    expect(wrapper.findComponent({ name: "ApiLogView" }).props("initialCliId")).toBe("claude");
  });
});

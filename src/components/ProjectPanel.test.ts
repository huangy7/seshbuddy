import { describe, it, expect, vi, beforeEach } from "vitest";

// Mock Tauri invoke & dialog
vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn().mockImplementation(async (cmd: string, args: any) => {
    if (cmd === "list_project_files") {
      const p = args?.projectPath || "";
      if (p.includes("coredns")) {
        return [
          { name: "plugin", path: "/workspace/coredns/plugin", isDir: true, children: [] },
          { name: "main.go", path: "/workspace/coredns/main.go", isDir: false },
          { name: "go.mod", path: "/workspace/coredns/go.mod", isDir: false },
        ];
      }
      if (p.includes("seshbuddy")) {
        return [
          { name: "src", path: "/workspace/seshbuddy/src", isDir: true, children: [] },
          { name: "package.json", path: "/workspace/seshbuddy/package.json", isDir: false },
        ];
      }
      return [
        { name: "README.md", path: `${p}/README.md`, isDir: false },
      ];
    }
    return [];
  }),
}));

vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: vi.fn(),
  ask: vi.fn().mockResolvedValue(true),
}));

// Mock localStorage
vi.stubGlobal("localStorage", {
  getItem: () => null,
  setItem: () => {},
  removeItem: () => {},
  clear: () => {},
  key: () => null,
  length: 0,
});

const { mount } = await import("@vue/test-utils");
const ProjectPanel = (await import("./ProjectPanel.vue")).default;

const mockProjects = [
  { project_key: "coredns", original_path: "/workspace/coredns", session_count: 5, sessions: [] },
  { project_key: "seshbuddy", original_path: "/workspace/seshbuddy", session_count: 10, sessions: [] },
];

describe("ProjectPanel Single-Project Focus & Breadcrumb Navigation", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("directly renders the activeProjectPath root file tree", async () => {
    const wrapper = mount(ProjectPanel, {
      props: {
        projects: mockProjects,
        manualPaths: [],
        activeProjectPath: "/workspace/coredns",
      },
    });

    await wrapper.vm.$nextTick();
    await new Promise((r) => setTimeout(r, 50));

    const breadcrumb = wrapper.find(".project-breadcrumb-trigger");
    expect(breadcrumb.exists()).toBe(true);
    expect(breadcrumb.text()).toContain("coredns");

    const text = wrapper.text();
    expect(text).toContain("main.go");
    expect(text).toContain("go.mod");
    expect(text).toContain("plugin");
  });

  it("switches active project via breadcrumb dropdown picker", async () => {
    const wrapper = mount(ProjectPanel, {
      props: {
        projects: mockProjects,
        manualPaths: [],
        activeProjectPath: "/workspace/coredns",
      },
    });

    await wrapper.vm.$nextTick();
    await new Promise((r) => setTimeout(r, 50));

    const trigger = wrapper.find(".project-breadcrumb-trigger");
    expect(trigger.attributes("aria-expanded")).toBe("false");
    await trigger.trigger("click");
    expect(trigger.attributes("aria-expanded")).toBe("true");

    const dropdown = wrapper.find(".project-picker-menu");
    expect(dropdown.exists()).toBe(true);
    expect(dropdown.text()).toContain("seshbuddy");

    const items = wrapper.findAll(".project-picker-item");
    const seshbuddyItem = items.find((i) => i.text().includes("seshbuddy"));
    expect(seshbuddyItem).toBeDefined();
    await seshbuddyItem!.trigger("click");

    await wrapper.vm.$nextTick();
    await new Promise((r) => setTimeout(r, 50));

    expect(wrapper.find(".project-breadcrumb-trigger").text()).toContain("seshbuddy");
    expect(wrapper.text()).toContain("package.json");
  });

  it("emits openFile when clicking a file node", async () => {
    const wrapper = mount(ProjectPanel, {
      props: {
        projects: mockProjects,
        manualPaths: [],
        activeProjectPath: "/workspace/coredns",
      },
    });

    await wrapper.vm.$nextTick();
    await new Promise((r) => setTimeout(r, 50));

    const fileNode = wrapper.find(".tree-file");
    expect(fileNode.exists()).toBe(true);
    await fileNode.trigger("click");

    expect(wrapper.emitted("openFile")).toBeTruthy();
    expect(wrapper.emitted("openFile")?.[0]).toEqual(["/workspace/coredns/main.go", "/workspace/coredns"]);
  });
});
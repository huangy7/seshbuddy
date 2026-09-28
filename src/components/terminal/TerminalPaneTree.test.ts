import { describe, it, expect, vi } from "vitest";
import { mount } from "@vue/test-utils";
import TerminalPaneTree from "./TerminalPaneTree.vue";
import type { TerminalPaneSplit, TerminalPaneLeaf } from "../../types/terminal";

vi.mock("../AgentTerminal.vue", () => ({
  default: {
    name: "AgentTerminal",
    template: `<div class="agent-terminal-mock"></div>`,
    props: ["sessionId", "projectRoot"],
  },
}));

function createLeaf(id: string, cliKind = "shell"): TerminalPaneLeaf {
  return {
    kind: "leaf",
    id,
    sessionId: `sess-${id}`,
    cliKind,
    projectPath: "/my/project",
    createdAt: Date.now(),
  };
}

function create2Split(l1Id = "p1", l2Id = "p2"): TerminalPaneSplit {
  return {
    kind: "split",
    id: "s1",
    dir: "row",
    ratios: [0.5, 0.5],
    children: [createLeaf(l1Id, "claude"), createLeaf(l2Id, "shell")],
  };
}

describe("TerminalPaneTree.vue", () => {
  it("renders 2 split panes normally", () => {
    const split = create2Split();
    const wrapper = mount(TerminalPaneTree, {
      props: {
        node: split,
        activePaneId: "p1",
        totalLeafCount: 2,
        maximizedPaneId: null,
      },
    });

    const wraps = wrapper.findAll(".tree-child-wrap");
    expect(wraps).toHaveLength(2);
    expect(wraps[0].attributes("style")).not.toContain("display: none");
    expect(wraps[1].attributes("style")).not.toContain("display: none");
  });

  it("hides other panes and allocates 100% when maximizedPaneId is set", () => {
    const split = create2Split("p1", "p2");
    const wrapper = mount(TerminalPaneTree, {
      props: {
        node: split,
        activePaneId: "p2",
        totalLeafCount: 2,
        maximizedPaneId: "p2",
      },
    });

    const wraps = wrapper.findAll(".tree-child-wrap");
    expect(wraps).toHaveLength(2);
    // First child (p1) should be hidden
    expect(wraps[0].attributes("style")).toContain("display: none");
    // Second child (p2) should take 100%
    expect(wraps[1].attributes("style")).toContain("flex: 1 1 100%");
    expect(wraps[1].attributes("style")).not.toContain("display: none");
  });

  it("emits toggleMaximize when pane header is double clicked or button clicked", async () => {
    const split = create2Split("p1", "p2");
    const wrapper = mount(TerminalPaneTree, {
      props: {
        node: split,
        activePaneId: "p1",
        totalLeafCount: 2,
        maximizedPaneId: null,
      },
    });

    const headers = wrapper.findAll(".pane-header");
    expect(headers.length).toBeGreaterThanOrEqual(1);

    await headers[0].trigger("dblclick");
    expect(wrapper.emitted("toggleMaximize")).toBeTruthy();
    expect(wrapper.emitted("toggleMaximize")?.[0]).toEqual(["p1"]);
  });
});

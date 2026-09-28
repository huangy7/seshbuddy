import { describe, it, expect } from "vitest";
import {
  isLeaf,
  leafIds,
  firstLeafId,
  findLeaf,
  splitLeaf,
  removeLeaf,
  nextLeafId,
  siblingLeafOf,
  containsLeaf,
} from "./panes";
import type { TerminalPaneLeaf } from "../types/terminal";

function createLeaf(id: string): TerminalPaneLeaf {
  return {
    kind: "leaf",
    id,
    sessionId: `sess-${id}`,
    cliKind: "shell",
    projectPath: "/test",
    createdAt: Date.now(),
  };
}

describe("panes split tree utility", () => {
  it("manages single leaf node", () => {
    const root = createLeaf("p1");
    expect(isLeaf(root)).toBe(true);
    expect(leafIds(root)).toEqual(["p1"]);
    expect(firstLeafId(root)).toBe("p1");
    expect(findLeaf(root, "p1")).toBe(root);
    expect(findLeaf(root, "p2")).toBeNull();
  });

  it("splits leaf horizontally into row", () => {
    const l1 = createLeaf("p1");
    const l2 = createLeaf("p2");
    const splitTree = splitLeaf(l1, "p1", "s1", l2, "row");

    expect(isLeaf(splitTree)).toBe(false);
    expect(leafIds(splitTree)).toEqual(["p1", "p2"]);
    expect(firstLeafId(splitTree)).toBe("p1");
    expect(splitTree).toMatchObject({
      kind: "split",
      id: "s1",
      dir: "row",
      children: [l1, l2],
    });
  });

  it("appends as sibling if enclosing split direction matches", () => {
    const l1 = createLeaf("p1");
    const l2 = createLeaf("p2");
    const l3 = createLeaf("p3");
    let tree = splitLeaf(l1, "p1", "s1", l2, "row");
    tree = splitLeaf(tree, "p2", "s2", l3, "row");

    expect(tree.kind).toBe("split");
    if (tree.kind === "split") {
      expect(tree.children.length).toBe(3);
      expect(tree.children.map((c) => c.id)).toEqual(["p1", "p2", "p3"]);
    }
  });

  it("nests split if direction differs", () => {
    const l1 = createLeaf("p1");
    const l2 = createLeaf("p2");
    const l3 = createLeaf("p3");
    let tree = splitLeaf(l1, "p1", "s1", l2, "row");
    tree = splitLeaf(tree, "p2", "s2", l3, "col");

    expect(leafIds(tree)).toEqual(["p1", "p2", "p3"]);
    expect(findLeaf(tree, "p3")?.id).toBe("p3");
  });

  it("removes leaf and collapses single child splits", () => {
    const l1 = createLeaf("p1");
    const l2 = createLeaf("p2");
    const l3 = createLeaf("p3");
    let tree = splitLeaf(l1, "p1", "s1", l2, "row");
    tree = splitLeaf(tree, "p2", "s2", l3, "row");

    // Remove middle leaf
    tree = removeLeaf(tree, "p2")!;
    expect(leafIds(tree)).toEqual(["p1", "p3"]);

    // Remove another leaf, should collapse to single leaf
    tree = removeLeaf(tree, "p3")!;
    expect(isLeaf(tree)).toBe(true);
    expect(leafIds(tree)).toEqual(["p1"]);

    // Remove last leaf
    const empty = removeLeaf(tree, "p1");
    expect(empty).toBeNull();
  });

  it("navigates next and prev leaves", () => {
    const l1 = createLeaf("p1");
    const l2 = createLeaf("p2");
    const l3 = createLeaf("p3");
    let tree = splitLeaf(l1, "p1", "s1", l2, "row");
    tree = splitLeaf(tree, "p2", "s2", l3, "row");

    expect(nextLeafId(tree, "p1", 1)).toBe("p2");
    expect(nextLeafId(tree, "p2", 1)).toBe("p3");
    expect(nextLeafId(tree, "p3", 1)).toBe("p1");
    expect(nextLeafId(tree, "p1", -1)).toBe("p3");
  });

  it("finds sibling leaf when closing", () => {
    const l1 = createLeaf("p1");
    const l2 = createLeaf("p2");
    const l3 = createLeaf("p3");
    let tree = splitLeaf(l1, "p1", "s1", l2, "row");
    tree = splitLeaf(tree, "p2", "s2", l3, "row");

    expect(siblingLeafOf(tree, "p1")).toBe("p2");
    expect(siblingLeafOf(tree, "p2")).toBe("p3");
  });

  it("checks if tree contains a leaf with containsLeaf", () => {
    const l1 = createLeaf("p1");
    const l2 = createLeaf("p2");
    const l3 = createLeaf("p3");
    let tree = splitLeaf(l1, "p1", "s1", l2, "row");
    tree = splitLeaf(tree, "p2", "s2", l3, "col");

    expect(containsLeaf(tree, "p1")).toBe(true);
    expect(containsLeaf(tree, "p2")).toBe(true);
    expect(containsLeaf(tree, "p3")).toBe(true);
    expect(containsLeaf(tree, "p4")).toBe(false);
  });
});

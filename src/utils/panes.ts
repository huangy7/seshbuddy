import type {
  PaneId,
  SplitDir,
  TerminalPaneLeaf,
  TerminalPaneNode,
  TerminalPaneSplit,
} from "../types/terminal";

export function isLeaf(node: TerminalPaneNode): node is TerminalPaneLeaf {
  return node.kind === "leaf";
}

export function isSplit(node: TerminalPaneNode): node is TerminalPaneSplit {
  return node.kind === "split";
}

export function leafIds(node: TerminalPaneNode): PaneId[] {
  if (isLeaf(node)) return [node.id];
  return node.children.flatMap(leafIds);
}

export function firstLeafId(node: TerminalPaneNode): PaneId {
  if (isLeaf(node)) return node.id;
  return firstLeafId(node.children[0]);
}

export function findLeaf(
  node: TerminalPaneNode,
  id: PaneId,
): TerminalPaneLeaf | null {
  if (isLeaf(node)) return node.id === id ? node : null;
  for (const child of node.children) {
    const found = findLeaf(child, id);
    if (found) return found;
  }
  return null;
}

export function containsLeaf(node: TerminalPaneNode, leafId: PaneId): boolean {
  if (isLeaf(node)) return node.id === leafId;
  return node.children.some((c) => containsLeaf(c, leafId));
}

/**
 * 在目标叶子节点旁边按 dir 方向插入新叶子节点。
 * 如果目标的父级 split 方向与 dir 相同，则直接作为兄弟节点插入（避免相同方向的多层嵌套分割）。
 */
export function splitLeaf(
  tree: TerminalPaneNode,
  targetId: PaneId,
  newSplitId: PaneId,
  newLeaf: TerminalPaneLeaf,
  dir: SplitDir,
): TerminalPaneNode {
  if (isSplit(tree) && tree.dir === dir) {
    const idx = tree.children.findIndex(
      (c) => isLeaf(c) && c.id === targetId,
    );
    if (idx >= 0) {
      const nextChildren = [
        ...tree.children.slice(0, idx + 1),
        newLeaf,
        ...tree.children.slice(idx + 1),
      ];
      // 平分比例
      const count = nextChildren.length;
      const ratios = Array.from({ length: count }, () => 1 / count);
      return {
        ...tree,
        ratios,
        children: nextChildren,
      };
    }
  }

  if (isLeaf(tree)) {
    if (tree.id !== targetId) return tree;
    return {
      kind: "split",
      id: newSplitId,
      dir,
      ratios: [0.5, 0.5],
      children: [tree, newLeaf],
    };
  }

  return {
    ...tree,
    children: tree.children.map((c) =>
      splitLeaf(c, targetId, newSplitId, newLeaf, dir),
    ),
  };
}

/**
 * 移除目标叶子节点，并自动折叠只剩单个子节点的 split 分支。若整棵树均被移除则返回 null。
 */
export function removeLeaf(
  tree: TerminalPaneNode,
  targetId: PaneId,
): TerminalPaneNode | null {
  if (isLeaf(tree)) return tree.id === targetId ? null : tree;

  const newChildren: TerminalPaneNode[] = [];
  for (const c of tree.children) {
    const r = removeLeaf(c, targetId);
    if (r !== null) newChildren.push(r);
  }

  if (newChildren.length === 0) return null;
  if (newChildren.length === 1) return newChildren[0];

  const count = newChildren.length;
  const ratios = Array.from({ length: count }, () => 1 / count);
  return { ...tree, ratios, children: newChildren };
}

/**
 * 循环获取下一个或上一个叶子节点 ID。
 */
export function nextLeafId(
  tree: TerminalPaneNode,
  currentId: PaneId,
  delta: 1 | -1,
): PaneId {
  const ids = leafIds(tree);
  if (ids.length === 0) return currentId;
  const idx = ids.indexOf(currentId);
  if (idx < 0) return ids[0];
  return ids[(idx + delta + ids.length) % ids.length];
}

/**
 * 获取 targetLeafId 最近的相邻叶子节点（优先下一个兄弟，其次上一个兄弟）。
 * 用于某个分屏关闭时光标焦点自然转移到相邻分屏。
 */
export function siblingLeafOf(
  tree: TerminalPaneNode,
  leafId: PaneId,
): PaneId | null {
  if (isLeaf(tree)) return null;
  for (let i = 0; i < tree.children.length; i++) {
    const c = tree.children[i];
    if (isLeaf(c) && c.id === leafId) {
      const sibling = tree.children[i + 1] ?? tree.children[i - 1];
      if (!sibling) return null;
      return leafIds(sibling)[0] ?? null;
    }
  }
  for (const c of tree.children) {
    if (!isLeaf(c)) {
      const r = siblingLeafOf(c, leafId);
      if (r !== null) return r;
    }
  }
  return null;
}

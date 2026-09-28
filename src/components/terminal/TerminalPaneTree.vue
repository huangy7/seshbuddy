<script setup lang="ts">
import { ref } from "vue";
import type {
  PaneId,
  TerminalPaneNode,
  TerminalPaneSplit,
} from "../../types/terminal";
import { containsLeaf } from "../../utils/panes";
import TerminalPane from "./TerminalPane.vue";
import TerminalSplitResizer from "./TerminalSplitResizer.vue";

defineProps<{
  node: TerminalPaneNode;
  activePaneId: PaneId;
  totalLeafCount: number;
  maximizedPaneId?: PaneId | null;
}>();

const emit = defineEmits<{
  focus: [paneId: PaneId];
  splitRight: [paneId: PaneId];
  splitDown: [paneId: PaneId];
  close: [paneId: PaneId];
  toggleMaximize: [paneId: PaneId];
  openFile: [path: string, projectRoot: string];
  updateRatios: [splitId: PaneId, ratios: number[]];
}>();

const containerRef = ref<HTMLDivElement>();

function onResizeDelta(split: TerminalPaneSplit, index: number, deltaPx: number) {
  const container = containerRef.value;
  if (!container) return;

  const totalPx = split.dir === "row" ? container.offsetWidth : container.offsetHeight;
  if (totalPx <= 0) return;

  const deltaRatio = deltaPx / totalPx;
  const count = split.children.length;
  const currentRatios = split.ratios && split.ratios.length === count
    ? [...split.ratios]
    : Array.from({ length: count }, () => 1 / count);

  const leftIndex = index;
  const rightIndex = index + 1;

  if (leftIndex < 0 || rightIndex >= count) return;

  const minRatio = 0.1;
  let newLeft = currentRatios[leftIndex] + deltaRatio;
  let newRight = currentRatios[rightIndex] - deltaRatio;

  if (newLeft < minRatio) {
    newRight -= (minRatio - newLeft);
    newLeft = minRatio;
  } else if (newRight < minRatio) {
    newRight -= (minRatio - newRight);
    newRight = minRatio;
  }

  currentRatios[leftIndex] = newLeft;
  currentRatios[rightIndex] = newRight;

  emit("updateRatios", split.id, currentRatios);
}

function onResetRatio(split: TerminalPaneSplit) {
  const count = split.children.length;
  const ratios = Array.from({ length: count }, () => 1 / count);
  emit("updateRatios", split.id, ratios);
}
</script>

<template>
  <div
    v-if="node.kind === 'leaf'"
    class="tree-leaf-container"
  >
    <TerminalPane
      :leaf="node"
      :isActive="node.id === activePaneId"
      :canClose="totalLeafCount > 1"
      :showHeader="totalLeafCount > 1"
      :isMaximized="node.id === maximizedPaneId"
      @focus="emit('focus', node.id)"
      @splitRight="emit('splitRight', node.id)"
      @splitDown="emit('splitDown', node.id)"
      @close="emit('close', node.id)"
      @toggleMaximize="emit('toggleMaximize', node.id)"
      @openFile="(path, root) => emit('openFile', path, root)"
    />
  </div>

  <div
    v-else
    ref="containerRef"
    class="tree-split-container"
    :class="node.dir"
  >
    <template v-for="(child, idx) in node.children" :key="child.id">
      <div
        class="tree-child-wrap"
        :style="{
          display: (maximizedPaneId && !containsLeaf(child, maximizedPaneId)) ? 'none' : 'flex',
          flex: maximizedPaneId
            ? '1 1 100%'
            : `${(node.ratios && node.ratios[idx]) ? node.ratios[idx] : 1} 1 0%`,
          width: maximizedPaneId ? '100%' : undefined,
          height: maximizedPaneId ? '100%' : undefined,
        }"
      >
        <TerminalPaneTree
          :node="child"
          :activePaneId="activePaneId"
          :totalLeafCount="totalLeafCount"
          :maximizedPaneId="maximizedPaneId"
          @focus="(id) => emit('focus', id)"
          @splitRight="(id) => emit('splitRight', id)"
          @splitDown="(id) => emit('splitDown', id)"
          @close="(id) => emit('close', id)"
          @toggleMaximize="(id) => emit('toggleMaximize', id)"
          @openFile="(path, root) => emit('openFile', path, root)"
          @updateRatios="(id, ratios) => emit('updateRatios', id, ratios)"
        />
      </div>

      <TerminalSplitResizer
        v-if="!maximizedPaneId && idx < node.children.length - 1"
        :dir="node.dir"
        @resizeDelta="(delta) => onResizeDelta(node as TerminalPaneSplit, idx, delta)"
        @resetRatio="() => onResetRatio(node as TerminalPaneSplit)"
      />
    </template>
  </div>
</template>

<style scoped>
.tree-leaf-container {
  flex: 1;
  display: flex;
  flex-direction: column;
  width: 100%;
  height: 100%;
  position: relative;
  min-width: 0;
  min-height: 0;
}

.tree-split-container {
  flex: 1;
  display: flex;
  width: 100%;
  height: 100%;
  position: relative;
  min-width: 0;
  min-height: 0;
  overflow: hidden;
}

.tree-split-container.row {
  flex-direction: row;
}

.tree-split-container.col {
  flex-direction: column;
}

.tree-child-wrap {
  position: relative;
  min-width: 0;
  min-height: 0;
  overflow: hidden;
  display: flex;
  flex-direction: column;
}
</style>

<script setup lang="ts">
import { ref, onBeforeUnmount } from "vue";
import type { SplitDir } from "../../types/terminal";
import { t } from "../../i18n";

const props = defineProps<{
  dir: SplitDir;
}>();

const emit = defineEmits<{
  resizeDelta: [deltaPx: number];
  resetRatio: [];
}>();

const isDragging = ref(false);
let startPos = 0;

function onMouseDown(e: MouseEvent) {
  e.preventDefault();
  isDragging.value = true;
  startPos = props.dir === "row" ? e.clientX : e.clientY;

  document.addEventListener("mousemove", onMouseMove);
  document.addEventListener("mouseup", onMouseUp);
  document.body.style.cursor = props.dir === "row" ? "col-resize" : "row-resize";
  document.body.style.userSelect = "none";
}

function onMouseMove(e: MouseEvent) {
  if (!isDragging.value) return;
  const currentPos = props.dir === "row" ? e.clientX : e.clientY;
  const delta = currentPos - startPos;
  startPos = currentPos;
  emit("resizeDelta", delta);
}

function onMouseUp() {
  if (!isDragging.value) return;
  isDragging.value = false;
  document.removeEventListener("mousemove", onMouseMove);
  document.removeEventListener("mouseup", onMouseUp);
  document.body.style.cursor = "";
  document.body.style.userSelect = "";
}

function onDblClick() {
  emit("resetRatio");
}

onBeforeUnmount(() => {
  if (isDragging.value) {
    document.removeEventListener("mousemove", onMouseMove);
    document.removeEventListener("mouseup", onMouseUp);
    document.body.style.cursor = "";
    document.body.style.userSelect = "";
  }
});
</script>

<template>
  <div
    class="terminal-split-resizer"
    :class="[dir, { dragging: isDragging }]"
    :title="t('common.terminal.resizeHint')"
    @mousedown="onMouseDown"
    @dblclick="onDblClick"
  >
    <div class="resizer-handle"></div>
  </div>
</template>

<style scoped>
.terminal-split-resizer {
  position: relative;
  flex-shrink: 0;
  z-index: 10;
  background: var(--color-border, #333);
  transition: background var(--transition-fast);
}

.terminal-split-resizer.row {
  width: 4px;
  cursor: col-resize;
  margin: 0 -2px;
}

.terminal-split-resizer.col {
  height: 4px;
  cursor: row-resize;
  margin: -2px 0;
}

.terminal-split-resizer:hover,
.terminal-split-resizer.dragging {
  background: var(--color-primary, #3b82f6);
}

.resizer-handle {
  position: absolute;
  inset: 0;
}

.terminal-split-resizer.row .resizer-handle {
  inset: 0 -3px;
}

.terminal-split-resizer.col .resizer-handle {
  inset: -3px 0;
}
</style>

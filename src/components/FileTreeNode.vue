<script setup lang="ts">
import { computed } from "vue";
import SvgIcon from "./icons/SvgIcon.vue";

interface FileEntry {
  name: string;
  path: string;
  isDir: boolean;
  children?: FileEntry[];
}

const props = defineProps<{
  entry: FileEntry;
  projectRoot: string;
  expandedDirs: Set<string>;
  depth: number;
  filterText?: string;
}>();

const emit = defineEmits<{
  openFile: [path: string, projectRoot: string];
  pinFile: [path: string, projectRoot: string];
  toggleDir: [path: string];
  contextMenu: [e: MouseEvent, entry: FileEntry, root: string];
}>();

function fileIcon(entry: FileEntry): string {
  if (entry.isDir) return "folder";
  const ext = entry.name.split(".").pop()?.toLowerCase();
  if (ext === "html" || ext === "htm") return "globe";
  if (ext === "sh" || ext === "bash" || ext === "zsh") return "terminal";
  if (ext === "json" || ext === "yaml" || ext === "yml" || ext === "toml") return "code";
  if (
    ext === "go" ||
    ext === "rs" ||
    ext === "ts" ||
    ext === "js" ||
    ext === "vue" ||
    ext === "py" ||
    ext === "c" ||
    ext === "cpp" ||
    ext === "java" ||
    ext === "sql" ||
    ext === "css" ||
    ext === "scss"
  )
    return "code";
  if (ext === "png" || ext === "jpg" || ext === "jpeg" || ext === "svg" || ext === "gif" || ext === "webp")
    return "image";
  return "file-text";
}

const highlightParts = computed(() => {
  const q = props.filterText?.trim().toLowerCase();
  if (!q) return [{ text: props.entry.name, highlight: false }];
  const name = props.entry.name;
  const lower = name.toLowerCase();
  const idx = lower.indexOf(q);
  if (idx < 0) return [{ text: name, highlight: false }];
  const parts: { text: string; highlight: boolean }[] = [];
  if (idx > 0) parts.push({ text: name.slice(0, idx), highlight: false });
  parts.push({ text: name.slice(idx, idx + q.length), highlight: true });
  if (idx + q.length < name.length) parts.push({ text: name.slice(idx + q.length), highlight: false });
  return parts;
});
</script>

<template>
  <div class="tree-node" :style="{ paddingLeft: (depth * 10) + 'px' }">
    <!-- Directory -->
    <div
      v-if="entry.isDir"
      class="tree-item tree-dir"
      :data-path="entry.path"
      @click="emit('toggleDir', entry.path)"
      @mousedown.right.prevent
      @contextmenu="emit('contextMenu', $event, entry, projectRoot)"
    >
      <SvgIcon class="tree-chevron" :name="(expandedDirs.has(entry.path) || filterText) ? 'chevron-down' : 'chevron-right'" :size="11" />
      <SvgIcon class="tree-folder-icon" name="folder" :size="13" />
      <span class="item-name">
        <template v-for="(part, i) in highlightParts" :key="i">
          <mark v-if="part.highlight" class="filter-match">{{ part.text }}</mark>
          <template v-else>{{ part.text }}</template>
        </template>
      </span>
    </div>
    <!-- Children (recursive) -->
    <template v-if="entry.isDir && (expandedDirs.has(entry.path) || filterText)">
      <FileTreeNode
        v-for="child in entry.children ?? []"
        :key="child.path"
        :entry="child"
        :projectRoot="projectRoot"
        :expandedDirs="expandedDirs"
        :depth="depth + 1"
        :filterText="filterText"
        @openFile="(path: string, root: string) => emit('openFile', path, root)"
        @pinFile="(path: string, root: string) => emit('pinFile', path, root)"
        @toggleDir="(path: string) => emit('toggleDir', path)"
        @contextMenu="(e: MouseEvent, en: FileEntry, root: string) => emit('contextMenu', e, en, root)"
      />
    </template>
    <!-- File -->
    <div
      v-if="!entry.isDir"
      class="tree-item tree-file"
      :data-path="entry.path"
      @click="emit('openFile', entry.path, projectRoot)"
      @dblclick="emit('pinFile', entry.path, projectRoot)"
      @mousedown.right.prevent
      @contextmenu="emit('contextMenu', $event, entry, projectRoot)"
    >
      <SvgIcon :name="fileIcon(entry)" :size="13" />
      <span class="item-name">
        <template v-for="(part, i) in highlightParts" :key="i">
          <mark v-if="part.highlight" class="filter-match">{{ part.text }}</mark>
          <template v-else>{{ part.text }}</template>
        </template>
      </span>
    </div>
  </div>
</template>

<style scoped>
.tree-node {
  display: flex;
  flex-direction: column;
  user-select: none;
  -webkit-user-select: none;
}
.tree-item {
  display: flex;
  align-items: center;
  min-height: 28px;
  height: 28px;
  box-sizing: border-box;
  gap: 6px;
  padding: 0 8px;
  margin: 1px 6px;
  font-size: var(--text-xs);
  color: var(--color-text-secondary);
  cursor: pointer;
  border-radius: 6px;
  transition: background var(--transition-fast), color var(--transition-fast);
}
.tree-item:hover {
  background: var(--color-bg-hover);
  color: var(--color-text);
}
.tree-chevron { color: var(--color-text-muted); opacity: 0.8; }
.tree-folder-icon { color: var(--color-primary); }
.tree-dir { font-weight: 500; color: var(--color-text); }
.tree-file { color: var(--color-text-secondary); }
.item-name {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.filter-match {
  background: var(--color-primary-light, rgba(59, 130, 246, 0.2));
  color: var(--color-primary, #3b82f6);
  border-radius: 2px;
  padding: 0 2px;
  font-weight: 600;
}
@keyframes locate-flash {
  0% { background-color: transparent; }
  20% { background-color: var(--color-primary-light); }
  100% { background-color: transparent; }
}
.locate-highlight {
  animation: locate-flash 1.5s ease-out;
}
</style>

<script setup lang="ts">
import { ref, computed, onMounted, onBeforeUnmount } from "vue";
import { invokeApp, renderAppError } from "../utils/invokeApp";
import SvgIcon from "./icons/SvgIcon.vue";
import { t } from "../i18n";

const props = defineProps<{
  filePath: string;
  projectRoot: string;
}>();

const emit = defineEmits<{
  openFile: [path: string, projectRoot: string];
}>();

interface FileEntry {
  name: string;
  path: string;
  isDir: boolean;
}

// Splitting the path into clickable segments
const breadcrumbs = computed(() => {
  // Fix for Windows paths: replace \ with / for easier processing
  const safeFilePath = props.filePath.replace(/\\/g, "/");
  const safeProjectRoot = props.projectRoot.replace(/\\/g, "/");

  const projName = safeProjectRoot.split("/").pop() ?? safeProjectRoot;
  const relative = safeFilePath.startsWith(safeProjectRoot)
    ? safeFilePath.slice(safeProjectRoot.length + 1)
    : safeFilePath;
  
  const parts = relative.split("/").filter(Boolean);
  
  const crumbs = [{ name: projName, isRoot: true, path: safeProjectRoot }];
  let currentPath = safeProjectRoot;
  for (let i = 0; i < parts.length; i++) {
    currentPath += "/" + parts[i];
    crumbs.push({ name: parts[i], isRoot: false, path: currentPath });
  }
  return crumbs;
});

const activeMenuIndex = ref<number | null>(null);
const menuItems = ref<{ name: string; isDir: boolean; path: string }[]>([]);
const menuPosition = ref({ x: 0, y: 0 });
const isLoading = ref(false);
const errorMessage = ref<string | null>(null);

async function handleBreadcrumbClick(event: MouseEvent, index: number, crumb: any) {
  event.stopPropagation();
  
  if (activeMenuIndex.value === index) {
    closeMenu();
    return;
  }

  // If clicking the file itself, don't show a dropdown
  if (index === breadcrumbs.value.length - 1 && !crumb.isDir) {
    return;
  }

  // Position logic
  const target = event.currentTarget as HTMLElement;
  const rect = target.getBoundingClientRect();
  menuPosition.value = { x: rect.left, y: rect.bottom + 4 };
  activeMenuIndex.value = index;
  isLoading.value = true;
  errorMessage.value = null;
  menuItems.value = [];

  try {
    const dirPath = crumb.path;
    const entries = await invokeApp<FileEntry[]>("read_directory", { path: dirPath, projectRoot: props.projectRoot });
    
    // entries returned from backend are already sorted and filtered for hidden files
    menuItems.value = entries.map(entry => ({
      name: entry.name,
      isDir: entry.isDir,
      path: entry.path
    }));
  } catch (e: any) {
    console.error("Failed to read dir:", e);
    errorMessage.value = renderAppError(e);
  } finally {
    isLoading.value = false;
  }
}

function closeMenu() {
  activeMenuIndex.value = null;
  errorMessage.value = null;
}

function handleOutsideClick(_e: MouseEvent) {
  closeMenu();
}

function fileIcon(item: any): string {
  if (item.isDir) return "folder";
  const ext = item.name.split(".").pop()?.toLowerCase();
  if (ext === "html" || ext === "htm") return "globe";
  if (ext === "sh") return "terminal";
  return "file-text";
}
function openFile(item: any) {
  closeMenu();
  if (!item.isDir) {
    emit("openFile", item.path, props.projectRoot);
  }
}

onMounted(() => {
  document.addEventListener("click", handleOutsideClick);
});

onBeforeUnmount(() => {
  document.removeEventListener("click", handleOutsideClick);
});
</script>

<template>
  <div class="interactive-breadcrumb">
    <template v-for="(crumb, idx) in breadcrumbs" :key="idx">
      <div 
        class="breadcrumb-part"
        :class="{ 
          'is-last': idx === breadcrumbs.length - 1, 
          'has-menu': idx < breadcrumbs.length - 1,
          'is-active': activeMenuIndex === idx
        }"
        @click="(e) => handleBreadcrumbClick(e, idx, crumb)"
      >
        <span>{{ crumb.name }}</span>
        <SvgIcon v-if="idx < breadcrumbs.length - 1" name="chevron-right" :size="12" class="sep-icon" />
      </div>
    </template>

    <Teleport to="body">
      <Transition name="fade-fast">
        <div 
          v-if="activeMenuIndex !== null" 
          class="breadcrumb-dropdown"
          :style="{ left: menuPosition.x + 'px', top: menuPosition.y + 'px' }"
          @click.stop
        >
          <div v-if="isLoading" class="dropdown-item loading">{{ t("common.breadcrumb.loading") }}</div>
          <div v-else-if="errorMessage" class="dropdown-item error">{{ errorMessage }}</div>
          <div v-else-if="menuItems.length === 0" class="dropdown-item empty">{{ t("common.breadcrumb.empty") }}</div>
          <template v-else>
            <div 
              v-for="item in menuItems" 
              :key="item.name"
              class="dropdown-item"
              :class="{ 'is-dir': item.isDir }"
              @click="openFile(item)"
            >
              <SvgIcon :name="fileIcon(item)" :size="14" class="item-icon" />
              <span class="item-name">{{ item.name }}</span>
            </div>
          </template>
        </div>
      </Transition>
    </Teleport>
  </div>
</template>

<style scoped>
.interactive-breadcrumb {
  display: flex;
  align-items: center;
  gap: 2px;
  padding: var(--space-1) var(--space-3);
  font-size: var(--text-xs);
  color: var(--color-text-muted);
  border-bottom: 1px solid var(--color-border);
  background: var(--color-bg-secondary);
  flex-shrink: 0;
  overflow-x: auto;
  white-space: nowrap;
}

.breadcrumb-part {
  display: flex;
  align-items: center;
  gap: 4px;
  padding: 4px 6px;
  border-radius: var(--radius-sm);
  cursor: default;
  transition: all var(--transition-fast);
}

.breadcrumb-part.has-menu {
  cursor: pointer;
}

.breadcrumb-part.has-menu:hover,
.breadcrumb-part.is-active {
  background: var(--color-bg-hover);
  color: var(--color-text);
}

.breadcrumb-part.is-last {
  color: var(--color-text);
  font-weight: 500;
}

.sep-icon {
  opacity: 0.5;
  margin-left: 2px;
}

.breadcrumb-part:hover .sep-icon {
  opacity: 1;
}

.breadcrumb-dropdown {
  position: fixed;
  z-index: 1000;
  background: var(--color-bg, #ffffff);
  border: 1px solid var(--color-border);
  border-radius: var(--radius-md);
  box-shadow: var(--shadow-context);
  min-width: 200px;
  max-height: 400px;
  overflow-y: auto;
  padding: 4px;
  font-size: var(--text-xs);
}

.dropdown-item {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 6px 8px;
  border-radius: var(--radius-sm);
  cursor: pointer;
  color: var(--color-text);
}

.dropdown-item:hover {
  background: var(--color-bg-hover);
}

.dropdown-item.loading,
.dropdown-item.empty,
.dropdown-item.error {
  color: var(--color-text-muted);
  cursor: default;
  justify-content: center;
}

.dropdown-item.error {
  color: var(--color-error);
  white-space: normal;
  word-break: break-all;
  padding: 8px;
}

.dropdown-item.loading:hover,
.dropdown-item.empty:hover,
.dropdown-item.error:hover {
  background: transparent;
}

.item-icon {
  color: var(--color-text-secondary);
}

.fade-fast-enter-active,
.fade-fast-leave-active {
  transition: opacity 0.1s ease, transform 0.1s ease;
}
.fade-fast-enter-from,
.fade-fast-leave-to {
  opacity: 0;
  transform: translateY(-4px);
}
</style>

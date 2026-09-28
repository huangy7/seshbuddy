<script setup lang="ts">
import { ref, onMounted, onBeforeUnmount, nextTick, watch } from "vue";
import SvgIcon from "./icons/SvgIcon.vue";
import ChatAvatar from "./chat/ChatAvatar.vue";
import type { TabItem } from "../types/tab";
import { t } from "../i18n";

const props = defineProps<{
  tabs: TabItem[];
  activeTabId: string | null;
  rightSidebarOpen?: boolean;
}>();

const emit = defineEmits<{
  selectTab: [id: string];
  closeTab: [id: string];
  pinTab: [id: string];
  closeOtherTabs: [id: string];
  closeRightTabs: [id: string];
  closeAllTabs: [];
  closeSavedTabs: [];
  toggleRightSidebar: [];
  renameTab: [id: string, newLabel: string];
  openRenameDialog: [id: string, currentLabel: string];
  newTerminalTab: [];
  newCliTab: [cliKind: string];
  openNewSessionDialog: [];
  splitRight: [tabId: string];
  splitDown: [tabId: string];
}>();

const ctxMenu = ref<{ x: number; y: number; tabId: string } | null>(null);
const tabListRef = ref<HTMLElement | null>(null);
const tabRefs = new Map<string, HTMLElement>();
const scrollbar = ref({ visible: false, left: 0, width: 0 });
let resizeObserver: ResizeObserver | undefined;
let stopScrollbarDrag: (() => void) | undefined;

function onTabDoubleClick(tab: TabItem) {
  if (tab.isPreview) {
    emit("pinTab", tab.id);
  } else {
    emit("openRenameDialog", tab.id, tab.label.replace(/^●\s*/, ""));
  }
}

// New Tab Dropdown Menu
const newMenuOpen = ref(false);
const newMenuPos = ref({ x: 0, y: 0 });

function toggleNewMenu(e: MouseEvent) {
  e.stopPropagation();
  closeCtxMenu();
  const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
  const menuWidth = 150;
  let left = rect.right - menuWidth;
  if (left < 8) left = 8;
  if (left + menuWidth > window.innerWidth - 8) {
    left = window.innerWidth - menuWidth - 8;
  }
  newMenuPos.value = { x: left, y: rect.bottom + 4 };
  newMenuOpen.value = !newMenuOpen.value;
}

function closeNewMenu() {
  newMenuOpen.value = false;
}

function registerTabRef(id: string, el: unknown) {
  if (el instanceof HTMLElement) {
    tabRefs.set(id, el);
  } else {
    tabRefs.delete(id);
  }
}

function activeContextTab(): TabItem | undefined {
  return ctxMenu.value ? props.tabs.find((tab) => tab.id === ctxMenu.value?.tabId) : undefined;
}

async function scrollActiveTabIntoView() {
  await nextTick();
  if (!props.activeTabId) return;
  const el = tabRefs.get(props.activeTabId);
  el?.scrollIntoView({ block: "nearest", inline: "nearest", behavior: "smooth" });
  requestAnimationFrame(updateScrollbar);
}

function onTabContextMenu(e: MouseEvent, tabId: string) {
  e.preventDefault();
  closeNewMenu();
  ctxMenu.value = { x: e.clientX, y: e.clientY, tabId };
}

function closeCtxMenu() {
  ctxMenu.value = null;
}

function handleClickOutside() {
  if (ctxMenu.value) {
    closeCtxMenu();
  }
  if (newMenuOpen.value) {
    closeNewMenu();
  }
}

function updateScrollbar() {
  const el = tabListRef.value;
  if (!el) return;

  const maxScroll = el.scrollWidth - el.clientWidth;
  if (maxScroll <= 1) {
    scrollbar.value = { visible: false, left: 0, width: 0 };
    return;
  }

  const width = Math.max(28, (el.clientWidth / el.scrollWidth) * el.clientWidth);
  const maxLeft = el.clientWidth - width;
  const left = maxLeft * (el.scrollLeft / maxScroll);
  scrollbar.value = { visible: true, left, width };
}

function onScrollbarMouseDown(e: MouseEvent) {
  const el = tabListRef.value;
  if (!el || !scrollbar.value.visible) return;
  e.preventDefault();

  const trackRect = (e.currentTarget as HTMLElement).getBoundingClientRect();
  const thumbStart = scrollbar.value.left;
  const clickedThumb =
    e.clientX >= trackRect.left + thumbStart &&
    e.clientX <= trackRect.left + thumbStart + scrollbar.value.width;

  if (!clickedThumb) {
    const nextLeft = Math.min(
      Math.max(e.clientX - trackRect.left - scrollbar.value.width / 2, 0),
      el.clientWidth - scrollbar.value.width,
    );
    el.scrollLeft = (nextLeft / (el.clientWidth - scrollbar.value.width)) * (el.scrollWidth - el.clientWidth);
    updateScrollbar();
  }

  const startX = e.clientX;
  const startScrollLeft = el.scrollLeft;
  const maxScroll = el.scrollWidth - el.clientWidth;
  const thumbTravel = el.clientWidth - scrollbar.value.width;
  const scrollPerPixel = thumbTravel > 0 ? maxScroll / thumbTravel : 0;

  const onMove = (moveEvent: MouseEvent) => {
    el.scrollLeft = startScrollLeft + (moveEvent.clientX - startX) * scrollPerPixel;
    updateScrollbar();
  };
  const onUp = () => {
    document.removeEventListener("mousemove", onMove);
    document.removeEventListener("mouseup", onUp);
    stopScrollbarDrag = undefined;
  };

  document.addEventListener("mousemove", onMove);
  document.addEventListener("mouseup", onUp);
  stopScrollbarDrag = onUp;
}

function onTabWheel(e: WheelEvent) {
  const el = tabListRef.value;
  if (!el) return;

  const maxScroll = el.scrollWidth - el.clientWidth;
  if (maxScroll <= 1) return;

  const delta = Math.abs(e.deltaX) > Math.abs(e.deltaY) ? e.deltaX : e.deltaY;
  if (delta === 0) return;

  e.preventDefault();
  el.scrollLeft += delta;
  updateScrollbar();
}

onMounted(() => {
  document.addEventListener("click", handleClickOutside);
  if (tabListRef.value) {
    resizeObserver = new ResizeObserver(updateScrollbar);
    resizeObserver.observe(tabListRef.value);
  }
  scrollActiveTabIntoView();
  updateScrollbar();
});
onBeforeUnmount(() => {
  document.removeEventListener("click", handleClickOutside);
  resizeObserver?.disconnect();
  stopScrollbarDrag?.();
});

watch(
  () => [props.activeTabId, props.tabs.length],
  () => {
    scrollActiveTabIntoView();
    nextTick(updateScrollbar);
  },
  { flush: "post" }
);
</script>

<template>
  <div class="tab-bar">
    <div class="tab-scroll-shell" @wheel="onTabWheel">
      <div ref="tabListRef" class="tab-list" @scroll="updateScrollbar">
        <div
          v-for="tab in tabs"
          :key="tab.id"
          :ref="(el) => registerTabRef(tab.id, el)"
          class="tab-item"
          :data-tab-id="tab.id"
          :class="{ active: activeTabId === tab.id, preview: tab.isPreview }"
          @click="emit('selectTab', tab.id)"
          @dblclick="onTabDoubleClick(tab)"
          @contextmenu="onTabContextMenu($event, tab.id)"
        >
          <span v-if="tab.statusColor" class="tab-status-dot" :style="{ background: tab.statusColor }"></span>
          <ChatAvatar
            v-if="tab.cliId && (tab.type === 'history' || tab.type === 'terminal')"
            role="assistant"
            :cliId="tab.cliId"
            class="tab-cli-badge"
          />
          <SvgIcon v-else :name="tab.icon" :size="13" />

          <span class="tab-label" :title="tab.label">{{ tab.label }}</span>

          <span
            v-if="tab.splitCount && tab.splitCount > 1"
            class="tab-split-badge"
            :title="t('session.tabBar.splitCount')"
          >
            {{ t("session.tabBar.splitBadge", { count: tab.splitCount }) }}
          </span>

          <button
            v-if="tab.closable !== false"
            class="tab-close"
            :title="t('session.tabBar.closeTab')"
            :aria-label="t('session.tabBar.closeTab')"
            @click.stop="emit('closeTab', tab.id)"
          >
            <SvgIcon name="x" :size="10" />
          </button>
        </div>
      </div>
      <div
        v-if="scrollbar.visible"
        class="tab-scrollbar"
        @mousedown="onScrollbarMouseDown"
      >
        <div
          class="tab-scrollbar-thumb"
          :style="{ width: scrollbar.width + 'px', transform: `translateX(${scrollbar.left}px)` }"
        ></div>
      </div>
    </div>

    <!-- New Terminal Tab Actions -->
    <div class="new-tab-actions">
      <button
        type="button"
        class="new-tab-btn main"
        :title="t('session.tabBar.newTerminalTitle')"
        :aria-label="t('session.tabBar.newTerminal')"
        @click="emit('newTerminalTab')"
      >
        <SvgIcon name="plus" :size="13" />
      </button>
      <button
        type="button"
        class="new-tab-btn dropdown"
        :title="t('session.tabBar.newTerminalOptions')"
        :aria-label="t('session.tabBar.newTerminalOptions')"
        @click="toggleNewMenu"
      >
        <SvgIcon name="chevron-down" :size="10" />
      </button>
    </div>

    <button
      type="button"
      class="tab-bar-action-btn"
      :class="{ active: rightSidebarOpen }"
      :title="rightSidebarOpen ? t('session.common.collapseSidebar') : t('session.common.expandSidebar')"
      :aria-label="rightSidebarOpen ? t('session.common.collapseSidebar') : t('session.common.expandSidebar')"
      @click="emit('toggleRightSidebar')"
    >
      <SvgIcon name="panel-right" :size="14" />
    </button>

    <!-- Tab context menu -->
    <Teleport to="body">
      <div
        v-if="ctxMenu"
        class="tab-ctx-menu"
        :style="{ left: ctxMenu.x + 'px', top: ctxMenu.y + 'px' }"
        @click.stop
      >
        <button
          class="tab-ctx-item"
          @click="() => { const tab = activeContextTab(); if (tab) { emit('openRenameDialog', tab.id, tab.label.replace(/^●\s*/, '')); closeCtxMenu(); } }"
        >
          <span>{{ t("session.tabBar.rename") }}</span>
        </button>

        <template v-if="activeContextTab()?.type === 'terminal'">
          <div class="tab-ctx-sep" />
          <button
            class="tab-ctx-item"
            @click="emit('splitRight', ctxMenu!.tabId); closeCtxMenu()"
          >
            <span>{{ t("session.tabBar.splitRight") }}</span>
            <span class="ctx-shortcut">⌘D</span>
          </button>
          <button
            class="tab-ctx-item"
            @click="emit('splitDown', ctxMenu!.tabId); closeCtxMenu()"
          >
            <span>{{ t("session.tabBar.splitDown") }}</span>
            <span class="ctx-shortcut">⌘⇧D</span>
          </button>
        </template>

        <div class="tab-ctx-sep" />
        <button
          v-if="activeContextTab()?.isPreview"
          class="tab-ctx-item"
          @click="emit('pinTab', ctxMenu!.tabId); closeCtxMenu()"
        >{{ t("session.tabBar.pinTab") }}</button>
        <button class="tab-ctx-item" @click="emit('closeTab', ctxMenu!.tabId); closeCtxMenu()">
          <span>{{ t("dialogs.common.close") }}</span>
          <span class="ctx-shortcut">⌘W</span>
        </button>
        <button class="tab-ctx-item" @click="emit('closeOtherTabs', ctxMenu!.tabId); closeCtxMenu()">{{ t("session.tabBar.closeOthers") }}</button>
        <button class="tab-ctx-item" @click="emit('closeRightTabs', ctxMenu!.tabId); closeCtxMenu()">{{ t("session.tabBar.closeRight") }}</button>
        <button class="tab-ctx-item" @click="emit('closeAllTabs'); closeCtxMenu()">{{ t("session.tabBar.closeAll") }}</button>
        <div class="tab-ctx-sep" />
        <button class="tab-ctx-item" @click="emit('closeSavedTabs'); closeCtxMenu()">{{ t("session.tabBar.closeSaved") }}</button>
      </div>

      <!-- New Tab Dropdown Menu -->
      <div
        v-if="newMenuOpen"
        class="tab-dropdown-menu"
        :style="{ left: newMenuPos.x + 'px', top: newMenuPos.y + 'px' }"
        @click.stop
      >
        <button
          class="tab-menu-btn"
          @click="() => { emit('newTerminalTab'); closeNewMenu(); }"
        >
          <span class="tab-menu-label">{{ t("session.tabBar.newTerminalMenu") }}</span>
          <span class="tab-menu-shortcut">⌘T</span>
        </button>
        <button
          class="tab-menu-btn"
          @click="() => { emit('openNewSessionDialog'); closeNewMenu(); }"
        >
          <span class="tab-menu-label">{{ t("app.ctxMenu.newSession") }}</span>
        </button>
      </div>
    </Teleport>
  </div>
</template>

<style scoped>
.tab-bar {
  display: flex;
  align-items: center;
  min-height: 35px;
  height: 35px;
  border-bottom: 1px solid var(--color-border);
  background: var(--color-bg-sidebar);
  overflow: hidden;
  flex-shrink: 0;
  padding: 0 4px;
  user-select: none;
  -webkit-user-select: none;
}
.tab-scroll-shell {
  position: relative;
  display: flex;
  flex: 1;
  min-width: 0;
  height: 100%;
  overflow: hidden;
}
.tab-list {
  display: flex;
  align-items: center;
  gap: 3px;
  flex: 1;
  min-width: 0;
  height: 100%;
  padding: 2px 2px;
  overflow-x: auto;
  overflow-y: hidden;
  scrollbar-width: none;
  -ms-overflow-style: none;
}
.tab-list::-webkit-scrollbar {
  width: 0;
  height: 0;
}
.tab-scrollbar {
  position: absolute;
  left: 0;
  right: 0;
  bottom: 0;
  height: 2px;
  cursor: default;
  opacity: 0;
  transition: opacity var(--transition-fast);
  z-index: 2;
}
.tab-scroll-shell:hover .tab-scrollbar,
.tab-scroll-shell:focus-within .tab-scrollbar {
  opacity: 1;
}
.tab-scrollbar-thumb {
  height: 2px;
  border-radius: var(--radius-full);
  background: rgba(100, 116, 139, 0.48);
  cursor: ew-resize;
}
.tab-scrollbar-thumb:hover {
  background: rgba(100, 116, 139, 0.68);
}
.tab-item {
  display: flex;
  align-items: center;
  gap: 6px;
  height: 26px;
  padding: 0 7px 0 9px;
  font-size: var(--text-xs);
  color: var(--color-text-secondary);
  border-radius: 6px;
  border: 1px solid transparent;
  background: transparent;
  cursor: pointer;
  white-space: nowrap;
  transition: all var(--transition-fast);
}
.tab-item:hover {
  color: var(--color-text);
  background: var(--color-bg-hover);
}
.tab-item.active {
  color: var(--color-text);
  background: var(--color-surface);
  border-color: var(--color-border);
  box-shadow: var(--shadow-sm);
  font-weight: 500;
}
.tab-item.preview .tab-label {
  font-style: italic;
  opacity: 0.85;
}
.tab-status-dot {
  width: 6px;
  height: 6px;
  border-radius: var(--radius-full);
  flex-shrink: 0;
}
.tab-label {
  max-width: 140px;
  overflow: hidden;
  text-overflow: ellipsis;
}
.tab-close {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 15px;
  height: 15px;
  border-radius: var(--radius-sm);
  color: var(--color-text-muted);
  opacity: 0.4;
  cursor: pointer;
  transition: all var(--transition-fast);
}
.tab-item:hover .tab-close,
.tab-item.active .tab-close {
  opacity: 0.8;
}
.tab-close:hover {
  opacity: 1;
  background: var(--color-bg-active);
  color: var(--color-danger);
}

/* Context menu */
.tab-ctx-menu {
  position: fixed;
  z-index: 9999;
  background: var(--color-bg-elevated, var(--color-bg));
  border: 1px solid var(--color-border);
  border-radius: var(--radius-md);
  box-shadow: var(--shadow-context);
  min-width: 160px;
  padding: var(--space-1) 0;
}
.tab-ctx-item {
  display: flex;
  align-items: center;
  width: 100%;
  padding: var(--space-1) var(--space-3);
  font-size: var(--text-xs);
  color: var(--color-text);
  cursor: pointer;
  transition: background var(--transition-fast);
  text-align: left;
}
.tab-ctx-item:hover {
  background: var(--color-bg-hover);
}
.tab-ctx-sep {
  height: 1px;
  background: var(--color-border);
  margin: var(--space-1) 0;
}

/* CLI brand badge */
.tab-cli-badge {
  pointer-events: none;
}
/* ChatAvatar 根元素即 .chat-avatar，fallthrough class 落在同一元素上，
   容器尺寸须直写根选择器（父 scopeId 会应用到子组件根），
   加 .tab-item 前缀提升优先级，战胜 ChatAvatar 自带 .chat-avatar 规则 */
.tab-item .tab-cli-badge {
  width: 14px;
  height: 14px;
  border-radius: var(--radius-sm);
}
.tab-cli-badge :deep(.avatar-svg) {
  width: 10px;
  height: 10px;
}
.tab-cli-badge :deep(.brand-svg) {
  width: 12px;
  height: 12px;
}
.tab-cli-badge :deep(.brand-svg-fill) {
  width: 14px;
  height: 14px;
}
/* brand-codex / brand-workbuddy 类同样绑在根元素上，用根选择器覆盖圆角 */
.tab-item .tab-cli-badge.brand-codex,
.tab-item .tab-cli-badge.brand-workbuddy {
  border-radius: var(--radius-sm);
}

.tab-bar-action-btn {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 26px;
  height: 26px;
  margin-left: 4px;
  margin-right: 2px;
  padding: 0;
  border: 1px solid transparent;
  border-radius: var(--radius-md, 6px);
  background: transparent;
  color: var(--color-text-muted);
  cursor: pointer;
  flex-shrink: 0;
  transition: all var(--transition-fast);
}
.tab-bar-action-btn:hover {
  color: var(--color-text);
  background: var(--color-bg-hover);
  border-color: var(--color-border);
}
.tab-bar-action-btn.active {
  color: var(--color-primary);
  background: var(--color-surface);
  border-color: var(--color-border);
}

.tab-split-badge {
  font-size: 10px;
  padding: 1px 4px;
  border-radius: var(--radius-sm, 3px);
  background: var(--color-bg-hover);
  color: var(--color-text-muted);
  flex-shrink: 0;
}

.new-tab-actions {
  display: inline-flex;
  align-items: center;
  border-radius: var(--radius-md, 6px);
  background: var(--color-bg-hover);
  border: 1px solid var(--color-border);
  overflow: hidden;
  height: 24px;
  margin-left: 4px;
  margin-right: 2px;
  flex-shrink: 0;
}

.new-tab-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  height: 100%;
  border: none;
  background: transparent;
  color: var(--color-text-muted);
  cursor: pointer;
  padding: 0;
  transition: all var(--transition-fast);
}

.new-tab-btn.main {
  width: 22px;
}

.new-tab-btn.dropdown {
  width: 16px;
  border-left: 1px solid var(--color-border);
}

.new-tab-btn:hover {
  background: var(--color-bg-active, rgba(255, 255, 255, 0.12));
  color: var(--color-text);
}

.ctx-shortcut {
  margin-left: auto;
  font-size: 10px;
  color: var(--color-text-muted);
  opacity: 0.7;
}

.tab-dropdown-menu {
  position: fixed;
  z-index: 9999;
  min-width: 140px;
  background: var(--color-bg);
  border: 1px solid var(--color-border);
  border-radius: var(--radius-lg, 8px);
  box-shadow: var(--shadow-lg, 0 10px 25px rgba(0, 0, 0, 0.4));
  padding: 4px;
  display: flex;
  flex-direction: column;
  gap: 2px;
  user-select: none;
  -webkit-user-select: none;
}

.tab-menu-btn {
  display: flex;
  align-items: center;
  justify-content: space-between;
  width: 100%;
  padding: 6px 10px;
  border: none;
  background: transparent;
  color: var(--color-text);
  font-size: var(--text-xs, 12px);
  font-weight: 500;
  border-radius: var(--radius-md, 6px);
  cursor: pointer;
  text-align: left;
  white-space: nowrap;
  transition: all var(--transition-fast);
}

.tab-menu-btn:hover {
  background: var(--color-primary);
  color: #ffffff;
}

.tab-menu-label {
  white-space: nowrap;
  flex-shrink: 0;
}

.tab-menu-shortcut {
  margin-left: 14px;
  font-size: 11px;
  color: var(--color-text-muted);
  flex-shrink: 0;
}

.tab-menu-btn:hover .tab-menu-shortcut {
  color: rgba(255, 255, 255, 0.85);
}
</style>

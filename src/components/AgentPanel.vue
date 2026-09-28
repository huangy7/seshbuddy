<script setup lang="ts">
import { computed, ref } from "vue";
import SvgIcon from "./icons/SvgIcon.vue";
import { usePtySession } from "../composables/usePtySession";
import { t } from "../i18n";
import type { PtyStatus } from "../types/pty";
import { tabLabelText, type OpenTab } from "../composables/useTabs";
import { leafIds, findLeaf, firstLeafId } from "../utils/panes";
import { basename } from "../utils/projectPath";
import { formatRelativeTime } from "../utils/format";

const props = defineProps<{
  tabs?: OpenTab[];
  activeTabId?: string | null;
}>();

const emit = defineEmits<{
  selectTab: [tabId: string];
  renameTab: [tabId: string, newLabel: string];
  openRenameDialog: [tabId: string, currentLabel: string];
  closeTab: [tabId: string];
  newSession: [];
  openProject: [projectPath: string];
  openDashboard: [];
  splitRight: [tabId: string];
  splitDown: [tabId: string];
}>();

const { sessions } = usePtySession();

// Group open terminal tabs by project path
const groupedTabs = computed(() => {
  const list = props.tabs ?? [];
  const map = new Map<string, OpenTab[]>();
  for (const tab of list) {
    const p = tab.projectRoot || (tab.rootPane?.kind === "leaf" ? tab.rootPane.projectPath : "") || "";
    const items = map.get(p) ?? [];
    items.push(tab);
    map.set(p, items);
  }
  return map;
});

const totalTabCount = computed(() => (props.tabs ?? []).length);

function getTabSplitCount(tab: OpenTab): number {
  return tab.rootPane ? leafIds(tab.rootPane).length : 1;
}

function getTabCliKind(tab: OpenTab): string {
  if (tab.rootPane) {
    const leaf = findLeaf(tab.rootPane, tab.activePaneId || firstLeafId(tab.rootPane));
    if (leaf?.cliKind) return leaf.cliKind;
  }
  if (tab.sessionId) {
    const s = sessions.value.find((sess) => sess.sessionId === tab.sessionId);
    if (s?.cliKind) return s.cliKind;
  }
  return "shell";
}

function getTabStatus(tab: OpenTab): PtyStatus {
  const sids: string[] = [];
  if (tab.rootPane) {
    for (const lid of leafIds(tab.rootPane)) {
      const leaf = findLeaf(tab.rootPane, lid);
      if (leaf?.sessionId) sids.push(leaf.sessionId);
    }
  } else if (tab.sessionId) {
    sids.push(tab.sessionId);
  }

  const ptyList = sessions.value.filter((s) => sids.includes(s.sessionId));
  if (ptyList.some((s) => s.status === "waiting_input")) return "waiting_input";
  if (ptyList.some((s) => s.status === "active")) return "active";
  if (ptyList.some((s) => s.status === "idle")) return "idle";
  if (ptyList.length > 0) return "exited";
  return "active";
}

function getTabCreatedAt(tab: OpenTab): string {
  if (tab.sessionId) {
    const s = sessions.value.find((sess) => sess.sessionId === tab.sessionId);
    if (s?.createdAt) return s.createdAt;
  }
  return new Date().toISOString();
}

function findTabById(tabId: string): OpenTab | undefined {
  return (props.tabs ?? []).find((t) => t.id === tabId);
}

function startRename(tab: OpenTab) {
  closeContextMenu();
  emit("openRenameDialog", tab.id, tabLabelText(tab.label));
}

// Context menu
const ctxMenu = ref({ visible: false, x: 0, y: 0, tabId: "", projectPath: "" });

function showContextMenu(e: MouseEvent, tab: OpenTab) {
  e.preventDefault();
  e.stopPropagation();
  const projectPath = tab.projectRoot || (tab.rootPane?.kind === "leaf" ? tab.rootPane.projectPath : "") || "";
  ctxMenu.value = { visible: true, x: e.clientX, y: e.clientY, tabId: tab.id, projectPath };
  window.addEventListener("click", closeContextMenu, { once: true });
}

function closeContextMenu() {
  ctxMenu.value.visible = false;
}

function openProjectDir() {
  if (ctxMenu.value.projectPath) {
    emit("openProject", ctxMenu.value.projectPath);
  }
  closeContextMenu();
}

function closeFromMenu() {
  emit("closeTab", ctxMenu.value.tabId);
  closeContextMenu();
}

function statusColor(status: PtyStatus): string {
  switch (status) {
    case "active": return "var(--color-success)";
    case "idle": return "var(--color-warning)";
    case "waiting_input": return "var(--color-danger)";
    case "exited": return "var(--color-text-muted)";
  }
}

function statusLabel(status: PtyStatus): string {
  switch (status) {
    case "active": return t("session.agentPanel.statusActive");
    case "idle": return t("session.agentPanel.statusIdle");
    case "waiting_input": return t("session.agentPanel.statusWaitingInput");
    case "exited": return t("session.agentPanel.statusExited");
  }
}

function projectName(path: string): string {
  if (!path) return t("dialogs.common.terminal");
  return basename(path) || path;
}
</script>

<template>
  <div class="agent-panel">
    <div class="agent-panel-header">
      <span class="agent-panel-title">{{ t("session.agentPanel.title") }}</span>
      <button class="agent-panel-add agent-panel-dashboard" :title="t('session.agentPanel.openDashboard')" @click="emit('openDashboard')">
        <SvgIcon name="maximize-2" :size="14" />
      </button>
      <button class="agent-panel-add" :title="t('app.ctxMenu.newSession')" @click="emit('newSession')">
        <SvgIcon name="plus" :size="14" />
      </button>
    </div>

    <div class="agent-list" v-if="totalTabCount > 0">
      <div
        v-for="[projectPath, projectTabs] in groupedTabs"
        :key="projectPath"
        class="project-group"
      >
        <!-- Project header -->
        <div class="project-header">
          <SvgIcon name="folder" :size="12" />
          <span class="project-header-name" :title="projectPath">{{ projectName(projectPath) }}</span>
        </div>

        <!-- Terminal Tabs under this project -->
        <div
          v-for="tab in projectTabs"
          :key="tab.id"
          class="agent-item"
          :class="{
            active: tab.id === activeTabId,
            'is-waiting': getTabStatus(tab) === 'waiting_input'
          }"
          @click="emit('selectTab', tab.id)"
          @contextmenu="showContextMenu($event, tab)"
          @dblclick.stop="startRename(tab)"
        >
          <span class="agent-status-dot" :style="{ background: statusColor(getTabStatus(tab)) }"></span>

          <div class="agent-item-info">
            <div class="agent-item-title-row">
              <span class="agent-item-name" :title="tabLabelText(tab.label)">{{ tabLabelText(tab.label) }}</span>
              <span v-if="getTabSplitCount(tab) > 1" class="agent-split-tag">
                {{ t("session.tabBar.splitBadge", { count: getTabSplitCount(tab) }) }}
              </span>
            </div>
            <span class="agent-item-meta">
              {{ getTabCliKind(tab) }} · {{ statusLabel(getTabStatus(tab)) }} · {{ formatRelativeTime(getTabCreatedAt(tab)) }}
            </span>
          </div>

          <button
            class="agent-item-close"
            :title="t('session.tabBar.closeTab')"
            @click.stop="emit('closeTab', tab.id)"
          >
            <SvgIcon name="x" :size="12" />
          </button>
        </div>
      </div>
    </div>

    <div v-else class="agent-empty">
      <p>{{ t("session.agentPanel.empty") }}</p>
      <button class="agent-empty-btn" @click="emit('newSession')">
        <SvgIcon name="plus" :size="14" />
        {{ t("app.ctxMenu.newSession") }}
      </button>
    </div>

    <Teleport to="body">
      <div
        v-if="ctxMenu.visible"
        class="ctx-menu"
        :style="{ left: ctxMenu.x + 'px', top: ctxMenu.y + 'px' }"
        @click.stop
      >
        <button class="ctx-item" @click="() => { emit('selectTab', ctxMenu.tabId); closeContextMenu(); }">
          <SvgIcon name="terminal" :size="13" />
          {{ t("session.agentPanel.openTerminal") }}
        </button>
        <!-- 局部变量避开 t：模板里的 t 是 i18n 的翻译函数，重名会让后续在此处写翻译调用时静默调到查找结果上 -->
        <button class="ctx-item" @click="() => { const tab = findTabById(ctxMenu.tabId); if (tab) startRename(tab); }">
          <SvgIcon name="pencil" :size="13" />
          {{ t("app.ctxMenu.rename") }}
        </button>
        <div class="ctx-separator" />
        <button class="ctx-item" @click="() => { emit('splitRight', ctxMenu.tabId); closeContextMenu(); }">
          <SvgIcon name="columns" :size="13" />
          <span>{{ t("session.tabBar.splitRight") }}</span>
          <span class="ctx-shortcut">⌘D</span>
        </button>
        <button class="ctx-item" @click="() => { emit('splitDown', ctxMenu.tabId); closeContextMenu(); }">
          <SvgIcon name="rows" :size="13" />
          <span>{{ t("session.tabBar.splitDown") }}</span>
          <span class="ctx-shortcut">⌘⇧D</span>
        </button>
        <template v-if="ctxMenu.projectPath">
          <div class="ctx-separator" />
          <button class="ctx-item" @click="openProjectDir">
            <SvgIcon name="folder" :size="13" />
            {{ t("session.agentPanel.openProjectDir") }}
          </button>
        </template>
        <div class="ctx-separator" />
        <button class="ctx-item ctx-item-danger" @click="closeFromMenu">
          <SvgIcon name="x" :size="13" />
          <span>{{ t("session.tabBar.closeTab") }}</span>
          <span class="ctx-shortcut">⌘W</span>
        </button>
      </div>
    </Teleport>
  </div>
</template>

<style scoped>
.agent-panel {
  display: flex;
  flex-direction: column;
  height: 100%;
  user-select: none;
  -webkit-user-select: none;
}
.agent-panel-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-1);
  padding: var(--space-2) var(--space-3);
  border-bottom: 1px solid var(--color-border);
}
.agent-panel-title {
  font-size: var(--text-xs);
  font-weight: 600;
  color: var(--color-text-muted);
  text-transform: uppercase;
  letter-spacing: 0.5px;
}
.agent-panel-add {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 24px;
  height: 24px;
  border-radius: var(--radius-sm);
  color: var(--color-text-muted);
  cursor: pointer;
  transition: all var(--transition-fast);
}
.agent-panel-add:hover {
  background: var(--color-bg-hover);
  color: var(--color-text);
}
.agent-panel-dashboard {
  margin-left: auto;
}

/* Scrollable list */
.agent-list {
  flex: 1;
  overflow-y: auto;
  padding: var(--space-1) 0;
}

/* Project group */
.project-group {
  margin-bottom: var(--space-1);
}
.project-header {
  display: flex;
  align-items: center;
  gap: var(--space-1);
  min-height: 30px;
  height: 30px;
  box-sizing: border-box;
  padding: 0 8px;
  margin: 1px 6px;
  border-radius: 7px;
  color: var(--color-text-muted);
}
.project-header-name {
  font-size: 11px;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.4px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

/* Tab item */
.agent-item {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  min-height: 30px;
  box-sizing: border-box;
  padding: 4px 8px;
  margin: 1px 6px;
  border-radius: 7px;
  cursor: pointer;
  transition: background var(--transition-fast);
  border: 1px solid transparent;
}
.agent-item:hover {
  background: var(--color-bg-hover);
}
.agent-item.active {
  background: var(--color-bg-hover);
  border-color: var(--color-border);
}
.agent-item.is-waiting {
  animation: agent-shake 0.5s ease-in-out;
}
@keyframes agent-shake {
  0%, 100% { transform: translateX(0); }
  25% { transform: translateX(-2px); }
  75% { transform: translateX(2px); }
}
.agent-status-dot {
  width: 8px;
  height: 8px;
  border-radius: var(--radius-full);
  flex-shrink: 0;
}
.agent-item-info {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 2px;
}
.agent-item-title-row {
  display: flex;
  align-items: center;
  gap: 6px;
  min-width: 0;
}
.agent-item-name {
  font-size: var(--text-sm);
  font-weight: 500;
  color: var(--color-text);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.agent-split-tag {
  font-size: 10px;
  padding: 0 4px;
  border-radius: var(--radius-sm, 3px);
  background: var(--color-bg-secondary);
  color: var(--color-text-muted);
  flex-shrink: 0;
}
.agent-item-meta {
  font-size: var(--text-2xs);
  color: var(--color-text-muted);
}

.agent-item-close {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 20px;
  height: 20px;
  border-radius: var(--radius-sm);
  color: var(--color-text-muted);
  opacity: 0;
  cursor: pointer;
  transition: all var(--transition-fast);
  flex-shrink: 0;
}
.agent-item:hover .agent-item-close {
  opacity: 1;
}
.agent-item-close:hover {
  background: var(--color-bg-active);
  color: var(--color-danger);
}

/* Empty state */
.agent-empty {
  flex: 1;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: var(--space-3);
  color: var(--color-text-muted);
  font-size: var(--text-xs);
}
.agent-empty-btn {
  display: flex;
  align-items: center;
  gap: var(--space-1);
  padding: var(--space-1) var(--space-3);
  font-size: var(--text-xs);
  color: var(--color-primary);
  background: var(--color-primary-light);
  border: 1px solid var(--color-primary);
  border-radius: var(--radius-md);
  cursor: pointer;
  transition: all var(--transition-fast);
}
.agent-empty-btn:hover {
  background: var(--color-primary);
  color: white;
}

/* Context menu */
.ctx-menu {
  position: fixed;
  z-index: 9999;
  min-width: 160px;
  padding: var(--space-1);
  background: var(--color-bg);
  border: 1px solid var(--color-border);
  border-radius: var(--radius-lg);
  box-shadow: var(--shadow-context);
}
.ctx-item {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  width: 100%;
  padding: var(--space-2) var(--space-2);
  font-size: var(--text-xs);
  color: var(--color-text);
  border-radius: var(--radius-sm);
  cursor: pointer;
  transition: background var(--transition-fast);
  text-align: left;
}
.ctx-item:hover {
  background: var(--color-bg-hover);
}
.ctx-shortcut {
  margin-left: auto;
  font-size: 10px;
  color: var(--color-text-muted);
  opacity: 0.7;
}
.ctx-item-danger {
  color: var(--color-danger);
}
.ctx-item-danger:hover {
  background: rgba(220, 38, 38, 0.08);
}
.ctx-separator {
  height: 1px;
  background: var(--color-border);
  margin: var(--space-1) 0;
}
</style>

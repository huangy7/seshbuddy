<script setup lang="ts">
import { computed, ref } from "vue";
import SvgIcon from "./icons/SvgIcon.vue";
import { useUpdater } from "../composables/useUpdater";
import { useActivityBar } from "../composables/useActivityBar";
import type { ActivityTab } from "../types/activity";
import { t } from "../i18n";

const { hasAvailableUpdate } = useUpdater();
const { isExpanded, railWidth, toggleExpanded, startResize } = useActivityBar();

const props = defineProps<{
  activeMode: ActivityTab;
  sidebarCollapsed: boolean;
  agentBadge?: number;
  showUsage?: boolean;
  showApiDebug?: boolean;
}>();

const emit = defineEmits<{
  "update:activeMode": [mode: ActivityTab];
  toggleSidebar: [];
  openSettings: [];
  openUsage: [];
  openApiDebug: [];
  openAssistant: [];
  openFeedback: [];
}>();

// 标签文案在 computed 内取：模块级数组会把文案冻结在 import 时的语言
const tabs = computed<{ id: ActivityTab; icon: string; label: string }[]>(() => [
  { id: "history", icon: "home", label: t("session.activityBar.tabHistory") },
  { id: "favorites", icon: "star", label: t("session.favoritesPanel.title") },
  { id: "bookmarks", icon: "bookmark", label: t("session.bookmarkList.title") },
  { id: "agent", icon: "square-terminal", label: t("session.activityBar.tabAgent") },
]);

function onTabClick(tab: ActivityTab) {
  if (props.activeMode === tab && !props.sidebarCollapsed) {
    emit("toggleSidebar");
  } else {
    emit("update:activeMode", tab);
  }
}

function onFeedbackClick() {
  emit("openFeedback");
}

// ─── 经典箭头就地展开全部工具 ───
const toolsExpanded = ref(false);

defineExpose({
  isExpanded,
  railWidth,
  toggleExpanded,
  toolsExpanded,
});
</script>

<template>
  <div
    class="activity-bar"
    :class="{ expanded: isExpanded }"
    :style="{ width: railWidth + 'px', minWidth: railWidth + 'px' }"
  >
    <!-- 右侧拖拽拉手 -->
    <div
      class="rail-resizer"
      :title="t('session.activityBar.resizerTitle')"
      @mousedown="startResize"
      @dblclick="toggleExpanded"
    />

    <!-- 1. 顶部对话组 & Agent -->
    <div class="activity-tabs">
      <button
        v-for="tab in tabs"
        :key="tab.id"
        class="activity-item"
        :class="{ active: activeMode === tab.id && !sidebarCollapsed }"
        :title="tab.label"
        @click="onTabClick(tab.id)"
      >
        <SvgIcon :name="tab.icon" :size="18" class="item-icon" />
        <span v-if="isExpanded" class="item-label">{{ tab.label }}</span>
        <span
          v-if="tab.id === 'agent' && agentBadge && agentBadge > 0"
          class="activity-badge"
          :class="{ 'badge-inline': isExpanded }"
        >{{ agentBadge > 9 ? '9+' : agentBadge }}</span>
      </button>
    </div>

    <!-- 2. 底部工具组（带经典箭头就地展开次要工具，用量统计常驻在下方不折叠） -->
    <div class="activity-bottom">
      <!-- 箭头切换展开/收起次要工具 -->
      <button
        class="activity-item arrow-toggle-btn"
        :title="toolsExpanded ? t('session.activityBar.collapseSecondaryTools') : t('session.activityBar.expandAllTools')"
        @click="toolsExpanded = !toolsExpanded"
      >
        <SvgIcon :name="toolsExpanded ? 'chevron-down' : 'chevron-up'" :size="14" class="item-icon" />
        <span v-if="isExpanded" class="item-label sub-label">{{ toolsExpanded ? t('session.activityBar.hideTools') : t('session.activityBar.moreTools') }}</span>
      </button>

      <!-- 次要工具：展开时平滑滑出 -->
      <template v-if="toolsExpanded">
        <button
          v-if="showApiDebug !== false"
          class="activity-item sub-tool"
          :title="t('dialogs.apiDebug.title')"
          @click="emit('openApiDebug')"
        >
          <SvgIcon name="zap" :size="18" class="item-icon" />
          <span v-if="isExpanded" class="item-label">{{ t('session.activityBar.apiDebugShort') }}</span>
        </button>

        <button
          class="activity-item sub-tool"
          :title="t('app.about.feedbackTitle')"
          @click="onFeedbackClick"
        >
          <SvgIcon name="github" :size="18" class="item-icon" />
          <span v-if="isExpanded" class="item-label">{{ t('session.activityBar.feedbackShort') }}</span>
        </button>
      </template>

      <!-- 常驻核心工具：用量统计 (放下面，不折叠) / 助手 / KPI -->
      <button
        v-if="showUsage !== false"
        class="activity-item"
        :title="t('session.usageDashboard.title')"
        @click="emit('openUsage')"
      >
        <SvgIcon name="bar-chart-2" :size="18" class="item-icon" />
        <span v-if="isExpanded" class="item-label">{{ t('session.activityBar.usageShort') }}</span>
      </button>

      <button
        class="activity-item"
        :title="t('native.assistantWindowTitle')"
        @click="emit('openAssistant')"
      >
        <SvgIcon name="bot" :size="18" class="item-icon" />
        <span v-if="isExpanded" class="item-label">{{ t('app.filter.assistant') }}</span>
      </button>

      <!-- 分隔细线 -->
      <div class="rail-divider" />

      <!-- 设置 -->
      <button
        class="activity-item settings-btn"
        :title="t('session.activityBar.settings')"
        @click="emit('openSettings')"
      >
        <SvgIcon name="settings" :size="18" class="item-icon" />
        <span v-if="isExpanded" class="item-label">{{ t('session.activityBar.settings') }}</span>
        <span v-if="hasAvailableUpdate" class="red-dot" :class="{ 'dot-inline': isExpanded }" />
      </button>
    </div>
  </div>
</template>

<style scoped>
.activity-bar {
  display: flex;
  flex-direction: column;
  align-items: center;
  background: transparent;
  padding-top: 10px;
  position: relative;
  flex-shrink: 0;
  -webkit-user-select: none;
  user-select: none;
  transition: width var(--transition-fast) var(--ease-standard);
  overflow: visible;
  z-index: 50;
}

.rail-resizer {
  position: absolute;
  top: 0;
  bottom: 0;
  right: 0;
  width: 4px;
  cursor: col-resize;
  z-index: 20;
  transition: background var(--transition-fast);
}
.rail-resizer:hover {
  background: var(--color-primary);
  opacity: 0.5;
}

.activity-tabs {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: var(--space-1);
  width: 100%;
  flex: 1;
  margin-bottom: var(--space-2);
}

.activity-bottom {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: var(--space-1);
  width: 100%;
  padding-bottom: var(--space-2);
}

.activity-item {
  position: relative;
  display: flex;
  align-items: center;
  justify-content: center;
  width: 36px;
  height: 36px;
  border: none;
  border-radius: var(--radius-md);
  color: var(--color-text-muted);
  background: transparent;
  cursor: pointer;
  transition: color var(--transition-fast), background var(--transition-fast), width var(--transition-fast), transform 0.1s var(--ease-standard), opacity 0.1s var(--ease-standard);
  text-decoration: none;
  outline: none;
  flex-shrink: 0;
  overflow: hidden;
}

.activity-bar.expanded .activity-item {
  width: calc(100% - 12px);
  margin: 0 6px;
  padding: 0 8px;
  justify-content: flex-start;
  gap: 10px;
}

.activity-item:hover {
  color: var(--color-text);
  background: var(--color-bg-hover);
}

.activity-item:active {
  transform: scale(0.92);
  opacity: 0.85;
}

.activity-item.active {
  color: var(--color-activity-bar-active);
  background: var(--color-bg-active);
}

.arrow-toggle-btn {
  height: 24px;
  opacity: 0.6;
  transition: opacity var(--transition-fast), color var(--transition-fast);
}
.arrow-toggle-btn:hover {
  opacity: 1;
  color: var(--color-text);
}

.sub-tool {
  animation: slideDown 0.15s var(--ease-standard);
}

@keyframes slideDown {
  from {
    opacity: 0;
    transform: translateY(-4px);
  }
  to {
    opacity: 1;
    transform: translateY(0);
  }
}

.rail-divider {
  width: calc(100% - 16px);
  height: 1px;
  margin: 4px auto;
  background: var(--color-border);
  opacity: 0.6;
}

.item-icon {
  flex-shrink: 0;
}

.item-label {
  font-size: var(--text-sm);
  font-weight: 600;
  white-space: nowrap;
  color: inherit;
  line-height: 1;
}

.item-label.sub-label {
  font-size: var(--text-xs);
  font-weight: 500;
  opacity: 0.7;
}

.activity-badge {
  position: absolute;
  top: 2px;
  right: 2px;
  min-width: 16px;
  height: 16px;
  padding: 0 4px;
  border-radius: var(--radius-full);
  background: var(--color-danger);
  color: white;
  font-size: 10px;
  font-weight: 700;
  display: flex;
  align-items: center;
  justify-content: center;
  line-height: 1;
}

.activity-badge.badge-inline {
  position: static;
  margin-left: auto;
}

.settings-btn {
  position: relative;
}

.red-dot {
  position: absolute;
  top: 4px;
  right: 4px;
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: var(--color-danger);
}

.red-dot.dot-inline {
  position: static;
  margin-left: auto;
}

.toggle-expand-btn {
  color: var(--color-text-muted);
}
.toggle-expand-btn:hover {
  color: var(--color-text);
}
</style>

<script setup lang="ts">
import { computed, ref, watch } from "vue";
import SvgIcon from "./icons/SvgIcon.vue";
import GeneralSettings from "./GeneralSettings.vue";
import DataSourceSettings from "./DataSourceSettings.vue";
import ApiProfileManager from "./ApiProfileManager.vue";
import AboutView from "./AboutView.vue";
import DataIndexSettings from "./DataIndexSettings.vue";
import { useSessions } from "../composables/useSessions";
import { useUpdater } from "../composables/useUpdater";
import { t } from "../i18n";
import type { SessionIdentity } from "../types/session";
import type { CliId } from "../types/cli";

type SettingsTab = "general" | "dataSources" | "data" | "api" | "about";
type SettingsTabInfo = {
  id: SettingsTab;
  label: string;
  visible: boolean;
  hasRedDot?: boolean;
};

const props = withDefaults(defineProps<{
  initialTab?: SettingsTab;
  initialCliId?: CliId;
}>(), {
  initialTab: "general",
});

const emit = defineEmits<{
  close: [];
  registerMenu: [];
  unregisterMenu: [];
  openSession: [identity: SessionIdentity];
  openSearch: [];
  openFeedback: [];
}>();

const activeTab = ref<SettingsTab>(props.initialTab);
const maximized = ref(false);
const minimized = ref(false);
const sidebarCollapsed = ref(false);
const { cliOptions } = useSessions();
const { hasAvailableUpdate } = useUpdater();

const apiRef = ref<InstanceType<typeof ApiProfileManager> | null>(null);
const mountedTabs = ref<Record<SettingsTab, boolean>>({
  general: true,
  dataSources: false,
  data: false,
  api: false,
  about: false,
});

function ensureTabMounted(tab: SettingsTab) {
  if (mountedTabs.value[tab]) return;
  mountedTabs.value = {
    ...mountedTabs.value,
    [tab]: true,
  };
}

function switchTab(tab: SettingsTab) {
  ensureTabMounted(tab);
  activeTab.value = tab;
}

function restoreFromMinimized() {
  minimized.value = false;
}

defineExpose({
  restoreFromMinimized
});

watch(
  () => props.initialTab,
  (tab) => {
    switchTab(tab);
  },
);

// 标签在 computed 内求值而非写成模块级常量：t 读的是响应式的 locale，
// 写在求值之外的文案会冻结在 import 时的语言。
const tabs = computed<SettingsTabInfo[]>(() => {
  const items: SettingsTabInfo[] = [
    { id: "general", label: t("settings.nav.tabGeneral"), visible: true },
    { id: "dataSources", label: t("settings.nav.tabDataSources"), visible: true },
    { id: "data", label: t("settings.nav.tabData"), visible: true },
    { id: "api", label: t("settings.nav.tabApi"), visible: cliOptions.value.some((cli) => cli.supportsApiProfiles) },
    { id: "about", label: t("settings.nav.tabAbout"), visible: true, hasRedDot: hasAvailableUpdate.value },
  ];

  return items.filter((tab) => tab.visible);
});

watch(
  tabs,
  (nextTabs) => {
    if (!nextTabs.some((tab) => tab.id === activeTab.value)) {
      activeTab.value = "general";
    }
    ensureTabMounted(activeTab.value);
  },
  { immediate: true },
);

function getIcon(tabId: SettingsTab) {
  switch (tabId) {
    case 'general': return 'settings';
    case 'dataSources': return 'database';
    case 'data': return 'archive';
    case 'api': return 'code';
    case 'about': return 'info';
    default: return 'settings';
  }
}
</script>

<template>
  <Transition name="fade">
    <div class="modal-backdrop" :class="{ 'modal-backdrop--maximized': maximized, 'modal-backdrop--minimized': minimized }" @contextmenu.prevent>
      <div v-show="!minimized" class="settings-window" :class="{ 'settings-window-maximized': maximized }" @contextmenu.prevent>
        
        <!-- Left Sidebar -->
        <div class="settings-sidebar" :class="{ 'collapsed': sidebarCollapsed }">
          <div class="sidebar-header">
            <h2 class="settings-title">{{ t("session.activityBar.settings") }}</h2>
          </div>
          <div class="sidebar-tabs">
            <button
              v-for="tab in tabs"
              :key="tab.id"
              class="sidebar-tab"
              :class="{ active: activeTab === tab.id }"
              @click="switchTab(tab.id)"
            >
              <SvgIcon :name="getIcon(tab.id)" :size="16" class="tab-icon"/>
              <span class="tab-label">{{ tab.label }}</span>
              <span v-if="tab.hasRedDot" class="red-dot"></span>
            </button>
          </div>
        </div>

        <!-- Right Main Content -->
        <div class="settings-main">
          <div class="main-header">
            <div class="header-left">
              <button class="close-btn icon-btn sidebar-toggle-btn" @click="sidebarCollapsed = !sidebarCollapsed" :title="t('settings.nav.toggleSidebar')">
                <SvgIcon name="sidebar" :size="16" />
              </button>
              <h3 class="main-title">{{ tabs.find(tab => tab.id === activeTab)?.label }}</h3>
            </div>
            <div class="header-actions">
              <button class="close-btn icon-btn" @click="minimized = true" :title="t('dialogs.apiDebug.minimize')">
                <SvgIcon name="minus" :size="16" />
              </button>
              <button class="close-btn icon-btn" @click="maximized = !maximized" :title="maximized ? t('dialogs.apiDebug.restore') : t('dialogs.apiDebug.maximize')">
                <SvgIcon :name="maximized ? 'minimize-2' : 'maximize-2'" :size="16" />
              </button>
              <button class="close-btn icon-btn" @click="emit('close')" :title="t('dialogs.common.close')">
                <SvgIcon name="x" :size="18" />
              </button>
            </div>
          </div>

          <div class="settings-body">
            <div v-if="mountedTabs.general" v-show="activeTab === 'general'" class="settings-tab-panel">
              <GeneralSettings
                :initial-cli-id="props.initialCliId"
                @register-menu="emit('registerMenu')"
                @unregister-menu="emit('unregisterMenu')"
                @close="emit('close')"
              />
            </div>

            <div v-if="mountedTabs.dataSources" v-show="activeTab === 'dataSources'" class="settings-tab-panel">
              <DataSourceSettings />
            </div>

            <div v-if="mountedTabs.data" v-show="activeTab === 'data'" class="settings-tab-panel">
              <DataIndexSettings
                :initial-cli-id="props.initialCliId"
                @open-session="emit('openSession', $event)"
                @open-search="emit('openSearch')"
                @close-settings="emit('close')"
              />
            </div>

            <div v-if="mountedTabs.api" v-show="activeTab === 'api'" class="settings-tab-panel">
              <ApiProfileManager ref="apiRef" :initial-cli-id="props.initialCliId" />
            </div>

            <div v-if="mountedTabs.about" v-show="activeTab === 'about'" class="settings-tab-panel">
              <AboutView
                :active="activeTab === 'about'"
                @openFeedback="emit('openFeedback')"
              />
            </div>
          </div>

          <div class="settings-footer" v-if="activeTab === 'api' && apiRef?.editingProfile">
            <span v-if="apiRef?.editSaveSuccess" class="save-hint">{{ t("api-profile.rawEditor.saved") }}</span>
            <button class="btn-secondary" @click="apiRef?.closeEditor()">{{ t("dialogs.common.cancel") }}</button>
            <button v-if="!apiRef?.isEditingActiveProfile" class="btn-primary" :disabled="!!apiRef?.editJsonError || apiRef?.editSaving" @click="apiRef?.saveAndApplyEditingProfile()">
              {{ apiRef?.editSaving ? t('settings.nav.saving') : t('settings.nav.saveAndApply') }}
            </button>
            <button class="btn-primary" :disabled="!!apiRef?.editJsonError || apiRef?.editSaving" @click="apiRef?.saveEditingProfile()">
              {{ apiRef?.editSaving ? t('settings.nav.saving') : t('app.dialog.save') }}
            </button>
          </div>
          <div class="settings-footer" v-else-if="activeTab === 'general' || activeTab === 'dataSources' || activeTab === 'data' || activeTab === 'about'">
            <button class="btn-secondary" @click="emit('close')">{{ t("dialogs.common.close") }}</button>
          </div>
        </div>

      </div>
    </div>
  </Transition>

  <Teleport v-if="minimized" to="#minimized-widgets">
    <Transition name="fade">
      <div class="minimized-widget dialog-minimized-widget" @click="minimized = false">
        <SvgIcon name="settings" :size="20" class="minimized-icon" />
        <div class="minimized-info">
          <div class="minimized-title">{{ t("session.activityBar.settings") }}</div>
          <div class="minimized-status">{{ t("dialogs.apiDebug.runningInBackground") }}</div>
        </div>
        <button class="close-btn icon-btn" style="margin-left:8px;" @click.stop="emit('close')" :title="t('dialogs.common.close')">
          <SvgIcon name="x" :size="16" />
        </button>
      </div>
    </Transition>
  </Teleport>
</template>

<style scoped>
/* 共享样式见 src/styles/utilities.css 的 .minimized-widget 系列；
   以下仅保留设置 widget 与共享基础样式的差异项。 */
.dialog-minimized-widget {
  padding: 12px 16px;
  gap: 12px;
  box-shadow: var(--shadow-md);
  transition: all var(--transition-base);
}

.dialog-minimized-widget:hover {
  transform: translateY(-2px);
  box-shadow: 0 6px 16px rgba(0, 0, 0, 0.12);
  border-color: var(--color-primary-light);
}

.dialog-minimized-widget .minimized-icon {
  filter: none;
}

.dialog-minimized-widget .minimized-info {
  justify-content: normal;
  gap: 2px;
}

.dialog-minimized-widget .minimized-title {
  font-size: 13px;
  line-height: 1;
}

.dialog-minimized-widget .minimized-status {
  margin-top: 0;
  color: var(--color-text-muted);
  display: flex;
  align-items: center;
  gap: 6px;
}
.settings-window {
  width: 820px;
  max-width: 92vw;
  height: 600px;
  max-height: 88vh;
  background: var(--color-bg);
  border-radius: var(--radius-xl);
  box-shadow: 0 20px 40px rgba(0, 0, 0, 0.15), 0 0 0 1px rgba(0,0,0,0.05);
  display: flex;
  flex-direction: row;
  overflow: hidden;
  transition: width var(--transition-base), height var(--transition-base);
}
.settings-window-maximized {
  width: 100vw;
  max-width: 100vw;
  height: 100vh;
  max-height: 100vh;
  border-radius: 0;
  box-shadow: none;
}

/* Sidebar */
.settings-sidebar {
  width: 200px;
  background: var(--color-bg-hover);
  border-right: 1px solid var(--color-border);
  display: flex;
  flex-direction: column;
  flex-shrink: 0;
  transition: width 0.3s cubic-bezier(0.4, 0, 0.2, 1);
  overflow: hidden;
}
.settings-sidebar.collapsed {
  width: 0;
  border-right: none;
}
.sidebar-header {
  padding: 20px 16px 12px;
  min-width: 200px;
  box-sizing: border-box;
}
.settings-title {
  font-size: 20px;
  font-weight: 600;
  margin: 0;
  color: var(--color-text);
  letter-spacing: -0.02em;
}
.sidebar-tabs {
  display: flex;
  flex-direction: column;
  padding: 0 10px;
  gap: 4px;
  min-width: 200px;
  box-sizing: border-box;
}
.sidebar-tab {
  position: relative;
  display: flex;
  align-items: center;
  padding: 8px 12px;
  font-size: 14px;
  font-weight: 500;
  color: var(--color-text);
  background: transparent;
  border-radius: var(--radius-lg);
  border: none;
  cursor: pointer;
  transition: background 0.15s, color 0.15s;
  text-align: left;
}
.sidebar-tab:hover {
  background: rgba(0, 0, 0, 0.05);
}
.sidebar-tab.active {
  background: var(--color-primary);
  color: white;
}
.sidebar-tab.active .tab-icon {
  color: white;
}
.tab-icon {
  margin-right: 8px;
  opacity: 0.8;
}
.tab-label {
  line-height: 1.2;
}
.red-dot {
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background-color: var(--color-danger);
  margin-left: 6px;
  flex-shrink: 0;
}
.sidebar-tab.active .red-dot {
  box-shadow: 0 0 0 1.5px rgba(255, 255, 255, 0.6);
}

/* Main Content */
.settings-main {
  flex: 1;
  display: flex;
  flex-direction: column;
  background: var(--color-bg);
  min-width: 0;
}
.main-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 16px 24px;
  border-bottom: 1px solid var(--color-border);
  background: rgba(var(--color-bg-rgb), 0.8);
  backdrop-filter: blur(10px);
  z-index: 10;
}
.header-left {
  display: flex;
  align-items: center;
  gap: 12px;
}
.main-title {
  font-size: 16px;
  font-weight: 600;
  margin: 0;
}
.header-actions {
  display: flex;
  align-items: center;
  gap: 8px;
}

.settings-body {
  flex: 1;
  overflow-y: auto;
  padding: 24px;
}
.settings-tab-panel {
  width: 100%;
}

/* Footer & Buttons */
.settings-footer {
  display: flex;
  align-items: center;
  justify-content: flex-end;
  gap: 12px;
  padding: 16px 24px;
  border-top: 1px solid var(--color-border);
  background: var(--color-bg);
  flex-shrink: 0;
}
.save-hint {
  font-size: 12px;
  color: var(--color-success);
  margin-right: 8px;
}
.btn-secondary, .btn-primary {
  padding: 6px 16px;
  font-size: 13px;
  font-weight: 500;
  border-radius: var(--radius-md);
  cursor: pointer;
  transition: all var(--transition-fast);
}
.btn-secondary {
  border: 1px solid var(--color-border);
  color: var(--color-text);
  background: transparent;
}
.btn-secondary:hover {
  background: var(--color-bg-hover);
}
.btn-primary {
  background: var(--color-primary);
  color: white;
  border: none;
}
.btn-primary:hover {
  opacity: 0.9;
}
.btn-primary:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

/* Scrollbar tweaks */
.settings-body::-webkit-scrollbar {
  width: 8px;
}
.settings-body::-webkit-scrollbar-track {
  background: transparent;
}
.settings-body::-webkit-scrollbar-thumb {
  background-color: var(--color-border);
  border-radius: var(--radius-sm);
}

/* Transitions */
.fade-enter-active, .fade-leave-active {
  transition: opacity 0.15s;
}
.fade-enter-from, .fade-leave-to {
  opacity: 0;
}
</style>

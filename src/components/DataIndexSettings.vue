<script setup lang="ts">
import { computed, ref } from "vue";
import IndexSettings from "./IndexSettings.vue";
import ArchiveRetentionSettings from "./ArchiveRetentionSettings.vue";
import ArchivedSessionsSettings from "./ArchivedSessionsSettings.vue";
import CacheMaintenanceSettings from "./CacheMaintenanceSettings.vue";
import BlockedFoldersSettings from "./BlockedFoldersSettings.vue";
import ToggleSwitch from "./ToggleSwitch.vue";
import { useSessions } from "../composables/useSessions";
import type { SessionIdentity } from "../types/session";
import type { CliId } from "../types/cli";
import { t } from "../i18n";

const props = defineProps<{ initialCliId?: CliId }>();

const emit = defineEmits<{
  openSession: [identity: SessionIdentity];
  openSearch: [];
  closeSettings: [];
}>();

// 页内唯一 CLI 上下文：IndexSettings 负责兜底解析与持久化，
// 归档列表跟随同一状态
const activeCliId = ref<CliId | undefined>(props.initialCliId);
const {
  showArchivedSessions,
  showSnapshotSessions,
  setShowArchivedSessions,
  setShowSnapshotSessions,
} = useSessions();
const showArchived = computed({
  get: () => showArchivedSessions.value,
  set: setShowArchivedSessions,
});
const showSnapshots = computed({
  get: () => showSnapshotSessions.value,
  set: setShowSnapshotSessions,
});
</script>

<template>
  <div class="data-index-settings-container">
    <div class="settings-section">
      <h3 class="section-title">{{ t("settings.dataIndex.globalIndex") }}</h3>
      <div class="settings-card">
        <IndexSettings
          :initial-cli-id="initialCliId"
          mode="global"
          @open-search="emit('openSearch')"
          @close-settings="emit('closeSettings')"
        />
      </div>
    </div>

    <div class="settings-section">
      <h3 class="section-title">{{ t("settings.dataIndex.cliIndex") }}</h3>
      <div class="settings-card">
        <IndexSettings
          v-model:cli-id="activeCliId"
          :initial-cli-id="initialCliId"
          mode="cli"
          @open-search="emit('openSearch')"
          @close-settings="emit('closeSettings')"
        />
      </div>
    </div>

    <div class="settings-section" data-visibility-settings>
      <h3 class="section-title">{{ t("settings.dataIndex.sessionListVisibility") }}</h3>
      <div class="settings-card">
        <div class="visibility-row">
          <div class="visibility-copy">
            <span class="visibility-title">{{ t("settings.dataIndex.showArchivedSessions") }}</span>
            <span class="visibility-description">{{ t("settings.dataIndex.showArchivedSessionsDesc") }}</span>
          </div>
          <ToggleSwitch v-model="showArchived" :aria-label="t('settings.dataIndex.showArchivedSessions')" />
        </div>
        <div class="card-divider" />
        <div class="visibility-row">
          <div class="visibility-copy">
            <span class="visibility-title">{{ t("settings.dataIndex.showSnapshotSessions") }}</span>
            <span class="visibility-description">{{ t("settings.dataIndex.showSnapshotSessionsDesc") }}</span>
          </div>
          <ToggleSwitch v-model="showSnapshots" :aria-label="t('settings.dataIndex.showSnapshotSessions')" />
        </div>
      </div>
    </div>

    <div class="settings-section">
      <h3 class="section-title">{{ t("settings.dataIndex.archive") }}</h3>
      <div class="settings-card">
        <ArchiveRetentionSettings />
        <div class="card-divider" />
        <ArchivedSessionsSettings
          :cli-id="activeCliId"
          @open-session="emit('openSession', $event)"
        />
      </div>
    </div>

    <div class="settings-section">
      <h3 class="section-title">{{ t("settings.dataIndex.cache") }}</h3>
      <div class="settings-card">
        <CacheMaintenanceSettings />
      </div>
    </div>

    <div class="settings-section">
      <h3 class="section-title">{{ t("settings.dataIndex.blockedFolders") }}</h3>
      <div class="settings-card">
        <BlockedFoldersSettings />
      </div>
    </div>
  </div>
</template>

<style scoped>
.data-index-settings-container {
  display: flex;
  flex-direction: column;
}
.settings-section {
  display: flex;
  flex-direction: column;
}
.settings-section:not(:first-child) {
  margin-top: var(--space-6);
}
.section-title {
  margin: 0 0 var(--space-2) var(--space-1);
  font-size: var(--text-xs);
  font-weight: 600;
  color: var(--color-text-muted);
  text-transform: uppercase;
  letter-spacing: 0.5px;
}
.settings-card {
  background: var(--color-bg);
  border: 1px solid var(--color-border);
  border-radius: var(--radius-lg);
  box-shadow: var(--shadow-sm);
  display: flex;
  flex-direction: column;
  overflow: hidden;
}
.card-divider {
  border-top: 1px solid var(--color-border);
}
.visibility-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-4);
  padding: var(--space-4);
}
.visibility-copy {
  display: flex;
  flex-direction: column;
  gap: var(--space-1);
  min-width: 0;
}
.visibility-title {
  font-size: var(--text-sm);
  font-weight: 500;
  color: var(--color-text);
}
.visibility-description {
  font-size: var(--text-xs);
  color: var(--color-text-muted);
}
</style>

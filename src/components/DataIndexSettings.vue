<script setup lang="ts">
import { ref } from "vue";
import IndexSettings from "./IndexSettings.vue";
import ArchiveRetentionSettings from "./ArchiveRetentionSettings.vue";
import ArchivedSessionsSettings from "./ArchivedSessionsSettings.vue";
import CacheMaintenanceSettings from "./CacheMaintenanceSettings.vue";
import BlockedFoldersSettings from "./BlockedFoldersSettings.vue";
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
</style>

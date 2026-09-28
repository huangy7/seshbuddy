<script setup lang="ts">

import AppearanceSettings from "./AppearanceSettings.vue";
import TerminalSettings from "./TerminalSettings.vue";
import IntegrationSettings from "./IntegrationSettings.vue";
import type { CliId } from "../types/cli";
import { t } from "../i18n";

defineProps<{ initialCliId?: CliId }>();

const emit = defineEmits<{
  registerMenu: [];
  unregisterMenu: [];
  close: [];
}>();
</script>

<template>
  <div class="general-settings-container">
    <div class="settings-section">
      <h3 class="section-title">{{ t("settings.general.interface") }}</h3>
      <div class="settings-card">
        <AppearanceSettings />
      </div>
    </div>

    <div class="settings-section">
      <h3 class="section-title">{{ t("settings.general.terminal") }}</h3>
      <div class="settings-card">
        <TerminalSettings />
      </div>
    </div>

    <div class="settings-section">
      <h3 class="section-title">{{ t("settings.general.permissions") }}</h3>
      <div class="settings-card">
        <IntegrationSettings
          :initial-cli-id="initialCliId"
          @register-menu="emit('registerMenu')"
          @unregister-menu="emit('unregisterMenu')"
          @close="emit('close')"
        />
      </div>
    </div>
  </div>
</template>

<style scoped>
.general-settings-container {
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
</style>

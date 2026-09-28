<script setup lang="ts">
import { computed, ref, onMounted, watch } from "vue";
import { invokeApp } from "../utils/invokeApp";
import ToggleSwitch from "./ToggleSwitch.vue";
import ElegantSelect, { type SelectOption } from "./common/ElegantSelect.vue";
import { useSessions } from "../composables/useSessions";
import { resolveFeatureCliId } from "../composables/cliFilter";
import { t } from "../i18n";
import { cliPermissionHint, cliPermissionLabel, isCliId, type CliId } from "../types/cli";

const props = defineProps<{ initialCliId?: CliId }>();

const { cliOptions, skipPermissions, setSkipPermissions, registerContextMenu, unregisterContextMenu } = useSessions();
const FEATURE_CLI_STORAGE_KEY = "seshbuddy-feature-cli-integration";
const persistedCliId = localStorage.getItem(FEATURE_CLI_STORAGE_KEY);
// 权限开关语义按 CLI 不同（Claude/Codex/Gemini 有权限旁路参数），无权限语义的 CLI 不在此列出
const featureCliOptions = computed(() =>
  cliOptions.value.filter((cli) => cli.hasSessions && cliPermissionLabel(cli)),
);
const cliSelectOptions = computed<SelectOption[]>(() =>
  featureCliOptions.value.map((cli) => ({
    value: cli.id,
    label: cli.name,
    cliId: cli.id,
  })),
);
const featureCliId = ref<CliId | undefined>(resolveFeatureCliId(
  featureCliOptions.value,
  props.initialCliId,
  persistedCliId && isCliId(persistedCliId) ? persistedCliId : undefined,
));
const featureCli = computed(() =>
  featureCliOptions.value.find((cli) => cli.id === featureCliId.value) ?? null,
);

const contextMenuRegistered = ref(false);
const contextMenuToggling = ref(false);

async function checkContextMenu() {
  if (!featureCli.value?.supportsContextMenu) {
    contextMenuRegistered.value = false;
    return;
  }
  try {
    contextMenuRegistered.value = await invokeApp<boolean>("is_context_menu_registered");
  } catch (e) {
    console.error("Failed to check context menu status", e);
  }
}

onMounted(() => {
  checkContextMenu();
});

watch(featureCliOptions, (candidates) => {
  if (featureCliId.value && candidates.some((cli) => cli.id === featureCliId.value)) return;
  featureCliId.value = resolveFeatureCliId(candidates, props.initialCliId, featureCliId.value);
});

watch(featureCliId, (cliId) => {
  if (cliId) localStorage.setItem(FEATURE_CLI_STORAGE_KEY, cliId);
  void checkContextMenu();
});

watch(
  () => props.initialCliId,
  (entryCliId) => {
    featureCliId.value = resolveFeatureCliId(
      featureCliOptions.value,
      entryCliId,
      featureCliId.value,
    );
  },
);

async function onContextMenuChange(checked: boolean) {
  if (contextMenuToggling.value || !featureCliId.value) return;

  // Optimistic UI update
  contextMenuRegistered.value = checked;
  contextMenuToggling.value = true;

  try {
    if (checked) {
      await registerContextMenu(featureCliId.value);
    } else {
      await unregisterContextMenu();
    }
  } finally {
    await checkContextMenu();
    contextMenuToggling.value = false;
  }
}

function onSkipPermissionsChange(checked: boolean) {
  setSkipPermissions(checked);
}
</script>

<template>
  <div class="settings-group">
    <div class="settings-row">
      <div class="row-content">
        <div class="row-title">{{ t("settings.integration.cliTitle") }}</div>
      </div>
      <div class="row-action">
        <div class="cli-select-wrap">
          <ElegantSelect
            v-model="featureCliId"
            :options="cliSelectOptions"
            :aria-label="t('settings.integration.cliTitle')"
            data-testid="integration-cli-select"
          />
        </div>
      </div>
    </div>

    <template v-if="featureCli">
      <div class="settings-row">
        <div class="row-content">
          <div class="row-title">{{ cliPermissionLabel(featureCli) }}</div>
          <p class="row-hint">{{ cliPermissionHint(featureCli) }}</p>
        </div>
        <div class="row-action">
          <ToggleSwitch
            :model-value="skipPermissions"
            :aria-label="cliPermissionLabel(featureCli)"
            @update:model-value="onSkipPermissionsChange"
          />
        </div>
      </div>

      <div v-if="featureCli.supportsContextMenu" class="settings-row">
        <div class="row-content">
          <div class="row-title">{{ t("settings.integration.contextMenuTitle") }}</div>
          <p class="row-hint">{{ t("settings.integration.contextMenuHint", { name: featureCli.name }) }}</p>
        </div>
        <div class="row-action">
          <ToggleSwitch
            :model-value="contextMenuRegistered"
            :disabled="contextMenuToggling"
            :aria-label="t('settings.integration.contextMenuTitle')"
            @update:model-value="onContextMenuChange"
          />
        </div>
      </div>
      <div v-else class="settings-row">
        <div class="row-content">
          <div class="row-title">{{ t("settings.integration.contextMenuTitle") }}</div>
          <p class="row-hint">{{ t("settings.integration.contextMenuUnsupported") }}</p>
        </div>
      </div>
    </template>
  </div>
</template>

<style scoped>
.settings-group {
  display: flex;
  flex-direction: column;
}
.settings-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: var(--space-4) var(--space-5);
  gap: var(--space-4);
}
.settings-row + .settings-row {
  border-top: 1px solid var(--color-border);
}
.row-content {
  display: flex;
  flex-direction: column;
  gap: 4px;
  flex: 1;
  min-width: 0;
}
.row-title {
  font-size: var(--text-sm);
  font-weight: 500;
  color: var(--color-text);
}
.row-hint {
  font-size: var(--text-xs);
  color: var(--color-text-muted);
  margin: 0;
  line-height: 1.5;
}
.row-action {
  flex-shrink: 0;
}
.cli-select-wrap {
  min-width: 120px;
}
</style>

<script setup lang="ts">
import { ref, computed, watch, onMounted } from "vue";
import { invokeApp } from "../utils/invokeApp";
import SvgIcon from "./icons/SvgIcon.vue";
import ElegantSelect from "./common/ElegantSelect.vue";
import { useSessions } from "../composables/useSessions";
import { cliPermissionLabel, SUPPORTED_CLIS } from "../types/cli";
import { t } from "../i18n";

type LaunchMode = "agent" | "terminal";

const props = defineProps<{
  profiles?: string[];
  cliId: string;
  isArchived?: boolean;
  /** 对话框标题与主按钮文案，默认“恢复会话” */
  title?: string;
}>();

const emit = defineEmits<{
  close: [];
  resume: [launchMode: LaunchMode, profileName: string | null, skipPermissions: boolean];
  copyCommand: [profileName: string | null, skipPermissions: boolean, cb?: (ok: boolean) => void];
}>();

const { skipPermissions } = useSessions();

const launchMode = ref<LaunchMode>("agent");
const selectedProfile = ref<string | null>(null);
const profileNames = ref<string[]>(props.profiles && props.profiles.length > 0 ? [...props.profiles] : []);
const copyFeedback = ref(false);
const copyFailed = ref(false);
// 本次启动的权限开关：默认跟随全局设置（设置 → 通用），仅作用于本次，不写回
const dialogSkipPermissions = ref(skipPermissions.value);

const permissionLabel = computed(() => {
  const selected = SUPPORTED_CLIS.find((cli) => cli.id === props.cliId);
  return selected ? cliPermissionLabel(selected) : "";
});

async function loadProfiles() {
  if (props.cliId !== "claude") return;
  try {
    const names = await invokeApp<string[]>("list_profiles", { cliId: "claude" });
    if (Array.isArray(names) && names.length > 0) {
      profileNames.value = names;
    }
    const active = await invokeApp<string>("get_active_profile", { cliId: "claude" });
    if (active && profileNames.value.includes(active)) {
      selectedProfile.value = active;
    } else if (profileNames.value.length > 0 && !selectedProfile.value) {
      selectedProfile.value = profileNames.value[0];
    }
  } catch {
    if (profileNames.value.length > 0 && !selectedProfile.value) {
      selectedProfile.value = profileNames.value[0];
    }
  }
}

// API 配置选择仅对 Claude 开放（与新建会话弹窗一致），其他 CLI 不显示也不应用
const showProfileSelector = computed(
  () => props.cliId === "claude" && profileNames.value.length >= 2
);

const profileOptions = computed(() =>
  profileNames.value.map((name) => ({
    value: name,
    label: name,
    icon: "settings",
  }))
);

watch(
  () => props.profiles,
  (newProfiles) => {
    if (newProfiles && newProfiles.length > 0) {
      profileNames.value = [...newProfiles];
      if (!selectedProfile.value || !newProfiles.includes(selectedProfile.value)) {
        selectedProfile.value = newProfiles[0];
      }
    }
  }
);

onMounted(() => {
  if (props.profiles && props.profiles.length > 0) {
    selectedProfile.value = props.profiles[0];
  }
  void loadProfiles();
});

function onResume() {
  const profile = showProfileSelector.value ? selectedProfile.value : null;
  emit("resume", launchMode.value, profile, dialogSkipPermissions.value);
}

function onCopyCommand() {
  if (copyFeedback.value) return;
  const profile = showProfileSelector.value ? selectedProfile.value : null;
  copyFeedback.value = true;
  copyFailed.value = false;
  emit("copyCommand", profile, dialogSkipPermissions.value, (ok) => {
    copyFailed.value = !ok;
    setTimeout(() => {
      copyFeedback.value = false;
    }, 1500);
  });
}
</script>

<template>
  <div class="modal-backdrop" @click.self="emit('close')">
    <div class="dialog">
      <div class="dialog-header">
        <h3>{{ props.title ?? t("dialogs.resumeSession.title") }}</h3>
        <button class="dialog-close icon-btn" :title="t('dialogs.common.close')" :aria-label="t('dialogs.common.close')" @click="emit('close')">
          <SvgIcon name="x" :size="16" />
        </button>
      </div>

      <div class="dialog-body">
        <div class="form-group">
          <label class="field-label">{{ t("dialogs.common.launchMode") }}</label>
          <div class="launch-options">
            <label class="launch-option" :class="{ selected: launchMode === 'agent' }">
              <input type="radio" v-model="launchMode" value="agent" />
              <SvgIcon name="terminal" :size="13" />
              {{ t("dialogs.common.agent") }}
            </label>
            <label class="launch-option" :class="{ selected: launchMode === 'terminal' }">
              <input type="radio" v-model="launchMode" value="terminal" />
              <SvgIcon name="external-link" :size="13" />
              {{ t("dialogs.common.terminal") }}
            </label>
          </div>
        </div>

        <div class="form-group" v-if="showProfileSelector">
          <label class="field-label">{{ t("dialogs.common.apiProfile") }}</label>
          <ElegantSelect
            v-model="selectedProfile"
            :options="profileOptions"
            :placeholder="t('dialogs.common.apiProfilePlaceholder')"
            :emptyHint="t('dialogs.common.apiProfileEmpty')"
          />
        </div>

        <label class="permission-toggle" v-if="permissionLabel">
          <input type="checkbox" v-model="dialogSkipPermissions" />
          <span>{{ permissionLabel }}</span>
        </label>

        <div v-if="isArchived" class="archive-notice">
          <SvgIcon name="archive" :size="14" class="archive-notice-icon" />
          <span class="archive-notice-text">{{ t("dialogs.resumeSession.archiveNotice") }}</span>
        </div>
      </div>

      <div class="dialog-footer">
        <button
          class="btn-copy"
          :class="{ copied: copyFeedback && !copyFailed, failed: copyFeedback && copyFailed }"
          type="button"
          @click="onCopyCommand"
        >
          <SvgIcon :name="copyFeedback ? (copyFailed ? 'alert-circle' : 'check') : 'copy'" :size="14" />
          <span>{{ copyFeedback ? (copyFailed ? t("dialogs.common.copyFailed") : t("dialogs.common.copied")) : t("dialogs.common.copyCommand") }}</span>
        </button>
        <div class="footer-spacer" />
        <button class="btn-cancel" type="button" @click="emit('close')">{{ t("dialogs.common.cancel") }}</button>
        <button class="btn-primary" type="button" @click="onResume">
          <SvgIcon :name="launchMode === 'agent' ? 'terminal' : 'external-link'" :size="14" />
          {{ props.title ?? t("dialogs.resumeSession.title") }}
        </button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.dialog {
  width: 440px;
  max-width: calc(100vw - var(--space-8));
  background: var(--color-bg);
  border: 1px solid var(--color-border);
  border-radius: var(--radius-xl);
  box-shadow: var(--shadow-lg);
}
.dialog-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: var(--space-4);
  border-bottom: 1px solid var(--color-border);
}
.dialog-header h3 {
  font-size: var(--text-base);
  font-weight: 600;
  color: var(--color-text);
}
.dialog-close {
  border-radius: var(--radius-sm);
}
.dialog-body {
  padding: var(--space-4);
  display: flex;
  flex-direction: column;
  gap: var(--space-4);
}
.form-group {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
}
.field-label {
  display: block;
  font-size: var(--text-xs);
  font-weight: 600;
  color: var(--color-text-secondary);
}
.launch-options {
  display: flex;
  flex-wrap: wrap;
  gap: var(--space-2);
  margin-top: var(--space-1);
}
.launch-option {
  display: flex;
  align-items: center;
  white-space: nowrap;
  gap: var(--space-1);
  padding: var(--space-1) var(--space-3);
  font-size: var(--text-sm);
  border: 1px solid var(--color-border);
  border-radius: var(--radius-md);
  cursor: pointer;
  transition: all var(--transition-fast);
}
.launch-option:hover {
  border-color: var(--color-primary);
}
.launch-option.selected {
  border-color: var(--color-primary);
  background: var(--color-primary-light);
  color: var(--color-primary);
}
.launch-option input {
  display: none;
}
.permission-toggle {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  font-size: var(--text-sm);
  color: var(--color-text);
  cursor: pointer;
  user-select: none;
}
.permission-toggle input {
  accent-color: var(--color-primary);
  cursor: pointer;
}
.archive-notice {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  padding: var(--space-2) var(--space-3);
  background: var(--color-bg-subtle, rgba(0, 0, 0, 0.03));
  border: 1px solid var(--color-border);
  border-radius: var(--radius-md);
  font-size: var(--text-xs);
  line-height: 1.5;
  color: var(--color-text-secondary);
}
.archive-notice-icon {
  flex-shrink: 0;
  color: var(--color-primary);
}
.archive-notice-text {
  flex: 1;
}
.dialog-footer {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  padding: var(--space-3) var(--space-4);
  border-top: 1px solid var(--color-border);
}
.footer-spacer {
  flex: 1;
}
.btn-copy {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: var(--space-1);
  min-width: 96px;
  height: 32px;
  padding: 0 var(--space-3);
  font-size: var(--text-sm);
  color: var(--color-text-secondary);
  border: 1px solid var(--color-border);
  border-radius: var(--radius-md);
  background: var(--color-bg);
  cursor: pointer;
  box-sizing: border-box;
  transition: all var(--transition-fast);
}
.btn-copy:hover:not(.copied) {
  background: var(--color-bg-hover);
  color: var(--color-text);
  border-color: var(--color-border-hover, var(--color-border));
}
.btn-copy.copied {
  color: var(--color-success, #22c55e);
  border-color: var(--color-success, #22c55e);
  background: var(--color-success-light, rgba(34, 197, 94, 0.08));
  cursor: default;
}
.btn-copy.failed {
  color: var(--color-error, #ef4444);
  border-color: var(--color-error, #ef4444);
  background: var(--color-error-light, rgba(239, 68, 68, 0.08));
  cursor: default;
}
.btn-cancel {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  height: 32px;
  padding: 0 var(--space-3);
  font-size: var(--text-sm);
  color: var(--color-text-secondary);
  border: 1px solid var(--color-border);
  border-radius: var(--radius-md);
  background: transparent;
  cursor: pointer;
  box-sizing: border-box;
  transition: all var(--transition-fast);
}
.btn-cancel:hover {
  background: var(--color-bg-hover);
}
.btn-primary {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: var(--space-1);
  height: 32px;
  padding: 0 var(--space-3);
  font-size: var(--text-sm);
  color: white;
  background: var(--color-primary);
  border: none;
  border-radius: var(--radius-md);
  cursor: pointer;
  box-sizing: border-box;
  transition: all var(--transition-fast);
}
.btn-primary:hover {
  background: var(--color-primary-hover);
}
</style>

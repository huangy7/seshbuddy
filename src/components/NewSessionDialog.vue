<script setup lang="ts">
import { ref, computed, watch, onMounted } from "vue";
import { open } from "@tauri-apps/plugin-dialog";
import { invokeApp } from "../utils/invokeApp";
import SvgIcon from "./icons/SvgIcon.vue";
import ElegantSelect from "./common/ElegantSelect.vue";
import { useSessions } from "../composables/useSessions";
import { useTerminalApp } from "../composables/useTerminalApp";
import { copyPromiseToClipboard } from "../utils/clipboard";
import type { CliId } from "../types/cli";
import { cliPermissionLabel, SUPPORTED_CLIS } from "../types/cli";
import { t } from "../i18n";

type LaunchMode = "agent" | "terminal";

const emit = defineEmits<{
  close: [];
  create: [projectPath: string, cliKind: string, launchMode: LaunchMode, profileName: string | null, skipPermissions: boolean];
}>();

const props = defineProps<{
  initialProjectPath?: string | null;
  initialCliId?: CliId;
  cliBinaryStatuses?: Record<string, boolean>;
}>();

const { projects, launchCliId, skipPermissions } = useSessions();
const { terminalAppForLaunch } = useTerminalApp();

const selectedProject = ref<string | null>(props.initialProjectPath ?? null);
const selectedCli = ref<CliId>(props.initialCliId ?? launchCliId.value);
const launchMode = ref<LaunchMode>("agent");
const copyFeedback = ref(false);
// 本次启动的权限开关：默认跟随全局设置（设置 → 通用），仅作用于本次，不写回
const dialogSkipPermissions = ref(skipPermissions.value);

const permissionLabel = computed(() => {
  const selected = SUPPORTED_CLIS.find((cli) => cli.id === selectedCli.value);
  return selected ? cliPermissionLabel(selected) : "";
});

const selectedProfile = ref<string | null>(null);
const profileNames = ref<string[]>([]);
const activeProfile = ref<string>("");

async function loadProfiles() {
  try {
    const names = await invokeApp<string[]>("list_profiles", { cliId: "claude" });
    profileNames.value = names;
    const active = await invokeApp<string>("get_active_profile", { cliId: "claude" });
    activeProfile.value = active;
    selectedProfile.value = active || (names.length > 0 ? names[0] : null);
  } catch {
    profileNames.value = [];
  }
}

const showProfileSelector = computed(() =>
  selectedCli.value === "claude" && profileNames.value.length >= 2
);

watch(selectedCli, () => {
  selectedProfile.value = activeProfile.value || null;
});

onMounted(() => {
  loadProfiles();
});

const availableClis = computed(() =>
  SUPPORTED_CLIS.filter(
    (cli) => cli.supportsNewSession && (!props.cliBinaryStatuses || props.cliBinaryStatuses[cli.id])
  )
);

// Deduplicated project paths from existing session history + initial path
const projectPaths = computed(() => {
  const paths = new Set<string>();
  if (props.initialProjectPath) paths.add(props.initialProjectPath);
  for (const p of projects.value) {
    // 没有项目路径的会话不产生候选目录（空串会被渲染成一个可选的空选项）
    if (p.original_path) paths.add(p.original_path);
  }
  return Array.from(paths).sort();
});

const projectOptions = computed(() => {
  return projectPaths.value.map(p => ({
    value: p,
    label: projectName(p),
    description: p,
    icon: 'folder'
  }));
});

const profileOptions = computed(() => {
  return profileNames.value.map(name => ({
    value: name,
    label: name,
    icon: 'settings'
  }));
});

async function browseDirectory() {
  const selected = await open({
    directory: true,
    multiple: false,
    title: t("dialogs.newSession.browseTitle"),
  });
  if (selected && typeof selected === "string") {
    selectedProject.value = selected;
  }
}

function projectName(path: string): string {
  return path.split("/").pop() || path;
}

function onCreate() {
  if (!selectedProject.value) return;
  emit("create", selectedProject.value, selectedCli.value, launchMode.value,
    showProfileSelector.value ? selectedProfile.value : null, dialogSkipPermissions.value);
}

async function onCopyCommand() {
  if (!selectedProject.value || copyFeedback.value) return;
  // 不要先 await invoke 再复制：旧版 WKWebView 中 await 会消耗用户手势激活，
  // 导致剪贴板写入被拒。把 Promise 交给 copyPromiseToClipboard 同步发起写入。
  const cmdPromise = invokeApp<string>("get_launch_command", {
    cliId: selectedCli.value,
    projectPath: selectedProject.value,
    skipPermissions: dialogSkipPermissions.value,
    profileName: showProfileSelector.value ? selectedProfile.value : undefined,
    terminalApp: terminalAppForLaunch.value ?? undefined,
  });
  const ok = await copyPromiseToClipboard(cmdPromise);
  if (ok) {
    copyFeedback.value = true;
    setTimeout(() => {
      copyFeedback.value = false;
    }, 1500);
  } else {
    cmdPromise.catch((e) => console.error("get_launch_command failed:", e));
  }
}
</script>

<template>
  <div class="modal-backdrop" @click.self="emit('close')">
    <div class="dialog">
      <div class="dialog-header">
        <h3>{{ t("dialogs.newSession.title") }}</h3>
        <button class="dialog-close icon-btn" :title="t('dialogs.common.close')" :aria-label="t('dialogs.common.close')" @click="emit('close')">
          <SvgIcon name="x" :size="16" />
        </button>
      </div>

      <div class="dialog-body">
        <div class="form-group">
          <label class="field-label">{{ t("dialogs.newSession.projectDir") }}</label>
          <ElegantSelect
            v-model="selectedProject"
            :options="projectOptions"
            :placeholder="t('dialogs.newSession.projectPlaceholder')"
            :emptyHint="t('dialogs.newSession.projectEmpty')"
          >
            <template #footer>
              <div class="menu-footer" @click="browseDirectory">
                <SvgIcon name="folder-open" :size="14" />
                <span>{{ t("dialogs.newSession.browse") }}</span>
              </div>
            </template>
          </ElegantSelect>
        </div>

        <div class="form-group" v-if="availableClis.length > 1">
          <label class="field-label">{{ t("dialogs.newSession.cliType") }}</label>
          <div class="cli-options">
            <label
              v-for="cli in availableClis"
              :key="cli.id"
              class="cli-option"
              :class="{ selected: selectedCli === cli.id }"
            >
              <input type="radio" v-model="selectedCli" :value="cli.id" />
              {{ cli.name }}
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

        <div class="form-group">
          <label class="field-label">{{ t("dialogs.common.launchMode") }}</label>
          <div class="cli-options">
            <label class="cli-option" :class="{ selected: launchMode === 'agent' }">
              <input type="radio" v-model="launchMode" value="agent" />
              <SvgIcon name="terminal" :size="13" />
              {{ t("dialogs.common.agent") }}
            </label>
            <label class="cli-option" :class="{ selected: launchMode === 'terminal' }">
              <input type="radio" v-model="launchMode" value="terminal" />
              <SvgIcon name="external-link" :size="13" />
              {{ t("dialogs.common.terminal") }}
            </label>
          </div>
        </div>

        <label class="permission-toggle" v-if="permissionLabel">
          <input type="checkbox" v-model="dialogSkipPermissions" />
          <span>{{ permissionLabel }}</span>
        </label>
      </div>

      <div class="dialog-footer">
        <button
          class="btn-copy"
          :class="{ copied: copyFeedback }"
          type="button"
          :disabled="!selectedProject"
          @click="onCopyCommand"
        >
          <SvgIcon :name="copyFeedback ? 'check' : 'copy'" :size="14" />
          <span>{{ copyFeedback ? t("dialogs.common.copied") : t("dialogs.common.copyCommand") }}</span>
        </button>
        <div class="footer-spacer" />
        <button class="btn-cancel" type="button" @click="emit('close')">{{ t("dialogs.common.cancel") }}</button>
        <button class="btn-create" type="button" :disabled="!selectedProject" @click="onCreate">
          <SvgIcon :name="launchMode === 'agent' ? 'terminal' : 'external-link'" :size="14" />
          {{ launchMode === 'agent' ? t("dialogs.newSession.createInAgent") : t("dialogs.newSession.createInTerminal") }}
        </button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.dialog {
  width: 420px;
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
.menu-footer {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: var(--space-2);
  padding: var(--space-3);
  border-top: 1px solid var(--color-border);
  background: var(--color-bg-hover);
  font-size: var(--text-sm);
  font-weight: 500;
  color: var(--color-text);
  cursor: pointer;
  transition: all 0.15s ease;
}
.menu-footer:hover {
  color: var(--color-primary);
  background: var(--color-primary-light);
}
.cli-options {
  display: flex;
  gap: var(--space-2);
  margin-top: var(--space-1);
}
.cli-option {
  display: flex;
  align-items: center;
  gap: var(--space-1);
  padding: var(--space-1) var(--space-3);
  font-size: var(--text-sm);
  border: 1px solid var(--color-border);
  border-radius: var(--radius-md);
  cursor: pointer;
  transition: all var(--transition-fast);
}
.cli-option:hover {
  border-color: var(--color-primary);
}
.cli-option.selected {
  border-color: var(--color-primary);
  background: var(--color-primary-light);
  color: var(--color-primary);
}
.cli-option input {
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
.btn-copy:hover:not(:disabled):not(.copied) {
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
.btn-copy:disabled:not(.copied) {
  opacity: 0.5;
  cursor: not-allowed;
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
.btn-create {
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
.btn-create:hover:not(:disabled) {
  background: var(--color-primary-hover);
}
.btn-create:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}
</style>

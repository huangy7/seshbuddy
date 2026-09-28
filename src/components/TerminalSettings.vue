<script setup lang="ts">
import { computed, onMounted, ref, watch } from "vue";
import { invokeApp } from "../utils/invokeApp";
import OptionCard from "./OptionCard.vue";
import ElegantSelect from "./common/ElegantSelect.vue";
import { useAgentStatusMode } from "../composables/useAgentStatusMode";
import { useTerminalApp } from "../composables/useTerminalApp";
import { useSessions } from "../composables/useSessions";
import { t } from "../i18n";
import type { AgentStatusMode } from "../types/pty";

const XTERM_RENDERER_KEY = "seshbuddy-xterm-renderer";
type XtermRenderer = "canvas" | "webgl" | "dom";

const isWindows = navigator.userAgent.toLowerCase().includes("windows");

// 选项文案在 computed 内求值：常量数组只在 setup 时求值一次，
// 切换语言后标签不会跟着变。
const rendererOptions = computed<Array<{ value: XtermRenderer; label: string; hint: string }>>(() => [
  { value: "canvas", label: "Canvas 2D", hint: t("settings.terminal.rendererCanvasHint") },
  { value: "webgl", label: "WebGL", hint: t("settings.terminal.rendererWebglHint") },
  { value: "dom", label: "DOM", hint: t("settings.terminal.rendererDomHint") },
]);

function isXtermRenderer(value: string): value is XtermRenderer {
  return rendererOptions.value.some((option) => option.value === value);
}

function loadXtermRenderer(): XtermRenderer {
  try {
    const raw = localStorage.getItem(XTERM_RENDERER_KEY);
    if (raw && isXtermRenderer(raw)) {
      return raw;
    }
  } catch {
    // ignore
  }
  return isWindows ? "canvas" : "webgl";
}

const xtermRenderer = ref<XtermRenderer>(loadXtermRenderer());

watch(xtermRenderer, (value) => {
  if (!isXtermRenderer(value)) {
    return;
  }
  try {
    localStorage.setItem(XTERM_RENDERER_KEY, value);
  } catch {
    // ignore
  }
});

const activeRendererHint = computed(() => {
  return rendererOptions.value.find((opt) => opt.value === xtermRenderer.value)?.hint
    ?? t("settings.terminal.rendererFallbackHint");
});

const { agentStatusMode } = useAgentStatusMode();

const {
  terminalApp,
  terminalAppOptions,
  detectTerminalApps,
  setTerminalApp,
} = useTerminalApp();
const { skipPermissions } = useSessions();

onMounted(() => {
  void detectTerminalApps();
});

async function onSelectTerminalApp(value: unknown) {
  if (typeof value !== "string" || !value) return;
  setTerminalApp(value);
  // 右键菜单的终端应用在注册时固化进工作流文件，设置变更后静默重注册使其生效
  try {
    if (await invokeApp<boolean>("is_context_menu_registered")) {
      await invokeApp("register_context_menu", {
        skipPermissions: skipPermissions.value,
        terminalApp: value,
      });
    }
  } catch {
    // 静默失败：设置本身已保存，用户下次手动注册时生效
  }
}

const agentStatusModeOptions = computed<Array<{ value: AgentStatusMode; label: string; hint: string }>>(() => [
  { value: "osc", label: t("settings.terminal.agentStatusOsc"), hint: t("settings.terminal.agentStatusOscHint") },
  { value: "hook-relay", label: t("settings.terminal.agentStatusHookRelay"), hint: t("settings.terminal.agentStatusHookRelayHint") },
]);

const activeStatusModeHint = computed(() => {
  return agentStatusModeOptions.value.find((opt) => opt.value === agentStatusMode.value)?.hint
    ?? t("settings.terminal.agentStatusFallbackHint");
});
</script>

<template>
  <div class="settings-group">
    <div class="settings-row" v-if="terminalAppOptions.length > 0">
      <div class="row-content">
        <div class="row-title">{{ t("settings.terminal.externalAppTitle") }}</div>
        <p class="row-hint">{{ t("settings.terminal.externalAppHint") }}</p>
      </div>
      <div class="row-action">
        <ElegantSelect
          :model-value="terminalApp ?? ''"
          :options="terminalAppOptions"
          size="small"
          min-menu-width="180px"
          @update:model-value="onSelectTerminalApp"
        />
      </div>
    </div>

    <div class="settings-row">
      <div class="row-content">
        <div class="row-title">{{ t("settings.terminal.rendererTitle") }}</div>
        <p class="row-hint">{{ activeRendererHint }}</p>
      </div>
      <div class="row-action">
        <OptionCard v-model="xtermRenderer" :options="rendererOptions" />
      </div>
    </div>

    <div class="settings-row">
      <div class="row-content">
        <div class="row-title">{{ t("settings.terminal.agentStatusTitle") }}</div>
        <p class="row-hint">{{ activeStatusModeHint }}</p>
      </div>
      <div class="row-action">
        <OptionCard v-model="agentStatusMode" :options="agentStatusModeOptions" />
      </div>
    </div>
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
</style>

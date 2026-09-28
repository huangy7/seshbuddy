<script setup lang="ts">
import { ref, onMounted, onBeforeUnmount } from "vue";
import { invokeApp } from "../utils/invokeApp";
import { getVersion } from "@tauri-apps/api/app";
import SvgIcon from "./icons/SvgIcon.vue";
import { openExternalUrl } from "../utils/openExternalUrl";
import { t } from "../i18n";

const emit = defineEmits<{
  close: [];
}>();

export interface CliDiagnosticInfo {
  id: string;
  name: string;
  command: string;
  installed: boolean;
  hasSessions: boolean;
}

export interface SystemDiagnostics {
  appName: string;
  appVersion: string;
  tauriVersion: string;
  os: string;
  osName: string;
  osVersion: string;
  kernelVersion: string;
  arch: string;
  cpuModel: string;
  cpuCores: number;
  memoryTotal: string;
  clis: CliDiagnosticInfo[];
}

const diag = ref<SystemDiagnostics | null>(null);
// 版本号一律从运行时取，绝不写死：它会随反馈内容进入 issue 正文，
// 写死的值会把错误的版本报给维护者，且版本升级后不会有任何编译期提示
const appVersion = ref("");

// 环境信息检测
const osPlatform = ref("");
const userAgent = typeof navigator !== "undefined" ? navigator.userAgent : "";

onMounted(async () => {
  try {
    const res = await invokeApp<SystemDiagnostics>("get_system_diagnostics");
    diag.value = res;
    appVersion.value = res.appVersion;
    osPlatform.value = `${res.osName} ${res.osVersion} (${res.arch})`.trim();
  } catch {
    // 诊断命令不可用（纯前端调试、单测）时退回 Tauri 运行时版本；仍取不到就留空
    appVersion.value = await getVersion().catch(() => "");
    // 纯前端或测试环境回退
    const isMac = /mac/i.test(userAgent);
    const isWin = /win/i.test(userAgent);
    const isLinux = /linux/i.test(userAgent);
    const isArm = /arm|aarch64/i.test(userAgent) || /mac/i.test(userAgent);
    
    let fallbackOs = "Desktop";
    if (isMac) fallbackOs = `macOS ${isArm ? "(Apple Silicon / ARM64)" : "(Intel / x64)"}`;
    else if (isWin) fallbackOs = "Windows (x64)";
    else if (isLinux) fallbackOs = "Linux (x64)";
    osPlatform.value = fallbackOs;
  }

  window.addEventListener("keydown", handleKeyDown);
});

onBeforeUnmount(() => {
  window.removeEventListener("keydown", handleKeyDown);
});

function handleKeyDown(e: KeyboardEvent) {
  if (e.key === "Escape") {
    emit("close");
  }
}

function getDiagnosticsText(): string {
  if (diag.value) {
    const cliLines = diag.value.clis
      .map(
        (c) =>
          `  - **${c.name}** (\`${c.command}\`): ${c.installed ? t("dialogs.feedback.cliInstalled") : t("dialogs.feedback.cliMissing")}${c.hasSessions ? t("dialogs.feedback.cliHasSessions") : ""}`
      )
      .join("\n");

    return [
      t("dialogs.feedback.diagTitle"),
      t("dialogs.feedback.diagApp", {
        app: diag.value.appName,
        version: diag.value.appVersion,
        tauri: diag.value.tauriVersion,
      }),
      t("dialogs.feedback.diagOs", {
        osName: diag.value.osName,
        osVersion: diag.value.osVersion,
        arch: diag.value.arch,
      }),
      t("dialogs.feedback.diagKernel", { kernel: diag.value.kernelVersion }),
      t("dialogs.feedback.diagCpu", { cpu: diag.value.cpuModel, cores: diag.value.cpuCores }),
      t("dialogs.feedback.diagMemory", { memory: diag.value.memoryTotal }),
      t("dialogs.feedback.diagClis"),
      cliLines,
      t("dialogs.feedback.diagUserAgent", { userAgent }),
      t("dialogs.feedback.diagTimestamp", { timestamp: new Date().toISOString() }),
    ].join("\n");
  }

  return [
    t("dialogs.feedback.diagTitle"),
    t("dialogs.feedback.diagAppFallback", {
      value: appVersion.value
        ? `SeshBuddy v${appVersion.value}`
        : t("dialogs.feedback.diagUnknownEnv"),
    }),
    t("dialogs.feedback.diagPlatform", { platform: osPlatform.value }),
    t("dialogs.feedback.diagUserAgent", { userAgent }),
    t("dialogs.feedback.diagTimestamp", { timestamp: new Date().toISOString() }),
  ].join("\n");
}

function openIssue(type: "bug" | "feature" | "search") {
  const base = "https://github.com/huangy7/seshbuddy/issues";
  let targetUrl = base;

  if (type === "bug") {
    const title = encodeURIComponent("[Bug]: ");
    const body = encodeURIComponent(
      t("dialogs.feedback.bugBody", { diagnostics: getDiagnosticsText() })
    );
    targetUrl = `${base}/new?title=${title}&body=${body}`;
  } else if (type === "feature") {
    const title = encodeURIComponent("[Feature]: ");
    const body = encodeURIComponent(
      t("dialogs.feedback.featureBody", {
        signature: appVersion.value ? `*SeshBuddy v${appVersion.value}*` : "*SeshBuddy*",
      })
    );
    targetUrl = `${base}/new?title=${title}&body=${body}`;
  } else if (type === "search") {
    targetUrl = base;
  }

  void openExternalUrl(targetUrl);
  emit("close");
}
</script>

<template>
  <div class="modal-backdrop" @click.self="emit('close')">
    <div class="feedback-dialog">
      <!-- 弹窗顶部 -->
      <div class="feedback-header">
        <div class="header-left">
          <div class="github-icon-badge">
            <SvgIcon name="github" :size="20" />
          </div>
          <div>
            <h3 class="feedback-title">{{ t("dialogs.feedback.title") }}</h3>
            <p class="feedback-subtitle">{{ t("dialogs.feedback.subtitle") }}</p>
          </div>
        </div>
        <button class="feedback-close-btn" :title="t('common.chatSearchBar.close')" :aria-label="t('dialogs.common.close')" @click="emit('close')">
          <SvgIcon name="x" :size="16" />
        </button>
      </div>

      <!-- 三个分类选项卡片 -->
      <div class="feedback-body">
        <div class="options-grid">
          <!-- Bug Report -->
          <div class="feedback-card" @click="openIssue('bug')">
            <div class="card-icon bug-icon">
              <SvgIcon name="bug" :size="20" />
            </div>
            <div class="card-content">
              <div class="card-title-row">
                <span class="card-title">{{ t("dialogs.feedback.bugTitle") }}</span>
                <SvgIcon name="external-link" :size="14" class="external-indicator" />
              </div>
              <p class="card-desc">{{ t("dialogs.feedback.bugDesc") }}</p>
            </div>
          </div>

          <!-- Feature Request -->
          <div class="feedback-card" @click="openIssue('feature')">
            <div class="card-icon feature-icon">
              <SvgIcon name="lightbulb" :size="20" />
            </div>
            <div class="card-content">
              <div class="card-title-row">
                <span class="card-title">{{ t("dialogs.feedback.featureTitle") }}</span>
                <SvgIcon name="external-link" :size="14" class="external-indicator" />
              </div>
              <p class="card-desc">{{ t("dialogs.feedback.featureDesc") }}</p>
            </div>
          </div>

          <!-- Search Existing -->
          <div class="feedback-card" @click="openIssue('search')">
            <div class="card-icon search-icon">
              <SvgIcon name="search" :size="20" />
            </div>
            <div class="card-content">
              <div class="card-title-row">
                <span class="card-title">{{ t("dialogs.feedback.searchTitle") }}</span>
                <SvgIcon name="external-link" :size="14" class="external-indicator" />
              </div>
              <p class="card-desc">{{ t("dialogs.feedback.searchDesc") }}</p>
            </div>
          </div>
        </div>
      </div>

      <!-- 底部栏 -->
      <div class="feedback-footer">
        <div class="footer-tip">
          <SvgIcon name="info" :size="13" />
          <span>{{ t("dialogs.feedback.footerTip") }}</span>
        </div>
        <div class="footer-actions">
          <button class="btn-cancel" type="button" @click="emit('close')">
            {{ t("dialogs.common.close") }}
          </button>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.modal-backdrop {
  animation: fadeIn 0.15s var(--ease-standard, ease);
}

@keyframes fadeIn {
  from { opacity: 0; }
  to { opacity: 1; }
}

.feedback-dialog {
  width: 500px;
  max-width: 92vw;
  background: var(--color-bg);
  border: 1px solid var(--color-border);
  border-radius: var(--radius-lg, 12px);
  box-shadow: 0 16px 36px rgba(0, 0, 0, 0.25);
  display: flex;
  flex-direction: column;
  overflow: hidden;
  animation: popIn 0.18s cubic-bezier(0.16, 1, 0.3, 1);
}

@keyframes popIn {
  from {
    opacity: 0;
    transform: scale(0.96) translateY(4px);
  }
  to {
    opacity: 1;
    transform: scale(1) translateY(0);
  }
}

.feedback-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: var(--space-4, 16px) var(--space-5, 20px);
  border-bottom: 1px solid var(--color-border);
  background: var(--color-bg-secondary, var(--color-bg));
}

.header-left {
  display: flex;
  align-items: center;
  gap: var(--space-3, 12px);
}

.github-icon-badge {
  width: 36px;
  height: 36px;
  border-radius: var(--radius-md, 8px);
  background: var(--color-bg-hover, rgba(255, 255, 255, 0.08));
  border: 1px solid var(--color-border);
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--color-text);
  flex-shrink: 0;
}

.feedback-title {
  margin: 0;
  font-size: var(--text-base, 15px);
  font-weight: 600;
  color: var(--color-text);
  line-height: 1.3;
}

.feedback-subtitle {
  margin: 2px 0 0;
  font-size: var(--text-xs, 12px);
  color: var(--color-text-muted);
}

.feedback-close-btn {
  width: 28px;
  height: 28px;
  border-radius: var(--radius-sm, 6px);
  border: none;
  background: transparent;
  color: var(--color-text-muted);
  cursor: pointer;
  display: flex;
  align-items: center;
  justify-content: center;
  transition: all var(--transition-fast, 0.15s);
}

.feedback-close-btn:hover {
  background: var(--color-bg-hover);
  color: var(--color-text);
}

.feedback-close-btn:active {
  transform: scale(0.92);
}

.feedback-body {
  padding: var(--space-4, 16px) var(--space-5, 20px);
  display: flex;
  flex-direction: column;
}

.options-grid {
  display: flex;
  flex-direction: column;
  gap: var(--space-2, 8px);
}

.feedback-card {
  display: flex;
  align-items: flex-start;
  gap: var(--space-3, 12px);
  padding: 12px 14px;
  border-radius: var(--radius-md, 8px);
  border: 1px solid var(--color-border);
  background: var(--color-bg-secondary, var(--color-bg));
  cursor: pointer;
  transition: all var(--transition-fast, 0.15s);
  user-select: none;
}

.feedback-card:hover {
  border-color: var(--color-primary);
  background: var(--color-bg-hover);
  transform: translateY(-1px);
  box-shadow: 0 4px 12px rgba(0, 0, 0, 0.08);
}

.feedback-card:active {
  transform: scale(0.985);
  opacity: 0.9;
}

.card-icon {
  width: 34px;
  height: 34px;
  border-radius: var(--radius-sm, 6px);
  display: flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
  margin-top: 2px;
}

.bug-icon {
  background: rgba(239, 68, 68, 0.12);
  color: #ef4444;
}

.feature-icon {
  background: rgba(245, 158, 11, 0.12);
  color: #f59e0b;
}

.search-icon {
  background: rgba(59, 130, 246, 0.12);
  color: #3b82f6;
}

.card-content {
  flex: 1;
  min-width: 0;
}

.card-title-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-2, 8px);
}

.card-title {
  font-size: var(--text-sm, 13px);
  font-weight: 600;
  color: var(--color-text);
}

.external-indicator {
  color: var(--color-text-muted);
  opacity: 0.6;
  transition: opacity 0.15s, transform 0.15s;
}

.feedback-card:hover .external-indicator {
  opacity: 1;
  color: var(--color-primary);
  transform: translate(1px, -1px);
}

.card-desc {
  margin: 3px 0 0;
  font-size: var(--text-xs, 12px);
  color: var(--color-text-secondary);
  line-height: 1.4;
}

.feedback-footer {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: var(--space-3, 12px) var(--space-5, 20px);
  border-top: 1px solid var(--color-border);
  background: var(--color-bg-secondary, var(--color-bg));
}

.footer-tip {
  display: flex;
  align-items: center;
  gap: 5px;
  font-size: var(--text-2xs, 11px);
  color: var(--color-text-muted);
}

.footer-actions {
  display: flex;
  align-items: center;
  gap: var(--space-2, 8px);
}

.btn-cancel {
  padding: 6px 14px;
  font-size: var(--text-xs, 12px);
  border-radius: var(--radius-md, 6px);
  border: 1px solid var(--color-border);
  background: var(--color-bg);
  color: var(--color-text-secondary);
  cursor: pointer;
  transition: all var(--transition-fast, 0.15s);
}

.btn-cancel:hover {
  background: var(--color-bg-hover);
  color: var(--color-text);
}

.btn-cancel:active {
  transform: scale(0.96);
}
</style>

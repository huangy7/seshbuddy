<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from "vue";
import { getVersion } from "@tauri-apps/api/app";
import { changelog } from "../changelog";
import { t } from "../i18n";
import { renderMarkdown } from "../utils/markdown";
import { openExternalUrl } from "../utils/openExternalUrl";
import { renderAppError } from "../utils/invokeApp";
import { useUpdater } from "../composables/useUpdater";
import SvgIcon from "./icons/SvgIcon.vue";
import mascot from "../assets/mascot.png";

const props = withDefaults(defineProps<{ active?: boolean }>(), {
  active: true,
});

const emit = defineEmits<{
  openFeedback: [];
}>();

let cachedAppVersion: string | null = null;

const appVersion = ref("");
type UpdateFeedbackLocation = "header" | "token";
const updateError = ref("");
const updateErrorLocation = ref<UpdateFeedbackLocation>("header");
const updateNotice = ref("");
const updateNoticeTone = ref<"info" | "success">("info");
const updateNoticeLocation = ref<UpdateFeedbackLocation>("header");
let updateFeedbackTimer: number | null = null;

const {
  settings: updaterSettings,
  checking,
  loadingState: updaterLoading,
  savingSettings,
  loadState: loadUpdaterState,
  saveSettings: saveUpdaterSettings,
  hasAvailableUpdate,
  tauriUpdateInfo,
  tauriDownloadProgress,
  installingTauriUpdate,
  checkTauriUpdate,
  installTauriUpdate,
} = useUpdater();

const checkedAndIsLatest = ref(false);
const showReleaseNotes = ref(false);

// 更新日志手风琴：默认仅展开最新版本，更早版本折叠为单行，过老版本收纳到「显示更早版本」
const RECENT_CHANGELOG_COUNT = 5;
const expandedVersions = ref<Set<string>>(new Set([changelog[0]?.version ?? ""]));
const showAllChangelog = ref(false);

const currentChangelogVersion = computed(() => appVersion.value || changelog[0]?.version || "");

const visibleChangelog = computed(() =>
  showAllChangelog.value ? changelog : changelog.slice(0, 1 + RECENT_CHANGELOG_COUNT),
);

const hiddenChangelogCount = computed(() => changelog.length - visibleChangelog.value.length);

function toggleChangelogEntry(version: string) {
  const next = new Set(expandedVersions.value);
  if (next.has(version)) {
    next.delete(version);
  } else {
    next.add(version);
  }
  expandedVersions.value = next;
}

const updateSummary = computed(() => {
  if (installingTauriUpdate.value) {
    const { downloaded, total } = tauriDownloadProgress.value;
    if (total && total > 0) {
      const totalMb = (total / 1024 / 1024).toFixed(1);
      const downloadedMb = (downloaded / 1024 / 1024).toFixed(1);
      const percent = Math.min(100, Math.floor((downloaded / total) * 100));
      return t("app.about.downloadingWithProgress", {
        percent,
        downloaded: downloadedMb,
        total: totalMb,
      });
    }
    if (downloaded > 0) {
      const downloadedMb = (downloaded / 1024 / 1024).toFixed(1);
      return t("app.about.downloadingWithSize", { downloaded: downloadedMb });
    }
    return t("app.about.downloading");
  }

  if (checking.value) return t("app.about.checking");
  if (tauriUpdateInfo.value) {
    return t("app.about.updateAvailable", { version: tauriUpdateInfo.value.version });
  }  if (checkedAndIsLatest.value) {
    return t("app.about.upToDate");
  }
  return t("app.about.ready");
});


function clearUpdateFeedbackTimer() {
  if (updateFeedbackTimer !== null) {
    window.clearTimeout(updateFeedbackTimer);
    updateFeedbackTimer = null;
  }
}

function clearUpdateFeedback() {
  updateNotice.value = "";
  updateNoticeLocation.value = "header";
  updateError.value = "";
  updateErrorLocation.value = "header";
}

function scheduleUpdateFeedbackClear(delay = 5000) {
  clearUpdateFeedbackTimer();
  updateFeedbackTimer = window.setTimeout(() => {
    clearUpdateFeedback();
    updateFeedbackTimer = null;
  }, delay);
}

function setUpdateNotice(
  message: string,
  tone: "info" | "success" = "info",
  location: UpdateFeedbackLocation = "header",
  autoHideMs = 4000,
) {
  updateError.value = "";
  updateNotice.value = message;
  updateNoticeTone.value = tone;
  updateNoticeLocation.value = location;
  if (autoHideMs > 0) {
    scheduleUpdateFeedbackClear(autoHideMs);
  } else {
    clearUpdateFeedbackTimer();
  }
}

function setUpdateError(
  error: unknown,
  location: UpdateFeedbackLocation = "header",
  autoHideMs = 5000,
) {
  updateNotice.value = "";
  updateError.value = renderAppError(error);
  updateErrorLocation.value = location;
  if (autoHideMs > 0) {
    scheduleUpdateFeedbackClear(autoHideMs);
  } else {
    clearUpdateFeedbackTimer();
  }
}

onBeforeUnmount(() => {
  clearUpdateFeedbackTimer();
});

async function ensureVersionInfo() {
  if (!props.active) return;
  if (cachedAppVersion) {
    appVersion.value = cachedAppVersion;
    return;
  }

  try {
    cachedAppVersion = await getVersion();
    appVersion.value = cachedAppVersion;
  } catch {
    appVersion.value = appVersion.value || "--";
  }
}

async function ensureUpdaterInfo() {
  try {
    await loadUpdaterState();
    clearUpdateFeedback();
    

  } catch (error) {
    setUpdateError(error, "header", 6000);
  }
}



async function onUpdaterAutoCheckChange(checked: boolean) {
  try {
    await saveUpdaterSettings({
      autoCheck: checked,
    });
    setUpdateNotice(
      checked ? t("app.about.autoCheckEnabled") : t("app.about.autoCheckDisabled"),
      "success",
      "header",
    );
  } catch (error) {
    setUpdateError(error, "header");
  }
}



async function runUpdateCheck() {
  if (checking.value || installingTauriUpdate.value) return;
  try {
    clearUpdateFeedback();
    checkedAndIsLatest.value = false;
    showReleaseNotes.value = false;    

    // Try Tauri updater path first
    const tauriResult = await checkTauriUpdate();
    if (tauriResult !== null) {
      // Tauri updater protocol: update found
      const msg = t("app.about.updateAvailable", { version: tauriResult.version });
      setUpdateNotice(msg, "info", "header", 0);
      return;
    }

    // Tauri updater protocol unavailable or no update
    checkedAndIsLatest.value = true;
    setUpdateNotice(t("app.about.upToDate"), "success", "header", 5000);
  } catch (error) {
    setUpdateError(error, "header", 6000);
  }
}

async function startTauriInstall() {
  try {
    const isWindows = navigator.userAgent.includes("Windows");
    const isMac = navigator.userAgent.includes("Macintosh");
    if (isWindows) {
      setUpdateNotice(t("app.about.installingQuit"), "info", "header", 0);
    } else if (isMac) {
      setUpdateNotice(t("app.about.installingRestart"), "info", "header", 0);
    } else {
      setUpdateNotice(t("app.about.installing"), "info", "header", 0);
    }
    await installTauriUpdate();
    // macOS: won't reach here (relaunched)
    // Windows: won't reach here (exited)
  } catch (error) {
    setUpdateError(error, "header", 6000);
  }
}

function isUrl(s: string): boolean {
  return s.startsWith("http://") || s.startsWith("https://");
}

// 发布说明按钮的三档逻辑：
//   1. notes 有内容且不是 URL → 切换内联展示
//   2. notes 是 URL → 在浏览器打开
//   3. notes 为空 → 回退到 releasePageUrl
const hasInlineNotes = computed(() => {
  const notes = tauriUpdateInfo.value?.notes;
  return !!notes && !isUrl(notes);
});

const releaseNotesButtonLabel = computed(() => {
  if (hasInlineNotes.value) {
    return showReleaseNotes.value ? t("app.about.hideNotes") : t("app.about.viewReleaseNotes");
  }
  return t("app.about.viewReleaseNotes");
});

const hasReleaseNotesAction = computed(() => {
  const info = tauriUpdateInfo.value;
  if (!info) return false;
  return !!(info.notes || info.releasePageUrl);
});

async function handleReleaseNotes() {
  const info = tauriUpdateInfo.value;
  if (!info) return;
  const { notes } = info;

  if (notes && !isUrl(notes)) {
    // Case 1: 纯文本内容 → 内联展示/收起
    showReleaseNotes.value = !showReleaseNotes.value;
  } else if (notes && isUrl(notes)) {
    // Case 2: notes 本身是 URL → 打开
    try {
      await openExternalUrl(notes);
    } catch (error) {
      setUpdateError(error, "header", 6000);
    }
  }
}

watch(
  () => props.active,
  (active) => {
    if (!active) return;
    void ensureVersionInfo();
    void ensureUpdaterInfo();
  },
  { immediate: true },
);


</script>

<template>
  <div class="about-page">
    <section class="about-hero">
      <div class="hero-left">
        <img class="hero-logo" :src="mascot" alt="SeshBuddy" />
      </div>
      <div class="hero-right">
        <h2 class="hero-title">SeshBuddy <span class="hero-version">v{{ appVersion || "..." }}</span></h2>
        <p class="hero-status" :class="{ 'has-update': hasAvailableUpdate }">
          <SvgIcon v-if="hasAvailableUpdate" name="zap" :size="14" class="status-icon" />
          {{ updateSummary }}
        </p>
        <div class="hero-actions">
          <button class="action-btn action-btn-primary" type="button" :disabled="checking || installingTauriUpdate" @click="runUpdateCheck">
            {{ checking ? t("app.about.checkingShort") : t("app.about.checkNow") }}
          </button>

          <template v-if="tauriUpdateInfo">
            <button
              class="action-btn action-btn-primary"
              type="button"
              :disabled="installingTauriUpdate"
              @click="startTauriInstall"
            >
              {{ installingTauriUpdate ? t("app.about.downloadingShort") : t("app.about.downloadAndInstall") }}
            </button>
          </template>

          <button v-if="hasReleaseNotesAction" class="action-btn" type="button" @click="handleReleaseNotes">
            {{ releaseNotesButtonLabel }}
          </button>
          
          <button
            class="action-btn action-btn-outline"
            type="button"
            :title="t('app.about.feedbackTitle')"
            @click="emit('openFeedback')"
          >
            {{ t("app.about.feedback") }}
          </button>
        </div>

        <!-- 内联展示发布说明（支持 Markdown 渲染） -->
        <div v-if="showReleaseNotes && hasInlineNotes" class="release-notes-panel">
          <div class="release-notes-content markdown-body" v-html="renderMarkdown(tauriUpdateInfo?.notes || '')"></div>
        </div>

        <!-- Download progress bar -->
        <div v-if="installingTauriUpdate" class="download-progress-wrap">
          <div class="download-progress-bar">
            <div
              class="download-progress-fill"
              :style="{
                width: tauriDownloadProgress.total && tauriDownloadProgress.total > 0
                  ? Math.min(100, Math.floor((tauriDownloadProgress.downloaded / tauriDownloadProgress.total) * 100)) + '%'
                  : '0%'
              }"
            />
          </div>
        </div>

        <div
          v-if="(updateNotice && updateNoticeLocation === 'header') || (updateError && updateErrorLocation === 'header')"
          class="feedback-stack"
        >
          <div
            v-if="updateNotice && updateNoticeLocation === 'header'"
            class="update-notice"
            :class="`is-${updateNoticeTone}`"
          >
            {{ updateNotice }}
          </div>
          <div v-if="updateError && updateErrorLocation === 'header'" class="settings-error inline-error">
            {{ updateError }}
          </div>
        </div>
      </div>
    </section>

    <section ref="tokenSectionRef" class="settings-section">
      <h3 class="section-title">{{ t("app.about.updateSettings") }}</h3>
      
      <label class="field-toggle" :class="{ 'is-disabled': savingSettings || updaterLoading }">
        <input
          type="checkbox"
          :checked="updaterSettings.autoCheck"
          :disabled="savingSettings || updaterLoading"
          @change="onUpdaterAutoCheckChange(($event.target as HTMLInputElement).checked)"
        />
        <div class="field-copy">
          <span>{{ t("app.about.autoCheck") }}</span>
          <span class="field-hint inline">{{ t("app.about.autoCheckHint") }}</span>
        </div>
      </label>

    </section>

    <section class="settings-section changelog-section">
      <h3 class="section-title">{{ t("app.about.changelog") }}</h3>
      <div v-for="entry in visibleChangelog" :key="entry.version" class="changelog-entry">
        <button
          class="changelog-entry-header"
          type="button"
          :aria-expanded="expandedVersions.has(entry.version)"
          @click="toggleChangelogEntry(entry.version)"
        >
          <SvgIcon
            name="chevron-right"
            :size="12"
            class="changelog-chevron"
            :class="{ 'is-expanded': expandedVersions.has(entry.version) }"
          />
          <span class="changelog-version">v{{ entry.version }}</span>
          <span v-if="entry.version === currentChangelogVersion" class="changelog-current-badge">{{ t("app.about.currentVersion") }}</span>
          <span class="changelog-date">{{ entry.date }}</span>
          <span v-if="!expandedVersions.has(entry.version)" class="changelog-count">{{ t("app.about.changeCount", { count: entry.changeKeys.length }) }}</span>
        </button>
        <ul v-if="expandedVersions.has(entry.version)" class="changelog-changes">
          <li v-for="(changeKey, i) in entry.changeKeys" :key="i">{{ t(changeKey) }}</li>
        </ul>
      </div>
      <button
        v-if="hiddenChangelogCount > 0"
        class="changelog-more-btn"
        type="button"
        @click="showAllChangelog = true"
      >
        {{ t("app.about.showOlder", { count: hiddenChangelogCount }) }}
      </button>
    </section>
  </div>
</template>

<style scoped>
.about-page {
  max-width: 620px;
  margin: 0 auto;
  padding: var(--space-6) var(--space-4);
  display: flex;
  flex-direction: column;
  gap: var(--space-8);
}

.about-hero {
  display: flex;
  align-items: flex-start;
  gap: var(--space-4);
}

.hero-logo {
  width: 64px;
  height: 64px;
  object-fit: contain;
  display: block;
  flex: 0 0 auto;
}

.hero-right {
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  min-width: 0;
}

.hero-title {
  margin: 0;
  font-size: 24px;
  font-weight: 700;
  color: var(--color-text);
  display: flex;
  align-items: center;
  gap: var(--space-3);
  line-height: 1.2;
}

.hero-version {
  font-size: var(--text-xs);
  font-weight: 500;
  font-family: var(--font-mono);
  color: var(--color-text-secondary);
  background: var(--color-bg-hover);
  padding: 2px 8px;
  border-radius: var(--radius-full);
  border: 1px solid var(--color-border);
}

.hero-status {
  margin: var(--space-2) 0 0;
  font-size: var(--text-sm);
  color: var(--color-text-secondary);
  display: flex;
  align-items: center;
  gap: 6px;
}

.hero-status.has-update {
  color: var(--color-primary);
  font-weight: 500;
}

.status-icon {
  flex-shrink: 0;
}

.hero-actions {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  margin-top: var(--space-4);
}

.release-notes-panel {
  width: 100%;
  margin-top: var(--space-3);
  border: 1px solid var(--color-border);
  border-radius: var(--radius-md);
  background: var(--color-bg);
  overflow: hidden;
}

.release-notes-content {
  margin: 0;
  padding: var(--space-4);
  font-size: var(--text-sm);
  color: var(--color-text-secondary);
  max-height: 240px;
  overflow-y: auto;
}

.release-notes-content.markdown-body {
  font-size: 13px;
  line-height: 1.5;
}

.release-notes-content.markdown-body :deep(h3) {
  margin-top: 0;
  font-size: 14px;
}

.release-notes-content.markdown-body :deep(ul) {
  margin-bottom: 0;
}

.download-progress-wrap {
  width: 100%;
  margin-top: var(--space-3);
}

.download-progress-bar {
  width: 100%;
  height: 6px;
  background: var(--color-border);
  border-radius: var(--radius-full);
  overflow: hidden;
}

.download-progress-fill {
  height: 100%;
  background: linear-gradient(90deg, var(--color-primary) 0%, color-mix(in srgb, var(--color-primary) 70%, white) 100%);
  border-radius: var(--radius-full);
  transition: width var(--transition-slow);
  min-width: 4px;
  position: relative;
  overflow: hidden;
}

.download-progress-fill::after {
  content: '';
  position: absolute;
  inset: 0;
  background: linear-gradient(90deg, transparent 0%, rgba(255,255,255,0.35) 50%, transparent 100%);
  animation: progress-shimmer 1.4s infinite;
}

@keyframes progress-shimmer {
  0%   { transform: translateX(-100%); }
  100% { transform: translateX(100%); }
}

.settings-section {
  display: flex;
  flex-direction: column;
}

.section-title {
  font-size: var(--text-sm);
  font-weight: 600;
  margin: 0 0 var(--space-4);
  color: var(--color-text);
  padding-bottom: var(--space-2);
  border-bottom: 1px solid var(--color-border-light);
}

.field-toggle {
  display: flex;
  align-items: flex-start;
  gap: var(--space-2);
  font-size: var(--text-sm);
  cursor: pointer;
}

.field-toggle input {
  margin-top: 2px;
  cursor: pointer;
}

.field-toggle.is-disabled {
  opacity: 0.7;
  cursor: not-allowed;
}

.field-copy {
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.field-input {
  flex: 1 1 240px;
  min-width: 0;
  width: auto;
  margin: 0;
  padding: 8px 10px;
  font-size: var(--text-sm);
  color: var(--color-text);
  background: var(--color-bg);
  border: 1px solid var(--color-border);
  border-radius: var(--radius-md);
  transition: border-color var(--transition-fast), box-shadow var(--transition-fast);
  box-sizing: border-box;
}

.field-input:focus {
  outline: none;
  border-color: var(--color-primary);
  box-shadow: 0 0 0 3px var(--color-primary-ring);
}

.field-hint {
  display: block;
  font-size: 11px;
  color: var(--color-text-muted);
  margin-top: var(--space-2);
  line-height: 1.5;
}

.field-hint.inline {
  margin-top: 0;
}

.action-btn {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  padding: 7px 14px;
  font-size: var(--text-xs);
  font-weight: 500;
  border: 1px solid var(--color-border);
  border-radius: var(--radius-md);
  color: var(--color-text-secondary);
  background: var(--color-bg);
  cursor: pointer;
  transition: all var(--transition-fast);
}

.action-btn:hover:not(:disabled) {
  background: var(--color-bg-hover);
  color: var(--color-text);
}

.action-btn:active:not(:disabled) {
  transform: scale(0.96);
  opacity: 0.85;
}

.action-btn:disabled {
  opacity: 0.55;
  cursor: not-allowed;
}

.action-btn-primary {
  color: white;
  background: var(--color-primary);
  border-color: var(--color-primary);
}

.action-btn-primary:hover:not(:disabled) {
  background: var(--color-primary-hover);
  border-color: var(--color-primary-hover);
}

.action-btn-outline {
  color: var(--color-text-secondary);
  background: transparent;
  border: 1px solid var(--color-border);
}

.action-btn-outline:hover:not(:disabled) {
  background: var(--color-bg-hover);
  color: var(--color-text);
  border-color: var(--color-text-tertiary, var(--color-border));
}

.feedback-stack {
  margin-top: var(--space-3);
  width: 100%;
}

.settings-error,
.update-notice {
  padding: var(--space-2) var(--space-3);
  font-size: var(--text-xs);
  border-radius: var(--radius-md);
  border: 1px solid transparent;
}

.settings-error {
  color: var(--color-danger);
  background: rgba(220, 38, 38, 0.08);
  border-color: rgba(220, 38, 38, 0.16);
}

.inline-error {
  margin-top: var(--space-2);
}

.update-notice {
  color: var(--color-text-secondary);
  background: rgba(59, 130, 246, 0.08);
  border-color: rgba(59, 130, 246, 0.16);
}

.update-notice.is-success {
  color: var(--color-success);
  background: rgba(22, 163, 74, 0.08);
  border-color: rgba(22, 163, 74, 0.16);
}

.changelog-entry {
  padding: var(--space-2) 0;
  border-bottom: 1px dashed var(--color-border-light);
}

.changelog-entry:last-of-type {
  border-bottom: none;
}

.changelog-entry-header {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  width: 100%;
  padding: 4px 6px;
  margin: 0 0 0 -6px;
  border: none;
  background: transparent;
  cursor: pointer;
  border-radius: var(--radius-sm);
  text-align: left;
  transition: background var(--transition-fast);
}

.changelog-entry-header:hover {
  background: var(--color-bg-hover);
}

.changelog-chevron {
  flex-shrink: 0;
  color: var(--color-text-muted);
  transition: transform var(--transition-fast);
}

.changelog-chevron.is-expanded {
  transform: rotate(90deg);
}

.changelog-version {
  font-size: var(--text-sm);
  font-weight: 600;
  color: var(--color-primary);
  font-family: var(--font-mono);
}

.changelog-current-badge {
  font-size: 10px;
  font-weight: 500;
  color: var(--color-primary);
  background: var(--color-primary-ring);
  padding: 1px 8px;
  border-radius: var(--radius-full);
}

.changelog-date {
  font-size: var(--text-xs);
  color: var(--color-text-muted);
}

.changelog-count {
  margin-left: auto;
  font-size: var(--text-xs);
  color: var(--color-text-muted);
}

.changelog-more-btn {
  align-self: flex-start;
  margin-top: var(--space-3);
  padding: 5px 12px;
  font-size: var(--text-xs);
  color: var(--color-text-secondary);
  background: transparent;
  border: 1px dashed var(--color-border);
  border-radius: var(--radius-md);
  cursor: pointer;
  transition: all var(--transition-fast);
}

.changelog-more-btn:hover {
  color: var(--color-text);
  background: var(--color-bg-hover);
}

.changelog-changes {
  margin: 2px 0 4px;
  padding: 0 0 0 calc(var(--space-4) + 6px);
  list-style: none;
}

.changelog-changes li {
  position: relative;
  font-size: var(--text-xs);
  color: var(--color-text-secondary);
  line-height: var(--leading-relaxed);
  padding: 2px 0;
}

.changelog-changes li::before {
  content: "-";
  position: absolute;
  left: -12px;
  color: var(--color-text-muted);
}

@media (max-width: 720px) {
  .about-page {
    padding-inline: var(--space-4);
  }

  .about-hero {
    flex-direction: column;
    align-items: center;
    text-align: center;
  }
  
  .hero-right {
    align-items: center;
  }
  
  .hero-title {
    justify-content: center;
  }
}
</style>

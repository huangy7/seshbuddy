<script setup lang="ts">
import { ref, computed, watch } from "vue";
import { t } from "../../i18n";
import { appErrorCode, invokeApp, renderAppError } from "../../utils/invokeApp";
import { copyToClipboard } from "../../utils/clipboard";
import { open } from "@tauri-apps/plugin-dialog";
import SvgIcon from "../icons/SvgIcon.vue";
import { useSessions } from "../../composables/useSessions";

export interface TabInfo {
  id: string;
  cli_id: string;
  name: string;
  sort_order: number;
  dirs: string[];
}

const props = defineProps<{
  tab: TabInfo;
  existingNames: string[];
}>();

const emit = defineEmits<{
  close: [];
  submit: [name: string, dirs: string[]];
}>();

const { projects } = useSessions();

const tabName = ref(props.tab.name);
const selectedDirs = ref<string[]>([...props.tab.dirs]);
const customDirs = ref<string[]>([]);
const isSubmitting = ref(false);
const submitError = ref("");
const copiedPath = ref("");
let copyTimeout: any = null;
const missingDirs = ref<Set<string>>(new Set());
const actionNotice = ref<{ type: "error" | "info" | "success"; message: string } | null>(null);
let noticeTimer: any = null;

function showNotice(type: "error" | "info" | "success", message: string, duration = 3500) {
  actionNotice.value = { type, message };
  if (noticeTimer) clearTimeout(noticeTimer);
  noticeTimer = setTimeout(() => {
    actionNotice.value = null;
  }, duration);
}

async function copyPath(path: string) {
  if (await copyToClipboard(path)) {
    copiedPath.value = path;
    if (copyTimeout) clearTimeout(copyTimeout);
    copyTimeout = setTimeout(() => {
      copiedPath.value = "";
    }, 1500);
  } else {
    console.error("Failed to copy path");
  }
}

async function checkDirectoriesExistence(dirs: string[]) {
  if (!dirs.length) return;
  try {
    const res = await invokeApp<Record<string, boolean>>("check_paths_exist", { paths: dirs });
    const missing = new Set<string>();
    for (const [p, exists] of Object.entries(res)) {
      if (!exists) {
        missing.add(p);
      }
    }
    missingDirs.value = missing;
  } catch (err) {
    console.error("Failed to check paths existence:", err);
  }
}

async function openInFinder(path: string) {
  try {
    await invokeApp("open_path_in_file_manager", { path });
  } catch (err: any) {
    console.error("Failed to open in file manager:", err);
    missingDirs.value = new Set([...missingDirs.value, path]);
    await copyPath(path);
    // 判据是后端的错误码（`system_ops.path_missing`），不是错误文案：文案随语言变，
    // 按它做子串匹配时译文一变分支就静默失效。
    const isNotFound = appErrorCode(err) === "system_ops.path_missing";
    const noticeText = isNotFound
      ? t("api-profile.dialog.common.openFailedMissing")
      : t("api-profile.dialog.common.openFailed", {
          error:
            renderAppError(err) || t("api-profile.dialog.common.fileManagerUnavailable"),
        });
    showNotice("error", noticeText);
  }
}

watch(
  () => props.tab,
  (newTab) => {
    tabName.value = newTab.name;
    selectedDirs.value = [...newTab.dirs];
    customDirs.value = [];
  },
  { deep: true }
);

function projectName(path: string): string {
  return path.split("/").pop() || path;
}

const allProjects = computed(() => {
  // 解析不出项目路径的会话不构成候选目录：它没有可配置的目录
  const list = projects.value.flatMap((p) =>
    p.original_path ? [{ name: projectName(p.original_path), path: p.original_path }] : []
  );

  // Ensure directories already bound to the tab are in the list
  for (const path of props.tab.dirs) {
    if (!list.some((item) => item.path === path)) {
      list.push({
        name: projectName(path),
        path,
      });
    }
  }

  // Add newly browsed dirs
  for (const path of customDirs.value) {
    if (!list.some((item) => item.path === path)) {
      list.push({
        name: projectName(path),
        path,
      });
    }
  }

  return list;
});

watch(
  allProjects,
  (list) => {
    const paths = list.map((item) => item.path);
    checkDirectoriesExistence(paths);
  },
  { immediate: true, deep: true }
);

const inputTouched = ref(false);

const nameError = computed(() => {
  const trimmed = tabName.value.trim();
  if (!trimmed) {
    return inputTouched.value ? t("api-profile.dialog.common.nameRequired") : "";
  }
  if (trimmed.toLowerCase() === "global") {
    return t("api-profile.dialog.common.nameReserved");
  }
  // Exclude current tab name from duplicates check
  const duplicates = props.existingNames.filter(
    (n) => n.toLowerCase() !== props.tab.name.toLowerCase()
  );
  if (duplicates.some((n) => n.toLowerCase() === trimmed.toLowerCase())) {
    return t("api-profile.dialog.common.nameDuplicate");
  }
  return "";
});

const isValid = computed(() => {
  const trimmed = tabName.value.trim();
  return trimmed.length > 0 && !nameError.value;
});

const submitDisabledReason = computed(() => {
  if (isSubmitting.value) return t("api-profile.dialog.scopeEdit.saving");
  const trimmed = tabName.value.trim();
  if (!trimmed) return t("api-profile.dialog.scopeEdit.nameEmptyHint");
  if (nameError.value) return nameError.value;
  return "";
});

async function browseDirectory() {
  try {
    const selected = await open({
      directory: true,
      multiple: false,
      title: t("api-profile.dialog.common.chooseDirTitle"),
    });
    if (selected && typeof selected === "string") {
      if (!customDirs.value.includes(selected)) {
        customDirs.value.push(selected);
      }
      if (!selectedDirs.value.includes(selected)) {
        selectedDirs.value.push(selected);
      }
    }
  } catch (err: any) {
    console.error("Browse directory failed:", err);
  }
}

async function handleSubmit() {
  if (!isValid.value) {
    inputTouched.value = true;
    return;
  }
  isSubmitting.value = true;
  submitError.value = "";
  try {
    emit("submit", tabName.value.trim(), selectedDirs.value);
  } catch (err: any) {
    submitError.value = renderAppError(err) || t("api-profile.dialog.scopeEdit.updateFailed");
    isSubmitting.value = false;
  }
}
</script>

<template>
  <div class="modal-backdrop" @click.self="emit('close')">
    <div class="dialog">
      <div class="dialog-header">
        <h3>{{ t("api-profile.dialog.scopeEdit.title") }}</h3>
        <button class="dialog-close icon-btn" @click="emit('close')" :title="t('dialogs.common.close')">
          <SvgIcon name="x" :size="16" />
        </button>
      </div>

      <div class="dialog-body">
        <div class="form-group">
          <div class="field-label-row">
            <label class="field-label" for="edit-scope-name-input">
              {{ t("api-profile.dialog.common.nameLabel") }} <span class="required-star">*</span>
            </label>
          </div>
          <input
            id="edit-scope-name-input"
            v-model="tabName"
            type="text"
            :placeholder="t('api-profile.dialog.scopeEdit.namePlaceholder')"
            class="text-input"
            :class="{ 'has-error': !!nameError }"
            autocomplete="off"
            autocorrect="off"
            autocapitalize="none"
            spellcheck="false"
            :disabled="isSubmitting"
            @blur="inputTouched = true"
            @keyup.enter="handleSubmit"
          />
          <span v-if="nameError" class="field-error">{{ nameError }}</span>
        </div>

        <div class="form-group">
          <div class="project-header">
            <label class="field-label">{{ t("api-profile.dialog.common.projectDirsLabel") }}</label>
            <button
              type="button"
              class="btn-browse"
              :disabled="isSubmitting"
              @click="browseDirectory"
            >
              <SvgIcon name="plus" :size="12" />
              <span>{{ t("api-profile.dialog.common.browse") }}</span>
            </button>
          </div>

          <div class="project-list-container">
            <div v-if="allProjects.length === 0" class="empty-projects">
              {{ t("api-profile.dialog.common.emptyProjects") }}
            </div>
            <div
              v-for="proj in allProjects"
              :key="proj.path"
              class="project-item"
              :class="{ 'is-missing': missingDirs.has(proj.path) }"
            >
              <label class="checkbox-label">
                <input
                  v-model="selectedDirs"
                  type="checkbox"
                  :value="proj.path"
                  :disabled="isSubmitting"
                />
                <span class="project-name" :title="proj.name">{{ proj.name }}</span>
                <span
                  v-if="missingDirs.has(proj.path)"
                  class="missing-badge"
                  :title="t('api-profile.dialog.common.missingDirTitle')"
                >
                  {{ t("api-profile.dialog.common.staleBadge") }}
                </span>
                <span
                  class="project-path"
                  :class="{ 'is-missing-path': missingDirs.has(proj.path) }"
                  :title="missingDirs.has(proj.path) ? t('api-profile.dialog.common.pathMissing', { path: proj.path }) : proj.path"
                >
                  {{ proj.path }}
                </span>
              </label>

              <div class="project-actions">
                <button
                  type="button"
                  class="project-action-btn"
                  :class="{ 'is-copied': copiedPath === proj.path }"
                  :title="copiedPath === proj.path ? t('api-profile.dialog.common.pathCopied') : t('api-profile.dialog.common.copyFullPath')"
                  @click.stop.prevent="copyPath(proj.path)"
                  @mousedown.stop
                >
                  <SvgIcon :name="copiedPath === proj.path ? 'check' : 'copy'" :size="12" />
                </button>
                <button
                  type="button"
                  class="project-action-btn"
                  :class="{ 'is-missing-btn': missingDirs.has(proj.path) }"
                  :title="missingDirs.has(proj.path) ? t('api-profile.dialog.common.dirMissingTitle') : t('api-profile.dialog.common.openInFileManagerTitle')"
                  @click.stop.prevent="openInFinder(proj.path)"
                  @mousedown.stop
                >
                  <SvgIcon name="external-link" :size="12" />
                </button>
              </div>
            </div>
          </div>

          <!-- 操作反馈通知 -->
          <Transition name="fade">
            <div v-if="actionNotice" class="action-notice" :class="`is-${actionNotice.type}`">
              <SvgIcon :name="actionNotice.type === 'error' ? 'alert-circle' : 'check'" :size="13" />
              <span>{{ actionNotice.message }}</span>
              <button type="button" class="notice-close" :title="t('api-profile.dialog.common.closeNotice')" @click="actionNotice = null">
                <SvgIcon name="x" :size="12" />
              </button>
            </div>
          </Transition>
        </div>

        <div v-if="submitError" class="dialog-error-banner">
          {{ submitError }}
        </div>
      </div>

      <div class="dialog-footer">
        <button
          type="button"
          class="btn-cancel"
          :disabled="isSubmitting"
          @click="emit('close')"
        >
          {{ t("dialogs.common.cancel") }}
        </button>
        <button
          type="button"
          class="btn-submit"
          :disabled="!isValid || isSubmitting"
          :title="submitDisabledReason"
          @click="handleSubmit"
        >
          {{ isSubmitting ? t("api-profile.rawEditor.saving") : t("app.dialog.save") }}
        </button>
      </div>
    </div>
  </div>
</template>

<style scoped>
/* Share the exact styling with ProfileTabCreateDialog.vue for visual consistency */

.dialog {
  width: 480px;
  background: var(--color-bg);
  border: 1px solid var(--color-border);
  border-radius: var(--radius-xl);
  box-shadow: var(--shadow-lg);
  display: flex;
  flex-direction: column;
  overflow: hidden;
}

.dialog-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: var(--space-4) var(--space-5);
  border-bottom: 1px solid var(--color-border);
}

.dialog-header h3 {
  font-size: var(--text-base);
  font-weight: 600;
  color: var(--color-text);
  margin: 0;
}

.dialog-close {
  border-radius: var(--radius-sm);
}

.dialog-body {
  padding: var(--space-4) var(--space-5);
  display: flex;
  flex-direction: column;
  gap: var(--space-4);
}

.form-group {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
}

.field-label-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
}

.field-label {
  font-size: var(--text-xs);
  font-weight: 600;
  color: var(--color-text-secondary);
}

.required-star {
  color: var(--color-danger, #ef4444);
  font-weight: bold;
}

.text-input {
  width: 100%;
  height: 32px;
  padding: 0 var(--space-3);
  font-size: var(--text-sm);
  border: 1px solid var(--color-border);
  border-radius: var(--radius-sm);
  background: var(--color-bg);
  color: var(--color-text);
  outline: none;
  transition: border-color var(--transition-fast);
  box-sizing: border-box;
}

.text-input:focus {
  border-color: var(--color-primary);
}

.text-input.has-error {
  border-color: var(--color-danger, #ef4444);
}

.field-error {
  font-size: var(--text-2xs);
  color: var(--color-danger, #ef4444);
  margin-top: 2px;
}

.project-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
}

.btn-browse {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  padding: 3px var(--space-2);
  font-size: var(--text-2xs);
  border: 1px solid var(--color-border);
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--color-text-secondary);
  cursor: pointer;
  transition: all var(--transition-fast);
}

.btn-browse:hover {
  background: var(--color-bg-hover);
  color: var(--color-text);
}

.project-list-container {
  border: 1px solid var(--color-border);
  border-radius: var(--radius-sm);
  background: var(--color-bg-sidebar);
  max-height: 180px;
  overflow-y: auto;
  padding: var(--space-2);
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.empty-projects {
  padding: var(--space-3);
  font-size: var(--text-xs);
  color: var(--color-text-muted);
  text-align: center;
}

.project-item {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 3px var(--space-2);
  border-radius: var(--radius-sm);
  transition: background var(--transition-fast);
  gap: 6px;
}

.project-item:hover {
  background: var(--color-bg-hover);
}

.checkbox-label {
  display: flex;
  align-items: center;
  gap: 8px;
  flex: 1;
  min-width: 0;
  cursor: pointer;
  font-size: var(--text-sm);
}

.checkbox-label input {
  margin: 0;
  cursor: pointer;
  flex-shrink: 0;
}

.project-name {
  font-weight: 500;
  color: var(--color-text);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  max-width: 120px;
  flex-shrink: 0;
}

.project-path {
  font-size: var(--text-xs);
  color: var(--color-text-muted);
  margin-left: auto;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  max-width: 180px;
  text-align: right;
}

.project-actions {
  display: flex;
  align-items: center;
  gap: 2px;
  flex-shrink: 0;
  opacity: 0.6;
  transition: opacity var(--transition-fast);
}

.project-item:hover .project-actions {
  opacity: 1;
}

.project-action-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 22px;
  height: 22px;
  padding: 0;
  border-radius: var(--radius-sm);
  border: 1px solid transparent;
  background: transparent;
  color: var(--color-text-muted);
  cursor: pointer;
  transition: all var(--transition-fast);
}

.project-action-btn:hover {
  color: var(--color-text);
  background: var(--color-bg);
  border-color: var(--color-border);
}

.project-action-btn.is-copied {
  color: var(--color-success, #10b981);
  background: rgba(16, 185, 129, 0.1);
  border-color: rgba(16, 185, 129, 0.3);
}

.project-action-btn.is-missing-btn {
  color: var(--color-danger, #ef4444);
  opacity: 0.7;
}

.project-action-btn.is-missing-btn:hover {
  opacity: 1;
  background: rgba(239, 68, 68, 0.08);
  border-color: rgba(239, 68, 68, 0.2);
}

.missing-badge {
  font-size: 10px;
  padding: 1px 4px;
  border-radius: var(--radius-xs, 3px);
  background: rgba(239, 68, 68, 0.12);
  color: var(--color-danger, #ef4444);
  border: 1px solid rgba(239, 68, 68, 0.25);
  font-weight: 500;
  flex-shrink: 0;
  line-height: 1.2;
}

.project-item.is-missing .project-name {
  color: var(--color-text-secondary);
}

.project-item.is-missing .project-path {
  max-width: 140px;
}

.is-missing-path {
  text-decoration: line-through;
  opacity: 0.7;
}

.action-notice {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 6px var(--space-3);
  border-radius: var(--radius-sm);
  font-size: var(--text-xs);
  margin-top: var(--space-1);
}

.action-notice.is-error {
  background: rgba(239, 68, 68, 0.1);
  color: var(--color-danger, #ef4444);
  border: 1px solid rgba(239, 68, 68, 0.25);
}

.action-notice.is-success {
  background: rgba(16, 185, 129, 0.1);
  color: var(--color-success, #10b981);
  border: 1px solid rgba(16, 185, 129, 0.25);
}

.action-notice.is-info {
  background: rgba(59, 130, 246, 0.1);
  color: var(--color-primary, #3b82f6);
  border: 1px solid rgba(59, 130, 246, 0.25);
}

.action-notice span {
  flex: 1;
  min-width: 0;
  line-height: 1.4;
}

.notice-close {
  background: transparent;
  border: none;
  color: inherit;
  opacity: 0.6;
  cursor: pointer;
  padding: 2px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border-radius: 2px;
}

.notice-close:hover {
  opacity: 1;
}

.fade-enter-active,
.fade-leave-active {
  transition: opacity 0.2s ease, transform 0.2s ease;
}

.fade-enter-from,
.fade-leave-to {
  opacity: 0;
  transform: translateY(-4px);
}

.dialog-error-banner {
  font-size: var(--text-xs);
  color: var(--color-danger);
  padding: var(--space-2);
  background: rgba(239, 68, 68, 0.1);
  border-radius: var(--radius-sm);
  margin-top: var(--space-2);
}

.dialog-footer {
  display: flex;
  justify-content: flex-end;
  gap: var(--space-2);
  padding: var(--space-3) var(--space-5) var(--space-4);
  border-top: 1px solid var(--color-border);
}

.btn-cancel {
  padding: 6px var(--space-4);
  font-size: var(--text-sm);
  color: var(--color-text-secondary);
  border: 1px solid var(--color-border);
  border-radius: var(--radius-md);
  background: transparent;
  cursor: pointer;
  transition: all var(--transition-fast);
}

.btn-cancel:hover {
  background: var(--color-bg-hover);
}

.btn-submit {
  padding: 6px var(--space-4);
  font-size: var(--text-sm);
  color: white;
  background: var(--color-primary);
  border: none;
  border-radius: var(--radius-md);
  cursor: pointer;
  transition: all var(--transition-fast);
}

.btn-submit:hover {
  opacity: 0.9;
}

.btn-submit:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}
</style>

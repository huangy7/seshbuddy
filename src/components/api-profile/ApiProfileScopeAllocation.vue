<script setup lang="ts">
import { computed } from "vue";
import { t } from "../../i18n";
import SvgIcon from "../icons/SvgIcon.vue";
import ElegantSelect, { type SelectOption } from "../common/ElegantSelect.vue";

export interface ScopeBinding {
  scope: string;
  /** 项目作用域是用户数据（tab 名）；全局作用域为 `null`，标签由本组件按语言渲染 */
  name: string | null;
  isGlobal: boolean;
  dirs: string[];
  activeProfile: string | null;
}

export interface ProfileSummary {
  name: string;
  baseUrl?: string;
  model?: string;
}

/**
 * 后端 `ScopeDirDetail` 的镜像（serde `tag = "kind"`，见 `commands/profile.rs`）。
 * 每个变体承载的是**数据**不是文案：tooltip 由本组件按当前语言渲染。
 */
export type ScopeDirDetail =
  | { kind: "diverged"; dir: string }
  | { kind: "unbound_with_config"; base_url: string }
  | { kind: "dirs_inconsistent"; dirs: string[] };

export interface ScopeDirStatus {
  scope: string;
  /** aligned | unboundWithConfig | diverged | dirsInconsistent | none */
  status: string;
  detail?: ScopeDirDetail | null;
}

const props = withDefaults(
  defineProps<{
    scopeBindings: ScopeBinding[];
    availableProfiles: (string | ProfileSummary)[];
    currentCliName: string;
    currentCliId: string;
    isClaude: boolean;
    scopeDirStatus?: ScopeDirStatus[];
  }>(),
  {
    scopeBindings: () => [],
    availableProfiles: () => [],
    scopeDirStatus: () => [],
  }
);

const emit = defineEmits<{
  changeBinding: [scope: string, profileName: string];
  createScope: [];
  editScope: [scopeId: string];
  deleteScope: [scopeId: string];
  editProfile: [profileName: string];
  createProfileForScope: [scopeId: string];
}>();

const projectScopes = computed<ScopeBinding[]>(() => {
  return props.scopeBindings.filter((b) => !b.isGlobal && b.scope !== "global");
});

const profileOptions = computed<SelectOption[]>(() => {
  const list: SelectOption[] = [
    {
      value: "",
      label: t("api-profile.scopeAllocation.inheritGlobal"),
      icon: "globe",
    },
  ];
  for (const p of props.availableProfiles) {
    const name = typeof p === "string" ? p : p.name;
    const model = typeof p === "string" ? undefined : p.model;
    list.push({
      value: name,
      label: name,
      icon: "zap",
      description: model || undefined,
    });
  }
  return list;
});

function projectName(path: string): string {
  return path.replace(/\\/g, "/").split("/").filter(Boolean).pop() || path;
}

function getDirsTooltip(dirs: string[]): string {
  if (!dirs || dirs.length === 0) return t("api-profile.scopeAllocation.noDirsBound");
  return (
    t("api-profile.scopeAllocation.dirsTooltip", { count: dirs.length }) +
    "\n" +
    dirs.join("\n")
  );
}

function getDirsSummaryText(dirs: string[]): string {
  if (!dirs || dirs.length === 0) return "";
  const names = dirs.map(projectName);
  if (names.length <= 2) {
    return names.join(", ");
  }
  return `${names.slice(0, 2).join(", ")} +${names.length - 2}`;
}

function onProfileSelectChange(scope: string, val: any) {
  emit("changeBinding", scope, String(val ?? ""));
}

interface StatusBadge {
  text: string;
  tooltip: string;
  tone: "info" | "warn";
}

const statusBadges = computed<Map<string, StatusBadge>>(() => {
  const map = new Map<string, StatusBadge>();
  for (const entry of props.scopeDirStatus) {
    const detail = entry.detail;
    switch (entry.status) {
      case "unboundWithConfig":
        map.set(entry.scope, {
          text: t("api-profile.scopeAllocation.statusExisting"),
          tooltip: t("api-profile.scopeAllocation.statusExistingTooltip", {
            detail:
              detail?.kind === "unbound_with_config"
                ? t("api-profile.scopeAllocation.statusExistingDetail", { detail: detail.base_url })
                : "",
          }),
          tone: "info",
        });
        break;
      case "diverged":
        map.set(entry.scope, {
          text: t("api-profile.scopeAllocation.statusDiverged"),
          tooltip: t("api-profile.scopeAllocation.statusDivergedTooltip", {
            detail: detail?.kind === "diverged" ? detail.dir : "",
          }),
          tone: "warn",
        });
        break;
      case "dirsInconsistent": {
        // 计数取自后端给的结构化目录列表（后端只在「有配置的目录多于一个」时才置这个状态）。
        // 列表缺失时退回徽标本身那句：今天不可达（单一生产者），留着只为让分派是完备的。
        const dirs = detail?.kind === "dirs_inconsistent" ? detail.dirs : [];
        map.set(entry.scope, {
          text: t("api-profile.scopeAllocation.statusDirsInconsistent"),
          tooltip: dirs.length
            ? t("api-profile.scopeAllocation.statusDirsInconsistentTooltip", { count: dirs.length })
            : t("api-profile.scopeAllocation.statusDirsInconsistent"),
          tone: "warn",
        });
        break;
      }
    }
  }
  return map;
});
</script>

<template>
  <div class="scope-allocation-section">
    <!-- Project Scopes Section Header -->
    <div class="scope-section-header">
      <div class="title-group">
        <h3 class="section-title">{{ t("api-profile.scopeAllocation.title") }}</h3>
        <span class="section-sub-hint">{{ t("api-profile.scopeAllocation.subtitle") }}</span>
      </div>

      <button
        type="button"
        class="btn-create-scope-header"
        @click="emit('createScope')"
      >
        <SvgIcon name="plus" :size="12" />
        <span>{{ t("api-profile.scopeAllocation.createScope") }}</span>
      </button>
    </div>

    <!-- Project Rows List (Single-line flat cards ~42px) -->
    <div v-if="projectScopes.length > 0" class="scope-rows-list">
      <div
        v-for="scope in projectScopes"
        :key="scope.scope"
        class="scope-row"
      >
        <!-- Left Col: Icon + Scope Name + Dirs Summary Badge -->
        <div class="scope-left">
          <div class="scope-icon-box">
            <SvgIcon name="folder" :size="14" class="scope-folder-icon" />
          </div>

          <div class="scope-info">
            <span class="scope-title" :title="scope.name || ''">{{ scope.name }}</span>

            <!-- Directory Summary Pill with rich full-path tooltip -->
            <div
              v-if="scope.dirs && scope.dirs.length > 0"
              class="dirs-pill"
              :title="getDirsTooltip(scope.dirs)"
              @click="emit('editScope', scope.scope)"
            >
              <span class="dirs-count">{{
                t("api-profile.scopeAllocation.dirsCount", { count: scope.dirs.length })
              }}</span>
              <span class="dirs-names">{{ getDirsSummaryText(scope.dirs) }}</span>
            </div>

            <div
              v-else
              class="dirs-empty-pill"
              :title="t('api-profile.scopeAllocation.bindDirsTitle')"
              @click="emit('editScope', scope.scope)"
            >
              <SvgIcon name="alert-circle" :size="11" />
              <span>{{ t("api-profile.scopeAllocation.unboundDirs") }}</span>
            </div>

            <!-- 磁盘配置状态徽标 -->
            <span
              v-if="statusBadges.get(scope.scope)"
              class="status-badge"
              :class="`tone-${statusBadges.get(scope.scope)!.tone}`"
              :title="statusBadges.get(scope.scope)!.tooltip"
            >
              <SvgIcon name="alert-circle" :size="10" />
              <span>{{ statusBadges.get(scope.scope)!.text }}</span>
            </span>
          </div>
        </div>

        <!-- Right Col: Custom ElegantSelect Dropdown + Action Buttons -->
        <div class="scope-right">
          <div class="scope-select-wrap">
            <ElegantSelect
              :model-value="scope.activeProfile || ''"
              :options="profileOptions"
              size="small"
              :placeholder="t('api-profile.scopeAllocation.inheritGlobal')"
              @change="(val) => onProfileSelectChange(scope.scope, val)"
            >
              <template #footer>
                <div
                  class="select-footer-create"
                  @click="emit('createProfileForScope', scope.scope)"
                >
                  <SvgIcon name="plus" :size="12" />
                  <span>{{ t("api-profile.scopeAllocation.createAndBind") }}</span>
                </div>
              </template>
            </ElegantSelect>
          </div>

          <!-- Actions Group -->
          <div class="scope-actions">
            <button
              type="button"
              class="scope-action-btn"
              :title="t('api-profile.scopeAllocation.manageDirsTitle')"
              @click.stop="emit('editScope', scope.scope)"
            >
              <SvgIcon name="folder" :size="12" />
            </button>
            <button
              type="button"
              class="scope-action-btn action-danger"
              :title="t('api-profile.dialog.scopeDelete.title')"
              @click.stop="emit('deleteScope', scope.scope)"
            >
              <SvgIcon name="trash-2" :size="12" />
            </button>
          </div>
        </div>
      </div>
    </div>

    <!-- Empty State -->
    <div v-else class="project-scopes-empty">
      <div class="empty-icon-wrap">
        <SvgIcon name="folder" :size="20" class="empty-state-icon" />
      </div>
      <p class="empty-state-title">{{ t("api-profile.scopeAllocation.emptyTitle") }}</p>
      <p class="empty-state-desc">
        {{ t("api-profile.scopeAllocation.emptyDesc") }}
      </p>
    </div>
  </div>
</template>

<style scoped>
.scope-allocation-section {
  display: flex;
  flex-direction: column;
  gap: 8px;
  margin-top: 14px;
  padding-top: 14px;
  border-top: 1px solid var(--color-border);
}

.scope-section-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 2px 2px;
}

.title-group {
  display: flex;
  align-items: center;
  gap: 6px;
}

.section-title {
  font-size: 13px;
  font-weight: 600;
  color: var(--color-text);
  margin: 0;
}

.section-sub-hint {
  font-size: 11px;
  color: var(--color-text-muted);
}

.btn-create-scope-header {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  height: 26px;
  padding: 0 9px;
  border-radius: 6px;
  border: 1px solid var(--color-border);
  background: var(--color-bg);
  color: var(--color-text-secondary);
  font-size: 11px;
  font-weight: 500;
  cursor: pointer;
  transition: all var(--transition-fast);
  white-space: nowrap;
}

.btn-create-scope-header:hover {
  color: var(--color-primary);
  border-color: var(--color-primary);
  background: var(--color-bg-hover);
}

/* List of Flat Single-Line Scope Rows (~42px) */
.scope-rows-list {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.scope-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 0 12px;
  height: 42px;
  background: var(--color-bg);
  border: 1px solid var(--color-border);
  border-radius: 6px;
  gap: 12px;
  transition: all 0.12s cubic-bezier(0.16, 1, 0.3, 1);
}

.scope-row:hover {
  border-color: var(--color-primary-light, rgba(37, 99, 235, 0.4));
  background: var(--color-bg-hover, rgba(0, 0, 0, 0.015));
  box-shadow: 0 1px 4px rgba(0, 0, 0, 0.04);
}

/* Left Group */
.scope-left {
  display: flex;
  align-items: center;
  gap: 10px;
  min-width: 0;
  flex: 1;
}

.scope-icon-box {
  width: 26px;
  height: 26px;
  border-radius: 5px;
  background: var(--color-bg-secondary);
  border: 1px solid var(--color-border);
  display: flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
}

.scope-folder-icon {
  color: var(--color-primary);
}

.scope-info {
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
  flex: 1;
}

.scope-title {
  font-size: 12px;
  font-weight: 600;
  color: var(--color-text);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  max-width: 140px;
}

/* Dirs Summary Pill */
.dirs-pill {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  padding: 2px 8px;
  border-radius: 4px;
  background: var(--color-bg-secondary);
  border: 1px solid var(--color-border);
  font-size: 11px;
  color: var(--color-text-muted);
  cursor: pointer;
  transition: all 0.12s ease;
  min-width: 0;
  max-width: 320px;
}

.dirs-pill:hover {
  background: var(--color-bg-hover);
  color: var(--color-text);
  border-color: var(--color-primary-light, rgba(37, 99, 235, 0.3));
}

.dirs-count {
  font-weight: 600;
  color: var(--color-text-secondary);
  white-space: nowrap;
}

.dirs-names {
  color: var(--color-text-muted);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.dirs-empty-pill {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  padding: 2px 8px;
  border-radius: 4px;
  background: rgba(245, 158, 11, 0.08);
  border: 1px dashed rgba(245, 158, 11, 0.4);
  font-size: 11px;
  color: #d97706;
  cursor: pointer;
  transition: all 0.12s ease;
}

.dirs-empty-pill:hover {
  background: rgba(245, 158, 11, 0.15);
}

/* 磁盘配置状态徽标 */
.status-badge {
  display: inline-flex;
  align-items: center;
  gap: 3px;
  padding: 2px 7px;
  border-radius: 4px;
  font-size: 10px;
  font-weight: 500;
  white-space: nowrap;
  flex-shrink: 0;
  cursor: default;
}

.status-badge.tone-info {
  color: #d97706;
  background: rgba(245, 158, 11, 0.08);
  border: 1px solid rgba(245, 158, 11, 0.3);
}

.status-badge.tone-warn {
  color: var(--color-danger, #ef4444);
  background: rgba(239, 68, 68, 0.08);
  border: 1px solid rgba(239, 68, 68, 0.3);
}

/* Right Group */
.scope-right {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-shrink: 0;
}

/* Custom ElegantSelect Dropdown Wrapper */
.scope-select-wrap {
  width: 170px;
  flex-shrink: 0;
}

.select-footer-create {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 8px 12px;
  font-size: 11px;
  font-weight: 500;
  color: var(--color-primary);
  border-top: 1px solid var(--color-border);
  cursor: pointer;
  background: var(--color-bg);
  transition: all 0.12s ease;
}

.select-footer-create:hover {
  background: var(--color-primary-light, rgba(37, 99, 235, 0.08));
  color: var(--color-primary-hover, #1d4ed8);
}

/* Action Icons */
.scope-actions {
  display: flex;
  align-items: center;
  gap: 3px;
  opacity: 0.85;
  transition: opacity 0.12s ease;
}

.scope-row:hover .scope-actions {
  opacity: 1;
}

.scope-action-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 24px;
  height: 24px;
  border: none;
  background: transparent;
  color: var(--color-text-muted);
  border-radius: 4px;
  cursor: pointer;
  transition: all var(--transition-fast);
}

.scope-action-btn:hover {
  background: var(--color-bg-hover);
  color: var(--color-text);
}

.scope-action-btn.action-danger:hover {
  background: rgba(239, 68, 68, 0.1);
  color: var(--color-danger);
}

/* Empty State */
.project-scopes-empty {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  padding: 24px 16px;
  background: var(--color-bg-secondary);
  border: 1px dashed var(--color-border);
  border-radius: var(--radius-md);
  text-align: center;
}

.empty-icon-wrap {
  width: 34px;
  height: 34px;
  display: flex;
  align-items: center;
  justify-content: center;
  border-radius: var(--radius-full);
  background: var(--color-bg);
  border: 1px solid var(--color-border);
  color: var(--color-text-muted);
  margin-bottom: 6px;
}

.empty-state-title {
  font-size: 12px;
  font-weight: 600;
  color: var(--color-text);
  margin: 0 0 2px;
}

.empty-state-desc {
  font-size: 11px;
  color: var(--color-text-muted);
}

/* Animations */
.fade-enter-active,
.fade-leave-active {
  transition: opacity 0.15s ease;
}

.fade-enter-from,
.fade-leave-to {
  opacity: 0;
}
</style>

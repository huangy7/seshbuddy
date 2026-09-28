<script setup lang="ts">
import { ref, computed } from "vue";
import { t } from "../../i18n";
import SvgIcon from "../icons/SvgIcon.vue";
import ElegantSelect, { type SelectOption } from "../common/ElegantSelect.vue";

export interface ImportCandidate {
  cli_id: string;
  scope: string | null;
  name: string;
  content: Record<string, any>;
  is_active: boolean;
  isConflict: boolean;
  action: "overwrite" | "skip" | "rename";
  newName: string;
}

// 选项与状态文案在 computed 内求值：写成模块级常量会把语言冻结在 import 时。
const conflictActionOptions = computed<SelectOption[]>(() => [
  { value: "overwrite", label: t("api-profile.dialog.importConflict.action.overwrite") },
  { value: "rename", label: t("app.ctxMenu.rename") },
  { value: "skip", label: t("api-profile.dialog.importConflict.action.skip") },
]);

// 非冲突的新配置没有"覆盖/重命名"语义，action 沿用 overwrite/skip 取值（overwrite 即新增）
const newConfigActionOptions = computed<SelectOption[]>(() => [
  { value: "overwrite", label: t("api-profile.dialog.importConflict.action.add") },
  { value: "skip", label: t("api-profile.dialog.importConflict.action.skip") },
]);

export interface ScopeBindingInfo {
  cli_id: string;
  scope: string;
  profile_name: string;
}

export type ScopeImportStatus = "existing" | "nameConflict" | "new";
export type ScopeImportAction = "overwrite" | "skip" | "add";

export interface ScopeImportItem {
  id: string;
  cli_id: string;
  name: string;
  dirs: string[];
  status: ScopeImportStatus;
  action: ScopeImportAction;
}

const scopeActionOptions = computed<Record<ScopeImportStatus, SelectOption[]>>(() => ({
  existing: [
    { value: "overwrite", label: t("api-profile.dialog.importConflict.action.overwrite") },
    { value: "skip", label: t("api-profile.dialog.importConflict.action.skip") },
  ],
  nameConflict: [
    { value: "skip", label: t("api-profile.dialog.importConflict.action.skip") },
    { value: "add", label: t("api-profile.dialog.importConflict.action.addCopy") },
  ],
  new: [
    { value: "add", label: t("api-profile.dialog.importConflict.action.add") },
    { value: "skip", label: t("api-profile.dialog.importConflict.action.skip") },
  ],
}));

const scopeStatusLabel = computed<Record<ScopeImportStatus, string>>(() => ({
  existing: t("api-profile.dialog.importConflict.status.existing"),
  nameConflict: t("api-profile.dialog.importConflict.status.nameConflict"),
  new: t("api-profile.dialog.importConflict.status.newScope"),
}));

const props = withDefaults(
  defineProps<{
    candidates: ImportCandidate[];
    scopes?: ScopeImportItem[];
    scopeBindings?: ScopeBindingInfo[];
  }>(),
  { scopes: () => [], scopeBindings: () => [] }
);

const emit = defineEmits<{
  close: [];
  confirm: [items: ImportCandidate[], scopes: ScopeImportItem[]];
}>();

const items = ref<ImportCandidate[]>(
  props.candidates.map((c) => ({ ...c }))
);
const scopeItems = ref<ScopeImportItem[]>(
  props.scopes.map((s) => ({ ...s }))
);

const conflictCount = computed(() => items.value.filter((i) => i.isConflict).length);

// 汇总句由三段拼成：冲突数与作用域数两段只在对应数量大于 0 时出现，
// 片段各自进语言包，避免把整句拆成孤立的量词。
const summaryText = computed(() =>
  t("api-profile.dialog.importConflict.summary", {
    total: items.value.length,
    conflicts:
      conflictCount.value > 0
        ? t("api-profile.dialog.importConflict.summaryConflicts", { count: conflictCount.value })
        : "",
    scopes:
      scopeItems.value.length > 0
        ? t("api-profile.dialog.importConflict.summaryScopes", { count: scopeItems.value.length })
        : "",
  })
);

function bindingFor(scope: ScopeImportItem): string | null {
  const b = props.scopeBindings.find(
    (b) => b.cli_id === scope.cli_id && b.scope === scope.id
  );
  return b?.profile_name ?? null;
}

// 批量处理同时作用到作用域 (用户需求)
function setAllActions(action: "overwrite" | "skip" | "rename") {
  items.value.forEach((item) => {
    if (item.isConflict) {
      item.action = action;
      if (action === "rename" && (!item.newName || item.newName === item.name)) {
        item.newName = `${item.name}_imported`;
      }
    }
  });

  scopeItems.value.forEach((scope) => {
    if (action === "overwrite") {
      if (scope.status === "existing") {
        scope.action = "overwrite";
      }
    } else if (action === "rename") {
      if (scope.status === "nameConflict") {
        scope.action = "add";
      }
    } else if (action === "skip") {
      if (scope.status === "existing" || scope.status === "nameConflict") {
        scope.action = "skip";
      }
    }
  });
}

function onActionChange(item: ImportCandidate) {
  if (item.action === "rename" && (!item.newName || item.newName === item.name)) {
    item.newName = `${item.name}_imported`;
  }
}

function onConfirm() {
  emit("confirm", items.value, scopeItems.value);
}
</script>

<template>
  <div class="modal-backdrop" @click.self="emit('close')">
    <div class="dialog">
      <div class="dialog-header">
        <h3>{{ t("api-profile.dialog.importConflict.title") }}</h3>
        <button class="dialog-close icon-btn" :title="t('dialogs.common.close')" :aria-label="t('dialogs.common.close')" @click="emit('close')">
          <SvgIcon name="x" :size="16" />
        </button>
      </div>

      <div class="dialog-body">
        <div class="summary-banner" :class="{ warn: conflictCount > 0 }">
          <SvgIcon :name="conflictCount > 0 ? 'alert-circle' : 'check'" :size="16" />
          <span>{{ summaryText }}</span>
        </div>

        <div v-if="conflictCount > 0" class="batch-actions">
          <span class="batch-label">{{ t("api-profile.dialog.importConflict.batchLabel") }}</span>
          <button class="btn-sub" type="button" @click="setAllActions('overwrite')">{{ t("api-profile.dialog.importConflict.batchOverwrite") }}</button>
          <button class="btn-sub" type="button" @click="setAllActions('rename')">{{ t("api-profile.dialog.importConflict.batchRename") }}</button>
          <button class="btn-sub" type="button" @click="setAllActions('skip')">{{ t("api-profile.dialog.importConflict.batchSkip") }}</button>
        </div>

        <!-- 作用域恢复模块 -->
        <div v-if="scopeItems.length > 0" class="scope-section">
          <div class="scope-section-title">{{ t("api-profile.dialog.importConflict.scopeSectionTitle") }}</div>
          <div v-for="scope in scopeItems" :key="scope.id" class="scope-item">
            <div class="scope-col-info">
              <SvgIcon name="folder" :size="13" class="scope-icon" />
              <span class="scope-name" :title="scope.name">{{ scope.name }}</span>
              <span class="scope-meta">{{ t("api-profile.scopeAllocation.dirsCount", { count: scope.dirs.length }) }}</span>
            </div>
            <div class="scope-col-binding">
              <span v-if="bindingFor(scope)" class="scope-binding">{{ t("api-profile.dialog.importConflict.boundTo", { name: bindingFor(scope) }) }}</span>
              <span v-else class="scope-binding-empty">-</span>
            </div>
            <div class="scope-col-action">
              <span
                class="scope-status"
                :class="scope.status === 'nameConflict' ? 'status-warn' : (scope.status === 'new' ? 'status-new' : '')"
              >{{ scopeStatusLabel[scope.status] }}</span>
              <div class="scope-action">
                <ElegantSelect
                  v-model="scope.action"
                  :options="scopeActionOptions[scope.status]"
                  size="small"
                />
              </div>
            </div>
          </div>
        </div>

        <!-- 基于 master 经典 5 列表格 UI -->
        <div class="table-wrapper">
          <table class="import-table">
            <thead>
              <tr>
                <th>CLI</th>
                <th>{{ t("api-profile.dialog.importConflict.colScope") }}</th>
                <th>{{ t("api-profile.connection.name") }}</th>
                <th>{{ t("api-profile.dialog.importConflict.colStatus") }}</th>
                <th>{{ t("api-profile.dialog.importConflict.colAction") }}</th>
              </tr>
            </thead>
            <tbody>
              <tr v-for="item in items" :key="`${item.cli_id}-${item.scope}-${item.name}`">
                <td class="cli-tag">{{ item.cli_id.toUpperCase() }}</td>
                <td class="scope-cell">{{ item.scope === 'global' || !item.scope ? t("api-profile.scope.global") : item.scope }}</td>
                <td class="name-cell">{{ item.name }}</td>
                <td>
                  <span v-if="item.isConflict" class="status-warn">{{ t("api-profile.dialog.importConflict.status.nameConflict") }}</span>
                  <span v-else class="status-new">{{ t("api-profile.dialog.importConflict.status.newProfile") }}</span>
                </td>
                <td class="action-cell">
                  <div class="action-select-wrap">
                    <ElegantSelect
                      v-model="item.action"
                      :options="item.isConflict ? conflictActionOptions : newConfigActionOptions"
                      size="small"
                      @change="onActionChange(item)"
                    />
                  </div>
                  <input
                    v-if="item.action === 'rename'"
                    v-model="item.newName"
                    type="text"
                    class="rename-input"
                    :placeholder="t('api-profile.dialog.importConflict.renamePlaceholder')"
                  />
                </td>
              </tr>
            </tbody>
          </table>
        </div>
      </div>

      <div class="dialog-footer">
        <button class="btn-cancel" type="button" @click="emit('close')">{{ t("dialogs.common.cancel") }}</button>
        <button class="btn-primary" type="button" @click="onConfirm">
          <SvgIcon name="download" :size="14" />
          {{ t("api-profile.dialog.importConflict.confirm") }}
        </button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.dialog {
  width: 620px;
  max-height: 85vh;
  background: var(--color-bg);
  border: 1px solid var(--color-border);
  border-radius: var(--radius-xl);
  box-shadow: var(--shadow-lg);
  display: flex;
  flex-direction: column;
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
  margin: 0;
}
.dialog-close {
  background: transparent;
  border: none;
  color: var(--color-text-secondary);
  cursor: pointer;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 4px;
  border-radius: var(--radius-sm);
  transition: background var(--transition-fast);
}
.dialog-close:hover {
  background: var(--color-bg-hover);
  color: var(--color-text);
}
.dialog-body {
  flex: 1;
  min-height: 0;
  padding: var(--space-4);
  display: flex;
  flex-direction: column;
  gap: var(--space-3);
  overflow-y: auto;
}
.summary-banner {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  padding: var(--space-2) var(--space-3);
  border-radius: var(--radius-md);
  background: var(--color-bg-hover);
  font-size: var(--text-xs);
  color: var(--color-text);
}
.summary-banner.warn {
  background: rgba(245, 158, 11, 0.1);
  color: #b45309;
}
.batch-actions {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  font-size: var(--text-xs);
}
.batch-label {
  color: var(--color-text-secondary);
}
.btn-sub {
  padding: 2px 8px;
  font-size: 11px;
  border: 1px solid var(--color-border);
  border-radius: var(--radius-sm);
  background: var(--color-bg);
  color: var(--color-text);
  cursor: pointer;
  transition: all var(--transition-fast);
}
.btn-sub:hover {
  background: var(--color-bg-hover);
  border-color: var(--color-primary);
}

/* 作用域模块 */
.scope-section {
  border: 1px solid var(--color-border);
  border-radius: var(--radius-md);
  padding: 10px 12px;
  font-size: var(--text-xs);
}
.scope-section-title {
  font-weight: 600;
  color: var(--color-text-secondary);
  margin-bottom: 6px;
}
.scope-item {
  display: flex;
  align-items: center;
  gap: 8px;
  color: var(--color-text);
  padding: 5px 6px;
  border-radius: var(--radius-sm);
  transition: background var(--transition-fast);
}
.scope-item:hover {
  background: var(--color-bg-hover);
}
.scope-col-info {
  flex: 1.2;
  min-width: 0;
  display: flex;
  align-items: center;
  gap: 8px;
}
.scope-icon {
  color: var(--color-primary);
  flex-shrink: 0;
}
.scope-name {
  max-width: 160px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-weight: 500;
  color: var(--color-text);
}
.scope-meta {
  color: var(--color-text-muted);
  font-size: 11px;
  background: var(--color-bg-hover);
  padding: 1px 6px;
  border-radius: 4px;
  white-space: nowrap;
}
.scope-col-binding {
  width: 170px;
  flex-shrink: 0;
  display: flex;
  align-items: center;
}
.scope-binding {
  color: var(--color-primary);
  font-size: 12px;
  font-weight: 500;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.scope-binding-empty {
  color: var(--color-text-muted);
  padding-left: 10px;
}
.scope-col-action {
  display: flex;
  align-items: center;
  gap: 10px;
  margin-left: auto;
  flex-shrink: 0;
}
.scope-status {
  color: var(--color-text-muted);
  font-size: 12px;
  white-space: nowrap;
}
.scope-action {
  min-width: 86px;
}

/* 基于 master 的表格样式 */
.table-wrapper {
  border: 1px solid var(--color-border);
  border-radius: var(--radius-md);
  overflow-y: auto;
  overflow-x: hidden;
  max-height: 280px;
}
.import-table {
  width: 100%;
  border-collapse: collapse;
  font-size: 12px;
}
.import-table th {
  position: sticky;
  top: 0;
  z-index: 2;
  background: var(--color-bg-hover);
  box-shadow: 0 1px 0 var(--color-border);
  padding: 8px 10px;
  text-align: left;
  font-weight: 600;
  color: var(--color-text-secondary);
}
.import-table td {
  padding: 8px 10px;
  border-bottom: 1px solid var(--color-border);
  color: var(--color-text);
}
.import-table tr:last-child td {
  border-bottom: none;
}
.cli-tag {
  font-weight: 600;
  text-transform: uppercase;
}
.scope-cell, .name-cell {
  max-width: 120px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.status-warn {
  color: #d97706;
  font-weight: 500;
}
.status-new {
  color: #10b981;
  font-weight: 500;
}
.action-cell {
  display: flex;
  align-items: center;
  gap: 6px;
}
.action-select-wrap {
  min-width: 95px;
}
.rename-input {
  width: 90px;
  height: 28px;
  padding: 0 8px;
  box-sizing: border-box;
  font-size: 11px;
  border: 1px solid var(--color-border);
  border-radius: var(--radius-md, 6px);
  background: var(--color-bg);
  color: var(--color-text);
}
.dialog-footer {
  display: flex;
  justify-content: flex-end;
  gap: var(--space-2);
  padding: var(--space-3) var(--space-4);
  border-top: 1px solid var(--color-border);
}
.btn-cancel {
  padding: var(--space-1) var(--space-3);
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
.btn-primary {
  display: flex;
  align-items: center;
  gap: var(--space-1);
  padding: var(--space-1) var(--space-3);
  font-size: var(--text-sm);
  color: white;
  background: var(--color-primary);
  border: none;
  border-radius: var(--radius-md);
  cursor: pointer;
  transition: opacity var(--transition-fast);
}
.btn-primary:hover {
  opacity: 0.9;
}
.table-wrapper::-webkit-scrollbar,
.dialog-body::-webkit-scrollbar,
.scope-section::-webkit-scrollbar {
  width: 6px;
  height: 6px;
}
.table-wrapper::-webkit-scrollbar-thumb,
.dialog-body::-webkit-scrollbar-thumb,
.scope-section::-webkit-scrollbar-thumb {
  background: var(--color-border);
  border-radius: 3px;
}
.table-wrapper::-webkit-scrollbar-thumb:hover,
.dialog-body::-webkit-scrollbar-thumb:hover,
.scope-section::-webkit-scrollbar-thumb:hover {
  background: var(--color-text-muted);
}
</style>

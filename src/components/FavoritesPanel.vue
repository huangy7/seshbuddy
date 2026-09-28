<script setup lang="ts">
import { computed } from "vue";
import SvgIcon from "./icons/SvgIcon.vue";
import { useFavorites } from "../composables/useFavorites";
import { t } from "../i18n";
import { isCliId } from "../types/cli";
import { sessionIdentityKey, type AggregatedProjectInfo, type SessionIdentity } from "../types/session";

interface FavoriteRow {
  /** 稳定渲染键；身份无效（未知 cli_id）时退化为原始 entry 串。 */
  key: string;
  identity: SessionIdentity | null;
  displayName: string;
  encodedDir: string;
  missing: boolean;
}

interface FavoriteGroup {
  groupKey: string;
  title: string;
  rows: FavoriteRow[];
}

const MISSING_GROUP_KEY = "__missing__";

const props = defineProps<{ projects: AggregatedProjectInfo[] }>();
const emit = defineEmits<{
  openSession: [identity: SessionIdentity, encodedDir: string];
}>();

const { favorites } = useFavorites();

// 星标条目按 cli_id + file_path 复合匹配 projects 内会话，取 display_name 与所在项目分组；
// 匹配不到（对话已删/未加载）归入"源对话缺失"组，灰态且不可点击。
const groups = computed<FavoriteGroup[]>(() => {
  const sessionIndex = new Map<string, { project: AggregatedProjectInfo; displayName: string }>();
  for (const project of props.projects) {
    for (const session of project.sessions) {
      if (!isCliId(session.cli_id)) continue;
      const key = sessionIdentityKey({ cliId: session.cli_id, filePath: session.file_path });
      sessionIndex.set(key, { project, displayName: session.display_name });
    }
  }

  const byProject = new Map<string, FavoriteGroup>();
  const missingRows: FavoriteRow[] = [];
  for (const entry of favorites.value) {
    const fallbackKey = `${entry.cli_id}:${entry.path}`;
    const identity: SessionIdentity | null = isCliId(entry.cli_id)
      ? { cliId: entry.cli_id, filePath: entry.path }
      : null;
    const hit = identity ? sessionIndex.get(sessionIdentityKey(identity)) : undefined;
    if (identity && hit) {
      const groupKey = hit.project.project_key;
      let group = byProject.get(groupKey);
      if (!group) {
        group = {
          groupKey,
          title: hit.project.original_path || t("session.unknownProject"),
          rows: [],
        };
        byProject.set(groupKey, group);
      }
      group.rows.push({
        key: sessionIdentityKey(identity),
        identity,
        displayName: hit.displayName,
        encodedDir: hit.project.encoded_dir ?? "",
        missing: false,
      });
    } else {
      missingRows.push({
        key: identity ? sessionIdentityKey(identity) : fallbackKey,
        identity,
        displayName: fileNameOf(entry.path),
        encodedDir: "",
        missing: true,
      });
    }
  }

  const result = Array.from(byProject.values());
  if (missingRows.length > 0) {
    result.push({ groupKey: MISSING_GROUP_KEY, title: t("session.favoritesPanel.missingGroup"), rows: missingRows });
  }
  return result;
});

function fileNameOf(path: string): string {
  const parts = path.replace(/\\/g, "/").split("/");
  return parts[parts.length - 1] || path;
}

function onRowClick(row: FavoriteRow) {
  if (row.missing || !row.identity) return;
  emit("openSession", row.identity, row.encodedDir);
}
</script>

<template>
  <div class="favorites-panel">
    <div class="favorites-header">
      <SvgIcon name="star" :size="14" class="header-star" />
      <span class="favorites-title">{{ t("session.favoritesPanel.title") }}</span>
      <span class="count-badge">{{ favorites.length }}</span>
    </div>

    <div v-if="favorites.length === 0" class="favorites-empty">
      <SvgIcon name="star" :size="24" />
      <span>{{ t("session.favoritesPanel.empty") }}</span>
      <span class="favorites-empty-hint">{{ t("session.favoritesPanel.emptyHint") }}</span>
    </div>

    <div v-else class="favorites-groups">
      <div v-for="group in groups" :key="group.groupKey" class="project-node">
        <div class="project-header" :class="{ 'group-missing': group.groupKey === MISSING_GROUP_KEY }">
          <SvgIcon :name="group.groupKey === MISSING_GROUP_KEY ? 'alert-circle' : 'folder'" :size="14" class="folder-icon" />
          <span class="project-path truncate" :title="group.title">{{ group.title }}</span>
          <span class="group-count">{{ group.rows.length }}</span>
        </div>
        <div class="session-list">
          <div
            v-for="row in group.rows"
            :key="row.key"
            class="session-item"
            :class="{ 'favorite-missing': row.missing }"
            @click="onRowClick(row)"
          >
            <SvgIcon name="star" :size="12" class="favorite-star" />
            <div class="session-content">
              <span class="session-text truncate" :title="row.displayName">{{ row.displayName }}</span>
            </div>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.favorites-panel {
  flex: 1;
  display: flex;
  flex-direction: column;
  overflow: hidden;
}
.favorites-header {
  display: flex;
  align-items: center;
  gap: var(--space-1);
  height: 35px;
  min-height: 35px;
  padding: 0 var(--space-3);
  font-size: var(--text-sm);
  font-weight: 600;
  color: var(--color-text);
  border-bottom: 1px solid var(--color-border);
  flex-shrink: 0;
}
.header-star {
  color: var(--color-warning);
  fill: currentColor;
}
.favorites-title {
  flex: 1;
}
.count-badge {
  min-width: 18px;
  height: 18px;
  padding: 1px 6px;
  border-radius: var(--radius-full);
  background: var(--color-surface-selected);
  border: 1px solid var(--color-border-light);
  color: var(--color-text-muted);
  font-size: var(--text-2xs);
  font-variant-numeric: tabular-nums;
  font-weight: 500;
  text-align: center;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  line-height: 1;
}
.favorites-empty {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: var(--space-2);
  padding: var(--space-6);
  color: var(--color-text-muted);
  font-size: var(--text-sm);
}
.favorites-empty-hint {
  font-size: var(--text-xs);
  color: var(--color-text-muted);
  text-align: center;
}
.favorites-groups {
  flex: 1;
  overflow-y: auto;
  padding: var(--space-1) 0;
}
/* 行样式复用 SessionTree 的会话行契约（project-header / session-item / session-content / session-text） */
.project-node {
  margin-bottom: 2px;
}
.project-header {
  display: flex;
  align-items: center;
  min-height: 30px;
  height: 30px;
  box-sizing: border-box;
  padding: 0 8px;
  margin: 1px 6px;
  border-radius: 7px;
  font-size: var(--text-sm);
  font-weight: 500;
  gap: var(--space-1);
  color: var(--color-text);
  transition: background var(--transition-fast);
}
.project-header:hover {
  background: var(--color-bg-hover);
}
.folder-icon {
  color: var(--color-text-secondary);
  flex-shrink: 0;
}
.project-path {
  min-width: 0;
  flex: 1;
}
.group-count {
  font-size: var(--text-2xs);
  font-variant-numeric: tabular-nums;
  font-weight: 500;
  color: var(--color-text-muted);
  background: var(--color-bg-secondary);
  border: 1px solid var(--color-border-light);
  border-radius: var(--radius-full);
  padding: 1px 6px;
  min-width: 18px;
  height: 18px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  margin-left: auto;
  line-height: 1;
}
.group-missing {
  color: var(--color-text-muted);
}
.session-list {
  padding-left: var(--space-4);
  overflow: hidden;
}
.session-item {
  display: flex;
  align-items: center;
  min-height: 30px;
  height: 30px;
  box-sizing: border-box;
  padding: 0 8px;
  margin: 1px 6px;
  cursor: pointer;
  border-radius: 7px;
  font-size: var(--text-xs);
  gap: var(--space-1);
  color: var(--color-text-secondary);
  transition: background var(--transition-fast), color var(--transition-fast);
}
.session-item:hover {
  background: var(--color-bg-hover);
  color: var(--color-text);
}
.favorite-star {
  color: var(--color-warning);
  fill: currentColor;
  flex-shrink: 0;
}
.session-content {
  display: flex;
  flex-direction: column;
  min-width: 0;
  flex: 1;
  gap: 1px;
}
.session-text {
  min-width: 0;
  font-weight: 500;
  color: inherit;
}
.favorite-missing {
  color: var(--color-text-muted);
  cursor: default;
}
.favorite-missing .favorite-star {
  color: var(--color-text-muted);
  fill: none;
}
.favorite-missing:hover {
  background: transparent;
}
.truncate {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
</style>

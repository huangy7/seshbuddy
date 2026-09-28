<script setup lang="ts">
import { onMounted } from "vue";
import SvgIcon from "./icons/SvgIcon.vue";
import { open } from "@tauri-apps/plugin-dialog";
import { useBlockedFolders } from "../composables/useBlockedFolders";
import { t } from "../i18n";

const { blockedFolders, blockFolder, unblockFolder, loadBlockedFolders } = useBlockedFolders();

onMounted(() => {
  void loadBlockedFolders()?.catch?.((err) => {
    console.error("loadBlockedFolders failed:", err);
  });
});

async function onAddFolder() {
  try {
    const selected = await open({
      directory: true,
      multiple: false,
      title: t("settings.blockedFolders.pickFolderTitle"),
    });
    if (selected && typeof selected === "string") {
      await blockFolder(selected);
    }
  } catch (err) {
    console.error("选择要屏蔽的文件夹失败:", err);
  }
}
</script>

<template>
  <div class="blocked-folders-settings">
    <div class="settings-header-row">
      <p class="desc-text">
        {{ t("settings.blockedFolders.desc") }}
      </p>
      <button type="button" class="add-folder-btn" @click="onAddFolder">
        <SvgIcon name="plus" :size="14" />
        <span>{{ t("settings.blockedFolders.addFolder") }}</span>
      </button>
    </div>

    <div v-if="blockedFolders.length === 0" class="empty-state">
      <SvgIcon name="folder" :size="32" class="empty-icon" />
      <p class="empty-text">{{ t("settings.blockedFolders.emptyTitle") }}</p>
      <p class="empty-subtext">{{ t("settings.blockedFolders.emptyHint") }}</p>
      <button type="button" class="empty-add-btn" @click="onAddFolder">
        <SvgIcon name="plus" :size="14" />
        <span>{{ t("settings.blockedFolders.emptyAction") }}</span>
      </button>
    </div>

    <div v-else class="blocked-folders-list">
      <div
        v-for="path in blockedFolders"
        :key="path"
        class="blocked-folder-item"
      >
        <div class="folder-info">
          <SvgIcon name="folder" :size="16" class="folder-icon" />
          <span class="folder-path" :title="path">{{ path }}</span>
        </div>
        <button
          type="button"
          class="unblock-btn"
          :title="t('settings.blockedFolders.unblock')"
          @click="unblockFolder(path)"
        >
          <SvgIcon name="trash-2" :size="14" />
          <span>{{ t("settings.blockedFolders.unblock") }}</span>
        </button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.blocked-folders-settings {
  padding: var(--space-4);
  display: flex;
  flex-direction: column;
  gap: var(--space-3);
}

.settings-header-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-3);
}

.desc-text {
  margin: 0;
  line-height: 1.5;
  font-size: var(--text-sm);
  color: var(--color-text-secondary);
}

.add-folder-btn {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  padding: 6px 12px;
  background: var(--color-primary);
  color: #fff;
  border: none;
  border-radius: var(--radius-md);
  font-size: var(--text-xs);
  font-weight: 500;
  cursor: pointer;
  white-space: nowrap;
  flex-shrink: 0;
  transition: opacity var(--transition-fast), transform var(--transition-fast);
}

.add-folder-btn:hover {
  opacity: 0.9;
}

.empty-add-btn {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  margin-top: var(--space-3);
  padding: 6px 14px;
  background: var(--color-bg-hover);
  color: var(--color-text);
  border: 1px solid var(--color-border);
  border-radius: var(--radius-md);
  font-size: var(--text-xs);
  cursor: pointer;
  transition: all var(--transition-fast);
}

.empty-add-btn:hover {
  border-color: var(--color-primary);
  color: var(--color-primary);
}

.empty-state {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  padding: var(--space-8) var(--space-4);
  text-align: center;
  background: var(--color-bg-secondary);
  border-radius: var(--radius-md);
  border: 1px dashed var(--color-border);
}

.empty-icon {
  color: var(--color-text-muted);
  margin-bottom: var(--space-2);
  opacity: 0.6;
}

.empty-text {
  margin: 0;
  font-size: var(--text-sm);
  font-weight: 500;
  color: var(--color-text);
}

.empty-subtext {
  margin: var(--space-1) 0 0 0;
  font-size: var(--text-xs);
  color: var(--color-text-muted);
}

.blocked-folders-list {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
}

.blocked-folder-item {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: var(--space-2) var(--space-3);
  background: var(--color-bg-secondary);
  border: 1px solid var(--color-border);
  border-radius: var(--radius-md);
  gap: var(--space-3);
}

.folder-info {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  min-width: 0;
  flex: 1;
}

.folder-icon {
  color: var(--color-text-muted);
  flex-shrink: 0;
}

.folder-path {
  font-size: var(--text-sm);
  font-family: var(--font-mono, monospace);
  color: var(--color-text);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.unblock-btn {
  display: inline-flex;
  align-items: center;
  gap: var(--space-1);
  padding: var(--space-1) var(--space-2);
  font-size: var(--text-xs);
  color: var(--color-text-secondary);
  background: var(--color-bg);
  border: 1px solid var(--color-border);
  border-radius: var(--radius-sm);
  cursor: pointer;
  white-space: nowrap;
  transition: all 0.15s ease;
  flex-shrink: 0;
}

.unblock-btn:hover {
  color: var(--color-danger, #ef4444);
  border-color: var(--color-danger, #ef4444);
  background: var(--color-danger-subtle, rgba(239, 68, 68, 0.08));
}
</style>

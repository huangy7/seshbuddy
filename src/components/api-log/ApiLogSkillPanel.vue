<script setup lang="ts">
import { computed, ref } from "vue";
import SvgIcon from "../icons/SvgIcon.vue";
import { t } from "../../i18n";
import { parseSkillInfo } from "../../utils/reqSkills";
import { copyToClipboard } from "../../utils/clipboard";

// 请求上下文中的 Skill 信息：可用清单 + 实际调用记录
const props = defineProps<{
  reqBody: string | null;
}>();

const emit = defineEmits<{
  copied: [text: string];
}>();

const info = computed(() => parseSkillInfo(props.reqBody));

const showAvailable = ref(true);
const skillSearch = ref("");
const expandedSkillNames = ref<Set<string>>(new Set());

function toggleSkill(name: string) {
  const next = new Set(expandedSkillNames.value);
  if (next.has(name)) {
    next.delete(name);
  } else {
    next.add(name);
  }
  expandedSkillNames.value = next;
}

function toggleAllSkills(expand: boolean) {
  if (expand) {
    expandedSkillNames.value = new Set(info.value.available.map((s) => s.name));
  } else {
    expandedSkillNames.value = new Set();
  }
}

const filteredAvailable = computed(() => {
  const q = skillSearch.value.trim().toLowerCase();
  if (!q) return info.value.available;
  return info.value.available.filter(
    (s) => s.name.toLowerCase().includes(q) || s.description.toLowerCase().includes(q),
  );
});

const copiedName = ref<string | null>(null);

async function copyText(text: string, name: string, label: string) {
  if (await copyToClipboard(text)) {
    copiedName.value = name;
    emit("copied", label);
    setTimeout(() => {
      if (copiedName.value === name) copiedName.value = null;
    }, 1500);
  }
}
</script>

<template>
  <div class="skill-panel">
    <!-- 已调用 Skills 区块 -->
    <div v-if="info.invoked.length > 0" class="skill-section">
      <div class="skill-section-header">
        <span class="section-icon invoked-icon">
          <SvgIcon name="sparkles" :size="13" />
        </span>
        <span class="section-label">{{ t('api-log.skillPanel.invokedTitle') }}</span>
        <span class="section-count">{{ info.invoked.length }}</span>
      </div>
      <div class="skill-chips">
        <div
          v-for="s in info.invoked"
          :key="s.name"
          class="skill-chip invoked"
          :title="t('api-log.skillPanel.invokedCountTitle', { count: s.count })"
          @click="toggleSkill(s.name)"
        >
          <SvgIcon name="sparkles" :size="11" class="chip-icon" />
          <span class="chip-name">{{ s.name }}</span>
          <em v-if="s.count > 1" class="chip-count">×{{ s.count }}</em>
          <button
            class="chip-copy-btn"
            type="button"
            :title="t('api-log.skillPanel.copySkillNameTitle')"
            @click.stop="copyText(s.name, `inv-${s.name}`, t('api-log.skillPanel.copiedSkillName'))"
          >
            <SvgIcon :name="copiedName === `inv-${s.name}` ? 'check' : 'copy'" :size="10" />
          </button>
        </div>
      </div>
    </div>

    <!-- 可用技能清单区块 (带搜索与可展开查看完整内容卡片) -->
    <div v-if="info.available.length > 0" class="skill-section">
      <div class="skill-available-header-bar">
        <div class="skill-section-header" @click="showAvailable = !showAvailable">
          <span class="section-icon available-icon">
            <SvgIcon name="wand" :size="13" />
          </span>
          <span class="section-label">{{ t('api-log.skillPanel.availableTitle') }}</span>
          <span class="section-count">{{ info.available.length }}</span>
          <SvgIcon :name="showAvailable ? 'chevron-down' : 'chevron-right'" :size="12" class="toggle-chevron" />
        </div>

        <div v-if="showAvailable" class="skill-header-actions">
          <div class="skill-search-wrap">
            <SvgIcon name="search" :size="11" class="search-icon" />
            <input
              v-model="skillSearch"
              class="skill-search-input"
              type="text"
              :placeholder="t('api-log.skillPanel.searchPlaceholder')"
            />
          </div>
          <button
            class="toggle-all-btn"
            type="button"
            @click="toggleAllSkills(expandedSkillNames.size < info.available.length)"
          >
            {{ expandedSkillNames.size === info.available.length ? t('api-log.messagesView.collapseAll') : t('api-log.messagesView.expandAll') }}
          </button>
        </div>
      </div>

      <Transition name="skill-expand">
        <div v-if="showAvailable" class="skill-available-container">
          <div
            v-for="s in filteredAvailable"
            :key="s.name"
            class="skill-card"
            :class="{ expanded: expandedSkillNames.has(s.name) }"
          >
            <div class="skill-card-head" @click="toggleSkill(s.name)">
              <SvgIcon :name="expandedSkillNames.has(s.name) ? 'chevron-down' : 'chevron-right'" :size="12" class="card-chevron" />
              <div class="skill-head-info">
                <span class="skill-name">{{ s.name }}</span>
                <span v-if="!expandedSkillNames.has(s.name)" class="skill-desc-summary" :title="s.description">
                  {{ s.description }}
                </span>
              </div>
              <div class="card-actions" @click.stop>
                <button
                  class="card-action-btn"
                  type="button"
                  :title="t('api-log.skillPanel.copySkillNameTitle')"
                  @click="copyText(s.name, `name-${s.name}`, t('api-log.skillPanel.copiedSkillName'))"
                >
                  <SvgIcon :name="copiedName === `name-${s.name}` ? 'check' : 'copy'" :size="11" />
                </button>
              </div>
            </div>

            <!-- 展开后展示完整说明与详情内容 -->
            <div v-if="expandedSkillNames.has(s.name)" class="skill-card-body">
              <div class="skill-full-desc-wrap">
                <div class="desc-label">{{ t('api-log.skillPanel.descLabel') }}</div>
                <div class="desc-content">{{ s.description }}</div>
              </div>
              <div class="skill-prompt-hint">
                <span class="hint-label">{{ t('api-log.skillPanel.promptHintLabel') }}</span>
                <code>/{{ s.name }}</code>
                <button
                  class="mini-copy-link"
                  type="button"
                  @click="copyText(`/${s.name}`, `cmd-${s.name}`, t('api-log.skillPanel.copiedCommand'))"
                >
                  {{ copiedName === `cmd-${s.name}` ? t('dialogs.common.copied') : t('api-log.skillPanel.copyCommand') }}
                </button>
              </div>
            </div>
          </div>

          <div v-if="filteredAvailable.length === 0" class="skill-empty-search">
            {{ t('api-log.skillPanel.noMatchingSkills') }}
          </div>
        </div>
      </Transition>
    </div>

    <!-- 没有任何 Skill 信息 -->
    <div v-if="info.invoked.length === 0 && info.available.length === 0" class="inspector-empty">
      <SvgIcon name="wand" :size="24" class="empty-icon" />
      <div class="empty-text">{{ t('api-log.skillPanel.empty') }}</div>
    </div>
  </div>
</template>

<style scoped>
.skill-panel {
  display: flex;
  flex-direction: column;
  gap: var(--space-3);
  font-size: var(--text-xs);
  padding: 10px 14px;
}

.skill-section {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.skill-section-header {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  font-weight: 600;
  font-size: 11.5px;
  color: var(--color-text);
  background: none;
  border: none;
  padding: 0;
  cursor: pointer;
  text-align: left;
  user-select: none;
}

.section-icon {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 20px;
  height: 20px;
  border-radius: var(--radius-sm, 4px);
  flex-shrink: 0;
}

.invoked-icon {
  background: color-mix(in srgb, var(--color-primary) 12%, transparent);
  color: var(--color-primary);
}

.available-icon {
  background: color-mix(in srgb, var(--color-text-muted) 12%, transparent);
  color: var(--color-text-muted);
}

.section-label {
  font-weight: 600;
}

.section-count {
  font-family: var(--font-mono);
  font-size: 9.5px;
  font-weight: 600;
  color: var(--color-text-muted);
  background: var(--color-bg-hover);
  padding: 0 5px;
  border-radius: 999px;
  line-height: 16px;
}

.toggle-chevron {
  color: var(--color-text-muted);
  margin-left: 2px;
}

/* ─── Invoked chips ─── */
.skill-chips {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
}

.skill-chip {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  padding: 3px 8px;
  font-family: var(--font-mono);
  font-size: 11px;
  border-radius: var(--radius-sm, 4px);
  border: 1px solid var(--color-border);
  color: var(--color-text);
  background: var(--color-bg);
  cursor: pointer;
  transition: all var(--transition-fast);
}

.skill-chip.invoked {
  border-color: color-mix(in srgb, var(--color-primary) 30%, transparent);
  color: var(--color-primary);
  background: color-mix(in srgb, var(--color-primary) 6%, transparent);
}

.skill-chip.invoked:hover {
  background: color-mix(in srgb, var(--color-primary) 12%, transparent);
  border-color: var(--color-primary);
}

.chip-name {
  font-weight: 600;
}

.chip-count {
  font-style: normal;
  font-size: 10px;
  opacity: 0.7;
  font-weight: 600;
}

.chip-copy-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  background: transparent;
  border: none;
  color: var(--color-text-muted);
  padding: 1px;
  border-radius: 2px;
  cursor: pointer;
}

.chip-copy-btn:hover {
  color: var(--color-primary);
}

/* ─── Available Header Bar ─── */
.skill-available-header-bar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
}

.skill-header-actions {
  display: flex;
  align-items: center;
  gap: 6px;
}

.skill-search-wrap {
  position: relative;
  display: flex;
  align-items: center;
}

.search-icon {
  position: absolute;
  left: 6px;
  color: var(--color-text-muted);
  pointer-events: none;
}

.skill-search-input {
  width: 130px;
  height: 22px;
  padding: 0 6px 0 22px;
  font-size: 10.5px;
  color: var(--color-text);
  background: var(--color-bg-sidebar);
  border: 1px solid var(--color-border);
  border-radius: var(--radius-sm, 4px);
  outline: none;
}

.skill-search-input:focus {
  border-color: var(--color-primary);
  background: var(--color-bg);
}

.toggle-all-btn {
  font-size: 10.5px;
  color: var(--color-primary);
  background: var(--color-bg-sidebar);
  border: 1px solid var(--color-border);
  border-radius: var(--radius-sm, 4px);
  padding: 2px 7px;
  cursor: pointer;
  transition: all var(--transition-fast);
}

.toggle-all-btn:hover {
  border-color: var(--color-primary);
}

/* ─── Available Skills Container ─── */
.skill-available-container {
  display: flex;
  flex-direction: column;
  gap: 5px;
}

.skill-card {
  border: 1px solid var(--color-border);
  border-radius: var(--radius-md);
  background: var(--color-bg);
  overflow: hidden;
  transition: all var(--transition-fast);
}

.skill-card:hover {
  border-color: color-mix(in srgb, var(--color-primary) 30%, var(--color-border));
}

.skill-card.expanded {
  border-color: color-mix(in srgb, var(--color-primary) 35%, var(--color-border));
  box-shadow: 0 1px 3px rgba(0, 0, 0, 0.03);
}

.skill-card-head {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 7px 10px;
  background: var(--color-bg-sidebar);
  cursor: pointer;
  user-select: none;
}

.card-chevron {
  color: var(--color-text-muted);
  flex-shrink: 0;
}

.skill-head-info {
  display: flex;
  align-items: center;
  gap: 8px;
  flex: 1;
  min-width: 0;
}

.skill-name {
  font-family: var(--font-mono);
  font-weight: 700;
  color: var(--color-text);
  font-size: 11.5px;
  flex-shrink: 0;
}

.skill-desc-summary {
  font-size: 11px;
  color: var(--color-text-muted);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  flex: 1;
  min-width: 0;
}

.card-actions {
  display: flex;
  align-items: center;
  gap: 4px;
}

.card-action-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 20px;
  height: 20px;
  border: none;
  background: transparent;
  color: var(--color-text-muted);
  border-radius: var(--radius-sm, 4px);
  cursor: pointer;
}

.card-action-btn:hover {
  color: var(--color-text);
  background: var(--color-bg-hover);
}

.skill-card-body {
  padding: 10px 12px;
  border-top: 1px solid var(--color-border);
  display: flex;
  flex-direction: column;
  gap: 8px;
  background: var(--color-bg);
}

.skill-full-desc-wrap {
  display: flex;
  flex-direction: column;
  gap: 3px;
}

.desc-label {
  font-size: 10px;
  font-weight: 600;
  color: var(--color-text-muted);
  text-transform: uppercase;
  letter-spacing: 0.05em;
}

.desc-content {
  font-size: 11.5px;
  line-height: 1.6;
  color: var(--color-text);
  white-space: pre-wrap;
  word-break: break-word;
}

.skill-prompt-hint {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 5px 8px;
  background: var(--color-bg-sidebar);
  border-radius: var(--radius-sm, 4px);
  border: 1px solid var(--color-border);
  font-size: 10.5px;
}

.hint-label {
  color: var(--color-text-muted);
}

.skill-prompt-hint code {
  font-family: var(--font-mono);
  font-weight: 600;
  color: var(--color-primary);
  background: color-mix(in srgb, var(--color-primary) 8%, transparent);
  padding: 1px 5px;
  border-radius: 3px;
}

.mini-copy-link {
  margin-left: auto;
  font-size: 10.5px;
  color: var(--color-primary);
  background: transparent;
  border: none;
  cursor: pointer;
}

.mini-copy-link:hover {
  text-decoration: underline;
}

.skill-empty-search {
  text-align: center;
  padding: 16px;
  color: var(--color-text-muted);
  font-size: 11px;
}

.inspector-empty {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 8px;
  padding: 32px 16px;
  color: var(--color-text-muted);
  text-align: center;
}

.empty-icon {
  opacity: 0.4;
}

.empty-text {
  font-size: 11.5px;
}
</style>

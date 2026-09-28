<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted } from "vue";
import { invokeApp } from "../../utils/invokeApp";
import { ask } from "@tauri-apps/plugin-dialog";
import { t } from "../../i18n";
import { expandPhraseVariables, PHRASE_VARIABLE_KEYS } from "./phrase-variables";

interface QuickPhrase {
  id: string;
  name: string;
  content: string;
}

const props = defineProps<{ disabled?: boolean }>();
// pick = 填入输入框（不直接发送，变量展开所见即所得）
const emit = defineEmits<{ pick: [prompt: string] }>();

const phrases = ref<QuickPhrase[]>([]);
const open = ref(false);
const rootRef = ref<HTMLElement | null>(null);

// 提示语里的变量名按当前语言列出，与 phrase-variables.ts 的展开清单同源
const variableHint = computed(() =>
  PHRASE_VARIABLE_KEYS.map((key) => `{{${t(`assistant.phraseVariables.${key}`)}}}`).join(" ")
);

// 编辑态：null = 列表视图；{ id: null } = 新建；{ id } = 编辑
const editing = ref<{ id: string | null; name: string; content: string } | null>(null);
const saving = ref(false);

// 点击组件外部时收起面板
function onDocClick(e: MouseEvent) {
  if (open.value && rootRef.value && !rootRef.value.contains(e.target as Node)) {
    open.value = false;
    editing.value = null;
  }
}

async function refresh() {
  phrases.value = await invokeApp<QuickPhrase[]>("assistant_list_quick_phrases");
}

onMounted(async () => {
  document.addEventListener("click", onDocClick, true);
  try {
    await refresh();
  } catch (err) {
    console.error("加载快捷用语失败:", err);
  }
});

onUnmounted(() => document.removeEventListener("click", onDocClick, true));

/** 填入前展开短语中的日期变量（{{今天}} / {{本周一}} 等），展开规则见 phrase-variables.ts */
async function expand(content: string): Promise<string> {
  return expandPhraseVariables(content);
}

async function pickPhrase(p: QuickPhrase) {
  if (props.disabled) return;
  emit("pick", await expand(p.content));
  open.value = false;
}

function toggle() {
  if (props.disabled) return;
  open.value = !open.value;
  editing.value = null;
}

function startCreate() {
  editing.value = { id: null, name: "", content: "" };
}

function startEdit(e: MouseEvent, p: QuickPhrase) {
  e.stopPropagation();
  editing.value = { id: p.id, name: p.name, content: p.content };
}

async function save() {
  const e = editing.value;
  if (!e || saving.value) return;
  saving.value = true;
  try {
    await invokeApp("assistant_save_quick_phrase", {
      id: e.id,
      name: e.name,
      content: e.content,
    });
    await refresh();
    editing.value = null;
  } catch (err) {
    console.error("保存快捷用语失败:", err);
  } finally {
    saving.value = false;
  }
}

async function remove(e: MouseEvent, p: QuickPhrase) {
  e.stopPropagation();
  // 用 Tauri 异步对话框：WebView 的 window.confirm 不阻塞
  const ok = await ask(t("assistant.quickPhrases.deleteConfirm", { name: p.name }), {
    title: t("assistant.history.deleteTitle"),
    kind: "warning",
    okLabel: t("app.batch.delete"),
    cancelLabel: t("dialogs.common.cancel"),
  });
  if (!ok) return;
  try {
    await invokeApp("assistant_delete_quick_phrase", { id: p.id });
    await refresh();
  } catch (err) {
    console.error("删除快捷用语失败:", err);
  }
}
</script>

<template>
  <div class="quick-phrases" ref="rootRef">
    <button
      class="qp-trigger"
      :class="{ open }"
      :title="t('assistant.input.quickPhrases')"
      :disabled="disabled"
      @click="toggle"
    >
      <span class="qp-spark">✦</span>
    </button>

    <div v-if="open && !disabled" class="qp-panel">
      <!-- 列表视图 -->
      <template v-if="!editing">
        <div class="qp-section">{{ t("assistant.input.quickPhrases") }}</div>
        <div v-if="phrases.length === 0" class="qp-empty">{{ t("assistant.quickPhrases.empty") }}</div>
        <div
          v-for="p in phrases"
          :key="p.id"
          class="qp-item"
          role="button"
          @click="pickPhrase(p)"
        >
          <span class="qp-name">{{ p.name }}</span>
          <span class="qp-actions">
            <span class="qp-action" :title="t('assistant.quickPhrases.edit')" @click="startEdit($event, p)">✎</span>
            <span class="qp-action danger" :title="t('app.batch.delete')" @click="remove($event, p)">×</span>
          </span>
        </div>
        <div class="qp-divider" />
        <button class="qp-create" @click="startCreate">{{ t("assistant.quickPhrases.create") }}</button>
      </template>

      <!-- 编辑/新建表单 -->
      <template v-else>
        <div class="qp-section">{{ editing.id ? t("assistant.quickPhrases.editTitle") : t("assistant.quickPhrases.createTitle") }}</div>
        <input
          v-model="editing.name"
          class="qp-input"
          :placeholder="t('assistant.quickPhrases.namePlaceholder')"
          maxlength="30"
        />
        <textarea
          v-model="editing.content"
          class="qp-textarea"
          rows="6"
          :placeholder="t('assistant.quickPhrases.contentPlaceholder', { variables: variableHint })"
        />
        <div class="qp-form-bar">
          <button class="qp-form-btn" @click="editing = null">{{ t("dialogs.common.cancel") }}</button>
          <button
            class="qp-form-btn primary"
            :disabled="saving || !editing.name.trim() || !editing.content.trim()"
            @click="save"
          >{{ t("app.dialog.save") }}</button>
        </div>
      </template>
    </div>
  </div>
</template>

<style scoped>
.quick-phrases {
  position: relative;
}
.qp-trigger {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 30px;
  height: 30px;
  border-radius: var(--radius-full);
  border: none;
  background: transparent;
  color: var(--color-text-muted);
  font-size: 15px;
  cursor: pointer;
  transition: color var(--transition-fast), background var(--transition-fast);
}
.qp-trigger:hover,
.qp-trigger.open {
  color: var(--color-primary);
  background: var(--color-primary-light);
}
.qp-trigger:disabled {
  opacity: 0.5;
  cursor: default;
}
.qp-spark {
  display: inline-block;
  transition: transform var(--transition-base);
}
.qp-trigger:hover .qp-spark {
  transform: rotate(45deg) scale(1.15);
}
.qp-panel {
  position: absolute;
  bottom: 38px;
  left: 0;
  width: 260px;
  background: var(--color-bg-secondary);
  border: 1px solid var(--color-border);
  border-radius: var(--radius-xl);
  box-shadow: var(--shadow-lg);
  padding: var(--space-2);
  z-index: var(--z-dropdown);
  animation: panel-in 0.15s ease-out;
}
@keyframes panel-in {
  from { opacity: 0; transform: translateY(4px); }
  to { opacity: 1; transform: translateY(0); }
}
.qp-section {
  padding: 2px 8px 6px;
  font-size: var(--text-2xs);
  color: var(--color-text-muted);
  letter-spacing: 0.05em;
}
.qp-empty {
  padding: 6px 8px;
  font-size: var(--text-xs);
  color: var(--color-text-muted);
}
.qp-item {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  width: 100%;
  padding: 7px 8px;
  border-radius: var(--radius-md);
  color: var(--color-text);
  font-size: var(--text-sm);
  cursor: pointer;
  transition: background var(--transition-fast);
}
.qp-item:hover {
  background: var(--color-bg-hover);
}
.qp-name {
  flex: 1;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.qp-actions {
  display: none;
  gap: 2px;
  flex-shrink: 0;
}
.qp-item:hover .qp-actions {
  display: flex;
}
.qp-action {
  width: 20px;
  height: 20px;
  line-height: 20px;
  text-align: center;
  border-radius: var(--radius-sm);
  color: var(--color-text-muted);
  font-size: 12px;
}
.qp-action:hover {
  background: var(--color-bg-active);
  color: var(--color-text);
}
.qp-action.danger:hover {
  background: var(--color-danger);
  color: var(--color-text-inverse);
}
.qp-divider {
  height: 1px;
  background: var(--color-border);
  margin: var(--space-2) 4px;
}
.qp-create {
  width: 100%;
  padding: 6px 8px;
  border: none;
  border-radius: var(--radius-md);
  background: transparent;
  color: var(--color-text-muted);
  font-size: var(--text-xs);
  cursor: pointer;
  text-align: left;
}
.qp-create:hover {
  background: var(--color-bg-hover);
  color: var(--color-text);
}
.qp-input,
.qp-textarea {
  width: 100%;
  border: 1px solid var(--color-border);
  border-radius: var(--radius-md);
  background: var(--color-bg);
  color: var(--color-text);
  font-size: var(--text-sm);
  padding: 6px 8px;
  margin-bottom: var(--space-2);
  font-family: inherit;
  box-sizing: border-box;
}
.qp-textarea {
  resize: vertical;
  line-height: var(--leading-normal);
}
.qp-input:focus,
.qp-textarea:focus {
  outline: none;
  border-color: var(--color-primary);
}
.qp-form-bar {
  display: flex;
  justify-content: flex-end;
  gap: var(--space-2);
}
.qp-form-btn {
  padding: 4px 14px;
  border: 1px solid var(--color-border);
  border-radius: var(--radius-md);
  background: transparent;
  color: var(--color-text-secondary);
  font-size: var(--text-xs);
  cursor: pointer;
}
.qp-form-btn.primary {
  background: var(--color-primary);
  border-color: var(--color-primary);
  color: var(--color-text-inverse);
}
.qp-form-btn.primary:disabled {
  opacity: 0.5;
  cursor: default;
}
</style>

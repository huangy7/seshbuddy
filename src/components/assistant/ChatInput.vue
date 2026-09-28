<script setup lang="ts">
import { ref, computed, watch, nextTick } from "vue";
import QuickPhrasesButton from "./QuickPhrasesButton.vue";
import ModelPicker from "./ModelPicker.vue";
import { formatTokenCount } from "../../utils/format";
import { invokeApp } from "../../utils/invokeApp";
import { t } from "../../i18n";
import { expandPhraseVariables } from "./phrase-variables";

interface QuickPhrase {
  id: string;
  name: string;
  content: string;
}

const props = defineProps<{
  disabled: boolean;
  running: boolean;
  profile?: string;
  usage?: { outputTotal: number } | null;
}>();
const emit = defineEmits<{ send: [text: string]; cancel: [] }>();
const selectedModel = defineModel<string>("model", { default: "" });
const draft = ref("");
const textareaRef = ref<HTMLTextAreaElement | null>(null);

const MAX_HEIGHT = 160; // 约 8 行，超出后内部滚动

const phrases = ref<QuickPhrase[]>([]);
const slashOpen = ref(false);
const selectedIndex = ref(0);
const slashQuery = ref("");

async function loadPhrases() {
  try {
    phrases.value = await invokeApp<QuickPhrase[]>("assistant_list_quick_phrases");
  } catch (err) {
    console.error("加载快捷用语失败:", err);
  }
}

async function expandPhrase(content: string): Promise<string> {
  return expandPhraseVariables(content);
}

const matchedPhrases = computed(() => {
  const q = slashQuery.value.trim().toLowerCase();
  if (!q) return phrases.value;
  return phrases.value.filter(
    (p) => p.name.toLowerCase().includes(q) || p.content.toLowerCase().includes(q)
  );
});

function checkSlashCommand() {
  const text = draft.value;
  const match = text.match(/(?:^|\s)\/([^\s]*)$/);
  if (match && !props.disabled) {
    slashQuery.value = match[1];
    if (!slashOpen.value) {
      slashOpen.value = true;
      selectedIndex.value = 0;
      void loadPhrases();
    }
  } else {
    slashOpen.value = false;
  }
}

watch(draft, () => {
  nextTick(resize);
  checkSlashCommand();
});

watch(matchedPhrases, () => {
  if (selectedIndex.value >= matchedPhrases.value.length) {
    selectedIndex.value = Math.max(0, matchedPhrases.value.length - 1);
  }
});

function resize() {
  const el = textareaRef.value;
  if (!el) return;
  el.style.height = "auto";
  el.style.height = `${Math.min(el.scrollHeight, MAX_HEIGHT)}px`;
}

function submit() {
  if (!draft.value.trim() || props.disabled) return;
  emit("send", draft.value);
  draft.value = "";
}

/** 最近一次输入法合成结束的时间戳（0 = 从未合成过）。 */
let lastCompositionEndAt = 0;
/** 部分输入法提交合成后才派发 Enter keydown（isComposing 已是 false），
 *  紧跟 compositionend 的 Enter 应视为"确认输入法"而非发送。 */
function onCompositionEnd() {
  lastCompositionEndAt = Date.now();
}

async function selectPhrase(p: QuickPhrase) {
  const text = draft.value;
  const match = text.match(/(?:^|\s)\/([^\s]*)$/);
  const expanded = await expandPhrase(p.content);
  if (match && match.index !== undefined) {
    const prefix = text.slice(0, match.index + (match[0].startsWith(" ") ? 1 : 0));
    draft.value = prefix + expanded;
  } else {
    draft.value = expanded;
  }
  slashOpen.value = false;
  await nextTick();
  textareaRef.value?.focus();
}

function onKeydown(e: KeyboardEvent) {
  // 输入法合成中：Enter 用于确认候选/提交，不发送（keyCode 229 兜底 Windows IME）
  if (e.isComposing || e.keyCode === 229) return;

  // 部分 IME 提交合成后才派发 Enter keydown（isComposing 已 false）：
  // 紧跟 compositionend 的 Enter 视为"确认输入法"，抑制发送，不进入任何发送/选择分支
  // （业界通行的 IME 时间窗守卫，窗口 ~100ms）
  if (
    e.key === "Enter" &&
    !e.shiftKey &&
    lastCompositionEndAt !== 0 &&
    Date.now() - lastCompositionEndAt < 100
  ) {
    return;
  }

  if (slashOpen.value && matchedPhrases.value.length > 0) {
    if (e.key === "ArrowDown") {
      e.preventDefault();
      selectedIndex.value = (selectedIndex.value + 1) % matchedPhrases.value.length;
      return;
    }
    if (e.key === "ArrowUp") {
      e.preventDefault();
      selectedIndex.value = (selectedIndex.value - 1 + matchedPhrases.value.length) % matchedPhrases.value.length;
      return;
    }
    if (e.key === "Enter" || e.key === "Tab") {
      e.preventDefault();
      const target = matchedPhrases.value[selectedIndex.value];
      if (target) void selectPhrase(target);
      return;
    }
    if (e.key === "Escape") {
      e.preventDefault();
      slashOpen.value = false;
      return;
    }
  }

  if (e.key === "Enter" && !e.shiftKey) {
    e.preventDefault();
    submit();
  }
}

/** 快捷用语：填入输入框（不直接发送，用户可改可发） */
function onPickPhrase(prompt: string) {
  if (props.disabled) return;
  draft.value = prompt;
}

function setDraft(text: string) {
  draft.value = text;
  nextTick(() => {
    resize();
    textareaRef.value?.focus();
  });
}
defineExpose({ setDraft });
</script>

<template>
  <div class="composer-wrap">
    <!-- Slash 快捷指令选择面板 -->
    <div v-if="slashOpen && !disabled" class="slash-menu">
      <div class="slash-header">
        <span class="slash-icon">/</span>
        <span class="slash-title">{{ t("assistant.input.quickPhrases") }}</span>
        <span class="slash-tip">{{ t("assistant.input.slashTip") }}</span>
      </div>
      <div v-if="matchedPhrases.length === 0" class="slash-empty">
        {{ t("assistant.input.noPhraseMatch") }}
      </div>
      <div
        v-for="(p, index) in matchedPhrases"
        :key="p.id"
        class="slash-item"
        :class="{ active: index === selectedIndex }"
        @mousedown.prevent="selectPhrase(p)"
        @mouseenter="selectedIndex = index"
      >
        <span class="slash-badge">/{{ p.name }}</span>
        <span class="slash-content">{{ p.content }}</span>
      </div>
    </div>

    <div class="composer" :class="{ disabled }">
      <textarea
        ref="textareaRef"
        v-model="draft"
        :disabled="disabled"
        rows="2"
        :placeholder="t('assistant.input.placeholder')"
        autocomplete="off"
        autocapitalize="off"
        autocorrect="off"
        spellcheck="false"
        @keydown="onKeydown"
        @compositionend="onCompositionEnd"
      />
      <div class="composer-bar">
        <QuickPhrasesButton :disabled="disabled" @pick="onPickPhrase" />
        <ModelPicker v-if="props.profile" v-model="selectedModel" :profile="props.profile" />
        <span
          v-if="usage"
          class="usage-badge"
          :title="t('assistant.input.usageTooltip')"
        ><span class="ub-label">{{ t("assistant.input.cumulative") }}</span><span class="ub-dir">↓</span>{{ formatTokenCount(usage.outputTotal) }}</span>
        <div class="bar-spacer" />
        <button
          v-if="running"
          class="action-btn cancel"
          :title="t('assistant.input.stopTitle')"
          @click="emit('cancel')"
        >■</button>
        <button
          v-else
          class="action-btn send"
          :title="t('session.dashboard.send')"
          :disabled="disabled || !draft.trim()"
          @click="submit"
        >↑</button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.composer-wrap {
  position: relative;
  padding: var(--space-2) var(--space-3) var(--space-3);
  flex-shrink: 0;
}
.slash-menu {
  position: absolute;
  bottom: calc(100% - 4px);
  left: var(--space-3);
  right: var(--space-3);
  max-height: 220px;
  overflow-y: auto;
  background: var(--color-bg-secondary);
  border: 1px solid var(--color-border);
  border-radius: var(--radius-xl);
  box-shadow: var(--shadow-lg);
  padding: var(--space-1);
  z-index: var(--z-dropdown);
  animation: slash-in 0.15s ease-out;
}
@keyframes slash-in {
  from { opacity: 0; transform: translateY(4px); }
  to { opacity: 1; transform: translateY(0); }
}
.slash-header {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 4px 8px 6px;
  font-size: var(--text-2xs);
  color: var(--color-text-muted);
  border-bottom: 1px solid var(--color-border-light);
  margin-bottom: 4px;
}
.slash-icon {
  font-family: var(--font-mono);
  font-weight: bold;
  color: var(--color-primary);
}
.slash-title {
  font-weight: 600;
  color: var(--color-text-secondary);
}
.slash-tip {
  margin-left: auto;
  font-size: 10px;
  opacity: 0.75;
}
.slash-empty {
  padding: 8px 10px;
  font-size: var(--text-xs);
  color: var(--color-text-muted);
}
.slash-item {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 6px 10px;
  border-radius: var(--radius-md);
  cursor: pointer;
  transition: background var(--transition-fast);
}
.slash-item.active {
  background: var(--color-bg-hover);
}
.slash-badge {
  font-family: var(--font-mono);
  font-size: var(--text-xs);
  font-weight: 600;
  color: var(--color-primary);
  background: var(--color-primary-light);
  padding: 1px 6px;
  border-radius: var(--radius-sm);
  flex-shrink: 0;
}
.slash-content {
  font-size: var(--text-xs);
  color: var(--color-text-muted);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  flex: 1;
}
.composer {
  display: flex;
  flex-direction: column;
  border: 1px solid var(--color-border, #dce3ee);
  border-radius: 16px;
  background: var(--color-bg, #fff);
  padding: 8px 10px 8px 12px;
  box-shadow: 0 4px 16px rgba(36, 52, 80, 0.03);
  transition: border-color var(--transition-fast), box-shadow var(--transition-fast);
}
.composer:focus-within {
  border-color: var(--color-primary);
  box-shadow: 0 0 0 3px var(--color-primary-ring), 0 4px 16px rgba(36, 52, 80, 0.06);
}
.composer.disabled {
  opacity: 0.6;
}
textarea {
  resize: none;
  border: none;
  outline: none;
  background: transparent;
  color: var(--color-text);
  font-size: 13.5px;
  font-family: inherit;
  line-height: var(--leading-normal);
  padding: 4px 4px 8px;
  max-height: 160px;
  overflow-y: auto;
}
textarea::placeholder {
  color: var(--color-text-muted, #9aa5b5);
}
.composer-bar {
  display: flex;
  align-items: center;
  gap: var(--space-1);
  padding: 0 2px;
}
.bar-spacer {
  flex: 1;
}
.usage-badge {
  display: inline-flex;
  align-items: center;
  gap: 3px;
  padding: 1px 8px;
  border-radius: var(--radius-full);
  background: var(--color-bg-hover);
  font-size: 10px;
  font-family: var(--font-mono);
  line-height: 1.5;
  color: var(--color-text-secondary);
  user-select: none;
}
.ub-label {
  font-family: var(--font-sans);
  font-size: 9px;
  color: var(--color-text-muted);
  margin-right: 1px;
}
.ub-dir {
  font-size: 9px;
  color: var(--color-role-assistant);
}
.action-btn {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 28px;
  height: 28px;
  border-radius: 8px;
  border: none;
  font-size: 13px;
  font-weight: 700;
  cursor: pointer;
  transition: background var(--transition-fast), opacity var(--transition-fast), transform var(--transition-fast), box-shadow var(--transition-fast);
}
.action-btn.send {
  background: var(--color-text, #172033);
  color: var(--color-bg, #fff);
}
.action-btn.send:not(:disabled):hover {
  transform: scale(1.04);
  box-shadow: 0 2px 8px rgba(23, 32, 51, 0.2);
}
.action-btn.send:not(:disabled):active {
  transform: scale(0.96);
}
.action-btn.send:disabled {
  opacity: 0.35;
  cursor: default;
}
.action-btn.cancel {
  background: var(--color-bg-active);
  color: var(--color-text-secondary);
  font-size: 10px;
}
.action-btn.cancel:hover {
  color: var(--color-danger);
}
</style>

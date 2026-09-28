<script setup lang="ts">
import { ref, watch, nextTick, computed } from "vue";
import SvgIcon from "./icons/SvgIcon.vue";
import { t } from "../i18n";

const props = defineProps<{
  currentIndex: number;
  totalMatches: number;
  caseSensitive: boolean;
  wholeWord: boolean;
  toolbar?: boolean;
}>();

const emit = defineEmits<{
  close: [];
  search: [query: string];
  next: [];
  prev: [];
  toggleCaseSensitive: [];
  toggleWholeWord: [];
}>();

const query = ref("");
const inputRef = ref<HTMLInputElement | null>(null);

watch(query, (q) => {
  emit("search", q);
});

function goNext() {
  if (props.totalMatches === 0) return;
  emit("next");
}

function goPrev() {
  if (props.totalMatches === 0) return;
  emit("prev");
}

function refocusInput() {
  nextTick(() => inputRef.value?.focus());
}

function toggleCaseSensitive() {
  emit("toggleCaseSensitive");
  refocusInput();
}

function toggleWholeWord() {
  emit("toggleWholeWord");
  refocusInput();
}

function onKeyDown(e: KeyboardEvent) {
  if (e.isComposing) return;

  if (e.altKey && !e.ctrlKey && !e.metaKey) {
    const key = e.key.toLowerCase();
    if (key === "c") {
      e.preventDefault();
      toggleCaseSensitive();
      return;
    }
    if (key === "w") {
      e.preventDefault();
      toggleWholeWord();
      return;
    }
  }

  if (e.key === "Escape") {
    e.preventDefault();
    e.stopPropagation();
    close();
  } else if (e.key === "Enter") {
    e.preventDefault();
    e.stopPropagation();
    if (e.shiftKey) {
      goPrev();
    } else {
      goNext();
    }
  }
}

function close() {
  query.value = "";
  emit("close");
}

function focusInput() {
  nextTick(() => inputRef.value?.focus());
}

function setQuery(nextQuery: string) {
  const normalized = nextQuery ?? "";
  const same = query.value === normalized;
  query.value = normalized;
  refocusInput();
  if (same) {
    nextTick(() => emit("search", normalized));
  }
}

defineExpose({ focusInput, setQuery });

const statusText = computed(() => {
  if (!query.value.trim()) return "";
  if (props.totalMatches === 0) return t("common.chatSearchBar.noMatches");
  return `${props.currentIndex + 1} / ${props.totalMatches}`;
});
</script>

<template>
  <div class="chat-search-bar" :class="{ toolbar }" @keydown="onKeyDown">
    <div class="search-field">
      <SvgIcon name="search" :size="14" class="search-icon" />
      <input
        ref="inputRef"
        v-model="query"
        class="search-input"
        :placeholder="t('common.chatSearchBar.placeholder')"
        autocomplete="off"
        autocapitalize="off"
        autocorrect="off"
        spellcheck="false"
      />
    </div>
    <div class="search-options" v-if="query.trim()">
      <button
        type="button"
        class="mode-btn"
        :class="{ active: caseSensitive }"
        :aria-pressed="caseSensitive"
        :title="t('common.chatSearchBar.caseSensitive')"
        @click="toggleCaseSensitive"
      >
        Aa
      </button>
      <button
        type="button"
        class="mode-btn"
        :class="{ active: wholeWord }"
        :aria-pressed="wholeWord"
        :title="t('common.chatSearchBar.wholeWord')"
        @click="toggleWholeWord"
      >
        W
      </button>
    </div>
    <span v-if="query.trim()" class="match-status">{{ statusText }}</span>
    <button class="nav-btn" type="button" :title="t('common.chatSearchBar.previous')" @click="goPrev" :disabled="totalMatches === 0">
      <SvgIcon name="chevron-right" :size="14" style="transform: rotate(-90deg)" />
    </button>
    <button class="nav-btn" type="button" :title="t('common.chatSearchBar.next')" @click="goNext" :disabled="totalMatches === 0">
      <SvgIcon name="chevron-right" :size="14" style="transform: rotate(90deg)" />
    </button>
    <button class="nav-btn" type="button" :title="t('common.chatSearchBar.close')" @click="close">
      <SvgIcon name="x" :size="14" />
    </button>
  </div>
</template>

<style scoped>
.chat-search-bar {
  -webkit-user-select: none;
  user-select: none;
  display: flex;
  align-items: center;
  gap: var(--space-2);
  padding: var(--space-2) var(--space-3);
  background: var(--color-bg);
  border-bottom: 1px solid var(--color-border);
  flex-shrink: 0;
}
.chat-search-bar.toolbar {
  height: 32px;
  width: 100%;
  padding: 0;
  border-bottom: 0;
  background: transparent;
  min-width: 0;
}
.search-field {
  flex: 1;
  display: flex;
  align-items: center;
  gap: var(--space-2);
  min-width: 0;
  height: 28px;
  padding: 0 var(--space-2);
  background: var(--color-bg-secondary);
  border: 1px solid var(--color-border);
  border-radius: var(--radius-md);
  transition: border-color var(--transition-fast), box-shadow var(--transition-fast);
}
.search-field:focus-within {
  border-color: var(--color-primary);
  box-shadow: 0 0 0 2px var(--color-primary-ring);
}
.search-icon {
  color: var(--color-text-muted);
  flex-shrink: 0;
}
.search-input {
  flex: 1;
  border: none;
  background: none;
  outline: none;
  font-size: var(--text-sm);
  color: var(--color-text);
  font-family: inherit;
  min-width: 0;
}
.search-input::placeholder {
  color: var(--color-text-muted);
}
.search-input:focus-visible {
  outline: none;
}
.search-options {
  display: flex;
  align-items: center;
  gap: var(--space-1);
  flex-shrink: 0;
}
.mode-btn {
  min-width: 28px;
  height: 24px;
  padding: 0 8px;
  border-radius: var(--radius-sm);
  border: 1px solid var(--color-border);
  background: var(--color-bg-secondary);
  color: var(--color-text-secondary);
  font-size: 11px;
  font-weight: 700;
  line-height: 1;
  transition: all var(--transition-fast);
}
.mode-btn:hover {
  border-color: var(--color-primary);
  color: var(--color-text);
}
.mode-btn.active {
  border-color: color-mix(in srgb, var(--color-primary) 55%, var(--color-border));
  background: color-mix(in srgb, var(--color-primary) 16%, var(--color-bg));
  color: var(--color-primary);
}
.match-status {
  font-size: var(--text-xs);
  color: var(--color-text-muted);
  white-space: nowrap;
}
.nav-btn {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 24px;
  height: 24px;
  border-radius: var(--radius-sm);
  color: var(--color-text-secondary);
  transition: all var(--transition-fast);
}
.nav-btn:hover:not(:disabled) {
  background: var(--color-bg-hover);
  color: var(--color-text);
}
.nav-btn:disabled {
  opacity: 0.3;
  cursor: default;
}
</style>

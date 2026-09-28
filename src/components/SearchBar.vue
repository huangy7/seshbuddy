<script setup lang="ts">
import { computed } from "vue";
import SvgIcon from "./icons/SvgIcon.vue";
import { t } from "../i18n";

defineProps<{ modelValue: string; placeholder?: string }>();
const emit = defineEmits<{
  "update:modelValue": [value: string];
  "openSearch": [];
}>();

const isMac = typeof navigator !== "undefined" && navigator.platform.toUpperCase().includes("MAC");
const shortcutBadge = isMac ? "⌘⇧F" : "Ctrl+Shift+F";
// computed 而非常量：常量在组件创建时求值一次，之后切换语言标题不会跟随
const shortcutTitle = computed(() =>
  isMac ? t("common.searchBar.shortcutMac") : t("common.searchBar.shortcutOther")
);

function clear() {
  emit("update:modelValue", "");
}

function onClick() {
  emit("openSearch");
}
</script>

<template>
  <div class="search-bar" :title="shortcutTitle" @click="onClick">
    <div class="search-wrapper">
      <SvgIcon name="search" :size="14" class="search-icon" />
      <input
        type="text"
        :value="modelValue"
        @input="emit('update:modelValue', ($event.target as HTMLInputElement).value)"
        :placeholder="placeholder || t('common.searchBar.placeholder')"
        class="search-input"
        :class="{ 'has-badge': !modelValue }"
        :aria-label="placeholder || t('common.searchBar.placeholder')"
        autocomplete="off"
        autocapitalize="off"
        autocorrect="off"
        spellcheck="false"
        @click.stop="onClick"
      />
      <span v-if="!modelValue" class="search-shortcut-badge" aria-hidden="true">{{ shortcutBadge }}</span>
      <button v-if="modelValue" class="clear-btn" @click.stop="clear" :aria-label="t('common.searchBar.clear')">
        <SvgIcon name="x" :size="12" />
      </button>
    </div>
  </div>
</template>

<style scoped>
.search-bar {
  padding: var(--space-2);
  flex-shrink: 0;
  cursor: pointer;
}
.search-wrapper {
  position: relative;
  display: flex;
  align-items: center;
}
.search-icon {
  position: absolute;
  left: 10px;
  color: var(--color-text-muted);
  pointer-events: none;
}
.search-input {
  width: 100%;
  box-sizing: border-box;
  padding: 6px 30px 6px 30px;
  border: 1px solid transparent;
  border-radius: var(--radius-full);
  font-size: var(--text-sm);
  font-family: var(--font-sans);
  outline: none;
  background: var(--color-bg-hover);
  color: var(--color-text);
  transition: all var(--transition-base);
  cursor: pointer;
}
.search-input.has-badge {
  padding-right: 68px;
}
.search-input::placeholder {
  color: var(--color-text-muted);
}
.search-input:focus {
  border-color: var(--color-primary);
  background: var(--color-bg);
  box-shadow: 0 0 0 3px var(--color-primary-ring);
}
.search-shortcut-badge {
  position: absolute;
  right: 8px;
  font-size: 10px;
  line-height: 1.2;
  padding: 1px 5px;
  border-radius: var(--radius-xs, 3px);
  background: var(--color-bg);
  color: var(--color-text-muted);
  border: 1px solid var(--color-border-light);
  pointer-events: none;
  user-select: none;
  opacity: 0.85;
}
.clear-btn {
  position: absolute;
  right: 6px;
  display: flex;
  align-items: center;
  justify-content: center;
  width: 20px;
  height: 20px;
  border-radius: var(--radius-full);
  color: var(--color-text-muted);
  cursor: pointer;
  transition: all var(--transition-fast);
}
.clear-btn:hover {
  background: var(--color-bg-active);
  color: var(--color-text);
}
</style>

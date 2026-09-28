<script setup lang="ts">
import { ref, computed, watch, onBeforeUnmount, nextTick } from "vue";
import SvgIcon from "../icons/SvgIcon.vue";
import ChatAvatar from "../chat/ChatAvatar.vue";
import { t } from "../../i18n";

export type OptionValue = string | number | boolean | null | undefined;

export interface SelectOption {
  value: OptionValue;
  label: string;
  icon?: string;
  cliId?: string;
  description?: string;
  isGroupHeader?: boolean;
  disabled?: boolean;
}

const props = withDefaults(
  defineProps<{
    modelValue?: OptionValue;
    options: (SelectOption | string)[];
    placeholder?: string;
    icon?: string;
    disabled?: boolean;
    emptyHint?: string;
    size?: "small" | "default" | "compact";
    fitTriggerWidth?: boolean;
    minMenuWidth?: string;
    maxMenuWidth?: string;
  }>(),
  {
    modelValue: undefined,
    // 默认值留空，回退到模板里求值：prop 默认值只在模块加载时算一次，切换语言不会跟着变
    placeholder: undefined,
    icon: undefined,
    disabled: false,
    emptyHint: undefined,
    size: "default",
    fitTriggerWidth: true,
    minMenuWidth: undefined,
    maxMenuWidth: undefined,
  }
);

const emit = defineEmits<{
  "update:modelValue": [value: OptionValue];
  "change": [value: OptionValue];
}>();

const normalizedOptions = computed<SelectOption[]>(() => {
  return props.options.map((opt) => {
    if (typeof opt === "string") {
      return { value: opt, label: opt };
    }
    return opt;
  });
});

const selectedOption = computed(() => {
  return normalizedOptions.value.find((opt) => opt.value === props.modelValue);
});

const isOpen = ref(false);
const selectRef = ref<HTMLElement | null>(null);
const menuRef = ref<HTMLElement | null>(null);
const scrollAreaRef = ref<HTMLElement | null>(null);
const placement = ref<"bottom" | "top">("bottom");
const highlightedIndex = ref<number>(-1);

const menuStyle = ref<{
  top?: string;
  left?: string;
  minWidth?: string;
  width?: string;
  maxWidth?: string;
  maxHeight?: string;
  transform?: string;
}>({ minWidth: "0px", width: "max-content", maxWidth: "360px" });

function toggleDropdown() {
  if (props.disabled) return;
  if (isOpen.value) {
    closeDropdown();
  } else {
    openDropdown();
  }
}

function openDropdown() {
  if (props.disabled) return;
  isOpen.value = true;
  const selIdx = normalizedOptions.value.findIndex((opt) => opt.value === props.modelValue && !opt.isGroupHeader);
  highlightedIndex.value = selIdx >= 0 ? selIdx : getFirstSelectableIndex();
}

function closeDropdown() {
  isOpen.value = false;
  highlightedIndex.value = -1;
}

function selectOption(value: OptionValue) {
  emit("update:modelValue", value);
  emit("change", value);
  closeDropdown();
  selectRef.value?.focus();
}

function getFirstSelectableIndex(): number {
  return normalizedOptions.value.findIndex((opt) => !opt.isGroupHeader && !opt.disabled);
}

function getLastSelectableIndex(): number {
  for (let i = normalizedOptions.value.length - 1; i >= 0; i--) {
    if (!normalizedOptions.value[i].isGroupHeader && !normalizedOptions.value[i].disabled) {
      return i;
    }
  }
  return -1;
}

function getNextSelectableIndex(current: number, direction: 1 | -1): number {
  const count = normalizedOptions.value.length;
  if (count === 0) return -1;
  let idx = current + direction;
  while (idx >= 0 && idx < count) {
    const opt = normalizedOptions.value[idx];
    if (!opt.isGroupHeader && !opt.disabled) {
      return idx;
    }
    idx += direction;
  }
  return current;
}

function scrollToHighlighted() {
  nextTick(() => {
    if (!scrollAreaRef.value) return;
    const items = scrollAreaRef.value.querySelectorAll(".elegant-option");
    const target = items[highlightedIndex.value] as HTMLElement | undefined;
    if (target) {
      const container = scrollAreaRef.value;
      const top = target.offsetTop;
      const bottom = top + target.offsetHeight;
      if (top < container.scrollTop) {
        container.scrollTop = top;
      } else if (bottom > container.scrollTop + container.clientHeight) {
        container.scrollTop = bottom - container.clientHeight;
      }
    }
  });
}

// Type-ahead buffer
let typeAheadQuery = "";
let typeAheadTimer: ReturnType<typeof setTimeout> | null = null;

function handleKeyDown(e: KeyboardEvent) {
  if (props.disabled) return;

  if (e.key === "Tab") {
    if (isOpen.value) closeDropdown();
    return;
  }

  if (e.key === "Escape") {
    if (isOpen.value) {
      e.preventDefault();
      closeDropdown();
      selectRef.value?.focus();
    }
    return;
  }

  if (e.key === "Enter" || e.key === " ") {
    e.preventDefault();
    if (!isOpen.value) {
      openDropdown();
    } else {
      if (highlightedIndex.value >= 0 && highlightedIndex.value < normalizedOptions.value.length) {
        const opt = normalizedOptions.value[highlightedIndex.value];
        if (!opt.isGroupHeader && !opt.disabled) {
          selectOption(opt.value);
        }
      }
    }
    return;
  }

  if (e.key === "ArrowDown") {
    e.preventDefault();
    if (!isOpen.value) {
      openDropdown();
    } else {
      highlightedIndex.value = getNextSelectableIndex(highlightedIndex.value, 1);
      scrollToHighlighted();
    }
    return;
  }

  if (e.key === "ArrowUp") {
    e.preventDefault();
    if (!isOpen.value) {
      openDropdown();
    } else {
      highlightedIndex.value = getNextSelectableIndex(highlightedIndex.value, -1);
      scrollToHighlighted();
    }
    return;
  }

  if (e.key === "Home") {
    if (isOpen.value) {
      e.preventDefault();
      highlightedIndex.value = getFirstSelectableIndex();
      scrollToHighlighted();
    }
    return;
  }

  if (e.key === "End") {
    if (isOpen.value) {
      e.preventDefault();
      highlightedIndex.value = getLastSelectableIndex();
      scrollToHighlighted();
    }
    return;
  }

  // Type ahead search
  if (e.key.length === 1 && !e.ctrlKey && !e.altKey && !e.metaKey) {
    if (!isOpen.value) openDropdown();
    if (typeAheadTimer) clearTimeout(typeAheadTimer);
    typeAheadQuery += e.key.toLowerCase();
    typeAheadTimer = setTimeout(() => {
      typeAheadQuery = "";
    }, 600);

    const matchIdx = normalizedOptions.value.findIndex((opt) => {
      if (opt.isGroupHeader || opt.disabled) return false;
      return opt.label.toLowerCase().startsWith(typeAheadQuery);
    });
    if (matchIdx >= 0) {
      highlightedIndex.value = matchIdx;
      scrollToHighlighted();
    }
  }
}

const updatePosition = () => {
  if (isOpen.value && selectRef.value) {
    const rect = selectRef.value.getBoundingClientRect();
    const viewportHeight = window.innerHeight;
    const viewportWidth = window.innerWidth;

    const spaceBelow = viewportHeight - rect.bottom;
    const spaceAbove = rect.top;

    // Flip upwards if below is constrained (< 220px) and above has more room
    const isTop = spaceBelow < 220 && spaceAbove > spaceBelow;
    placement.value = isTop ? "top" : "bottom";

    const baseMin = typeof props.minMenuWidth === "number"
      ? props.minMenuWidth
      : typeof props.minMenuWidth === "string"
        ? parseInt(props.minMenuWidth, 10) || 200
        : (props.fitTriggerWidth ? rect.width : Math.max(rect.width, 180));

    const estimatedMinWidth = Math.max(rect.width, baseMin);
    const isRight = rect.left + estimatedMinWidth > viewportWidth - 16;

    const calculatedMaxHeight = isTop
      ? Math.min(280, Math.max(120, spaceAbove - 16))
      : Math.min(280, Math.max(120, spaceBelow - 16));

    const maxW = typeof props.maxMenuWidth === "number"
      ? `${props.maxMenuWidth}px`
      : props.maxMenuWidth || "360px";

    const styleObj: Record<string, string> = {
      minWidth: `${props.fitTriggerWidth ? Math.max(rect.width, 150) : Math.max(rect.width, baseMin)}px`,
      width: props.fitTriggerWidth ? `${Math.max(rect.width, 150)}px` : "max-content",
      maxWidth: maxW,
      maxHeight: `${calculatedMaxHeight}px`,
    };

    if (isTop) {
      styleObj.top = `${rect.top + window.scrollY - 4}px`;
      styleObj.transform = isRight ? "translate(-100%, -100%)" : "translateY(-100%)";
    } else {
      styleObj.top = `${rect.bottom + window.scrollY + 4}px`;
      styleObj.transform = isRight ? "translateX(-100%)" : "none";
    }

    if (isRight) {
      styleObj.left = `${rect.right + window.scrollX}px`;
    } else {
      styleObj.left = `${rect.left + window.scrollX}px`;
    }

    menuStyle.value = styleObj;
  }
};

function handleDocumentPointerDown(e: PointerEvent) {
  const target = e.target as Node | null;
  if (!target) return;
  if (selectRef.value?.contains(target)) return;
  if (menuRef.value?.contains(target)) return;
  closeDropdown();
}

watch(isOpen, async (val) => {
  if (val) {
    await nextTick();
    updatePosition();
    window.addEventListener("scroll", updatePosition, { capture: true, passive: true });
    window.addEventListener("resize", updatePosition, { passive: true });
    window.addEventListener("pointerdown", handleDocumentPointerDown, true);
    scrollToHighlighted();
  } else {
    window.removeEventListener("scroll", updatePosition, true);
    window.removeEventListener("resize", updatePosition);
    window.removeEventListener("pointerdown", handleDocumentPointerDown, true);
  }
});

onBeforeUnmount(() => {
  window.removeEventListener("scroll", updatePosition, true);
  window.removeEventListener("resize", updatePosition);
  window.removeEventListener("pointerdown", handleDocumentPointerDown, true);
});
</script>

<template>
  <div 
    class="elegant-select" 
    :class="['size-' + size, { 'is-disabled': disabled, 'is-open': isOpen }]"
    tabindex="0" 
    role="combobox"
    :aria-expanded="isOpen"
    aria-haspopup="listbox"
    @keydown="handleKeyDown"
    ref="selectRef"
  >
    <slot name="trigger" :selected="selectedOption" :isOpen="isOpen" :toggle="toggleDropdown">
      <div 
        class="elegant-select-trigger" 
        :class="{ 'is-open': isOpen }" 
        @click="toggleDropdown"
      >
        <div class="trigger-content" v-if="selectedOption">
          <ChatAvatar v-if="selectedOption.cliId" role="assistant" :cli-id="String(selectedOption.cliId)" class="trigger-avatar" />
          <SvgIcon v-else-if="selectedOption.icon || icon" :name="selectedOption.icon || icon!" :size="size === 'small' ? 14 : 16" class="trigger-icon" />
          <div class="trigger-text" :title="String(selectedOption.label || '')">
            <div class="t-name">{{ selectedOption.label }}</div>
          </div>
        </div>
        <div class="trigger-content empty" v-else>
          <SvgIcon v-if="icon" :name="icon" :size="size === 'small' ? 14 : 16" class="trigger-icon" />
          <span class="placeholder-text">{{ placeholder ?? t("common.select.placeholder") }}</span>
        </div>
        <SvgIcon name="chevron-down" :size="size === 'small' ? 13 : 15" class="trigger-arrow" />
      </div>
    </slot>
    
    <Teleport to="body">
      <Transition :name="placement === 'top' ? 'dropdown-fade-up' : 'dropdown-fade-down'">
        <div 
          class="elegant-select-menu" 
          v-if="isOpen" 
          :class="['placement-' + placement, 'menu-size-' + size]"
          :style="menuStyle" 
          @mousedown.prevent 
          @click.stop
          ref="menuRef"
          role="listbox"
        >
          <div class="menu-scroll-area" ref="scrollAreaRef">
            <div 
              v-for="(opt, idx) in normalizedOptions" 
              :key="String(opt.value ?? idx)"
              class="elegant-option"
              :class="{ 
                selected: modelValue === opt.value && !opt.isGroupHeader,
                highlighted: highlightedIndex === idx && !opt.isGroupHeader,
                'is-group-header': opt.isGroupHeader,
                'is-option-disabled': opt.disabled,
                'has-desc': !!opt.description
              }"
              :role="opt.isGroupHeader ? 'presentation' : 'option'"
              :aria-selected="modelValue === opt.value"
              :title="opt.description ? `${opt.label} (${opt.description})` : opt.label"
              @mouseenter="!opt.isGroupHeader && !opt.disabled && (highlightedIndex = idx)"
              @click="!opt.isGroupHeader && !opt.disabled && selectOption(opt.value)"
            >
              <slot name="option" :option="opt" :selected="modelValue === opt.value">
                <ChatAvatar v-if="opt.cliId && !opt.isGroupHeader" role="assistant" :cli-id="String(opt.cliId)" class="opt-avatar" />
                <SvgIcon v-else-if="(opt.icon || icon) && !opt.isGroupHeader" :name="opt.icon || icon!" :size="size === 'small' ? 13 : 14" class="opt-icon" />
                <div class="opt-text">
                  <div class="opt-row">
                    <span class="o-name">{{ opt.label }}</span>
                    <span v-if="opt.description" class="o-desc-badge">{{ opt.description }}</span>
                  </div>
                </div>
                <SvgIcon v-if="modelValue === opt.value && !opt.isGroupHeader" name="check" :size="size === 'small' ? 13 : 14" class="opt-check" />
              </slot>
            </div>
            <div v-if="normalizedOptions.length === 0" class="empty-hint">{{ emptyHint ?? t("common.select.empty") }}</div>
          </div>
          <slot name="footer"></slot>
        </div>
      </Transition>
    </Teleport>
  </div>
</template>

<style scoped>
.elegant-select {
  position: relative;
  width: 100%;
  outline: none;
  display: inline-block;
  vertical-align: middle;
}
.elegant-select:focus-visible .elegant-select-trigger {
  border-color: var(--color-primary);
  box-shadow: 0 0 0 2px var(--color-primary-ring);
}
.elegant-select.is-disabled {
  opacity: 0.55;
  pointer-events: none;
  cursor: not-allowed;
}

.elegant-select-trigger {
  display: flex;
  align-items: center;
  justify-content: space-between;
  width: 100%;
  padding: 0 var(--space-3);
  background: var(--color-bg);
  border: 1px solid var(--color-border);
  border-radius: var(--radius-md);
  cursor: pointer;
  transition: all 0.18s cubic-bezier(0.16, 1, 0.3, 1);
  box-shadow: 0 1px 2px rgba(0, 0, 0, 0.03);
  height: 34px;
  box-sizing: border-box;
  user-select: none;
}
.elegant-select-trigger:hover {
  border-color: var(--color-primary);
  background: var(--color-bg-hover, var(--color-bg));
  box-shadow: 0 2px 5px rgba(0, 0, 0, 0.05);
}
.elegant-select-trigger.is-open {
  border-color: var(--color-primary);
  box-shadow: 0 0 0 2px var(--color-primary-ring);
  background: var(--color-bg);
}

/* Small size variant */
.elegant-select.size-small .elegant-select-trigger {
  height: 28px;
  padding: 0 8px;
  border-radius: var(--radius-md, 6px);
  font-size: var(--text-xs, 12px);
}
.elegant-select.size-small .t-name {
  font-size: 12px;
  font-weight: 500;
}
.elegant-select.size-small .trigger-arrow {
  margin-left: 4px;
}

/* Compact size variant */
.elegant-select.size-compact .elegant-select-trigger {
  height: 24px;
  padding: 0 6px;
  border-radius: var(--radius-sm, 4px);
  font-size: 11px;
}
.elegant-select.size-compact .t-name {
  font-size: 11px;
  font-weight: 500;
}

.trigger-content {
  display: flex;
  align-items: center;
  gap: 6px;
  flex: 1;
  min-width: 0;
}
.trigger-content.empty {
  color: var(--color-text-muted);
  font-size: var(--text-sm, 13px);
}
.elegant-select.size-small .trigger-content.empty {
  font-size: 12px;
}
.trigger-avatar {
  width: 16px;
  height: 16px;
  flex-shrink: 0;
}
.size-small .trigger-avatar {
  width: 14px;
  height: 14px;
}
.trigger-icon {
  color: var(--color-text-secondary);
  flex-shrink: 0;
  transition: color 0.15s ease;
}
.elegant-select-trigger:hover .trigger-icon,
.elegant-select-trigger.is-open .trigger-icon {
  color: var(--color-primary);
}

.trigger-text {
  display: flex;
  flex-direction: column;
  flex: 1;
  min-width: 0;
}
.t-name {
  font-size: var(--text-sm, 13px);
  font-weight: 500;
  color: var(--color-text);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  text-align: left;
}
.placeholder-text {
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  text-align: left;
}
.trigger-arrow {
  color: var(--color-text-muted);
  transition: transform 0.2s cubic-bezier(0.16, 1, 0.3, 1), color 0.15s ease;
  flex-shrink: 0;
  margin-left: 6px;
}
.elegant-select-trigger:hover .trigger-arrow,
.elegant-select-trigger.is-open .trigger-arrow {
  color: var(--color-text);
}
.elegant-select-trigger.is-open .trigger-arrow {
  transform: rotate(180deg);
}

.elegant-select-menu {
  position: absolute;
  top: calc(100% + 4px);
  left: 0;
  width: 100%;
  background: var(--color-bg-elevated, var(--color-bg));
  border: 1px solid var(--color-border);
  border-radius: var(--radius-lg, 8px);
  box-shadow: 0 10px 30px rgba(0, 0, 0, 0.12), 0 2px 6px rgba(0, 0, 0, 0.06);
  z-index: 10050;
  display: flex;
  flex-direction: column;
  overflow: hidden;
  backdrop-filter: blur(16px);
  -webkit-backdrop-filter: blur(16px);
}

.menu-scroll-area {
  max-height: 240px;
  overflow-y: auto;
  padding: 4px;
}

.menu-scroll-area::-webkit-scrollbar {
  width: 5px;
}
.menu-scroll-area::-webkit-scrollbar-track {
  background: transparent;
}
.menu-scroll-area::-webkit-scrollbar-thumb {
  background: var(--color-border);
  border-radius: 3px;
}
.menu-scroll-area::-webkit-scrollbar-thumb:hover {
  background: var(--color-text-muted);
}

.elegant-option {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 6px 10px;
  border-radius: var(--radius-sm, 4px);
  cursor: pointer;
  transition: background 0.12s ease, color 0.12s ease;
  user-select: none;
}
.menu-size-small .elegant-option {
  padding: 5px 8px;
  gap: 6px;
}
.elegant-option:hover,
.elegant-option.highlighted {
  background: var(--color-bg-hover);
}
.elegant-option.selected {
  background: var(--color-primary-light);
}
.elegant-option.is-option-disabled {
  opacity: 0.45;
  cursor: not-allowed;
  pointer-events: none;
}

.elegant-option.is-group-header {
  cursor: default;
  pointer-events: none;
  background: transparent !important;
  padding: 8px 10px 4px;
  margin-top: 4px;
}
.elegant-option.is-group-header:first-child {
  margin-top: 0;
  padding-top: 4px;
}
.elegant-option.is-group-header .o-name {
  font-size: 11px;
  font-weight: 700;
  text-transform: uppercase;
  letter-spacing: 0.6px;
  color: var(--color-text-muted);
}

.opt-avatar {
  width: 16px;
  height: 16px;
  flex-shrink: 0;
}
.menu-size-small .opt-avatar {
  width: 14px;
  height: 14px;
}

.opt-icon {
  color: var(--color-text-muted);
  flex-shrink: 0;
  transition: color 0.12s ease;
}
.elegant-option.selected .opt-icon,
.elegant-option:hover .opt-icon,
.elegant-option.highlighted .opt-icon {
  color: var(--color-primary);
}

.opt-text {
  display: flex;
  flex-direction: column;
  flex: 1;
  min-width: 0;
}
.opt-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  width: 100%;
}
.o-name {
  font-size: var(--text-sm, 13px);
  font-weight: 500;
  color: var(--color-text);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  text-align: left;
}
.menu-size-small .o-name {
  font-size: 12px;
}
.elegant-option.selected .o-name {
  color: var(--color-primary);
  font-weight: 600;
}
.o-desc-badge {
  font-size: 10px;
  font-weight: 500;
  padding: 1px 6px;
  border-radius: 4px;
  background: var(--color-bg-secondary);
  border: 1px solid var(--color-border);
  color: var(--color-text-muted);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  max-width: 160px;
  flex-shrink: 0;
}
.elegant-option.selected .o-desc-badge {
  border-color: var(--color-primary-ring);
  color: var(--color-primary);
  background: var(--color-primary-light);
}
.opt-check {
  color: var(--color-primary);
  flex-shrink: 0;
}

.empty-hint {
  padding: var(--space-4);
  text-align: center;
  font-size: var(--text-xs, 12px);
  color: var(--color-text-muted);
}

.dropdown-fade-down-enter-active,
.dropdown-fade-down-leave-active {
  transition: opacity 0.16s ease, transform 0.16s cubic-bezier(0.16, 1, 0.3, 1);
  transform-origin: top;
}
.dropdown-fade-down-enter-from,
.dropdown-fade-down-leave-to {
  opacity: 0;
  transform: translateY(-4px) scaleY(0.96);
}

.dropdown-fade-up-enter-active,
.dropdown-fade-up-leave-active {
  transition: opacity 0.16s ease, transform 0.16s cubic-bezier(0.16, 1, 0.3, 1);
  transform-origin: bottom;
}
.dropdown-fade-up-enter-from,
.dropdown-fade-up-leave-to {
  opacity: 0;
  transform: translateY(4px) scaleY(0.96);
}
</style>

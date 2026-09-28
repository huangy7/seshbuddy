<script setup lang="ts">
import { ref, computed, watch, onUnmounted } from "vue";
import SvgIcon from "./icons/SvgIcon.vue";
import { t } from "../i18n";

const props = defineProps<{
  container: HTMLElement | null;
  unreadCount?: number;
}>();

const emit = defineEmits<{
  scrollToEnd: [];
  scrollToTop: [];
}>();

const distanceFromBottom = ref(0);
const distanceFromTop = ref(0);
const scrollDirection = ref<"down" | "up">("down");
let lastScrollTop = 0;

function updateScrollMetrics() {
  const el = props.container;
  if (!el) return;

  const scrollTop = el.scrollTop;
  distanceFromBottom.value = el.scrollHeight - scrollTop - el.clientHeight;
  distanceFromTop.value = scrollTop;

  if (scrollTop > lastScrollTop) {
    scrollDirection.value = "down";
  } else if (scrollTop < lastScrollTop) {
    scrollDirection.value = "up";
  }
  lastScrollTop = scrollTop;
}

const hasUnread = computed(() => (props.unreadCount ?? 0) > 0);

const currentDirection = computed<"down" | "up">({
  get() {
    if (hasUnread.value) return "down";
    if (distanceFromBottom.value < 50) return "up";
    if (distanceFromTop.value < 50) return "down";
    return scrollDirection.value;
  },
  set(val) {
    scrollDirection.value = val;
  },
});

const visible = computed(() => {
  if (hasUnread.value) return true;
  return distanceFromTop.value > 200 || distanceFromBottom.value > 200;
});

function onScroll() {
  updateScrollMetrics();
}

function handleClick() {
  const el = props.container;
  if (!el) return;
  if (currentDirection.value === "down") {
    emit("scrollToEnd");
  } else {
    emit("scrollToTop");
  }
}

watch(
  () => props.container,
  (el, oldEl) => {
    oldEl?.removeEventListener("scroll", onScroll);
    el?.addEventListener("scroll", onScroll, { passive: true });
    if (el) updateScrollMetrics();
  },
  { immediate: true },
);

onUnmounted(() => {
  props.container?.removeEventListener("scroll", onScroll);
});
</script>

<template>
  <Transition name="fade-up">
    <button
      v-if="visible"
      class="scroll-btn"
      :class="{ 'has-unread': hasUnread }"
      :title="hasUnread ? t('common.scrollToBottom.titleUnread') : (currentDirection === 'down' ? t('common.scrollToBottom.titleScrollToEnd') : t('common.scrollToBottom.titleScrollToTop'))"
      @click="handleClick"
    >
      <SvgIcon name="arrow-down" :size="16" :class="{ flipped: currentDirection === 'up' && !hasUnread }" />
      <span v-if="hasUnread" class="unread-text">
        {{ (props.unreadCount ?? 0) > 99 ? t('common.scrollToBottom.newMessagesCapped') : t('common.scrollToBottom.newMessages', { count: props.unreadCount ?? 0 }) }}
      </span>
    </button>
  </Transition>
</template>

<style scoped>
.scroll-btn {
  position: absolute;
  bottom: 20px;
  right: 20px;
  min-width: 36px;
  height: 36px;
  padding: 0;
  border-radius: var(--radius-full);
  background: var(--color-bg);
  color: var(--color-text-secondary);
  border: 1px solid var(--color-border);
  box-shadow: var(--shadow-md);
  display: flex;
  align-items: center;
  justify-content: center;
  cursor: pointer;
  transition: all var(--transition-base);
  z-index: var(--z-sticky);
  user-select: none;
}
.scroll-btn:hover {
  background: var(--color-bg-hover);
  color: var(--color-text);
  box-shadow: var(--shadow-lg);
}
.scroll-btn.has-unread {
  width: auto;
  padding: 0 14px 0 10px;
  gap: 6px;
  background: var(--color-primary);
  color: #fff;
  border-color: transparent;
  box-shadow: 0 4px 14px color-mix(in srgb, var(--color-primary) 40%, transparent);
}
.scroll-btn.has-unread:hover {
  background: color-mix(in srgb, var(--color-primary) 88%, #000);
  color: #fff;
  box-shadow: 0 6px 18px color-mix(in srgb, var(--color-primary) 50%, transparent);
}
.unread-text {
  font-size: 13px;
  font-weight: 500;
  white-space: nowrap;
}
.flipped {
  transform: rotate(180deg);
}
.fade-up-enter-active,
.fade-up-leave-active {
  transition: opacity var(--transition-base), transform var(--transition-base);
}
.fade-up-enter-from,
.fade-up-leave-to {
  opacity: 0;
  transform: translateY(8px);
}
</style>

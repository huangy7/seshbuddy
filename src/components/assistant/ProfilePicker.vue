<script setup lang="ts">
import { ref, onMounted, onUnmounted } from "vue";
import { invokeApp } from "../../utils/invokeApp";
import { t } from "../../i18n";

const model = defineModel<string>({ default: "" });
// locked = 对话进行中：profile 在对话创建时绑定（--resume 续聊），中途不可切换
const props = defineProps<{ locked?: boolean }>();
const profiles = ref<string[]>([]);
const open = ref(false);
const rootRef = ref<HTMLElement | null>(null);

// 记住上次选择：ChatGPT 式零摩擦，打开即上次 profile
const STORAGE_KEY = "assistant.lastProfile";

// 点击组件外部时收起下拉
function onDocClick(e: MouseEvent) {
  if (open.value && rootRef.value && !rootRef.value.contains(e.target as Node)) {
    open.value = false;
  }
}

onMounted(async () => {
  document.addEventListener("click", onDocClick, true);
  try {
    profiles.value = await invokeApp<string[]>("list_profiles", { cliId: "claude", scope: null });
    const saved = localStorage.getItem(STORAGE_KEY);
    if (saved && profiles.value.includes(saved)) {
      model.value = saved;
    } else if (!model.value && profiles.value.length > 0) {
      // 优先使用 API 配置中的活跃 profile，fallback 到列表第一个
      try {
        const active = await invokeApp<string>("get_active_profile", { cliId: "claude", scope: null });
        model.value = active && profiles.value.includes(active) ? active : profiles.value[0];
      } catch {
        model.value = profiles.value[0];
      }
    }
  } catch (err) {
    console.error("加载模型配置失败:", err);
  }
});

onUnmounted(() => document.removeEventListener("click", onDocClick, true));

function select(p: string) {
  model.value = p;
  localStorage.setItem(STORAGE_KEY, p);
  open.value = false;
}

function toggle() {
  if (props.locked) return;
  open.value = !open.value;
}
</script>

<template>
  <div class="profile-chip-wrap" ref="rootRef">
    <button
      class="profile-chip"
      :class="{ locked: props.locked }"
      :title="props.locked ? t('assistant.profilePicker.lockedTitle') : t('assistant.profilePicker.switchTitle')"
      @click="toggle"
    >
      <svg v-if="props.locked" class="chip-lock" viewBox="0 0 16 16" width="10" height="10" fill="none" stroke="currentColor" stroke-width="1.6">
        <rect x="3" y="7" width="10" height="7" rx="1.5" />
        <path d="M5.5 7V5a2.5 2.5 0 0 1 5 0v2" />
      </svg>
      <span v-else class="chip-dot" />
      <span class="chip-name">{{ model || t("assistant.profilePicker.select") }}</span>
      <span v-if="!props.locked" class="chip-caret" :class="{ open }">▾</span>
    </button>
    <div v-if="open && !props.locked" class="profile-menu">
      <button
        v-for="p in profiles"
        :key="p"
        class="profile-option"
        :class="{ current: model === p }"
        @click="select(p)"
      >
        <span class="option-check">{{ model === p ? "✓" : "" }}</span>{{ p }}
      </button>
      <div v-if="profiles.length === 0" class="profile-empty">{{ t("assistant.profilePicker.empty") }}</div>
    </div>
  </div>
</template>

<style scoped>
.profile-chip-wrap {
  position: relative;
}
.profile-chip {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 3px 10px;
  border: 1px solid var(--color-border);
  border-radius: var(--radius-full);
  background: transparent;
  color: var(--color-text-secondary);
  font-size: var(--text-xs);
  cursor: pointer;
  max-width: 200px;
  transition: border-color var(--transition-fast), color var(--transition-fast);
}
.profile-chip:hover {
  border-color: var(--color-primary);
  color: var(--color-text);
}
.chip-dot {
  width: 6px;
  height: 6px;
  border-radius: var(--radius-full);
  background: var(--color-primary);
  flex-shrink: 0;
}
.chip-lock {
  flex-shrink: 0;
  opacity: 0.8;
}
.chip-name {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.chip-caret {
  font-size: 10px;
  transition: transform var(--transition-fast);
}
.chip-caret.open {
  transform: rotate(180deg);
}
/* 对话进行中锁定：弱化呈现，表达「已绑定」而非「可点击」 */
.profile-chip.locked {
  cursor: default;
  opacity: 0.6;
}
.profile-chip.locked:hover {
  border-color: var(--color-border);
  color: var(--color-text-secondary);
}
.profile-menu {
  position: absolute;
  top: 30px;
  left: 0;
  min-width: 160px;
  background: var(--color-bg-secondary);
  border: 1px solid var(--color-border);
  border-radius: var(--radius-xl);
  box-shadow: var(--shadow-lg);
  padding: var(--space-1);
  z-index: var(--z-dropdown);
  animation: menu-in 0.15s ease-out;
}
@keyframes menu-in {
  from { opacity: 0; transform: translateY(-4px); }
  to { opacity: 1; transform: translateY(0); }
}
.profile-option {
  display: flex;
  align-items: center;
  gap: 4px;
  width: 100%;
  padding: 6px 10px;
  border: none;
  border-radius: var(--radius-md);
  background: transparent;
  color: var(--color-text);
  font-size: var(--text-sm);
  cursor: pointer;
  text-align: left;
}
.profile-option:hover {
  background: var(--color-bg-hover);
}
.profile-option.current {
  color: var(--color-primary);
}
.option-check {
  width: 14px;
  flex-shrink: 0;
}
.profile-empty {
  padding: 6px 10px;
  color: var(--color-text-muted);
  font-size: var(--text-sm);
}
</style>

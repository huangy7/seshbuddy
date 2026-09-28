<script setup lang="ts">
import { ref, onMounted, onUnmounted, computed, watch } from "vue";
import { invokeApp } from "../../utils/invokeApp";
import { t } from "../../i18n";

export interface ConfiguredModelItem {
  id: string;
  /** 配置里的自定义显示名（`ANTHROPIC_*_MODEL_NAME`）；为空表示显示名按 id 推导 */
  customName: string;
  role?: string;
}

const model = defineModel<string>({ default: "" });
const props = defineProps<{ profile: string }>();
const open = ref(false);
const rootRef = ref<HTMLElement | null>(null);

const STORAGE_KEY = "assistant.lastModel";

/** 从当前 profile 配置中读取的模型列表（只存与语言无关的字段） */
const profileModels = ref<ConfiguredModelItem[]>([]);

/**
 * 下拉选项。显示名在**渲染时**求值：主模型那条文案取自语言包，若在加载配置时求值
 * 并缓存进 `profileModels`，用户切换语言后它不会重算，标签就停在旧语言。
 */
const modelOptions = computed(() =>
  profileModels.value.map((item) => {
    const shortLabel = item.customName || formatModelLabel(item.id);
    return { id: item.id, role: item.role, shortLabel, label: optionLabel(item, shortLabel) };
  }),
);

/**
 * 选项显示名。`customName` 是用户自己配的名字，与界面语言无关，优先原样显示；
 * 其余按角色套文案——主模型没有天然名字，用文案把推导名包起来，其它角色只在
 * id 本身不含角色词时才用角色名兜底。
 */
function optionLabel(item: ConfiguredModelItem, shortLabel: string): string {
  if (item.customName) return item.customName;
  switch (item.role) {
    case "main":
      return t("assistant.modelPicker.mainModel", { model: shortLabel });
    case "sonnet":
      return item.id.toLowerCase().includes("sonnet") ? shortLabel : "Sonnet";
    case "opus":
      return item.id.toLowerCase().includes("opus") ? shortLabel : "Opus";
    case "haiku":
      return item.id.toLowerCase().includes("haiku") ? shortLabel : "Haiku";
    default:
      return shortLabel;
  }
}

function formatModelLabel(apiModelId: string): string {
  if (!apiModelId) return t("assistant.modelPicker.defaultModel");
  let clean = apiModelId.replace(/^anthropic\./i, "").replace(/\[1m\]$/i, "").trim();
  if (clean.includes("/")) {
    clean = clean.split("/").pop() || clean;
  }
  if (clean.toLowerCase().includes("sonnet")) {
    const v = clean.match(/(\d+(?:\.\d+)?)/);
    return v ? `Sonnet ${v[1]}` : "Sonnet";
  }
  if (clean.toLowerCase().includes("opus")) {
    const v = clean.match(/(\d+(?:\.\d+)?)/);
    return v ? `Opus ${v[1]}` : "Opus";
  }
  if (clean.toLowerCase().includes("haiku")) {
    const v = clean.match(/(\d+(?:\.\d+)?)/);
    return v ? `Haiku ${v[1]}` : "Haiku";
  }
  return clean;
}

/** 当前选中模型的显示名 */
const displayLabel = computed(() => {
  // 1. 用户手动选中了某个具体模型 ID
  if (model.value) {
    const found = modelOptions.value.find((m) => m.id === model.value);
    if (found) return found.label || found.shortLabel || formatModelLabel(found.id);
    // 选中的 id 不在 profileModels 里（配置读取失败、profile 为空等）：只能按 id 推导，
    // 与语言无关，故这里不取语言包。
    return formatModelLabel(model.value);
  }

  // 2. model.value 为空（跟随 Profile 默认）
  const main = modelOptions.value.find((m) => m.role === "main");
  if (main) return main.label || main.shortLabel;

  const sonnet = modelOptions.value.find((m) => m.role === "sonnet");
  if (sonnet) return sonnet.label || sonnet.shortLabel;

  if (modelOptions.value.length > 0) {
    return modelOptions.value[0].label || modelOptions.value[0].shortLabel;
  }

  return t("assistant.modelPicker.defaultModel");
});

/** 默认模型下拉选项的提示文字 */
const defaultModelHint = computed(() => {
  const main = modelOptions.value.find((m) => m.role === "main");
  if (main) return main.label;
  const sonnet = modelOptions.value.find((m) => m.role === "sonnet");
  if (sonnet) return sonnet.label;
  return t("assistant.modelPicker.followConfig");
});

// 点击外部收起
function onDocClick(e: MouseEvent) {
  if (open.value && rootRef.value && !rootRef.value.contains(e.target as Node)) {
    open.value = false;
  }
}

onMounted(() => {
  document.addEventListener("click", onDocClick, true);
  const saved = localStorage.getItem(STORAGE_KEY);
  if (saved !== null) {
    model.value = saved;
  }
});

onUnmounted(() => document.removeEventListener("click", onDocClick, true));

// profile 变化时重新加载配置中的模型
watch(
  () => props.profile,
  (name) => {
    if (name) loadModelsFromProfile(name);
  },
  { immediate: true },
);

/** 从 profile 配置中提取配置的模型 */
async function loadModelsFromProfile(profileName: string) {
  if (!profileName) {
    profileModels.value = [];
    return;
  }

  try {
    const raw = await invokeApp<string>("read_profile", {
      cliId: "claude",
      name: profileName,
      scope: null,
    });
    const parsed = JSON.parse(raw);
    const env = parsed?.env || {};
    const items: ConfiguredModelItem[] = [];

    // 1. 主模型 (ANTHROPIC_MODEL)
    if (env.ANTHROPIC_MODEL?.trim()) {
      const id = env.ANTHROPIC_MODEL.trim();
      items.push({
        id,
        customName: env.ANTHROPIC_MODEL_NAME?.trim() || "",
        role: "main",
      });
    }

    // 2. Sonnet
    if (env.ANTHROPIC_DEFAULT_SONNET_MODEL?.trim()) {
      const id = env.ANTHROPIC_DEFAULT_SONNET_MODEL.trim();
      const customName = env.ANTHROPIC_DEFAULT_SONNET_MODEL_NAME?.trim() || "";
      if (!items.some((i) => i.id === id)) {
        items.push({ id, customName, role: "sonnet" });
      }
    }

    // 3. Opus
    if (env.ANTHROPIC_DEFAULT_OPUS_MODEL?.trim()) {
      const id = env.ANTHROPIC_DEFAULT_OPUS_MODEL.trim();
      const customName = env.ANTHROPIC_DEFAULT_OPUS_MODEL_NAME?.trim() || "";
      if (!items.some((i) => i.id === id)) {
        items.push({ id, customName, role: "opus" });
      }
    }

    // 4. Haiku
    if (env.ANTHROPIC_DEFAULT_HAIKU_MODEL?.trim()) {
      const id = env.ANTHROPIC_DEFAULT_HAIKU_MODEL.trim();
      const customName = env.ANTHROPIC_DEFAULT_HAIKU_MODEL_NAME?.trim() || "";
      if (!items.some((i) => i.id === id)) {
        items.push({ id, customName, role: "haiku" });
      }
    }

    profileModels.value = items;

    // 如果当前选中的模型不在新 profile 中，重置为跟随新 profile 默认
    if (model.value && !items.some((i) => i.id === model.value)) {
      model.value = "";
      localStorage.setItem(STORAGE_KEY, "");
    }
  } catch {
    profileModels.value = [];
  }
}

function select(id: string) {
  model.value = id;
  localStorage.setItem(STORAGE_KEY, id);
  open.value = false;
}

function toggle() {
  open.value = !open.value;
}
</script>

<template>
  <div class="model-picker-wrap" ref="rootRef">
    <button
      type="button"
      class="model-chip"
      :title="model ? t('assistant.modelPicker.chipTitleSelected', { label: displayLabel, id: model }) : t('assistant.modelPicker.chipTitleFollow', { hint: defaultModelHint })"
      @click="toggle"
    >
      <svg class="chip-icon" viewBox="0 0 16 16" width="11" height="11" fill="none" stroke="currentColor" stroke-width="1.4">
        <rect x="2" y="2" width="12" height="12" rx="2" />
        <circle cx="5.5" cy="5.5" r="1" fill="currentColor" stroke="none" />
        <circle cx="10.5" cy="5.5" r="1" fill="currentColor" stroke="none" />
        <circle cx="5.5" cy="10.5" r="1" fill="currentColor" stroke="none" />
        <circle cx="10.5" cy="10.5" r="1" fill="currentColor" stroke="none" />
      </svg>
      <span class="chip-name">{{ displayLabel }}</span>
      <span class="chip-caret" :class="{ open }">▾</span>
    </button>

    <!-- 向上弹出的模型菜单 -->
    <div v-if="open" class="model-menu">
      <div class="model-options">
        <!-- 默认模型 -->
        <button
          type="button"
          class="model-option"
          :class="{ current: !model }"
          @click="select('')"
        >
          <span class="option-check">{{ !model ? '✓' : '' }}</span>
          <span class="option-label">{{ t("assistant.modelPicker.defaultModel") }}</span>
          <span class="option-hint">{{ defaultModelHint }}</span>
        </button>

        <!-- 配置中的模型 -->
        <button
          v-for="m in modelOptions"
          :key="m.id"
          type="button"
          class="model-option"
          :class="{ current: model === m.id }"
          :title="m.id"
          @click="select(m.id)"
        >
          <span class="option-check">{{ model === m.id ? '✓' : '' }}</span>
          <span class="option-label">{{ m.label }}</span>
          <span class="option-id">{{ m.id }}</span>
        </button>

        <div v-if="modelOptions.length === 0" class="model-empty">
          {{ t("assistant.modelPicker.noOverride") }}
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.model-picker-wrap {
  position: relative;
  display: inline-flex;
  align-items: center;
}
.model-chip {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  padding: 2px 7px;
  border: 1px solid var(--color-border);
  border-radius: var(--radius-full);
  background: transparent;
  color: var(--color-text-secondary);
  font-size: 11px;
  cursor: pointer;
  max-width: 180px;
  line-height: 1.4;
  transition: border-color var(--transition-fast), color var(--transition-fast), background var(--transition-fast);
}
.model-chip:hover {
  border-color: var(--color-primary);
  color: var(--color-text);
  background: var(--color-bg-hover);
}
.chip-icon {
  flex-shrink: 0;
  opacity: 0.75;
}
.chip-name {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.chip-caret {
  font-size: 9px;
  transition: transform var(--transition-fast);
}
.chip-caret.open {
  transform: rotate(180deg);
}

/* 向上弹出菜单 */
.model-menu {
  position: absolute;
  bottom: calc(100% + 6px);
  left: 0;
  min-width: 280px;
  max-width: 400px;
  width: max-content;
  background: var(--color-bg-secondary);
  border: 1px solid var(--color-border);
  border-radius: var(--radius-xl);
  box-shadow: var(--shadow-lg);
  padding: var(--space-1);
  z-index: var(--z-dropdown);
  animation: menu-in 0.15s ease-out;
}
@keyframes menu-in {
  from { opacity: 0; transform: translateY(4px); }
  to { opacity: 1; transform: translateY(0); }
}
.model-options {
  max-height: 200px;
  overflow-y: auto;
}
.model-option {
  display: flex;
  align-items: center;
  gap: 6px;
  width: 100%;
  padding: 6px 8px;
  border: none;
  border-radius: var(--radius-md);
  background: transparent;
  color: var(--color-text);
  font-size: var(--text-xs);
  cursor: pointer;
  text-align: left;
  transition: background var(--transition-fast);
}
.model-option:hover {
  background: var(--color-bg-hover);
}
.model-option.current {
  color: var(--color-primary);
  font-weight: 500;
}
.option-check {
  width: 12px;
  flex-shrink: 0;
  font-size: 11px;
}
.option-label {
  flex-shrink: 0;
  font-weight: 500;
}
.option-id {
  color: var(--color-text-muted);
  font-family: var(--font-mono);
  font-size: 10px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  flex: 1;
  margin-left: 8px;
  text-align: right;
}
.option-hint {
  color: var(--color-text-muted);
  font-size: 10px;
  margin-left: auto;
  flex-shrink: 0;
}
.model-empty {
  padding: 8px 10px;
  font-size: var(--text-2xs);
  color: var(--color-text-muted);
  text-align: center;
}
</style>

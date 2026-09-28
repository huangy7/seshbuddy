<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { invokeApp, renderAppError } from "../utils/invokeApp";
import OptionCard from "./OptionCard.vue";
import ToggleSwitch from "./ToggleSwitch.vue";
import SvgIcon from "./icons/SvgIcon.vue";
import { useTheme, type Theme } from "../composables/useTheme";
import { afterNextPaint } from "../utils/defer";
import { changeLocale, currentLocale, t } from "../i18n";
import { SUPPORTED_LOCALES, isLocale, type Locale } from "../i18n/types";

const localError = ref("");

const { theme } = useTheme();
const isWindows = navigator.userAgent.toLowerCase().includes("windows");

// 选项文案在 computed 内求值：写成常量数组会在 setup 时求值一次，
// 之后切换语言时不会重算，标签就停在旧语言。
const themeOptions = computed<Array<{ value: Theme; label: string; hint: string }>>(() => [
  { value: "system", label: t("settings.appearance.themeSystem"), hint: t("settings.appearance.themeSystemHint") },
  { value: "light", label: t("settings.appearance.themeLight"), hint: t("settings.appearance.themeLightHint") },
  { value: "dark", label: t("settings.appearance.themeDark"), hint: t("settings.appearance.themeDarkHint") },
]);

const activeThemeHint = computed(() => {
  return themeOptions.value.find((opt) => opt.value === theme.value)?.hint
    ?? t("settings.appearance.themeFallbackHint");
});

// 语言名用本族语书写（Deutsch 而非 German），四份语言包取值相同，
// 界面语言看不懂时用户也能找到自己的语言。键表按 Locale 穷举，
// 新增语言时由类型检查逼出对应条目。
const LANGUAGE_LABEL_KEYS: Record<Locale, string> = {
  en: "common.language.en",
  zh: "common.language.zh",
  ja: "common.language.ja",
  de: "common.language.de",
};

const languageOptions = computed<Array<{ value: Locale; label: string }>>(() =>
  SUPPORTED_LOCALES.map((value) => ({ value, label: t(LANGUAGE_LABEL_KEYS[value]) })),
);

// OptionCard 的选项值类型是 string，此处用类型守卫收窄到 Locale，
// 避免把一个未经验证的值写进语言偏好
async function onSelectLanguage(locale: string) {
  if (!isLocale(locale)) return;
  await changeLocale(locale);
}

const dockVisible = ref(true);
const dockToggling = ref(false);

async function loadDockVisible() {
  if (isWindows) return;
  dockToggling.value = true;
  try {
    dockVisible.value = await invokeApp<boolean>("get_dock_visible");
  } catch (e: any) {
    localError.value = renderAppError(e);
  } finally {
    dockToggling.value = false;
  }
}

async function onDockVisibleChange(val: boolean) {
  if (dockToggling.value) return;
  const prev = dockVisible.value;
  dockVisible.value = val;
  dockToggling.value = true;
  try {
    await invokeApp("set_dock_visible", { visible: val });
  } catch (e: any) {
    dockVisible.value = prev;
    localError.value = renderAppError(e);
  } finally {
    dockToggling.value = false;
  }
}

onMounted(async () => {
  await afterNextPaint();
  void loadDockVisible();
});
</script>

<template>
  <div class="settings-group">
    <div v-if="localError" class="settings-error">
      <span>{{ localError }}</span>
      <button class="clear-error" type="button" :aria-label="t('settings.common.closeError')" @click="localError = ''">
        <SvgIcon name="x" :size="16" />
      </button>
    </div>

    <div class="settings-row">
      <div class="row-content">
        <div class="row-title">{{ t("settings.appearance.title") }}</div>
        <p class="row-hint">{{ activeThemeHint }}</p>
      </div>
      <div class="row-action">
        <OptionCard v-model="theme" :options="themeOptions" />
      </div>
    </div>

    <div class="settings-row">
      <div class="row-content">
        <div class="row-title">{{ t("common.language.title") }}</div>
        <p class="row-hint">{{ t("common.language.hint") }}</p>
      </div>
      <div class="row-action">
        <OptionCard
          :model-value="currentLocale"
          :options="languageOptions"
          @update:model-value="onSelectLanguage"
        />
      </div>
    </div>

    <div v-if="!isWindows" class="settings-row">
      <div class="row-content">
        <div class="row-title">{{ t("settings.appearance.dockTitle") }}</div>
        <p class="row-hint">
          {{ dockToggling ? t("settings.appearance.dockTogglingHint") : t("settings.appearance.dockHint") }}
        </p>
      </div>
      <div class="row-action">
        <ToggleSwitch
          :model-value="dockVisible"
          :disabled="dockToggling"
          :aria-label="t('settings.appearance.dockTitle')"
          @update:model-value="onDockVisibleChange"
        />
      </div>
    </div>

  </div>
</template>

<style scoped>
.settings-group {
  display: flex;
  flex-direction: column;
}
.settings-error {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: var(--space-3) var(--space-4);
  background: rgba(220, 38, 38, 0.08);
  border-bottom: 1px solid rgba(220, 38, 38, 0.16);
  color: var(--color-danger);
  font-size: var(--text-sm);
}
.clear-error {
  display: flex;
  align-items: center;
  justify-content: center;
  background: transparent;
  border: none;
  color: var(--color-danger);
  cursor: pointer;
  padding: 2px;
  border-radius: 50%;
  opacity: 0.6;
  transition: all var(--transition-fast);
}
.clear-error:hover {
  opacity: 1;
  background: rgba(220, 38, 38, 0.1);
}
.settings-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: var(--space-4) var(--space-5);
  gap: var(--space-4);
}
.settings-row + .settings-row {
  border-top: 1px solid var(--color-border);
}
.row-content {
  display: flex;
  flex-direction: column;
  gap: 4px;
  flex: 1;
  min-width: 0;
}
.row-title {
  font-size: var(--text-sm);
  font-weight: 500;
  color: var(--color-text);
}
.row-hint {
  font-size: var(--text-xs);
  color: var(--color-text-muted);
  margin: 0;
  line-height: 1.5;
}
.row-action {
  flex-shrink: 0;
}
</style>

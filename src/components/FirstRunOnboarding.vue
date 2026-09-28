<script setup lang="ts">
import { computed, nextTick, onMounted, ref } from "vue";
import { currentLocale, setLocale, t } from "../i18n";
import { SUPPORTED_LOCALES, type Locale } from "../i18n/types";
import type { CliId, CliOption } from "../types/cli";
import mascot from "../assets/mascot.png";

const props = withDefaults(defineProps<{
  cliOptions: CliOption[];
  saving?: boolean;
  error?: string;
  statusError?: boolean;
}>(), { saving: false, error: "", statusError: false });

const emit = defineEmits<{
  complete: [selection: { locale: Locale; cliIds: CliId[] }];
  retry: [];
  continueDefault: [];
}>();

const step = ref<"language" | "sources">("language");
const selectedLocale = ref<Locale>(currentLocale.value);
const selectedCliIds = ref<CliId[]>((() => {
  const found = props.cliOptions.filter((cli) => cli.hasSessions).map((cli) => cli.id);
  return found.length ? found : props.cliOptions.map((cli) => cli.id);
})());
const title = computed(() => props.statusError
  ? t("app.onboarding.statusErrorTitle")
  : step.value === "language"
    ? t("app.onboarding.languageTitle")
    : t("app.onboarding.sourcesTitle"));
const primaryButton = ref<HTMLButtonElement | null>(null);
const dialogElement = ref<HTMLDivElement | null>(null);

function focusPrimary() {
  void nextTick(() => primaryButton.value?.focus());
}

onMounted(focusPrimary);

function trapFocus(event: KeyboardEvent) {
  if (event.key !== "Tab" || !dialogElement.value) return;
  const focusable = [...dialogElement.value.querySelectorAll<HTMLElement>("button:not([disabled]), input:not([disabled])")];
  if (focusable.length === 0) return;
  if (event.shiftKey && document.activeElement === focusable[0]) {
    event.preventDefault();
    focusable[focusable.length - 1].focus();
  } else if (!event.shiftKey && document.activeElement === focusable[focusable.length - 1]) {
    event.preventDefault();
    focusable[0].focus();
  }
}

function selectLanguage(locale: Locale) {
  selectedLocale.value = locale;
  setLocale(locale);
}

function nextStep() {
  step.value = "sources";
  focusPrimary();
}

function previousStep() {
  step.value = "language";
  focusPrimary();
}

function toggleCli(cliId: CliId, checked: boolean) {
  const selected = new Set(selectedCliIds.value);
  if (checked) selected.add(cliId);
  else selected.delete(cliId);
  selectedCliIds.value = props.cliOptions.map((cli) => cli.id).filter((id) => selected.has(id));
}

function finish() {
  if (props.saving || selectedCliIds.value.length === 0) return;
  emit("complete", { locale: selectedLocale.value, cliIds: [...selectedCliIds.value] });
}

const languageLabelKeys: Record<Locale, string> = {
  en: "common.language.en",
  zh: "common.language.zh",
  ja: "common.language.ja",
  de: "common.language.de",
};
</script>

<template>
  <div ref="dialogElement" class="onboarding-backdrop" role="dialog" aria-modal="true" :aria-label="title" @keydown="trapFocus">
    <section class="onboarding-window">
      <div class="onboarding-hero">
        <div class="onboarding-logo"><img :src="mascot" alt="" class="onboarding-mascot" /></div>
        <div class="onboarding-identity">
          <span class="onboarding-brand">SeshBuddy</span>
          <span class="onboarding-tagline">{{ t("app.boot.brandSubtitle") }}</span>
        </div>
        <div v-if="!statusError" class="onboarding-journey" aria-hidden="true">
          <div :class="{ current: step === 'language', complete: step === 'sources' }"><span>01</span>{{ t("app.onboarding.languageTitle") }}</div>
          <div :class="{ current: step === 'sources' }"><span>02</span>{{ t("app.onboarding.sourcesTitle") }}</div>
        </div>
      </div>

      <div v-if="statusError" class="onboarding-content">
        <h1>{{ title }}</h1>
        <p>{{ t("app.onboarding.statusErrorHint") }}</p>
        <p v-if="error" class="onboarding-error" role="alert">{{ error }}</p>
        <div class="onboarding-actions">
          <button type="button" data-action="continue-default" @click="emit('continueDefault')">{{ t("app.onboarding.continueDefault") }}</button>
          <button ref="primaryButton" type="button" class="primary" data-action="retry" @click="emit('retry')">{{ t("app.onboarding.retry") }}</button>
        </div>
      </div>

      <div v-else class="onboarding-content">
        <div class="onboarding-progress" aria-hidden="true"><span>{{ step === 'language' ? '01' : '02' }} / 02</span><i><b :class="{ second: step === 'sources' }"></b></i></div>
        <h1>{{ title }}</h1>
        <p>{{ step === "language" ? t("app.onboarding.languageHint") : t("app.onboarding.sourcesHint") }}</p>

        <div v-if="step === 'language'" class="language-grid">
          <button
            v-for="locale in SUPPORTED_LOCALES"
            :key="locale"
            type="button"
            class="option-tile"
            :class="{ selected: selectedLocale === locale }"
            :data-locale="locale"
            :aria-pressed="selectedLocale === locale"
            @click="selectLanguage(locale)"
          ><span>{{ t(languageLabelKeys[locale]) }}</span><span class="option-check" aria-hidden="true">✓</span></button>
        </div>

        <div v-else class="source-list">
          <label v-for="cli in cliOptions" :key="cli.id" class="source-row" :class="{ selected: selectedCliIds.includes(cli.id) }" :data-cli-id="cli.id">
            <span class="source-copy"><strong>{{ cli.name }}</strong><small :class="{ available: cli.hasSessions }">{{ cli.hasSessions ? t("app.onboarding.hasSessions") : t("app.onboarding.noSessions") }}</small></span>
            <input type="checkbox" :checked="selectedCliIds.includes(cli.id)" :aria-label="cli.name" @change="toggleCli(cli.id, ($event.target as HTMLInputElement).checked)" />
          </label>
        </div>

        <p v-if="error" class="onboarding-error" role="alert">{{ error }}</p>
        <div class="onboarding-actions">
          <button v-if="step === 'sources'" type="button" data-action="back" :disabled="saving" @click="previousStep">{{ t("app.onboarding.back") }}</button>
          <span v-else></span>
          <button v-if="step === 'language'" ref="primaryButton" type="button" class="primary" data-action="next" @click="nextStep">{{ t("app.onboarding.next") }}</button>
          <button v-else ref="primaryButton" type="button" class="primary" data-action="finish" :disabled="saving || selectedCliIds.length === 0" @click="finish">{{ saving ? t("app.onboarding.saving") : t("app.onboarding.finish") }}</button>
        </div>
      </div>
    </section>
  </div>
</template>

<style scoped>
.onboarding-backdrop {
  position: fixed;
  inset: 0;
  z-index: 10000;
  display: grid;
  place-items: center;
  padding: 24px;
  overflow-y: auto;
  background:
    radial-gradient(circle at 20% 20%, color-mix(in srgb, var(--color-primary) 16%, transparent), transparent 34%),
    radial-gradient(circle at 84% 78%, color-mix(in srgb, #f3786c 13%, transparent), transparent 36%),
    var(--color-bg);
}
.onboarding-window {
  display: grid;
  grid-template-columns: 220px minmax(0, 1fr);
  width: min(100%, 740px);
  height: min(470px, calc(100dvh - 48px));
  min-height: 0;
  overflow: hidden;
  border: 1px solid color-mix(in srgb, var(--color-border) 72%, transparent);
  border-radius: 26px;
  background: var(--color-bg);
  box-shadow: 0 30px 80px rgba(20, 33, 58, .14), 0 8px 26px rgba(20, 33, 58, .06);
}
.onboarding-hero {
  position: relative;
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  padding: 26px 22px;
  overflow: hidden;
  background:
    radial-gradient(circle at 30% 34%, rgba(255, 255, 255, .82), transparent 42%),
    linear-gradient(150deg, color-mix(in srgb, var(--color-primary) 14%, var(--color-bg)), color-mix(in srgb, #f3786c 13%, var(--color-bg)));
  border-right: 1px solid color-mix(in srgb, var(--color-border) 55%, transparent);
}
.onboarding-hero::after {
  position: absolute;
  right: -85px;
  bottom: -115px;
  width: 270px;
  height: 270px;
  border: 1px solid color-mix(in srgb, var(--color-primary) 24%, transparent);
  border-radius: 50%;
  box-shadow: 0 0 0 38px color-mix(in srgb, var(--color-primary) 5%, transparent), 0 0 0 80px color-mix(in srgb, var(--color-primary) 4%, transparent);
  content: "";
  pointer-events: none;
}
.onboarding-logo { display: grid; place-items: center; width: 82px; height: 82px; margin: 36px auto 18px; border-radius: 23px; background: rgba(255, 255, 255, .48); box-shadow: 0 14px 30px rgba(27, 42, 70, .08); }
.onboarding-mascot { width: 70px; height: 70px; object-fit: contain; filter: drop-shadow(0 10px 15px rgba(243, 120, 108, .18)); }
.onboarding-identity { display: grid; gap: 7px; width: 100%; text-align: center; position: relative; z-index: 1; }
.onboarding-brand { color: var(--color-text); font-size: 25px; font-weight: 800; letter-spacing: -.045em; line-height: 1.1; }
.onboarding-tagline { color: var(--color-text-muted); font-size: 9px; font-weight: 650; letter-spacing: .1em; text-transform: uppercase; }
.onboarding-journey { display: grid; gap: 10px; margin-top: auto; position: relative; z-index: 1; }
.onboarding-journey div { display: flex; align-items: center; gap: 12px; color: var(--color-text-muted); font-size: 13px; font-weight: 600; }
.onboarding-journey span { display: grid; place-items: center; width: 28px; height: 28px; border: 1px solid color-mix(in srgb, var(--color-border) 70%, transparent); border-radius: 50%; background: color-mix(in srgb, var(--color-bg) 72%, transparent); font-size: 10px; }
.onboarding-journey .current { color: var(--color-text); }
.onboarding-journey .current span, .onboarding-journey .complete span { color: white; background: var(--color-primary); border-color: var(--color-primary); }
.onboarding-content { display: flex; flex-direction: column; min-width: 0; min-height: 0; padding: 27px 30px 23px; }
.onboarding-progress { display: flex; align-items: center; gap: 13px; width: 100%; color: var(--color-primary); font-size: 11px; font-weight: 800; letter-spacing: .12em; }
.onboarding-progress i { flex: 1; height: 3px; overflow: hidden; border-radius: 3px; background: var(--color-bg-secondary); }
.onboarding-progress b { display: block; width: 50%; height: 100%; border-radius: inherit; background: var(--color-primary); transition: width .25s ease; }
.onboarding-progress b.second { width: 100%; }
.onboarding-content h1 { margin: 20px 0 6px; color: var(--color-text); font-size: clamp(24px, 3vw, 28px); font-weight: 760; letter-spacing: -.035em; line-height: 1.2; }
.onboarding-content p { margin: 0 0 12px; color: var(--color-text-secondary); font-size: 13px; line-height: 1.55; }
.language-grid { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 10px; margin: 12px 0 18px; }
.option-tile { display: flex; align-items: center; justify-content: space-between; min-height: 62px; padding: 0 16px; border: 1px solid var(--color-border); border-radius: 13px; background: var(--color-bg-secondary); color: var(--color-text); font-size: 16px; font-weight: 600; text-align: left; transition: border-color .2s, background .2s, transform .2s, box-shadow .2s; }
.option-tile:hover, .source-row:hover { transform: translateY(-2px); border-color: color-mix(in srgb, var(--color-primary) 55%, var(--color-border)); }
.option-tile.selected, .source-row.selected { border-color: var(--color-primary); background: color-mix(in srgb, var(--color-primary) 7%, var(--color-bg)); box-shadow: 0 0 0 3px color-mix(in srgb, var(--color-primary) 10%, transparent); }
.option-check { display: grid; place-items: center; width: 24px; height: 24px; border-radius: 50%; color: transparent; font-size: 13px; }
.option-tile.selected .option-check { color: white; background: var(--color-primary); }
.source-list { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 10px; min-height: 0; margin: 12px 0 18px; }
.source-row { display: flex; align-items: center; justify-content: space-between; gap: 8px; min-height: 64px; padding: 8px 12px; border: 1px solid var(--color-border); border-radius: 11px; background: var(--color-bg-secondary); cursor: pointer; transition: border-color .2s, background .2s, transform .2s, box-shadow .2s; }
.source-copy { display: grid; flex: 1; min-width: 0; gap: 3px; color: var(--color-text); }
.source-copy strong { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-size: 14px; }
.source-copy small { width: fit-content; padding: 3px 7px; border-radius: 999px; background: color-mix(in srgb, var(--color-text-muted) 9%, transparent); color: var(--color-text-muted); font-size: 11px; white-space: nowrap; }
.source-copy small.available { background: color-mix(in srgb, var(--color-primary) 12%, transparent); color: var(--color-primary); }
.source-row input { flex: none; width: 18px; height: 18px; accent-color: var(--color-primary); }
.onboarding-error { color: var(--color-danger, #c0392b) !important; }
.onboarding-actions { display: flex; justify-content: space-between; align-items: center; gap: 12px; flex: none; margin-top: auto; padding-top: 12px; border-top: 1px solid var(--color-border); }
.onboarding-actions button { min-height: 40px; padding: 8px 15px; border: 1px solid var(--color-border); border-radius: 10px; background: var(--color-bg-secondary); color: var(--color-text); font-weight: 650; transition: transform .2s, background .2s, box-shadow .2s; }
.onboarding-actions button:hover:not(:disabled) { transform: translateY(-1px); }
.onboarding-actions button.primary { border-color: var(--color-primary); background: var(--color-primary); color: white; box-shadow: 0 7px 18px color-mix(in srgb, var(--color-primary) 24%, transparent); }
.onboarding-actions button:disabled { opacity: .5; cursor: not-allowed; }
.onboarding-actions button:focus-visible, .source-row:focus-within, .option-tile:focus-visible { outline: 2px solid var(--color-primary); outline-offset: 2px; }
@media (max-width: 700px) {
  .onboarding-backdrop { padding: 16px; }
  .onboarding-window { grid-template-columns: 1fr; grid-template-rows: auto minmax(0, 1fr); width: min(100%, 560px); height: min(550px, calc(100dvh - 32px)); }
  .onboarding-hero { flex-direction: row; align-items: center; gap: 15px; min-height: 100px; padding: 18px 24px; border-right: 0; border-bottom: 1px solid var(--color-border); }
  .onboarding-logo { width: 58px; height: 58px; margin: 0; border-radius: 17px; }
  .onboarding-mascot { width: 51px; height: 51px; }
  .onboarding-identity { width: auto; text-align: left; }
  .onboarding-brand { font-size: 23px; }
  .onboarding-journey { display: none; }
  .onboarding-content { padding: 22px 24px; }
  .onboarding-content h1 { margin-top: 18px; }
  .source-list { grid-template-columns: 1fr; gap: 6px; overflow-y: auto; padding: 3px; margin: 3px -3px 10px; }
  .source-row { min-height: 52px; }
}
@media (max-width: 480px) { .language-grid { grid-template-columns: 1fr; } .option-tile { min-height: 58px; } }
</style>

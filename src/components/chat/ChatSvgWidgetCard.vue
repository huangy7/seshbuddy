<script setup lang="ts">
import { ref, computed } from "vue";
import SvgIcon from "../icons/SvgIcon.vue";
import CopyButton from "../CopyButton.vue";
import { ensureWorkBuddySvgStyles } from "../../utils/svg";
import { t } from "../../i18n";

const props = defineProps<{
  title?: string;
  svgCode: string;
}>();

const emit = defineEmits<{
  preview: [url: string];
  download: [url: string];
}>();

const showCode = ref(false);

// 标题缺省值走 computed 而非 withDefaults 默认值：后者在组件挂载时求值一次，
// 用户之后切换语言时这个标题不会跟着变。
const displayTitle = computed(() => props.title ?? t("chat.svgCard.defaultTitle"));

const computedSvg = computed(() => {
  return ensureWorkBuddySvgStyles(props.svgCode);
});

function onPreview() {
  emit("preview", computedSvg.value);
}

async function onDownload() {
  const safeTitle = (displayTitle.value || "diagram").replace(/[\\/:*?"<>|]/g, "_").trim() || "diagram";

  try {
    const { save } = await import("@tauri-apps/plugin-dialog");
    const { writeTextFile } = await import("@tauri-apps/plugin-fs");
    const savePath = await save({
      filters: [{ name: t("common.chatView.svgImage"), extensions: ["svg"] }],
      defaultPath: `${safeTitle}.svg`,
    });
    if (savePath) {
      await writeTextFile(savePath, computedSvg.value);
      return;
    }
  } catch {
    // Browser or fallback download
    const blob = new Blob([computedSvg.value], { type: "image/svg+xml;charset=utf-8" });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = `${safeTitle}.svg`;
    document.body.appendChild(a);
    a.click();
    document.body.removeChild(a);
    URL.revokeObjectURL(url);
  }
}
</script>

<template>
  <div class="svg-widget-card">
    <div class="svg-widget-header">
      <div class="header-left">
        <SvgIcon name="image" :size="13" class="widget-icon" />
        <span class="widget-badge">SVG</span>
        <span class="widget-title" :title="displayTitle">{{ displayTitle }}</span>
      </div>
      <div class="header-actions">
        <button
          type="button"
          class="widget-action-btn"
          :class="{ active: showCode }"
          :title="showCode ? t('chat.svgCard.switchToChart') : t('chat.svgCard.viewSource')"
          @click="showCode = !showCode"
        >
          <SvgIcon name="code" :size="12" />
        </button>
        <CopyButton :text="computedSvg" size="sm" />
        <button
          type="button"
          class="widget-action-btn"
          :title="t('chat.svgCard.preview')"
          @click="onPreview"
        >
          <SvgIcon name="zoom-in" :size="12" />
        </button>
        <button
          type="button"
          class="widget-action-btn"
          :title="t('chat.svgCard.downloadSvg')"
          @click="onDownload"
        >
          <SvgIcon name="download" :size="12" />
        </button>
      </div>
    </div>

    <div class="svg-widget-content">
      <pre v-if="showCode" class="svg-code-view"><code>{{ svgCode }}</code></pre>
      <div
        v-else
        class="svg-viewport workbuddy-svg-widget"
        v-html="computedSvg"
      ></div>
    </div>
  </div>
</template>

<style scoped>
.svg-widget-card {
  margin: var(--space-2) 0;
  border-radius: var(--radius-lg);
  border: 1px solid var(--color-border);
  background: var(--color-bg-secondary);
  overflow: hidden;
  box-shadow: 0 1px 3px rgba(0, 0, 0, 0.04);
  transition: border-color var(--transition-fast);
}

.svg-widget-card:hover {
  border-color: color-mix(in srgb, var(--color-primary) 35%, var(--color-border));
}

.svg-widget-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 6px 10px;
  background: color-mix(in srgb, var(--color-bg-secondary) 85%, var(--color-bg));
  border-bottom: 1px solid var(--color-border);
  gap: var(--space-2);
}

.header-left {
  display: flex;
  align-items: center;
  gap: 6px;
  min-width: 0;
  flex: 1;
}

.widget-icon {
  color: var(--color-primary);
  flex-shrink: 0;
}

.widget-badge {
  font-size: 10px;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.5px;
  padding: 1px 5px;
  border-radius: var(--radius-sm);
  background: color-mix(in srgb, var(--color-primary) 12%, transparent);
  color: var(--color-primary);
  flex-shrink: 0;
}

.widget-title {
  font-size: var(--text-xs);
  font-weight: 500;
  color: var(--color-text);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.header-actions {
  display: flex;
  align-items: center;
  gap: 4px;
  flex-shrink: 0;
}

.widget-action-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 24px;
  height: 24px;
  border-radius: var(--radius-sm);
  border: 1px solid transparent;
  background: transparent;
  color: var(--color-text-secondary);
  cursor: pointer;
  transition: all var(--transition-fast);
}

.widget-action-btn:hover {
  background: var(--color-bg-hover);
  color: var(--color-text);
  border-color: var(--color-border);
}

.widget-action-btn.active {
  background: var(--color-primary-light);
  color: var(--color-primary);
}

.svg-widget-content {
  padding: var(--space-3);
  display: flex;
  justify-content: center;
  align-items: center;
  background: var(--color-bg);
  min-height: 120px;
  overflow-x: auto;
}

.svg-viewport {
  width: 100%;
  display: flex;
  justify-content: center;
  align-items: center;
}

.svg-viewport :deep(svg) {
  max-width: 100%;
  height: auto;
  display: block;
  margin: 0 auto;
}

.svg-code-view {
  margin: 0;
  width: 100%;
  max-height: 380px;
  overflow: auto;
  font-family: var(--font-mono);
  font-size: var(--text-xs);
  line-height: 1.5;
  color: var(--color-text);
  background: var(--color-bg-secondary);
  padding: var(--space-3);
  border-radius: var(--radius-md);
}
</style>

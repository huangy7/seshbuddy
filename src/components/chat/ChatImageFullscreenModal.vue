<script setup lang="ts">
import { ref, computed, watch, onMounted, onBeforeUnmount } from "vue";
import SvgIcon from "../icons/SvgIcon.vue";
import { t } from "../../i18n";
import { copyToClipboard } from "../../utils/clipboard";

const props = defineProps<{
  imageUrl: string | null;
}>();

const emit = defineEmits<{
  close: [];
  download: [url: string];
}>();

const isSvg = computed(() => {
  if (!props.imageUrl) return false;
  const trimmed = props.imageUrl.trim();
  return trimmed.startsWith("<svg") || trimmed.startsWith("data:image/svg+xml");
});

const svgContent = computed(() => {
  if (!props.imageUrl) return "";
  const trimmed = props.imageUrl.trim();
  if (trimmed.startsWith("<svg")) {
    return trimmed;
  }
  if (trimmed.startsWith("data:image/svg+xml")) {
    if (trimmed.includes(";base64,")) {
      try {
        return atob(trimmed.split(";base64,")[1]);
      } catch {
        // ignore
      }
    }
    const commaIdx = trimmed.indexOf(",");
    if (commaIdx !== -1) {
      try {
        return decodeURIComponent(trimmed.slice(commaIdx + 1));
      } catch {
        return trimmed.slice(commaIdx + 1);
      }
    }
  }
  return "";
});

// Zoom & Pan state
const zoom = ref(1);
const isFit = ref(true);
const panX = ref(0);
const panY = ref(0);
const isPanning = ref(false);
const startPan = ref({ x: 0, y: 0 });

function resetToFit() {
  isFit.value = true;
  zoom.value = 1;
  panX.value = 0;
  panY.value = 0;
}

function setZoom(newZoom: number) {
  isFit.value = false;
  zoom.value = Math.max(0.2, Math.min(6, Math.round(newZoom * 100) / 100));
}

function zoomIn() {
  setZoom(zoom.value * 1.25);
}

function zoomOut() {
  setZoom(zoom.value / 1.25);
}

function onWheel(e: WheelEvent) {
  e.preventDefault();
  const delta = e.deltaY < 0 ? 1.15 : 0.85;
  setZoom(zoom.value * delta);
}

function onMouseDown(e: MouseEvent) {
  if (e.button !== 0) return;
  isPanning.value = true;
  startPan.value = {
    x: e.clientX - panX.value,
    y: e.clientY - panY.value,
  };
}

function onMouseMove(e: MouseEvent) {
  if (!isPanning.value) return;
  isFit.value = false;
  panX.value = e.clientX - startPan.value.x;
  panY.value = e.clientY - startPan.value.y;
}

function onMouseUp() {
  isPanning.value = false;
}

function onDoubleClick() {
  if (isFit.value) {
    setZoom(1.5);
  } else {
    resetToFit();
  }
}

const copied = ref(false);
let copyTimer: ReturnType<typeof setTimeout> | null = null;

async function onCopySvg() {
  if (!svgContent.value) return;
  if (await copyToClipboard(svgContent.value)) {
    copied.value = true;
    if (copyTimer) clearTimeout(copyTimer);
    copyTimer = setTimeout(() => {
      copied.value = false;
    }, 2000);
  } else {
    console.error("Failed to copy SVG content");
  }
}

function onKeydown(e: KeyboardEvent) {
  if (!props.imageUrl) return;
  if (e.key === "Escape") {
    emit("close");
  } else if (e.key === "+" || e.key === "=") {
    zoomIn();
  } else if (e.key === "-") {
    zoomOut();
  } else if (e.key === "0") {
    resetToFit();
  }
}

watch(() => props.imageUrl, () => {
  resetToFit();
  copied.value = false;
});

onMounted(() => {
  window.addEventListener("mouseup", onMouseUp);
  window.addEventListener("keydown", onKeydown);
});

onBeforeUnmount(() => {
  window.removeEventListener("mouseup", onMouseUp);
  window.removeEventListener("keydown", onKeydown);
});
</script>

<template>
  <Transition name="fade">
    <div v-if="imageUrl" class="modal-backdrop" @click="$emit('close')">
      <button class="fullscreen-close-btn" :title="t('common.chatSearchBar.close')" :aria-label="t('dialogs.common.close')" @click.stop="$emit('close')">
        <SvgIcon name="x" :size="24" />
      </button>

      <div
        class="fullscreen-content"
        @click.stop
        @wheel="onWheel"
        @mousedown="onMouseDown"
        @mousemove="onMouseMove"
        @dblclick="onDoubleClick"
      >
        <div
          v-if="isSvg"
          class="fullscreen-svg-wrapper workbuddy-svg-widget"
        >
          <div
            class="fullscreen-svg-container"
            :class="{ 'is-panning': isPanning, 'is-zoomed': !isFit }"
            :style="isFit ? {} : {
              transform: `translate(${panX}px, ${panY}px) scale(${zoom})`,
              transformOrigin: 'center center',
            }"
            v-html="svgContent"
          />
        </div>

        <div
          v-else
          class="fullscreen-image-wrapper"
        >
          <img
            :src="imageUrl"
            class="fullscreen-image"
            :class="{ 'is-panning': isPanning, 'is-zoomed': !isFit }"
            :style="isFit ? {} : {
              transform: `translate(${panX}px, ${panY}px) scale(${zoom})`,
              transformOrigin: 'center center',
            }"
            draggable="false"
          />
        </div>

        <div class="fullscreen-actions" @click.stop>
          <button
            type="button"
            class="fullscreen-action-btn"
            :title="t('chat.imageFullscreen.zoomOut')"
            @click="zoomOut"
          >
            <SvgIcon name="zoom-out" :size="16" />
          </button>
          <button
            type="button"
            class="fullscreen-zoom-text-btn"
            :title="t('chat.imageFullscreen.resetZoom')"
            @click="resetToFit"
          >
            {{ isFit ? t('chat.imageFullscreen.fit') : `${Math.round(zoom * 100)}%` }}
          </button>
          <button
            type="button"
            class="fullscreen-action-btn"
            :title="t('chat.imageFullscreen.zoomIn')"
            @click="zoomIn"
          >
            <SvgIcon name="zoom-in" :size="16" />
          </button>
          <button
            type="button"
            class="fullscreen-action-btn"
            :title="t('chat.imageFullscreen.fitToWindow')"
            :class="{ active: isFit }"
            @click="resetToFit"
          >
            <SvgIcon name="maximize-2" :size="15" />
          </button>

          <span class="action-divider" />

          <button
            v-if="isSvg"
            type="button"
            class="fullscreen-action-btn"
            :title="copied ? t('chat.imageFullscreen.copiedSvg') : t('chat.imageFullscreen.copySvg')"
            @click="onCopySvg"
          >
            <SvgIcon :name="copied ? 'check' : 'copy'" :size="16" />
          </button>
          <button
            type="button"
            class="fullscreen-action-btn"
            :title="isSvg ? t('chat.imageFullscreen.downloadSvg') : t('chat.imagePart.downloadImage')"
            @click="$emit('download', imageUrl)"
          >
            <SvgIcon name="download" :size="16" />
          </button>
        </div>
      </div>
    </div>
  </Transition>
</template>

<style scoped>
.modal-backdrop {
  /* 看图需要把背景压得更黑、糊得更重，故覆盖共享 token */
  --overlay-bg: rgba(0, 0, 0, 0.85);
  --overlay-blur: 8px;
}

.fullscreen-close-btn {
  position: absolute;
  top: var(--space-6);
  right: var(--space-6);
  width: 44px;
  height: 44px;
  border-radius: 50%;
  border: none;
  background-color: rgba(255, 255, 255, 0.12);
  color: white;
  display: flex;
  align-items: center;
  justify-content: center;
  cursor: pointer;
  transition: all var(--transition-fast);
  z-index: 1001;
}

.fullscreen-close-btn:hover {
  background-color: rgba(255, 255, 255, 0.25);
  transform: rotate(90deg);
}

.fullscreen-content {
  position: relative;
  width: 92vw;
  max-width: 1400px;
  height: 84vh;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
}

.fullscreen-svg-wrapper {
  width: 100%;
  height: 100%;
  background-color: var(--color-surface, #ffffff);
  border-radius: var(--radius-xl, 16px);
  padding: var(--space-6, 24px);
  box-shadow: 0 24px 64px rgba(0, 0, 0, 0.6);
  border: 1px solid var(--color-border, rgba(255, 255, 255, 0.15));
  overflow: hidden;
  position: relative;
  display: flex;
  align-items: center;
  justify-content: center;
  user-select: none;
}

[data-theme="dark"] .fullscreen-svg-wrapper {
  background-color: var(--color-bg-elevated, #232730);
  border-color: var(--color-border, #3a3f4d);
}

.fullscreen-svg-container {
  width: 100%;
  height: 100%;
  display: flex;
  align-items: center;
  justify-content: center;
  transition: transform var(--transition-fast);
  will-change: transform;
}

.fullscreen-svg-container.is-panning {
  transition: none;
  cursor: grabbing !important;
}

.fullscreen-svg-container.is-zoomed {
  cursor: grab;
}

.fullscreen-svg-container :deep(svg) {
  width: 100%;
  height: 100%;
  max-width: 100%;
  max-height: 100%;
  display: block;
}

.fullscreen-image-wrapper {
  width: 100%;
  height: 100%;
  display: flex;
  align-items: center;
  justify-content: center;
  overflow: hidden;
}

.fullscreen-image {
  max-width: 100%;
  max-height: 100%;
  object-fit: contain;
  border-radius: var(--radius-md);
  box-shadow: 0 8px 32px rgba(0, 0, 0, 0.5);
  transition: transform var(--transition-fast);
  will-change: transform;
  user-select: none;
}

.fullscreen-image.is-panning {
  transition: none;
  cursor: grabbing !important;
}

.fullscreen-image.is-zoomed {
  cursor: grab;
}

.fullscreen-actions {
  position: absolute;
  bottom: -54px;
  display: flex;
  align-items: center;
  gap: 6px;
  background: rgba(24, 26, 32, 0.88);
  backdrop-filter: blur(14px);
  padding: 5px 12px;
  border-radius: var(--radius-full, 9999px);
  border: 1px solid rgba(255, 255, 255, 0.16);
  box-shadow: 0 8px 28px rgba(0, 0, 0, 0.45);
  z-index: 1002;
}

.fullscreen-action-btn {
  width: 32px;
  height: 32px;
  border-radius: 50%;
  border: none;
  background-color: rgba(255, 255, 255, 0.1);
  color: #f1f4f9;
  display: flex;
  align-items: center;
  justify-content: center;
  cursor: pointer;
  transition: all var(--transition-fast);
}

.fullscreen-action-btn:hover {
  background-color: rgba(255, 255, 255, 0.22);
  color: #ffffff;
  transform: translateY(-1px);
}

.fullscreen-action-btn.active {
  background-color: var(--color-primary, #007aff);
  color: white;
}

.fullscreen-zoom-text-btn {
  min-width: 48px;
  height: 28px;
  border-radius: var(--radius-sm, 4px);
  border: none;
  background-color: transparent;
  color: #f1f4f9;
  font-size: var(--text-xs, 12px);
  font-family: var(--font-mono, monospace);
  font-weight: 500;
  display: flex;
  align-items: center;
  justify-content: center;
  cursor: pointer;
  padding: 0 6px;
  transition: all var(--transition-fast);
}

.fullscreen-zoom-text-btn:hover {
  background-color: rgba(255, 255, 255, 0.15);
  color: #ffffff;
}

.action-divider {
  width: 1px;
  height: 18px;
  background: rgba(255, 255, 255, 0.2);
  margin: 0 4px;
}

.fade-enter-active,
.fade-leave-active {
  transition: opacity var(--transition-base);
}
.fade-enter-from,
.fade-leave-to {
  opacity: 0;
}
</style>



<script setup lang="ts">
import { ref, computed, watch, onMounted, onBeforeUnmount } from "vue";
import { convertFileSrc } from "@tauri-apps/api/core";
import { stat } from "@tauri-apps/plugin-fs";
import SvgIcon from "./icons/SvgIcon.vue";
import InteractiveBreadcrumb from "./InteractiveBreadcrumb.vue";
import { t } from "../i18n";

const props = defineProps<{
  filePath: string;
  projectRoot: string;
}>();

const emit = defineEmits<{
  openFile: [path: string, projectRoot: string];
}>();

const imageSrc = computed(() => convertFileSrc(props.filePath));
const imageLoadError = ref(false);
const naturalDimensions = ref<{ width: number; height: number } | null>(null);

// 超大图护栏：按扩展名直接进 WebView 解码没有尺寸上限，数百 MB 的图片
// （或解压炸弹）可能在 @load 触发前就拖垮 WebView。加载前先用 fs.stat 预检，
// 超过阈值直接进错误态；stat 失败（如权限外路径）不阻塞，交给 img onerror 兜底。
const MAX_IMAGE_BYTES = 100 * 1024 * 1024;
const fileTooLarge = ref(false);
let sizeCheckToken = 0;

async function checkFileSize(path: string) {
  const token = ++sizeCheckToken;
  fileTooLarge.value = false;
  try {
    const meta = await stat(path);
    if (token !== sizeCheckToken) return; // 期间已切换文件
    if (meta.size > MAX_IMAGE_BYTES) {
      fileTooLarge.value = true;
    }
  } catch {
    // 忽略：交给 <img> 的加载错误分支兜底
  }
}

// Zoom & Pan state
const zoom = ref(1); // 1 = 100%
const isFit = ref(true);
const panX = ref(0);
const panY = ref(0);
const isPanning = ref(false);
const startPan = ref({ x: 0, y: 0 });

function onImageLoad(e: Event) {
  imageLoadError.value = false;
  const target = e.target as HTMLImageElement;
  naturalDimensions.value = {
    width: target.naturalWidth,
    height: target.naturalHeight,
  };
}

function onImageError() {
  imageLoadError.value = true;
}

function resetToFit() {
  isFit.value = true;
  zoom.value = 1;
  panX.value = 0;
  panY.value = 0;
}

function setZoom(newZoom: number) {
  isFit.value = false;
  zoom.value = Math.max(0.1, Math.min(10, Math.round(newZoom * 100) / 100));
}

function zoomIn() {
  setZoom(zoom.value * 1.25);
}

function zoomOut() {
  setZoom(zoom.value / 1.25);
}

function setOriginalSize() {
  isFit.value = false;
  zoom.value = 1;
  panX.value = 0;
  panY.value = 0;
}

function onWheel(e: WheelEvent) {
  e.preventDefault();
  const delta = e.deltaY < 0 ? 1.15 : 0.85;
  setZoom(zoom.value * delta);
}

function onMouseDown(e: MouseEvent) {
  if (e.button !== 0) return; // Only left click
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
    setOriginalSize();
  } else {
    resetToFit();
  }
}

watch(() => props.filePath, (path) => {
  imageLoadError.value = false;
  naturalDimensions.value = null;
  resetToFit();
  void checkFileSize(path);
});

onMounted(() => {
  window.addEventListener("mouseup", onMouseUp);
  void checkFileSize(props.filePath);
});

onBeforeUnmount(() => {
  window.removeEventListener("mouseup", onMouseUp);
});
</script>

<template>
  <div class="image-preview-wrapper">
    <div class="image-preview-toolbar">
      <InteractiveBreadcrumb
        :filePath="filePath"
        :projectRoot="projectRoot"
        @openFile="(path, root) => emit('openFile', path, root)"
      />

      <div class="toolbar-actions">
        <span v-if="naturalDimensions" class="dimension-badge">
          {{ naturalDimensions.width }} × {{ naturalDimensions.height }} px
        </span>

        <div class="zoom-controls">
          <button
            class="zoom-btn"
            :title="t('common.imagePreview.zoomOut')"
            @click="zoomOut"
          >
            <SvgIcon name="zoom-out" :size="15" />
          </button>

          <button
            class="zoom-btn zoom-text-btn"
            :class="{ active: !isFit && Math.abs(zoom - 1) < 0.01 }"
            :title="t('common.imagePreview.originalSize')"
            @click="setOriginalSize"
          >
            {{ isFit ? t('common.imagePreview.fit') : `${Math.round(zoom * 100)}%` }}
          </button>

          <button
            class="zoom-btn"
            :title="t('common.imagePreview.zoomIn')"
            @click="zoomIn"
          >
            <SvgIcon name="zoom-in" :size="15" />
          </button>

          <button
            class="zoom-btn"
            :class="{ active: isFit }"
            :title="t('common.imagePreview.fitToWindow')"
            @click="resetToFit"
          >
            <SvgIcon name="maximize-2" :size="14" />
          </button>
        </div>
      </div>
    </div>

    <div
      class="image-preview-body checkerboard-bg"
      :class="{ grabbing: isPanning }"
      @wheel="onWheel"
      @mousedown="onMouseDown"
      @mousemove="onMouseMove"
      @dblclick="onDoubleClick"
    >
      <div v-if="imageLoadError" class="image-error-state">
        <SvgIcon name="alert-circle" :size="36" class="error-icon" />
        <div class="error-title">{{ t("common.imagePreview.loadFailed") }}</div>
        <div class="error-desc">{{ filePath }}</div>
      </div>

      <div v-else-if="fileTooLarge" class="image-error-state">
        <SvgIcon name="alert-circle" :size="36" class="error-icon" />
        <div class="error-title">{{ t("common.imagePreview.tooLargeTitle") }}</div>
        <div class="error-desc">{{ t("common.imagePreview.tooLargeDesc") }}</div>
      </div>

      <div
        v-else
        class="image-canvas-container"
        :class="{ 'is-fit': isFit }"
      >
        <img
          :src="imageSrc"
          :alt="filePath"
          class="preview-image"
          :class="{ 'fit-mode': isFit }"
          :style="isFit ? {} : {
            transform: `translate(${panX}px, ${panY}px) scale(${zoom})`,
            transformOrigin: 'center center',
          }"
          draggable="false"
          @load="onImageLoad"
          @error="onImageError"
        />
      </div>
    </div>
  </div>
</template>

<style scoped>
.image-preview-wrapper {
  display: flex;
  flex-direction: column;
  flex: 1;
  min-height: 0;
  background: var(--color-bg);
}

.image-preview-toolbar {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding: 6px 16px;
  border-bottom: 1px solid var(--color-border);
  background: var(--color-bg);
  flex-shrink: 0;
  gap: 16px;
}

:deep(.interactive-breadcrumb) {
  flex: 1;
  min-width: 0;
  padding: 0;
  border-bottom: none;
  background: transparent;
}

.toolbar-actions {
  display: flex;
  align-items: center;
  gap: 12px;
  flex-shrink: 0;
}

.dimension-badge {
  font-size: 11px;
  font-family: var(--font-mono, monospace);
  color: var(--color-text-muted);
  background: var(--color-bg-secondary);
  padding: 3px 8px;
  border-radius: var(--radius-sm);
  border: 1px solid var(--color-border-subtle, var(--color-border));
}

.zoom-controls {
  display: flex;
  align-items: center;
  background: var(--color-bg-secondary);
  border-radius: var(--radius-md);
  padding: 3px;
  gap: 2px;
}

.zoom-btn {
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 5px 7px;
  border-radius: var(--radius-sm);
  color: var(--color-text-muted);
  cursor: pointer;
  border: none;
  background: transparent;
  font-size: 12px;
  transition: all var(--transition-fast);
}

.zoom-btn:hover {
  color: var(--color-text);
  background: var(--color-bg-hover, rgba(128, 128, 128, 0.15));
}

.zoom-btn.active {
  background: var(--color-bg);
  color: var(--color-text);
  box-shadow: 0 1px 3px rgba(0, 0, 0, 0.1);
}

.zoom-text-btn {
  min-width: 48px;
  font-family: var(--font-mono, monospace);
  font-size: 11px;
  font-weight: 500;
}

.image-preview-body {
  flex: 1;
  min-height: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  position: relative;
  overflow: hidden;
  user-select: none;
  cursor: grab;
}

.image-preview-body.grabbing {
  cursor: grabbing;
}

/* Checkered pattern for transparent PNG / SVG */
.checkerboard-bg {
  background-color: var(--color-bg-tertiary, #1e1e1e);
  background-image:
    linear-gradient(45deg, rgba(128, 128, 128, 0.08) 25%, transparent 25%),
    linear-gradient(-45deg, rgba(128, 128, 128, 0.08) 25%, transparent 25%),
    linear-gradient(45deg, transparent 75%, rgba(128, 128, 128, 0.08) 75%),
    linear-gradient(-45deg, transparent 75%, rgba(128, 128, 128, 0.08) 75%);
  background-size: 20px 20px;
  background-position: 0 0, 0 10px, 10px -10px, -10px 0px;
}

.image-canvas-container {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 100%;
  height: 100%;
  padding: 24px;
  box-sizing: border-box;
}

.preview-image {
  box-shadow: 0 4px 20px rgba(0, 0, 0, 0.15);
  border-radius: var(--radius-sm, 4px);
  transition: transform 0.05s ease-out;
}

.preview-image.fit-mode {
  max-width: 100%;
  max-height: 100%;
  object-fit: contain;
  transition: none;
}

.image-error-state {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 12px;
  color: var(--color-text-muted);
  text-align: center;
  padding: 32px;
}

.error-icon {
  color: var(--color-warning, #f59e0b);
}

.error-title {
  font-size: 15px;
  font-weight: 600;
  color: var(--color-text);
}

.error-desc {
  font-size: 12px;
  font-family: var(--font-mono, monospace);
  word-break: break-all;
  max-width: 480px;
}
</style>

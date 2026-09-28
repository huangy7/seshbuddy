<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref } from "vue";
import { invokeApp, renderAppError } from "../utils/invokeApp";
import SvgIcon from "./icons/SvgIcon.vue";
import { t } from "../i18n";

interface Milestone {
  pos: number;      // 0 ~ 1000
  days: number;     // 天数，-1 表示永久
  label: string;
}

// 标尺文案在 computed 内求值：写成模块级常量会在 import 时求值一次，
// 之后切换语言时不会重算，刻度标签就停在旧语言。
const milestones = computed<Milestone[]>(() => [
  { pos: 0,    days: 0,    label: t("settings.archive.milestoneImmediate") },
  { pos: 220,  days: 90,   label: t("settings.archive.milestone90d") },
  { pos: 440,  days: 365,  label: t("settings.archive.milestone1y") },
  { pos: 640,  days: 1095, label: t("settings.archive.milestone3y") },
  { pos: 840,  days: 3650, label: t("settings.archive.milestone10y") },
  { pos: 1000, days: -1,   label: t("settings.archive.milestoneForever") },
]);

// 双向非线性映射：0~1年占前44%，1~3年占20%，3~10年占20%，>840为10年到永久
function daysToPos(days: number): number {
  if (days < 0 || days > 3650) return 1000;
  if (days === 0) return 0;
  if (days <= 90) {
    return Math.round((days / 90) * 220);
  }
  if (days <= 365) {
    return Math.round(220 + ((days - 90) / (365 - 90)) * (440 - 220));
  }
  if (days <= 1095) {
    return Math.round(440 + ((days - 365) / (1095 - 365)) * (640 - 440));
  }
  if (days <= 3650) {
    return Math.round(640 + ((days - 1095) / (3650 - 1095)) * (840 - 640));
  }
  return 1000;
}

function posToDays(pos: number): number {
  if (pos <= 0) return 0;
  if (pos >= 960) return -1; // 顶部吸附为永久保留
  if (pos <= 220) {
    return Math.max(0, Math.min(90, Math.round((pos / 220) * 90)));
  }
  if (pos <= 440) {
    const ratio = (pos - 220) / (440 - 220);
    return Math.round(90 + ratio * (365 - 90));
  }
  if (pos <= 640) {
    const ratio = (pos - 440) / (640 - 440);
    const raw = 365 + ratio * (1095 - 365);
    return Math.round(raw / 7) * 7; // 按周步长
  }
  if (pos <= 840) {
    const ratio = (pos - 640) / (840 - 640);
    const raw = 1095 + ratio * (3650 - 1095);
    return Math.round(raw / 30) * 30; // 按月步长
  }
  // 840 ~ 960 之间过渡到 10 年
  return 3650;
}

const sliderPos = ref(daysToPos(60));
const savedDays = ref(60);
const isDragging = ref(false);
const isAnimating = ref(false);
const loading = ref(false);
const feedback = ref("");
const feedbackType = ref<"success" | "error" | "">("");
const isDirectEditing = ref(false);
const directInputVal = ref("");
const directInputRef = ref<HTMLInputElement | null>(null);
let feedbackTimer: ReturnType<typeof window.setTimeout> | undefined;
let animTimer: ReturnType<typeof window.setTimeout> | undefined;

const currentDays = computed(() => posToDays(sliderPos.value));

const isForever = computed(() => currentDays.value === -1);

// 滑动比例 (0 ~ 1)
const sliderRatio = computed(() => {
  return Math.min(1, Math.max(0, sliderPos.value / 1000));
});

// 手柄真实中心物理位置（原生 range thumb 宽度为 18px，中心从 9px 到 calc(100% - 9px)）
const thumbPosStyle = computed(() => {
  return `calc(9px + (100% - 18px) * ${sliderRatio.value})`;
});

// 气泡水平位移：0% 时向右展开 (translateX(0%))，100% 时向左展开 (translateX(-100%))，50% 时居中
// 彻底杜绝左右两端截断溢出！
const tooltipTranslateX = computed(() => {
  return `-${sliderRatio.value * 100}%`;
});

// 气泡下方指向手柄的小箭头位置：始终精准指向手柄中心
const tooltipArrowLeft = computed(() => {
  return `clamp(8px, ${sliderRatio.value * 100}%, calc(100% - 8px))`;
});

const currentBadgeText = computed(() => {
  const days = currentDays.value;
  if (days === -1) return t("settings.archive.milestoneForever");
  if (days === 0) return t("settings.archive.milestoneImmediate");
  if (days < 365) return t("settings.archive.daysCount", { count: days });
  const years = (days / 365).toFixed(1).replace(/\.0$/, "");
  return t("settings.archive.yearsDays", { years, days });
});

const floatingTooltipText = computed(() => {
  const days = currentDays.value;
  if (days === -1) return t("settings.archive.tooltipForever");
  if (days === 0) return t("settings.archive.tooltipImmediate");
  if (days < 365) return t("settings.archive.daysCount", { count: days });
  const years = (days / 365).toFixed(1).replace(/\.0$/, "");
  return t("settings.archive.yearsDays", { years, days });
});

const currentHint = computed(() => {
  const days = currentDays.value;
  if (days === -1) {
    return t("settings.archive.hintForever");
  }
  if (days === 0) {
    return t("settings.archive.hintImmediate");
  }
  if (days >= 365) {
    const years = (days / 365).toFixed(1).replace(/\.0$/, "");
    return t("settings.archive.hintYears", { years, days });
  }
  return t("settings.archive.hintDays", { days });
});

function clearFeedback() {
  feedback.value = "";
  feedbackType.value = "";
  if (feedbackTimer) {
    window.clearTimeout(feedbackTimer);
    feedbackTimer = undefined;
  }
}

function setFeedback(type: "success" | "error", text: string) {
  feedback.value = text;
  feedbackType.value = type;
  if (feedbackTimer) window.clearTimeout(feedbackTimer);
  feedbackTimer = window.setTimeout(() => {
    feedback.value = "";
    feedbackType.value = "";
    feedbackTimer = undefined;
  }, 2500);
}

async function loadArchiveRetention() {
  try {
    const days = await invokeApp<number>("get_archive_retention_days");
    savedDays.value = days;
    sliderPos.value = daysToPos(days);
  } catch (e: any) {
    console.error("get_archive_retention_days failed:", e);
  }
}

async function saveRetention(days: number) {
  if (days === savedDays.value) return;
  loading.value = true;
  clearFeedback();
  try {
    await invokeApp("set_archive_retention_days", { days });
    savedDays.value = days;
    setFeedback("success", t("api-profile.rawEditor.saved"));
  } catch (e: any) {
    console.error("set_archive_retention_days failed:", e);
    setFeedback("error", renderAppError(e));
  } finally {
    loading.value = false;
  }
}

function onSliderInput(e: Event) {
  const val = Number((e.target as HTMLInputElement).value);
  isAnimating.value = false;
  sliderPos.value = val;
  isDragging.value = true;
}

function onSliderChange() {
  isDragging.value = false;
  void saveRetention(currentDays.value);
}

function jumpToMilestone(m: Milestone) {
  isAnimating.value = true;
  if (animTimer) window.clearTimeout(animTimer);
  animTimer = window.setTimeout(() => {
    isAnimating.value = false;
  }, 240);

  sliderPos.value = m.pos;
  void saveRetention(m.days);
}

function isMilestoneActive(m: Milestone): boolean {
  if (m.days === -1) return currentDays.value === -1;
  if (m.days === 0) return currentDays.value === 0;
  return Math.abs(currentDays.value - m.days) < 20;
}

function startDirectEditing() {
  directInputVal.value = currentDays.value === -1 ? "-1" : String(currentDays.value);
  isDirectEditing.value = true;
  void nextTick(() => {
    directInputRef.value?.focus();
    directInputRef.value?.select();
  });
}

async function commitDirectInput() {
  if (!isDirectEditing.value) return;
  isDirectEditing.value = false;
  let val = Math.trunc(Number(directInputVal.value));
  if (Number.isNaN(val)) return;
  if (val < 0) val = -1;
  else if (val > 3650) val = -1;
  isAnimating.value = true;
  sliderPos.value = daysToPos(val);
  setTimeout(() => { isAnimating.value = false; }, 240);
  await saveRetention(val);
}

onMounted(() => {
  void loadArchiveRetention();
});

onBeforeUnmount(() => {
  clearFeedback();
  if (animTimer) window.clearTimeout(animTimer);
});
</script>

<template>
  <div class="retention-settings">
    <!-- 头部信息 -->
    <div class="retention-header">
      <div class="header-content">
        <div class="title-row">
          <span class="row-title">{{ t("settings.archive.title") }}</span>
          <Transition name="fade">
            <span v-if="feedback" class="inline-feedback" :class="feedbackType">
              <SvgIcon v-if="feedbackType === 'success'" name="check" :size="13" />
              <SvgIcon v-else-if="feedbackType === 'error'" name="alert-circle" :size="13" />
              {{ feedback }}
            </span>
          </Transition>
        </div>
        <p class="row-hint">{{ currentHint }}</p>
      </div>

      <!-- 右侧数值徽标（支持点击直接微调输入） -->
      <div class="badge-container">
        <div v-if="isDirectEditing" class="direct-input-wrap">
          <input
            ref="directInputRef"
            v-model="directInputVal"
            type="number"
            class="direct-input"
            :aria-label="t('settings.archive.inputAria')"
            :placeholder="t('settings.archive.inputPlaceholder')"
            @keydown.enter="commitDirectInput"
            @blur="commitDirectInput"
          />
          <span class="direct-input-unit">{{ t("settings.archive.unitDays") }}</span>
        </div>
        <button
          v-else
          type="button"
          class="retention-badge"
          :class="{ forever: isForever }"
          :title="t('settings.archive.badgeTitle')"
          @click="startDirectEditing"
        >
          <span>{{ currentBadgeText }}</span>
        </button>
      </div>
    </div>

    <!-- 流光无极滑动条区 -->
    <div class="slider-interactive-box">
      <!-- 动态悬浮跟随气泡：平滑防溢出定位与对齐 -->
      <Transition name="tooltip-fade">
        <div
          v-if="isDragging"
          class="floating-tooltip"
          :style="{
            left: thumbPosStyle,
            transform: `translateX(${tooltipTranslateX})`,
          }"
        >
          <span>{{ floatingTooltipText }}</span>
          <div
            class="tooltip-arrow"
            :style="{ left: tooltipArrowLeft }"
          />
        </div>
      </Transition>

      <div class="slider-track-wrap">
        <!-- 底轨背景 -->
        <div class="track-bg" />

        <!-- 填充高光轨（渐变）：精确对齐手柄真实中心，拖动零延迟，点击跳转带过渡 -->
        <div
          class="track-fill"
          :class="{ forever: isForever, 'is-animating': isAnimating }"
          :style="{ width: thumbPosStyle }"
        />

        <!-- 里程碑内嵌刻度微点（Pips）：绝对几何对齐 -->
        <div
          v-for="m in milestones"
          :key="m.pos"
          class="track-pip"
          :class="{ active: sliderPos >= m.pos }"
          :style="{ left: `calc(9px + (100% - 18px) * (${m.pos} / 1000))` }"
        />

        <!-- 原生 range 控件（透明覆盖在上层以获得完全无障碍与原生拖动手感） -->
        <input
          type="range"
          class="range-slider-input"
          min="0"
          max="1000"
          step="1"
          :value="sliderPos"
          :disabled="loading"
          :aria-label="t('settings.archive.sliderAria')"
          @input="onSliderInput"
          @change="onSliderChange"
          @pointerdown="isDragging = true; isAnimating = false"
          @pointerup="isDragging = false"
        />
      </div>

      <!-- 底部刻度标尺（点击可直接瞬移跳转） -->
      <div class="milestones-row">
        <button
          v-for="m in milestones"
          :key="m.pos"
          type="button"
          class="milestone-btn"
          :class="{ active: isMilestoneActive(m), forever: m.days === -1 }"
          @click="jumpToMilestone(m)"
        >
          {{ m.label }}
        </button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.retention-settings {
  display: flex;
  flex-direction: column;
  gap: var(--space-4, 16px);
  padding: var(--space-4, 16px) var(--space-5, 20px) var(--space-5, 20px);
}

.retention-header {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: var(--space-3, 12px);
}

.header-content {
  display: flex;
  flex-direction: column;
  gap: 4px;
  flex: 1;
  min-width: 0;
}

.title-row {
  display: flex;
  align-items: center;
  gap: 8px;
}

.row-title {
  font-size: var(--text-sm, 13px);
  font-weight: 500;
  color: var(--color-text);
}

.row-hint {
  font-size: var(--text-xs, 12px);
  color: var(--color-text-muted);
  line-height: 1.5;
  margin: 0;
}

.inline-feedback {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  font-size: 11.5px;
  font-weight: 500;
}

.inline-feedback.success {
  color: var(--color-success, #22c55e);
}

.inline-feedback.error {
  color: var(--color-danger, #ef4444);
}

.badge-container {
  flex-shrink: 0;
  display: flex;
  align-items: center;
}

.retention-badge {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  padding: 3px 10px;
  border-radius: var(--radius-full, 9999px);
  background: var(--color-bg-tertiary, rgba(255, 255, 255, 0.08));
  color: var(--color-primary, #6366f1);
  font-size: 12px;
  font-weight: 600;
  line-height: 1.4;
  white-space: nowrap;
  border: 1px solid var(--color-border);
  cursor: pointer;
  transition: all var(--transition-fast, 120ms ease);
}

.retention-badge:hover {
  background: var(--color-bg-hover);
  border-color: var(--color-border-hover, var(--color-border));
}

.retention-badge.forever {
  background: rgba(168, 85, 247, 0.12);
  color: #a855f7;
  border-color: rgba(168, 85, 247, 0.3);
}

[data-theme="dark"] .retention-badge.forever {
  background: rgba(168, 85, 247, 0.2);
  color: #c084fc;
}

.direct-input-wrap {
  display: inline-flex;
  align-items: center;
  position: relative;
}

.direct-input {
  width: 90px;
  padding: 2px 24px 2px 8px;
  font-size: 12px;
  font-weight: 500;
  border-radius: var(--radius-md, 6px);
  border: 1px solid var(--color-primary, #6366f1);
  background: var(--color-bg);
  color: var(--color-text);
  outline: none;
}

.direct-input-unit {
  position: absolute;
  right: 8px;
  font-size: 11px;
  color: var(--color-text-muted);
  pointer-events: none;
}

/* 滑动条交互盒 */
.slider-interactive-box {
  position: relative;
  display: flex;
  flex-direction: column;
  gap: var(--space-2, 8px);
  padding: 16px 2px 4px;
}

/* 浮动跟随气泡：带圆润微投影，位置精准防溢出 */
.floating-tooltip {
  position: absolute;
  top: -14px;
  background: var(--color-text, #1e293b);
  color: var(--color-bg, #ffffff);
  padding: 3px 8px;
  border-radius: var(--radius-md, 6px);
  font-size: 11px;
  font-weight: 600;
  white-space: nowrap;
  pointer-events: none;
  box-shadow: var(--shadow-md, 0 4px 12px rgba(0, 0, 0, 0.18));
  z-index: 10;
  will-change: left, transform;
}

.tooltip-arrow {
  position: absolute;
  bottom: -4px;
  transform: translateX(-50%);
  width: 0;
  height: 0;
  border-left: 4px solid transparent;
  border-right: 4px solid transparent;
  border-top: 4px solid var(--color-text, #1e293b);
  will-change: left;
}

.slider-track-wrap {
  position: relative;
  width: 100%;
  height: 20px;
  display: flex;
  align-items: center;
}

.track-bg {
  position: absolute;
  left: 0;
  right: 0;
  height: 6px;
  border-radius: 9999px;
  background: var(--color-bg-tertiary, rgba(255, 255, 255, 0.12));
  transition: background var(--transition-fast, 120ms ease);
}

.track-fill {
  position: absolute;
  left: 0;
  height: 6px;
  border-radius: 9999px;
  background: linear-gradient(90deg, #6366f1, #818cf8);
  pointer-events: none;
  /* 默认拖动时 0 延迟跟手，杜绝脱节 */
  transition: none;
  will-change: width;
}

.track-fill.is-animating {
  transition: width 200ms cubic-bezier(0.16, 1, 0.3, 1);
}

.track-fill.forever {
  background: linear-gradient(90deg, #6366f1, #c084fc);
  box-shadow: 0 0 10px rgba(192, 132, 252, 0.35);
}

.track-pip {
  position: absolute;
  top: 50%;
  transform: translate(-50%, -50%);
  width: 4px;
  height: 4px;
  border-radius: 50%;
  background: rgba(255, 255, 255, 0.35);
  pointer-events: none;
  transition: all var(--transition-fast, 120ms ease);
  z-index: 2;
}

.track-pip.active {
  background: #ffffff;
  box-shadow: 0 0 4px rgba(255, 255, 255, 0.8);
}

.range-slider-input {
  position: absolute;
  left: 0;
  top: 0;
  width: 100%;
  height: 100%;
  margin: 0;
  opacity: 1;
  -webkit-appearance: none;
  appearance: none;
  background: transparent;
  cursor: grab;
  z-index: 5;
}

.range-slider-input:active {
  cursor: grabbing;
}

.range-slider-input::-webkit-slider-runnable-track {
  height: 100%;
  background: transparent;
}

.range-slider-input::-moz-range-track {
  height: 100%;
  background: transparent;
}

.range-slider-input::-webkit-slider-thumb {
  -webkit-appearance: none;
  appearance: none;
  width: 18px;
  height: 18px;
  border-radius: 50%;
  background: #ffffff;
  border: 2px solid #6366f1;
  box-shadow: 0 1px 5px rgba(0, 0, 0, 0.28);
  cursor: grab;
  margin-top: 1px;
  transition: transform 100ms ease, box-shadow 100ms ease;
}

.range-slider-input:active::-webkit-slider-thumb {
  cursor: grabbing;
  transform: scale(1.22);
  box-shadow: 0 0 0 5px rgba(99, 102, 241, 0.25);
}

.range-slider-input::-moz-range-thumb {
  width: 18px;
  height: 18px;
  border-radius: 50%;
  background: #ffffff;
  border: 2px solid #6366f1;
  box-shadow: 0 1px 5px rgba(0, 0, 0, 0.28);
  cursor: grab;
  transition: transform 100ms ease, box-shadow 100ms ease;
}

.range-slider-input:active::-moz-range-thumb {
  cursor: grabbing;
  transform: scale(1.22);
  box-shadow: 0 0 0 5px rgba(99, 102, 241, 0.25);
}

/* 底部标尺按钮行 */
.milestones-row {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding: 0 2px;
}

.milestone-btn {
  background: transparent;
  border: none;
  padding: 2px 4px;
  font-size: 11px;
  font-weight: 500;
  color: var(--color-text-muted);
  cursor: pointer;
  border-radius: var(--radius-sm, 4px);
  transition: all var(--transition-fast, 120ms ease);
}

.milestone-btn:hover {
  color: var(--color-text);
  background: var(--color-bg-hover);
}

.milestone-btn.active {
  color: var(--color-primary, #6366f1);
  font-weight: 600;
}

.milestone-btn.forever.active {
  color: #a855f7;
}

[data-theme="dark"] .milestone-btn.forever.active {
  color: #c084fc;
}

/* 动画过渡 */
.tooltip-fade-enter-active,
.tooltip-fade-leave-active {
  transition: opacity 120ms ease, transform 120ms ease;
}

.tooltip-fade-enter-from,
.tooltip-fade-leave-to {
  opacity: 0;
  transform: translateY(4px);
}

.fade-enter-active,
.fade-leave-active {
  transition: opacity 150ms ease;
}

.fade-enter-from,
.fade-leave-to {
  opacity: 0;
}
</style>

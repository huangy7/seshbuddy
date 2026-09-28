import { ref, readonly } from "vue";

export const COLLAPSED_WIDTH = 48;
export const DEFAULT_EXPANDED_WIDTH = 160;
export const MIN_EXPANDED_WIDTH = 120;
export const MAX_EXPANDED_WIDTH = 260;
export const SNAP_COLLAPSE_THRESHOLD = 100;

export const STORAGE_EXPANDED_KEY = "seshbuddy-activity-bar-expanded";
export const STORAGE_WIDTH_KEY = "seshbuddy-activity-bar-width";

function readStorage(): { expanded: boolean; width: number } {
  try {
    const savedExpanded = typeof localStorage !== "undefined" && localStorage.getItem(STORAGE_EXPANDED_KEY) === "true";
    const rawSavedWidth = typeof localStorage !== "undefined" ? Number(localStorage.getItem(STORAGE_WIDTH_KEY)) : NaN;
    const savedWidth = (!isNaN(rawSavedWidth) && rawSavedWidth >= MIN_EXPANDED_WIDTH)
      ? rawSavedWidth
      : DEFAULT_EXPANDED_WIDTH;
    return {
      expanded: savedExpanded,
      width: Math.min(MAX_EXPANDED_WIDTH, Math.max(MIN_EXPANDED_WIDTH, savedWidth)),
    };
  } catch {
    return { expanded: false, width: DEFAULT_EXPANDED_WIDTH };
  }
}

const initial = readStorage();
const isExpanded = ref<boolean>(initial.expanded);
const lastExpandedWidth = ref<number>(initial.width);
const railWidth = ref<number>(initial.expanded ? initial.width : COLLAPSED_WIDTH);

function persist() {
  try {
    localStorage.setItem(STORAGE_EXPANDED_KEY, String(isExpanded.value));
    localStorage.setItem(STORAGE_WIDTH_KEY, String(lastExpandedWidth.value));
  } catch {}
}

export function resetActivityBarStateForTest() {
  const current = readStorage();
  isExpanded.value = current.expanded;
  lastExpandedWidth.value = current.width;
  railWidth.value = current.expanded ? current.width : COLLAPSED_WIDTH;
}

export function useActivityBar() {
  resetActivityBarStateForTest();

  function setExpanded(val: boolean) {
    isExpanded.value = val;
    railWidth.value = val ? lastExpandedWidth.value : COLLAPSED_WIDTH;
    persist();
  }

  function toggleExpanded() {
    setExpanded(!isExpanded.value);
  }

  function setRailWidth(width: number) {
    if (width < SNAP_COLLAPSE_THRESHOLD) {
      isExpanded.value = false;
      railWidth.value = COLLAPSED_WIDTH;
    } else {
      const clamped = Math.min(MAX_EXPANDED_WIDTH, Math.max(MIN_EXPANDED_WIDTH, width));
      isExpanded.value = true;
      lastExpandedWidth.value = clamped;
      railWidth.value = clamped;
    }
    persist();
  }

  function startResize(e: MouseEvent) {
    e.preventDefault();
    const startX = e.clientX;
    const startWidth = railWidth.value;

    function onMouseMove(moveEvent: MouseEvent) {
      const delta = moveEvent.clientX - startX;
      setRailWidth(startWidth + delta);
    }

    function onMouseUp() {
      document.removeEventListener("mousemove", onMouseMove);
      document.removeEventListener("mouseup", onMouseUp);
    }

    document.addEventListener("mousemove", onMouseMove);
    document.addEventListener("mouseup", onMouseUp);
  }

  return {
    isExpanded: readonly(isExpanded),
    railWidth: readonly(railWidth),
    setExpanded,
    toggleExpanded,
    setRailWidth,
    startResize,
  };
}

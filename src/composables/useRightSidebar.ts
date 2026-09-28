import { ref, watch } from "vue";

export type RightSidebarMode = "project" | "git" | "timeline";

export const STORAGE_KEY_RIGHT_SIDEBAR_OPEN = "seshbuddy-right-sidebar-open";
export const STORAGE_KEY_RIGHT_SIDEBAR_MODE = "seshbuddy-right-sidebar-mode";
export const STORAGE_KEY_RIGHT_SIDEBAR_WIDTH = "seshbuddy-right-sidebar-width";

export const DEFAULT_RIGHT_SIDEBAR_WIDTH = 280;
export const MIN_RIGHT_SIDEBAR_WIDTH = 200;
export const MAX_RIGHT_SIDEBAR_WIDTH = 600;

function parseStoredMode(raw: string | null): RightSidebarMode {
  if (raw === "project" || raw === "git" || raw === "timeline") {
    return raw;
  }
  return "project";
}

function parseStoredWidth(raw: string | null): number {
  if (!raw) return DEFAULT_RIGHT_SIDEBAR_WIDTH;
  const num = Number(raw);
  if (Number.isNaN(num) || num <= 0) return DEFAULT_RIGHT_SIDEBAR_WIDTH;
  return Math.max(MIN_RIGHT_SIDEBAR_WIDTH, Math.min(num, MAX_RIGHT_SIDEBAR_WIDTH));
}

export function useRightSidebar() {
  const rightSidebarOpen = ref<boolean>(
    typeof localStorage !== "undefined"
      ? localStorage.getItem(STORAGE_KEY_RIGHT_SIDEBAR_OPEN) === "1"
      : false,
  );

  const rightSidebarMode = ref<RightSidebarMode>(
    typeof localStorage !== "undefined"
      ? parseStoredMode(localStorage.getItem(STORAGE_KEY_RIGHT_SIDEBAR_MODE))
      : "project",
  );

  const rightSidebarWidth = ref<number>(
    typeof localStorage !== "undefined"
      ? parseStoredWidth(localStorage.getItem(STORAGE_KEY_RIGHT_SIDEBAR_WIDTH))
      : DEFAULT_RIGHT_SIDEBAR_WIDTH,
  );

  const isRightResizing = ref(false);

  watch(rightSidebarOpen, (val) => {
    if (typeof localStorage !== "undefined") {
      localStorage.setItem(STORAGE_KEY_RIGHT_SIDEBAR_OPEN, val ? "1" : "0");
    }
  });

  watch(rightSidebarMode, (val) => {
    if (typeof localStorage !== "undefined") {
      localStorage.setItem(STORAGE_KEY_RIGHT_SIDEBAR_MODE, val);
    }
  });

  watch(rightSidebarWidth, (val) => {
    if (typeof localStorage !== "undefined") {
      localStorage.setItem(STORAGE_KEY_RIGHT_SIDEBAR_WIDTH, String(val));
    }
  });

  function toggleRightSidebar(mode?: RightSidebarMode) {
    if (mode) {
      if (!rightSidebarOpen.value) {
        rightSidebarOpen.value = true;
        rightSidebarMode.value = mode;
      } else if (rightSidebarMode.value === mode) {
        rightSidebarOpen.value = false;
      } else {
        rightSidebarMode.value = mode;
      }
    } else {
      rightSidebarOpen.value = !rightSidebarOpen.value;
    }
  }

  function openRightSidebar(mode?: RightSidebarMode) {
    rightSidebarOpen.value = true;
    if (mode) {
      rightSidebarMode.value = mode;
    }
  }

  function closeRightSidebar() {
    rightSidebarOpen.value = false;
  }

  function setRightSidebarMode(mode: RightSidebarMode) {
    rightSidebarMode.value = mode;
  }

  function startRightResize(e: MouseEvent) {
    isRightResizing.value = true;
    if (typeof document !== "undefined") {
      document.body.style.userSelect = "none";
    }
    const startX = e.clientX;
    const startWidth = rightSidebarWidth.value;

    const onMove = (ev: MouseEvent) => {
      // Dragging handle to the left increases width; dragging to the right decreases width
      const newWidth = startWidth + (startX - ev.clientX);
      rightSidebarWidth.value = Math.max(
        MIN_RIGHT_SIDEBAR_WIDTH,
        Math.min(newWidth, MAX_RIGHT_SIDEBAR_WIDTH),
      );
    };

    const onUp = () => {
      isRightResizing.value = false;
      if (typeof document !== "undefined") {
        document.body.style.userSelect = "";
      }
      document.removeEventListener("mousemove", onMove);
      document.removeEventListener("mouseup", onUp);
    };

    document.addEventListener("mousemove", onMove);
    document.addEventListener("mouseup", onUp);
  }

  return {
    rightSidebarOpen,
    rightSidebarMode,
    rightSidebarWidth,
    isRightResizing,
    toggleRightSidebar,
    openRightSidebar,
    closeRightSidebar,
    setRightSidebarMode,
    startRightResize,
  };
}

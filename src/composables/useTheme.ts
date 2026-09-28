import { ref, computed, watch } from "vue";
import { emit, listen } from "@tauri-apps/api/event";

export type Theme = "light" | "dark" | "system";

const theme = ref<Theme>(
  (localStorage.getItem("seshbuddy-theme") as Theme) || "system"
);

function getSystemPrefersDark(): boolean {
  if (typeof window === "undefined" || typeof window.matchMedia !== "function") {
    return false;
  }
  return window.matchMedia("(prefers-color-scheme: dark)").matches;
}

function applyTheme() {
  if (typeof document === "undefined") return;
  const isDark =
    theme.value === "dark" ||
    (theme.value === "system" && getSystemPrefersDark());
  document.documentElement.setAttribute(
    "data-theme",
    isDark ? "dark" : "light"
  );
}

// Watch for changes and broadcast
watch(theme, (val) => {
  localStorage.setItem("seshbuddy-theme", val);
  applyTheme();
  try {
    void emit("app-theme-changed", val);
  } catch {
    // ignore in environments where Tauri IPC is unavailable
  }
});

// Listen to cross-window theme broadcasts
try {
  void listen<Theme>("app-theme-changed", (event) => {
    if (event.payload && theme.value !== event.payload) {
      theme.value = event.payload;
      applyTheme();
    }
  });
} catch {
  // ignore in environments where Tauri IPC is unavailable
}

// Backup: Listen to storage event for same-origin WebViews
if (typeof window !== "undefined") {
  window.addEventListener("storage", (e) => {
    if (e.key === "seshbuddy-theme" && e.newValue) {
      theme.value = e.newValue as Theme;
      applyTheme();
    }
  });
}

// Listen to system preference changes
let systemDarkMql: MediaQueryList | null = null;
const systemThemeHandler = () => {
  if (theme.value === "system") {
    applyTheme();
  }
};
if (typeof window !== "undefined" && typeof window.matchMedia === "function") {
  systemDarkMql = window.matchMedia("(prefers-color-scheme: dark)");
  systemDarkMql.addEventListener?.("change", systemThemeHandler);
}

// Apply on load
applyTheme();

export function useTheme() {
  const resolvedTheme = computed(() => {
    if (theme.value === "system") {
      return getSystemPrefersDark() ? "dark" : "light";
    }
    return theme.value;
  });

  function cycleTheme() {
    const order: Theme[] = ["system", "light", "dark"];
    const idx = order.indexOf(theme.value);
    theme.value = order[(idx + 1) % order.length];
  }

  return { theme, resolvedTheme, cycleTheme };
}

import { computed, ref } from "vue";
import { invokeApp } from "../utils/invokeApp";

export const TERMINAL_APP_STORAGE_KEY = "seshbuddy-terminal-app";

export const TERMINAL_APP_LABELS: Record<string, string> = {
  iterm2: "iTerm2",
  terminal: "Terminal.app",
  "windows-terminal": "Windows Terminal",
  powershell: "PowerShell",
  cmd: "CMD",
};

// 单例：全应用共享同一份探测结果与选择
const installedApps = ref<string[]>([]);
const savedChoice = ref<string | null>(loadSavedChoice());
let detectPromise: Promise<void> | null = null;

function loadSavedChoice(): string | null {
  try {
    return localStorage.getItem(TERMINAL_APP_STORAGE_KEY);
  } catch {
    return null;
  }
}

/**
 * 后端 detect_terminal_apps 按推荐顺序返回已安装终端：
 * macOS: iterm2 → terminal；Windows: windows-terminal → powershell → cmd。
 * 取第一项即「装了 iTerm/wt 则默认它，否则系统自带」，与历史自动探测行为一致。
 */
async function detectTerminalApps(): Promise<void> {
  if (!detectPromise) {
    detectPromise = (async () => {
      try {
        installedApps.value = await invokeApp<string[]>("detect_terminal_apps");
      } catch {
        installedApps.value = [];
      }
      // 已选值失效（如 iTerm 被卸载）→ 清掉存储，落到默认
      if (savedChoice.value && !installedApps.value.includes(savedChoice.value)) {
        savedChoice.value = null;
        try {
          localStorage.removeItem(TERMINAL_APP_STORAGE_KEY);
        } catch {
          // ignore
        }
      }
    })();
  }
  return detectPromise;
}

export function useTerminalApp() {
  /** 设置页展示用：已保存且仍有效 → 保存值，否则默认（探测列表第一项） */
  const terminalApp = computed<string | null>(() => {
    const saved = savedChoice.value;
    if (saved && installedApps.value.includes(saved)) {
      return saved;
    }
    return installedApps.value[0] ?? null;
  });

  /**
   * 启动/注册时传给后端的值：直接传用户原始选择。
   * 选择已失效时后端会自行回退（见 terminal.rs），无需等前端探测完成。
   */
  const terminalAppForLaunch = computed<string | null>(() => savedChoice.value);

  const terminalAppOptions = computed(() =>
    installedApps.value.map((id) => ({
      value: id,
      label: TERMINAL_APP_LABELS[id] ?? id,
    })),
  );

  function setTerminalApp(value: string) {
    savedChoice.value = value;
    try {
      localStorage.setItem(TERMINAL_APP_STORAGE_KEY, value);
    } catch {
      // ignore
    }
  }

  return {
    terminalApp,
    terminalAppForLaunch,
    terminalAppOptions,
    installedApps,
    detectTerminalApps,
    setTerminalApp,
  };
}

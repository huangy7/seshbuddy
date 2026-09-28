<script setup lang="ts">
import { ref, onMounted, onBeforeUnmount, watch } from "vue";
import { Terminal } from "@xterm/xterm";
import type { IDisposable, ILink } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { WebglAddon } from "@xterm/addon-webgl";
import { CanvasAddon } from "@xterm/addon-canvas";
import { invokeApp } from "../utils/invokeApp";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { useTheme } from "../composables/useTheme";
import type { PtyFileLinkPayload } from "../types/pty";
import "@xterm/xterm/css/xterm.css";

const props = defineProps<{
  sessionId: string;
  projectRoot?: string;
}>();

const emit = defineEmits<{
  openFile: [path: string, projectRoot: string];
}>();

const { resolvedTheme } = useTheme();

const containerRef = ref<HTMLDivElement>();
let terminal: Terminal | null = null;
let fitAddon: FitAddon | null = null;
let unlisten: UnlistenFn | null = null;
let fileLinkUnlisten: UnlistenFn | null = null;
let linkProviderDisposable: IDisposable | null = null;
let resizeObserver: ResizeObserver | null = null;
let resizeDebounceTimer: ReturnType<typeof setTimeout> | null = null;
const fileLinkHints: PtyFileLinkPayload[] = [];

function getThemeColors(isDark: boolean) {
  if (isDark) {
    return {
      background: "#0d0d0d",
      foreground: "#d4d4d4",
      cursor: "#aeafad",
      cursorAccent: "#0d0d0d",
      selectionBackground: "#264f78",
      black: "#000000",
      red: "#cd3131",
      green: "#0dbc79",
      yellow: "#e5e510",
      blue: "#2472c8",
      magenta: "#bc3fbc",
      cyan: "#11a8cd",
      white: "#e5e5e5",
      brightBlack: "#666666",
      brightRed: "#f14c4c",
      brightGreen: "#23d18b",
      brightYellow: "#f5f543",
      brightBlue: "#3b8eea",
      brightMagenta: "#d670d6",
      brightCyan: "#29b8db",
      brightWhite: "#e5e5e5",
    };
  } else {
    return {
      background: "#1e1e1e",
      foreground: "#cccccc",
      cursor: "#aeafad",
      cursorAccent: "#1e1e1e",
      selectionBackground: "#264f78",
      black: "#000000",
      red: "#cd3131",
      green: "#0dbc79",
      yellow: "#e5e510",
      blue: "#2472c8",
      magenta: "#bc3fbc",
      cyan: "#11a8cd",
      white: "#e5e5e5",
      brightBlack: "#666666",
      brightRed: "#f14c4c",
      brightGreen: "#23d18b",
      brightYellow: "#f5f543",
      brightBlue: "#3b8eea",
      brightMagenta: "#d670d6",
      brightCyan: "#29b8db",
      brightWhite: "#e5e5e5",
    };
  }
}

function retryFocus(attempts = 0) {
  if (!terminal || attempts >= 20) return;
  const el = containerRef.value;
  if (!el || el.offsetWidth === 0 || el.offsetHeight === 0) {
    setTimeout(() => retryFocus(attempts + 1), 100);
    return;
  }
  terminal.focus();
  const textarea = el.querySelector("textarea");
  if (document.activeElement !== textarea) {
    setTimeout(() => retryFocus(attempts + 1), 100);
  }
}

function actionLabel(action: PtyFileLinkPayload["action"]): string {
  switch (action) {
    case "read":
      return "read";
    case "write":
      return "wrote";
    case "edit":
      return "edited";
  }
}

function rememberFileLinkHint(payload: PtyFileLinkPayload): {
  hint: PtyFileLinkPayload;
  isNew: boolean;
} {
  const displayPath = payload.displayPath || payload.path;
  const hint = { ...payload, displayPath };
  const existingIndex = fileLinkHints.findIndex(
    (item) => item.path === hint.path && item.displayPath === hint.displayPath,
  );
  const isNew = existingIndex < 0;
  if (existingIndex >= 0) {
    fileLinkHints.splice(existingIndex, 1);
  }
  fileLinkHints.push(hint);
  if (fileLinkHints.length > 200) {
    fileLinkHints.shift();
  }
  return { hint, isNew };
}

function writeFileLinkHint(payload: PtyFileLinkPayload) {
  if (!terminal) return;
  const { hint, isNew } = rememberFileLinkHint(payload);
  if (!isNew) return;
  const lineText = `SeshBuddy: ${actionLabel(hint.action)} ${hint.displayPath}`;
  terminal.write(`\r\n${lineText}\r\n`);
}

function isCombiningCodePoint(code: number): boolean {
  return (
    (code >= 0x0300 && code <= 0x036f) ||
    (code >= 0x1ab0 && code <= 0x1aff) ||
    (code >= 0x1dc0 && code <= 0x1dff) ||
    (code >= 0x20d0 && code <= 0x20ff) ||
    (code >= 0xfe20 && code <= 0xfe2f)
  );
}

function isFullWidthCodePoint(code: number): boolean {
  return (
    code >= 0x1100 &&
    (code <= 0x115f ||
      code === 0x2329 ||
      code === 0x232a ||
      (code >= 0x2e80 && code <= 0xa4cf && code !== 0x303f) ||
      (code >= 0xac00 && code <= 0xd7a3) ||
      (code >= 0xf900 && code <= 0xfaff) ||
      (code >= 0xfe10 && code <= 0xfe19) ||
      (code >= 0xfe30 && code <= 0xfe6f) ||
      (code >= 0xff00 && code <= 0xff60) ||
      (code >= 0xffe0 && code <= 0xffe6) ||
      (code >= 0x1f300 && code <= 0x1f64f) ||
      (code >= 0x1f900 && code <= 0x1f9ff) ||
      (code >= 0x20000 && code <= 0x3fffd))
  );
}

function stringCellWidth(value: string): number {
  let width = 0;
  for (const char of value) {
    const code = char.codePointAt(0) ?? 0;
    if (code === 0 || isCombiningCodePoint(code)) continue;
    width += isFullWidthCodePoint(code) ? 2 : 1;
  }
  return width;
}

function linkRangeFromStringIndex(text: string, index: number, linkText: string, y: number) {
  const startX = stringCellWidth(text.slice(0, index)) + 1;
  const endX = startX + stringCellWidth(linkText);
  return {
    range: {
      start: { x: startX, y },
      end: { x: endX, y },
    },
    cellRange: {
      start: startX - 1,
      end: endX - 1,
    },
  };
}

function registerSeshBuddyFileLinkProvider() {
  if (!terminal) return;

  linkProviderDisposable = terminal.registerLinkProvider({
    provideLinks(bufferLineNumber, callback) {
      const line = terminal?.buffer.active.getLine(bufferLineNumber - 1);
      if (!line) {
        callback(undefined);
        return;
      }

      const text = line.translateToString(true);
      const links: ILink[] = [];
      const hints = [...fileLinkHints].sort((a, b) => b.displayPath.length - a.displayPath.length);
      const occupiedRanges: Array<{ start: number; end: number }> = [];
      for (const hint of hints) {
        const index = text.indexOf(hint.displayPath);
        if (index < 0) continue;
        const { range, cellRange } = linkRangeFromStringIndex(
          text,
          index,
          hint.displayPath,
          bufferLineNumber,
        );
        const overlaps = occupiedRanges.some(
          (occupied) => cellRange.start < occupied.end && cellRange.end > occupied.start,
        );
        if (overlaps) continue;
        occupiedRanges.push(cellRange);
        links.push({
          text: hint.displayPath,
          range,
          decorations: { pointerCursor: true, underline: true },
          activate() {
            if (!props.projectRoot) return;
            emit("openFile", hint.path, props.projectRoot);
          },
        });
      }

      callback(links.length > 0 ? links : undefined);
    },
  });
}

onMounted(async () => {
  if (!containerRef.value) return;

  const isDark = resolvedTheme.value === "dark";

  terminal = new Terminal({
    cursorBlink: true,
    fontSize: 13,
    fontFamily: '"SF Mono", "Fira Code", Menlo, Consolas, "DejaVu Sans Mono", monospace',
    theme: getThemeColors(isDark),
    allowProposedApi: true,
    convertEol: true,
    lineHeight: 1.2,
    scrollback: 5000,
    tabStopWidth: 4,
    macOptionIsMeta: true,
    macOptionClickForcesSelection: true,
  });

  fitAddon = new FitAddon();
  terminal.loadAddon(fitAddon);

  terminal.open(containerRef.value);
  registerSeshBuddyFileLinkProvider();

  // Track Shift key state for Shift+Enter → newline conversion
  let shiftPressed = false;

  terminal.attachCustomKeyEventHandler((e: KeyboardEvent) => {
    if (e.key === "Shift") {
      shiftPressed = e.type === "keydown";
    }
    return true;
  });

  const isWin = navigator.userAgent.toLowerCase().includes('windows');
  const renderer = (localStorage.getItem('seshbuddy-xterm-renderer') || (isWin ? 'canvas' : 'webgl')) as 'canvas' | 'webgl' | 'dom';

  let addonLoaded = false;
  if (renderer === 'webgl') {
    try {
      const webglAddon = new WebglAddon();
      webglAddon.onContextLoss(() => {
        webglAddon.dispose();
      });
      terminal.loadAddon(webglAddon);
      addonLoaded = true;
    } catch {
      // WebGL not available, fall through to canvas
    }
  }

  if (!addonLoaded && renderer !== 'dom') {
    try {
      terminal.loadAddon(new CanvasAddon());
    } catch {
      // Canvas not available, fall back to DOM renderer
    }
  }

  await new Promise<void>((resolve) => {
    requestAnimationFrame(() => {
      requestAnimationFrame(() => resolve());
    });
  });
  fitAddon.fit();

  await invokeApp("resize_pty_session", {
    sessionId: props.sessionId,
    cols: terminal.cols,
    rows: terminal.rows,
  }).catch(() => {});

  unlisten = await listen<string>(`pty-output-${props.sessionId}`, (event) => {
    if (!terminal) return;
    try {
      const bytes = Uint8Array.from(atob(event.payload), (c) => c.charCodeAt(0));
      terminal.write(bytes);
    } catch (e) {
      console.error("Failed to decode PTY output:", e);
    }
  });

  fileLinkUnlisten = await listen<PtyFileLinkPayload>(`pty-file-link-${props.sessionId}`, (event) => {
    if (!event.payload?.path || !event.payload?.displayPath) return;
    writeFileLinkHint(event.payload);
  });

  terminal.onData((data) => {
    let out = data;
    if (shiftPressed && data === "\r") {
      out = "\x0a";
    }
    const encoded = btoa(unescape(encodeURIComponent(out)));
    invokeApp("write_pty_session", {
      sessionId: props.sessionId,
      data: encoded,
    }).catch(() => {});
  });

  terminal.onResize(({ cols, rows }) => {
    invokeApp("resize_pty_session", {
      sessionId: props.sessionId,
      cols,
      rows,
    }).catch(() => {});
  });

  setTimeout(() => {
    fitAddon?.fit();
    invokeApp("resize_pty_session", {
      sessionId: props.sessionId,
      cols: terminal!.cols,
      rows: terminal!.rows,
    }).catch(() => {});
    invokeApp("write_pty_session", {
      sessionId: props.sessionId,
      data: btoa("\x0c"), // Ctrl+L
    }).catch(() => {});
  }, 600);

  retryFocus();

  resizeObserver = new ResizeObserver(() => {
    if (resizeDebounceTimer) clearTimeout(resizeDebounceTimer);
    resizeDebounceTimer = setTimeout(() => {
      fitAddon?.fit();
    }, 100);
  });
  resizeObserver.observe(containerRef.value);
});

watch(resolvedTheme, (newTheme) => {
  if (!terminal) return;
  terminal.options.theme = getThemeColors(newTheme === "dark");
});

onBeforeUnmount(() => {
  if (resizeDebounceTimer) clearTimeout(resizeDebounceTimer);
  resizeObserver?.disconnect();
  unlisten?.();
  fileLinkUnlisten?.();
  linkProviderDisposable?.dispose();
  fileLinkHints.splice(0, fileLinkHints.length);
  terminal?.dispose();
  terminal = null;
  fitAddon = null;
});

function focus() {
  terminal?.focus();
}

function fit() {
  fitAddon?.fit();
}

defineExpose({ focus, fit });
</script>

<template>
  <div class="agent-terminal-wrapper">
    <div ref="containerRef" class="agent-terminal"></div>
  </div>
</template>

<style scoped>
.agent-terminal-wrapper {
  flex: 1;
  height: 100%;
  width: 100%;
  min-height: 0;
  min-width: 0;
  padding: 8px;
  background: #0d0d0d;
  overflow: hidden;
  box-sizing: border-box;
  display: flex;
  flex-direction: column;
}
.agent-terminal {
  flex: 1;
  height: 100%;
  width: 100%;
  min-height: 0;
  min-width: 0;
  overflow: hidden;
}
.agent-terminal :deep(.xterm) {
  height: 100%;
  width: 100%;
}
.agent-terminal :deep(.xterm-viewport) {
  overflow-y: auto !important;
}
</style>

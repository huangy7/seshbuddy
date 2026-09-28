import { watch } from "vue";
import { copyToClipboard } from "./clipboard";
import { t, currentLocale } from "../i18n";

export const MERMAID_THEME_VARIABLES = {
  light: {
    darkMode: false,
    background: "#ffffff",
    primaryColor: "#e8f2ff",
    primaryBorderColor: "#6a9ed6",
    primaryTextColor: "#1d1d1f",
    secondaryColor: "#e8f7ef",
    secondaryBorderColor: "#67a886",
    secondaryTextColor: "#1d1d1f",
    tertiaryColor: "#f2edff",
    tertiaryBorderColor: "#9482bd",
    tertiaryTextColor: "#1d1d1f",
    lineColor: "#66717f",
    arrowheadColor: "#66717f",
    defaultLinkColor: "#66717f",
    textColor: "#1d1d1f",
    mainBkg: "#e8f2ff",
    nodeBorder: "#6a9ed6",
    nodeTextColor: "#1d1d1f",
    clusterBkg: "#f8fafc",
    clusterBorder: "#e2e7ef",
    edgeLabelBackground: "#ffffff",
    titleColor: "#1d1d1f",
    noteBkgColor: "#fff4cf",
    noteBorderColor: "#d0a736",
    noteTextColor: "#3a300d",
    activationBkgColor: "#dcecff",
    activationBorderColor: "#6a9ed6",
    actorLineColor: "#6a9ed6",
    signalColor: "#456f9e",
    signalTextColor: "#1d1d1f",
    sequenceNumberColor: "#ffffff",
    altSectionBkgColor: "#f8fafc",
    gridColor: "#e2e7ef",
    fontFamily: '-apple-system, BlinkMacSystemFont, "SF Pro Text", "PingFang SC", system-ui, sans-serif',
    fontSize: "14px",
    useGradient: false,
    dropShadow: "none",
  },
  dark: {
    darkMode: true,
    background: "#16181d",
    primaryColor: "#223147",
    primaryBorderColor: "#4f84c4",
    primaryTextColor: "#f1f3f5",
    secondaryColor: "#1d362d",
    secondaryBorderColor: "#478c6f",
    secondaryTextColor: "#f1f3f5",
    tertiaryColor: "#2f2842",
    tertiaryBorderColor: "#7c66a8",
    tertiaryTextColor: "#f1f3f5",
    lineColor: "#8f9cae",
    arrowheadColor: "#8f9cae",
    defaultLinkColor: "#8f9cae",
    textColor: "#f1f3f5",
    mainBkg: "#223147",
    nodeBorder: "#4f84c4",
    nodeTextColor: "#f1f3f5",
    clusterBkg: "#1e2128",
    clusterBorder: "#383f4d",
    edgeLabelBackground: "#16181d",
    titleColor: "#f1f3f5",
    noteBkgColor: "#3a321a",
    noteBorderColor: "#a3862b",
    noteTextColor: "#f6e8ad",
    activationBkgColor: "#223147",
    activationBorderColor: "#4f84c4",
    actorLineColor: "#4f84c4",
    signalColor: "#7daadb",
    signalTextColor: "#f1f3f5",
    sequenceNumberColor: "#16181d",
    altSectionBkgColor: "#1e2128",
    gridColor: "#383f4d",
    fontFamily: '-apple-system, BlinkMacSystemFont, "SF Pro Text", "PingFang SC", system-ui, sans-serif',
    fontSize: "14px",
    useGradient: false,
    dropShadow: "none",
  },
} as const;

export interface MermaidTransform {
  scale: number;
  x: number;
  y: number;
}

const MERMAID_MIN_SCALE = 0.3;
const MERMAID_MAX_SCALE = 4.0;
const MERMAID_ZOOM_STEP = 1.2;

const COPY_ICON = `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="9" y="9" width="13" height="13" rx="2" ry="2"></rect><path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"></path></svg>`;
const CHECK_ICON = `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M20 6 9 17l-5-5"></path></svg>`;
const ZOOM_IN_ICON = `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="11" cy="11" r="8"></circle><line x1="21" y1="21" x2="16.65" y2="16.65"></line><line x1="11" y1="8" x2="11" y2="14"></line><line x1="8" y1="11" x2="14" y2="11"></line></svg>`;
const ZOOM_OUT_ICON = `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="11" cy="11" r="8"></circle><line x1="21" y1="21" x2="16.65" y2="16.65"></line><line x1="8" y1="11" x2="14" y2="11"></line></svg>`;
const RESET_ICON = `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M3 12a9 9 0 1 0 9-9 9.75 9.75 0 0 0-6.74 2.74L3 8"></path><path d="M3 3v5h5"></path></svg>`;
const MAXIMIZE_ICON = `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M8 3H5a2 2 0 0 0-2 2v3"></path><path d="M21 8V5a2 2 0 0 0-2-2h-3"></path><path d="M3 16v3a2 2 0 0 0 2 2h3"></path><path d="M16 21h3a2 2 0 0 0 2-2v-3"></path></svg>`;
const MINIMIZE_ICON = `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M8 3v3a2 2 0 0 1-2 2H3"></path><path d="M21 8h-3a2 2 0 0 1-2-2V3"></path><path d="M3 16h3a2 2 0 0 1 2 2v3"></path><path d="M16 21v-3a2 2 0 0 1 2-2h3"></path></svg>`;

type MermaidApi = typeof import("mermaid")["default"];
let mermaidPromise: Promise<MermaidApi> | null = null;
let mermaidInitializedTheme: "light" | "dark" | null = null;
let mermaidSequenceId = 0;
let escapeListenerBound = false;
let activeBackdrop: HTMLElement | null = null;
let themeObserverInitialized = false;

const copyTimers = new WeakMap<HTMLButtonElement, number>();

export function getCurrentTheme(): "light" | "dark" {
  if (typeof document === "undefined") return "light";
  return document.documentElement.getAttribute("data-theme") === "dark" ? "dark" : "light";
}

export async function ensureMermaid(theme: "light" | "dark"): Promise<MermaidApi> {
  if (mermaidPromise === null) {
    mermaidPromise = import("mermaid").then((m) => m.default);
  }
  const mermaid = await mermaidPromise;
  if (mermaidInitializedTheme !== theme) {
    const natural = { useMaxWidth: false };
    mermaid.initialize({
      startOnLoad: false,
      theme: "base",
      themeVariables: MERMAID_THEME_VARIABLES[theme],
      flowchart: natural,
      sequence: natural,
      gantt: natural,
      er: natural,
      journey: natural,
      state: natural,
      class: natural,
      pie: natural,
      securityLevel: "loose",
    });
    mermaidInitializedTheme = theme;
  }
  return mermaid;
}

/**
 * WKWebView can leave a <style> inside dynamically inserted SVG unapplied,
 * which turns every shape and connector into the SVG defaults (black). Mirror
 * Mermaid's complete, id-scoped stylesheet into HTML while keeping the SVG's
 * original style so exported markup remains self-contained.
 */
export function mountMermaidSvg(content: HTMLElement, svgMarkup: string): void {
  const parsed = new DOMParser().parseFromString(svgMarkup, "image/svg+xml");
  const parsedSvg = parsed.documentElement;
  if (parsedSvg.namespaceURI !== "http://www.w3.org/2000/svg" || parsedSvg.tagName.toLowerCase() !== "svg") {
    throw new Error("Mermaid did not return an SVG document");
  }
  const svg = document.importNode(parsedSvg, true);
  const stylesheet = document.createElement("style");
  stylesheet.dataset.mermaidStylesheet = "";
  stylesheet.textContent = Array.from(svg.querySelectorAll("style"), (style) => style.textContent ?? "").join("\n");
  content.replaceChildren(stylesheet, svg);
}

export function readMermaidTransform(block: HTMLElement): MermaidTransform {
  const scale = Number(block.dataset.mermaidScale);
  const x = Number(block.dataset.mermaidX);
  const y = Number(block.dataset.mermaidY);
  return {
    scale: Number.isFinite(scale) && scale > 0 ? scale : 1,
    x: Number.isFinite(x) ? x : 0,
    y: Number.isFinite(y) ? y : 0,
  };
}

export function applyMermaidTransform(block: HTMLElement): void {
  const content = block.querySelector<HTMLElement>(".markdown-mermaid-content");
  if (!content) return;
  const { scale, x, y } = readMermaidTransform(block);
  content.style.transform = `translate(${x}px, ${y}px) scale(${scale})`;
}

export function setMermaidTransform(block: HTMLElement, transform: MermaidTransform): void {
  const scale = Math.min(MERMAID_MAX_SCALE, Math.max(MERMAID_MIN_SCALE, transform.scale));
  block.dataset.mermaidScale = String(scale);
  block.dataset.mermaidX = String(transform.x);
  block.dataset.mermaidY = String(transform.y);
  applyMermaidTransform(block);
}

export function resetMermaidTransform(block: HTMLElement): void {
  setMermaidTransform(block, { scale: 1, x: 0, y: 0 });
}

export function zoomMermaidBlock(block: HTMLElement, factor: number): void {
  const transform = readMermaidTransform(block);
  setMermaidTransform(block, {
    ...transform,
    scale: transform.scale * factor,
  });
}

function handleEscapeKey(e: KeyboardEvent) {
  if (e.key !== "Escape") return;
  const fullscreens = document.querySelectorAll<HTMLElement>(".markdown-mermaid-fullscreen");
  if (fullscreens.length === 0) {
    syncEscapeListener();
    return;
  }
  e.preventDefault();
  for (const block of fullscreens) {
    setMermaidFullscreen(block, false);
  }
}

function syncEscapeListener() {
  const hasFullscreen = document.querySelector(".markdown-mermaid-fullscreen") !== null;
  if (hasFullscreen && !escapeListenerBound) {
    document.addEventListener("keydown", handleEscapeKey);
    escapeListenerBound = true;
  } else if (!hasFullscreen && escapeListenerBound) {
    document.removeEventListener("keydown", handleEscapeKey);
    escapeListenerBound = false;
  }
}

export function setMermaidFullscreen(block: HTMLElement, enabled: boolean): void {
  block.classList.toggle("markdown-mermaid-fullscreen", enabled);
  const fullscreenBtn = block.querySelector<HTMLButtonElement>('button[data-mermaid-action="fullscreen"]');
  if (fullscreenBtn) {
    fullscreenBtn.title = enabled ? t("common.mermaid.exitFullscreen") : t("common.mermaid.fullscreen");
    fullscreenBtn.setAttribute("aria-label", fullscreenBtn.title);
    fullscreenBtn.innerHTML = enabled ? MINIMIZE_ICON : MAXIMIZE_ICON;
  }

  if (enabled) {
    if (!activeBackdrop) {
      activeBackdrop = document.createElement("div");
      activeBackdrop.className = "markdown-mermaid-fullscreen-backdrop";
      activeBackdrop.addEventListener("click", () => {
        setMermaidFullscreen(block, false);
      });
      document.body.appendChild(activeBackdrop);
    }
    block.querySelector<HTMLElement>(".markdown-mermaid-canvas")?.focus();
  } else {
    if (activeBackdrop) {
      activeBackdrop.remove();
      activeBackdrop = null;
    }
  }
  syncEscapeListener();
}

export async function copyMermaidSource(block: HTMLElement, button: HTMLButtonElement): Promise<void> {
  const source = block.dataset.mermaidSource;
  if (!source) return;
  const success = await copyToClipboard(source);
  if (success) {
    const prev = copyTimers.get(button);
    if (prev) clearTimeout(prev);
    button.classList.add("copied");
    button.innerHTML = CHECK_ICON;
    button.title = t("dialogs.common.copied");
    const timer = window.setTimeout(() => {
      button.classList.remove("copied");
      button.innerHTML = COPY_ICON;
      button.title = t("common.mermaid.copySource");
      copyTimers.delete(button);
    }, 2000);
    copyTimers.set(button, timer);
  }
}

function bindCanvasEvents(block: HTMLElement) {
  const canvas = block.querySelector<HTMLElement>(".markdown-mermaid-canvas");
  if (!canvas || canvas.dataset.eventsBound === "true") return;
  canvas.dataset.eventsBound = "true";

  let dragState: {
    pointerId: number;
    startX: number;
    startY: number;
    initialTransformX: number;
    initialTransformY: number;
  } | null = null;

  canvas.addEventListener("pointerdown", (e: PointerEvent) => {
    if (e.button !== 0) return;
    if ((e.target as HTMLElement).closest("button")) return;
    const transform = readMermaidTransform(block);
    dragState = {
      pointerId: e.pointerId,
      startX: e.clientX,
      startY: e.clientY,
      initialTransformX: transform.x,
      initialTransformY: transform.y,
    };
    canvas.classList.add("markdown-mermaid-canvas-dragging");
    canvas.setPointerCapture(e.pointerId);
  });

  canvas.addEventListener("pointermove", (e: PointerEvent) => {
    if (!dragState || dragState.pointerId !== e.pointerId) return;
    e.preventDefault();
    const dx = e.clientX - dragState.startX;
    const dy = e.clientY - dragState.startY;
    const transform = readMermaidTransform(block);
    setMermaidTransform(block, {
      ...transform,
      x: dragState.initialTransformX + dx,
      y: dragState.initialTransformY + dy,
    });
  });

  const finishDrag = (e: PointerEvent) => {
    if (!dragState || dragState.pointerId !== e.pointerId) return;
    canvas.classList.remove("markdown-mermaid-canvas-dragging");
    if (canvas.hasPointerCapture(e.pointerId)) {
      canvas.releasePointerCapture(e.pointerId);
    }
    dragState = null;
  };

  canvas.addEventListener("pointerup", finishDrag);
  canvas.addEventListener("pointercancel", finishDrag);

  canvas.addEventListener(
    "wheel",
    (e: WheelEvent) => {
      const isFullscreen = block.classList.contains("markdown-mermaid-fullscreen");
      if (!isFullscreen && !e.ctrlKey && !e.metaKey) return;
      e.preventDefault();
      const factor = e.deltaY < 0 ? MERMAID_ZOOM_STEP : 1 / MERMAID_ZOOM_STEP;
      zoomMermaidBlock(block, factor);
    },
    { passive: false }
  );
}

function bindToolbarEvents(block: HTMLElement) {
  const toolbar = block.querySelector<HTMLElement>(".markdown-mermaid-toolbar");
  if (!toolbar || toolbar.dataset.eventsBound === "true") return;
  toolbar.dataset.eventsBound = "true";

  toolbar.addEventListener("click", (e: MouseEvent) => {
    const btn = (e.target as HTMLElement).closest<HTMLButtonElement>("button[data-mermaid-action]");
    if (!btn) return;
    e.preventDefault();
    e.stopPropagation();
    const action = btn.dataset.mermaidAction;
    if (action === "copy") {
      void copyMermaidSource(block, btn);
    } else if (action === "zoomIn") {
      zoomMermaidBlock(block, MERMAID_ZOOM_STEP);
    } else if (action === "zoomOut") {
      zoomMermaidBlock(block, 1 / MERMAID_ZOOM_STEP);
    } else if (action === "reset") {
      resetMermaidTransform(block);
    } else if (action === "fullscreen") {
      const isFullscreen = block.classList.contains("markdown-mermaid-fullscreen");
      setMermaidFullscreen(block, !isFullscreen);
    }
  });
}

function ensureThemeObserver() {
  if (typeof document === "undefined" || themeObserverInitialized) return;
  themeObserverInitialized = true;
  const observer = new MutationObserver(() => {
    const theme = getCurrentTheme();
    const blocks = document.querySelectorAll<HTMLElement>(".markdown-mermaid-block[data-mermaid-source]");
    for (const block of blocks) {
      void renderSingleMermaidBlock(block, theme, true);
    }
  });
  observer.observe(document.documentElement, {
    attributes: true,
    attributeFilter: ["data-theme"],
  });
}

/**
 * 工具栏是内嵌 HTML 字符串，按钮文案在模板里插值：title 与 aria-label 各取一次 t()，
 * 与 ChatView 的 capture overlay 同一写法。插值发生在调用时（渲染期或语言切换时），
 * 不是模块加载期。`Mermaid` 是品牌名，不进语言包。
 *
 * 全屏态的两处文案与图标由 block 当前状态决定：重写工具栏时不能把「退出全屏」重置成
 * 「进入全屏」，那会让按钮看起来与当前状态相反。
 */
function mermaidToolbarHtml(block: HTMLElement): string {
  const isFullscreen = block.classList.contains("markdown-mermaid-fullscreen");
  const fullscreenLabel = t(
    isFullscreen ? "common.mermaid.exitFullscreen" : "common.mermaid.fullscreen"
  );
  return `
      <span class="markdown-mermaid-toolbar-title">
        <svg viewBox="0 0 24 24" width="12" height="12" fill="none" stroke="currentColor" stroke-width="2"><circle cx="6" cy="6" r="3"/><circle cx="18" cy="18" r="3"/><path d="M6 9v12m12-9V3m-9 9h9"/></svg>
        Mermaid
      </span>
      <button type="button" class="markdown-mermaid-action" data-mermaid-action="copy" title="${t("common.mermaid.copySource")}" aria-label="${t("common.mermaid.copySource")}">${COPY_ICON}</button>
      <button type="button" class="markdown-mermaid-action" data-mermaid-action="zoomOut" title="${t("common.mermaid.zoomOut")}" aria-label="${t("common.mermaid.zoomOut")}">${ZOOM_OUT_ICON}</button>
      <button type="button" class="markdown-mermaid-action" data-mermaid-action="zoomIn" title="${t("common.mermaid.zoomIn")}" aria-label="${t("common.mermaid.zoomIn")}">${ZOOM_IN_ICON}</button>
      <button type="button" class="markdown-mermaid-action" data-mermaid-action="reset" title="${t("common.mermaid.resetZoom")}" aria-label="${t("common.mermaid.resetZoom")}">${RESET_ICON}</button>
      <button type="button" class="markdown-mermaid-action" data-mermaid-action="fullscreen" title="${fullscreenLabel}" aria-label="${fullscreenLabel}">${isFullscreen ? MINIMIZE_ICON : MAXIMIZE_ICON}</button>
    `;
}

/** 原地重写工具栏的 innerHTML；toolbar 元素本身与 canvas 都不动。 */
function rewriteMermaidToolbar(block: HTMLElement): void {
  const toolbar = block.querySelector<HTMLElement>(".markdown-mermaid-toolbar");
  if (!toolbar) return;
  toolbar.innerHTML = mermaidToolbarHtml(block);
}

let localeWatcherInitialized = false;

/**
 * 语言切换后重写所有已存在块的工具栏。
 *
 * 工具栏由 innerHTML 命令式插入，不在 Vue 渲染树里——切语言不会重渲它，五个按钮的
 * title/aria-label 会停在创建时的语言（copy 按钮点一次会自愈，其余四个不会）。
 * 事件委托绑在 toolbar 元素上而非按钮上，所以换掉按钮不丢监听，重写 innerHTML 即可；
 * 不重建块，也就不会连带重渲已经渲染好的 SVG。
 */
function ensureLocaleWatcher() {
  if (localeWatcherInitialized) return;
  localeWatcherInitialized = true;
  watch(currentLocale, () => {
    const blocks = document.querySelectorAll<HTMLElement>(
      ".markdown-mermaid-block[data-mermaid-source]"
    );
    for (const block of blocks) {
      rewriteMermaidToolbar(block);
    }
  });
}

export function wrapMermaidBlocks(root: HTMLElement): HTMLElement[] {
  ensureLocaleWatcher();
  const blocks: HTMLElement[] = [];
  const codeElements = Array.from(root.querySelectorAll<HTMLElement>("code.language-mermaid"));

  for (const codeEl of codeElements) {
    if (codeEl.closest(".markdown-mermaid-block")) continue;
    const source = codeEl.textContent ?? "";
    const replaceTarget = codeEl.parentElement instanceof HTMLPreElement ? codeEl.parentElement : codeEl;

    const block = document.createElement("div");
    block.className = "markdown-mermaid-block";
    block.dataset.mermaidSource = source;
    resetMermaidTransform(block);

    const toolbar = document.createElement("div");
    toolbar.className = "markdown-mermaid-toolbar";
    toolbar.innerHTML = mermaidToolbarHtml(block);

    const canvas = document.createElement("div");
    canvas.className = "markdown-mermaid-canvas";
    canvas.tabIndex = 0;

    const surface = document.createElement("div");
    surface.className = "markdown-mermaid-surface";

    const content = document.createElement("div");
    content.className = "markdown-mermaid-content";
    content.textContent = source;

    surface.appendChild(content);
    canvas.appendChild(surface);
    block.appendChild(toolbar);
    block.appendChild(canvas);

    replaceTarget.replaceWith(block);
    bindCanvasEvents(block);
    bindToolbarEvents(block);
    blocks.push(block);
  }

  const existingBlocks = Array.from(root.querySelectorAll<HTMLElement>(".markdown-mermaid-block[data-mermaid-source]"));
  for (const b of existingBlocks) {
    bindCanvasEvents(b);
    bindToolbarEvents(b);
    if (!blocks.includes(b)) blocks.push(b);
  }
  return blocks;
}

export async function renderSingleMermaidBlock(
  block: HTMLElement,
  theme: "light" | "dark",
  force = false
): Promise<void> {
  const source = block.dataset.mermaidSource;
  const content = block.querySelector<HTMLElement>(".markdown-mermaid-content");
  if (!source || !content) return;

  if (!force && block.dataset.mermaidTheme === theme && block.dataset.mermaidRendered === "true") {
    return;
  }

  block.dataset.mermaidTheme = theme;
  block.dataset.mermaidRendered = "false";

  try {
    const mermaid = await ensureMermaid(theme);
    const id = `seshbuddy-mermaid-${++mermaidSequenceId}`;
    const { svg, bindFunctions } = await mermaid.render(id, source);
    if (!block.isConnected) return;

    content.classList.remove("markdown-mermaid-error");
    mountMermaidSvg(content, svg);
    applyMermaidTransform(block);
    bindFunctions?.(content);
    block.dataset.mermaidRendered = "true";
    delete block.dataset.mermaidError;
  } catch (err) {
    if (!block.isConnected) return;
    block.dataset.mermaidError = "true";
    content.classList.add("markdown-mermaid-error");
    content.textContent = source;
    console.warn("[SeshBuddy Mermaid] Render error:", err);
  }
}

export async function renderMermaidInElement(
  root: HTMLElement,
  targetTheme?: "light" | "dark"
): Promise<void> {
  if (!root.querySelector("code.language-mermaid, .markdown-mermaid-block[data-mermaid-source]")) {
    return;
  }
  ensureThemeObserver();
  const theme = targetTheme ?? getCurrentTheme();
  const blocks = wrapMermaidBlocks(root);
  for (const block of blocks) {
    void renderSingleMermaidBlock(block, theme);
  }
}

export function cleanupMermaidInElement(root: HTMLElement): void {
  for (const block of root.querySelectorAll<HTMLElement>(".markdown-mermaid-fullscreen")) {
    setMermaidFullscreen(block, false);
  }
  syncEscapeListener();
}

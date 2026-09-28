import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { nextTick } from "vue";
import { setLocale, t } from "../i18n";
import {
  wrapMermaidBlocks,
  readMermaidTransform,
  setMermaidTransform,
  resetMermaidTransform,
  zoomMermaidBlock,
  setMermaidFullscreen,
  mountMermaidSvg,
  renderSingleMermaidBlock,
} from "./mermaid";

/** 工具栏五个控件各自的文案键——action 名与语言包键名不同名，故显式成表。 */
const TOOLBAR_LABEL_KEYS: ReadonlyArray<[string, string]> = [
  ["copy", "common.mermaid.copySource"],
  ["zoomOut", "common.mermaid.zoomOut"],
  ["zoomIn", "common.mermaid.zoomIn"],
  ["reset", "common.mermaid.resetZoom"],
  ["fullscreen", "common.mermaid.fullscreen"],
];

describe("mermaid utility functions", () => {
  beforeEach(() => {
    document.body.innerHTML = "";
  });

  afterEach(() => {
    document.body.innerHTML = "";
    vi.restoreAllMocks();
  });

  it("wrapMermaidBlocks transforms pre > code.language-mermaid into markdown-mermaid-block", () => {
    const container = document.createElement("div");
    container.innerHTML = `
      <pre class="hljs language-mermaid"><code class="language-mermaid">graph TD;\n  A-->B;</code></pre>
    `;
    document.body.appendChild(container);

    const blocks = wrapMermaidBlocks(container);
    expect(blocks).toHaveLength(1);

    const block = blocks[0];
    expect(block.classList.contains("markdown-mermaid-block")).toBe(true);
    expect(block.dataset.mermaidSource).toContain("graph TD;\n  A-->B;");

    const toolbar = block.querySelector(".markdown-mermaid-toolbar");
    expect(toolbar).not.toBeNull();
    expect(toolbar?.querySelector('button[data-mermaid-action="copy"]')).not.toBeNull();
    expect(toolbar?.querySelector('button[data-mermaid-action="zoomIn"]')).not.toBeNull();
    expect(toolbar?.querySelector('button[data-mermaid-action="zoomOut"]')).not.toBeNull();
    expect(toolbar?.querySelector('button[data-mermaid-action="reset"]')).not.toBeNull();
    expect(toolbar?.querySelector('button[data-mermaid-action="fullscreen"]')).not.toBeNull();

    const canvas = block.querySelector(".markdown-mermaid-canvas");
    expect(canvas).not.toBeNull();
    const content = block.querySelector(".markdown-mermaid-content");
    expect(content).not.toBeNull();
    expect(content?.textContent).toContain("graph TD;\n  A-->B;");
  });

  it("pan and zoom transform functions correctly clamp and apply values", () => {
    const block = document.createElement("div");
    const content = document.createElement("div");
    content.className = "markdown-mermaid-content";
    block.appendChild(content);

    resetMermaidTransform(block);
    let transform = readMermaidTransform(block);
    expect(transform.scale).toBe(1);
    expect(transform.x).toBe(0);
    expect(transform.y).toBe(0);

    setMermaidTransform(block, { scale: 2, x: 50, y: -30 });
    transform = readMermaidTransform(block);
    expect(transform.scale).toBe(2);
    expect(transform.x).toBe(50);
    expect(transform.y).toBe(-30);
    expect(content.style.transform).toBe("translate(50px, -30px) scale(2)");

    // Zoom beyond max scale (4.0) clamps
    zoomMermaidBlock(block, 3);
    transform = readMermaidTransform(block);
    expect(transform.scale).toBe(4.0);

    // Zoom below min scale (0.3) clamps
    zoomMermaidBlock(block, 0.01);
    transform = readMermaidTransform(block);
    expect(transform.scale).toBe(0.3);

    // Reset restores scale 1 and (0,0)
    resetMermaidTransform(block);
    transform = readMermaidTransform(block);
    expect(transform.scale).toBe(1);
    expect(transform.x).toBe(0);
    expect(transform.y).toBe(0);
  });

  it("setMermaidFullscreen toggles fullscreen class and manages backdrop", () => {
    const block = document.createElement("div");
    block.className = "markdown-mermaid-block";
    const btn = document.createElement("button");
    btn.dataset.mermaidAction = "fullscreen";
    block.appendChild(btn);
    document.body.appendChild(block);

    setMermaidFullscreen(block, true);
    expect(block.classList.contains("markdown-mermaid-fullscreen")).toBe(true);
    expect(document.querySelector(".markdown-mermaid-fullscreen-backdrop")).not.toBeNull();

    setMermaidFullscreen(block, false);
    expect(block.classList.contains("markdown-mermaid-fullscreen")).toBe(false);
    expect(document.querySelector(".markdown-mermaid-fullscreen-backdrop")).toBeNull();
  });

  it("mountMermaidSvg extracts SVG styles to container for WKWebView compatibility", () => {
    const content = document.createElement("div");
    const svgMarkup = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 50"><style>.node { fill: red; }</style><rect class="node" width="10" height="10"/></svg>`;

    mountMermaidSvg(content, svgMarkup);
    const styleEl = content.querySelector("style[data-mermaid-stylesheet]");
    expect(styleEl).not.toBeNull();
    expect(styleEl?.textContent).toContain(".node { fill: red; }");
    expect(content.querySelector("svg")).not.toBeNull();
  });

  it("renderSingleMermaidBlock handles rendering errors gracefully", async () => {
    const block = document.createElement("div");
    block.dataset.mermaidSource = "invalid mermaid syntax syntax error !!!";
    const content = document.createElement("div");
    content.className = "markdown-mermaid-content";
    block.appendChild(content);
    document.body.appendChild(block);

    await renderSingleMermaidBlock(block, "light");
    expect(block.dataset.mermaidError).toBe("true");
    expect(content.classList.contains("markdown-mermaid-error")).toBe(true);
    expect(content.textContent).toContain("invalid mermaid syntax");
  });

  it("语言切换后工具栏五个控件的 title 与 aria-label 跟随语言，且块是原地重写", async () => {
    const container = document.createElement("div");
    container.innerHTML = `
      <pre class="hljs language-mermaid"><code class="language-mermaid">graph TD;\n  A-->B;</code></pre>
    `;
    document.body.appendChild(container);

    setLocale("en");
    const block = wrapMermaidBlocks(container)[0];
    const toolbar = block.querySelector<HTMLElement>(".markdown-mermaid-toolbar");
    const canvas = block.querySelector<HTMLElement>(".markdown-mermaid-canvas");
    expect(toolbar).not.toBeNull();
    expect(canvas).not.toBeNull();
    const control = (action: string) =>
      toolbar!.querySelector<HTMLButtonElement>(`button[data-mermaid-action="${action}"]`)!;

    // 正向对照一：en 下的取值确实来自语言包（否则下面「等于 t()」恒真）
    for (const [action, key] of TOOLBAR_LABEL_KEYS) {
      expect(control(action).title, `${action} 的初始 title 不是包值`).toBe(t(key));
    }
    // 正向对照二：这五个键在 zh 下确实换词。取值四语同值时，下面的断言会退化成恒真。
    const enTitles = new Map(TOOLBAR_LABEL_KEYS.map(([a]) => [a, control(a).title]));

    try {
      setLocale("zh");
      await nextTick();

      // 正向对照三：**同一个 toolbar 节点**原地重写。块被整体重建（重新挂载）时
      // 这两个引用会脱开，断言即红——「重挂载后正好是新语言」蒙混不过去。
      // canvas 未被重建，说明这次修复没有连带重渲已渲染的 SVG。
      expect(block.querySelector(".markdown-mermaid-toolbar")).toBe(toolbar);
      expect(block.querySelector(".markdown-mermaid-canvas")).toBe(canvas);

      for (const [action, key] of TOOLBAR_LABEL_KEYS) {
        expect(control(action).title, `${action} 的 title 未跟随语言`).toBe(t(key));
        // aria-label 是屏幕阅读器用户唯一拿得到的文案，漏掉它等于没修
        expect(control(action).getAttribute("aria-label"), `${action} 的 aria-label 未跟随语言`).toBe(
          t(key),
        );
        expect(enTitles.get(action), `${key} 在 zh 下与 en 同值，本用例对该键无鉴别力`).not.toBe(
          t(key),
        );
      }
    } finally {
      setLocale("en");
      await nextTick();
    }
  });

  it("切语言重写 innerHTML 之后，点击仍被 toolbar 上的委托接住（控件换了、监听没换）", async () => {
    const container = document.createElement("div");
    container.innerHTML = `
      <pre class="hljs language-mermaid"><code class="language-mermaid">graph TD;\n  A-->B;</code></pre>
    `;
    document.body.appendChild(container);

    setLocale("en");
    const block = wrapMermaidBlocks(container)[0];
    const toolbar = block.querySelector<HTMLElement>(".markdown-mermaid-toolbar")!;
    const zoomInButton = () =>
      toolbar.querySelector<HTMLButtonElement>('button[data-mermaid-action="zoomIn"]')!;
    const before = zoomInButton();

    try {
      setLocale("zh");
      await nextTick();

      // 正向对照一：重写确实换掉了按钮节点。节点没换的话，下面「点击仍生效」可能只是因为
      // 监听从来没丢过，与事件委托这条性质无关。
      expect(zoomInButton()).not.toBe(before);
      // 正向对照二：初始 scale 不是待断言的终值，否则断言恒真。
      expect(readMermaidTransform(block).scale).toBe(1);

      // 点击**重写后**的按钮。监听委托在 toolbar 元素上（它不在被替换的子树里），
      // 处理器用 e.target.closest(...) 反查按钮，故换掉按钮不丢监听。
      // 若将来改成逐个按钮 bind 监听，新按钮没有监听，这里的 scale 会停在 1。
      zoomInButton().dispatchEvent(new MouseEvent("click", { bubbles: true }));
      expect(readMermaidTransform(block).scale).toBeCloseTo(1.2);
    } finally {
      setLocale("en");
      await nextTick();
    }
  });

  it("全屏态下切语言：仍是「退出全屏」的文案与图标，不被重置为进入全屏", async () => {
    const container = document.createElement("div");
    container.innerHTML = `
      <pre class="hljs language-mermaid"><code class="language-mermaid">graph TD;\n  A-->B;</code></pre>
    `;
    document.body.appendChild(container);

    setLocale("en");
    const block = wrapMermaidBlocks(container)[0];
    const toolbar = block.querySelector<HTMLElement>(".markdown-mermaid-toolbar")!;
    const fullscreenBtn = () =>
      toolbar.querySelector<HTMLButtonElement>('button[data-mermaid-action="fullscreen"]')!;

    try {
      setMermaidFullscreen(block, true);
      expect(fullscreenBtn().title).toBe(t("common.mermaid.exitFullscreen"));

      setLocale("zh");
      await nextTick();

      expect(fullscreenBtn().title).toBe(t("common.mermaid.exitFullscreen"));
      expect(fullscreenBtn().getAttribute("aria-label")).toBe(t("common.mermaid.exitFullscreen"));
      // 图标同理：全屏中重写成「进入全屏」的图标，按钮看起来就与当前状态相反了
      expect(fullscreenBtn().innerHTML).toContain('d="M8 3v3');
      expect(fullscreenBtn().innerHTML).not.toContain('d="M8 3H5');
    } finally {
      setMermaidFullscreen(block, false);
      setLocale("en");
      await nextTick();
    }
  });
});

import { describe, it, expect } from "vitest";
import {
  extractWidgetData,
  sanitizeSvg,
  ensureWorkBuddySvgStyles,
  svgToDataUrl,
} from "./svg";

describe("svg utilities", () => {
  it("extracts widget data from WorkBuddy show_widget tool call input", () => {
    const input = JSON.stringify({
      title: "消息树 fork 分叉示意",
      widget_code: '<svg viewBox="0 0 680 380" width="100%"><text class="th">m1</text></svg>',
    });

    const data = extractWidgetData(input);
    expect(data).not.toBeNull();
    expect(data?.title).toBe("消息树 fork 分叉示意");
    expect(data?.widgetType).toBe("svg");
    expect(data?.widgetCode).toContain("<svg");
  });

  it("extracts widget data from direct SVG string", () => {
    const rawSvg = '<svg viewBox="0 0 100 100"><title>Direct Diagram</title><circle r="10"/></svg>';
    const data = extractWidgetData(rawSvg);
    expect(data).not.toBeNull();
    expect(data?.title).toBe("Direct Diagram");
    expect(data?.widgetType).toBe("svg");
    expect(data?.widgetCode).toBe(rawSvg);
  });

  it("extracts widget data from visualizer_show_widget_result output format", () => {
    const output = JSON.stringify({
      type: "visualizer_show_widget_result",
      success: true,
      title: "分叉结构",
      widget_code: '<svg viewBox="0 0 680 200"><rect class="node c-blue"/></svg>',
    });

    const data = extractWidgetData(output);
    expect(data).not.toBeNull();
    expect(data?.title).toBe("分叉结构");
    expect(data?.widgetCode).toContain("c-blue");
  });

  it("returns null for non-widget input", () => {
    expect(extractWidgetData("")).toBeNull();
    expect(extractWidgetData(null)).toBeNull();
    expect(extractWidgetData(JSON.stringify({ file_path: "test.ts" }))).toBeNull();
    expect(extractWidgetData("Plain text output")).toBeNull();
  });

  it("sanitizes dangerous scripts and event handlers from SVG", () => {
    const malicious = `
      <svg viewBox="0 0 100 100" onload="alert('pwn')">
        <script>alert('xss')</script>
        <circle cx="50" cy="50" r="40" onclick="alert(1)" />
        <a href="javascript:alert(2)"><text>link</text></a>
      </svg>
    `;

    const cleaned = sanitizeSvg(malicious);
    expect(cleaned).not.toContain("<script>");
    expect(cleaned).not.toContain("alert('xss')");
    expect(cleaned).not.toContain("onload=");
    expect(cleaned).not.toContain("onclick=");
    expect(cleaned).not.toContain("javascript:alert(2)");
    expect(cleaned).toContain("<circle");
  });

  it("injects WorkBuddy CSS styles and ensures responsive attributes", () => {
    const rawSvg = `<svg viewBox="0 0 680 380"><defs></defs><g class="node c-blue"><rect/></g></svg>`;
    const styled = ensureWorkBuddySvgStyles(rawSvg);
    expect(styled).toContain("width=\"100%\"");
    expect(styled).toContain(".c-blue");
    expect(styled).toContain(".th");
    expect(styled).toContain(".arr");
  });

  it("encodes SVG as data URL", () => {
    const svg = `<svg viewBox="0 0 10 10"><title>测试</title></svg>`;
    const dataUrl = svgToDataUrl(svg);
    expect(dataUrl.startsWith("data:image/svg+xml;base64,")).toBe(true);
    const decoded = Buffer.from(dataUrl.split(",")[1], "base64").toString("utf-8");
    expect(decoded).toBe(svg);
  });
});

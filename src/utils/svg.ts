/**
 * SVG and widget parsing, sanitization, and styling utilities.
 * Supports WorkBuddy show_widget and arbitrary SVG diagrams.
 */

import { t } from "../i18n";

export interface WidgetData {
  title: string;
  widgetCode: string;
  widgetType: "svg" | "html";
}

/**
 * Extracts widget data from tool input or tool result string/JSON.
 */
export function extractWidgetData(inputOrContent: string | null | undefined): WidgetData | null {
  if (!inputOrContent) return null;
  const raw = inputOrContent.trim();
  if (!raw) return null;

  if (!raw.includes("<svg") && !raw.includes("widget_code")) {
    return null;
  }

  // 1. If it directly starts with <svg and ends with </svg>
  if (raw.startsWith("<svg") && raw.endsWith("</svg>")) {
    const titleMatch = raw.match(/<title>(.*?)<\/title>/i);
    return {
      title: titleMatch ? titleMatch[1].trim() : t("common.svg.defaultTitle"),
      widgetCode: raw,
      widgetType: "svg",
    };
  }

  // 2. Try JSON parse
  try {
    const parsed = JSON.parse(raw);
    const fromObj = extractFromParsedJson(parsed);
    if (fromObj) return fromObj;
  } catch {
    // Not top-level JSON directly, try looking for {"widget_code": or {"title":
  }

  // 3. If it's an array or nested json format like [{"type":"input_text","text":"{...}"}]
  if (raw.includes('"widget_code"')) {
    try {
      const firstBrace = raw.indexOf("{");
      const lastBrace = raw.lastIndexOf("}");
      if (firstBrace !== -1 && lastBrace !== -1 && lastBrace > firstBrace) {
        const substr = raw.substring(firstBrace, lastBrace + 1);
        const parsed = JSON.parse(substr);
        const fromObj = extractFromParsedJson(parsed);
        if (fromObj) return fromObj;
      }
    } catch {
      // fallback regex extraction
    }

    const widgetCodeMatch = raw.match(/"widget_code"\s*:\s*"((?:[^"\\]|\\.)*)"/);
    if (widgetCodeMatch) {
      try {
        const widgetCode = JSON.parse(`"${widgetCodeMatch[1]}"`);
        const titleMatch = raw.match(/"title"\s*:\s*"((?:[^"\\]|\\.)*)"/);
        // 缺标题时复用 ChatSvgWidgetCard 的缺省标题：抽出的 title 正是那张卡片渲染的 title，
        // 同一个展示位不该有两份措辞。
        const title = titleMatch ? JSON.parse(`"${titleMatch[1]}"`) : t("chat.svgCard.defaultTitle");
        const isSvg = widgetCode.trim().startsWith("<svg") || /<svg[\s>]/i.test(widgetCode);
        return {
          title,
          widgetCode,
          widgetType: isSvg ? "svg" : "html",
        };
      } catch {
        // ignore
      }
    }
  }

  return null;
}

function extractFromParsedJson(parsed: any): WidgetData | null {
  if (!parsed) return null;

  // If array, search items
  if (Array.isArray(parsed)) {
    for (const item of parsed) {
      const res = extractFromParsedJson(item);
      if (res) return res;
    }
    return null;
  }

  if (typeof parsed !== "object") return null;

  // Check direct widget_code property
  if (typeof parsed.widget_code === "string" && parsed.widget_code.trim()) {
    const code = parsed.widget_code.trim();
    const title = typeof parsed.title === "string" && parsed.title.trim()
      ? parsed.title.trim()
      : (code.match(/<title>(.*?)<\/title>/i)?.[1]?.trim() || t("chat.svgCard.defaultTitle"));
    const isSvg = code.startsWith("<svg") || /<svg[\s>]/i.test(code);
    return {
      title,
      widgetCode: code,
      widgetType: isSvg ? "svg" : "html",
    };
  }

  // If object has 'text' field which might be a JSON string (e.g. WorkBuddy output)
  if (typeof parsed.text === "string" && parsed.text.includes('"widget_code"')) {
    try {
      const inner = JSON.parse(parsed.text);
      const res = extractFromParsedJson(inner);
      if (res) return res;
    } catch {
      // ignore
    }
  }

  // If object has 'output' field
  if (parsed.output) {
    const res = extractFromParsedJson(parsed.output);
    if (res) return res;
  }

  return null;
}

/**
 * Sanitizes SVG code to prevent XSS.
 * Removes script tags, event handlers, javascript: URLs, and unsafe elements.
 */
export function sanitizeSvg(svgCode: string): string {
  if (!svgCode) return "";

  let cleaned = svgCode;

  // 1. Remove <script> tags
  cleaned = cleaned.replace(/<script\b[^<]*(?:(?!<\/script>)<[^<]*)*<\/script\s*>/gi, "");

  // 2. Remove <foreignObject>, <iframe>, <object>, <embed>
  cleaned = cleaned.replace(/<foreignObject\b[^<]*(?:(?!<\/foreignObject>)<[^<]*)*<\/foreignObject\s*>/gi, "");
  cleaned = cleaned.replace(/<iframe\b[^<]*(?:(?!<\/iframe>)<[^<]*)*<\/iframe\s*>/gi, "");
  cleaned = cleaned.replace(/<object\b[^<]*(?:(?!<\/object>)<[^<]*)*<\/object\s*>/gi, "");
  cleaned = cleaned.replace(/<embed\b[^>]*\/?>/gi, "");

  // 3. Remove inline event handlers (on*="...")
  cleaned = cleaned.replace(/\s+on[a-z]+\s*=\s*(["'])[\s\S]*?\1/gi, "");
  cleaned = cleaned.replace(/\s+on[a-z]+\s*=\s*[^\s>]+/gi, "");

  // 4. Remove javascript: in href/xlink:href/src
  cleaned = cleaned.replace(/(href|xlink:href|src)\s*=\s*(["'])\s*javascript:[\s\S]*?\2/gi, '$1="#"');

  return cleaned;
}

/**
 * WorkBuddy Visualizer CSS styles for pre-built classes and color ramps.
 * Applied inside SVG or wrapper to ensure proper coloring and typography.
 */
export const WORKBUDDY_SVG_STYLES = `
/* WorkBuddy Visualizer Base Typography & Elements */
text { font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, "PingFang SC", "Hiragino Sans GB", "Microsoft YaHei", sans-serif; }
text.th { font-size: 14px; font-weight: 500; fill: #1e293b; }
text.ts { font-size: 12px; font-weight: 400; fill: #64748b; }
text.t { font-size: 14px; font-weight: 400; fill: #0f172a; }
.box { fill: #f8fafc; stroke: #cbd5e1; }
.arr { stroke: #64748b; stroke-width: 1.5px; fill: none; }
.leader { stroke: #cbd5e1; stroke-width: 0.5px; stroke-dasharray: 5 4; fill: none; }
.node { cursor: pointer; transition: opacity 0.15s ease; }
.node:hover { opacity: 0.85; }
marker#arrow path, marker#arrow polygon { stroke: #64748b; fill: none; }

/* Color Ramps (Light Mode) */
.c-purple rect, .c-purple circle, .c-purple ellipse { fill: #EEEDFE; stroke: #534AB7; }
.c-purple text.th { fill: #3C3489; }
.c-purple text.ts { fill: #534AB7; }

.c-teal rect, .c-teal circle, .c-teal ellipse { fill: #E1F5EE; stroke: #0F6E56; }
.c-teal text.th { fill: #085041; }
.c-teal text.ts { fill: #0F6E56; }

.c-coral rect, .c-coral circle, .c-coral ellipse { fill: #FAECE7; stroke: #993C1D; }
.c-coral text.th { fill: #712B13; }
.c-coral text.ts { fill: #993C1D; }

.c-pink rect, .c-pink circle, .c-pink ellipse { fill: #FBEAF0; stroke: #993556; }
.c-pink text.th { fill: #72243E; }
.c-pink text.ts { fill: #993556; }

.c-gray rect, .c-gray circle, .c-gray ellipse { fill: #F1EFE8; stroke: #5F5E5A; }
.c-gray text.th { fill: #444441; }
.c-gray text.ts { fill: #5F5E5A; }

.c-blue rect, .c-blue circle, .c-blue ellipse { fill: #E6F1FB; stroke: #185FA5; }
.c-blue text.th { fill: #0C447C; }
.c-blue text.ts { fill: #185FA5; }

.c-green rect, .c-green circle, .c-green ellipse { fill: #EAF3DE; stroke: #3B6D11; }
.c-green text.th { fill: #27500A; }
.c-green text.ts { fill: #3B6D11; }

.c-amber rect, .c-amber circle, .c-amber ellipse { fill: #FAEEDA; stroke: #854F0B; }
.c-amber text.th { fill: #633806; }
.c-amber text.ts { fill: #854F0B; }

.c-red rect, .c-red circle, .c-red ellipse { fill: #FCEBEB; stroke: #A32D2D; }
.c-red text.th { fill: #791F1F; }
.c-red text.ts { fill: #A32D2D; }

/* Dark Mode Adaptation */
@media (prefers-color-scheme: dark) {
  text.th { fill: #f1f5f9; }
  text.ts { fill: #94a3b8; }
  text.t { fill: #f1f5f9; }
  .box { fill: #1e293b; stroke: #334155; }
  .arr { stroke: #94a3b8; }
  .leader { stroke: #475569; }
  marker#arrow path, marker#arrow polygon { stroke: #94a3b8; }

  .c-purple rect, .c-purple circle, .c-purple ellipse { fill: #3C3489; stroke: #AFA9EC; }
  .c-purple text.th { fill: #CECBF6; }
  .c-purple text.ts { fill: #AFA9EC; }

  .c-teal rect, .c-teal circle, .c-teal ellipse { fill: #085041; stroke: #5DCAA5; }
  .c-teal text.th { fill: #9FE1CB; }
  .c-teal text.ts { fill: #5DCAA5; }

  .c-coral rect, .c-coral circle, .c-coral ellipse { fill: #712B13; stroke: #F0997B; }
  .c-coral text.th { fill: #F5C4B3; }
  .c-coral text.ts { fill: #F0997B; }

  .c-pink rect, .c-pink circle, .c-pink ellipse { fill: #72243E; stroke: #ED93B1; }
  .c-pink text.th { fill: #F4C0D1; }
  .c-pink text.ts { fill: #ED93B1; }

  .c-gray rect, .c-gray circle, .c-gray ellipse { fill: #444441; stroke: #B4B2A9; }
  .c-gray text.th { fill: #D3D1C7; }
  .c-gray text.ts { fill: #B4B2A9; }

  .c-blue rect, .c-blue circle, .c-blue ellipse { fill: #0C447C; stroke: #85B7EB; }
  .c-blue text.th { fill: #B5D4F4; }
  .c-blue text.ts { fill: #85B7EB; }

  .c-green rect, .c-green circle, .c-green ellipse { fill: #27500A; stroke: #97C459; }
  .c-green text.th { fill: #C0DD97; }
  .c-green text.ts { fill: #97C459; }

  .c-amber rect, .c-amber circle, .c-amber ellipse { fill: #633806; stroke: #EF9F27; }
  .c-amber text.th { fill: #FAC775; }
  .c-amber text.ts { fill: #EF9F27; }

  .c-red rect, .c-red circle, .c-red ellipse { fill: #791F1F; stroke: #F09595; }
  .c-red text.th { fill: #F7C1C1; }
  .c-red text.ts { fill: #F09595; }
}
`.trim();

/**
 * Ensures the SVG has WorkBuddy styling injected inside <defs><style>...</style></defs>.
 * Also ensures responsive sizing attributes.
 */
export function ensureWorkBuddySvgStyles(svgCode: string): string {
  const sanitized = sanitizeSvg(svgCode);
  if (!sanitized) return "";

  let result = sanitized;

  // Ensure responsive attributes on root <svg>
  result = result.replace(/<svg\b([^>]*)>/i, (_match, attrs) => {
    let newAttrs = attrs;
    if (!/width\s*=/i.test(newAttrs)) {
      newAttrs += ' width="100%"';
    }
    return `<svg ${newAttrs.trim()}>`;
  });

  // Inject <style> into <defs> if not already containing WorkBuddy styles
  if (!result.includes(".c-blue rect") && !result.includes("/* WorkBuddy Visualizer")) {
    const styleTag = `<style type="text/css">\n${WORKBUDDY_SVG_STYLES}\n</style>`;
    if (/<defs[\s>]/i.test(result)) {
      result = result.replace(/(<defs[\s>][^>]*>)/i, `$1\n${styleTag}\n`);
    } else {
      result = result.replace(/(<svg\b[^>]*>)/i, `$1\n<defs>\n${styleTag}\n</defs>\n`);
    }
  }

  return result;
}

/**
 * Encodes SVG code as a data URL for <img> tags or downloading.
 */
export function svgToDataUrl(svgCode: string): string {
  try {
    const base64 = btoa(unescape(encodeURIComponent(svgCode)));
    return `data:image/svg+xml;base64,${base64}`;
  } catch {
    return `data:image/svg+xml;charset=utf-8,${encodeURIComponent(svgCode)}`;
  }
}

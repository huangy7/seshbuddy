import MarkdownIt from "markdown-it";
import hljs from "highlight.js/lib/core";

// Register languages
import javascript from "highlight.js/lib/languages/javascript";
import typescript from "highlight.js/lib/languages/typescript";
import python from "highlight.js/lib/languages/python";
import rust from "highlight.js/lib/languages/rust";
import bash from "highlight.js/lib/languages/bash";
import json from "highlight.js/lib/languages/json";
import css from "highlight.js/lib/languages/css";
import xml from "highlight.js/lib/languages/xml";
import go from "highlight.js/lib/languages/go";
import java from "highlight.js/lib/languages/java";
import sql from "highlight.js/lib/languages/sql";
import yaml from "highlight.js/lib/languages/yaml";
import markdown from "highlight.js/lib/languages/markdown";
import diff from "highlight.js/lib/languages/diff";

hljs.registerLanguage("javascript", javascript);
hljs.registerLanguage("js", javascript);
hljs.registerLanguage("typescript", typescript);
hljs.registerLanguage("ts", typescript);
hljs.registerLanguage("python", python);
hljs.registerLanguage("rust", rust);
hljs.registerLanguage("bash", bash);
hljs.registerLanguage("sh", bash);
hljs.registerLanguage("shell", bash);
hljs.registerLanguage("json", json);
hljs.registerLanguage("css", css);
hljs.registerLanguage("html", xml);
hljs.registerLanguage("xml", xml);
hljs.registerLanguage("go", go);
hljs.registerLanguage("java", java);
hljs.registerLanguage("sql", sql);
hljs.registerLanguage("yaml", yaml);
hljs.registerLanguage("yml", yaml);
hljs.registerLanguage("markdown", markdown);
hljs.registerLanguage("md", markdown);
hljs.registerLanguage("diff", diff);
hljs.registerLanguage("svg", xml);

import { ensureWorkBuddySvgStyles } from "./svg";
import katexPluginModule from "@vscode/markdown-it-katex";
import "katex/dist/katex.min.css";
import DOMPurify, { type Config } from "dompurify";

type KatexPlugin = (md: MarkdownIt, options?: { enableFencedBlocks?: boolean; throwOnError?: boolean }) => MarkdownIt;

function defaultExport(value: unknown): unknown {
  return value !== null && typeof value === "object" && "default" in value ? (value as any).default : undefined;
}

function isKatexPlugin(value: unknown): value is KatexPlugin {
  return typeof value === "function";
}

function resolveKatexPlugin(value: unknown): KatexPlugin {
  if (isKatexPlugin(value)) return value;

  const firstDefault = defaultExport(value);
  if (isKatexPlugin(firstDefault)) return firstDefault;

  const secondDefault = defaultExport(firstDefault);
  if (isKatexPlugin(secondDefault)) return secondDefault;

  throw new Error("Unable to load markdown-it KaTeX plugin");
}

const md = new MarkdownIt({
  html: true,
  linkify: true,
  typographer: false,
  highlight(str: string, lang: string): string {
    if (lang && lang.toLowerCase() === "svg") {
      try {
        const styled = ensureWorkBuddySvgStyles(str);
        const result = hljs.getLanguage("xml")
          ? hljs.highlight(str, { language: "xml" })
          : { value: md.utils.escapeHtml(str) };
        return `<div class="markdown-svg-block"><div class="markdown-svg-graphic workbuddy-svg-widget">${styled}</div><pre class="hljs"><code class="language-svg">${result.value}</code></pre></div>`;
      } catch {
        // fall through
      }
    }
    if (lang && lang.toLowerCase() === "mermaid") {
      return `<pre class="hljs language-mermaid"><code class="language-mermaid">${md.utils.escapeHtml(str)}</code></pre>`;
    }
    if (lang && hljs.getLanguage(lang)) {
      try {
        const result = hljs.highlight(str, { language: lang });
        return `<pre class="hljs"><code class="language-${lang}">${result.value}</code></pre>`;
      } catch {
        // fall through
      }
    }
    // Auto-detect
    try {
      const result = hljs.highlightAuto(str);
      return `<pre class="hljs"><code>${result.value}</code></pre>`;
    } catch {
      // fall through
    }
    return `<pre class="hljs"><code>${md.utils.escapeHtml(str)}</code></pre>`;
  },
});

md.use(resolveKatexPlugin(katexPluginModule), {
  enableFencedBlocks: true,
  throwOnError: false,
});

function isExternalLinkHref(href: string): boolean {
  return /^(https?:|mailto:|tel:)/i.test(href) || href.startsWith("//");
}

const defaultLinkOpenRenderer = md.renderer.rules.link_open ?? ((tokens, idx, options, _env, self) => self.renderToken(tokens, idx, options));

md.renderer.rules.link_open = (tokens, idx, options, env, self) => {
  const token = tokens[idx];
  const href = token.attrGet("href") ?? "";

  if (isExternalLinkHref(href)) {
    token.attrSet("target", "_blank");
    token.attrSet("rel", "noopener noreferrer");
    token.attrSet("data-external-link", "true");
  }

  return defaultLinkOpenRenderer(tokens, idx, options, env, self);
};

/** Strip ANSI escape codes and format as a code block or inline code. */
function formatOutput(raw: string): string {
  const stripped = raw.replace(/\x1b\[[0-9;]*m/g, "").trim();
  if (!stripped) return "";
  if (stripped.includes("\n")) {
    const fence = stripped.includes("```") ? "````" : "```";
    return `\n${fence}\n${stripped}\n${fence}`;
  }
  return stripped.includes("`") ? `\`\` ${stripped} \`\`` : `\`${stripped}\``;
}

/** Strip XML-like command tags and format them as readable text. */
function preprocessCommandTags(text: string): string {
  const hasCommandTags = /<command-name>/.test(text) || /<command-message>/.test(text);
  const hasLocalStdout = /<local-command-stdout>/.test(text);
  const hasBashTags = /<bash-input>/.test(text) || /<bash-stdout>/.test(text) || /<bash-stderr>/.test(text);

  if (!hasCommandTags && !hasLocalStdout && !hasBashTags) return text;

  let result = text;

  // Replace command tags with formatted version: **`/command`** args
  if (hasCommandTags) {
    // Remove <command-message>...</command-message> entirely (redundant with command-name)
    result = result.replace(/<command-message>[\s\S]*?<\/command-message>/g, "");
    // Replace <command-name> with bold code
    result = result.replace(/<command-name>(.*?)<\/command-name>/g, "**`$1`**");
    // Replace <command-args> with inline content
    result = result.replace(/<command-args>(.*?)<\/command-args>/g, (_m, args: string) => {
      const trimmed = args.trim();
      return trimmed ? ` \`${trimmed}\`` : "";
    });
  }

  // Replace <local-command-stdout> with a code block or inline
  if (hasLocalStdout) {
    result = result.replace(/<local-command-stdout>([\s\S]*?)<\/local-command-stdout>/g, (_m, stdout: string) => formatOutput(stdout));
  }

  // Replace <bash-input>, <bash-stdout>, <bash-stderr>
  if (hasBashTags) {
    result = result.replace(/<bash-input>([\s\S]*?)<\/bash-input>/g, (_m, cmd: string) => {
      const stripped = cmd.replace(/\x1b\[[0-9;]*m/g, "").trim();
      if (!stripped) return "";
      const lines = stripped.split("\n");
      const prompted = lines.map((l, i) => (i === 0 ? `$ ${l}` : `> ${l}`)).join("\n");
      return `\n\`\`\`bash\n${prompted}\n\`\`\``;
    });
    result = result.replace(/<bash-stdout>([\s\S]*?)<\/bash-stdout>/g, (_m, stdout: string) => formatOutput(stdout));
    result = result.replace(/<bash-stderr>([\s\S]*?)<\/bash-stderr>/g, (_m, stderr: string) => formatOutput(stderr));
  }

  // Clean up extra whitespace
  return result.replace(/\n{3,}/g, "\n\n").trim();
}

const DOM_PURIFY_CONFIG: Config = {
  USE_PROFILES: { html: true, svg: true, mathMl: true },
  ADD_TAGS: [
    "details",
    "summary",
    "kbd",
    "mark",
    "u",
    "sub",
    "sup",
    "semantics",
    "annotation",
  ],
  ADD_ATTR: [
    "data-external-link",
    "data-prefix",
    "data-tool-key",
    "target",
    "rel",
    "open",
    "align",
    "aria-hidden",
    "encoding",
  ],
  FORBID_TAGS: [
    "script",
    "iframe",
    "frame",
    "object",
    "embed",
    "applet",
    "form",
    "input",
    "textarea",
    "button",
    "select",
  ],
};

type Purifier = ReturnType<typeof DOMPurify>;
let purifierInstance: Purifier | null = null;

function getPurifier(): Purifier | null {
  if (purifierInstance) return purifierInstance;
  if (typeof window === "undefined") return null;

  const instance: Purifier =
    typeof (DOMPurify as unknown) === "function" ? DOMPurify(window) : DOMPurify;

  instance.addHook("uponSanitizeElement", (node: Node, data: { tagName: string }) => {
    if (data.tagName === "a" && node instanceof Element) {
      const href = node.getAttribute("href");
      if (href && isExternalLinkHref(href)) {
        node.setAttribute("target", "_blank");
        node.setAttribute("rel", "noopener noreferrer");
        node.setAttribute("data-external-link", "true");
      }
    }
  });

  instance.addHook("uponSanitizeAttribute", (_node: Element, data: { attrName: string; attrValue: string }) => {
    if (data.attrName === "style" && typeof data.attrValue === "string") {
      if (/position\s*:\s*(fixed|absolute)/i.test(data.attrValue) || /z-index/i.test(data.attrValue)) {
        data.attrValue = data.attrValue
          .replace(/position\s*:\s*(fixed|absolute)\s*;?/gi, "")
          .replace(/z-index\s*:\s*[^;]+;?/gi, "");
      }
    }
  });

  purifierInstance = instance;
  return purifierInstance;
}

export function sanitizeHtml(html: string): string {
  const purifier = getPurifier();
  if (!purifier) return html;
  return purifier.sanitize(html, DOM_PURIFY_CONFIG) as string;
}

export function renderMarkdown(text: string): string {
  const rawHtml = md.render(preprocessCommandTags(text));
  return sanitizeHtml(rawHtml);
}

const SESSION_REF_RE = /⟦(\d+):([0-9a-fA-F]{8})⟧/g;

/** 从源文本提取所有 ⟦N:前缀⟧ 的会话前缀（去重，保持出现顺序） */
export function extractSessionRefPrefixes(text: string): string[] {
  const seen = new Set<string>();
  const out: string[] = [];
  for (const m of text.matchAll(SESSION_REF_RE)) {
    const prefix = m[2].toLowerCase();
    if (!seen.has(prefix)) {
      seen.add(prefix);
      out.push(prefix);
    }
  }
  return out;
}

function escapeAttr(s: string): string {
  return s
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

/**
 * 渲染后处理：把 HTML 里的 ⟦N:前缀⟧ 替换为引用胶囊（prefix 在 refs 内），
 * 无效引用（编造/已删，不在 refs 内）直接抹掉——用户看不到曾经有过标记。
 * refs: prefix → 会话标题（tooltip）；标题为 `null`/`undefined` 一律按无效引用抹掉。
 */
export function renderSessionRefs(html: string, refs: ReadonlyMap<string, string>): string {
  return html.replace(SESSION_REF_RE, (_whole, n: string, prefix: string) => {
    const key = prefix.toLowerCase();
    const title = refs.get(key);
    // `== null` 而不是 `=== undefined`：契约说值一定是 string，但漏进来的 null 会被
    // escapeAttr 当成字符串取 `.replace` 而抛 TypeError——异常冒到模板表达式会让整个
    // 消息列表连用户自己的发言一起白屏。宽松判据把「缺值」与「null 值」都归到抹掉。
    if (title == null) return ""; // 无效引用：抹掉
    return `<sup class="session-ref" data-prefix="${key}" title="${escapeAttr(title)}">${n}</sup>`;
  });
}

const FOLLOWUPS_RE = /«FOLLOWUPS:\s*([\s\S]*?)»/;
const FOLLOWUPS_BLOCK_RE = /«FOLLOWUPS:[\s\S]*?(?:»|$)/g;

/** 从源文本提取大模型动态输出的 «FOLLOWUPS: 建议一 | 建议二» 追问列表 */
export function extractFollowups(text: string): string[] {
  const match = text.match(FOLLOWUPS_RE);
  if (!match || !match[1]) return [];
  return match[1]
    .split("|")
    .map((s) => s.trim().replace(/^[\d\.\-\*✦•\s]+/, ""))
    .filter((s) => s.length > 0 && s.length <= 50);
}

/** 过滤并移除源文本末尾的 «FOLLOWUPS:...» 标签，防止污染 Markdown 正文渲染 */
export function stripFollowups(text: string): string {
  return text.replace(FOLLOWUPS_BLOCK_RE, "").trimEnd();
}


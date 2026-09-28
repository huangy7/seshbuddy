import { describe, it, expect } from "vitest";
import {
  extractSessionRefPrefixes,
  renderSessionRefs,
  extractFollowups,
  stripFollowups,
} from "./markdown";

describe("extractSessionRefPrefixes", () => {
  it("提取所有⟦N:前缀⟧并去重", () => {
    expect(extractSessionRefPrefixes("拆了 ⟦1:90259911⟧ 又 ⟦2:3f8e5341⟧ 再 ⟦1:90259911⟧"))
      .toEqual(["90259911", "3f8e5341"]);
  });
  it("无引用返回空数组", () => {
    expect(extractSessionRefPrefixes("没有引用的文本 [1] 普通方括号")).toEqual([]);
  });
});

describe("renderSessionRefs", () => {
  const refs = new Map([["90259911", "cores 部署"], ["3f8e5341", "重构讨论"]]);

  it("有效前缀渲染成胶囊 sup 元素", () => {
    const out = renderSessionRefs("<p>拆了 ⟦1:90259911⟧</p>", refs);
    expect(out).toContain('<sup class="session-ref" data-prefix="90259911" title="cores 部署">1</sup>');
    expect(out).not.toContain("⟦1:90259911⟧");
  });

  it("无效前缀（编造/已删）从文本抹掉", () => {
    const out = renderSessionRefs("<p>拆了 ⟦1:deadbeef⟧</p>", refs);
    expect(out).not.toContain("session-ref");
    expect(out).not.toContain("⟦1:deadbeef⟧");
    expect(out).toContain("<p>拆了 </p>"); // 内容保留，标记消失
  });

  it("多个连续引用各自处理（有效+无效混合）", () => {
    const out = renderSessionRefs("<p>结论 ⟦1:90259911⟧⟦2:deadbeef⟧⟦3:3f8e5341⟧</p>", refs);
    expect(out).toContain('data-prefix="90259911"');
    expect(out).toContain('data-prefix="3f8e5341"');
    expect(out).not.toContain("deadbeef");
  });

  it("title 含 HTML 特殊字符时转义", () => {
    const dirty = new Map([["aaaaaaaa", '标题 <img> "引号"']]);
    const out = renderSessionRefs("<p>x ⟦1:aaaaaaaa⟧</p>", dirty);
    expect(out).not.toContain("<img>");
    expect(out).toContain("&lt;img&gt;");
  });

  it("值为 null 时按无效引用抹掉，不抛异常", () => {
    // 契约是 string，但漏进来的 null 会被 escapeAttr 取 `.replace` 而抛 TypeError；
    // 异常冒到模板表达式会让整个消息列表白屏，故判据是 `== null` 而不是 `=== undefined`。
    const withNull = new Map([["aaaaaaaa", null as unknown as string]]);
    expect(() => renderSessionRefs("<p>x ⟦1:aaaaaaaa⟧</p>", withNull)).not.toThrow();
    const out = renderSessionRefs("<p>x ⟦1:aaaaaaaa⟧</p>", withNull);
    expect(out).not.toContain("session-ref");
    expect(out).toContain("<p>x </p>");
  });
});

describe("extractFollowups and stripFollowups", () => {
  it("正确提取模型输出的 «FOLLOWUPS: ...» 标签并分割为列表", () => {
    const text = "这是回答正文。\n«FOLLOWUPS: 深入解释架构设计 | 补充单元测试用例 | 生成周报»";
    expect(extractFollowups(text)).toEqual([
      "深入解释架构设计",
      "补充单元测试用例",
      "生成周报",
    ]);
  });

  it("无 FOLLOWUPS 标签时返回空数组", () => {
    expect(extractFollowups("普通回答内容")).toEqual([]);
  });

  it("流式生成中未闭合的 FOLLOWUPS 标签返回空数组", () => {
    expect(extractFollowups("正文\n«FOLLOWUPS: 生成周")).toEqual([]);
  });

  it("stripFollowups 能够干净剥离已闭合与未闭合的标签", () => {
    expect(stripFollowups("正文内容\n«FOLLOWUPS: 追问一 | 追问二»")).toBe("正文内容");
    expect(stripFollowups("正文内容\n«FOLLOWUPS: 正在生成中")).toBe("正文内容");
  });
});

describe("renderMarkdown with SVG code block", () => {
  it("renders both SVG graphic and code block for ```svg fences", async () => {
    const { renderMarkdown } = await import("./markdown");
    const mdText = '```svg\n<svg viewBox="0 0 100 100"><circle r="10"/></svg>\n```';
    const html = renderMarkdown(mdText);
    expect(html).toContain("markdown-svg-block");
    expect(html).toContain("markdown-svg-graphic");
    expect(html).toContain("<circle");
    expect(html).toContain("<pre class=\"hljs\"><code class=\"language-svg\">");
  });
});

describe("renderMarkdown with Mermaid code block", () => {
  it("renders language-mermaid code block for ```mermaid fences", async () => {
    const { renderMarkdown } = await import("./markdown");
    const mdText = "```mermaid\ngraph TD;\n    A[开始] --> B{判断};\n```";
    const html = renderMarkdown(mdText);
    expect(html).toContain('<pre class="hljs language-mermaid"><code class="language-mermaid">');
    expect(html).toContain("graph TD;");
    expect(html).toContain("A[开始]");
  });
});

describe("renderMarkdown with KaTeX math formula", () => {
  it("renders inline math formula $E = mc^2$", async () => {
    const { renderMarkdown } = await import("./markdown");
    const mdText = "质量能量方程 $E = mc^2$ 说明质能等价。";
    const html = renderMarkdown(mdText);
    expect(html).toContain("katex");
    expect(html).toContain("math");
    expect(html).toContain("mc");
  });

  it("renders display math block $$\\int_{-\\infty}^{\\infty} e^{-x^2} dx$$", async () => {
    const { renderMarkdown } = await import("./markdown");
    const mdText = "$$\n\\int_{-\\infty}^{\\infty} e^{-x^2} \\, dx = \\sqrt{\\pi}\n$$";
    const html = renderMarkdown(mdText);
    expect(html).toContain("katex-display");
    expect(html).toContain("katex-block");
  });
});

describe("renderMarkdown with HTML mixed content and XSS sanitization", () => {
  it("renders safe HTML elements: kbd, mark, u, sub, sup, details, summary", async () => {
    const { renderMarkdown } = await import("./markdown");
    const mdText = "按快捷键 <kbd>Cmd</kbd> + <kbd>K</kbd>，这是 <mark>高亮文本</mark>，<u>下划线</u>，H<sub>2</sub>O 和 E=mc<sup>2</sup>。\n\n<details open><summary>折叠详情</summary><p>详细内容</p></details>";
    const html = renderMarkdown(mdText);

    expect(html).toContain("<kbd>Cmd</kbd>");
    expect(html).toContain("<kbd>K</kbd>");
    expect(html).toContain("<mark>高亮文本</mark>");
    expect(html).toContain("<u>下划线</u>");
    expect(html).toContain("<sub>2</sub>");
    expect(html).toContain("<sup>2</sup>");
    expect(html).toContain("<details");
    expect(html).toContain("<summary>折叠详情</summary>");
    expect(html).toContain("详细内容");
  });

  it("sanitizes malicious script tags and inline event handlers", async () => {
    const { renderMarkdown } = await import("./markdown");
    const mdText = '<script>alert("xss")</script><img src="x" onerror="alert(1)">';
    const html = renderMarkdown(mdText);

    expect(html).not.toContain("<script");
    expect(html).not.toContain("alert");
    expect(html).not.toContain("onerror");
    expect(html).toContain('<img src="x">');
  });

  it("sanitizes dangerous iframes, embeds, and forms", async () => {
    const { renderMarkdown } = await import("./markdown");
    const mdText = '<iframe src="https://evil.com"></iframe><form action="/phish"><input type="text"><button>提交</button></form>';
    const html = renderMarkdown(mdText);

    expect(html).not.toContain("<iframe");
    expect(html).not.toContain("<form");
    expect(html).not.toContain("<input");
    expect(html).not.toContain("<button");
  });

  it("strips javascript: pseudoprotocols and adds external link attributes to raw HTML links", async () => {
    const { renderMarkdown } = await import("./markdown");
    const mdText = '<a href="javascript:alert(1)">恶意链接</a> 和 <a href="https://example.com">外部链接</a>';
    const html = renderMarkdown(mdText);

    expect(html).not.toContain("javascript:");
    expect(html).toContain('data-external-link="true"');
    expect(html).toContain('target="_blank"');
    expect(html).toContain('rel="noopener noreferrer"');
    expect(html).toContain('href="https://example.com"');
  });

  it("strips UI redressing styles (position: fixed/absolute, z-index)", async () => {
    const { renderMarkdown } = await import("./markdown");
    const mdText = '<div style="position:fixed;inset:0;z-index:999999;color:red;">盖住全屏</div>';
    const html = renderMarkdown(mdText);

    expect(html).not.toContain("position:fixed");
    expect(html).not.toContain("position: fixed");
    expect(html).not.toContain("z-index");
    expect(html).toContain("color:red");
  });

  it("preserves HTML code blocks without stripping code contents", async () => {
    const { renderMarkdown } = await import("./markdown");
    const mdText = '```html\n<script>alert("hello")</script>\n```';
    const html = renderMarkdown(mdText);

    expect(html).toContain("language-html");
    expect(html).toContain("hljs-name");
    expect(html).toContain("script");
    expect(html).toContain("alert");
    expect(html).toContain('"hello"');
  });

  it("renders complex markdown documents cleanly", async () => {
    const { renderMarkdown } = await import("./markdown");
    const complexMd = [
      "# Complex Document",
      "| Col 1 | Col 2 |",
      "|---|---|",
      "| Data A | Data B |",
      "```mermaid",
      "graph TD; A-->B;",
      "```",
      '<script>alert("xss")</script>',
    ].join("\n");
    const html = renderMarkdown(complexMd);
    expect(html).toContain("<table");
    expect(html).toContain("language-mermaid");
    expect(html).not.toContain("<script>alert");
  });
});

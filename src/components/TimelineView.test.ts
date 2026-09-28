import { describe, expect, it, afterEach } from "vitest";
import { mount } from "@vue/test-utils";
import { setLocale } from "../i18n";
import type { ChatMessage } from "../types/session";
import TimelineView from "./TimelineView.vue";

function imageMessage(timestamp: string): ChatMessage {
  return {
    role: "user",
    timestamp,
    model: null,
    content_parts: [{ type: "image", media_type: "image/png", data: "" }],
  };
}

function textMessage(timestamp: string, text: string): ChatMessage {
  return {
    role: "user",
    timestamp,
    model: null,
    content_parts: [{ type: "text", text }],
  };
}

function render(messages: ChatMessage[]) {
  return mount(TimelineView, { props: { messages, loading: false } });
}

function previewsOf(wrapper: ReturnType<typeof render>): string[] {
  return wrapper.findAll(".timeline-preview").map((node) => node.text());
}

function previews(messages: ChatMessage[]): string[] {
  return previewsOf(render(messages));
}

describe("TimelineView 相邻纯图片消息合并", () => {
  it("把 3 条相邻纯图片消息合并成一个节点，张数等于实际图片数", () => {
    const wrapper = render([
      imageMessage("2026-09-01T10:00:00Z"),
      imageMessage("2026-09-01T10:00:01Z"),
      imageMessage("2026-09-01T10:00:02Z"),
    ]);

    expect(wrapper.findAll(".timeline-node")).toHaveLength(1);
    expect(previewsOf(wrapper)).toEqual(["📷 3 images"]);
  });

  it("只合并相邻的图片节点，被文本隔开的两组各自计数", () => {
    expect(
      previews([
        imageMessage("2026-09-01T10:00:00Z"),
        imageMessage("2026-09-01T10:00:01Z"),
        textMessage("2026-09-01T10:00:02Z", "look at this"),
        imageMessage("2026-09-01T10:00:03Z"),
        imageMessage("2026-09-01T10:00:04Z"),
      ])
    ).toEqual(["📷 2 images", "look at this", "📷 2 images"]);
  });

  it("首行以 📷 开头的文本消息不参与合并", () => {
    expect(
      previews([
        imageMessage("2026-09-01T10:00:00Z"),
        textMessage("2026-09-01T10:00:01Z", "📷 see the screenshot"),
      ])
    ).toEqual(["📷 Image", "📷 see the screenshot"]);
  });
});

function agentCallMessage(timestamp: string, summary: string): ChatMessage {
  return {
    role: "assistant",
    timestamp,
    model: null,
    content_parts: [{ type: "tool_use", tool_name: "Agent", summary, input: "" }],
  };
}

// Agent 调用节点的 preview 只在 summary 为空时才回退到标签，所以这条回退既容易写错、
// 又只在一种输入下出现。
//
// ja/de 的取值**不在这里抄一遍**——那是语言包的第二个事实来源，改了包还要改测试，
// 而测试会在包被改坏时照样绿。ja/de 只钉「en 骨架不再出现」——与 reqMessages.test.ts 同一思路。
describe("TimelineView Agent 调用节点回退文案", () => {
  afterEach(() => setLocale("en"));

  it("summary 非空时用它，不走回退", () => {
    setLocale("en");
    expect(previews([agentCallMessage("2026-09-01T10:00:00Z", "Explore the repo")])).toEqual([
      "Explore the repo",
    ]);
  });

  it("summary 为空时回退到本地化标签", () => {
    setLocale("en");
    expect(previews([agentCallMessage("2026-09-01T10:00:00Z", "")])).toEqual(["[Agent Call]"]);
    for (const locale of ["ja", "de"] as const) {
      setLocale(locale);
      const [preview] = previews([agentCallMessage("2026-09-01T10:00:00Z", "")]);
      expect(preview).not.toContain("Agent Call");
      // ja/de 的包取值被清空时 preview 是空串，上面那条恒真——这条兜住那个方向。
      // （「回退整条被删掉」不靠这条：本用例开头的 en 断言先兜住了，实测摘掉这条仍 RED。）
      expect(preview.length).toBeGreaterThan(0);
    }
  });
});

import { describe, expect, it } from "vitest";
import type { ContentPart } from "../types/session";
import { scanToolPart, TOOL_ERROR_MARKER } from "./toolPartScan";

const SVG = '<svg viewBox="0 0 10 10"><title>t</title><rect/></svg>';

function toolResult(content: string, isError = false): ContentPart {
  return { type: "tool_result", summary: "", content, is_error: isError };
}

describe("scanToolPart", () => {
  it("同一 part 重复扫描返回同一结论对象（记忆化）", () => {
    const part = toolResult("plain output");
    expect(scanToolPart(part)).toBe(scanToolPart(part));
  });

  it("非工具 part 返回恒定空结论，且不区分对象", () => {
    const a: ContentPart = { type: "text", text: "hi" };
    const b: ContentPart = { type: "thinking", thinking: "hmm" };
    expect(scanToolPart(a)).toBe(scanToolPart(b));
    expect(scanToolPart(a).hasWidget).toBe(false);
  });

  it("识别 SVG 图表载荷", () => {
    const scan = scanToolPart(toolResult(SVG));
    expect(scan.hasWidget).toBe(true);
    expect(scan.widgetData?.widgetCode).toBe(SVG);
    expect(scan.widgetData?.widgetType).toBe("svg");
  });

  it("识别图表回执标记，且回执本身不算图表数据", () => {
    const scan = scanToolPart(toolResult('{"type":"visualizer_show_widget_result"}'));
    expect(scan.isVisualizerAck).toBe(true);
    expect(scan.hasWidget).toBe(false);
  });

  it("识别正文里的 [Tool Error] 标记", () => {
    expect(scanToolPart(toolResult(`${TOOL_ERROR_MARKER} boom`)).hasToolErrorMarker).toBe(true);
    expect(scanToolPart(toolResult("all good")).hasToolErrorMarker).toBe(false);
  });

  it("tool_use 扫描的是 input", () => {
    const part: ContentPart = {
      type: "tool_use",
      summary: "",
      tool_name: "write",
      input: SVG,
      tool_use_id: "call-1",
    };
    const scan = scanToolPart(part);
    expect(scan.hasWidget).toBe(true);
    // tool_use 不具备这两个 tool_result 专属结论
    expect(scan.isVisualizerAck).toBe(false);
    expect(scan.hasToolErrorMarker).toBe(false);
  });

  it("结论被冻结，防止调用方写穿缓存", () => {
    expect(Object.isFrozen(scanToolPart(toolResult("x")))).toBe(true);
  });
});

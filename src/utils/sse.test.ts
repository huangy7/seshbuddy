import { describe, it, expect } from "vitest";
import { isSseBody, reconstructSse, prettyToolInput, assembleSseMessage } from "./sse";

const ANTHROPIC_STREAM = `event: message_start
data: {"message":{"model":"claude-x","usage":{"input_tokens":87,"cache_read_input_tokens":381568,"cache_creation_input_tokens":0,"output_tokens":0}},"type":"message_start"}

event: content_block_start
data: {"content_block":{"thinking":"","type":"thinking"},"index":0,"type":"content_block_start"}

event: content_block_delta
data: {"delta":{"thinking":"想一","type":"thinking_delta"},"index":0,"type":"content_block_delta"}

event: content_block_delta
data: {"delta":{"thinking":"想","type":"thinking_delta"},"index":0,"type":"content_block_delta"}

event: content_block_stop
data: {"index":0,"type":"content_block_stop"}

event: content_block_start
data: {"content_block":{"text":"","type":"text"},"index":1,"type":"content_block_start"}

event: content_block_delta
data: {"delta":{"text":"你好","type":"text_delta"},"index":1,"type":"content_block_delta"}

event: content_block_delta
data: {"delta":{"text":"世界","type":"text_delta"},"index":1,"type":"content_block_delta"}

event: content_block_stop
data: {"index":1,"type":"content_block_stop"}

event: content_block_start
data: {"content_block":{"name":"Bash","type":"tool_use"},"index":2,"type":"content_block_start"}

event: content_block_delta
data: {"delta":{"partial_json":"{\\"command\\": \\"ls\\"}","type":"input_json_delta"},"index":2,"type":"content_block_delta"}

event: content_block_stop
data: {"index":2,"type":"content_block_stop"}

event: message_delta
data: {"delta":{"stop_reason":"end_turn"},"type":"message_delta","usage":{"output_tokens":79}}

event: message_stop
data: {"type":"message_stop"}
`;

describe("isSseBody", () => {
  it("识别 SSE 流", () => {
    expect(isSseBody("event: message_start\ndata: {}")).toBe(true);
    expect(isSseBody("data: {}")).toBe(true);
  });
  it("识别非 SSE", () => {
    expect(isSseBody('{"ok": true}')).toBe(false);
    expect(isSseBody("")).toBe(false);
    expect(isSseBody(null)).toBe(false);
  });
});

describe("reconstructSse - Anthropic", () => {
  it("重建 thinking / text / tool_use 分块与 usage", () => {
    const r = reconstructSse(ANTHROPIC_STREAM);
    expect(r.model).toBe("claude-x");
    expect(r.blocks).toHaveLength(3);
    expect(r.blocks[0]).toMatchObject({ type: "thinking", text: "想一想" });
    expect(r.blocks[1]).toMatchObject({ type: "text", text: "你好世界" });
    expect(r.blocks[2]).toMatchObject({ type: "tool_use", name: "Bash" });
    expect(r.blocks[2].inputJson).toBe('{"command": "ls"}');
    expect(r.stopReason).toBe("end_turn");
    expect(r.usage).toMatchObject({
      input: 87,
      output: 79,
      cacheRead: 381568,
      cacheCreation: 0,
    });
  });
});

describe("reconstructSse - OpenAI chat/completions", () => {
  it("累积 content / reasoning / tool_calls / usage", () => {
    const stream = [
      'data: {"choices":[{"delta":{"role":"assistant"}}]}',
      'data: {"choices":[{"delta":{"reasoning_content":"思"}}]}',
      'data: {"choices":[{"delta":{"reasoning_content":"考"}}]}',
      'data: {"choices":[{"delta":{"content":"答"}}]}',
      'data: {"choices":[{"delta":{"content":"案"}}]}',
      'data: {"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"name":"read_file","arguments":"{\\"path\\":"}}]}}]}',
      'data: {"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\\"a.ts\\"}"}}]}}]}',
      'data: {"choices":[],"usage":{"prompt_tokens":120,"completion_tokens":30}}',
      "data: [DONE]",
    ].join("\n");
    const r = reconstructSse(stream);
    expect(r.blocks[0]).toMatchObject({ type: "thinking", text: "思考" });
    expect(r.blocks[1]).toMatchObject({ type: "text", text: "答案" });
    expect(r.blocks[2]).toMatchObject({ type: "tool_use", name: "read_file" });
    expect(r.blocks[2].inputJson).toBe('{"path":"a.ts"}');
    expect(r.usage).toMatchObject({ input: 120, output: 30 });
  });
});

describe("reconstructSse - Codex responses API", () => {
  it("累积 output_text / function_call / usage", () => {
    const stream = [
      'data: {"type":"response.output_item.added","item":{"type":"function_call","name":"shell"}}',
      'data: {"type":"response.function_call_arguments.delta","delta":"{\\"cmd\\":\\"pwd\\"}"}',
      'data: {"type":"response.output_text.delta","delta":"完成"}',
      'data: {"type":"response.completed","response":{"usage":{"input_tokens":500,"output_tokens":64,"input_tokens_details":{"cached_tokens":200}}}}',
    ].join("\n");
    const r = reconstructSse(stream);
    expect(r.blocks[0]).toMatchObject({ type: "tool_use", name: "shell" });
    expect(r.blocks[0].inputJson).toBe('{"cmd":"pwd"}');
    expect(r.blocks[1]).toMatchObject({ type: "text", text: "完成" });
    expect(r.usage).toMatchObject({ input: 500, output: 64, cacheRead: 200 });
  });
});

describe("reconstructSse - 容错", () => {
  it("非 SSE 输入返回空分块且不抛出", () => {
    const r = reconstructSse("not an sse stream at all");
    expect(r.blocks).toHaveLength(0);
    expect(r.usage).toBeUndefined();
  });
  it("坏 JSON 行被跳过", () => {
    const r = reconstructSse('data: {broken\ndata: {"type":"message_delta","usage":{"output_tokens":5}}');
    expect(r.usage?.output).toBe(5);
  });
});

describe("prettyToolInput", () => {
  it("美化完整 JSON", () => {
    expect(prettyToolInput('{"a":1}')).toBe('{\n  "a": 1\n}');
  });
  it("不完整 JSON 原样返回", () => {
    expect(prettyToolInput('{"a":')).toBe('{"a":');
  });
  it("空输入", () => {
    expect(prettyToolInput(undefined)).toBe("");
  });
});

describe("assembleSseMessage", () => {
  it("Anthropic：组装成 Messages 响应结构", () => {
    const m = assembleSseMessage(ANTHROPIC_STREAM);
    expect(m).not.toBeNull();
    expect(m).toMatchObject({
      type: "message",
      role: "assistant",
      model: "claude-x",
      stop_reason: "end_turn",
    });
    const content = m!.content as any[];
    expect(content).toHaveLength(3);
    expect(content[0]).toMatchObject({ type: "thinking", thinking: "想一想" });
    expect(content[1]).toMatchObject({ type: "text", text: "你好世界" });
    expect(content[2]).toMatchObject({ type: "tool_use", name: "Bash", input: { command: "ls" } });
    const usage = m!.usage as any;
    expect(usage).toMatchObject({ input_tokens: 87, output_tokens: 79, cache_read_input_tokens: 381568 });
  });

  it("OpenAI：组装成 chat.completion 结构", () => {
    const stream = [
      'data: {"choices":[{"delta":{"reasoning_content":"思"}}]}',
      'data: {"choices":[{"delta":{"content":"答"}}]}',
      'data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_1","function":{"name":"f","arguments":"{}"}}]}}]}',
      'data: {"choices":[{"finish_reason":"stop"}],"usage":{"prompt_tokens":10,"completion_tokens":5}}',
      "data: [DONE]",
    ].join("\n");
    const m = assembleSseMessage(stream)!;
    expect(m.object).toBe("chat.completion");
    const msg = (m.choices as any[])[0].message;
    expect(msg.content).toBe("答");
    expect(msg.reasoning_content).toBe("思");
    expect(msg.tool_calls[0]).toMatchObject({ id: "call_1", function: { name: "f", arguments: "{}" } });
    expect((m.choices as any[])[0].finish_reason).toBe("stop");
    expect(m.usage).toMatchObject({ prompt_tokens: 10, completion_tokens: 5 });
  });

  it("Codex responses：组装成 response 结构", () => {
    const stream = [
      'data: {"type":"response.output_item.added","item":{"type":"function_call","name":"shell","call_id":"c1"}}',
      'data: {"type":"response.function_call_arguments.delta","delta":"{\\"cmd\\":1}"}',
      'data: {"type":"response.output_text.delta","delta":"完成"}',
      'data: {"type":"response.completed","response":{"usage":{"input_tokens":500,"output_tokens":64}}}',
    ].join("\n");
    const m = assembleSseMessage(stream)!;
    expect(m.object).toBe("response");
    const output = m.output as any[];
    expect(output[0]).toMatchObject({ type: "function_call", name: "shell", call_id: "c1" });
    expect(output[1].content[0]).toMatchObject({ type: "output_text", text: "完成" });
  });

  it("无有效分块返回 null（调用方回退原始流）", () => {
    expect(assembleSseMessage("garbage")).toBeNull();
  });
});

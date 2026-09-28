import { beforeEach, describe, expect, it, vi } from "vitest";
import { appErrorCode, invokeApp, renderAppError } from "./invokeApp";

const mocks = vi.hoisted(() => ({ t: vi.fn(), invoke: vi.fn() }));

// 用桩替换 t，直接断言 renderAppError 把 key 与 params 交给了翻译层。
// 插值本身是 vue-i18n 的职责，不属于本模块的契约。
vi.mock("../i18n", () => ({ t: mocks.t }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));

describe("renderAppError", () => {
  beforeEach(() => {
    mocks.t.mockReset();
    // 语言包缺条目时 vue-i18n 返回 key 本身，缺省按此模拟
    mocks.t.mockImplementation((key: string) => key);
  });

  it("字符串错误原样返回（迁移期未改造的后端仍发字符串）", () => {
    expect(renderAppError("会话文件不存在")).toBe("会话文件不存在");
  });

  it("结构化错误把 errors 命名空间的 key 与 params 一并交给 t 插值", () => {
    mocks.t.mockReturnValue("Hello Ada");
    const out = renderAppError({
      code: "test.greeting",
      params: { name: "Ada" },
    });
    expect(mocks.t).toHaveBeenCalledWith("errors.test.greeting", { name: "Ada" });
    expect(out).toBe("Hello Ada");
  });

  it("未带 params 时以空对象兜底", () => {
    renderAppError({ code: "test.plain" });
    expect(mocks.t).toHaveBeenCalledWith("errors.test.plain", {});
  });

  it("未知 code 不崩，回退为 code 本身", () => {
    expect(renderAppError({ code: "nope.missing", params: {} })).toBe("nope.missing");
  });

  it("Error 实例取 message", () => {
    expect(renderAppError(new Error("boom"))).toBe("boom");
  });

  it("其它类型转字符串", () => {
    expect(renderAppError(null)).toBe("null");
  });
});

describe("invokeApp 的拒绝契约", () => {
  beforeEach(() => {
    mocks.t.mockReset();
    mocks.t.mockImplementation((key: string) => key);
    mocks.invoke.mockReset();
  });

  // Tauri 以**裸值**拒绝（迁移期是字符串），而不是 Error。包装层把它归一成 Error 后，
  // `String(e)` 会渲染成 "Error: 文案"——这正是展示点必须走 renderAppError 的原因。
  // 调用方拿到的 message 必须是文案本身，不能被加上前缀。
  it("后端以字符串拒绝时，调用方看到的 message 就是文案本身", async () => {
    mocks.invoke.mockRejectedValue("会话文件不存在");

    const err: unknown = await invokeApp("load_session").catch((e) => e);

    expect(err).toBeInstanceOf(Error);
    expect((err as Error).message).toBe("会话文件不存在");
    expect(renderAppError(err)).toBe("会话文件不存在");
  });

  // 批次 6 起后端改发 { code, params }，调用方同样不该看到 "[object Object]"。
  it("后端以结构化错误拒绝时，调用方看到的是翻译后的文案", async () => {
    mocks.invoke.mockRejectedValue({ code: "session.missing", params: { id: "a" } });
    mocks.t.mockReturnValue("会话不存在");

    const err: unknown = await invokeApp("load_session").catch((e) => e);

    expect(mocks.t).toHaveBeenCalledWith("errors.session.missing", { id: "a" });
    expect(renderAppError(err)).toBe("会话不存在");
  });

  // 协议状态判据（`路径不存在` 那三处）要的是 code，不是渲染好的文案：文案随语言变，
  // 从它里面找子串等于把语言耦合换个名字装回去。故 code 必须以**结构化字段**留给调用方。
  it("结构化错误把 code 挂在抛出的 Error 上", async () => {
    mocks.invoke.mockRejectedValue({
      code: "system_ops.path_missing",
      params: { path: "/x" },
    });
    mocks.t.mockReturnValue("Path does not exist: /x");

    const err: unknown = await invokeApp("open_path_in_file_manager").catch((e) => e);

    expect(err).toBeInstanceOf(Error);
    expect(appErrorCode(err)).toBe("system_ops.path_missing");
    expect((err as Error).message).toBe("Path does not exist: /x");
  });

  // 双形状契约的另一半：迁移期后端仍发裸字符串，那类拒绝没有 code 可给。
  it("字符串拒绝不带 code", async () => {
    mocks.invoke.mockRejectedValue("会话文件不存在");

    const err: unknown = await invokeApp("load_session").catch((e) => e);

    expect(appErrorCode(err)).toBeUndefined();
  });
});

describe("appErrorCode", () => {
  // streaming 通道的 error 载荷（`{ code, params }` 或裸字符串）不经 invokeApp，
  // 消费点拿到的是原始载荷，故判据必须同时认它。
  it("原始载荷与包装后的 Error 都认", () => {
    expect(appErrorCode({ code: "proxy.traffic_db_missing" })).toBe("proxy.traffic_db_missing");
    expect(appErrorCode(Object.assign(new Error("boom"), { code: "assistant.cli_missing" }))).toBe(
      "assistant.cli_missing",
    );
  });

  it("裸字符串、无 code 的 Error 与非对象一律返回 undefined", () => {
    expect(appErrorCode("流量数据库不存在")).toBeUndefined();
    expect(appErrorCode(new Error("boom"))).toBeUndefined();
    expect(appErrorCode(null)).toBeUndefined();
    expect(appErrorCode(undefined)).toBeUndefined();
  });
});

import { describe, expect, it, afterEach } from "vitest";
import { mount } from "@vue/test-utils";
import { setLocale } from "../../i18n";
import { CLI_IDS } from "../../types/cli";
import ChatAvatar from "./ChatAvatar.vue";

// 本文件只钉两件在迁移里容易被静默改坏的事：四个分支的**回退顺序**，以及
// **模型名 / 厂商名是插值进去的**（它们是身份值，不进语言包）。
//
// ja/de 的取值**不在这里抄一遍**——那是语言包的第二个事实来源，改了包还要改测试，
// 而测试会在包被改坏时照样绿。所以 ja/de 只断言「en 的骨架不再出现」，
// 与 reqMessages.test.ts 的逐条 `not.toContain(en 片段)` 同一思路。
function titleOf(props: Record<string, unknown>): string {
  return mount(ChatAvatar, { props }).find(".chat-avatar").attributes("title") ?? "";
}

const MODEL_PROPS = { role: "assistant", model: "claude-x" };
const PROVIDER_PROPS = { role: "assistant", cliId: "codex" };

afterEach(() => setLocale("en"));

describe("ChatAvatar 头像 tooltip", () => {
  it("user 角色取角色标签，不受模型/厂商影响", () => {
    setLocale("en");
    expect(titleOf({ role: "user" })).toBe("User");
    expect(titleOf({ role: "user", model: "claude-x" })).toBe("User");
  });

  it("assistant 按 model → cliId → 兜底 的顺序回退，身份值原样插值", () => {
    setLocale("en");
    expect(titleOf(MODEL_PROPS)).toBe("Model: claude-x");
    expect(titleOf(PROVIDER_PROPS)).toBe("Provider: codex");
    expect(titleOf({ role: "assistant" })).toBe("AI Assistant");
    // 两者都在时 model 优先：只用单参数的用例抓不到回退顺序被写反
    expect(titleOf({ role: "assistant", model: "claude-x", cliId: "codex" })).toBe("Model: claude-x");
  });

  it("ja/de 下四个分支都不再停在英文，且身份值原样插值", () => {
    for (const locale of ["ja", "de"] as const) {
      setLocale(locale);
      const model = titleOf(MODEL_PROPS);
      const provider = titleOf(PROVIDER_PROPS);
      const fallback = titleOf({ role: "assistant" });
      const user = titleOf({ role: "user" });
      // 逐条钉：某一条退回英文时，只有这一条会红
      expect(model).not.toContain("Model:");
      expect(provider).not.toContain("Provider:");
      expect(fallback).not.toContain("AI Assistant");
      expect(user).not.toContain("User");
      // 上面四条在**包值被清空**时全部恒真（空串不含任何骨架），所以每条都要一条正向断言。
      // model/provider 有身份值可插，正向断言就是它原样出现；
      // fallback/user 没有身份值，只能钉非空——实测：ja 对应键置空串时，只有这两条会红。
      expect(model).toContain("claude-x");
      expect(provider).toContain("codex");
      expect(fallback.length).toBeGreaterThan(0);
      expect(user.length).toBeGreaterThan(0);
    }
  });

  it("每个 CLI id 都映射到官方品牌标，不得落到通用兜底", () => {
    // 品牌表是手写的，漏登记一个 CLI **不会编译失败** —— 只会静默显示成通用图标，
    // 而通用图标看起来完全正常。遍历 CLI_IDS 逼作者给每个新 CLI 补上品牌矢量。
    for (const id of CLI_IDS) {
      const wrapper = mount(ChatAvatar, { props: { role: "assistant", cliId: id } });
      expect(wrapper.find(".ai-icon").exists(), `${id} 落到了通用兜底图标`).toBe(false);
      expect(wrapper.find(".brand-svg").exists(), `${id} 没有品牌标`).toBe(true);
    }
  });
});

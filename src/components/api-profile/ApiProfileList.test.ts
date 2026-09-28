import { describe, it, expect, vi } from "vitest";
import { mount } from "@vue/test-utils";
import ApiProfileList from "./ApiProfileList.vue";
import { setLocale } from "../../i18n";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

/**
 * 「作用域 chip」的**渲染**证据。
 *
 * 后端 `get_scope_bindings` 回的作用域名里，全局那一行**没有**名字（`name: None`，
 * 见 `src-tauri/src/commands/profile.rs`），它的标签由前端按语言渲染。本用例钉的是
 * 「全局那一行的名字上不了屏」这个实测结论——它决定了 `name: None` 不会让界面少任何东西，
 * 也解释了 `ApiProfileList.vue:216` 那个 `||` 的 `isGlobal ? t(...)` 分支今天不可达。
 */

// 这是**夹具输入**（旧后端真回过的那句中文），不是「为保测试绿而留下的中文」：
// 断言正是「它不出现在 DOM 里」，删掉它这条用例就变成恒真的空断言。
const backendGlobalName = "全局默认";

function mountList(profiles: any[]) {
  return mount(ApiProfileList, {
    props: {
      profiles,
      profileFields: [],
      scopeBindings: [
        { scope: "global", name: backendGlobalName, isGlobal: true, activeProfile: "Q" },
        { scope: "tab1", name: "Project Beta", isGlobal: false, activeProfile: "Q" },
      ],
    },
  });
}

describe("ApiProfileList 作用域 chip", () => {
  it("全局那一行的名字不上屏；项目作用域的名字（用户数据）照常渲染", async () => {
    setLocale("en");
    // 两条 profile 覆盖 `getProfileScopes` 的两条取值路径：
    // P 走 `usedInScopes`，Q 走 `scopeBindings` 兜底。
    const wrapper = mountList([
      { name: "P", content: {}, usedInScopes: [{ scope: "global", name: backendGlobalName }] },
      { name: "Q", content: {} },
    ]);
    await wrapper.vm.$nextTick();

    // 阳性对照：项目作用域的 chip 确实渲染出来了——否则下面那条「没有全局名」恒真。
    // P 的 `usedInScopes` 只有全局那条，过滤后为空，故只有 Q 贡献 chip。
    const chips = wrapper.findAll(".scope-chip").map((c) => c.text());
    expect(chips).toEqual(["Project Beta"]);
    expect(wrapper.text()).not.toContain(backendGlobalName);
  });
});

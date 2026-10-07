import { beforeEach, describe, expect, it } from "vitest";
import { mount } from "@vue/test-utils";
import { setLocale } from "../i18n";
import { CLI_DEFINITIONS, type CliId, type CliOption } from "../types/cli";
import FirstRunOnboarding from "./FirstRunOnboarding.vue";

const cliIds = Object.keys(CLI_DEFINITIONS) as CliId[];
const options: CliOption[] = cliIds.map((id) => ({
  ...CLI_DEFINITIONS[id],
  hasSessions: id === "claude" || id === "codex",
  hasBinary: false,
}));

describe("FirstRunOnboarding", () => {
  beforeEach(() => setLocale("en"));

  it("shows all language names in their own language and updates copy immediately", async () => {
    const wrapper = mount(FirstRunOnboarding, { props: { cliOptions: options } });
    expect(wrapper.text()).toContain("English");
    expect(wrapper.text()).toContain("简体中文");
    expect(wrapper.text()).toContain("日本語");
    expect(wrapper.text()).toContain("Deutsch");
    await wrapper.find('[data-locale="zh"]').trigger("click");
    expect(wrapper.text()).toContain("选择语言");
  });

  it("prefers CLI sources with sessions, but lists all supported sources", async () => {
    const wrapper = mount(FirstRunOnboarding, { props: { cliOptions: options } });
    await wrapper.find('[data-action="next"]').trigger("click");
    expect(wrapper.findAll('[data-cli-id]')).toHaveLength(cliIds.length);
    expect(wrapper.find('[data-cli-id="claude"] input').element).toHaveProperty("checked", true);
    expect(wrapper.find('[data-cli-id="dsh"] input').element).toHaveProperty("checked", false);
  });

  it("allows finishing with every source when no sessions exist", async () => {
    const empty = options.map((option) => ({ ...option, hasSessions: false }));
    const wrapper = mount(FirstRunOnboarding, { props: { cliOptions: empty } });
    await wrapper.find('[data-action="next"]').trigger("click");
    expect(wrapper.findAll('[data-cli-id] input:checked')).toHaveLength(cliIds.length);
    await wrapper.find('[data-action="finish"]').trigger("click");
    expect(wrapper.emitted("complete")?.[0]).toEqual([{ locale: "en", cliIds }]);
  });

  it("requires one selected source and preserves selections when going back", async () => {
    const wrapper = mount(FirstRunOnboarding, {
      props: { cliOptions: options },
    });
    await wrapper.find('[data-action="next"]').trigger("click");
    await wrapper.find('[data-cli-id="claude"] input').setValue(false);
    await wrapper.find('[data-cli-id="codex"] input').setValue(false);
    expect(wrapper.find('[data-action="finish"]').attributes("disabled")).toBeDefined();
    await wrapper.find('[data-cli-id="gemini"] input').setValue(true);
    await wrapper.find('[data-action="back"]').trigger("click");
    await wrapper.find('[data-action="next"]').trigger("click");
    expect(wrapper.find('[data-cli-id="gemini"] input').element).toHaveProperty("checked", true);
  });

  it("keeps keyboard focus inside the onboarding dialog", async () => {
    const host = document.createElement("div");
    document.body.append(host);
    const wrapper = mount(FirstRunOnboarding, {
      attachTo: host,
      props: { cliOptions: options },
    });
    const next = wrapper.find('[data-action="next"]');
    (next.element as HTMLButtonElement).focus();
    next.element.dispatchEvent(new KeyboardEvent("keydown", { key: "Tab", bubbles: true }));
    expect(document.activeElement).toBe(wrapper.find('[data-locale="en"]').element);
    wrapper.unmount();
    host.remove();
  });

  it("supports batch selecting all and selecting only sources with sessions", async () => {
    const wrapper = mount(FirstRunOnboarding, { props: { cliOptions: options } });
    await wrapper.find('[data-action="next"]').trigger("click");

    expect(wrapper.find(".source-count").text()).toBe(`2 / ${cliIds.length}`);

    // 点击全选 -> 全部勾选
    await wrapper.find('[data-action="toggle-select-all"]').trigger("click");
    expect(wrapper.findAll('[data-cli-id] input:checked')).toHaveLength(cliIds.length);
    expect(wrapper.find(".source-count").text()).toBe(`${cliIds.length} / ${cliIds.length}`);

    // 点击仅已有会话 -> 只保留存在会话的选项
    await wrapper.find('[data-action="select-sessions-only"]').trigger("click");
    expect(wrapper.findAll('[data-cli-id] input:checked')).toHaveLength(2);
    expect(wrapper.find(".source-count").text()).toBe(`2 / ${cliIds.length}`);

    // 全选状态下再次点击 -> 清空选择并禁用提交
    await wrapper.find('[data-action="toggle-select-all"]').trigger("click");
    expect(wrapper.findAll('[data-cli-id] input:checked')).toHaveLength(cliIds.length);
    await wrapper.find('[data-action="toggle-select-all"]').trigger("click");
    expect(wrapper.findAll('[data-cli-id] input:checked')).toHaveLength(0);
    expect(wrapper.find(".source-count").text()).toBe(`0 / ${cliIds.length}`);
    expect(wrapper.find('[data-action="finish"]').attributes("disabled")).toBeDefined();
  });
});

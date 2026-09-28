import { describe, expect, it } from "vitest";
import { mount } from "@vue/test-utils";
import ApiProfileModelOverridesSection from "./ApiProfileModelOverridesSection.vue";
import { t } from "../../i18n";

function mountSection(props: Record<string, unknown> = {}) {
  return mount(ApiProfileModelOverridesSection, {
    props: {
      cliName: "Claude Code",
      modelsLoading: false,
      modelsError: "",
      activeCombobox: null,
      groupedModels: [{ label: "Anthropic", items: ["claude-sonnet-4-5"] }],
      filteredModels: ["claude-sonnet-4-5"],
      haikuModel: "",
      sonnetModel: "claude-sonnet-4-5",
      opusModel: "",
      fallbackModel: "",
      ...props,
    },
  });
}

describe("ApiProfileModelOverridesSection 显示名称的自动/手填标识", () => {
  it("未手填时显示「自动」标签，不给恢复按钮", () => {
    const wrapper = mountSection({
      sonnetModelName: "Claude Sonnet 4.5",
      autoNames: { sonnet: "Claude Sonnet 4.5" },
      typedNames: { sonnet: false },
    });

    const tag = wrapper.find(".auto-tag");
    expect(tag.exists()).toBe(true);
    expect(tag.text()).toBe(t("api-profile.modelOverrides.autoTag"));
    expect(wrapper.find(".auto-restore").exists()).toBe(false);
    expect(wrapper.find(".cell-display input").classes()).toContain("is-auto");
  });

  it("没手填但名字和网关名字对不上（还没认定）时，不显示「自动」", () => {
    const wrapper = mountSection({
      sonnetModelName: "配置里的旧名字",
      autoNames: { sonnet: "Claude Sonnet 4.5" },
      typedNames: { sonnet: false },
    });

    expect(wrapper.find(".auto-tag").exists()).toBe(false);
    expect(wrapper.find(".cell-display input").classes()).not.toContain("is-auto");
  });

  it("手填且该模型有网关名字时，改成可点的「↺」图标按钮", () => {
    const wrapper = mountSection({
      sonnetModelName: "我的主力模型",
      autoNames: { sonnet: "Claude Sonnet 4.5" },
      typedNames: { sonnet: true },
    });

    expect(wrapper.find(".auto-tag").exists()).toBe(false);
    const btn = wrapper.find(".auto-restore");
    expect(btn.exists()).toBe(true);
    expect(btn.text()).toBe("↺");
    expect(btn.attributes("aria-label")).toBe(t("api-profile.modelOverrides.restoreAuto"));
    expect(wrapper.find(".cell-display input").classes()).not.toContain("is-auto");
  });

  it("手填但该模型没有网关名字时，不给恢复按钮", () => {
    const wrapper = mountSection({
      sonnetModelName: "我的主力模型",
      autoNames: { sonnet: "" },
      typedNames: { sonnet: true },
    });

    expect(wrapper.find(".auto-tag").exists()).toBe(false);
    expect(wrapper.find(".auto-restore").exists()).toBe(false);
  });

  it("点击「↺」把该行的角色抛给父组件", async () => {
    const wrapper = mountSection({
      sonnetModelName: "我的主力模型",
      autoNames: { sonnet: "Claude Sonnet 4.5" },
      typedNames: { sonnet: true },
    });

    await wrapper.find(".auto-restore").trigger("click");

    expect(wrapper.emitted("restoreAutoName")).toEqual([["sonnet"]]);
  });

  it("手填后清空，提示改为「不设置显示名称」", () => {
    const wrapper = mountSection({
      sonnetModelName: "",
      autoNames: { sonnet: "Claude Sonnet 4.5" },
      typedNames: { sonnet: true },
    });

    expect(wrapper.find(".cell-display input").attributes("placeholder")).toBe(
      t("api-profile.modelOverrides.typedPlaceholder"),
    );
  });

  it("未手填且无网关名字时，提示回落到模型 id", () => {
    const wrapper = mountSection({
      sonnetModelName: "",
      autoNames: { sonnet: "" },
      typedNames: { sonnet: false },
    });

    expect(wrapper.find(".cell-display input").attributes("placeholder")).toBe("claude-sonnet-4-5");
  });
});

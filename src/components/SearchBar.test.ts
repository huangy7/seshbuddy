import { describe, it, expect } from "vitest";
import { mount } from "@vue/test-utils";
import SearchBar from "./SearchBar.vue";
import { t } from "../i18n";

describe("SearchBar", () => {
  it("默认渲染搜索输入框和占位符", () => {
    const wrapper = mount(SearchBar, {
      props: { modelValue: "" },
      global: {
        stubs: {
          SvgIcon: true,
        },
      },
    });
    const input = wrapper.find(".search-input");
    expect(input.exists()).toBe(true);
    expect(input.attributes("placeholder")).toBe(t("common.searchBar.placeholder"));
    expect(wrapper.find(".search-shortcut-badge").exists()).toBe(true);
  });

  it("点击搜索条发出 openSearch 事件", async () => {
    const wrapper = mount(SearchBar, {
      props: { modelValue: "" },
      global: {
        stubs: {
          SvgIcon: true,
        },
      },
    });
    await wrapper.find(".search-bar").trigger("click");
    expect(wrapper.emitted("openSearch")).toBeDefined();
    expect(wrapper.emitted("openSearch")).toHaveLength(1);
  });

  it("点击输入框发出 openSearch 事件", async () => {
    const wrapper = mount(SearchBar, {
      props: { modelValue: "" },
      global: {
        stubs: {
          SvgIcon: true,
        },
      },
    });
    await wrapper.find(".search-input").trigger("click");
    expect(wrapper.emitted("openSearch")).toBeDefined();
    expect(wrapper.emitted("openSearch")).toHaveLength(1);
  });

  it("输入文本时发出 update:modelValue", async () => {
    const wrapper = mount(SearchBar, {
      props: { modelValue: "" },
      global: {
        stubs: {
          SvgIcon: true,
        },
      },
    });
    await wrapper.find(".search-input").setValue("我的任务");
    expect(wrapper.emitted("update:modelValue")).toEqual([["我的任务"]]);
  });

  it("有值时渲染清除按钮，点击清除发出空串并阻止 openSearch 冒泡", async () => {
    const wrapper = mount(SearchBar, {
      props: { modelValue: "已输入内容" },
      global: {
        stubs: {
          SvgIcon: true,
        },
      },
    });
    const clearBtn = wrapper.find(".clear-btn");
    expect(clearBtn.exists()).toBe(true);
    await clearBtn.trigger("click");
    expect(wrapper.emitted("update:modelValue")).toEqual([[""]]);
    expect(wrapper.emitted("openSearch")).toBeUndefined();
  });
});

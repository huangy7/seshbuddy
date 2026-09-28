import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { mount } from "@vue/test-utils";
import { nextTick } from "vue";
import ElegantSelect from "./ElegantSelect.vue";
import { setLocale, t } from "../../i18n";

// placeholder / emptyHint 的回退刻意放在模板里求值，而不是写成 prop 默认值：
// 默认值只在模块加载时算一次，会冻结在 import 时的语言。这个组件调用点很多，
// 回退一旦被「清理」回默认值就会静默停在英文，故用真实语言包钉住切换后的取值。
function mountSelect(props: Record<string, unknown> = {}) {
  return mount(ElegantSelect, {
    props: { options: [], ...props },
    global: { stubs: { teleport: true } },
  });
}

describe("ElegantSelect 的文案回退跟随语言", () => {
  beforeEach(() => setLocale("en"));
  afterEach(() => setLocale("en"));

  it("未选中任何值时占位符随 setLocale 切换", async () => {
    const wrapper = mountSelect();
    expect(wrapper.find(".placeholder-text").text()).toBe("Select…");

    setLocale("zh");
    await nextTick();

    expect(wrapper.find(".placeholder-text").text()).toBe(t("common.select.placeholder"));
    expect(wrapper.find(".placeholder-text").text()).not.toBe("Select…");
  });

  it("无选项时的空态提示随 setLocale 切换", async () => {
    const wrapper = mountSelect();
    await wrapper.find(".elegant-select-trigger").trigger("click");
    expect(wrapper.find(".empty-hint").text()).toBe("No options");

    setLocale("zh");
    await nextTick();

    expect(wrapper.find(".empty-hint").text()).toBe(t("common.select.empty"));
    expect(wrapper.find(".empty-hint").text()).not.toBe("No options");
  });

  it("显式传入的 placeholder 优先于语言包", () => {
    const wrapper = mountSelect({ placeholder: "Pick one" });
    expect(wrapper.find(".placeholder-text").text()).toBe("Pick one");
  });
});

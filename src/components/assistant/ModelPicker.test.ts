import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { nextTick } from "vue";
import { flushPromises, mount } from "@vue/test-utils";

const mocks = vi.hoisted(() => ({ invokeApp: vi.fn() }));

vi.mock("../../utils/invokeApp", () => ({ invokeApp: mocks.invokeApp }));

// 运行环境的 localStorage 是个空对象（被 Node 的同名全局遮住），而组件 onMounted
// 要读 assistant.lastModel，故在引入组件前换成 Map 实现。
const storage = new Map<string, string>();
vi.stubGlobal("localStorage", {
  getItem: (key: string) => storage.get(key) ?? null,
  setItem: (key: string, value: string) => storage.set(key, value),
  removeItem: (key: string) => storage.delete(key),
  clear: () => storage.clear(),
  key: (index: number) => [...storage.keys()][index] ?? null,
  get length() { return storage.size; },
});

const { t, setLocale } = await import("../../i18n");
const ModelPicker = (await import("./ModelPicker.vue")).default;

/** 让 read_profile 返回一份只有 env 的配置。 */
function profileReturns(env: Record<string, string>) {
  mocks.invokeApp.mockResolvedValue(JSON.stringify({ env }));
}

async function mountPicker(props: { profile: string; modelValue?: string }) {
  const wrapper = mount(ModelPicker, {
    props: { profile: props.profile, modelValue: props.modelValue ?? "" },
  });
  await flushPromises();
  await nextTick();
  return wrapper;
}

describe("ModelPicker", () => {
  beforeEach(() => {
    // onMounted 会读 assistant.lastModel，残留值会把选中项改掉
    localStorage.clear();
    mocks.invokeApp.mockReset();
    setLocale("en");
  });

  afterEach(() => setLocale("en"));

  it("主模型标签随语言切换更新（加载时求值会把它冻在旧语言）", async () => {
    profileReturns({ ANTHROPIC_MODEL: "claude-sonnet-4.5" });
    const wrapper = await mountPicker({ profile: "work" });

    const enLabel = wrapper.find(".chip-name").text();
    expect(enLabel).toBe(t("assistant.modelPicker.mainModel", { model: "Sonnet 4.5" }));

    setLocale("zh");
    await nextTick();

    expect(wrapper.find(".chip-name").text()).toBe(
      t("assistant.modelPicker.mainModel", { model: "Sonnet 4.5" }),
    );
    // 两种语言取值不同：少了这条，「语言没跟着变」也能让上面那条通过
    expect(wrapper.find(".chip-name").text()).not.toBe(enLabel);
  });

  it("下拉里的选项标签同样随语言切换更新", async () => {
    profileReturns({ ANTHROPIC_MODEL: "claude-sonnet-4.5" });
    const wrapper = await mountPicker({ profile: "work" });
    await wrapper.find(".model-chip").trigger("click");

    // 第 0 项是「默认模型」，第 1 项才是配置里的主模型
    const optionLabels = () =>
      wrapper.findAll(".model-option .option-label").map((node) => node.text());

    expect(optionLabels()[1]).toBe("Main model (Sonnet 4.5)");

    setLocale("zh");
    await nextTick();

    expect(optionLabels()[1]).toBe("主模型 (Sonnet 4.5)");
    expect(optionLabels()[0]).toBe(t("assistant.modelPicker.defaultModel"));
  });

  it("选中的 id 不在配置里时按 id 推导显示名，不取语言包", async () => {
    // profile 为空 → 配置读不回来 → 走 displayLabel 的 formatModelLabel 分支。
    // 该分支是 id 归一化，与语言无关，重塑时不能被顺手改成语言包文案。
    profileReturns({ ANTHROPIC_MODEL: "claude-sonnet-4.5" });
    const wrapper = await mountPicker({ profile: "", modelValue: "anthropic.claude-haiku-4.5" });

    expect(wrapper.find(".chip-name").text()).toBe("Haiku 4.5");
    expect(wrapper.find(".chip-name").text()).not.toBe(t("assistant.modelPicker.defaultModel"));

    setLocale("zh");
    await nextTick();

    expect(wrapper.find(".chip-name").text()).toBe("Haiku 4.5");
  });

  it("自定义模型名原样显示，不随语言变化", async () => {
    profileReturns({
      ANTHROPIC_MODEL: "claude-sonnet-4.5",
      ANTHROPIC_MODEL_NAME: "Fast One",
    });
    const wrapper = await mountPicker({ profile: "work" });

    expect(wrapper.find(".chip-name").text()).toBe("Fast One");

    setLocale("zh");
    await nextTick();

    expect(wrapper.find(".chip-name").text()).toBe("Fast One");
  });

  it("id 不含角色词时用角色名兜底", async () => {
    profileReturns({ ANTHROPIC_DEFAULT_SONNET_MODEL: "my-custom-endpoint" });
    const wrapper = await mountPicker({ profile: "work" });

    expect(wrapper.find(".chip-name").text()).toBe("Sonnet");
  });
});

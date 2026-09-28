import { describe, expect, it } from "vitest";
import { mount } from "@vue/test-utils";
import InputDialog from "./InputDialog.vue";

describe("InputDialog", () => {
  it("renders when open is true with title, description, and default value", () => {
    const wrapper = mount(InputDialog, {
      props: {
        open: true,
        title: "重命名对话",
        description: "为该对话设置新的显示名称",
        defaultValue: "现有对话名称",
      },
      global: {
        stubs: {
          Teleport: true,
        },
      },
    });

    expect(wrapper.find(".input-dialog-title").text()).toBe("重命名对话");
    expect(wrapper.find(".input-dialog-desc").text()).toBe("为该对话设置新的显示名称");
    const input = wrapper.find<HTMLInputElement>(".dialog-input");
    expect(input.exists()).toBe(true);
    expect(input.element.value).toBe("现有对话名称");
  });

  it("emits confirm with trimmed new value when clicking confirm button", async () => {
    const wrapper = mount(InputDialog, {
      props: {
        open: true,
        title: "重命名对话",
        defaultValue: "旧名称",
      },
      global: {
        stubs: {
          Teleport: true,
        },
      },
    });

    const input = wrapper.find<HTMLInputElement>(".dialog-input");
    await input.setValue(" 新对话名称 ");
    await wrapper.find(".confirm-btn").trigger("click");

    expect(wrapper.emitted("confirm")?.[0]).toEqual(["新对话名称"]);
  });

  it("emits confirm on Enter key press", async () => {
    const wrapper = mount(InputDialog, {
      props: {
        open: true,
        title: "重命名对话",
        defaultValue: "旧名称",
      },
      global: {
        stubs: {
          Teleport: true,
        },
      },
    });

    const input = wrapper.find<HTMLInputElement>(".dialog-input");
    await input.setValue("回车提交新名称");
    await input.trigger("keydown.enter");

    expect(wrapper.emitted("confirm")?.[0]).toEqual(["回车提交新名称"]);
  });

  it("emits confirm with empty string when clearing the value and submitting", async () => {
    const wrapper = mount(InputDialog, {
      props: {
        open: true,
        title: "重命名对话",
        defaultValue: "旧名称",
      },
      global: {
        stubs: {
          Teleport: true,
        },
      },
    });

    const input = wrapper.find<HTMLInputElement>(".dialog-input");
    await input.setValue("   ");
    await wrapper.find(".confirm-btn").trigger("click");

    expect(wrapper.emitted("confirm")?.[0]).toEqual([""]);
  });

  it("emits cancel on Escape key or cancel button click", async () => {
    const wrapper = mount(InputDialog, {
      props: {
        open: true,
        title: "重命名对话",
        defaultValue: "旧名称",
      },
      global: {
        stubs: {
          Teleport: true,
        },
      },
    });

    await wrapper.find(".cancel-btn").trigger("click");
    expect(wrapper.emitted("cancel")).toBeTruthy();

    await wrapper.find(".dialog-input").trigger("keydown.esc");
    expect(wrapper.emitted("cancel")).toHaveLength(2);
  });

  it("ignores backdrop click during opening cooldown (<250ms)", async () => {
    const wrapper = mount(InputDialog, {
      props: {
        open: true,
        title: "重命名对话",
        defaultValue: "旧名称",
      },
      global: {
        stubs: {
          Teleport: true,
        },
      },
    });

    const overlay = wrapper.find(".modal-backdrop");
    await overlay.trigger("mousedown");
    await overlay.trigger("click");
    expect(wrapper.emitted("cancel")).toBeFalsy();
  });

  it("emits cancel when clicking backdrop after cooldown with mousedown on overlay", async () => {
    const wrapper = mount(InputDialog, {
      props: {
        open: true,
        title: "重命名对话",
        defaultValue: "旧名称",
      },
      global: {
        stubs: {
          Teleport: true,
        },
      },
    });

    // 等待超过 250ms 冷却期
    await new Promise((resolve) => setTimeout(resolve, 260));

    const overlay = wrapper.find(".modal-backdrop");
    await overlay.trigger("mousedown");
    await overlay.trigger("click");
    expect(wrapper.emitted("cancel")).toBeTruthy();
  });

  it("does not emit cancel if mousedown was inside card and click released on overlay", async () => {
    const wrapper = mount(InputDialog, {
      props: {
        open: true,
        title: "重命名对话",
        defaultValue: "旧名称",
      },
      global: {
        stubs: {
          Teleport: true,
        },
      },
    });

    await new Promise((resolve) => setTimeout(resolve, 260));

    // 卡片内 mousedown（如划词或点输入框）
    const card = wrapper.find(".input-dialog-card");
    await card.trigger("mousedown");

    // 鼠标滑出卡片在 overlay 释放
    const overlay = wrapper.find(".modal-backdrop");
    await overlay.trigger("click");

    expect(wrapper.emitted("cancel")).toBeFalsy();
  });
});

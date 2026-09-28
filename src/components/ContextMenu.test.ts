import { describe, expect, it, vi } from "vitest";
import { mount } from "@vue/test-utils";
import ContextMenu from "./ContextMenu.vue";

describe("ContextMenu.vue", () => {
  it("emits close and executes action when clicking normal item", async () => {
    const action = vi.fn();
    const wrapper = mount(ContextMenu, {
      props: {
        visible: true,
        x: 100,
        y: 100,
        items: [
          { label: "重命名", action },
        ],
      },
    });

    const item = wrapper.find(".menu-item");
    await item.trigger("click");

    expect(wrapper.emitted("close")).toHaveLength(1);
    expect(action).toHaveBeenCalledTimes(1);
  });

  it("does not emit close when closeOnClick is false", async () => {
    const action = vi.fn();
    const wrapper = mount(ContextMenu, {
      props: {
        visible: true,
        x: 100,
        y: 100,
        items: [
          { label: "复制路径", action, closeOnClick: false },
        ],
      },
    });

    const item = wrapper.find(".menu-item");
    await item.trigger("click");

    expect(wrapper.emitted("close")).toBeUndefined();
    expect(action).toHaveBeenCalledTimes(1);
  });
});

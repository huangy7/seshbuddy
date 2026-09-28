import { describe, it, expect, vi, beforeEach } from "vitest";
import { mount } from "@vue/test-utils";
import ChatImageFullscreenModal from "./ChatImageFullscreenModal.vue";

describe("ChatImageFullscreenModal", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("renders regular image using img tag", () => {
    const wrapper = mount(ChatImageFullscreenModal, {
      props: {
        imageUrl: "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==",
      },
    });

    const img = wrapper.find("img.fullscreen-image");
    expect(img.exists()).toBe(true);
    expect(wrapper.find(".fullscreen-svg-wrapper").exists()).toBe(false);
  });

  it("renders raw SVG markup using native svg wrapper instead of broken img tag", () => {
    const rawSvg = `<svg viewBox="0 0 100 50"><text>Hello SVG</text></svg>`;
    const wrapper = mount(ChatImageFullscreenModal, {
      props: {
        imageUrl: rawSvg,
      },
    });

    expect(wrapper.find("img.fullscreen-image").exists()).toBe(false);
    const svgWrapper = wrapper.find(".fullscreen-svg-wrapper");
    expect(svgWrapper.exists()).toBe(true);
    expect(svgWrapper.html()).toContain("<text>Hello SVG</text>");
  });

  it("decodes data:image/svg+xml;base64 URL and renders native svg wrapper", () => {
    const rawSvg = `<svg viewBox="0 0 100 50"><circle cx="25" cy="25" r="20"/></svg>`;
    const base64Url = `data:image/svg+xml;base64,${Buffer.from(rawSvg).toString("base64")}`;
    const wrapper = mount(ChatImageFullscreenModal, {
      props: {
        imageUrl: base64Url,
      },
    });

    expect(wrapper.find("img.fullscreen-image").exists()).toBe(false);
    const svgWrapper = wrapper.find(".fullscreen-svg-wrapper");
    expect(svgWrapper.exists()).toBe(true);
    expect(svgWrapper.html()).toContain("<circle cx=\"25\"");
  });

  it("emits close on backdrop click and close button click", async () => {
    const wrapper = mount(ChatImageFullscreenModal, {
      props: {
        imageUrl: "data:image/png;base64,abc",
      },
    });

    await wrapper.find(".fullscreen-close-btn").trigger("click");
    expect(wrapper.emitted("close")).toHaveLength(1);

    await wrapper.find(".modal-backdrop").trigger("click");
    expect(wrapper.emitted("close")).toHaveLength(2);
  });

  it("emits download event with current image/svg payload", async () => {
    const rawSvg = `<svg viewBox="0 0 10 10"></svg>`;
    const wrapper = mount(ChatImageFullscreenModal, {
      props: {
        imageUrl: rawSvg,
      },
    });

    const downloadBtn = wrapper.findAll(".fullscreen-action-btn").find(b => b.attributes("title")?.includes("Download"));
    expect(downloadBtn).toBeDefined();
    await downloadBtn!.trigger("click");
    expect(wrapper.emitted("download")?.[0]).toEqual([rawSvg]);
  });

  it("handles zoom controls and reset to fit", async () => {
    const rawSvg = `<svg viewBox="0 0 100 50"></svg>`;
    const wrapper = mount(ChatImageFullscreenModal, {
      props: {
        imageUrl: rawSvg,
      },
    });

    const zoomInBtn = wrapper.findAll(".fullscreen-action-btn").find(b => b.attributes("title")?.includes("Zoom in"));
    expect(zoomInBtn).toBeDefined();
    await zoomInBtn!.trigger("click");

    const zoomTextBtn = wrapper.find(".fullscreen-zoom-text-btn");
    expect(zoomTextBtn.text()).toBe("125%");

    const fitBtn = wrapper.findAll(".fullscreen-action-btn").find(b => b.attributes("title")?.includes("Fit to window"));
    expect(fitBtn).toBeDefined();
    await fitBtn!.trigger("click");
    expect(zoomTextBtn.text()).toBe("Fit");
  });
});

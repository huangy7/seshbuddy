import { describe, it, expect, vi, beforeEach } from "vitest";
import { mount } from "@vue/test-utils";
import ImagePreview from "./ImagePreview.vue";
import { t } from "../i18n";

vi.mock("@tauri-apps/api/core", () => ({
  convertFileSrc: vi.fn((path: string) => `asset://${path}`),
}));

describe("ImagePreview Component", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("renders image with asset url", () => {
    const wrapper = mount(ImagePreview, {
      props: {
        filePath: "/workspace/design/mockup.png",
        projectRoot: "/workspace",
      },
    });

    const img = wrapper.find("img.preview-image");
    expect(img.exists()).toBe(true);
    expect(img.attributes("src")).toBe("asset:///workspace/design/mockup.png");
  });

  it("handles image load and displays natural dimensions", async () => {
    const wrapper = mount(ImagePreview, {
      props: {
        filePath: "/workspace/design/mockup.png",
        projectRoot: "/workspace",
      },
    });

    const img = wrapper.find("img.preview-image");
    Object.defineProperty(img.element, "naturalWidth", { value: 1920, configurable: true });
    Object.defineProperty(img.element, "naturalHeight", { value: 1080, configurable: true });
    await img.trigger("load");

    const badge = wrapper.find(".dimension-badge");
    expect(badge.exists()).toBe(true);
    expect(badge.text()).toContain("1920 × 1080 px");
  });

  it("handles zoom controls", async () => {
    const wrapper = mount(ImagePreview, {
      props: {
        filePath: "/workspace/design/mockup.png",
        projectRoot: "/workspace",
      },
    });

    const zoomInBtn = wrapper.findAll(".zoom-btn")[2]; // zoom-in
    await zoomInBtn.trigger("click");

    const zoomTextBtn = wrapper.find(".zoom-text-btn");
    expect(zoomTextBtn.text()).toBe("125%");

    const fitBtn = wrapper.findAll(".zoom-btn")[3]; // maximize-2 / fit
    await fitBtn.trigger("click");
    expect(zoomTextBtn.text()).toBe(t("common.imagePreview.fit"));
  });

  it("displays error state when image fails to load", async () => {
    const wrapper = mount(ImagePreview, {
      props: {
        filePath: "/workspace/corrupted.png",
        projectRoot: "/workspace",
      },
    });

    const img = wrapper.find("img.preview-image");
    await img.trigger("error");

    expect(wrapper.find(".image-error-state").exists()).toBe(true);
    expect(wrapper.text()).toContain(t("common.imagePreview.loadFailed"));
  });
});

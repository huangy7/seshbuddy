import { beforeEach, describe, expect, it, vi } from "vitest";
import { mount, flushPromises } from "@vue/test-utils";
import { ref } from "vue";
import BlockedFoldersSettings from "./BlockedFoldersSettings.vue";
import { t } from "../i18n";

const blockedFoldersRef = ref<string[]>([]);
const mockBlockFolder = vi.fn().mockResolvedValue(undefined);
const mockUnblockFolder = vi.fn().mockResolvedValue(undefined);
const mockLoadBlockedFolders = vi.fn().mockResolvedValue(undefined);
const mockOpenDialog = vi.fn();

vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: (...args: unknown[]) => mockOpenDialog(...args),
}));

vi.mock("../composables/useBlockedFolders", () => ({
  useBlockedFolders: () => ({
    blockedFolders: blockedFoldersRef,
    blockFolder: mockBlockFolder,
    unblockFolder: mockUnblockFolder,
    loadBlockedFolders: mockLoadBlockedFolders,
  }),
}));

beforeEach(() => {
  blockedFoldersRef.value = [];
  mockBlockFolder.mockReset().mockResolvedValue(undefined);
  mockUnblockFolder.mockReset().mockResolvedValue(undefined);
  mockLoadBlockedFolders.mockReset().mockResolvedValue(undefined);
  mockOpenDialog.mockReset();
});

describe("BlockedFoldersSettings.vue", () => {
  it("无屏蔽文件夹时展示空状态提示", async () => {
    blockedFoldersRef.value = [];
    const wrapper = mount(BlockedFoldersSettings, {
      global: {
        stubs: { SvgIcon: true },
      },
    });

    expect(wrapper.find(".empty-state").exists()).toBe(true);
    expect(wrapper.text()).toContain(t("settings.blockedFolders.emptyTitle"));
    expect(wrapper.findAll(".blocked-folder-item")).toHaveLength(0);
  });

  it("有屏蔽文件夹时展示列表并可点击解除屏蔽", async () => {
    blockedFoldersRef.value = ["/workspace/secret-1", "/workspace/secret-2"];
    const wrapper = mount(BlockedFoldersSettings, {
      global: {
        stubs: { SvgIcon: true },
      },
    });

    const items = wrapper.findAll(".blocked-folder-item");
    expect(items).toHaveLength(2);
    expect(items[0].text()).toContain("/workspace/secret-1");
    expect(items[1].text()).toContain("/workspace/secret-2");

    // 点击第一个条目的解除屏蔽按钮
    const unblockBtn = items[0].find("button.unblock-btn");
    expect(unblockBtn.exists()).toBe(true);
    await unblockBtn.trigger("click");

    expect(mockUnblockFolder).toHaveBeenCalledWith("/workspace/secret-1");
  });

  it("点击添加文件夹按钮，选择目录后调用 blockFolder", async () => {
    mockOpenDialog.mockResolvedValue("/workspace/new-secret");
    const wrapper = mount(BlockedFoldersSettings, {
      global: {
        stubs: { SvgIcon: true },
      },
    });

    const addBtn = wrapper.find("button.add-folder-btn");
    expect(addBtn.exists()).toBe(true);
    await addBtn.trigger("click");
    await flushPromises();

    expect(mockOpenDialog).toHaveBeenCalledWith({
      directory: true,
      multiple: false,
      title: t("settings.blockedFolders.pickFolderTitle"),
    });
    expect(mockBlockFolder).toHaveBeenCalledWith("/workspace/new-secret");
  });
});

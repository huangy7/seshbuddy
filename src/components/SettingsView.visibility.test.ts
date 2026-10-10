import { ref } from "vue";
import { describe, expect, it, vi } from "vitest";
import { mount } from "@vue/test-utils";
import { t } from "../i18n";

const storage = new Map<string, string>();
vi.stubGlobal("localStorage", {
  getItem: (key: string) => storage.get(key) ?? null,
  setItem: (key: string, value: string) => storage.set(key, value),
  removeItem: (key: string) => storage.delete(key),
  clear: () => storage.clear(),
});
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => () => {}) }));

vi.mock("../composables/useSessions", () => ({
  useSessions: () => ({ cliOptions: ref([]) }),
}));
vi.mock("../composables/useUpdater", () => ({
  useUpdater: () => ({ hasAvailableUpdate: ref(false) }),
}));

const SettingsView = (await import("./SettingsView.vue")).default;

describe("SettingsView display settings shortcut", () => {
  it("opens the data tab and scrolls to session list visibility", async () => {
    const scrollIntoView = vi.fn();
    HTMLElement.prototype.scrollIntoView = scrollIntoView;
    const wrapper = mount(SettingsView, {
      global: {
        stubs: {
          SvgIcon: true,
          GeneralSettings: true,
          DataSourceSettings: true,
          ApiProfileManager: true,
          AboutView: true,
          DataIndexSettings: { template: "<div data-visibility-settings></div>" },
        },
      },
    });

    await (wrapper.vm as typeof wrapper.vm & { focusVisibilitySettings: () => Promise<void> }).focusVisibilitySettings();

    expect(wrapper.find(".sidebar-tab.active").text()).toBe(t("settings.nav.tabData"));
    expect(scrollIntoView).toHaveBeenCalledOnce();
  });
});

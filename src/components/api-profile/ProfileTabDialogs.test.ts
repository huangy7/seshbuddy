import { describe, it, expect, vi, beforeEach } from "vitest";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { join } from "node:path";
import { mount, flushPromises } from "@vue/test-utils";
import ProfileTabCreateDialog from "./ProfileTabCreateDialog.vue";
import ProfileTabEditDialog from "./ProfileTabEditDialog.vue";
import { invoke } from "@tauri-apps/api/core";
import { t } from "../../i18n";

// 三处 `.vue` 与后端 `system_ops.rs` 之间的协议标记。它是跨进程协议状态（「目录尚未创建」），
// 不是界面文案：早先三处按后端错误原文做子串匹配，译文一变（或换门语言）分支就静默失效，
// 用户可见的专门提示退化成原始错误串，而没有任何测试会红。
const SYSTEM_OPS_PATH_MISSING = "system_ops.path_missing";
// 不写成 `new URL("../../../…", import.meta.url)`：那个字面形状是 Vite 的资源引用语法，
// 会被改写成非 file: 的模块 URL，fileURLToPath 直接抛「The URL must be of scheme file」。
// 经一个变量转手可避开该改写，实测取到的是本文件真实路径。
const TEST_FILE_URL = import.meta.url;
const REPO_ROOT = fileURLToPath(new URL("../../../", TEST_FILE_URL));
const SITES_DIR = join(REPO_ROOT, "src/components/api-profile");
const SYSTEM_OPS_RS = join(REPO_ROOT, "src-tauri/src/commands/system_ops.rs");
const SITE_FILES = [
  "ApiProfileList.vue",
  "ProfileTabCreateDialog.vue",
  "ProfileTabEditDialog.vue",
];

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: vi.fn(),
}));

vi.mock("../../composables/useSessions", () => ({
  useSessions: () => ({
    projects: {
      value: [
        { original_path: "/workspace/valid-project" },
        { original_path: "/workspace/missing-project" },
      ],
    },
  }),
}));

describe("ProfileTabDialogs - 本地目录失效检测与 Finder 打开错误处理", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    Object.assign(navigator, {
      clipboard: {
        writeText: vi.fn().mockResolvedValue(undefined),
      },
    });
  });

  describe("ProfileTabCreateDialog", () => {
    it("检测到不存在的目录时显示「已失效」徽标与失效样式", async () => {
      vi.mocked(invoke).mockImplementation(async (cmd: string, _args: any) => {
        if (cmd === "check_paths_exist") {
          return {
            "/workspace/valid-project": true,
            "/workspace/missing-project": false,
          };
        }
        return [];
      });

      const wrapper = mount(ProfileTabCreateDialog, {
        props: { existingNames: [] },
      });

      await flushPromises();

      const items = wrapper.findAll(".project-item");
      expect(items.length).toBe(2);

      const missingItem = items.find((el) => el.text().includes("missing-project"));
      expect(missingItem).toBeDefined();
      expect(missingItem?.classes()).toContain("is-missing");
      expect(missingItem?.find(".missing-badge").exists()).toBe(true);
      expect(missingItem?.find(".missing-badge").text()).toBe(t("api-profile.dialog.common.staleBadge"));

      const validItem = items.find((el) => el.text().includes("valid-project"));
      expect(validItem?.classes()).not.toContain("is-missing");
      expect(validItem?.find(".missing-badge").exists()).toBe(false);
    });

    it("点击通过 Finder 打开不存在的目录时，给出错误通知并自动复制路径", async () => {
      vi.mocked(invoke).mockImplementation(async (cmd: string, args: any) => {
        if (cmd === "check_paths_exist") {
          return {
            "/workspace/valid-project": true,
            "/workspace/missing-project": false,
          };
        }
        if (cmd === "open_path_in_file_manager") {
          if (args?.path === "/workspace/missing-project") {
            // 后端以 AppError::Coded 的线格式拒绝（`system_ops.rs` 的 open_path_in_file_manager）。
            return Promise.reject({
              code: SYSTEM_OPS_PATH_MISSING,
              params: { path: "/workspace/missing-project" },
            });
          }
          return Promise.resolve();
        }
        return [];
      });

      const wrapper = mount(ProfileTabCreateDialog, {
        props: { existingNames: [] },
      });

      await flushPromises();

      const missingItem = wrapper.findAll(".project-item").find((el) => el.text().includes("missing-project"));
      const openBtn = missingItem?.findAll(".project-action-btn")[1];
      expect(openBtn).toBeDefined();

      await openBtn?.trigger("click");
      await flushPromises();

      expect(navigator.clipboard.writeText).toHaveBeenCalledWith("/workspace/missing-project");

      const notice = wrapper.find(".action-notice");
      expect(notice.exists()).toBe(true);
      expect(notice.classes()).toContain("is-error");
      expect(notice.text()).toContain(t("api-profile.dialog.common.openFailedMissing"));
    });
  });

  describe("ProfileTabEditDialog", () => {
    const tabMock = {
      id: "tab-1",
      cli_id: "claude",
      name: "TestScope",
      sort_order: 1,
      dirs: ["/workspace/missing-project"],
    };

    it("检测到绑定的失效目录时显示「已失效」徽标并在打开失败时提示", async () => {
      vi.mocked(invoke).mockImplementation(async (cmd: string, _args: any) => {
        if (cmd === "check_paths_exist") {
          return {
            "/workspace/valid-project": true,
            "/workspace/missing-project": false,
          };
        }
        if (cmd === "open_path_in_file_manager") {
          // 同上：coded 形状，前端按 code 判「目录尚未创建」。
          return Promise.reject({
            code: SYSTEM_OPS_PATH_MISSING,
            params: { path: "/workspace/missing-project" },
          });
        }
        return [];
      });

      const wrapper = mount(ProfileTabEditDialog, {
        props: { tab: tabMock, existingNames: [] },
      });

      await flushPromises();

      const missingItem = wrapper.findAll(".project-item").find((el) => el.text().includes("missing-project"));
      expect(missingItem?.classes()).toContain("is-missing");
      expect(missingItem?.find(".missing-badge").text()).toBe(t("api-profile.dialog.common.staleBadge"));

      const openBtn = missingItem?.findAll(".project-action-btn")[1];
      await openBtn?.trigger("click");
      await flushPromises();

      expect(navigator.clipboard.writeText).toHaveBeenCalledWith("/workspace/missing-project");
      const notice = wrapper.find(".action-notice");
      expect(notice.exists()).toBe(true);
      expect(notice.text()).toContain(t("api-profile.dialog.common.openFailedMissing"));
    });
  });
});

// 另外两族协议标记各有一条绊线（`useProxy.test.ts`、`useAssistantChat.test.ts`），
// 这一族此前一条也没有——三处 `.vue` 把后端文案当控制流标记，phase 3 改了 `system_ops.rs`
// 只有三处静默失效，没有任何测试会红。绊线补上两端：后端仍发这个 code、三处前端仍按它判。
describe("system_ops.path_missing 的绊线", () => {
  it("后端仍发出这个 code（改后端也会红）", () => {
    const rust = readFileSync(SYSTEM_OPS_RS, "utf8");
    expect(
      rust,
      `${SYSTEM_OPS_RS} 里已找不到 coded("${SYSTEM_OPS_PATH_MISSING}")`,
    ).toContain(`coded("${SYSTEM_OPS_PATH_MISSING}")`);
  });

  // 逐文件断言，不是只测两个对话框：`ApiProfileList.vue` 的那处没有行为测试覆盖，
  // 只钉住两个对话框会漏掉它——而它正是 spec 记名的第一处。
  it("三处前端仍按这个 code 判，且不再按后端文案匹配", () => {
    for (const file of SITE_FILES) {
      const src = readFileSync(join(SITES_DIR, file), "utf8");
      expect(src, `${file} 已不再按 ${SYSTEM_OPS_PATH_MISSING} 判`).toContain(
        SYSTEM_OPS_PATH_MISSING,
      );
      expect(src, `${file} 又在按后端错误原文做子串匹配`).not.toContain("路径不存在");
    }
  });
});

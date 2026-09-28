import { describe, it, expect, vi } from "vitest";
import { nextTick } from "vue";
import { shallowMount } from "@vue/test-utils";
import ProfileImportConflictDialog, {
  type ImportCandidate,
  type ScopeImportItem,
} from "./ProfileImportConflictDialog.vue";
import ElegantSelect from "../common/ElegantSelect.vue";
import { setLocale, t } from "../../i18n";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

function makeCandidate(overrides: Partial<ImportCandidate> = {}): ImportCandidate {
  return {
    cli_id: "claude",
    scope: "global",
    name: "test",
    content: {},
    is_active: false,
    isConflict: true,
    action: "overwrite",
    newName: "test",
    ...overrides,
  };
}

function makeScope(overrides: Partial<ScopeImportItem> = {}): ScopeImportItem {
  return {
    id: "tab1",
    cli_id: "claude",
    name: "client日志压缩包",
    dirs: ["/a", "/b"],
    status: "new",
    action: "add",
    ...overrides,
  };
}

describe("ProfileImportConflictDialog 处理动作选项", () => {
  it("冲突项提供 覆盖/重命名/跳过", () => {
    const wrapper = shallowMount(ProfileImportConflictDialog, {
      props: { candidates: [makeCandidate()] },
    });
    const select = wrapper.findComponent(ElegantSelect);
    expect(select.props("options").map((o: any) => o.label)).toEqual([
      t("api-profile.dialog.importConflict.action.overwrite"),
      t("app.ctxMenu.rename"),
      t("api-profile.dialog.importConflict.action.skip"),
    ]);
  });

  it("非冲突的新配置提供 新增/跳过（不应出现覆盖/重命名）", () => {
    const wrapper = shallowMount(ProfileImportConflictDialog, {
      props: { candidates: [makeCandidate({ isConflict: false })] },
    });
    const select = wrapper.findComponent(ElegantSelect);
    expect(select.props("options").map((o: any) => o.label)).toEqual([
      t("api-profile.dialog.importConflict.action.add"),
      t("api-profile.dialog.importConflict.action.skip"),
    ]);
  });
});

describe("ProfileImportConflictDialog 表格列与展示", () => {
  it("渲染 5 列表头及对应列数据", () => {
    const wrapper = shallowMount(ProfileImportConflictDialog, {
      props: {
        candidates: [
          makeCandidate({ name: "a", cli_id: "claude" }),
          makeCandidate({ name: "b", cli_id: "claude" }),
          makeCandidate({ cli_id: "codex", name: "c" }),
        ],
      },
    });
    const headers = wrapper.findAll("th").map((th) => th.text());
    expect(headers).toEqual([
      "CLI",
      t("api-profile.dialog.importConflict.colScope"),
      t("api-profile.connection.name"),
      t("api-profile.dialog.importConflict.colStatus"),
      t("api-profile.dialog.importConflict.colAction"),
    ]);

    const rows = wrapper.findAll("tbody tr");
    expect(rows).toHaveLength(3);
    expect(rows[0].text()).toContain("a");
    expect(rows[0].text()).toContain("CLAUDE");
    expect(rows[2].text()).toContain("CODEX");
  });
});

describe("ProfileImportConflictDialog 作用域区块", () => {
  it("展示作用域状态与对应动作选项", () => {
    const wrapper = shallowMount(ProfileImportConflictDialog, {
      props: {
        candidates: [makeCandidate()],
        scopes: [
          makeScope({ id: "t1", name: "scope-a", status: "existing", action: "overwrite" }),
          makeScope({ id: "t2", name: "scope-b", status: "nameConflict", action: "skip" }),
          makeScope({ id: "t3", name: "scope-c", status: "new", action: "add" }),
        ],
        scopeBindings: [{ cli_id: "claude", scope: "t1", profile_name: "kimi" }],
      },
    });

    const section = wrapper.find(".scope-section");
    expect(section.exists()).toBe(true);
    const rows = section.findAll(".scope-item");
    expect(rows).toHaveLength(3);
    expect(rows[0].text()).toContain("scope-a");
    expect(rows[0].text()).toContain(t("api-profile.dialog.importConflict.status.existing"));
    expect(rows[0].text()).toContain("kimi");
    expect(rows[1].text()).toContain(t("api-profile.dialog.importConflict.status.nameConflict"));
    expect(rows[2].text()).toContain(t("api-profile.dialog.importConflict.status.newScope"));

    const selects = section.findAllComponents(ElegantSelect);
    expect(selects[0].props("options").map((o: any) => o.label)).toEqual([
      t("api-profile.dialog.importConflict.action.overwrite"),
      t("api-profile.dialog.importConflict.action.skip"),
    ]);
    expect(selects[1].props("options").map((o: any) => o.label)).toEqual([
      t("api-profile.dialog.importConflict.action.skip"),
      t("api-profile.dialog.importConflict.action.addCopy"),
    ]);
    expect(selects[2].props("options").map((o: any) => o.label)).toEqual([
      t("api-profile.dialog.importConflict.action.add"),
      t("api-profile.dialog.importConflict.action.skip"),
    ]);
  });

  it("备份不含作用域时不展示作用域区块", () => {
    const wrapper = shallowMount(ProfileImportConflictDialog, {
      props: { candidates: [makeCandidate()] },
    });
    expect(wrapper.find(".scope-section").exists()).toBe(false);
  });

  it("确认时同时带出配置与作用域的动作选择", async () => {
    const wrapper = shallowMount(ProfileImportConflictDialog, {
      props: {
        candidates: [makeCandidate()],
        scopes: [makeScope({ action: "skip" })],
      },
    });
    await wrapper.find(".btn-primary").trigger("click");
    const emitted = wrapper.emitted("confirm");
    expect(emitted).toHaveLength(1);
    const [items, scopes] = emitted![0] as [ImportCandidate[], ScopeImportItem[]];
    expect(items).toHaveLength(1);
    expect(scopes[0].action).toBe("skip");
  });
});

describe("ProfileImportConflictDialog 批量处理作用到作用域", () => {
  it("全部覆盖：同时将冲突配置设为 overwrite，并将 existing 作用域设为 overwrite", async () => {
    const wrapper = shallowMount(ProfileImportConflictDialog, {
      props: {
        candidates: [makeCandidate({ action: "skip", isConflict: true })],
        scopes: [
          makeScope({ status: "existing", action: "skip" }),
          makeScope({ status: "nameConflict", action: "skip" }),
        ],
      },
    });

    const buttons = wrapper.findAll(".btn-sub");
    const overwriteBtn = buttons.find((b) => b.text().includes(t("api-profile.dialog.importConflict.batchOverwrite")));
    expect(overwriteBtn).toBeDefined();
    await overwriteBtn!.trigger("click");

    await wrapper.find(".btn-primary").trigger("click");
    const [items, scopes] = wrapper.emitted("confirm")![0] as [
      ImportCandidate[],
      ScopeImportItem[]
    ];
    expect(items[0].action).toBe("overwrite");
    expect(scopes[0].action).toBe("overwrite");
    // nameConflict 作用域不支持覆盖，保持原动作
    expect(scopes[1].action).toBe("skip");
  });

  it("全部重命名：同时将冲突配置设为 rename，并将 nameConflict 作用域设为 add（新增副本）", async () => {
    const wrapper = shallowMount(ProfileImportConflictDialog, {
      props: {
        candidates: [makeCandidate({ action: "overwrite", isConflict: true, name: "demo", newName: "demo" })],
        scopes: [
          makeScope({ status: "existing", action: "overwrite" }),
          makeScope({ status: "nameConflict", action: "skip" }),
        ],
      },
    });

    const buttons = wrapper.findAll(".btn-sub");
    const renameBtn = buttons.find((b) => b.text().includes(t("api-profile.dialog.importConflict.batchRename")));
    expect(renameBtn).toBeDefined();
    await renameBtn!.trigger("click");

    await wrapper.find(".btn-primary").trigger("click");
    const [items, scopes] = wrapper.emitted("confirm")![0] as [
      ImportCandidate[],
      ScopeImportItem[]
    ];
    expect(items[0].action).toBe("rename");
    expect(items[0].newName).toBe("demo_imported");
    expect(scopes[1].action).toBe("add");
  });

  it("全部跳过：同时将冲突配置与冲突作用域设为 skip", async () => {
    const wrapper = shallowMount(ProfileImportConflictDialog, {
      props: {
        candidates: [makeCandidate({ action: "overwrite", isConflict: true })],
        scopes: [
          makeScope({ status: "existing", action: "overwrite" }),
          makeScope({ status: "nameConflict", action: "add" }),
        ],
      },
    });

    const buttons = wrapper.findAll(".btn-sub");
    const skipBtn = buttons.find((b) => b.text().includes(t("api-profile.dialog.importConflict.batchSkip")));
    expect(skipBtn).toBeDefined();
    await skipBtn!.trigger("click");

    await wrapper.find(".btn-primary").trigger("click");
    const [items, scopes] = wrapper.emitted("confirm")![0] as [
      ImportCandidate[],
      ScopeImportItem[]
    ];
    expect(items[0].action).toBe("skip");
    expect(scopes[0].action).toBe("skip");
    expect(scopes[1].action).toBe("skip");
  });
});

describe("ProfileImportConflictDialog 语言切换", () => {
  // 「用户切换语言后文案全部变语言」是迁移的完成标准之一，此前的四语渲染验证是临时的、已删掉，
  // 于是这条标准在 CI 里不可重复。这里断言的是**渲染结果**：组件若把文案写成模块级常量
  // （语言冻结在 import 时）或让 computed 丢掉对 locale 的依赖，t() 会变而 DOM 不变，
  // 下面的断言立刻变红。
  it("切换语言后表头随语言变化", async () => {
    setLocale("en");
    const wrapper = shallowMount(ProfileImportConflictDialog, {
      props: { candidates: [makeCandidate()] },
    });
    const scopeHeader = () => wrapper.findAll("th")[1].text();
    const english = scopeHeader();
    expect(english).toBe(t("api-profile.dialog.importConflict.colScope"));

    setLocale("zh");
    await nextTick();
    // 两个语言包在这个 key 上取值相同的话，下面两条断言测不出「没跟随语言」，
    // 前提不成立时先在这里响亮失败，而不是悄悄退化成一条测不到东西的用例。
    expect(scopeHeader()).not.toBe(english);
    expect(scopeHeader()).toBe(t("api-profile.dialog.importConflict.colScope"));

    setLocale("en");
  });
});


import { describe, it, expect, vi } from "vitest";
import { shallowMount } from "@vue/test-utils";
import ApiProfileScopeAllocation, {
  type ScopeBinding,
  type ScopeDirStatus,
} from "./ApiProfileScopeAllocation.vue";
import { setLocale } from "../../i18n";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

/**
 * 磁盘状态徽标的 **渲染** 证据：后端只回结构化数据（`ScopeDirDetail`，见
 * `src-tauri/src/commands/profile.rs`），tooltip 的句子由本组件按当前语言拼。
 * 断言的是挂载后 DOM 上的 `title`，不是「函数应该会拼」。
 */

const SCOPE = "tab1";

function binding(): ScopeBinding {
  return {
    scope: SCOPE,
    name: "Project Beta",
    isGlobal: false,
    dirs: ["/a", "/b"],
    activeProfile: null,
  };
}

function status(detail: ScopeDirStatus["detail"]): ScopeDirStatus {
  return { scope: SCOPE, status: "dirsInconsistent", detail };
}

function mountSection(scopeDirStatus: ScopeDirStatus[]) {
  return shallowMount(ApiProfileScopeAllocation, {
    props: {
      scopeBindings: [binding()],
      availableProfiles: [],
      currentCliName: "Claude Code",
      currentCliId: "claude",
      isClaude: true,
      scopeDirStatus,
    },
  });
}

function badgeTitle(wrapper: ReturnType<typeof mountSection>): string {
  const badge = wrapper.find(".status-badge");
  expect(badge.exists(), "状态徽标应渲染出来").toBe(true);
  return badge.attributes("title")!;
}

/** 逐语言断言，每次切语言后等一次重渲染（不等就是恒真断言）。 */
async function titleInEveryLanguage(
  wrapper: ReturnType<typeof mountSection>,
  expected: Record<"en" | "zh" | "ja" | "de", string>
) {
  for (const lang of ["en", "zh", "ja", "de"] as const) {
    setLocale(lang);
    await wrapper.vm.$nextTick();
    expect(badgeTitle(wrapper), `${lang} 的 tooltip`).toBe(expected[lang]);
  }
}

describe("ApiProfileScopeAllocation 磁盘状态 tooltip", () => {
  it("目录不一致：计数由后端给的目录列表算出，四语各自渲染", async () => {
    const wrapper = mountSection([
      status({ kind: "dirs_inconsistent", dirs: ["/a", "/b"] }),
    ]);
    await titleInEveryLanguage(wrapper, {
      en: "2 directories have inconsistent configs",
      zh: "2 个目录配置不一致",
      ja: "2 個のディレクトリで設定が一致していません",
      de: "2 Verzeichnisse haben unterschiedliche Konfigurationen",
    });
  });

  /**
   * `count = 1` 的渲染：后端今天**产生不出来**（该状态只在「有配置的目录多于一个」时置出，
   * 见 `profile.rs:1950-1951` 与 `scripts/i18n-plural-adjudication.json` 的该键条目）。
   *
   * 本用例钉的是**前端在这一天的渲染值**（en/de 的语法确实不成立），
   * **不是**守卫的安全网——它直接喂 `dirs: ["/a"]`，对后端的守卫一无所知。
   * 守卫本身由 Rust 侧的 `test_scope_dir_status_inconsistent_guard` 钉住（那里才是
   * 「count=1 可达化」第一个变红的地方）。
   */
  it("目录不一致：count=1 的渲染（后端不可达，钉住前端这一天的取值）", async () => {
    const wrapper = mountSection([
      status({ kind: "dirs_inconsistent", dirs: ["/a"] }),
    ]);
    await titleInEveryLanguage(wrapper, {
      en: "1 directories have inconsistent configs",
      zh: "1 个目录配置不一致",
      ja: "1 個のディレクトリで設定が一致していません",
      de: "1 Verzeichnisse haben unterschiedliche Konfigurationen",
    });
  });

  it("已有配置：base_url 取自结构化变体，四语只换句子骨架", async () => {
    const wrapper = mountSection([
      { scope: SCOPE, status: "unboundWithConfig", detail: { kind: "unbound_with_config", base_url: "https://api.example.com" } },
    ]);
    await titleInEveryLanguage(wrapper, {
      en: "An API config already exists in the project directory (https://api.example.com); it is currently unbound and will inherit the global profile",
      zh: "项目目录中已存在 API 配置（https://api.example.com），当前未绑定，将继承全局",
      ja: "プロジェクトディレクトリに API 設定が既に存在します（https://api.example.com）。現在は未バインドで、グローバルを継承します",
      de: "Im Projektverzeichnis existiert bereits eine API-Konfiguration (https://api.example.com); sie ist derzeit nicht gebunden und erbt das globale Profil",
    });
  });

  it("配置偏离：目录路径取自结构化变体（第三种载荷，原 `detail` 也承载它）", async () => {
    const wrapper = mountSection([
      { scope: SCOPE, status: "diverged", detail: { kind: "diverged", dir: "/a" } },
    ]);
    await titleInEveryLanguage(wrapper, {
      en: "The on-disk config for directory /a does not match the bound profile (it may have been edited manually)",
      zh: "目录 /a 的磁盘配置与已绑定配置不一致（可能被手动修改）",
      ja: "ディレクトリ /a のディスク上の設定がバインド済みプロファイルと一致しません（手動で変更された可能性があります）",
      de: "Die Konfiguration im Verzeichnis /a stimmt nicht mit dem gebundenen Profil überein (möglicherweise manuell geändert)",
    });
  });

  it("detail 缺失时退回徽标本身那句，不渲染 `0 个目录配置不一致`", async () => {
    const wrapper = mountSection([status(null)]);
    await titleInEveryLanguage(wrapper, {
      en: "Directories inconsistent",
      zh: "目录不一致",
      ja: "ディレクトリが不一致",
      de: "Verzeichnisse inkonsistent",
    });
  });
});

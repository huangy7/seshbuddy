export interface ChangelogEntry {
  version: string;
  date: string;
  /**
   * 变更条目存 i18n key 而非文案：同一份更新日志要在四种界面语言下渲染，
   * 而 key 在渲染时才解析，语言切换后无需重载数据。
   */
  changeKeys: string[];
}

/** 更新日志按版本倒序排列；About 页默认展示最近若干条，可展开查看更早版本 */
export const changelog: ChangelogEntry[] = [
  {
    version: "0.1.2",
    date: "2026-10-10",
    changeKeys: [
      "app.changelog.v0_1_2.multicli",
      "app.changelog.v0_1_2.dshFaultTolerance",
      "app.changelog.v0_1_2.sessionPreferences",
      "app.changelog.v0_1_2.dialogFixes",
    ],
  },
  {
    version: "0.1.1",
    date: "2026-10-07",
    changeKeys: [
      "app.changelog.v0_1_1.opencode",
    ],
  },
  {
    version: "0.1.0",
    date: "2026-09-18",
    changeKeys: [
      "app.changelog.v0_1_0.multicli",
      "app.changelog.v0_1_0.fulltextSearch",
      "app.changelog.v0_1_0.proxy",
      "app.changelog.v0_1_0.editor",
      "app.changelog.v0_1_0.assistant",
      "app.changelog.v0_1_0.performance",
      "app.changelog.v0_1_0.updater",
    ],
  },
];

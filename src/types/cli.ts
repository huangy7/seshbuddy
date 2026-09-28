import { t } from "../i18n";

export type CliId = "claude" | "codex" | "gemini" | "workbuddy" | "dsh" | "antigravity";

export interface CliDefinition {
  id: CliId;
  /** 专有名词，不进语言包 */
  name: string;
  command: string;
  dataSourcePath: string;
  dataDirPath: string;
  /**
   * 整条安装提示的 i18n key。多数条目为 `cli.installHint.<id>`，Windows 另有
   * `.windows` / `.unix` 分支；无平台分支的条目（DSH）直接指向该条文案。
   */
  installHintKey: string;
  permissionLabelKey: string;
  permissionHintKey: string;
  supportsNewSession: boolean;
  supportsResumeSession: boolean;
  supportsContextMenu: boolean;
  supportsInPlaceFork: boolean;
  supportsUsageStats: boolean;
  supportsApiProfiles: boolean;
  supportsApiLogs: boolean;
}

export interface CliStatus {
  id: CliId;
  has_sessions: boolean;
  has_binary: boolean;
}

/** 前端运行时发现结果：hasSessions 驱动展示/切换，hasBinary 驱动新建/恢复等二进制动作 */
export interface CliRuntime {
  hasSessions: boolean;
  hasBinary: boolean;
}

export interface CliPathConfig {
  id: CliId;
  defaultDataDir: string;
  effectiveDataDir: string;
  sessionsDir: string;
  hasOverride: boolean;
}

export interface ResolvedCliDefinition extends CliDefinition {
  defaultDataDirPath: string;
  hasCustomDataDir: boolean;
}

export interface CliOption extends CliDefinition {
  /** 展示门：数据目录存在会话（has_sessions） */
  hasSessions: boolean;
  /** 二进制门：检测到可执行文件（detect_cli / which） */
  hasBinary: boolean;
}

/**
 * 系统右键菜单注册/注销的结果。**不是错误**——后端只回动作，措辞由前端渲染。
 *
 * 变体**不带平台**：macOS 与 Windows 的提示措辞不同，但平台是前端读得到的事实
 * （`IS_WINDOWS`），把它写进变体会让枚举随支持平台的数量增长。
 */
export type ContextMenuOutcome =
  | { kind: "registered" }
  | { kind: "unregistered" };

/**
 * 当前平台是否 Windows。判据是前端 UA（后端只在注册/注销动作上回话，不告诉平台）。
 * **导出**：默认数据目录、右键菜单措辞等都要用它，各自重新声明一份会让两处判据分叉，
 * 而分叉时界面只会说错平台，没有任何测试或类型能发现。
 */
export const IS_WINDOWS = typeof navigator !== "undefined"
  && navigator.userAgent.toLowerCase().includes("windows");

function defaultCliDataDir(id: CliId): string {
  const dirs: Record<CliId, { win: string; unix: string }> = {
    claude: { win: "%USERPROFILE%\\.claude", unix: "~/.claude" },
    codex: { win: "%USERPROFILE%\\.codex", unix: "~/.codex" },
    gemini: { win: "%USERPROFILE%\\.gemini", unix: "~/.gemini" },
    workbuddy: { win: "%USERPROFILE%\\.workbuddy", unix: "~/.workbuddy" },
    dsh: { win: "%USERPROFILE%\\.dsh", unix: "~/.dsh" },
    antigravity: {
      win: "%USERPROFILE%\\.gemini\\antigravity-cli",
      unix: "~/.gemini/antigravity-cli",
    },
  };
  return IS_WINDOWS ? dirs[id].win : dirs[id].unix;
}

function defaultCliDataSource(id: CliId): string {
  const base = defaultCliDataDir(id);
  const sep = IS_WINDOWS ? "\\" : "/";
  const subdir: Record<CliId, string> = {
    claude: "projects",
    codex: "sessions",
    gemini: "tmp",
    workbuddy: "projects",
    dsh: "sessions",
    antigravity: "brain",
  };
  return `${base}${sep}${subdir[id]}${sep}`;
}

export const CLI_DEFINITIONS: Record<CliId, CliDefinition> = {
  claude: {
    id: "claude",
    name: "Claude Code",
    command: "claude",
    dataSourcePath: defaultCliDataSource("claude"),
    dataDirPath: defaultCliDataDir("claude"),
    installHintKey: "cli.installHint.claude",
    permissionLabelKey: "cli.permissionLabel.claude",
    permissionHintKey: "cli.permissionHint.claude",
    supportsNewSession: true,
    supportsResumeSession: true,
    supportsContextMenu: true,
    supportsInPlaceFork: true,
    supportsUsageStats: true,
    supportsApiProfiles: true,
    supportsApiLogs: true,
  },
  codex: {
    id: "codex",
    name: "Codex",
    command: "codex",
    dataSourcePath: defaultCliDataSource("codex"),
    dataDirPath: defaultCliDataDir("codex"),
    installHintKey: "cli.installHint.codex",
    permissionLabelKey: "cli.permissionLabel.codex",
    permissionHintKey: "cli.permissionHint.codex",
    supportsNewSession: true,
    supportsResumeSession: true,
    supportsContextMenu: false,
    supportsInPlaceFork: false,
    supportsUsageStats: true,
    supportsApiProfiles: true,
    supportsApiLogs: true,
  },
  gemini: {
    id: "gemini",
    name: "Gemini",
    command: "gemini",
    dataSourcePath: defaultCliDataSource("gemini"),
    dataDirPath: defaultCliDataDir("gemini"),
    installHintKey: "cli.installHint.gemini",
    permissionLabelKey: "cli.permissionLabel.gemini",
    permissionHintKey: "cli.permissionHint.gemini",
    supportsNewSession: true,
    supportsResumeSession: true,
    supportsContextMenu: false,
    supportsInPlaceFork: false,
    supportsUsageStats: true,
    supportsApiProfiles: false,
    supportsApiLogs: false,
  },
  workbuddy: {
    id: "workbuddy",
    name: "WorkBuddy",
    command: "workbuddy",
    dataSourcePath: defaultCliDataSource("workbuddy"),
    dataDirPath: defaultCliDataDir("workbuddy"),
    installHintKey: "cli.installHint.workbuddy",
    permissionLabelKey: "cli.permissionLabel.workbuddy",
    permissionHintKey: "cli.permissionHint.workbuddy",
    supportsNewSession: false,
    supportsResumeSession: true,
    supportsContextMenu: false,
    supportsInPlaceFork: true,
    supportsUsageStats: false,
    supportsApiProfiles: false,
    supportsApiLogs: false,
  },
  dsh: {
    id: "dsh",
    name: "DSH",
    command: "dsh",
    dataSourcePath: "~/.dsh/sessions",
    dataDirPath: "~/.dsh",
    installHintKey: "cli.installDsh",
    permissionLabelKey: "cli.permissionLabel.dsh",
    permissionHintKey: "cli.permissionHint.dsh",
    supportsNewSession: false,
    supportsResumeSession: false,
    supportsContextMenu: false,
    supportsInPlaceFork: false,
    supportsUsageStats: true,
    supportsApiProfiles: false,
    supportsApiLogs: false,
  },
  antigravity: {
    id: "antigravity",
    name: "Antigravity",
    command: "agy",
    dataSourcePath: defaultCliDataSource("antigravity"),
    dataDirPath: defaultCliDataDir("antigravity"),
    installHintKey: "cli.installHint.antigravity",
    permissionLabelKey: "cli.permissionLabel.claude",
    permissionHintKey: "cli.permissionHint.claude",
    supportsNewSession: true,
    supportsResumeSession: true,
    supportsContextMenu: false,
    supportsInPlaceFork: false,
    supportsUsageStats: true,
    supportsApiProfiles: false,
    supportsApiLogs: false,
  },
};

export const SUPPORTED_CLIS = Object.values(CLI_DEFINITIONS);

export function isCliId(value: string): value is CliId {
  return value === "claude" || value === "codex" || value === "gemini" || value === "workbuddy" || value === "dsh" || value === "antigravity";
}

export function getCliDefinition(id: CliId): CliDefinition {
  return CLI_DEFINITIONS[id];
}

export function resolveCliDefinition(
  id: CliId,
  pathConfig?: CliPathConfig | null,
): ResolvedCliDefinition {
  const base = CLI_DEFINITIONS[id];

  return {
    ...base,
    dataDirPath: pathConfig?.effectiveDataDir ?? base.dataDirPath,
    dataSourcePath: pathConfig?.sessionsDir ?? base.dataSourcePath,
    defaultDataDirPath: pathConfig?.defaultDataDir ?? base.dataDirPath,
    hasCustomDataDir: pathConfig?.hasOverride ?? false,
  };
}

/**
 * 解析 CLI 定义的文案字段。
 *
 * 定义表是模块级常量，文案必须存 key 而非渲染结果——
 * 否则 import 时就固化，切换语言不会更新。
 */
export function cliInstallHint(def: CliDefinition): string {
  const base = def.installHintKey;
  const platformKey = IS_WINDOWS ? `${base}.windows` : `${base}.unix`;
  const text = t(platformKey);
  // workbuddy、dsh 等无平台分支的条目直接命中 base
  return text === platformKey ? t(base) : text;
}

export function cliPermissionLabel(def: CliDefinition): string {
  return t(def.permissionLabelKey);
}

export function cliPermissionHint(def: CliDefinition): string {
  return t(def.permissionHintKey);
}

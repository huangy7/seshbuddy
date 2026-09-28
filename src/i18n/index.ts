import { createI18n, type LocaleMessageValue } from "vue-i18n";
// 刻意直接调用 invoke，不经 invokeApp 封装：本模块被 test-setup 在测试注册模块
// mock 之前先行加载，一旦经由封装间接引用 @tauri-apps/api/core，那份模块图就会
// 绑定真实实现，令各测试里的 vi.mock("@tauri-apps/api/core") 静默失效。
import { invoke } from "@tauri-apps/api/core";
import { FALLBACK_LOCALE, isLocale, type Locale } from "./types";
import { broadcastLocale } from "./localeChannel";

// 语言包按 <lang>/<namespace>.json 组织，命名空间与迁移批次对齐，
// 使每批迁移只动自己那个文件。_meta.json 存翻译状态，不属于消息，故排除。
const modules = import.meta.glob<{ default: Record<string, unknown> }>(
  "../locales/*/*.json",
  { eager: true }
);

type Messages = Record<string, Record<string, LocaleMessageValue>>;

function buildMessages(): Messages {
  const messages: Messages = {};
  for (const [path, mod] of Object.entries(modules)) {
    const m = path.match(/\/locales\/([^/]+)\/([^/]+)\.json$/);
    if (!m) continue;
    const [, lang, namespace] = m;
    if (namespace === "_meta") continue;
    (messages[lang] ??= {})[namespace] = mod.default;
  }
  return messages;
}

const i18n = createI18n({
  legacy: false,
  globalInjection: false,
  locale: FALLBACK_LOCALE,
  fallbackLocale: FALLBACK_LOCALE,
  // 缺 key 时返回 key 本身而非抛错：迁移期语言包不全，界面不能崩
  missingWarn: false,
  fallbackWarn: false,
  messages: buildMessages(),
});

/**
 * 模块级 `t`，而非 `useI18n()`。
 *
 * 大量文案位于 composables 与 utils，不在组件 setup 上下文里，`useI18n()` 无法使用。
 * 副作用是测试不需要安装任何插件，直接 import 即可。
 *
 * 第三参是 vue-i18n 的选项（目前只用到 `locale`，见 phrase-variables.ts 的跨语言变量名）。
 * 它必须走**第三**参：第二参是具名插值，把 `{ locale }` 传在第二参会静默失效。
 */
export const t = i18n.global.t as (
  key: string,
  named?: Record<string, unknown>,
  options?: { locale?: Locale }
) => string;

export const currentLocale = i18n.global.locale as unknown as import("vue").Ref<Locale>;

export function setLocale(locale: Locale): void {
  i18n.global.locale.value = locale as never;
  document.documentElement.setAttribute("lang", locale);
}

export function normalizeLocale(value: unknown): Locale {
  return isLocale(value) ? value : FALLBACK_LOCALE;
}

/**
 * 启动时从后端取回界面语言并应用。
 *
 * 取不到时静默保持基准语言：语言偏好读失败不该阻塞应用启动，
 * 用户仍可在设置里手动选择。
 */
export async function initLocale(): Promise<void> {
  try {
    const stored = await invoke<string>("get_app_locale");
    setLocale(normalizeLocale(stored));
  } catch (err) {
    console.warn("读取界面语言失败，使用基准语言", err);
  }
}

/** 切换语言并写回后端（后端据此重建托盘菜单）。 */
export async function changeLocale(locale: Locale): Promise<boolean> {
  setLocale(locale);
  let saved = false;
  try {
    const applied = await invoke<string>("set_app_locale", { locale });
    setLocale(normalizeLocale(applied));
    saved = true;
  } catch (err) {
    console.warn("保存界面语言失败", err);
  }
  // 广播放在后端往返之后，其它窗口收到的才是后端确认过的语言。
  // 助手窗口是独立 webview、各自一份 i18n 实例，收不到这条广播就会一直停在旧语言，
  // 而它的 webview 只被 hide 不销毁，关掉再开也不会刷新。
  broadcastLocale(currentLocale.value);
  return saved;
}

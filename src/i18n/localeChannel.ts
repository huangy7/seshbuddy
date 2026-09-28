import { emit, listen } from "@tauri-apps/api/event";
import { isLocale, type Locale } from "./types";

/**
 * 跨窗口语言通道。与主题通道同形（`app-theme-changed` + `storage` 兜底）：
 * 变更方 emit，另一窗口监听后应用。
 *
 * 语言状态归 `i18n/index.ts`，本模块只搬事件，不持有状态——反向 import 会与
 * `index.ts` 形成循环导入，而 `index.ts` 需要在本模块里调用 `broadcastLocale`。
 */
export const LOCALE_CHANGED_EVENT = "app-locale-changed";

/** `storage` 事件是同源 webview 的兜底通道，两端必须约定同一个键。 */
export const LOCALE_STORAGE_KEY = "seshbuddy-locale";

/** 通知其它窗口语言已切换。`changeLocale` 在语言落定后调用。 */
export function broadcastLocale(locale: Locale): void {
  try {
    localStorage.setItem(LOCALE_STORAGE_KEY, locale);
  } catch {
    // 存储不可用时仍可走 emit，不影响语言切换本身
  }
  // 无 Tauri IPC 的环境（测试、浏览器预览）里 emit 会 reject，故吞掉：
  // 广播失败只意味着别的窗口晚一步知道，不该变成未处理的 rejection。
  void emit(LOCALE_CHANGED_EVENT, locale).catch(() => {});
}

/**
 * 订阅其它窗口的语言广播，返回退订函数。
 *
 * `apply` 由调用方给出（助手窗口传 `setLocale`）：本模块不 import 语言状态，
 * 见文件头的循环导入说明。
 */
export function onLocaleBroadcast(apply: (locale: Locale) => void): () => void {
  let disposed = false;
  let unlisten: (() => void) | undefined;
  void listen<Locale>(LOCALE_CHANGED_EVENT, (event) => {
    // 载荷来自另一个窗口，可能被伪造或版本不一致，先收窄再应用
    if (isLocale(event.payload)) apply(event.payload);
  })
    .then((off) => {
      // 退订发生在 listen 落定之前时 unlisten 还没拿到，此时必须在这里补一次；
      // 否则那个 Tauri 监听器再也没人释放，永久残留。
      if (disposed) off();
      else unlisten = off;
    })
    .catch(() => {
      // 无 Tauri IPC 的环境里没有广播可听，storage 兜底照常注册
    });

  const onStorage = (e: StorageEvent) => {
    if (e.key === LOCALE_STORAGE_KEY && isLocale(e.newValue)) apply(e.newValue);
  };
  window.addEventListener("storage", onStorage);

  return () => {
    disposed = true;
    unlisten?.();
    window.removeEventListener("storage", onStorage);
  };
}

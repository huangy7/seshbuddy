import { createApp } from "vue";
import { message } from "@tauri-apps/plugin-dialog";
import "./styles/utilities.css";
import "./styles/menu.css";
import App from "./App.vue";
import { initLocale, t } from "./i18n";
import { renderAppError } from "./utils/invokeApp";
import { loadStartupMode } from "./onboarding/startup";

const APP_RUNNING_KEY = "seshbuddy_app_running";
if (sessionStorage.getItem(APP_RUNNING_KEY) === "true") {
  setTimeout(() => window.dispatchEvent(new CustomEvent("app-crash-recovery")), 1000);
}
sessionStorage.setItem(APP_RUNNING_KEY, "true");

window.addEventListener("beforeunload", () => {
  sessionStorage.removeItem(APP_RUNNING_KEY);
});

window.addEventListener("unhandledrejection", (event) => {
  console.error("Unhandled Promise Rejection:", event.reason);
});

import { vMermaid } from "./directives/vMermaid";

function mountApp(initialStartupMode: Awaited<ReturnType<typeof loadStartupMode>>) {
  const app = createApp(App, { initialStartupMode });
  app.directive("mermaid", vMermaid);

  app.config.errorHandler = (err, instance, info) => {
    const stack = err instanceof Error && err.stack ? err.stack : String(err);
    const chain: string[] = [];
    let cur = instance;
    while (cur) {
      const compType = (cur as unknown as { type: { name?: string; __file?: string } }).type;
      chain.push(compType?.__file ?? compType?.name ?? "Anonymous");
      cur = cur.$parent;
    }
    console.error("Global Vue Error:", err, info, stack, chain);
    const detail = t("app.error.renderDetail", {
      err: renderAppError(err),
      info,
      chain: chain.join(" → ") || t("app.error.unknownComponent"),
      stack: stack.split("\n").slice(0, 8).join("\n"),
    });
    message(t("app.error.renderMessage", { detail }), { title: t("app.error.renderTitle"), kind: "error" }).catch(console.error);
  };
  app.mount("#app");
}

// 语言必须在挂载前落定：首帧渲染发生在 mount 时刻，放在 mount 之后会让非基准语言的
// 用户先看到一帧英文。构建目标不支持顶层 await，故用 then 把挂载串在语言落定之后；
// initLocale 内部吞掉读取失败并正常 resolve，不会卡住启动。
initLocale().then(async () => {
  mountApp(await loadStartupMode());
});

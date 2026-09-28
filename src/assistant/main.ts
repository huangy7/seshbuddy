import { createApp } from "vue";
import "../styles/utilities.css";
// 模块级副作用：应用 data-theme + 监听主窗口主题广播（app-theme-changed / storage 双通道）
import "../composables/useTheme";
import AssistantApp from "./AssistantApp.vue";
import { initLocale, setLocale, t } from "../i18n";
import { onLocaleBroadcast } from "../i18n/localeChannel";

window.addEventListener("unhandledrejection", (event) => {
  console.error("Unhandled Promise Rejection:", event.reason);
});

import { vMermaid } from "../directives/vMermaid";

const app = createApp(AssistantApp);
app.directive("mermaid", vMermaid);

// 主窗口切换语言时广播过来。订阅放在 initLocale 之前：启动瞬间到达的广播不会被漏掉。
// 两个来源仍可能不一致——后端保存失败时 changeLocale 广播的是本地已应用的语言，而
// initLocale 读的是后端里那份；这种情况下先到的广播会被随后的 initLocale 覆盖，
// 以启动时读到的为准（下一次切换会重新对齐两个窗口）。
onLocaleBroadcast(setLocale);

// 每个 Tauri 窗口是独立 webview、各自一份 i18n 模块实例，主窗口的 initLocale
// 覆盖不到这里，必须自行在挂载前取回语言。
initLocale().then(() => {
  // 静态 <title> 只能是基准语言（HTML 在语言落定前就已被解析），这里补上真实语言。
  document.title = t("native.assistantWindowTitle");
  app.mount("#app");
});

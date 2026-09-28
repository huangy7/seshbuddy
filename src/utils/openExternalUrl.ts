import { openUrl } from "@tauri-apps/plugin-opener";

/**
 * 安全打开外部链接：
 * 优先调用 Tauri 原生 opener 插件打开系统浏览器，
 * 若在非桌面运行时环境（如浏览器 dev 调试）或调用失败时，安全回退到 window.open。
 */
export async function openExternalUrl(url: string): Promise<void> {
  try {
    await openUrl(url);
  } catch {
    if (typeof window !== "undefined" && window.open) {
      window.open(url, "_blank", "noopener,noreferrer");
    }
  }
}

import { writeText as writeNativeText } from "@tauri-apps/plugin-clipboard-manager";

/**
 * 写入系统剪贴板。
 *
 * **原生优先**：原生插件在 Rust 侧直接写系统剪贴板，**不受 WebView 的用户手势激活期限制**，
 * 因此在 `await` 过一个原生弹窗或长耗时命令之后再调用同样可靠。Web 层的
 * `navigator.clipboard.writeText` 在 WKWebView 上会因 transient activation 已失效而被拒
 * ——「先弹窗问一句、再复制」这类入口反复失败就是这个原因，而不是权限问题。
 *
 * 插件不可用（非 Tauri 环境、窗口未授权）时依次降级到 Web API 与 `execCommand`。
 * **三层都失败才返回 `false`**：调用方据此给用户反馈，不要静默当作成功。
 */
export async function copyToClipboard(text: string): Promise<boolean> {
  try {
    await writeNativeText(text);
    return true;
  } catch {
    // 插件不可用，降级到 Web 链路
  }

  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    // 旧环境降级：临时 textarea + execCommand
    try {
      const textarea = document.createElement("textarea");
      textarea.value = text;
      textarea.style.position = "fixed";
      textarea.style.opacity = "0";
      document.body.appendChild(textarea);
      textarea.select();
      try {
        return document.execCommand("copy");
      } finally {
        document.body.removeChild(textarea);
      }
    } catch {
      return false;
    }
  }
}

/**
 * 复制异步产生的文本（如需要先 `await` 后端命令的结果）；失败返回 `false`。
 *
 * 直接等结果再交给 `copyToClipboard` 即可。原先那套「把 Promise 包进 `ClipboardItem`
 * 同步发起」的写法是为**绕过手势激活期**而存在的，原生写入没有这个限制，前提已不成立；
 * 而且它的降级分支会吞掉 `textPromise` 自身的错误，把「命令失败」伪装成「复制失败」。
 */
export function copyPromiseToClipboard(
  textPromise: Promise<string>
): Promise<boolean> {
  return textPromise.then(copyToClipboard, () => false);
}

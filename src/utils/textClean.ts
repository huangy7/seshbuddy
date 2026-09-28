import { t } from "../i18n";

/**
 * Clean system-injected XML tags, skill command wrappers, and image placeholders from user message text for clean UI rendering.
 */
export function cleanUserText(text: string): string {
  if (!text) return "";

  let result = text;

  // <cb_summary> 压缩检查点不展示（WorkBuddy 上下文压缩行）
  if (result.trimStart().startsWith("<cb_summary")) return "";

  // Extract <command-args> if present
  const commandArgsMatch = result.match(/<command-args>([\s\S]*?)<\/command-args>/);
  if (commandArgsMatch) {
    result = commandArgsMatch[1];
  }

  // Extract <user_query> if present（取最后一个：WorkBuddy 会把上下文包在前面的同名标签里）
  const userQueryMatches = [...result.matchAll(/<user_query>\s*([\s\S]*?)\s*<\/user_query>/g)];
  if (userQueryMatches.length > 0) {
    result = userQueryMatches[userQueryMatches.length - 1][1];
  }

  result = result
    .replace(/<command-message>[\s\S]*?<\/command-message>\s*/g, "")
    .replace(/<command-name>[\s\S]*?<\/command-name>\s*/g, "")
    .replace(/<system-reminder[^>]*>[\s\S]*?<\/system-reminder>\s*/g, "")
    .replace(/<user_info>[\s\S]*?<\/user_info>\s*/g, "")
    .replace(/<git_status>[\s\S]*?<\/git_status>\s*/g, "")
    .replace(/<attached_files>[\s\S]*?<\/attached_files>\s*/g, "")
    .replace(/<agent_transcripts>[\s\S]*?<\/agent_transcripts>\s*/g, "")
    .replace(/<agent_skills>[\s\S]*?<\/agent_skills>\s*/g, "")
    .replace(/<rules>[\s\S]*?<\/rules>\s*/g, "")
    .replace(/\[Image(?:\s*#\d+)?\]\s*/g, "");

  return result.trim();
}

/**
 * 格式化用于系统原生确认弹窗（如删除确认）的会话标题预览。
 *
 * 业务背景：
 * 会话 display_name 通常取自首条用户消息，可能包含数千字符与多行换行。
 * 原生系统弹窗（如 macOS NSAlert）若直接渲染未截断的多行大段文本，会导致弹窗
 * 高度纵向撑爆屏幕、确定/取消操作按钮被 Dock 栏遮挡甚至溢出不可见。
 * 该函数先将连续空白与换行压缩为单行，再裁剪至合理长度并增加优雅引号，保障界面高密度精致度与操作可用性。
 */
export function formatDialogSessionTitle(name: string, maxLen = 60): string {
  const trimmed = (name || "").trim();
  // 引号留在代码里而不是进语言包：三条 return 各自套同一对引号，进包会让每条译文
  // 都要重复一遍这对引号。
  if (!trimmed) return `“${t("settings.archivedSessions.untitledSession")}”`;
  const singleLine = trimmed.replace(/\r?\n+/g, " ").replace(/\s+/g, " ");
  if (singleLine.length <= maxLen) {
    return `“${singleLine}”`;
  }
  return `“${singleLine.slice(0, maxLen)}…”`;
}

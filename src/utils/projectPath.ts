import { t } from "../i18n";

function normalizePath(path: string): string {
  return path.replace(/[\\/]+$/, "");
}

export function isSubPath(filePath: string, projectPath: string): boolean {
  const root = normalizePath(projectPath);
  return filePath === root || filePath.startsWith(root + "/") || filePath.startsWith(root + "\\");
}

export function findBestProjectPath(filePath: string, projectPaths: readonly string[]): string | null {
  let best: string | null = null;
  for (const projectPath of projectPaths) {
    if (!isSubPath(filePath, projectPath)) continue;
    if (!best || normalizePath(projectPath).length > normalizePath(best).length) {
      best = projectPath;
    }
  }
  return best;
}

export function basename(path: string): string {
  if (!path) return "";
  const parts = path.replace(/\\/g, "/").split("/").filter(Boolean);
  return parts.pop() || path;
}

/**
 * 项目路径 → 展示名：取最后一段；**缺席时给本地化兜底句**。
 *
 * 后端不再造「未知项目」占位串（那是用户可见文案，属于语言包的活），缺席由 `null` 表达，
 * 兜底句在这里按当前语言渲染。会话树、仪表盘、最近列表都走这一个出口——
 * 各写一份会让兜底键名与「什么算缺席」的判据分叉，而分叉不会编译报错。
 */
export function projectLabel(path: string | null): string {
  if (!path) return t("session.unknownProject");
  return basename(path);
}

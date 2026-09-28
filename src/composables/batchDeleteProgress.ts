import { t } from "../i18n";

export interface BatchDeleteProgress {
  done: number;
  total: number;
  currentPath: string;
}

/** 取路径最后一段用于展示（兼容 Windows 反斜杠）。 */
function baseName(p: string): string {
  const seg = p.split(/[\\/]/).pop();
  return seg || p;
}

export function batchDeleteProgressText(p: BatchDeleteProgress): string {
  const base = t("session.batchDelete.progress", { done: p.done, total: p.total });
  return p.currentPath ? `${base} · ${baseName(p.currentPath)}` : base;
}

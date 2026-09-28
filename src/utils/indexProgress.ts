import { t } from "../i18n";

// 搜索索引进度的相位重映射（与 GlobalSearch / 后台索引指示共用）。
// 后端在扫描完成时 processedBytes 即达到 totalBytes，但写入/提交仍在进行，
// 直接按字节比会过早停在 100%，因此按相位分段：扫描 0-90%，写入 90-99%，提交 99%。
export interface IndexProgressLike {
  phase?: string;
  current: number;
  total: number;
  processedBytes?: number;
  totalBytes?: number;
}

export function indexProgressPercent(progress: IndexProgressLike): number {
  const totalBytes = progress.totalBytes || 0;
  const bytesRatio = totalBytes > 0
    ? Math.min(1, Math.max(0, (progress.processedBytes || 0) / totalBytes))
    : progress.total > 0
      ? Math.min(1, Math.max(0, progress.current / progress.total))
      : 0;

  switch (progress.phase) {
    case "writing":
    case "written": {
      const writeRatio = progress.total > 0
        ? Math.min(1, Math.max(0, progress.current / progress.total))
        : 0;
      return Math.round(90 + writeRatio * 9);
    }
    case "committing":
      return 99;
    case "committed":
      return 100;
    case "scanning":
    default:
      return Math.round(bytesRatio * 90);
  }
}

export function indexProgressPhaseLabel(phase?: string): string {
  return {
    scanning: t("session.indexProgress.scanning"),
    writing: t("session.indexProgress.writing"),
    written: t("session.indexProgress.writing"),
    committing: t("session.indexProgress.committing"),
    committed: t("session.indexProgress.committing"),
  }[phase ?? ""] ?? t("session.indexProgress.fallback");
}

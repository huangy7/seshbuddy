// 本地时区日期工具：避免 toISOString 在 UTC+8 下把"今天 0-8 点"算成昨天

/** 用本地年月日分量格式化为 yyyy-mm-dd，不受 UTC 偏移影响 */
export function formatLocalDate(d: Date): string {
  const y = d.getFullYear();
  const m = String(d.getMonth() + 1).padStart(2, "0");
  const day = String(d.getDate()).padStart(2, "0");
  return `${y}-${m}-${day}`;
}

/** 以传入日期（默认今天）为基准，返回本周一 ~ 本周五的本地日期范围（周报口径：工作周） */
export function currentWeekRange(now: Date = new Date()): { start: string; end: string } {
  const day = now.getDay() || 7; // 周日 getDay()=0 视为 7
  const monday = new Date(now);
  monday.setDate(now.getDate() - day + 1);
  const friday = new Date(monday);
  friday.setDate(monday.getDate() + 4);
  return { start: formatLocalDate(monday), end: formatLocalDate(friday) };
}

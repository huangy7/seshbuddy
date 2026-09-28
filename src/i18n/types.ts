/** 支持的界面语言。`en` 是基准语言，其余语言包缺 key 时回退到它。 */
export const SUPPORTED_LOCALES = ["en", "zh", "ja", "de"] as const;

export type Locale = (typeof SUPPORTED_LOCALES)[number];

export const FALLBACK_LOCALE: Locale = "en";

export function isLocale(value: unknown): value is Locale {
  return typeof value === "string" && (SUPPORTED_LOCALES as readonly string[]).includes(value);
}

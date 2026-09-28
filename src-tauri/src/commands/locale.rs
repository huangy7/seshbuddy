use crate::app_db;
use crate::error::AppResult;

/// 把任意系统 locale 归一化成受支持的界面语言码。
///
/// 繁体中文刻意落到英文而非简体：繁简不是机械转换（「設定」/「设置」、
/// 「檔案」/「文件」），转换产物会是别扭的混合体，应作为独立语言包另开一期。
pub(crate) fn normalize_locale(raw: Option<&str>) -> &'static str {
    let Some(tag) = raw.map(str::trim).filter(|s| !s.is_empty()) else {
        return "en";
    };
    let lower = tag.to_ascii_lowercase();
    let lang = lower.split(['-', '_']).next().unwrap_or("");

    match lang {
        "en" => "en",
        "ja" => "ja",
        "de" => "de",
        "zh" => {
            // 简体与「无地区后缀」都按简体处理；繁体落英文
            let is_traditional = lower.contains("hant")
                || lower.contains("-tw")
                || lower.contains("_tw")
                || lower.contains("-hk")
                || lower.contains("_hk")
                || lower.contains("-mo")
                || lower.contains("_mo");
            if is_traditional { "en" } else { "zh" }
        }
        _ => "en",
    }
}

/// 读系统语言。读不到时返回基准语言。
pub(crate) fn detect_system_locale() -> &'static str {
    let raw = sys_locale::get_locale();
    normalize_locale(raw.as_deref())
}

/// 设置表中的键。**存归一化后的语言码**（`zh` 而非 `zh-CN`），
/// 避免 `zh-CN` 与 `zh` 两种写法并存，读取端无需再解析。
const APP_SETTING_LOCALE: &str = "app.locale";

/// 解析顺序：设置表显式选择 → 系统 locale → 基准语言。
pub(crate) fn resolve_locale(setting: Option<String>, system: Option<&str>) -> &'static str {
    setting
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| normalize_locale(Some(s)))
        .unwrap_or_else(|| normalize_locale(system))
}

#[tauri::command]
pub fn get_app_locale() -> AppResult<String> {
    let stored = app_db::read_setting_json::<String>(APP_SETTING_LOCALE)?;
    Ok(resolve_locale(stored, Some(detect_system_locale())).to_string())
}

#[tauri::command]
pub fn set_app_locale(app: tauri::AppHandle, locale: String) -> AppResult<String> {
    use tauri::Manager;
    let normalized = normalize_locale(Some(locale.as_str()));
    app_db::write_setting_json(APP_SETTING_LOCALE, &normalized.to_string())?;
    // 托盘菜单文案在构建时固化，语言变了必须整体重建，否则原生菜单仍是旧语言。
    // 重建失败不回滚：语言已落库生效，此处只记录告警。
    if let Err(e) = crate::tray::rebuild_tray_menu(&app) {
        tracing::warn!("failed to rebuild tray menu after locale change: {}", e);
    }
    // 助手窗口的标题同理，且更隐蔽：它在**建窗时**取一次原生文案，而窗口是隐藏而非销毁的
    // （见 `assistant_open_window` 的 `show()` 分支），不在这里改就永远停在旧语言。
    // 窗口从未打开过时 `get_webview_window` 返回 `None`，属正常，不是错误。
    if let Some(win) = app.get_webview_window("assistant") {
        let title = crate::native_text::native_text("native", "assistantWindowTitle");
        if let Err(e) = win.set_title(&title) {
            tracing::warn!("failed to update assistant window title after locale change: {}", e);
        }
    }
    Ok(normalized.to_string())
}

#[cfg(test)]
mod tests {
    use super::{normalize_locale, resolve_locale};

    #[test]
    fn normalizes_supported_locales() {
        assert_eq!(normalize_locale(Some("en")), "en");
        assert_eq!(normalize_locale(Some("en-US")), "en");
        assert_eq!(normalize_locale(Some("zh-CN")), "zh");
        assert_eq!(normalize_locale(Some("zh-Hans")), "zh");
        assert_eq!(normalize_locale(Some("zh-SG")), "zh");
        assert_eq!(normalize_locale(Some("ja-JP")), "ja");
        assert_eq!(normalize_locale(Some("de")), "de");
        assert_eq!(normalize_locale(Some("de-AT")), "de");
    }

    #[test]
    fn falls_back_to_english() {
        assert_eq!(normalize_locale(None), "en");
        assert_eq!(normalize_locale(Some("")), "en");
        assert_eq!(normalize_locale(Some("fr-FR")), "en");
    }

    /// 繁体不是简体的机械转换（用词差异大），因此落到英文而非 zh
    #[test]
    fn traditional_chinese_falls_back_to_english() {
        assert_eq!(normalize_locale(Some("zh-TW")), "en");
        assert_eq!(normalize_locale(Some("zh-Hant")), "en");
        assert_eq!(normalize_locale(Some("zh-HK")), "en");
        assert_eq!(normalize_locale(Some("zh-MO")), "en");
    }

    #[test]
    fn resolution_prefers_explicit_setting_over_system() {
        // 显式选择优先；缺失时用系统检测值
        assert_eq!(resolve_locale(Some("ja".to_string()), Some("de-DE")), "ja");
        assert_eq!(resolve_locale(None, Some("de-DE")), "de");
        assert_eq!(resolve_locale(Some("".to_string()), Some("de-DE")), "de");
    }

    #[test]
    fn resolution_falls_back_to_english() {
        assert_eq!(resolve_locale(None, None), "en");
    }
}

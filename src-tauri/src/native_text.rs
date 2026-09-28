use crate::commands;
use std::collections::HashMap;
use std::sync::LazyLock;

/// 原生 UI 文案在编译期嵌入：托盘菜单、系统通知、窗口标题栏都不经前端渲染，
/// 复用同一份语言包保证文案与界面语言同源。
const TRAY_EN: &str = include_str!("../../src/locales/en/tray.json");
const TRAY_ZH: &str = include_str!("../../src/locales/zh/tray.json");
const TRAY_JA: &str = include_str!("../../src/locales/ja/tray.json");
const TRAY_DE: &str = include_str!("../../src/locales/de/tray.json");
const NATIVE_EN: &str = include_str!("../../src/locales/en/native.json");
const NATIVE_ZH: &str = include_str!("../../src/locales/zh/native.json");
const NATIVE_JA: &str = include_str!("../../src/locales/ja/native.json");
const NATIVE_DE: &str = include_str!("../../src/locales/de/native.json");

/// 命名空间 -> 语言码 -> 语言包
static TABLES: LazyLock<HashMap<&'static str, HashMap<&'static str, serde_json::Value>>> =
    LazyLock::new(|| {
        let mut tables = HashMap::new();
        for (namespace, bundles) in [
            (
                "tray",
                [("en", TRAY_EN), ("zh", TRAY_ZH), ("ja", TRAY_JA), ("de", TRAY_DE)],
            ),
            (
                "native",
                [
                    ("en", NATIVE_EN),
                    ("zh", NATIVE_ZH),
                    ("ja", NATIVE_JA),
                    ("de", NATIVE_DE),
                ],
            ),
        ] {
            let table = tables.entry(namespace).or_insert_with(HashMap::new);
            for (lang, raw) in bundles {
                // 语言包是构建期产物，解析失败属于打包错误，直接暴露
                let value: serde_json::Value = serde_json::from_str(raw)
                    .unwrap_or_else(|e| panic!("语言包 {namespace}/{lang} 不是合法 JSON: {e}"));
                table.insert(lang, value);
            }
        }
        tables
    });

/// 纯查表：语言不在表内时回退英文语言包；key 不在语言包内时返回 key 本身，
/// 便于从界面上直接定位缺失条目。
fn lookup(namespace: &str, lang: &str, key: &str) -> String {
    TABLES
        .get(namespace)
        .and_then(|table| table.get(lang).or_else(|| table.get("en")))
        .and_then(|table| table.get(key))
        .and_then(|value| value.as_str())
        .unwrap_or(key)
        .to_string()
}

/// 取原生 UI 文案，语言取当前界面语言。
pub(crate) fn native_text(namespace: &str, key: &str) -> String {
    let lang = commands::get_app_locale().unwrap_or_else(|_| "en".to_string());
    lookup(namespace, &lang, key)
}

/// `native_text` 的占位符替换版：把文案里的 `{name}` 换成 `value`。
pub(crate) fn native_text_with(namespace: &str, key: &str, name: &str, value: &str) -> String {
    native_text(namespace, key).replace(&format!("{{{}}}", name), value)
}

#[cfg(test)]
mod tests {
    use super::{lookup, TABLES};
    use std::collections::BTreeSet;

    fn keys_of(bundle: &serde_json::Value) -> BTreeSet<&str> {
        bundle
            .as_object()
            .expect("语言包必须是 JSON 对象")
            .keys()
            .map(String::as_str)
            .collect()
    }

    #[test]
    fn resolves_key_in_requested_language() {
        assert_eq!(lookup("tray", "zh", "quit"), "退出");
        assert_eq!(lookup("native", "en", "assistantWindowTitle"), "SeshBuddy Assistant");
    }

    #[test]
    fn unknown_language_falls_back_to_english_bundle() {
        assert_eq!(lookup("tray", "fr", "quit"), "Quit");
    }

    #[test]
    fn unknown_key_or_namespace_returns_key_itself() {
        assert_eq!(lookup("tray", "zh", "no-such-key"), "no-such-key");
        assert_eq!(lookup("no-such-namespace", "zh", "quit"), "quit");
    }

    /// 资源管理器右键菜单项的显示名：`context_menu.rs` 把查表结果写进注册表，由系统渲染。
    ///
    /// **四语逐条钉住**，因为这条文案的消费点（`windows_context_menu_label`）只在 Windows 上
    /// 被调用，本仓开发机编不出 Windows 目标——没有这个测试，四份语言包里这一格是死数据。
    /// `Claude Code` 是产品名，四语同值。
    #[test]
    fn context_menu_label_is_localized_in_every_language() {
        assert_eq!(
            lookup("native", "en", "contextMenuOpenWith"),
            "Open with Claude Code"
        );
        assert_eq!(
            lookup("native", "zh", "contextMenuOpenWith"),
            "用 Claude Code 打开"
        );
        assert_eq!(
            lookup("native", "ja", "contextMenuOpenWith"),
            "Claude Code で開く"
        );
        assert_eq!(
            lookup("native", "de", "contextMenuOpenWith"),
            "Mit Claude Code öffnen"
        );
    }

    /// 缺 key 时查表会静默回退英文，界面上看不出漏译，只能靠测试卡住：
    /// 每种语言的 key 集合必须与英文完全一致。
    #[test]
    fn every_language_covers_the_english_keys() {
        for (namespace, table) in TABLES.iter() {
            let en_keys = keys_of(&table["en"]);
            for (lang, bundle) in table.iter() {
                assert_eq!(
                    keys_of(bundle),
                    en_keys,
                    "{namespace}/{lang} 的 key 与英文语言包不一致"
                );
            }
        }
    }
}

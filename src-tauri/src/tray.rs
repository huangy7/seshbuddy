use crate::app_db;
use crate::cli::CliKind;
use crate::commands;
use crate::error::AppError;
use crate::native_text::{native_text, native_text_with};
use tauri::{
    image::Image,
    menu::{Menu, MenuBuilder, MenuItemBuilder, PredefinedMenuItem, Submenu, SubmenuBuilder},
    tray::TrayIconBuilder,
    AppHandle, Emitter, Manager, Runtime,
};

fn build_cli_profiles_submenu<R: Runtime, M: Manager<R>>(
    manager: &M,
    kind: CliKind,
) -> Result<Submenu<R>, Box<dyn std::error::Error>> {
    let cli_id = kind.id().to_string();
    let profiles = commands::list_profiles(Some(cli_id.clone()), None).unwrap_or_default();
    let active = commands::get_active_profile(Some(cli_id.clone()), None).unwrap_or_default();

    let mut submenu_builder = SubmenuBuilder::with_id(
        manager,
        format!("profiles:{}", cli_id),
        crate::cli_registry::descriptor_for(kind).tray_label,
    );

    if profiles.is_empty() {
        let empty_item = MenuItemBuilder::with_id(
            format!("no_profiles:{}", cli_id),
            native_text("tray", "noProfiles"),
        )
        .enabled(false)
        .build(manager)?;
        submenu_builder = submenu_builder.item(&empty_item);
    } else {
        for name in &profiles {
            let prefix = if *name == active { "✓ " } else { "    " };
            let label = format!("{}{}", prefix, name);
            let item = MenuItemBuilder::with_id(format!("profile:{}:{}", cli_id, name), label)
                .build(manager)?;
            submenu_builder = submenu_builder.item(&item);
        }
    }

    Ok(submenu_builder.build()?)
}

fn build_profile_switch_submenu<R: Runtime, M: Manager<R>>(
    manager: &M,
) -> Result<Submenu<R>, Box<dyn std::error::Error>> {
    let claude_submenu = build_cli_profiles_submenu(manager, CliKind::Claude)?;
    let codex_submenu = build_cli_profiles_submenu(manager, CliKind::Codex)?;

    Ok(SubmenuBuilder::with_id(manager, "tray-profiles-root", native_text("tray", "switchProfile"))
        .item(&claude_submenu)
        .item(&codex_submenu)
        .build()?)
}

fn build_tray_menu<R: Runtime, M: Manager<R>>(
    manager: &M,
    update_version: Option<&str>,
) -> Result<Menu<R>, Box<dyn std::error::Error>> {
    let show_item = MenuItemBuilder::with_id("show", native_text("tray", "openMain")).build(manager)?;
    let profile_switch_submenu = build_profile_switch_submenu(manager)?;
    let quit_item = PredefinedMenuItem::quit(manager, Some(&native_text("tray", "quit")))?;

    let mut builder = MenuBuilder::new(manager).item(&show_item);

    if let Some(version) = update_version {
        let update_item = MenuItemBuilder::with_id(
            "open_update_info",
            native_text_with("tray", "updateAvailable", "version", version),
        )
        .build(manager)?;
        let ignore_item = MenuItemBuilder::with_id(
            format!("ignore_update:{}", version),
            native_text("tray", "ignoreVersion"),
        )
        .build(manager)?;
        builder = builder
            .separator()
            .item(&update_item)
            .item(&ignore_item);
    }

    Ok(builder
        .separator()
        .item(&profile_switch_submenu)
        .separator()
        .item(&quit_item)
        .build()?)
}

/// Rebuild the tray menu without an update entry.
pub fn rebuild_tray_menu(app: &AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    rebuild_tray_menu_with_update(app, None)
}

/// Rebuild the tray menu, optionally showing an update entry for `version`.
///
/// macOS 26 将 NSStatusItem 迁移到 scene 架构后，托盘项必须在主线程上
/// 创建 / 替换 / 销毁，否则会触发 `assertBarrierOnQueue` 硬断言崩溃。
/// 这里把整段逻辑（含 TrayIcon 克隆的 drop）统一派发到主线程执行，
/// 避免 Rc 引用计数跨线程竞争导致的偶发崩溃。
pub fn rebuild_tray_menu_with_update(
    app: &AppHandle,
    update_version: Option<&str>,
) -> Result<(), Box<dyn std::error::Error>> {
    let version = update_version.map(str::to_owned);
    let app = app.clone();
    let app_for_task = app.clone();
    let (tx, rx) = std::sync::mpsc::channel();
    app_for_task.run_on_main_thread(move || {
        let result: Result<(), String> = (|| {
            let menu = build_tray_menu(&app, version.as_deref())?;
            if let Some(tray) = app.tray_by_id("main") {
                tray.set_menu(Some(menu))?;
            }
            Ok(())
        })()
        .map_err(|e: Box<dyn std::error::Error>| e.to_string());
        // 通过 channel 把结果送回调用线程；错误转成 String 以满足 Send
        let _ = tx.send(result);
    })?;
    rx.recv()
        // 主线程闭包 panic 时 recv 才失败——这句是我们自己的文案，故装 `Coded` 而不是裸串。
        // ⚠️ **「闭包 panic 会让 `tx` 在 unwind 中析构、从而使 `recv` 失败」是推断，不是实测**：
        // 这条罕见路径（panic → `recv` 失败 → 装箱 `Coded` → 调用点收口）**端到端从未被执行过**。
        // 若 Tauri/tao 在事件循环里 panic-abort（而非 unwind），下面这个 `Coded` 就是死码
        // ——那不影响正确性（它只是永不产出），但「瞬态已闭合」这句话的强度要按推断来读。
        // **拆箱保码的调用点有三处**（`commands/profile.rs`、`commands/updater.rs`、`updater.rs`）：
        // 三处都把这个装箱错误交给 [`attribute_rebuild_error`]，由它 `downcast::<AppError>()`、
        // 命中就原样放行、未命中才归属成 `tray.rebuild_failed`，所以这里装的 `Coded` 带着自己的
        // code 与文案跨出去，不会被压成裸 code 塞进 `detail`。
        // **但「三个调用点」不是全部**：`rebuild_tray_menu*` 还有别的调用点，它们都不拆箱 ——
        // `commands/locale.rs` 的 `set_app_locale` 是**第四个外部调用点**，它只把这个
        // `Box<dyn Error>` 交给 `tracing::warn!`，故拿到装箱的 `Coded` 时打印的是**裸 code**，
        // 是计划 11 具名留给计划 12 的缺口；其余在 `tray.rs` 内部：`create_tray` 的
        // `on_menu_event` 里两个 `let _ =` 直接丢弃（`ignore_update:` 与 `profile:` 两支），
        // 以及 `rebuild_tray_menu` 对 `rebuild_tray_menu_with_update` 的委托。
        // 这里刻意**不写行号**：本文件在改，行号会被改动自己顶偏（已经发生过两次）。
        // `tray.rebuild_failed` 的字面量留在 `attribute_rebuild_error` 的函数体里（生产代码），
        // 故闸门规则 10 的反向闭包仍能看到「有站点产出这个码」——**不能把它抽成 `code` 参数**：
        // 那时闸门只认 `coded("…")` 字面量，这个码会被判成「没有任何站点产出」。
        // 调用点是否真的走了收口函数，编译器与闸门都看不见（改写只会让收口函数变成死代码），
        // 由 `tray_rebuild_call_sites_route_through_the_shared_attribution` 钉住形状。
        .map_err(|_| {
            Box::<dyn std::error::Error>::from(AppError::coded("tray.rebuild_channel_closed"))
        })?
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e).into())
}

/// 把 [`rebuild_tray_menu_with_update`] 的装箱错误收口成 `AppError`。
///
/// **先拆箱，再归属**：内层已经是 `AppError`（`Coded`）就原样放行，保住它自己的 code 与
/// params；内层不是 `AppError`（`tauri::Error` / `io::Error` 等）才归属成
/// `tray.rebuild_failed`，原始文本进 `detail`（R3：第三方文本不翻译）。
/// 少了拆箱这一步，`Box<dyn Error>` 的 `Display` 会把内层 `Coded` 渲染成**裸 code** ——
/// `tray.rebuild_channel_closed` 就降级成 `tray.rebuild_failed` 里的一段英文，瞬态复活。
/// ⚠️ **「瞬态已闭合」是按【推断】而非【观测】**：本函数这一侧有行为钉
/// （`attribute_rebuild_error_preserves_a_boxed_coded`），但**装箱那一侧的那条罕见路径
/// 端到端从未被执行过** —— 推断链的第一环「主线程闭包 panic 会让 `tx` 在 unwind 中析构」
/// 见 `rebuild_tray_menu_with_update` 里 `rx.recv()` 处的说明；若 Tauri/tao 在事件循环里
/// panic-abort，`tray.rebuild_channel_closed` 是死码，本条说的「闭合」也就无从谈起。
///
/// **三个调用点共用本函数**（`updater.rs` / `commands/updater.rs` / `commands/profile.rs`）：
/// 收口写成函数才有可测的接缝 —— 那条真实路径要一个 `AppHandle<Wry>`，单测里造不出来。
/// ⚠️ **别在调用点把它换回内联的 `match` / `.with(...)`**：那正是「丢掉拆箱但保留字面量」
/// 那种善意重构的形状，编译器、闸门与行为测试**都不会响**（本函数只会变成死代码）。
/// 调用点的形状由 `tray_rebuild_call_sites_route_through_the_shared_attribution` 钉住。
pub(crate) fn attribute_rebuild_error(e: Box<dyn std::error::Error>) -> AppError {
    match e.downcast::<AppError>() {
        Ok(inner) => *inner,
        Err(other) => AppError::coded("tray.rebuild_failed").with("detail", other.to_string()),
    }
}

/// Create the system tray icon with menu.
pub fn create_tray(app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    const TRAY_ICON_BYTES: &[u8] = include_bytes!("../icons/tray-mascot.png");
    let icon = Image::from_bytes(TRAY_ICON_BYTES)?;
    let handle = app.handle().clone();
    let menu = build_tray_menu(app, None)?;

    TrayIconBuilder::with_id("main")
        .icon(icon)
        .icon_as_template(false)
        .tooltip("SeshBuddy")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(move |app, event| {
            let id = event.id().as_ref();
            if id == "show" {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.unminimize();
                    let _ = window.set_focus();
                }
            } else if id == "open_update_info" {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.unminimize();
                    let _ = window.set_focus();
                }
                let _ = app.emit("open-settings-about", ());
            } else if let Some(version) = id.strip_prefix("ignore_update:") {
                let version = version.to_string();
                if let Ok(mut settings) = app_db::read_updater_settings() {
                    settings.ignored_update_version = Some(version);
                    let _ = app_db::write_updater_settings(&settings);
                }
                let _ = rebuild_tray_menu_with_update(app, None);
            } else if let Some(profile_id) = id.strip_prefix("profile:") {
                if let Some((cli_id, profile_name)) = profile_id.split_once(':') {
                    let cli_id = cli_id.to_string();
                    let name = profile_name.to_string();
                    let _ = commands::apply_profile(Some(cli_id.clone()), name, None);
                    let _ = rebuild_tray_menu(&handle);
                    let _ = app.emit("profile-changed", serde_json::json!({ "cliId": cli_id }));
                }
            }
        })
        .on_tray_icon_event(|tray, event| {
            if let tauri::tray::TrayIconEvent::Click {
                button: tauri::tray::MouseButton::Left,
                ..
            } = event
            {
                // 250ms 防抖去重，避免 Down / Up 重复触发
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64;
                let last = LAST_TRAY_CLICK_TIME.swap(now, Ordering::Relaxed);
                if now.saturating_sub(last) < 250 {
                    return;
                }

                if let Some(window) = tray.app_handle().get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.unminimize();
                    let _ = window.set_focus();
                }
            }
        })
        .build(app)?;

    Ok(())
}

use std::sync::atomic::{AtomicU64, Ordering};

static LAST_TRAY_CLICK_TIME: AtomicU64 = AtomicU64::new(0);

#[cfg(test)]
mod tests {
    use super::attribute_rebuild_error;
    use crate::error::AppError;
    use regex::Regex;
    use serde_json::json;

    /// 线形状：前端拿到的那份 JSON。断言的是它，不是「函数应该会返回什么」。
    fn wire(error: AppError) -> serde_json::Value {
        serde_json::to_value(&error).expect("AppError 可序列化")
    }

    /// 内层是 `Coded` 时必须**原样放行**：线形状是它自己的 code，params 不被塞进 `detail`。
    ///
    /// 判据的来路：`rebuild_tray_menu_with_update` 里装箱的就是
    /// `Box::from(AppError::coded("tray.rebuild_channel_closed"))`（主线程闭包 panic 那条罕见路径），
    /// 这里用同一个表达式构造输入。
    ///
    /// 变异判据（实测过）：把 `attribute_rebuild_error` 的函数体改成
    /// `AppError::coded("tray.rebuild_failed").with("detail", e.to_string())`
    /// ——「丢掉拆箱但保留字面量」那种善意重构的形状 —— 本测试红：线形状会变成
    /// `{"code":"tray.rebuild_failed","params":{"detail":"tray.rebuild_channel_closed"}}`，
    /// 本地化的 `tray.rebuild_channel_closed` 降级成 `detail` 里的一段英文。
    #[test]
    fn attribute_rebuild_error_preserves_a_boxed_coded() {
        let boxed: Box<dyn std::error::Error> =
            Box::from(AppError::coded("tray.rebuild_channel_closed"));

        assert_eq!(
            wire(attribute_rebuild_error(boxed)),
            json!({ "code": "tray.rebuild_channel_closed", "params": {} })
        );
    }

    /// 反向：内层不是 `AppError`（`tauri::Error` / `io::Error`）时才归属成 `tray.rebuild_failed`，
    /// 原始文本进 `detail`（R3：第三方文本不翻译）。
    #[test]
    fn attribute_rebuild_error_attributes_only_foreign_errors() {
        let boxed: Box<dyn std::error::Error> = Box::new(std::io::Error::new(
            std::io::ErrorKind::Other,
            "menu build failed",
        ));

        assert_eq!(
            wire(attribute_rebuild_error(boxed)),
            json!({ "code": "tray.rebuild_failed", "params": { "detail": "menu build failed" } })
        );
    }

    /// 三个调用点必须把 `rebuild_tray_menu*` 的结果交给收口函数。
    ///
    /// 上一条只能证明「收口函数本身是对的」。把调用点改写成
    /// `coded("tray.rebuild_failed").with("detail", e.to_string())`（丢掉拆箱、保留字面量）时，
    /// 编译器不响、闸门绿（`coded("tray.rebuild_failed")` 字面量还在）、行为测试也绿
    /// —— 收口函数只是变成死代码。故形状本身也要钉。
    ///
    /// ⚠️ **这一对钉（上一条 + 本条的判据）看不到的四件事**，别把它当真实路径覆盖 ——
    /// 真实路径（`publish_update_available`）要一个 `AppHandle<Wry>`，单测里造不出来：
    /// - **源头的装箱没被行为钉住**：`rebuild_tray_menu_with_update` 里装箱的是
    ///   `Box::from(AppError::coded("tray.rebuild_channel_closed"))`，而上一条的输入是**手抄**的
    ///   同一个表达式。把源头换成别的 code，两条钉都绿；那一半只有闸门规则 10 的
    ///   「这个码有没有站点产出」在守。
    /// - **第四个调用点看不见**：下面的文件清单是写死的三个，仓库里别处新增一个不走收口函数的
    ///   `rebuild_tray_menu*` 调用点不会被抓住。
    /// - **恒等包装仍会匹配**：`.map_err(|e| e).map_err(…attribute_rebuild_error)` 也满足判据 ——
    ///   判据只认「紧跟一次 `.map_err(…attribute_rebuild_error)`」，不看中间是否被传递过。
    /// - **判据对形状敏感**：表达式被 `;` 拆开（先 `let r = rebuild_tray_menu(…)`，再
    ///   `.map_err(…)`）会让本条报红，而那是良性重构 —— 红了先看是不是拆行。
    #[test]
    fn tray_rebuild_call_sites_route_through_the_shared_attribution() {
        // 判据：托盘调用的结果**紧跟**一次 `.map_err(<路径>::attribute_rebuild_error)`。
        // 空白先剥掉，免得换行与缩进让判据漏判。
        let routed = Regex::new(
            r"rebuild_tray_menu[a-z_]*\([^;]*?\.map_err\([a-zA-Z_:]*attribute_rebuild_error\)",
        )
        .expect("正则");

        // 阳性对照：合法形状报得出；被变异过的形状（丢掉拆箱、保留字面量）报不出。
        assert!(routed.is_match(
            "crate::tray::rebuild_tray_menu(&h).map_err(crate::tray::attribute_rebuild_error)"
        ));
        assert!(!routed.is_match(
            r#"crate::tray::rebuild_tray_menu(&h).map_err(|e| AppError::coded("tray.rebuild_failed").with("detail", e.to_string()))"#
        ));

        for (name, source) in [
            ("updater.rs", include_str!("updater.rs")),
            ("commands/updater.rs", include_str!("commands/updater.rs")),
            ("commands/profile.rs", include_str!("commands/profile.rs")),
        ] {
            let compact: String = source.chars().filter(|c| !c.is_whitespace()).collect();
            assert_eq!(
                routed.find_iter(&compact).count(),
                1,
                "{name} 的托盘重建调用点没有走 tray::attribute_rebuild_error"
            );
        }
    }

    /// 托盘短名必须与改动前逐字相同。
    #[test]
    fn tray_labels_match_previous_values() {
        use crate::cli::CliKind;
        let expected = [
            (CliKind::Claude, "Claude"),
            (CliKind::Codex, "Codex"),
            (CliKind::Gemini, "Gemini"),
            (CliKind::WorkBuddy, "WorkBuddy"),
            (CliKind::Dsh, "DSH"),
            (CliKind::Antigravity, "Antigravity"),
        ];
        for (kind, label) in expected {
            assert_eq!(
                crate::cli_registry::descriptor_for(kind).tray_label,
                label,
                "{kind:?} 的托盘短名与改动前不同"
            );
        }
    }
}

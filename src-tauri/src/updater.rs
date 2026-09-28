use crate::app_db::{self, UpdaterSettings};
use crate::error::{AppError, AppResult};
use crate::native_text::{native_text, native_text_with};
use serde::Serialize;
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_updater::UpdaterExt;

const UPDATE_AVAILABLE_EVENT: &str = "update-available";
const UPDATE_CLEARED_EVENT: &str = "update-cleared";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdaterState {
    pub current_version: String,
    pub settings: UpdaterSettings,
    pub has_token: bool,
}

/// Pending Tauri updater update, held in app state waiting for install
pub struct PendingTauriUpdate(pub tauri_plugin_updater::Update);

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TauriUpdateInfo {
    pub version: String,
    pub notes: Option<String>,
    pub pub_date: Option<String>,
    pub release_page_url: Option<String>,
}

pub fn current_app_version(app: &AppHandle) -> String {
    app.package_info().version.to_string()
}

/// 读 updater 设置并组装状态。返回 `AppResult` 而非压平成 `String`：本文件四个读 / 写
/// updater 设置的站点原先各写一次 `.map_err(|e| e.to_string())`，而那两个 helper 返回
/// `AppResult`——底层错误一旦是 `Coded`，`Display` 就是**裸 code**（`params` 全丢），
/// 前端 `renderAppError` 认不出是码、原样上屏。`get_updater_state` 直通 updater 面板的
/// 渲染点，是 `error.rs` 里点名过的真泄漏之一；故整条链的签名加宽成 `AppResult`，
/// 四个 flatten 一并删掉。**实测（计划 11 的 T5 之后）**：把 `build_updater_state` 的签名改回
/// `Result<UpdaterState, String>`，函数体里的 `read_updater_settings()?` 报 `E0277`、
/// 本文件的守卫报 `E0308`——**不再「编译得过」**（T5 删掉了 `From<AppError> for String`）。
/// 但**连函数体一起回退**不碰任何 `AppError`，编译器看不见，故仍由本文件的
/// `app_error_signature_guard` 钉在类型上。
fn build_updater_state(app: &AppHandle) -> AppResult<UpdaterState> {
    Ok(UpdaterState {
        current_version: current_app_version(app),
        settings: app_db::read_updater_settings()?,
        has_token: false,
    })
}

pub fn get_updater_state(app: &AppHandle) -> AppResult<UpdaterState> {
    build_updater_state(app)
}

pub async fn check_tauri_update(app: &AppHandle) -> AppResult<Option<TauriUpdateInfo>> {
    let updater = app
        .updater_builder()
        .build()
        .map_err(|e| AppError::coded("updater.builder_failed").with("detail", e.to_string()))?;

    let update = updater
        .check()
        .await
        .map_err(|e| AppError::coded("updater.check_failed").with("detail", e.to_string()))?;

    match update {
        None => Ok(None),
        Some(u) => {
            tracing::info!("[update-check] update found: v{}", u.version);
            let info = TauriUpdateInfo {
                version: u.version.clone(),
                notes: u.body.clone(),
                pub_date: u
                    .date
                    .and_then(|d| {
                        d.format(&time::format_description::well_known::Rfc3339)
                            .ok()
                    }),
                release_page_url: None,
            };
            let state = app.state::<Mutex<Option<PendingTauriUpdate>>>();
            *lock_pending(&state)? = Some(PendingTauriUpdate(u));
            Ok(Some(info))
        }
    }
}

/// 取待安装更新的状态锁。`PoisonError` 是**第三方**文本（R3：进 `params.detail`，不翻译）。
///
/// **为什么抽成一处**：三个站点原先各写一遍 `.map_err(|e| e.to_string())?`，那是**函数体里的
/// 表达式**，fn 指针守卫钉不到函数体。抽成有名字的函数后，它的签名就能被
/// `app_error_signature_guard` 钉在 `AppResult` 上。
///
/// ⚠️ **别把这里的机制写成「函数体里的 `?`」**：本函数体是一条 `map_err(...)` 尾表达式，
/// **没有 `?`**。实测（`cargo check --tests`）把签名单独改回 `Result<_, String>` 得到的是
/// **4 条 `E0308`、0 条 `E0277`** —— 函数体那条是返回类型不匹配（lib 目标 1 条，同一个错在
/// lib test 目标里再报一次），另两条来自本文件的守卫 `:262` 与行为钉 `:293`。
/// 连函数体一起回退（`.map_err(|e| e.to_string())`）后函数体自己不再报错，只剩那两条
/// `E0308` 钉着 —— 所以「这条签名有守卫」靠的是那两条钉，不是函数体里的 `?`。
fn lock_pending(
    state: &Mutex<Option<PendingTauriUpdate>>,
) -> AppResult<std::sync::MutexGuard<'_, Option<PendingTauriUpdate>>> {
    state
        .lock()
        .map_err(|e| AppError::coded("internal.lock_poisoned").with("detail", e.to_string()))
}

fn publish_update_available(
    app: &AppHandle,
    update_info: &TauriUpdateInfo,
    notify_system: bool,
) -> AppResult<()> {
    let version = update_info.version.clone();

    // 收口在 `tray::attribute_rebuild_error` 里：先拆箱再归属，保住内层 `Coded` 的 code 与
    // params（`Box<dyn Error>` 的 `Display` 只渲染裸 code）。三个调用点共用它。
    crate::tray::rebuild_tray_menu_with_update(app, Some(&version))
        .map_err(crate::tray::attribute_rebuild_error)?;
    tracing::info!("[update-check] tray menu rebuilt with version {}", version);

    app.emit(UPDATE_AVAILABLE_EVENT, update_info)
        .map_err(|e| AppError::coded("updater.emit_failed").with("detail", e.to_string()))?;
    tracing::info!("[update-check] emitted update-available event to frontend");

    if notify_system {
        use tauri_plugin_notification::NotificationExt;
        if let Err(err) = app
            .notification()
            .builder()
            .title(native_text("native", "updateNotificationTitle"))
            .body(&native_text_with(
                "native",
                "updateNotificationBody",
                "version",
                &version,
            ))
            .show()
        {
            tracing::warn!("[update-check] failed to show system notification: {}", err);
        }
    }

    Ok(())
}

pub fn publish_manual_update_available(
    app: &AppHandle,
    update_info: &TauriUpdateInfo,
) -> AppResult<()> {
    publish_update_available(app, update_info, true)
}

pub fn emit_update_cleared(app: &AppHandle) {
    let _ = app.emit(UPDATE_CLEARED_EVENT, ());
}

/// 安装待处理更新。**返回 `AppResult` 而非 `Result<_, String>`**：错误槽里现在装的
/// 是我们自己写的那句话的**码**（`updater.no_pending_update`），`Coded` 只能经
/// `AppResult` 跨 IPC；`Result<_, String>` 会把它压成裸 code。
pub async fn install_tauri_update(app: &AppHandle) -> AppResult<()> {
    let state = app.state::<Mutex<Option<PendingTauriUpdate>>>();
    let update = lock_pending(&state)?
        .take()
        .ok_or_else(|| AppError::coded("updater.no_pending_update"))?;

    let app_clone = app.clone();
    match update
        .0
        .download_and_install(
            move |chunk_len, total| {
                let _ = app_clone.emit(
                    "tauri-updater-progress",
                    serde_json::json!({
                        "chunkLen": chunk_len,
                        "total": total
                    }),
                );
            },
            || {},
        )
        .await
    {
        Ok(()) => app.restart(),
        Err(e) => {
            *lock_pending(&state)? = Some(update);
            // updater 插件的原文是**第三方**（R3）：进 `params.detail`，包装文案由语言包给。
            Err(AppError::coded("updater.install_failed").with("detail", e.to_string()))
        }
    }
}

pub async fn check_and_notify_update(app: &AppHandle) -> AppResult<()> {
    let update_info = match check_tauri_update(app).await? {
        None => return Ok(()),
        Some(info) => info,
    };

    let version = update_info.version.clone();
    tracing::info!("[update-check] check_and_notify_update processing v{}", version);

    let settings = app_db::read_updater_settings()?;
    
    let is_ignored = settings
        .ignored_update_version
        .as_deref()
        .and_then(|v| semver::Version::parse(v).ok())
        .zip(semver::Version::parse(&version).ok())
        .map(|(ignored, detected)| detected <= ignored)
        .unwrap_or(false);

    if is_ignored {
        tracing::info!("[update-check] version {} is ignored, skipping", version);
        return Ok(());
    }

    publish_update_available(app, &update_info, true)?;

    Ok(())
}

#[cfg(test)]
mod app_error_signature_guard {
    use super::*;
    use std::sync::MutexGuard;

    /// 类型级回归守卫：本文件读 updater 设置的那条链必须一路以 `AppError` 到 IPC。
    ///
    /// **为什么必须有**：这条链原先在每个读 / 写站点各写一次
    /// `.map_err(|e| e.to_string())`，把 `AppResult` 压成 `String`；`Coded` 的 `Display`
    /// 就是裸 code，压平后 `params` 全丢，`get_updater_state` 直接把它送到 updater 面板上，
    /// 用户看到的是 `internal.database` / `internal.json` 这样的内部标识符
    /// （`read_setting_json` 的 `conn()?` 与 `serde_json::from_str(...)`，实测这两个码）。
    /// **实测（计划 11 的 T5 之后）**：把 `build_updater_state` 改回 `Result<_, String>`，
    /// `?` 报 `E0277`、本守卫报 `E0308`——**不再「没有任何编译期症状」**（T5 删掉了
    /// `From<AppError> for String`）。**但连签名带函数体一起回退**不碰任何 `AppError`，
    /// 编译器看不见，闸门规则 2 也只认 CJK 码点。故这里把签名钉死在类型上：
    /// 改回 `Result<_, String>` 就**编译不过**，而不是静默放行。
    ///
    /// 只钉类型，不跑逻辑：这些函数会真的去开数据库、发网络请求。
    ///
    /// `async fn` 的签名没法用 fn 指针直接钉（返回类型是 `impl Future`，写不出来），
    /// 故用一个辅助函数：约束落在 future 的 `Output` 上。
    fn pin1<A, T, F>(_: fn(A) -> F)
    where
        F: std::future::Future<Output = AppResult<T>>,
    {
    }

    #[test]
    fn updater_errors_reach_ipc_as_app_error() {
        let _: fn(&AppHandle) -> AppResult<UpdaterState> = build_updater_state;
        let _: fn(&AppHandle) -> AppResult<UpdaterState> = get_updater_state;
        let _: fn(&AppHandle, &TauriUpdateInfo, bool) -> AppResult<()> = publish_update_available;
        let _: fn(&AppHandle, &TauriUpdateInfo) -> AppResult<()> = publish_manual_update_available;
        // 三个 `state.lock()` 站点走这个助手：它们是函数体里的表达式，fn 指针钉不到，
        // 抽成有名字的函数才钉得住（单改签名 T5 之后已编译不过，但连函数体一起回退
        // 编译器看不见，故仍需把 `lock_pending` 的签名钉在类型上）。
        let _: fn(&Mutex<Option<PendingTauriUpdate>>) -> AppResult<MutexGuard<'_, Option<PendingTauriUpdate>>> =
            lock_pending;
        pin1(check_tauri_update);
        pin1(check_and_notify_update);
        // `install_tauri_update` 的错误槽里现在装的是 `updater.no_pending_update`（Coded）：
        // 改回 `Result<_, String>` 后，函数体里的 `lock_pending(...)?` 与 `ok_or_else(...)?`
        // 会因 `From<AppError> for String` 已删（计划 11 的 T5）而**编译不过**——但那覆盖的
        // 只是「签名改了、函数体没改」这一种回退。**连函数体一起回退**（`?` 的接收方也一并
        // 变回 `String`）不碰任何 `AppError`，编译器抓不到，故仍需这条 fn 指针把签名钉住。
        pin1(install_tauri_update);
    }

    /// 行为证明（上一条只钉类型，钉不住 `lock_pending` 的**函数体**）：锁被毒化时
    /// 必须给出 `Coded("internal.lock_poisoned")` 且带非空 `detail`。
    ///
    /// 变异判据：把 `lock_pending` 的函数体改回 `.map_err(|e| e.to_string())`，
    /// 线格式变成 `Business("<PoisonError 的英文>")`，本测试红。
    #[test]
    fn lock_pending_reports_poison_as_coded() {
        let poisoned: Mutex<Option<PendingTauriUpdate>> = Mutex::new(None);
        // 持锁期间 panic 即毒化该锁；`catch_unwind` 只为不让测试进程被这个 panic 带走。
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = poisoned.lock().expect("首次加锁成功");
            panic!("故意毒化这把锁");
        }));

        // 不用 `expect_err`：`MutexGuard` 不是 `Debug`（`PendingTauriUpdate` 没有 `Debug`）。
        let err = match lock_pending(&poisoned) {
            Ok(_) => panic!("锁已毒化，必须失败"),
            Err(e) => e,
        };
        match err {
            AppError::Coded { code, params } => {
                assert_eq!(code, "internal.lock_poisoned");
                assert!(
                    params.get("detail").is_some_and(|d| !d.is_empty()),
                    "必须带非空 params.detail"
                );
            }
            other => panic!("退回了非结构化错误：{other:?}"),
        }
    }
}

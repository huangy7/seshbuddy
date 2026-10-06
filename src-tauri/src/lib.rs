mod db;
pub mod agent_status;
pub mod claude_hooks;
pub mod shell_launch;
mod app_db {
    pub(crate) use crate::db::*;
}
mod error;
mod streaming;
mod cli;
mod cli_registry;
mod cli_config;
mod paths;
mod pty_manager;
mod commands;
mod context_menu;
mod git;
mod history;
mod parser;
mod title_resolver;
mod proxy;
mod session;
mod session_index_watcher;
mod terminal;
mod tray;
mod native_text;
mod updater;
mod agent_detect;
mod assistant;
mod pricing;

use std::sync::atomic::{AtomicBool, Ordering};

static STOP_DB_MAINTENANCE: AtomicBool = AtomicBool::new(false);

use tauri::Manager;
use tracing_subscriber::fmt;


fn init_logging() -> Option<tracing_appender::non_blocking::WorkerGuard> {
    let data_dir = crate::paths::app_data_dir().ok()?.join("logs");
    std::fs::create_dir_all(&data_dir).ok()?;

    let log_path = data_dir.join("seshbuddy.log");
    let file_appender = tracing_rolling_file::RollingFileAppenderBase::builder()
        .filename(log_path.to_string_lossy().into_owned())
        .max_filecount(2) // seshbuddy.log + seshbuddy.log.1 + seshbuddy.log.2
        .condition_max_file_size(10 * 1024 * 1024) // 10MB
        .build()
        .ok()?;

    let (non_blocking, guard) = file_appender.get_non_blocking_appender();
    fmt::Subscriber::builder()
        .with_writer(non_blocking)
        .with_ansi(false)
        .init();

    Some(guard)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    commands::onboarding::capture_install_state();
    let _log_guard = init_logging();

    tracing::info!("SeshBuddy starting");

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::Builder::new().callback(|app, _args, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.set_focus();
            }
        }).build())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .manage(commands::workspace_fs::FileWatcherStateType::default())
        .manage(git::GitWatcherState::default())
        .manage(std::sync::Mutex::<Option<updater::PendingTauriUpdate>>::new(None))
        .invoke_handler(tauri::generate_handler![
            commands::scan_projects_stream,
            commands::refresh_session_list_index,
            commands::get_search_index_status,
            commands::get_index_stats,
            commands::clear_session_index,
            commands::ensure_search_index_ready,
            commands::list_cli_statuses,
            commands::list_cli_path_configs,
            commands::set_cli_data_dir,
            commands::detect_cli,
            commands::get_system_diagnostics,
            commands::get_current_user_name,
            commands::delete_sessions_to_trash,
            commands::export_session,
            commands::get_launch_command,
            commands::clean_temp_configs,
            commands::open_in_terminal,
            commands::detect_terminal_apps,
            commands::register_context_menu,
            commands::unregister_context_menu,
            commands::is_context_menu_registered,
            commands::search_sessions_stream,
            commands::read_scope_settings,
            commands::write_scope_settings,
            commands::list_profiles,
            commands::save_profile,
            commands::delete_profile,
            commands::get_scope_bindings,
            commands::set_scope_binding,
            commands::read_profile,
            commands::rename_profile,
            commands::get_active_profile,
            commands::export_profiles_backup,
            commands::read_profiles_backup,
            commands::parse_profiles_backup,
            commands::import_profiles_backup,
            commands::sync_active_profile_from_cli,
            commands::create_profile_tab,
            commands::update_profile_tab,
            commands::delete_profile_tab,
            commands::list_profile_tabs,
            commands::detect_scope_dirs_config,
            commands::import_scope_dir_config,
            commands::get_scope_dir_status,
            commands::match_profile_by_settings,
            commands::refresh_tray_menu,
            commands::get_app_locale,
            commands::set_app_locale,
            commands::get_onboarding_status,
            commands::complete_onboarding,
            session_index_watcher::set_session_index_scan_clis,
            commands::read_favorite_entries,
            commands::write_favorite_entries,
            commands::read_blocked_folders,
            commands::write_blocked_folders,
            commands::rename_session,
            commands::get_usage_stats,
            pricing::refresh_pricing_catalog,
            commands::get_session_stats,
            commands::resolve_session_id,
            commands::batch_export_sessions,
            commands::fork_session,
            commands::read_bookmarks,
            commands::toggle_bookmark,
            commands::list_bookmarks_with_context,
            commands::get_archive_retention_days,
            commands::set_archive_retention_days,
            commands::get_session_archive_pinned,
            commands::get_session_archive_status,
            commands::set_session_archive_pinned,
            commands::list_archived_sessions,
            commands::delete_session_archive,
            commands::restore_session_to_disk,
            commands::rebuild_archive_index_from_disk,
            commands::open_path_in_file_manager,
            commands::check_paths_exist,
            commands::proxy_enable,
            commands::proxy_disable,
            commands::proxy_status,
            commands::proxy_get_detail,
            commands::proxy_clear_traffic,
            commands::proxy_db_size,
            commands::proxy_session_traffic_timestamps,
            commands::proxy_get_sessions_stream,
            commands::proxy_get_session_traffic,
            commands::proxy_find_by_timestamp,
            commands::load_session_incremental,
            commands::load_session_stream,
            commands::get_tool_result_full_content,
            commands::get_dock_visible,
            commands::set_dock_visible,
            commands::get_updater_state,
            commands::set_updater_settings,
            commands::updater::check_tauri_update,
            commands::updater::install_tauri_update,
            commands::updater::check_tauri_update_bg,
            commands::updater::ignore_update_version,
            commands::pty::create_pty_session,
            commands::pty::write_pty_session,
            commands::pty::resize_pty_session,
            commands::pty::close_pty_session,
            commands::pty::list_pty_sessions,
            commands::workspace_fs::list_project_files,
            commands::workspace_fs::read_directory,
            commands::workspace_fs::read_file_content,
            commands::workspace_fs::validate_workspace_file,
            commands::workspace_fs::save_file,
            commands::workspace_fs::delete_file,
            commands::workspace_fs::watch_file,
            commands::workspace_fs::unwatch_file,
            commands::fetch_available_models,
            assistant::commands::assistant_send,
            assistant::commands::assistant_cache_status,
            assistant::commands::assistant_backfill_cache,
            assistant::commands::assistant_conversation_messages,
            assistant::commands::assistant_list_conversations,
            assistant::commands::assistant_delete_conversation,
            assistant::commands::assistant_list_quick_phrases,
            assistant::commands::assistant_save_quick_phrase,
            assistant::commands::assistant_delete_quick_phrase,
            assistant::commands::assistant_engine_status,
            assistant::commands::assistant_cancel,
            assistant::commands::assistant_open_window,
            assistant::commands::assistant_open_with_prompt,
            assistant::commands::assistant_take_pending_prompt,
            assistant::commands::resolve_session_refs,
            assistant::commands::assistant_open_session,
            assistant::commands::assistant_set_always_on_top,
            git::git_status,
            git::git_log,
            git::git_branches,
            git::git_is_repo,
            git::git_diff_file_content,
            git::git_commit_diff,
            git::git_commit_file_diff,
            git::watch_git_state,
            git::unwatch_git_state,
        ])
        .setup(|app| {
            if let Err(e) = session_index_watcher::restart_session_index_watchers(&app.handle()) {
                tracing::warn!("Session index watcher init warning: {}", e.diagnostic());
            }
            tray::create_tray(app)?;
            pty_manager::inject_shell_env_into_process();
            pty_manager::start_guardian(app.handle().clone());
            pty_manager::preload_shell_env();
            // Apply saved dock visibility setting (macOS only)
            #[cfg(target_os = "macos")]
            {
                let _ = commands::refresh_macos_app_icon(&app.handle());
                if !commands::read_dock_visible_setting() {
                    let _ = app.set_dock_visibility(false);
                }
            }
            // Always show the main window on startup.
            // This is especially important after an updater restart: the previous instance
            // hides the window on CloseRequested (minimize-to-tray behaviour), so the new
            // process would start with the window invisible without this explicit show call.
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.set_focus();
            }

            // Check proxy state consistency in background so startup UI is not blocked.
            std::thread::spawn(|| {
                proxy::cleanup_stale_proxy_processes();
                proxy::check_consistency();
            });
            // Run database maintenance in background: purge orphan session index rows,
            // refresh aging archive snapshots, and clear expired caches/archives.
            // Delay 30s so the UI can initialize without DB mutex contention.
            std::thread::spawn(|| {
                std::thread::sleep(std::time::Duration::from_secs(30));
                app_db::run_startup_maintenance();
                loop {
                    std::thread::sleep(std::time::Duration::from_secs(6 * 60 * 60));
                    if STOP_DB_MAINTENANCE.load(Ordering::Relaxed) {
                        break;
                    }
                    app_db::run_startup_maintenance();
                }
            });
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                // Hide window instead of closing (minimize to tray)
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|_app, event| {
            match event {
                #[cfg(target_os = "macos")]
                tauri::RunEvent::Reopen { .. } => {
                    if let Some(window) = _app.get_webview_window("main") {
                        let _ = window.show();
                        let _ = window.set_focus();
                    }
                }
                tauri::RunEvent::Exit => {
                    // Signal background threads to stop
                    STOP_DB_MAINTENANCE.store(true, Ordering::Relaxed);
                    session_index_watcher::shutdown_session_index_watchers();
                    pty_manager::shutdown_guardian();
                    // Clean up resources
                    proxy::cleanup_on_exit();
                    pty_manager::cleanup_all();
                    git::cleanup_watchers();
                }
                _ => {}
            }
        });
}

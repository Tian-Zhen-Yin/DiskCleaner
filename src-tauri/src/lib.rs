pub mod analyzer;
mod autostart;
mod cleaner;
mod config;
mod history;
mod models;
mod paths;
mod scheduler;
mod sys_task;

use std::sync::Arc;

use models::{
    CleanCategory, CleanResult, DiskInfo, HistoryRecord, ScanResult, ScheduleConfig,
};
use scheduler::Scheduler;
use tauri::{Manager, State};

struct AppState {
    config: config::ConfigStore,
    scheduler: Arc<Scheduler>,
    history: Arc<history::HistoryStore>,
}

#[tauri::command]
fn get_disk_info(drive: String) -> Result<DiskInfo, String> {
    cleaner::get_disk_info(&drive)
}

#[tauri::command]
async fn scan_all(categories: Vec<CleanCategory>) -> Result<Vec<ScanResult>, String> {
    let results = tokio::task::spawn_blocking(move || {
        categories
            .into_iter()
            .map(cleaner::scan_category)
            .collect::<Vec<_>>()
    })
    .await
    .map_err(|e| e.to_string())?;
    Ok(results)
}

#[tauri::command]
async fn clean_categories(
    state: State<'_, AppState>,
    categories: Vec<CleanCategory>,
) -> Result<Vec<CleanResult>, String> {
    let results = tokio::task::spawn_blocking(move || {
        categories
            .into_iter()
            .map(cleaner::clean_category)
            .collect::<Vec<_>>()
    })
    .await
    .map_err(|e| e.to_string())?;

    let entries = history::build_entries_from_results(&results);
    let _ = state.history.append(entries, "manual");

    Ok(results)
}

#[tauri::command]
fn get_schedule(state: State<'_, AppState>) -> ScheduleConfig {
    state.config.get()
}

#[tauri::command]
async fn set_schedule(
    state: State<'_, AppState>,
    config: ScheduleConfig,
) -> Result<(), String> {
    state.config.set(config.clone())?;
    state
        .scheduler
        .reload(config, state.history.clone())
        .await;
    Ok(())
}

#[tauri::command]
fn register_system_task(config: ScheduleConfig) -> Result<String, String> {
    sys_task::register(&config)
}

#[tauri::command]
fn unregister_system_task() -> Result<String, String> {
    sys_task::unregister()
}

#[tauri::command]
fn get_history(state: State<'_, AppState>) -> Vec<HistoryRecord> {
    state.history.list()
}

#[tauri::command]
fn clear_history(state: State<'_, AppState>) -> Result<(), String> {
    state.history.clear()
}

#[tauri::command]
fn get_autostart() -> bool {
    autostart::is_enabled()
}

#[tauri::command]
fn set_autostart(enabled: bool, silent: bool) -> Result<String, String> {
    if enabled {
        autostart::enable(silent)
    } else {
        autostart::disable()
    }
}

pub fn run() {
    if let Some(args) = parse_headless() {
        let results = scheduler::run_cli_clean(&args);
        let store = history::HistoryStore::new();
        let entries = history::build_entries_from_cli(&results);
        let _ = store.append(entries, "headless");
        return;
    }

    let config_store = config::ConfigStore::new();
    let history_store = Arc::new(history::HistoryStore::new());
    let scheduler = Arc::new(Scheduler::new());
    let initial_cfg = config_store.get();

    let state = AppState {
        config: config_store,
        scheduler: scheduler.clone(),
        history: history_store.clone(),
    };

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_shell::init())
        .manage(state)
        .setup(move |app| {
            let scheduler_cloned = scheduler.clone();
            let history_cloned = history_store.clone();
            let cfg = initial_cfg.clone();
            tauri::async_runtime::spawn(async move {
                scheduler_cloned.reload(cfg, history_cloned).await;
            });
            let _ = app.get_webview_window("main");
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_disk_info,
            scan_all,
            clean_categories,
            get_schedule,
            set_schedule,
            register_system_task,
            unregister_system_task,
            get_history,
            clear_history,
            get_autostart,
            set_autostart,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

fn parse_headless() -> Option<Vec<CleanCategory>> {
    let mut args = std::env::args().skip(1);
    let first = args.next()?;
    if first != "--headless-clean" {
        return None;
    }
    let mut categories = Vec::new();
    for a in args {
        match a.as_str() {
            "WindowsTemp" => categories.push(CleanCategory::WindowsTemp),
            "BrowserCache" => categories.push(CleanCategory::BrowserCache),
            "WindowsUpdate" => categories.push(CleanCategory::WindowsUpdate),
            "RecycleBin" => categories.push(CleanCategory::RecycleBin),
            "Thumbnails" => categories.push(CleanCategory::Thumbnails),
            "ErrorReports" => categories.push(CleanCategory::ErrorReports),
            "MemoryDumps" => categories.push(CleanCategory::MemoryDumps),
            "WindowsLogs" => categories.push(CleanCategory::WindowsLogs),
            "DeliveryOptimization" => {
                categories.push(CleanCategory::DeliveryOptimization)
            }
            "Prefetch" => categories.push(CleanCategory::Prefetch),
            "DevCaches" => categories.push(CleanCategory::DevCaches),
            _ => {}
        }
    }
    if categories.is_empty() {
        categories = vec![
            CleanCategory::WindowsTemp,
            CleanCategory::BrowserCache,
            CleanCategory::WindowsUpdate,
        ];
    }
    Some(categories)
}

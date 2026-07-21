pub mod analyzer;
pub mod analyzer_store;
pub mod classify;
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
    AnalyzerConfig, CleanCategory, CleanResult, DirDelta, DiskInfo, HistoryRecord,
    MonitorEntry, MonitorSnapshot, ScanResult, ScheduleConfig, SnapshotSummary,
};
use scheduler::Scheduler;
use tauri::{Emitter, Manager, State};

struct AppState {
    config: config::ConfigStore,
    scheduler: Arc<Scheduler>,
    history: Arc<history::HistoryStore>,
    analyzer_store: Arc<analyzer_store::SnapshotStore>,
    analyzer_config: Arc<analyzer_store::AnalyzerConfigStore>,
}

fn drive_c_info() -> (u64, u64) {
    cleaner::get_disk_info("C:")
        .map(|d| (d.total_bytes, d.used_bytes))
        .unwrap_or((0, 0))
}

// ===== Analyzer Tauri commands =====

#[tauri::command]
async fn analyze_full_scan(app: tauri::AppHandle, state: State<'_, AppState>) -> Result<MonitorSnapshot, String> {
    let cfg = state.analyzer_config.get();
    let monitor_dirs = cfg.monitor_dirs.clone();
    let min_bytes = cfg.large_file_min_bytes;
    let top_n = cfg.large_file_top_n;
    let auto_append: Vec<String> = state
        .analyzer_store
        .latest()
        .map(|s| s.large_files.iter().map(|f| parent_dir_string(&f.path)).collect())
        .unwrap_or_default();
    let store = state.analyzer_store.clone();
    let snap = tokio::task::spawn_blocking(move || {
        let (total, used) = drive_c_info();
        analyzer::scan_full(monitor_dirs, auto_append, min_bytes, top_n, total, used, |done, total_dirs| {
            let _ = app.emit("analyze-scan-progress", serde_json::json!({ "done": done, "total": total_dirs }));
        })
    })
    .await
    .map_err(|e| e.to_string())?;
    store.save(&snap)?;
    Ok(snap)
}

#[tauri::command]
async fn analyze_rescan_monitors(state: State<'_, AppState>) -> Result<Vec<MonitorEntry>, String> {
    let dirs = state.analyzer_config.get().monitor_dirs;
    tokio::task::spawn_blocking(move || Ok(analyzer::scan_monitor_dirs(&dirs)))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
fn analyze_get_latest(state: State<'_, AppState>) -> Option<MonitorSnapshot> {
    state.analyzer_store.latest()
}

#[tauri::command]
fn analyze_list_snapshots(state: State<'_, AppState>) -> Vec<SnapshotSummary> {
    state.analyzer_store.list()
}

#[tauri::command]
async fn analyze_drilldown(path: String) -> Result<Vec<MonitorEntry>, String> {
    tokio::task::spawn_blocking(move || Ok(analyzer::list_children(std::path::Path::new(&path))))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
fn analyze_diff(prev_ts: String, curr_ts: String, state: State<'_, AppState>) -> Result<Vec<DirDelta>, String> {
    let prev = state.analyzer_store.load(&prev_ts).ok_or("prev snapshot not found")?;
    let curr = state.analyzer_store.load(&curr_ts).ok_or("curr snapshot not found")?;
    Ok(analyzer::diff_snapshots(&prev, &curr))
}

#[tauri::command]
fn analyze_get_config(state: State<'_, AppState>) -> AnalyzerConfig {
    state.analyzer_config.get()
}

#[tauri::command]
fn analyze_set_config(config: AnalyzerConfig, state: State<'_, AppState>) -> Result<(), String> {
    state.analyzer_config.set(config)
}

fn parent_dir_string(path: &str) -> String {
    std::path::Path::new(path)
        .parent()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default()
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
        analyzer_store: Arc::new(analyzer_store::SnapshotStore::new(analyzer_store::SnapshotStore::default_root())),
        analyzer_config: Arc::new(analyzer_store::AnalyzerConfigStore::new()),
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
            analyze_full_scan,
            analyze_rescan_monitors,
            analyze_get_latest,
            analyze_list_snapshots,
            analyze_drilldown,
            analyze_diff,
            analyze_get_config,
            analyze_set_config,
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

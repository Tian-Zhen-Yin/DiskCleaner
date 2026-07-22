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
    AnalyzerConfig, CleanAdvice, CleanCategory, CleanResult, DeleteResult, DirDelta, DiskInfo,
    HistoryRecord, MonitorEntry, MonitorSnapshot, ScanResult, ScheduleConfig, SnapshotSummary,
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

/// Delete files that classify as `Safe`. Each path is re-validated with
/// `classify::classify()` (the backend safety gate); non-Safe paths are
/// rejected into `errors` and never touched. `on_progress` is invoked once per
/// processed path with `(deleted, total, freed_bytes)` so callers can emit
/// progress events. Extracted as a pure function for unit testing.
fn perform_delete<F: FnMut(u64, u64, u64)>(paths: &[String], mut on_progress: F) -> DeleteResult {
    let total = paths.len() as u64;
    let mut deleted_count = 0u64;
    let mut freed_bytes = 0u64;
    let mut errors: Vec<String> = Vec::new();
    for path in paths {
        let (advice, _) = classify::classify(path);
        if advice != CleanAdvice::Safe {
            errors.push(format!("拒绝删除(非安全类): {}", path));
            on_progress(deleted_count, total, freed_bytes);
            continue;
        }
        let size = match std::fs::metadata(path) {
            Ok(m) => m.len(),
            Err(e) => {
                errors.push(format!("{}: {}", path, e));
                on_progress(deleted_count, total, freed_bytes);
                continue;
            }
        };
        match std::fs::remove_file(path) {
            Ok(()) => {
                deleted_count += 1;
                freed_bytes += size;
            }
            Err(e) => {
                errors.push(format!("{}: {}", path, e));
            }
        }
        on_progress(deleted_count, total, freed_bytes);
    }
    DeleteResult {
        deleted_count,
        freed_bytes,
        errors,
    }
}

#[tauri::command]
async fn analyze_delete_files(
    app: tauri::AppHandle,
    paths: Vec<String>,
) -> Result<DeleteResult, String> {
    let result = tokio::task::spawn_blocking(move || {
        perform_delete(&paths, |deleted, total, freed_bytes| {
            let _ = app.emit(
                "analyze-delete-progress",
                serde_json::json!({ "deleted": deleted, "total": total, "freed_bytes": freed_bytes }),
            );
        })
    })
    .await
    .map_err(|e| e.to_string())?;
    Ok(result)
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
            analyze_delete_files,
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

#[cfg(test)]
mod tests {
    use super::*;

    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

    fn tmp(name: &str) -> std::path::PathBuf {
        let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let mut p = std::env::temp_dir();
        p.push(format!("dct_del_{}_{}", name, n));
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    fn cleanup(p: &std::path::Path) {
        let _ = std::fs::remove_dir_all(p);
    }

    #[test]
    fn delete_safe_file_succeeds() {
        let root = tmp("safe");
        let cache_dir = root.join("Cache");
        std::fs::create_dir_all(&cache_dir).unwrap();
        let file = cache_dir.join("blob.bin");
        std::fs::write(&file, vec![0u8; 1024]).unwrap();
        let path = file.to_string_lossy().into_owned();
        let result = perform_delete(&[path], |_, _, _| {});
        assert_eq!(result.deleted_count, 1);
        assert_eq!(result.freed_bytes, 1024);
        assert!(result.errors.is_empty());
        assert!(!file.exists());
        cleanup(&root);
    }

    #[test]
    fn non_safe_path_is_rejected() {
        let path = r"C:\Program Files\fakeapp\app.exe".to_string();
        let result = perform_delete(&[path], |_, _, _| {});
        assert_eq!(result.deleted_count, 0);
        assert_eq!(result.freed_bytes, 0);
        assert_eq!(result.errors.len(), 1);
        assert!(result.errors[0].contains("拒绝删除"));
    }

    #[test]
    fn missing_safe_file_goes_to_errors() {
        let root = tmp("missing");
        let cache_dir = root.join("Cache");
        std::fs::create_dir_all(&cache_dir).unwrap();
        let file = cache_dir.join("nope.bin");
        let path = file.to_string_lossy().into_owned();
        let result = perform_delete(&[path], |_, _, _| {});
        assert_eq!(result.deleted_count, 0);
        assert_eq!(result.errors.len(), 1);
        assert!(!result.errors[0].contains("拒绝删除"));
        cleanup(&root);
    }

    #[test]
    fn mixed_safe_and_non_safe() {
        let root = tmp("mixed");
        let cache_dir = root.join("Cache");
        std::fs::create_dir_all(&cache_dir).unwrap();
        let safe_file = cache_dir.join("blob.bin");
        std::fs::write(&safe_file, vec![0u8; 512]).unwrap();
        let safe_path = safe_file.to_string_lossy().into_owned();
        let keep_path = r"C:\Program Files\fakeapp\app.exe".to_string();
        let result = perform_delete(&[safe_path, keep_path], |_, _, _| {});
        assert_eq!(result.deleted_count, 1);
        assert_eq!(result.freed_bytes, 512);
        assert_eq!(result.errors.len(), 1);
        assert!(result.errors[0].contains("拒绝删除"));
        assert!(!safe_file.exists());
        cleanup(&root);
    }
}

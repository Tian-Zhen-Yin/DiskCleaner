use crate::models::{CleanCategory, HistoryEntry, HistoryRecord};
use chrono::Local;
use std::path::PathBuf;
use std::sync::Mutex;

pub struct HistoryStore {
    path: PathBuf,
    inner: Mutex<Vec<HistoryRecord>>,
}

impl HistoryStore {
    pub fn new() -> Self {
        let path = history_path();
        let list = load_from_disk(&path).unwrap_or_default();
        Self {
            path,
            inner: Mutex::new(list),
        }
    }

    pub fn list(&self) -> Vec<HistoryRecord> {
        self.inner.lock().unwrap().clone()
    }

    pub fn append(&self, entries: Vec<HistoryEntry>, source: &str) -> Result<(), String> {
        let record = HistoryRecord {
            timestamp: Local::now().to_rfc3339(),
            source: source.to_string(),
            total_freed_bytes: entries.iter().map(|e| e.freed_bytes).sum(),
            total_removed: entries.iter().map(|e| e.removed_count).sum(),
            entries,
        };
        let mut guard = self.inner.lock().unwrap();
        guard.push(record);
        if guard.len() > 500 {
            let overflow = guard.len() - 500;
            guard.drain(0..overflow);
        }
        save_to_disk(&self.path, &guard)
    }

    pub fn clear(&self) -> Result<(), String> {
        let mut guard = self.inner.lock().unwrap();
        guard.clear();
        save_to_disk(&self.path, &guard)
    }
}

pub fn build_entries_from_results(
    results: &[crate::models::CleanResult],
) -> Vec<HistoryEntry> {
    results
        .iter()
        .map(|r| HistoryEntry {
            category: r.category,
            freed_bytes: r.freed_bytes,
            removed_count: r.removed_count,
            error_count: r.errors.len() as u64,
        })
        .collect()
}

#[allow(dead_code)]
pub fn build_entries_from_cli(
    results: &[(CleanCategory, u64, u64, u64)],
) -> Vec<HistoryEntry> {
    results
        .iter()
        .map(|(c, freed, removed, errors)| HistoryEntry {
            category: *c,
            freed_bytes: *freed,
            removed_count: *removed,
            error_count: *errors,
        })
        .collect()
}

fn load_from_disk(path: &PathBuf) -> Option<Vec<HistoryRecord>> {
    let s = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&s).ok()
}

fn save_to_disk(path: &PathBuf, list: &Vec<HistoryRecord>) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_string_pretty(list).map_err(|e| e.to_string())?;
    std::fs::write(path, json).map_err(|e| e.to_string())
}

fn history_path() -> PathBuf {
    let base = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
    base.join("DiskClearTool").join("history.json")
}

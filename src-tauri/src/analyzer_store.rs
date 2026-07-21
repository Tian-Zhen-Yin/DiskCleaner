//! Snapshot store: persists MonitorSnapshot files under a root dir, with a
//! small index.json summary so listing doesn't require parsing every file.
//! Writes are serialized by a Mutex to guard against concurrent scans
//! (manual + future background scheduler).

use crate::models::{MonitorSnapshot, SnapshotSummary};
use std::path::PathBuf;
use std::sync::Mutex;

/// Persistent AnalyzerConfig at %APPDATA%\DiskClearTool\analyzer.json.
pub struct AnalyzerConfigStore {
    path: PathBuf,
    inner: Mutex<crate::models::AnalyzerConfig>,
}

impl AnalyzerConfigStore {
    pub fn new() -> Self {
        let path = Self::default_path();
        let cfg = std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        Self { path, inner: Mutex::new(cfg) }
    }

    pub fn get(&self) -> crate::models::AnalyzerConfig {
        self.inner.lock().unwrap().clone()
    }

    pub fn set(&self, cfg: crate::models::AnalyzerConfig) -> Result<(), String> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let json = serde_json::to_string_pretty(&cfg).map_err(|e| e.to_string())?;
        std::fs::write(&self.path, json).map_err(|e| e.to_string())?;
        *self.inner.lock().unwrap() = cfg;
        Ok(())
    }

    fn default_path() -> PathBuf {
        let base = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
        base.join("DiskClearTool").join("analyzer.json")
    }
}

impl Default for AnalyzerConfigStore {
    fn default() -> Self {
        Self::new()
    }
}

pub struct SnapshotStore {
    root: PathBuf,
    lock: Mutex<()>,
}

impl SnapshotStore {
    pub fn new(root: PathBuf) -> Self {
        let _ = std::fs::create_dir_all(&root);
        Self { root, lock: Mutex::new(()) }
    }

    /// Default location: %APPDATA%\DiskClearTool\snapshots
    pub fn default_root() -> PathBuf {
        let base = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
        base.join("DiskClearTool").join("snapshots")
    }

    /// Persist a snapshot and refresh index.json. Timestamp from `snap.timestamp`.
    pub fn save(&self, snap: &MonitorSnapshot) -> Result<PathBuf, String> {
        let _g = self.lock.lock().unwrap();
        let ts = sanitize_ts(&snap.timestamp);
        let path = self.root.join(format!("{}.json", ts));
        let json = serde_json::to_string_pretty(snap).map_err(|e| e.to_string())?;
        std::fs::write(&path, json).map_err(|e| e.to_string())?;
        self.rewrite_index_locked()?;
        Ok(path)
    }

    /// List snapshot summaries (from index.json), newest last.
    pub fn list(&self) -> Vec<SnapshotSummary> {
        let _g = self.lock.lock().unwrap();
        self.read_index_locked()
    }

    /// Load one snapshot by its timestamp string.
    pub fn load(&self, ts: &str) -> Option<MonitorSnapshot> {
        let path = self.root.join(format!("{}.json", sanitize_ts(ts)));
        let s = std::fs::read_to_string(&path).ok()?;
        serde_json::from_str(&s).ok()
    }

    /// Most recent snapshot, or None.
    pub fn latest(&self) -> Option<MonitorSnapshot> {
        let items = self.list();
        let last = items.last()?;
        self.load(&last.timestamp)
    }

    /// Delete snapshots older than `keep_days`. Keeps at least the newest.
    pub fn prune(&self, keep_days: u32) -> Result<usize, String> {
        let _g = self.lock.lock().unwrap();
        let items = self.read_index_locked();
        if items.len() <= 1 {
            return Ok(0);
        }
        let now = chrono::Local::now();
        let cutoff = now - chrono::Duration::days(keep_days as i64);
        let mut removed = 0usize;
        // Never delete the newest item (last after sort). Iterate indices 0..len-1.
        let last_idx = items.len().saturating_sub(1);
        for (i, item) in items.iter().enumerate() {
            if i == last_idx {
                continue;
            }
            let parsed = chrono::DateTime::parse_from_rfc3339(&item.timestamp);
            let stale = match parsed {
                Ok(t) => t.with_timezone(&chrono::Local) < cutoff,
                Err(_) => false,
            };
            if stale {
                let path = self.root.join(format!("{}.json", sanitize_ts(&item.timestamp)));
                if std::fs::remove_file(&path).is_ok() {
                    removed += 1;
                }
            }
        }
        if removed > 0 {
            self.rewrite_index_locked()?;
        }
        Ok(removed)
    }

    fn snapshot_paths_locked(&self) -> Vec<PathBuf> {
        let mut paths: Vec<PathBuf> = std::fs::read_dir(&self.root)
            .map(|rd| {
                rd.flatten()
                    .filter(|e| {
                        e.path().extension().and_then(|x| x.to_str()) == Some("json")
                            && e.file_name() != "index.json"
                    })
                    .map(|e| e.path())
                    .collect()
            })
            .unwrap_or_default();
        paths.sort();
        paths
    }

    fn rewrite_index_locked(&self) -> Result<(), String> {
        let mut summaries: Vec<SnapshotSummary> = Vec::new();
        for path in self.snapshot_paths_locked() {
            if let Ok(s) = std::fs::read_to_string(&path) {
                if let Ok(snap) = serde_json::from_str::<MonitorSnapshot>(&s) {
                    summaries.push(SnapshotSummary {
                        timestamp: snap.timestamp,
                        scan_type: snap.scan_type,
                        drive_used: snap.drive_used,
                    });
                }
            }
        }
        summaries.sort_by(|a, b| a.timestamp.cmp(&b.timestamp));
        let idx_path = self.root.join("index.json");
        let json = serde_json::to_string_pretty(&summaries).map_err(|e| e.to_string())?;
        std::fs::write(&idx_path, json).map_err(|e| e.to_string())?;
        Ok(())
    }

    fn read_index_locked(&self) -> Vec<SnapshotSummary> {
        let idx_path: PathBuf = self.root.join("index.json");
        match std::fs::read_to_string(&idx_path) {
            Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
            Err(_) => Vec::new(),
        }
    }
}

/// Turn an RFC3339 timestamp into a filesystem-safe filename stem.
fn sanitize_ts(ts: &str) -> String {
    ts.replace([':', '.', '+', '/', '\\', '<', '>', '|', '?', '*'], "_")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use crate::models::{LargeFileEntry, MonitorEntry};

    fn tmp_root(tag: &str) -> PathBuf {
        static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let mut p = std::env::temp_dir();
        p.push(format!("dct_store_{}_{}", tag, n));
        let _ = std::fs::create_dir_all(&p);
        p
    }

    fn snap(ts: &str) -> MonitorSnapshot {
        MonitorSnapshot {
            timestamp: ts.into(),
            scan_type: "full".into(),
            drive_total: 1000,
            drive_used: 500,
            monitor_dirs: vec![MonitorEntry { path: "C:/A".into(), size_bytes: 100, file_count: 1, exists: true }],
            large_files: vec![LargeFileEntry { path: "C:/A/f.bin".into(), size_bytes: 50 }],
        }
    }

    #[test]
    fn save_list_load_roundtrip() {
        let root = tmp_root("roundtrip");
        let store = SnapshotStore::new(root.clone());
        store.save(&snap("2026-07-21T10:00:00+08:00")).unwrap();
        store.save(&snap("2026-07-21T11:00:00+08:00")).unwrap();
        let items = store.list();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].timestamp, "2026-07-21T10:00:00+08:00");
        let loaded = store.load(&items[1].timestamp).unwrap();
        assert_eq!(loaded.drive_used, 500);
    }

    #[test]
    fn latest_returns_newest() {
        let root = tmp_root("latest");
        let store = SnapshotStore::new(root);
        store.save(&snap("2026-07-21T10:00:00+08:00")).unwrap();
        store.save(&snap("2026-07-21T12:00:00+08:00")).unwrap();
        let latest = store.latest().unwrap();
        assert_eq!(latest.timestamp, "2026-07-21T12:00:00+08:00");
    }

    #[test]
    fn prune_keeps_newest_even_if_old() {
        let root = tmp_root("prune");
        let store = SnapshotStore::new(root);
        // Both ancient, but the newest must survive a prune.
        store.save(&snap("2020-01-01T00:00:00+08:00")).unwrap();
        store.save(&snap("2020-02-01T00:00:00+08:00")).unwrap();
        let removed = store.prune(14).unwrap();
        assert!(removed >= 1);
        assert_eq!(store.list().len(), 1);
    }

    #[test]
    fn empty_store_returns_empty_list() {
        let root = tmp_root("empty");
        let store = SnapshotStore::new(root);
        assert!(store.list().is_empty());
        assert!(store.latest().is_none());
    }

    #[test]
    fn concurrent_saves_dont_corrupt_index() {
        // Sequential 100 saves; index must end with 100 entries, sorted.
        let root = tmp_root("seq100");
        let store = std::sync::Arc::new(SnapshotStore::new(root));
        for i in 0..100 {
            let ts = format!("2026-07-21T10:{:02}:00+08:00", i);
            store.save(&snap(&ts)).unwrap();
        }
        let items = store.list();
        assert_eq!(items.len(), 100);
        // Confirm sorted ascending.
        let mut sorted = items.clone();
        sorted.sort_by(|a, b| a.timestamp.cmp(&b.timestamp));
        assert_eq!(items, sorted);
    }

    #[test]
    fn sanitize_ts_is_filesystem_safe() {
        let got = sanitize_ts("2026-07-21T10:00:00+08:00");
        assert!(!got.contains(':'));
        assert!(!got.contains('+'));
        let p = Path::new(&got);
        assert_eq!(p.extension(), None);
    }
}

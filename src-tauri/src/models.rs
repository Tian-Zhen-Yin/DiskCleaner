use serde::{Deserialize, Serialize};

/// Cleanability classification for analyzer results.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum CleanAdvice {
    /// Safe to delete - cache, temp, logs, dumps, etc.
    Safe,
    /// Proceed with caution - may be needed, user should verify.
    Caution,
    /// Keep - system files, program files, user data.
    Keep,
    /// Could not classify, user must judge.
    Unknown,
}

impl Default for CleanAdvice {
    fn default() -> Self {
        CleanAdvice::Unknown
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum CleanCategory {
    WindowsTemp,
    BrowserCache,
    WindowsUpdate,
    RecycleBin,
    Thumbnails,
    ErrorReports,
    MemoryDumps,
    WindowsLogs,
    DeliveryOptimization,
    Prefetch,
    DevCaches,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanResult {
    pub category: CleanCategory,
    pub size_bytes: u64,
    pub file_count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CleanResult {
    pub category: CleanCategory,
    pub freed_bytes: u64,
    pub removed_count: u64,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CleanProgress {
    pub category: CleanCategory,
    pub category_freed_bytes: u64,
    pub total_freed_bytes: u64,
    pub total_removed_count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiskInfo {
    pub total_bytes: u64,
    pub free_bytes: u64,
    pub used_bytes: u64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ScheduleFrequency {
    Daily,
    Weekly,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduleConfig {
    pub enabled: bool,
    pub frequency: ScheduleFrequency,
    pub hour: u32,
    pub minute: u32,
    pub categories: Vec<CleanCategory>,
}

impl Default for ScheduleConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            frequency: ScheduleFrequency::Daily,
            hour: 3,
            minute: 0,
            categories: vec![
                CleanCategory::WindowsTemp,
                CleanCategory::BrowserCache,
                CleanCategory::WindowsUpdate,
            ],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub category: CleanCategory,
    pub freed_bytes: u64,
    pub removed_count: u64,
    pub error_count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryRecord {
    pub timestamp: String,
    pub source: String,
    pub total_freed_bytes: u64,
    pub total_removed: u64,
    pub entries: Vec<HistoryEntry>,
}

// ===== C 盘占用分析器 =====

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonitorEntry {
    pub path: String,
    pub size_bytes: u64,
    pub file_count: u64,
    pub exists: bool,
    #[serde(default)]
    pub advice: CleanAdvice,
    #[serde(default)]
    pub advice_reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LargeFileEntry {
    pub path: String,
    pub size_bytes: u64,
    #[serde(default)]
    pub advice: CleanAdvice,
    #[serde(default)]
    pub advice_reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonitorSnapshot {
    pub timestamp: String,
    pub scan_type: String,
    pub drive_total: u64,
    pub drive_used: u64,
    pub monitor_dirs: Vec<MonitorEntry>,
    pub large_files: Vec<LargeFileEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirDelta {
    pub path: String,
    pub kind: String,
    pub prev_bytes: u64,
    pub curr_bytes: u64,
    pub delta_bytes: i64,
    pub pct: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeleteResult {
    pub deleted_count: u64,
    pub freed_bytes: u64,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct SnapshotSummary {
    pub timestamp: String,
    pub scan_type: String,
    pub drive_used: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalyzerConfig {
    pub monitor_dirs: Vec<String>,
    pub large_file_min_bytes: u64,
    pub large_file_top_n: u32,
    pub snapshot_keep_days: u32,
}

impl Default for AnalyzerConfig {
    fn default() -> Self {
        Self {
            monitor_dirs: default_monitor_dirs(),
            large_file_min_bytes: 500 * 1024 * 1024,
            large_file_top_n: 50,
            snapshot_keep_days: 14,
        }
    }
}

fn default_monitor_dirs() -> Vec<String> {
    let mut out: Vec<String> = vec![
        "C:\\Users".into(),
        "C:\\ProgramData".into(),
        "C:\\Program Files".into(),
        "C:\\Program Files (x86)".into(),
        "C:\\Windows".into(),
        "C:\\Windows\\Installer".into(),
        "C:\\Windows\\SoftwareDistribution".into(),
        "C:\\Windows\\Temp".into(),
    ];
    let local = std::env::var("LOCALAPPDATA").ok();
    let appdata = std::env::var("APPDATA").ok();
    let profile = std::env::var("USERPROFILE").ok();
    if let Some(l) = local {
        out.push(l.clone());
        out.push(format!("{}\\Docker", l));
        out.push(format!("{}\\Packages", l));
        out.push(format!("{}\\Programs", l));
    }
    if let Some(a) = appdata {
        out.push(a);
    }
    if let Some(p) = profile {
        out.push(format!("{}\\Downloads", p));
        out.push(format!("{}\\Documents", p));
    }
    out
}

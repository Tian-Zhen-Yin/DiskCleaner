use serde::{Deserialize, Serialize};

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

// Shared types + helpers between cleanup tab and analyzer tab.

export type CleanCategory =
  | "WindowsTemp"
  | "BrowserCache"
  | "WindowsUpdate"
  | "RecycleBin"
  | "Thumbnails"
  | "ErrorReports"
  | "MemoryDumps"
  | "WindowsLogs"
  | "DeliveryOptimization"
  | "Prefetch"
  | "DevCaches";

// ===== Analyzer types (mirror Rust models.rs) =====

export interface MonitorEntry {
  path: string;
  size_bytes: number;
  file_count: number;
  exists: boolean;
}

export interface LargeFileEntry {
  path: string;
  size_bytes: number;
}

export interface MonitorSnapshot {
  timestamp: string;
  scan_type: string;
  drive_total: number;
  drive_used: number;
  monitor_dirs: MonitorEntry[];
  large_files: LargeFileEntry[];
}

export interface DirDelta {
  path: string;
  kind: "added" | "removed" | "changed" | "moved";
  prev_bytes: number;
  curr_bytes: number;
  delta_bytes: number;
  pct: number;
}

export interface SnapshotSummary {
  timestamp: string;
  scan_type: string;
  drive_used: number;
}

export interface AnalyzerConfig {
  monitor_dirs: string[];
  large_file_min_bytes: number;
  large_file_top_n: number;
  snapshot_keep_days: number;
}

export function formatBytes(bytes: number): string {
  if (bytes <= 0) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB"];
  const i = Math.floor(Math.log(bytes) / Math.log(1024));
  return `${(bytes / Math.pow(1024, i)).toFixed(2)} ${units[i]}`;
}

const SYSTEM_FILES = ["pagefile.sys", "hiberfil.sys", "swapfile.sys"];

export function isSystemFile(path: string): boolean {
  const name = path.split("\\").pop()?.toLowerCase() ?? "";
  return SYSTEM_FILES.includes(name);
}

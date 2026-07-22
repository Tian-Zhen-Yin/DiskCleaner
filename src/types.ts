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

export type CleanAdvice = "Safe" | "Caution" | "Keep" | "Unknown";

export interface AdviceMeta {
  label: string;
  color: string;
  bg: string;
}

export const ADVICE_META: Record<CleanAdvice, AdviceMeta> = {
  Safe: { label: "安全可删", color: "#a6e3a1", bg: "rgba(166,227,161,0.12)" },
  Caution: { label: "谨慎处理", color: "#f9e2af", bg: "rgba(249,226,175,0.12)" },
  Keep: { label: "不建议删除", color: "#f38ba8", bg: "rgba(243,139,168,0.12)" },
  Unknown: { label: "未知", color: "#6c7086", bg: "rgba(108,112,134,0.12)" },
};

export interface MonitorEntry {
  path: string;
  size_bytes: number;
  file_count: number;
  exists: boolean;
  advice: CleanAdvice;
  advice_reason: string;
}

export interface LargeFileEntry {
  path: string;
  size_bytes: number;
  advice: CleanAdvice;
  advice_reason: string;
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

export interface DeleteResult {
  deleted_count: number;
  freed_bytes: number;
  errors: string[];
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

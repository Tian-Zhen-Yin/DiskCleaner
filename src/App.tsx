import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import AnalyzerTab from "./AnalyzerTab";

type CleanCategory =
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

interface ScanResult {
  category: CleanCategory;
  size_bytes: number;
  file_count: number;
}

interface CleanResult {
  category: CleanCategory;
  freed_bytes: number;
  removed_count: number;
  errors: string[];
}

interface DiskInfo {
  total_bytes: number;
  free_bytes: number;
  used_bytes: number;
}

interface ScheduleConfig {
  enabled: boolean;
  frequency: "Daily" | "Weekly";
  hour: number;
  minute: number;
  categories: CleanCategory[];
}

interface HistoryEntry {
  category: CleanCategory;
  freed_bytes: number;
  removed_count: number;
  error_count: number;
}

interface HistoryRecord {
  timestamp: string;
  source: string;
  total_freed_bytes: number;
  total_removed: number;
  entries: HistoryEntry[];
}

type LogLevel = "info" | "success" | "error";
interface LogEntry {
  time: string;
  level: LogLevel;
  message: string;
}

const CATEGORY_META: Record<CleanCategory, { title: string; desc: string; color: string }> = {
  WindowsTemp: {
    title: "Windows Temp 临时文件",
    desc: "%TEMP% 与 C:\\Windows\\Temp",
    color: "#89b4fa",
  },
  BrowserCache: {
    title: "浏览器缓存",
    desc: "Edge / Chrome 缓存目录",
    color: "#f9e2af",
  },
  WindowsUpdate: {
    title: "Windows Update 缓存与日志",
    desc: "SoftwareDistribution\\Download、CBS 日志等",
    color: "#a6e3a1",
  },
  RecycleBin: {
    title: "回收站（C 盘）",
    desc: "C:\\$Recycle.Bin，清空后不可恢复",
    color: "#f38ba8",
  },
  Thumbnails: {
    title: "缩略图与图标缓存",
    desc: "thumbcache_*.db、iconcache_*.db",
    color: "#f5c2e7",
  },
  ErrorReports: {
    title: "Windows 错误报告 WER",
    desc: "ProgramData 与用户目录下的 WER 队列",
    color: "#eba0ac",
  },
  MemoryDumps: {
    title: "内存转储",
    desc: "Memory.dmp、Minidump、LiveKernelReports",
    color: "#fab387",
  },
  WindowsLogs: {
    title: "Windows 日志",
    desc: "Panther、setupapi 日志、Logs 子目录",
    color: "#94e2d5",
  },
  DeliveryOptimization: {
    title: "传递优化缓存",
    desc: "SoftwareDistribution\\DeliveryOptimization",
    color: "#cba6f7",
  },
  Prefetch: {
    title: "预读取 Prefetch",
    desc: "Windows\\Prefetch，系统自动重建",
    color: "#89dceb",
  },
  DevCaches: {
    title: "开发者缓存",
    desc: "npm / pip / cargo / NuGet 缓存",
    color: "#b4befe",
  },
};

const ALL_CATEGORIES: CleanCategory[] = [
  "WindowsTemp",
  "BrowserCache",
  "WindowsUpdate",
  "RecycleBin",
  "Thumbnails",
  "ErrorReports",
  "MemoryDumps",
  "WindowsLogs",
  "DeliveryOptimization",
  "Prefetch",
  "DevCaches",
];

// Default interactive selection. RecycleBin (destructive, non-undoable) and
// DevCaches (slow to rebuild) are opt-in.
const DEFAULT_SELECTED: CleanCategory[] = [
  "WindowsTemp",
  "BrowserCache",
  "WindowsUpdate",
  "Thumbnails",
  "ErrorReports",
  "MemoryDumps",
  "WindowsLogs",
  "DeliveryOptimization",
  "Prefetch",
];

// Scheduled / system tasks keep the original conservative default unless the
// user explicitly opts new categories into the schedule.
const SCHEDULE_DEFAULT_CATEGORIES: CleanCategory[] = [
  "WindowsTemp",
  "BrowserCache",
  "WindowsUpdate",
];

const emptyScan = (): Record<CleanCategory, ScanResult | null> =>
  Object.fromEntries(ALL_CATEGORIES.map((c) => [c, null])) as Record<
    CleanCategory,
    ScanResult | null
  >;

function formatBytes(bytes: number): string {
  if (bytes <= 0) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB"];
  const i = Math.floor(Math.log(bytes) / Math.log(1024));
  return `${(bytes / Math.pow(1024, i)).toFixed(2)} ${units[i]}`;
}

function sourceLabel(s: string): string {
  if (s === "manual") return "手动";
  if (s === "scheduler") return "应用定时";
  if (s === "headless") return "系统/自启";
  return s;
}

export default function App() {
  const [diskInfo, setDiskInfo] = useState<DiskInfo | null>(null);
  const [tab, setTab] = useState<"cleanup" | "analyzer">("cleanup");
  const [scanResults, setScanResults] = useState<Record<CleanCategory, ScanResult | null>>(
    emptyScan
  );
  const [selected, setSelected] = useState<Set<CleanCategory>>(
    new Set(DEFAULT_SELECTED)
  );
  const [scanning, setScanning] = useState(false);
  const [cleaning, setCleaning] = useState(false);
  const [logs, setLogs] = useState<LogEntry[]>([]);
  const [schedule, setSchedule] = useState<ScheduleConfig>({
    enabled: false,
    frequency: "Daily",
    hour: 3,
    minute: 0,
    categories: [...SCHEDULE_DEFAULT_CATEGORIES],
  });
  const [autostart, setAutostart] = useState(false);
  const [autostartSilent, setAutostartSilent] = useState(true);
  const [history, setHistory] = useState<HistoryRecord[]>([]);

  const totalSelectedSize = useMemo(() => {
    let sum = 0;
    selected.forEach((c) => {
      const r = scanResults[c];
      if (r) sum += r.size_bytes;
    });
    return sum;
  }, [selected, scanResults]);

  const log = (level: LogLevel, message: string) => {
    const time = new Date().toLocaleTimeString();
    setLogs((prev) => [...prev.slice(-200), { time, level, message }]);
  };

  const refreshDisk = async () => {
    try {
      const info = await invoke<DiskInfo>("get_disk_info", { drive: "C:" });
      setDiskInfo(info);
    } catch (e) {
      log("error", `获取磁盘信息失败: ${e}`);
    }
  };

  const loadSchedule = async () => {
    try {
      const cfg = await invoke<ScheduleConfig>("get_schedule");
      setSchedule(cfg);
    } catch (e) {
      log("error", `加载定时配置失败: ${e}`);
    }
  };

  const loadAutostart = async () => {
    try {
      const enabled = await invoke<boolean>("get_autostart");
      setAutostart(enabled);
    } catch (e) {
      log("error", `加载开机自启状态失败: ${e}`);
    }
  };

  const loadHistory = async () => {
    try {
      const list = await invoke<HistoryRecord[]>("get_history");
      setHistory(list);
    } catch (e) {
      log("error", `加载历史记录失败: ${e}`);
    }
  };

  useEffect(() => {
    refreshDisk();
    loadSchedule();
    loadAutostart();
    loadHistory();
    log("info", "应用已启动，建议先点击「扫描」查看可清理空间");
  }, []);

  const handleScan = async () => {
    setScanning(true);
    log("info", "开始扫描...");
    try {
      const results = await invoke<ScanResult[]>("scan_all", {
        categories: ALL_CATEGORIES,
      });
      const map = emptyScan();
      results.forEach((r) => {
        map[r.category] = r;
      });
      setScanResults(map);
      log("success", `扫描完成，共发现可清理 ${formatBytes(
        results.reduce((s, r) => s + r.size_bytes, 0)
      )}`);
    } catch (e) {
      log("error", `扫描失败: ${e}`);
    } finally {
      setScanning(false);
    }
  };

  const handleClean = async () => {
    if (selected.size === 0) {
      log("error", "请至少选择一项要清理的内容");
      return;
    }
    setCleaning(true);
    log("info", `开始清理 ${selected.size} 项...`);
    try {
      const results = await invoke<CleanResult[]>("clean_categories", {
        categories: Array.from(selected),
      });
      let totalFreed = 0;
      results.forEach((r) => {
        totalFreed += r.freed_bytes;
        log(
          r.errors.length === 0 ? "success" : "error",
          `${CATEGORY_META[r.category].title}: 释放 ${formatBytes(
            r.freed_bytes
          )}，删除 ${r.removed_count} 个文件${
            r.errors.length > 0 ? `，${r.errors.length} 个错误` : ""
          }`
        );
        r.errors.slice(0, 5).forEach((err) => log("error", `  · ${err}`));
      });
      log("success", `清理完成，共释放 ${formatBytes(totalFreed)}`);
      await handleScan();
      await refreshDisk();
      await loadHistory();
    } catch (e) {
      log("error", `清理失败: ${e}`);
    } finally {
      setCleaning(false);
    }
  };

  const toggleSelected = (c: CleanCategory) => {
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(c)) next.delete(c);
      else next.add(c);
      return next;
    });
  };

  const applySchedule = async () => {
    try {
      await invoke("set_schedule", { config: schedule });
      log(
        "success",
        schedule.enabled
          ? `定时任务已启用（${schedule.frequency === "Daily" ? "每日" : "每周"} ${String(schedule.hour).padStart(2, "0")}:${String(schedule.minute).padStart(2, "0")}）`
          : "定时任务已停用"
      );
    } catch (e) {
      log("error", `保存定时配置失败: ${e}`);
    }
  };

  const registerSystemTask = async () => {
    try {
      const msg = await invoke<string>("register_system_task", {
        config: schedule,
      });
      log("success", msg);
    } catch (e) {
      log("error", `注册系统计划任务失败: ${e}`);
    }
  };

  const unregisterSystemTask = async () => {
    try {
      const msg = await invoke<string>("unregister_system_task");
      log("success", msg);
    } catch (e) {
      log("error", `卸载系统计划任务失败: ${e}`);
    }
  };

  const toggleAutostart = async (next: boolean) => {
    try {
      const msg = await invoke<string>("set_autostart", {
        enabled: next,
        silent: autostartSilent,
      });
      setAutostart(next);
      log("success", msg);
    } catch (e) {
      log("error", `设置开机自启失败: ${e}`);
    }
  };

  const reapplyAutostart = async () => {
    if (!autostart) return;
    try {
      const msg = await invoke<string>("set_autostart", {
        enabled: true,
        silent: autostartSilent,
      });
      log("success", `已更新自启模式：${msg}`);
    } catch (e) {
      log("error", `更新自启模式失败: ${e}`);
    }
  };

  const clearHistory = async () => {
    try {
      await invoke("clear_history");
      setHistory([]);
      log("success", "历史记录已清空");
    } catch (e) {
      log("error", `清空历史失败: ${e}`);
    }
  };

  const usagePercent =
    diskInfo && diskInfo.total_bytes > 0
      ? (diskInfo.used_bytes / diskInfo.total_bytes) * 100
      : 0;

  const historyStats = useMemo(() => {
    const totalFreed = history.reduce((s, r) => s + r.total_freed_bytes, 0);
    const totalRemoved = history.reduce((s, r) => s + r.total_removed, 0);
    const byCategory = Object.fromEntries(
      ALL_CATEGORIES.map((c) => [c, 0])
    ) as Record<CleanCategory, number>;
    history.forEach((r) =>
      r.entries.forEach((e) => {
        byCategory[e.category] += e.freed_bytes;
      })
    );
    return { totalFreed, totalRemoved, byCategory };
  }, [history]);

  const recent = useMemo(() => history.slice(-14), [history]);
  const maxFreedInRecent = useMemo(
    () => recent.reduce((m, r) => Math.max(m, r.total_freed_bytes), 0),
    [recent]
  );

  return (
    <div className="app">
      <div className="header">
        <div className="app-bar">
          <div className="brand-logo">🧹</div>
          <div className="brand-text">
            <h1>Windows C 盘清理工具</h1>
            <span className="brand-sub">扫描 · 清理 · 占用分析</span>
          </div>
        </div>
        <button className="secondary" onClick={refreshDisk}>
          刷新磁盘信息
        </button>
      </div>

      <div className="tabs">
        <button className={tab === "cleanup" ? "tab active" : "tab"} onClick={() => setTab("cleanup")}>清理</button>
        <button className={tab === "analyzer" ? "tab active" : "tab"} onClick={() => setTab("analyzer")}>占用分析</button>
      </div>

      {tab === "analyzer" && <AnalyzerTab />}

      {tab === "cleanup" && (
        <>
      <div className="disk-info">
        <div className="disk-drive">C: 盘</div>
        <div
          className="disk-bar"
          data-level={
            usagePercent >= 90 ? "high" : usagePercent >= 70 ? "mid" : "low"
          }
        >
          <div
            className="disk-bar-fill"
            style={{ width: `${usagePercent}%` }}
          />
        </div>
        <div className="disk-pct">{usagePercent.toFixed(1)}%</div>
        <div className="disk-text">
          {diskInfo
            ? `${formatBytes(diskInfo.used_bytes)} / ${formatBytes(
                diskInfo.total_bytes
              )}（剩余 ${formatBytes(diskInfo.free_bytes)}）`
            : "加载中..."}
        </div>
      </div>

      <div className="section">
        <h2>清理项</h2>
        <div className="items">
          {ALL_CATEGORIES.map((c) => {
            const r = scanResults[c];
            return (
              <label
                key={c}
                className="item"
                data-active={selected.has(c)}
              >
                <input
                  type="checkbox"
                  checked={selected.has(c)}
                  onChange={() => toggleSelected(c)}
                />
                <div className="item-info">
                  <div className="item-title">
                    <span
                      className="dot"
                      style={{ background: CATEGORY_META[c].color }}
                    />
                    {CATEGORY_META[c].title}
                  </div>
                  <div className="item-desc">{CATEGORY_META[c].desc}</div>
                </div>
                <div className="item-size">
                  {r ? formatBytes(r.size_bytes) : "—"}
                </div>
              </label>
            );
          })}
        </div>
        <div className="summary">
          已选合计：{formatBytes(totalSelectedSize)}
        </div>
        <div className="actions" style={{ marginTop: 14 }}>
          <button onClick={handleScan} disabled={scanning || cleaning}>
            {scanning ? "扫描中..." : "扫描"}
          </button>
          <button
            className="danger"
            onClick={handleClean}
            disabled={scanning || cleaning || selected.size === 0}
          >
            {cleaning ? "清理中..." : "立即清理"}
          </button>
        </div>
      </div>

      <div className="section">
        <h2>定时清理 & 开机自启</h2>
        <div className="schedule">
          <label>
            <input
              type="checkbox"
              checked={schedule.enabled}
              onChange={(e) =>
                setSchedule({ ...schedule, enabled: e.target.checked })
              }
            />{" "}
            启用定时
          </label>
          <label>
            频率：
            <select
              value={schedule.frequency}
              onChange={(e) =>
                setSchedule({
                  ...schedule,
                  frequency: e.target.value as "Daily" | "Weekly",
                })
              }
            >
              <option value="Daily">每日</option>
              <option value="Weekly">每周</option>
            </select>
          </label>
          <label>
            时间：
            <input
              type="number"
              min={0}
              max={23}
              value={schedule.hour}
              style={{ width: 50 }}
              onChange={(e) =>
                setSchedule({
                  ...schedule,
                  hour: Math.max(0, Math.min(23, Number(e.target.value))),
                })
              }
            />
            :
            <input
              type="number"
              min={0}
              max={59}
              value={schedule.minute}
              style={{ width: 50 }}
              onChange={(e) =>
                setSchedule({
                  ...schedule,
                  minute: Math.max(0, Math.min(59, Number(e.target.value))),
                })
              }
            />
          </label>
        </div>
        <div className="actions" style={{ marginTop: 14 }}>
          <button onClick={applySchedule}>保存(应用内定时)</button>
          <button className="secondary" onClick={registerSystemTask}>
            注册到系统计划任务
          </button>
          <button className="secondary" onClick={unregisterSystemTask}>
            移除系统计划任务
          </button>
        </div>

        <div className="divider" />

        <div className="schedule">
          <label>
            <input
              type="checkbox"
              checked={autostart}
              onChange={(e) => toggleAutostart(e.target.checked)}
            />{" "}
            开机自启
          </label>
          <label>
            <input
              type="checkbox"
              checked={autostartSilent}
              onChange={(e) => setAutostartSilent(e.target.checked)}
            />{" "}
            静默模式（开机直接执行清理，不显示界面）
          </label>
          <button className="secondary" onClick={reapplyAutostart} disabled={!autostart}>
            应用自启模式
          </button>
        </div>
      </div>

      <div className="section">
        <h2>清理历史统计</h2>
        <div className="stats">
          <div className="stat-card">
            <div className="stat-label">累计释放</div>
            <div className="stat-value">{formatBytes(historyStats.totalFreed)}</div>
          </div>
          <div className="stat-card">
            <div className="stat-label">累计文件数</div>
            <div className="stat-value">{historyStats.totalRemoved.toLocaleString()}</div>
          </div>
          <div className="stat-card">
            <div className="stat-label">清理次数</div>
            <div className="stat-value">{history.length}</div>
          </div>
          {ALL_CATEGORIES.map((c) => (
            <div className="stat-card" key={c}>
              <div className="stat-label">
                <span className="dot" style={{ background: CATEGORY_META[c].color }} />
                {CATEGORY_META[c].title}
              </div>
              <div className="stat-value small">
                {formatBytes(historyStats.byCategory[c])}
              </div>
            </div>
          ))}
        </div>

        <h3 className="sub-title">最近清理趋势（最近 {recent.length} 次）</h3>
        {recent.length === 0 ? (
          <div className="empty">暂无历史记录</div>
        ) : (
          <div className="chart">
            {recent.map((r, i) => {
              const heightPct =
                maxFreedInRecent > 0
                  ? (r.total_freed_bytes / maxFreedInRecent) * 100
                  : 0;
              const stack = ALL_CATEGORIES.map((c) => {
                const e = r.entries.find((x) => x.category === c);
                return {
                  category: c,
                  bytes: e ? e.freed_bytes : 0,
                };
              });
              const totalStack = stack.reduce((s, x) => s + x.bytes, 0) || 1;
              return (
                <div className="bar-wrap" key={i} title={`${new Date(r.timestamp).toLocaleString()} - ${formatBytes(r.total_freed_bytes)}`}>
                  <div
                    className="bar"
                    style={{ height: `${Math.max(heightPct, 2)}%` }}
                  >
                    {stack.map((s) => (
                      <div
                        key={s.category}
                        style={{
                          flex: s.bytes / totalStack,
                          background: CATEGORY_META[s.category].color,
                        }}
                      />
                    ))}
                  </div>
                  <div className="bar-label">
                    {new Date(r.timestamp).toLocaleDateString(undefined, {
                      month: "2-digit",
                      day: "2-digit",
                    })}
                  </div>
                </div>
              );
            })}
          </div>
        )}

        <h3 className="sub-title">历史明细</h3>
        {history.length === 0 ? (
          <div className="empty">暂无记录</div>
        ) : (
          <div className="history-list">
            {[...history].reverse().slice(0, 20).map((r, i) => (
              <div key={i} className="history-item">
                <div className="history-time">
                  {new Date(r.timestamp).toLocaleString()}
                  <span className="badge">{sourceLabel(r.source)}</span>
                </div>
                <div className="history-main">
                  释放 <b>{formatBytes(r.total_freed_bytes)}</b>，删除{" "}
                  {r.total_removed.toLocaleString()} 个文件
                </div>
                <div className="history-detail">
                  {r.entries.map((e) => (
                    <span key={e.category} className="chip">
                      <span
                        className="dot"
                        style={{ background: CATEGORY_META[e.category].color }}
                      />
                      {CATEGORY_META[e.category].title.split(" ")[0]}{" "}
                      {formatBytes(e.freed_bytes)}
                    </span>
                  ))}
                </div>
              </div>
            ))}
          </div>
        )}

        <div className="actions" style={{ marginTop: 12 }}>
          <button className="secondary" onClick={loadHistory}>
            刷新历史
          </button>
          <button className="danger" onClick={clearHistory} disabled={history.length === 0}>
            清空历史
          </button>
        </div>
      </div>

      <div className="section" style={{ display: "flex", flexDirection: "column", minHeight: 0 }}>
        <h2>运行日志</h2>
        <div className="logs">
          {logs.length === 0 && <div>暂无日志</div>}
          {logs.map((l, i) => (
            <div key={i} className={`log-${l.level}`}>
              [{l.time}] {l.message}
            </div>
          ))}
        </div>
      </div>
        </>
      )}
    </div>
  );
}

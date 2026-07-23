import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { AnalyzerConfig, DeleteResult, DirDelta, MonitorEntry, MonitorSnapshot, SnapshotSummary, formatBytes, isSystemFile } from "./types";
import { ADVICE_META, CleanAdvice } from "./types";

export default function AnalyzerTab() {
  const [latest, setLatest] = useState<MonitorSnapshot | null>(null);
  const [snapshots, setSnapshots] = useState<SnapshotSummary[]>([]);
  const [scanning, setScanning] = useState(false);
  const [progress, setProgress] = useState<{ done: number; total: number } | null>(null);
  const [drilldown, setDrilldown] = useState<Record<string, MonitorEntry[]>>({});
  const [diffPrev, setDiffPrev] = useState<string>("");
  const [diffCurr, setDiffCurr] = useState<string>("");
  const [deltas, setDeltas] = useState<DirDelta[] | null>(null);
  const [config, setConfig] = useState<AnalyzerConfig | null>(null);
  const [newDir, setNewDir] = useState("");
  const [expanded, setExpanded] = useState<Set<string>>(new Set());
  const [log, setLog] = useState<string[]>([]);
  const [deleting, setDeleting] = useState(false);
  const [deleteProgress, setDeleteProgress] = useState<{ deleted: number; total: number; freed_bytes: number } | null>(null);
  const [deletedPaths, setDeletedPaths] = useState<Set<string>>(new Set());

  const pushLog = (m: string) => setLog((p) => [...p.slice(-50), m]);

  useEffect(() => {
    invoke<MonitorSnapshot | null>("analyze_get_latest").then(setLatest).catch((e) => pushLog(`加载最新快照失败: ${e}`));
    invoke<SnapshotSummary[]>("analyze_list_snapshots").then(setSnapshots).catch((e) => pushLog(`加载快照列表失败: ${e}`));
    invoke<AnalyzerConfig>("analyze_get_config").then(setConfig).catch((e) => pushLog(`加载配置失败: ${e}`));
    const un = listen<{ done: number; total: number }>("analyze-scan-progress", (e) => setProgress(e.payload));
    const unDel = listen<{ deleted: number; total: number; freed_bytes: number }>("analyze-delete-progress", (e) => setDeleteProgress(e.payload));
    return () => { void un.then((fn) => fn()); void unDel.then((fn) => fn()); };
  }, []);

  const fullScan = async () => {
    setScanning(true); setProgress(null);
    try {
      const snap = await invoke<MonitorSnapshot>("analyze_full_scan");
      setLatest(snap);
      setSnapshots(await invoke<SnapshotSummary[]>("analyze_list_snapshots"));
      setDiffCurr(snap.timestamp);
      pushLog(`全盘扫描完成，监控目录 ${snap.monitor_dirs.length}，大文件 ${snap.large_files.length}`);
    } catch (e) { pushLog(`扫描失败: ${e}`); }
    finally { setScanning(false); setProgress(null); }
  };

  const rescan = async () => {
    try {
      const entries = await invoke<MonitorEntry[]>("analyze_rescan_monitors");
      if (latest) {
        const updated = { ...latest, monitor_dirs: entries, timestamp: new Date().toISOString(), scan_type: "monitor" };
        setLatest(updated);
      }
      pushLog(`监控目录重扫完成，${entries.length} 项`);
    } catch (e) { pushLog(`重扫失败: ${e}`); }
  };

  const toggle = async (path: string) => {
    const next = new Set(expanded);
    if (next.has(path)) { next.delete(path); setExpanded(next); return; }
    next.add(path); setExpanded(next);
    if (!drilldown[path]) {
      try {
        const kids = await invoke<MonitorEntry[]>("analyze_drilldown", { path });
        setDrilldown((d) => ({ ...d, [path]: kids }));
      } catch (e) { pushLog(`下钻失败 ${path}: ${e}`); }
    }
  };

  const runDiff = async () => {
    if (!diffPrev || !diffCurr) return;
    try { setDeltas(await invoke<DirDelta[]>("analyze_diff", { prevTs: diffPrev, currTs: diffCurr })); }
    catch (e) { pushLog(`diff 失败: ${e}`); }
  };

  const deltaFor = (path: string): DirDelta | undefined => deltas?.find((d) => d.path === path && d.kind !== "moved");

  const saveConfig = async () => {
    if (!config) return;
    try { await invoke("analyze_set_config", { config }); pushLog("配置已保存"); }
    catch (e) { pushLog(`保存配置失败: ${e}`); }
  };

  const addMonitorDir = () => {
    if (!config || !newDir.trim()) return;
    if (!config.monitor_dirs.includes(newDir.trim())) {
      setConfig({ ...config, monitor_dirs: [...config.monitor_dirs, newDir.trim()] });
    }
    setNewDir("");
  };

  const removedDirs = deltas?.filter((d) => d.kind === "removed") ?? [];
  const addedDirs = deltas?.filter((d) => d.kind === "added") ?? [];
  const movedDirs = deltas?.filter((d) => d.kind === "moved") ?? [];
  const usagePct = latest && latest.drive_total > 0 ? (latest.drive_used / latest.drive_total) * 100 : 0;

  const visibleLargeFiles = latest?.large_files.filter((f) => !deletedPaths.has(f.path)) ?? [];
  const safeFiles = visibleLargeFiles.filter((f) => f.advice === "Safe");
  const safeBytes = safeFiles.reduce((s, f) => s + f.size_bytes, 0);
  const cautionFiles = visibleLargeFiles.filter((f) => f.advice === "Caution");
  const cautionBytes = cautionFiles.reduce((s, f) => s + f.size_bytes, 0);

  const deleteOne = async (path: string) => {
    setDeleting(true);
    try {
      const res = await invoke<DeleteResult>("analyze_delete_files", { paths: [path] });
      if (res.deleted_count > 0) {
        setDeletedPaths((prev) => new Set(prev).add(path));
        pushLog(`已删除 1 个文件，释放 ${formatBytes(res.freed_bytes)}`);
      }
      res.errors.forEach((e) => pushLog(e));
    } catch (e) { pushLog(`删除失败: ${e}`); }
    finally { setDeleting(false); setDeleteProgress(null); }
  };

  const deleteAllSafe = async () => {
    if (safeFiles.length === 0) return;
    if (!window.confirm(`确定删除 ${safeFiles.length} 个安全可删文件，共 ${formatBytes(safeBytes)}？此操作不可撤销。`)) return;
    const paths = safeFiles.map((f) => f.path);
    setDeleting(true);
    setDeleteProgress({ deleted: 0, total: paths.length, freed_bytes: 0 });
    try {
      const res = await invoke<DeleteResult>("analyze_delete_files", { paths });
      const failed = new Set(paths.filter((p) => res.errors.some((e) => e.includes(p))));
      const deleted = paths.filter((p) => !failed.has(p));
      if (deleted.length > 0) {
        setDeletedPaths((prev) => {
          const next = new Set(prev);
          deleted.forEach((p) => next.add(p));
          return next;
        });
      }
      res.errors.forEach((e) => pushLog(e));
      pushLog(`批量删除完成：成功 ${res.deleted_count} 个，释放 ${formatBytes(res.freed_bytes)}${res.errors.length > 0 ? `，失败 ${res.errors.length} 个` : ""}`);
    } catch (e) { pushLog(`批量删除失败: ${e}`); }
    finally { setDeleting(false); setDeleteProgress(null); }
  };


function AdviceBadge({ advice, reason }: { advice: CleanAdvice; reason: string }) {
  const meta = ADVICE_META[advice];
  return (
    <span
      title={reason}
      style={{
        display: "inline-block",
        padding: "1px 6px",
        borderRadius: 4,
        fontSize: 11,
        color: meta.color,
        background: meta.bg,
        whiteSpace: "nowrap",
        flexShrink: 0,
      }}
    >
      {meta.label}
    </span>
  );
}

  return (
    <div className="section">
      <div className="header" style={{ justifyContent: "space-between" }}>
        <h2>占用分析</h2>
        <div>
          <button onClick={fullScan} disabled={scanning || deleting}>{scanning ? "扫描中..." : "全盘扫描"}</button>
          <button className="secondary" onClick={rescan} disabled={scanning || deleting} style={{ marginLeft: 8 }}>重扫监控目录</button>
        </div>
      </div>
      {progress && <div className="item-desc">扫描进度：{progress.done}/{progress.total} 目录</div>}

      {latest && (
        <div className="disk-info">
          <div className="disk-drive">C: 盘</div>
          <div
            className="disk-bar"
            data-level={usagePct >= 90 ? "high" : usagePct >= 70 ? "mid" : "low"}
          ><div className="disk-bar-fill" style={{ width: `${usagePct}%` }} /></div>
          <div className="disk-pct">{usagePct.toFixed(1)}%</div>
          <div className="disk-text">
            {formatBytes(latest.drive_used)} / {formatBytes(latest.drive_total)}（扫描于 {new Date(latest.timestamp).toLocaleString()}，{latest.scan_type === "full" ? "全盘" : "仅监控目录"}）
          </div>
        </div>
      )}

      <div className="section">
        <h3 className="sub-title">监控目录（{latest?.monitor_dirs.length ?? 0}）</h3>
        {!latest?.monitor_dirs.length && <div className="empty">暂无数据，点击「全盘扫描」</div>}
        {latest?.monitor_dirs.map((m) => {
          const d = deltaFor(m.path);
          const isOpen = expanded.has(m.path);
          return (
            <div key={m.path}>
              <div className="item" onClick={() => toggle(m.path)} style={{ cursor: "pointer" }}>
                <span className="dot" style={{ background: m.exists ? "#89b4fa" : "#6c7086" }} />
                <div className="item-info">
                  <div className="item-title">
                    <span className="path-truncate" title={m.path}>{m.path}</span>
                    {!m.exists && <span style={{ flexShrink: 0, color: "#6c7086" }}>(不存在)</span>}
                  </div>
                  <div className="item-desc">{m.file_count.toLocaleString()} 文件</div>
                </div>
                <div className="item-size" style={{ display: "flex", alignItems: "center", gap: 12 }}>
                  {d && (
                    <span style={{ color: d.delta_bytes >= 0 ? "#f38ba8" : "#a6e3a1", fontSize: 12 }}>
                      {d.delta_bytes >= 0 ? "+" : ""}{formatBytes(d.delta_bytes)}
                    </span>
                  )}
                  <span>{formatBytes(m.size_bytes)}</span>
                </div>
              </div>
              {isOpen && drilldown[m.path] && (
                <div style={{ marginLeft: 20, borderLeft: "2px solid #45475a", paddingLeft: 8 }}>
                  {drilldown[m.path].map((k) => (
                    <div key={k.path} className="item">
                      <span className="dot" style={{ background: ADVICE_META[k.advice ?? "Unknown"].color }} />
                      <div className="item-info">
                        <div className="item-title">
                          <span className="path-truncate" title={k.path}>{k.path.split("\\").pop()}</span>
                        </div>
                        {k.advice_reason && (
                          <div className="item-desc" style={{ display: "flex", gap: 6, alignItems: "center" }}>
                            <AdviceBadge advice={k.advice ?? "Unknown"} reason={k.advice_reason} />
                          </div>
                        )}
                      </div>
                      <div className="item-size">{formatBytes(k.size_bytes)}</div>
                    </div>
                  ))}
                </div>
              )}
            </div>
          );
        })}
      </div>

      <div className="section">
        <h3 className="sub-title">大文件 Top {visibleLargeFiles.length}</h3>
        {latest && latest.large_files.length > 0 && (
          <div style={{ display: "flex", gap: 16, marginBottom: 8, fontSize: 12, alignItems: "center", flexWrap: "wrap" }}>
            <span style={{ color: "#a6e3a1" }}>
              ✓ 安全可删 {safeFiles.length} 个 ({formatBytes(safeBytes)})
            </span>
            <span style={{ color: "#f9e2af" }}>
              ⚠ 谨慎处理 {cautionFiles.length} 个 ({formatBytes(cautionBytes)})
            </span>
            {safeFiles.length > 0 && (
              <button className="secondary" onClick={deleteAllSafe} disabled={deleting} style={{ marginLeft: "auto", color: "#f38ba8", borderColor: "#f38ba8" }}>
                {deleting && deleteProgress ? `删除中 ${deleteProgress.deleted}/${deleteProgress.total}` : "一键清理全部安全可删"}
              </button>
            )}
          </div>
        )}
        {!visibleLargeFiles.length && <div className="empty">暂无</div>}
        {visibleLargeFiles.slice(0, 50).map((f) => (
          <div key={f.path} className="item" style={{ opacity: isSystemFile(f.path) ? 0.5 : 1 }}>
            <span className="dot" style={{ background: ADVICE_META[f.advice ?? "Unknown"].color }} />
            <div className="item-info">
              <div className="item-title">
                <span className="path-truncate" title={f.path}>{f.path}</span>
                {isSystemFile(f.path) && <span style={{ flexShrink: 0, color: "#6c7086" }}>(系统文件)</span>}
              </div>
              <div className="item-desc" style={{ display: "flex", gap: 6, alignItems: "center" }}>
                <AdviceBadge advice={f.advice ?? "Unknown"} reason={f.advice_reason || ""} />
                {f.advice_reason && <span style={{ color: "#6c7086" }}>{f.advice_reason}</span>}
              </div>
            </div>
            <div className="item-size" style={{ display: "flex", alignItems: "center", gap: 8 }}>
              {formatBytes(f.size_bytes)}
              {f.advice === "Safe" && (
                <button onClick={() => deleteOne(f.path)} disabled={deleting} title="删除此文件" style={{ background: "none", border: "none", cursor: deleting ? "not-allowed" : "pointer", padding: 2, display: "flex", alignItems: "center", opacity: deleting ? 0.5 : 1, color: "#f38ba8" }}>
                  <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"><polyline points="3 6 5 6 21 6" /><path d="M19 6l-2 14a2 2 0 0 1-2 2H9a2 2 0 0 1-2-2L5 6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2" /></svg>
                </button>
              )}
            </div>
          </div>
        ))}
      </div>

      <div className="section">
        <h3 className="sub-title">对比快照</h3>
        <div className="schedule" style={{ gap: 8 }}>
          <label>从
            <select value={diffPrev} onChange={(e) => setDiffPrev(e.target.value)}>
              <option value="">选择...</option>
              {snapshots.map((s) => <option key={s.timestamp} value={s.timestamp}>{new Date(s.timestamp).toLocaleString()}</option>)}
            </select>
          </label>
          <label>到
            <select value={diffCurr} onChange={(e) => setDiffCurr(e.target.value)}>
              <option value="">选择...</option>
              {snapshots.map((s) => <option key={s.timestamp} value={s.timestamp}>{new Date(s.timestamp).toLocaleString()}</option>)}
            </select>
          </label>
          <button className="secondary" onClick={runDiff} disabled={!diffPrev || !diffCurr}>对比</button>
        </div>
        {deltas && (
          <div>
            {deltas.length === 0 && <div className="empty">无变化</div>}
            {removedDirs.length > 0 && <div><b>消失：</b>{removedDirs.map((d) => `${d.path} (${formatBytes(d.prev_bytes)})`).join("，")}</div>}
            {addedDirs.length > 0 && <div><b>新增：</b>{addedDirs.map((d) => `${d.path} (${formatBytes(d.curr_bytes)})`).join("，")}</div>}
            {movedDirs.length > 0 && <div style={{ color: "#6c7086" }}><b>疑似移动：</b>{movedDirs.length} 项已抑制</div>}
          </div>
        )}
      </div>

      {config && (
        <div className="section">
          <h3 className="sub-title">配置</h3>
          <div className="schedule" style={{ gap: 8 }}>
            <label>大文件阈值 (MB)
              <input type="number" style={{ width: 80 }} value={Math.round(config.large_file_min_bytes / 1024 / 1024)}
                onChange={(e) => setConfig({ ...config, large_file_min_bytes: Number(e.target.value) * 1024 * 1024 })} />
            </label>
            <label>Top-N
              <input type="number" style={{ width: 60 }} value={config.large_file_top_n}
                onChange={(e) => setConfig({ ...config, large_file_top_n: Number(e.target.value) })} />
            </label>
            <label>保留天数
              <input type="number" style={{ width: 60 }} value={config.snapshot_keep_days}
                onChange={(e) => setConfig({ ...config, snapshot_keep_days: Number(e.target.value) })} />
            </label>
          </div>
          <div style={{ marginTop: 8 }}>
            <b>监控目录：</b>
            {config.monitor_dirs.map((d) => (
              <span key={d} className="chip" style={{ cursor: "pointer", marginRight: 4 }}
                onClick={() => setConfig({ ...config, monitor_dirs: config.monitor_dirs.filter((x) => x !== d) })}
                title="点击移除">{d} ×</span>
            ))}
          </div>
          <div className="actions" style={{ marginTop: 8 }}>
            <input value={newDir} onChange={(e) => setNewDir(e.target.value)} placeholder="新增监控目录路径" style={{ flex: 1 }} />
            <button className="secondary" onClick={addMonitorDir}>添加</button>
            <button onClick={saveConfig}>保存</button>
          </div>
        </div>
      )}

      {log.length > 0 && (
        <div className="section">
          <h3 className="sub-title">日志</h3>
          <div className="logs">{log.map((l, i) => <div key={i}>· {l}</div>)}</div>
        </div>
      )}
    </div>
  );
}

use crate::models::{LargeFileEntry, MonitorEntry};
use std::path::Path;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DirSummary {
    pub size_bytes: u64,
    pub file_count: u64,
}

/// Callback driven Windows enumerator: opens `root\*`, invokes `f` for each
/// non-dot, non-reparse entry's WIN32_FIND_DATAW, then closes the handle.
/// Inlining the FindFirst/FindNext loop here avoids the "first entry already
/// returned by FindFirst" footgun that a split open/next API would cause.
#[cfg(windows)]
fn for_each_entry<F>(root: &Path, mut f: F)
where
    F: FnMut(&windows::Win32::Storage::FileSystem::WIN32_FIND_DATAW),
{
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::{
        FindFirstFileExW, FindNextFileW, FIND_FIRST_EX_LARGE_FETCH,
        FindExInfoBasic, FindExSearchNameMatch, WIN32_FIND_DATAW,
    };
    let pattern = to_wide_pattern(root);
    let mut data: WIN32_FIND_DATAW = unsafe { std::mem::zeroed() };
    let handle = unsafe {
        FindFirstFileExW(
            PCWSTR(pattern.as_ptr()),
            FindExInfoBasic,
            &mut data as *mut _ as *mut _,
            FindExSearchNameMatch,
            None,
            FIND_FIRST_EX_LARGE_FETCH,
        )
    };
    let Ok(handle) = handle else { return };
    // FindFirstFileExW fills `data` with the first match; loop processes it,
    // then FindNextFileW fills each subsequent one.
    loop {
        f(&data);
        if unsafe { FindNextFileW(handle, &mut data as *mut _ as *mut _) }.is_err() {
            break;
        }
    }
    let _ = unsafe { windows::Win32::Storage::FileSystem::FindClose(handle) };
}

pub fn walk_dir_fast(root: &Path) -> DirSummary {
    #[cfg(windows)]
    { walk_dir_fast_win(root) }
    #[cfg(not(windows))]
    { walk_dir_fast_fallback(root) }
}

pub fn collect_files(root: &Path, out: &mut Vec<(std::path::PathBuf, u64)>) {
    #[cfg(windows)]
    { collect_files_win(root, out) }
    #[cfg(not(windows))]
    { collect_files_fallback(root, out) }
}

#[cfg(windows)]
fn walk_dir_fast_win(root: &Path) -> DirSummary {
    let mut total: u64 = 0;
    let mut count: u64 = 0;
    for_each_entry(root, |data| {
        if !is_dots(&data.cFileName) && !is_reparse(data.dwFileAttributes) {
            if is_dir(data.dwFileAttributes) {
                let child = root.join(from_wide(&data.cFileName));
                let sub = walk_dir_fast_win(&child);
                total = total.saturating_add(sub.size_bytes);
                count = count.saturating_add(sub.file_count);
            } else {
                total = total.saturating_add(file_size_u64(data));
                count = count.saturating_add(1);
            }
        }
    });
    DirSummary { size_bytes: total, file_count: count }
}

#[cfg(windows)]
fn collect_files_win(root: &Path, out: &mut Vec<(std::path::PathBuf, u64)>) {
    for_each_entry(root, |data| {
        if !is_dots(&data.cFileName) && !is_reparse(data.dwFileAttributes) {
            if is_dir(data.dwFileAttributes) {
                let child = root.join(from_wide(&data.cFileName));
                collect_files_win(&child, out);
            } else {
                out.push((root.join(from_wide(&data.cFileName)), file_size_u64(data)));
            }
        }
    });
}

#[cfg(windows)]
fn file_size_u64(data: &windows::Win32::Storage::FileSystem::WIN32_FIND_DATAW) -> u64 {
    ((data.nFileSizeHigh as u64) << 32) | (data.nFileSizeLow as u64)
}

#[cfg(not(windows))]
fn walk_dir_fast_fallback(root: &Path) -> DirSummary {
    let mut total: u64 = 0;
    let mut count: u64 = 0;
    for entry in walkdir::WalkDir::new(root).follow_links(false).into_iter().filter_map(|e| e.ok()) {
        if entry.file_type().is_file() {
            if let Ok(md) = entry.metadata() {
                total = total.saturating_add(md.len());
                count = count.saturating_add(1);
            }
        }
    }
    DirSummary { size_bytes: total, file_count: count }
}

#[cfg(not(windows))]
fn collect_files_fallback(root: &Path, out: &mut Vec<(std::path::PathBuf, u64)>) {
    for entry in walkdir::WalkDir::new(root).follow_links(false).into_iter().filter_map(|e| e.ok()) {
        if entry.file_type().is_file() {
            if let Ok(md) = entry.metadata() {
                out.push((entry.path().to_path_buf(), md.len()));
            }
        }
    }
}
// (win helpers below moved above for clarity; this region intentionally left
// for the FFI small helpers and platform conditionals.)
#[cfg(windows)]
fn to_wide_pattern(root: &Path) -> Vec<u16> {
    let s = root.to_string_lossy();
    let with_wildcard = if s.ends_with('\\') { format!("{}*", s) } else { format!("{}\\*", s) };
    with_wildcard.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(windows)]
fn from_wide(buf: &[u16]) -> String {
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..len])
}

#[cfg(windows)]
fn is_dots(name: &[u16]) -> bool {
    let len = name.iter().position(|&c| c == 0).unwrap_or(name.len());
    (len == 1 && name[0] == b'.' as u16) || (len == 2 && name[0] == b'.' as u16 && name[1] == b'.' as u16)
}

#[cfg(windows)]
fn is_reparse(attrs: u32) -> bool {
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    (attrs & FILE_ATTRIBUTE_REPARSE_POINT) != 0
}

#[cfg(windows)]
fn is_dir(attrs: u32) -> bool {
    const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;
    (attrs & FILE_ATTRIBUTE_DIRECTORY) != 0
}

#[cfg(windows)]
fn list_children_win(path: &Path) -> Vec<MonitorEntry> {
    let mut out = Vec::new();
    for_each_entry(path, |data| {
        if !is_dots(&data.cFileName) && !is_reparse(data.dwFileAttributes) {
            let name = from_wide(&data.cFileName);
            let child = path.join(&name);
            if is_dir(data.dwFileAttributes) {
                let s = walk_dir_fast_win(&child);
                out.push(MonitorEntry {
                    path: child.to_string_lossy().into_owned(),
                    size_bytes: s.size_bytes,
                    file_count: s.file_count,
                    exists: true,
                });
            } else {
                out.push(MonitorEntry {
                    path: child.to_string_lossy().into_owned(),
                    size_bytes: file_size_u64(data),
                    file_count: 1,
                    exists: true,
                });
            }
        }
    });
    out
}

pub fn list_children(path: &Path) -> Vec<MonitorEntry> {
    let mut out: Vec<MonitorEntry>;
    #[cfg(windows)]
    {
        out = list_children_win(path);
    }
    #[cfg(not(windows))]
    {
        let mut tmp = Vec::new();
        if let Ok(rd) = std::fs::read_dir(path) {
            for entry in rd.flatten() {
                let p = entry.path();
                let ft = match entry.file_type() { Ok(t) => t, Err(_) => continue };
                if ft.is_dir() {
                    let s = walk_dir_fast_fallback(&p);
                    tmp.push(MonitorEntry { path: p.to_string_lossy().into_owned(), size_bytes: s.size_bytes, file_count: s.file_count, exists: true });
                } else if ft.is_file() {
                    let sz = entry.metadata().map(|m| m.len()).unwrap_or(0);
                    tmp.push(MonitorEntry { path: p.to_string_lossy().into_owned(), size_bytes: sz, file_count: 1, exists: true });
                }
            }
        }
        out = tmp;
    }
    out.sort_by(|a, b| b.size_bytes.cmp(&a.size_bytes));
    out
}

use crate::models::{MonitorSnapshot};
use std::collections::BinaryHeap;
use std::path::PathBuf;

/// Keep the N largest files from `candidates`, descending.
/// Uses a min-heap of size N so we don't sort the full set.
pub fn top_n_files(
    candidates: Vec<(PathBuf, u64)>,
    min_bytes: u64,
    n: usize,
) -> Vec<LargeFileEntry> {
    if n == 0 {
        return Vec::new();
    }
    // Min-heap keyed by size so the smallest of the kept set is on top; pop it
    // when a larger candidate arrives. Store (size, path) primitives so the
    // heap's Ord bound is satisfied (LargeFileEntry is not Ord).
    let mut heap: BinaryHeap<std::cmp::Reverse<(u64, String)>> = BinaryHeap::new();
    for (path, size) in candidates {
        if size < min_bytes {
            continue;
        }
        let path_str = path.to_string_lossy().into_owned();
        if heap.len() < n {
            heap.push(std::cmp::Reverse((size, path_str)));
        } else if let Some(std::cmp::Reverse((smallest, _))) = heap.peek() {
            if size > *smallest {
                heap.pop();
                heap.push(std::cmp::Reverse((size, path_str)));
            }
        }
    }
    let mut out: Vec<LargeFileEntry> = heap
        .into_iter()
        .map(|std::cmp::Reverse((size, path))| LargeFileEntry { path, size_bytes: size })
        .collect();
    out.sort_by(|a, b| b.size_bytes.cmp(&a.size_bytes));
    out
}

/// Rescan the configured monitor directories (and any auto-appended dirs from
/// the previous snapshot's large files). Fast: each dir is one walk_dir_fast.
pub fn scan_monitor_dirs(dirs: &[String]) -> Vec<MonitorEntry> {
    let mut out = Vec::with_capacity(dirs.len());
    for raw in dirs {
        let p = Path::new(raw);
        if !p.exists() {
            out.push(MonitorEntry { path: raw.clone(), size_bytes: 0, file_count: 0, exists: false });
            continue;
        }
        let s = walk_dir_fast(p);
        out.push(MonitorEntry {
            path: raw.clone(),
            size_bytes: s.size_bytes,
            file_count: s.file_count,
            exists: true,
        });
    }
    out.sort_by(|a, b| b.size_bytes.cmp(&a.size_bytes));
    out
}

/// Full C-drive scan: walks every monitor dir AND the whole C:\ root to collect
/// Top-N large files. `auto_append_dirs` lets the caller inject parent dirs of
/// the previous snapshot's large files so emerging culprits stay tracked.
/// `on_progress` receives (dirs_done, dirs_total) after each monitor dir.
pub fn scan_full<F>(
    monitor_dirs: Vec<String>,
    auto_append_dirs: Vec<String>,
    min_bytes: u64,
    top_n: u32,
    drive_total: u64,
    drive_used: u64,
    mut on_progress: F,
) -> MonitorSnapshot
where
    F: FnMut(u64, u64),
{
    // Merge + dedupe monitor dirs, capped at 200 (spec). Drop-blank entries.
    let mut dirs: Vec<String> = monitor_dirs;
    for d in auto_append_dirs {
        if !dirs.iter().any(|x| x.eq_ignore_ascii_case(&d)) {
            dirs.push(d);
        }
    }
    if dirs.len() > 200 {
        dirs.truncate(200);
    }
    let total_dirs = dirs.len() as u64;

    // 1. Walk each monitor dir for size entries.
    let mut monitor_entries: Vec<MonitorEntry> = Vec::with_capacity(dirs.len());
    for (i, raw) in dirs.iter().enumerate() {
        let p = Path::new(raw);
        if !p.exists() {
            monitor_entries.push(MonitorEntry { path: raw.clone(), size_bytes: 0, file_count: 0, exists: false });
        } else {
            let s = walk_dir_fast(p);
            monitor_entries.push(MonitorEntry { path: raw.clone(), size_bytes: s.size_bytes, file_count: s.file_count, exists: true });
        }
        on_progress((i as u64) + 1, total_dirs);
    }
    monitor_entries.sort_by(|a, b| b.size_bytes.cmp(&a.size_bytes));

    // 2. Walk the whole drive root once, collecting candidate files for Top-N.
    let mut candidates: Vec<(PathBuf, u64)> = Vec::new();
    collect_files(Path::new("C:\\"), &mut candidates);
    let large = top_n_files(candidates, min_bytes, top_n as usize);

    MonitorSnapshot {
        timestamp: chrono::Local::now().to_rfc3339(),
        scan_type: "full".into(),
        drive_total,
        drive_used,
        monitor_dirs: monitor_entries,
        large_files: large,
    }
}

use crate::models::DirDelta;
use std::collections::HashMap;

/// Diff two snapshots. Merges monitor_dirs and large_files by path key, then
/// suppresses moves: a prev-only file paired with a curr-only file of the same
/// extension and near-equal size (within 5%) is marked kind="moved" instead of
/// emitting one added + one removed.
pub fn diff_snapshots(prev: &MonitorSnapshot, curr: &MonitorSnapshot) -> Vec<DirDelta> {
    let mut out: Vec<DirDelta> = Vec::new();

    // ---- monitor dirs by path key ----
    diff_by_key(&prev.monitor_dirs, &curr.monitor_dirs, &mut out);

    // ---- large files: detect moves before emitting added/removed ----
    let mut prev_map: HashMap<&str, u64> = prev.large_files.iter().map(|e| (e.path.as_str(), e.size_bytes)).collect();
    let mut curr_map: HashMap<&str, u64> = curr.large_files.iter().map(|e| (e.path.as_str(), e.size_bytes)).collect();

    // changed: in both. Collect first, then remove from maps before iterating
    // the emitted deltas (avoids borrow-after-move on `common`).
    let changed: Vec<(&str, u64, u64)> = prev_map
        .keys()
        .filter(|k| curr_map.contains_key(*k))
        .map(|&k| (k, prev_map[k], curr_map[k]))
        .collect();
    for (path, p, c) in &changed {
        prev_map.remove(path);
        curr_map.remove(path);
    }
    for (path, p, c) in changed {
        if p != c {
            out.push(make_delta(path, "changed", p, c));
        }
    }

    // prev-only and curr-only -> try to pair as moves
    let prev_only: Vec<(&str, u64)> = prev_map.iter().map(|(k, v)| (*k, *v)).collect();
    let curr_only: Vec<(&str, u64)> = curr_map.iter().map(|(k, v)| (*k, *v)).collect();
    let mut matched_curr: Vec<bool> = vec![false; curr_only.len()];
    for (p_path, p_size) in &prev_only {
        let p_ext = ext(p_path);
        // Find a curr-only item with same extension and size within 5%.
        let pair = curr_only.iter().enumerate().find(|(i, (c_path, c_size))| {
            !matched_curr[*i] && ext(c_path) == p_ext && within_5pct(*p_size, *c_size)
        });
        match pair {
            Some((i, (c_path, c_size))) => {
                matched_curr[i] = true;
                // Represent the move as a single delta on the curr path; the
                // prev path is mentioned via kind only. prev_bytes is the old
                // location's size for reference.
                out.push(DirDelta {
                    path: c_path.to_string(),
                    kind: "moved".into(),
                    prev_bytes: *p_size,
                    curr_bytes: *c_size,
                    delta_bytes: 0,
                    pct: 0.0,
                });
            }
            None => {
                out.push(DirDelta {
                    path: p_path.to_string(),
                    kind: "removed".into(),
                    prev_bytes: *p_size,
                    curr_bytes: 0,
                    delta_bytes: -(*p_size as i64),
                    pct: -100.0,
                });
            }
        }
    }
    for (i, (c_path, c_size)) in curr_only.iter().enumerate() {
        if !matched_curr[i] {
            out.push(make_delta(c_path, "added", 0, *c_size));
        }
    }

    // Sort by absolute delta magnitude (changed/added/removed), moved last.
    out.sort_by(|a, b| {
        let rank = |d: &DirDelta| match d.kind.as_str() { "moved" => 1, _ => 0 };
        rank(a).cmp(&rank(b)).then_with(|| b.delta_bytes.unsigned_abs().cmp(&a.delta_bytes.unsigned_abs()))
    });
    out
}

fn diff_by_key(
    prev: &[MonitorEntry],
    curr: &[MonitorEntry],
    out: &mut Vec<DirDelta>,
) {
    let prev_map: HashMap<&str, u64> = prev.iter().map(|e| (e.path.as_str(), e.size_bytes)).collect();
    let curr_map: HashMap<&str, u64> = curr.iter().map(|e| (e.path.as_str(), e.size_bytes)).collect();
    let mut keys: Vec<&str> = prev_map.keys().chain(curr_map.keys()).copied().collect();
    keys.sort();
    keys.dedup();
    for path in keys {
        match (prev_map.get(path), curr_map.get(path)) {
            (Some(&p), Some(&c)) if p != c => out.push(make_delta(path, "changed", p, c)),
            (Some(&p), None) => out.push(DirDelta { path: path.into(), kind: "removed".into(), prev_bytes: p, curr_bytes: 0, delta_bytes: -(p as i64), pct: -100.0 }),
            (None, Some(&c)) => out.push(make_delta(path, "added", 0, c)),
            _ => {}
        }
    }
}

fn make_delta(path: &str, kind: &str, prev: u64, curr: u64) -> DirDelta {
    let delta = curr as i64 - prev as i64;
    let pct = if prev == 0 { if curr == 0 { 0.0 } else { 100.0 } } else { (delta as f64) / (prev as f64) * 100.0 };
    DirDelta { path: path.into(), kind: kind.into(), prev_bytes: prev, curr_bytes: curr, delta_bytes: delta, pct }
}

fn ext(path: &str) -> &str {
    match path.rsplit('.').next() {
        Some(e) if e.contains('\\') || e.contains('/') => "",
        Some(e) => e,
        None => "",
    }
}

fn within_5pct(a: u64, b: u64) -> bool {
    let max = a.max(b);
    if max == 0 { return true; }
    let diff = if a > b { a - b } else { b - a };
    (diff as f64) / (max as f64) <= 0.05
}

pub(crate) fn large_file_entries(pairs: Vec<(std::path::PathBuf, u64)>) -> Vec<LargeFileEntry> {
    pairs.into_iter().map(|(p, s)| LargeFileEntry { path: p.to_string_lossy().into_owned(), size_bytes: s }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

    fn tmp(name: &str) -> PathBuf {
        let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let mut p = std::env::temp_dir();
        p.push(format!("dct_an_{}_{}", name, n));
        fs::create_dir_all(&p).unwrap();
        p
    }

    fn write(path: &Path, bytes: &[u8]) {
        if let Some(parent) = path.parent() { fs::create_dir_all(parent).unwrap(); }
        fs::write(path, bytes).unwrap();
    }

    fn cleanup(p: &Path) { let _ = fs::remove_dir_all(p); }

    #[test]
    fn walk_dir_fast_sums_nested_sizes_and_counts() {
        let root = tmp("walk");
        write(&root.join("a.txt"), b"AAAAAAAAAA");
        write(&root.join("sub").join("b.txt"), &vec![b'B'; 20]);
        write(&root.join("sub").join("deeper").join("c.txt"), b"CCCCC");
        let s = walk_dir_fast(&root);
        assert_eq!(s.size_bytes, 35);
        assert_eq!(s.file_count, 3);
        cleanup(&root);
    }

    #[test]
    fn walk_dir_fast_empty_dir() {
        let root = tmp("empty");
        let s = walk_dir_fast(&root);
        assert_eq!(s, DirSummary::default());
        cleanup(&root);
    }

    #[test]
    fn collect_files_gathers_all_sizes() {
        let root = tmp("collect");
        write(&root.join("a.bin"), &vec![0; 100]);
        write(&root.join("d").join("b.bin"), &vec![0; 200]);
        let mut out = Vec::new();
        collect_files(&root, &mut out);
        assert_eq!(out.len(), 2);
        let total: u64 = out.iter().map(|(_, s)| *s).sum();
        assert_eq!(total, 300);
        cleanup(&root);
    }

    #[test]
    fn list_children_single_level_descending() {
        let root = tmp("children");
        write(&root.join("big.bin"), &vec![0; 500]);
        write(&root.join("small.bin"), &vec![0; 10]);
        write(&root.join("sub").join("inside.bin"), &vec![0; 300]);
        let kids = list_children(&root);
        assert_eq!(kids.len(), 3);
        assert!(kids[0].size_bytes >= kids[1].size_bytes);
        assert_eq!(kids[0].size_bytes, 500);
        cleanup(&root);
    }

    #[test]
    fn top_n_files_keeps_largest_descending() {
        // Sizes 1..5 MB, ask for top 3 -> expect [5,4,3].
        let cands: Vec<(PathBuf, u64)> = (1..=5)
            .map(|i| (PathBuf::from(format!("/f{}.bin", i)), i * 1024 * 1024))
            .collect();
        let got = top_n_files(cands, 0, 3);
        assert_eq!(got.len(), 3);
        assert_eq!(got[0].size_bytes, 5 * 1024 * 1024);
        assert_eq!(got[1].size_bytes, 4 * 1024 * 1024);
        assert_eq!(got[2].size_bytes, 3 * 1024 * 1024);
    }

    #[test]
    fn top_n_files_respects_min_bytes_filter() {
        let cands = vec![
            (PathBuf::from("/a"), 100),
            (PathBuf::from("/b"), 500),
            (PathBuf::from("/c"), 900),
        ];
        // min_bytes=400 -> only b and c qualify.
        let got = top_n_files(cands, 400, 10);
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].size_bytes, 900);
        assert_eq!(got[1].size_bytes, 500);
    }

    #[test]
    fn top_n_files_zero_n_returns_empty() {
        let cands = vec![(PathBuf::from("/a"), 1000)];
        assert!(top_n_files(cands, 0, 0).is_empty());
    }

    #[test]
    fn scan_monitor_dirs_marks_missing_and_sorts_desc() {
        let a = tmp("smd_a");
        let b = tmp("smd_b");
        write(&a.join("x.bin"), &vec![0; 100]);
        write(&b.join("y.bin"), &vec![0; 500]);
        let dirs = vec![
            b.to_string_lossy().into_owned(),
            a.to_string_lossy().into_owned(),
            "/no/such/path/here".into(),
        ];
        let got = scan_monitor_dirs(&dirs);
        assert_eq!(got.len(), 3);
        assert!(got[0].size_bytes >= got[1].size_bytes);
        assert!(got[0].exists);
        assert_eq!(got[0].size_bytes, 500);
        assert!(got[2].exists == false);
        cleanup(&a);
        cleanup(&b);
    }

    fn snap(ts: &str, dirs: Vec<(&str, u64)>, files: Vec<(&str, u64)>) -> MonitorSnapshot {
        MonitorSnapshot {
            timestamp: ts.into(),
            scan_type: "full".into(),
            drive_total: 100,
            drive_used: 50,
            monitor_dirs: dirs.into_iter().map(|(p, s)| MonitorEntry { path: p.into(), size_bytes: s, file_count: 0, exists: true }).collect(),
            large_files: files.into_iter().map(|(p, s)| LargeFileEntry { path: p.into(), size_bytes: s }).collect(),
        }
    }

    #[test]
    fn diff_detects_changed_added_removed() {
        let prev = snap("t1",
            vec![("C:/A", 100), ("C:/B", 50)],
            vec![("C:/A/f.bin", 1000)]);
        // B grows, C added, A unchanged, file f.bin removed (no move since nothing matches).
        let curr = snap("t2",
            vec![("C:/A", 100), ("C:/B", 200), ("C:/C", 30)],
            vec![]);
        let d = diff_snapshots(&prev, &curr);
        let kinds: Vec<(&str, &str)> = d.iter().map(|x| (x.kind.as_str(), x.path.as_str())).collect();
        assert!(kinds.contains(&("changed", "C:/B")));
        assert!(kinds.contains(&("added", "C:/C")));
        assert!(kinds.contains(&("removed", "C:/A/f.bin")));
        // B: 50 -> 200, delta +150, pct +300.
        let b = d.iter().find(|x| x.path == "C:/B").unwrap();
        assert_eq!(b.delta_bytes, 150);
        assert_eq!(b.pct, 300.0);
    }

    #[test]
    fn diff_suppresses_move_same_ext_similar_size() {
        let prev = snap("t1", vec![], vec![("C:/old/f.bin", 1000)]);
        let curr = snap("t2", vec![], vec![("C:/new/g.bin", 1000)]);
        let d = diff_snapshots(&prev, &curr);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].kind, "moved");
        assert_eq!(d[0].path, "C:/new/g.bin");
    }

    #[test]
    fn diff_does_not_suppress_when_size_differs_beyond_5pct() {
        let prev = snap("t1", vec![], vec![("C:/old/f.bin", 1000)]);
        let curr = snap("t2", vec![], vec![("C:/new/g.bin", 2000)]);
        let d = diff_snapshots(&prev, &curr);
        // 100% size difference -> not a move, emit added + removed.
        assert_eq!(d.len(), 2);
        assert!(d.iter().any(|x| x.kind == "added"));
        assert!(d.iter().any(|x| x.kind == "removed"));
    }

    #[test]
    fn diff_new_file_pct_is_100() {
        let prev = snap("t1", vec![("C:/A", 0)], vec![]);
        let curr = snap("t2", vec![("C:/A", 50)], vec![]);
        let d = diff_snapshots(&prev, &curr);
        let a = d.iter().find(|x| x.path == "C:/A").unwrap();
        assert_eq!(a.kind, "changed");
        assert_eq!(a.pct, 100.0);
    }
}

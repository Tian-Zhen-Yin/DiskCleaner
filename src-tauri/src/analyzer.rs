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

/// Streaming Top-N accumulator: keeps the N largest files seen so far in a
/// min-heap, so peak memory stays O(n) regardless of how many files the walker
/// visits. Replaces the old two-phase design (materialize the full file list
/// into a Vec, then filter+heap) that could hold hundreds of MB on a
/// multi-million-file drive.
pub struct TopNCollector {
    heap: std::collections::BinaryHeap<std::cmp::Reverse<(u64, String)>>,
    min_bytes: u64,
    n: usize,
}

impl TopNCollector {
    pub fn new(min_bytes: u64, n: usize) -> Self {
        Self { heap: std::collections::BinaryHeap::new(), min_bytes, n }
    }

    /// Consider one file. Below `min_bytes` or `n==0` are dropped with no
    /// allocation. A full heap evicts its smallest entry only when the new
    /// candidate is strictly larger (ties keep insertion order).
    pub fn consider(&mut self, path: String, size: u64) {
        if self.n == 0 || size < self.min_bytes {
            return;
        }
        if self.heap.len() < self.n {
            self.heap.push(std::cmp::Reverse((size, path)));
        } else if let Some(std::cmp::Reverse((smallest, _))) = self.heap.peek() {
            if size > *smallest {
                self.heap.pop();
                self.heap.push(std::cmp::Reverse((size, path)));
            }
        }
    }

    /// Drain into a size-descending list, tagging each path with its
    /// cleanability advice via classify::classify.
    pub fn finish(self) -> Vec<LargeFileEntry> {
        let mut out: Vec<LargeFileEntry> = self
            .heap
            .into_iter()
            .map(|std::cmp::Reverse((size, path))| {
                let (advice, advice_reason) = crate::classify::classify(&path);
                LargeFileEntry { path, size_bytes: size, advice, advice_reason }
            })
            .collect();
        out.sort_by(|a, b| b.size_bytes.cmp(&a.size_bytes));
        out
    }
}

/// Walk `root` recursively, streaming every regular file into `collector`.
/// The walker feeds the min-heap directly instead of materializing a full Vec,
/// keeping peak memory at O(top_n) rather than O(total files on the drive).
pub fn collect_top_n(root: &Path, collector: &mut TopNCollector) {
    #[cfg(windows)]
    { collect_top_n_win(root, collector) }
    #[cfg(not(windows))]
    { collect_top_n_fallback(root, collector) }
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
fn collect_top_n_win(root: &Path, collector: &mut TopNCollector) {
    for_each_entry(root, |data| {
        if !is_dots(&data.cFileName) && !is_reparse(data.dwFileAttributes) {
            if is_dir(data.dwFileAttributes) {
                let child = root.join(from_wide(&data.cFileName));
                collect_top_n_win(&child, collector);
            } else {
                collector.consider(
                    root.join(from_wide(&data.cFileName)).to_string_lossy().into_owned(),
                    file_size_u64(data),
                );
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
fn collect_top_n_fallback(root: &Path, collector: &mut TopNCollector) {
    for entry in walkdir::WalkDir::new(root).follow_links(false).into_iter().filter_map(|e| e.ok()) {
        if entry.file_type().is_file() {
            if let Ok(md) = entry.metadata() {
                collector.consider(entry.path().to_string_lossy().into_owned(), md.len());
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
                let pstr = child.to_string_lossy().into_owned();
                let (advice, advice_reason) = crate::classify::classify(&pstr);
                out.push(MonitorEntry {
                    path: pstr,
                    size_bytes: s.size_bytes,
                    file_count: s.file_count,
                    exists: true,
                    advice,
                    advice_reason,
                });
            } else {
                let pstr = child.to_string_lossy().into_owned();
                let (advice, advice_reason) = crate::classify::classify(&pstr);
                out.push(MonitorEntry {
                    path: pstr,
                    size_bytes: file_size_u64(data),
                    file_count: 1,
                    exists: true,
                    advice,
                    advice_reason,
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
                    let pstr = p.to_string_lossy().into_owned();
                    let (advice, advice_reason) = crate::classify::classify(&pstr);
                    tmp.push(MonitorEntry { path: pstr, size_bytes: s.size_bytes, file_count: s.file_count, exists: true, advice, advice_reason });
                } else if ft.is_file() {
                    let sz = entry.metadata().map(|m| m.len()).unwrap_or(0);
                    let pstr = p.to_string_lossy().into_owned();
                    let (advice, advice_reason) = crate::classify::classify(&pstr);
                    tmp.push(MonitorEntry { path: pstr, size_bytes: sz, file_count: 1, exists: true, advice, advice_reason });
                }
            }
        }
        out = tmp;
    }
    out.sort_by(|a, b| b.size_bytes.cmp(&a.size_bytes));
    out
}

use crate::models::{MonitorSnapshot};
use std::path::PathBuf;

/// Keep the N largest files from an already-materialized candidate list. Thin
/// wrapper over TopNCollector; the production scan path (scan_full) uses
/// collect_top_n to stream files into the heap without materializing them first.
pub fn top_n_files(
    candidates: Vec<(PathBuf, u64)>,
    min_bytes: u64,
    n: usize,
) -> Vec<LargeFileEntry> {
    let mut c = TopNCollector::new(min_bytes, n);
    for (path, size) in candidates {
        c.consider(path.to_string_lossy().into_owned(), size);
    }
    c.finish()
}

/// Rescan the configured monitor directories (and any auto-appended dirs from
/// the previous snapshot's large files). Fast: each dir is one walk_dir_fast.
pub fn scan_monitor_dirs(dirs: &[String]) -> Vec<MonitorEntry> {
    let mut out = Vec::with_capacity(dirs.len());
    for raw in dirs {
        let p = Path::new(raw);
        if !p.exists() {
            let (advice, advice_reason) = crate::classify::classify(raw);
            out.push(MonitorEntry { path: raw.clone(), size_bytes: 0, file_count: 0, exists: false, advice, advice_reason });
            continue;
        }
        let s = walk_dir_fast(p);
        let (advice, advice_reason) = crate::classify::classify(raw);
        out.push(MonitorEntry {
            path: raw.clone(),
            size_bytes: s.size_bytes,
            file_count: s.file_count,
            exists: true,
            advice,
            advice_reason,
        });
    }
    out.sort_by(|a, b| b.size_bytes.cmp(&a.size_bytes));
    out
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
    for (path, _p, _c) in &changed {
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
    fn collect_top_n_streams_all_files_into_heap() {
        let root = tmp("collect");
        write(&root.join("a.bin"), &vec![0; 100]);
        write(&root.join("d").join("b.bin"), &vec![0; 200]);
        // min_bytes=0, n large enough to keep everything.
        let mut c = TopNCollector::new(0, 10);
        collect_top_n(&root, &mut c);
        let got = c.finish();
        assert_eq!(got.len(), 2);
        let total: u64 = got.iter().map(|e| e.size_bytes).sum();
        assert_eq!(total, 300);
        cleanup(&root);
    }

    #[test]
    fn collect_top_n_evicts_smaller_files() {
        // Five files of decreasing size; keep only the top 2.
        let root = tmp("evict");
        write(&root.join("f1.bin"), &vec![0; 50]);
        write(&root.join("f2.bin"), &vec![0; 40]);
        write(&root.join("f3.bin"), &vec![0; 30]);
        write(&root.join("f4.bin"), &vec![0; 20]);
        write(&root.join("f5.bin"), &vec![0; 10]);
        let mut c = TopNCollector::new(0, 2);
        collect_top_n(&root, &mut c);
        let got = c.finish();
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].size_bytes, 50);
        assert_eq!(got[1].size_bytes, 40);
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

    #[test]
    fn drive_scan_single_pass_sizes_monitors_and_collects_topn() {
        // Tree (root is a tempdir standing in for C:\):
        //   Users/u/docs/big.iso        (1000)  <- under Users monitor
        //   Users/u/Cache/junk.bin      (500)   <- under Users monitor
        //   Windows/Temp/t.tmp          (300)   <- under Windows AND Windows\Temp
        //   pagefile.sys                (900)   <- NOT under any monitor (Top-N only)
        //   Missing/                    (empty) <- monitor dir that does not exist
        let root = tmp("drive");
        write(&root.join("Users").join("u").join("docs").join("big.iso"), &vec![0; 1000]);
        write(&root.join("Users").join("u").join("Cache").join("junk.bin"), &vec![0; 500]);
        write(&root.join("Windows").join("Temp").join("t.tmp"), &vec![0; 300]);
        write(&root.join("pagefile.sys"), &vec![0; 900]);

        let monitors = vec![
            root.join("Users").to_string_lossy().into_owned(),
            root.join("Windows").to_string_lossy().into_owned(),
            root.join("Windows").join("Temp").to_string_lossy().into_owned(),
            root.join("Missing").to_string_lossy().into_owned(),
        ];
        let mut scan = DriveScan::new(&monitors, 0, 10);
        let mut active = Vec::new();
        let mut last = (0u64, 0u64);
        collect_drive_with_monitors(&root, &mut scan, &mut active, &mut |d, t| {
            last = (d, t);
        });

        // Users: big.iso(1000) + junk.bin(500) = 1500, 2 files.
        let u_key = norm_lower(&root.join("Users"));
        let u_idx = scan.monitor_idx[&u_key];
        assert_eq!(scan.sizes[u_idx], 1500);
        assert_eq!(scan.counts[u_idx], 2);

        // Windows (parent of Temp): the Temp file counts here too (overlap).
        let w_key = norm_lower(&root.join("Windows"));
        let w_idx = scan.monitor_idx[&w_key];
        assert_eq!(scan.sizes[w_idx], 300);
        assert_eq!(scan.counts[w_idx], 1);

        // Windows\Temp (nested monitor): same file, charged independently.
        let t_key = norm_lower(&root.join("Windows").join("Temp"));
        let t_idx = scan.monitor_idx[&t_key];
        assert_eq!(scan.sizes[t_idx], 300);
        assert_eq!(scan.counts[t_idx], 1);

        // pagefile.sys at root: feeds Top-N, never attributed to a monitor.
        // Missing dir: never entered.
        let m_key = norm_lower(&root.join("Missing"));
        let m_idx = scan.monitor_idx[&m_key];
        assert!(!scan.reached[m_idx]);

        // Top-N collected all 4 files regardless of monitor membership.
        let large = scan.collector.finish();
        assert_eq!(large.len(), 4);
        let total: u64 = large.iter().map(|e| e.size_bytes).sum();
        assert_eq!(total, 2700);

        // Three monitors completed during the walk; Missing was not reached so
        // it is not counted here (scan_full accounts for it afterward).
        assert_eq!(last, (3, 4));

        cleanup(&root);
    }

    fn snap(ts: &str, dirs: Vec<(&str, u64)>, files: Vec<(&str, u64)>) -> MonitorSnapshot {
        MonitorSnapshot {
            timestamp: ts.into(),
            scan_type: "full".into(),
            drive_total: 100,
            drive_used: 50,
            monitor_dirs: dirs.into_iter().map(|(p, s)| { let (ad, ar) = crate::classify::classify(p); MonitorEntry { path: p.into(), size_bytes: s, file_count: 0, exists: true, advice: ad, advice_reason: ar } }).collect(),
            large_files: files.into_iter().map(|(p, s)| { let (ad, ar) = crate::classify::classify(p); LargeFileEntry { path: p.into(), size_bytes: s, advice: ad, advice_reason: ar } }).collect(),
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
/// Normalize a path for case-insensitive monitor matching: lowercased, with
/// `/` folded to `\`. Used both to key the monitor table and to derive the
/// lookup key for each directory the walker enters.
fn norm_lower(p: &Path) -> String {
    p.to_string_lossy().to_lowercase().replace('/', r"\")
}

/// True if `raw` is (case-insensitively) the C: drive or lives under it. The
/// single-pass walk only covers C:\, so monitors on other drives are walked
/// separately.
fn is_on_c_drive(raw: &str) -> bool {
    let lower = raw.to_lowercase().replace('/', r"\");
    lower == "c:" || lower.starts_with(r"c:\")
}

/// Accumulator for the single-pass drive scan. Owns the Top-N collector and the
/// per-monitor-dir size/count tallies. Monitor dirs are keyed by their
/// normalized (lowercased, backslash) path so the walker can detect, as it
/// descends, when it has entered a monitored directory.
struct DriveScan {
    monitor_idx: HashMap<String, usize>,
    sizes: Vec<u64>,
    counts: Vec<u64>,
    reached: Vec<bool>,
    completed: u64,
    total: u64,
    collector: TopNCollector,
}

impl DriveScan {
    fn new(monitors: &[String], min_bytes: u64, top_n: usize) -> Self {
        let mut monitor_idx = HashMap::new();
        // First occurrence wins; scan_full already dedupes case-insensitively,
        // so this just defends against path-variant collisions.
        for (i, m) in monitors.iter().enumerate() {
            monitor_idx.entry(norm_lower(Path::new(m))).or_insert(i);
        }
        let len = monitors.len();
        Self {
            monitor_idx,
            sizes: vec![0; len],
            counts: vec![0; len],
            reached: vec![false; len],
            completed: 0,
            total: len as u64,
            collector: TopNCollector::new(min_bytes, top_n),
        }
    }

    /// If `key` (the normalized path of the directory we just entered) names a
    /// monitor dir, push its index onto `active` and mark it reached. Returns
    /// whether a monitor was pushed (so the caller pops on the way back up).
    fn enter(&mut self, key: &str, active: &mut Vec<usize>) -> bool {
        if let Some(&i) = self.monitor_idx.get(key) {
            self.reached[i] = true;
            active.push(i);
            true
        } else {
            false
        }
    }

    /// Advance the completed-monitors counter; returns the new (done, total).
    fn note_done(&mut self) -> (u64, u64) {
        self.completed += 1;
        (self.completed, self.total)
    }
}

/// Single-pass drive walk: descend `root`, feeding every file to the Top-N
/// collector and adding each file's size to every monitor dir currently on the
/// `active` stack (i.e. every monitor dir that is an ancestor-or-self of the
/// file). Produces both the large-file list and the monitor-dir tallies from
/// one traversal instead of two.
fn collect_drive_with_monitors<F: FnMut(u64, u64)>(
    root: &Path,
    scan: &mut DriveScan,
    active: &mut Vec<usize>,
    on_progress: &mut F,
) {
    #[cfg(windows)]
    {
        scan_drive_win(root, scan, active, on_progress);
    }
    #[cfg(not(windows))]
    {
        scan_drive_fallback(root, scan, active, on_progress);
    }
}

#[cfg(windows)]
fn scan_drive_win<F: FnMut(u64, u64)>(
    root: &Path,
    scan: &mut DriveScan,
    active: &mut Vec<usize>,
    on_progress: &mut F,
) {
    let key = norm_lower(root);
    let pushed = scan.enter(&key, active);
    // Collect child dirs during enumeration, recurse only after the enumeration
    // closure is dropped. Otherwise the closure's &mut scan borrow would alias
    // the recursive call's &mut scan borrow and fail to compile.
    let mut subdirs: Vec<std::path::PathBuf> = Vec::new();
    for_each_entry(root, |data| {
        if !is_dots(&data.cFileName) && !is_reparse(data.dwFileAttributes) {
            if is_dir(data.dwFileAttributes) {
                subdirs.push(root.join(from_wide(&data.cFileName)));
            } else {
                let size = file_size_u64(data);
                let pstr = root
                    .join(from_wide(&data.cFileName))
                    .to_string_lossy()
                    .into_owned();
                scan.collector.consider(pstr, size);
                for &i in &*active {
                    scan.sizes[i] = scan.sizes[i].saturating_add(size);
                    scan.counts[i] = scan.counts[i].saturating_add(1);
                }
            }
        }
    });
    for child in subdirs {
        scan_drive_win(&child, scan, active, on_progress);
    }
    if pushed {
        active.pop();
        let (done, total) = scan.note_done();
        on_progress(done, total);
    }
}

#[cfg(not(windows))]
fn scan_drive_fallback<F: FnMut(u64, u64)>(
    root: &Path,
    scan: &mut DriveScan,
    active: &mut Vec<usize>,
    on_progress: &mut F,
) {
    let key = norm_lower(root);
    let pushed = scan.enter(&key, active);
    let mut subdirs: Vec<std::path::PathBuf> = Vec::new();
    // Skip symlinks to avoid cycles; mirrors the Windows branch's reparse skip.
    if let Ok(rd) = std::fs::read_dir(root) {
        for entry in rd.flatten() {
            let ft = match entry.file_type() {
                Ok(t) => t,
                Err(_) => continue,
            };
            if ft.is_symlink() {
                continue;
            }
            if ft.is_dir() {
                subdirs.push(entry.path());
            } else if ft.is_file() {
                let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
                let pstr = entry.path().to_string_lossy().into_owned();
                scan.collector.consider(pstr, size);
                for &i in &*active {
                    scan.sizes[i] = scan.sizes[i].saturating_add(size);
                    scan.counts[i] = scan.counts[i].saturating_add(1);
                }
            }
        }
    }
    for child in subdirs {
        scan_drive_fallback(&child, scan, active, on_progress);
    }
    if pushed {
        active.pop();
        let (done, total) = scan.note_done();
        on_progress(done, total);
    }
}

/// Full C-drive scan: ONE walk of C:\ that simultaneously streams files into the
/// Top-N heap and accumulates each monitor dir's size via an active-set stack.
/// `auto_append_dirs` lets the caller inject parent dirs of the previous
/// snapshot's large files so emerging culprits stay tracked. `on_progress`
/// receives (monitors_done, monitors_total) as each monitor dir finishes.
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

    // Single pass over C:\: stream files into the Top-N heap AND accumulate
    // each monitor dir's size via an active-set stack. A file under a nested
    // monitor (e.g. C:\Windows\Temp) is charged to EVERY enclosing monitor
    // (C:\Windows AND C:\Windows\Temp), matching the old per-dir walks exactly.
    // Files outside any monitor dir (e.g. pagefile/hiberfil at C:\ root) still
    // feed Top-N. Replaces the old two-phase design that walked every monitor
    // dir for size, then re-walked all of C:\ for Top-N.
    let mut scan = DriveScan::new(&dirs, min_bytes, top_n as usize);
    let mut active: Vec<usize> = Vec::new();
    collect_drive_with_monitors(Path::new("C:\\"), &mut scan, &mut active, &mut on_progress);

    // Build monitor entries. C: monitors reached during the walk already carry
    // their tallies; unreached dirs (missing, access-blocked, or on another
    // drive) are resolved here and counted toward progress so the bar reaches
    // 100% instead of stalling.
    let mut monitor_entries: Vec<MonitorEntry> = Vec::with_capacity(dirs.len());
    for (i, raw) in dirs.iter().enumerate() {
        let (advice, advice_reason) = crate::classify::classify(raw);
        if scan.reached[i] {
            monitor_entries.push(MonitorEntry {
                path: raw.clone(),
                size_bytes: scan.sizes[i],
                file_count: scan.counts[i],
                exists: true,
                advice,
                advice_reason,
            });
        } else {
            let p = Path::new(raw);
            let exists = p.exists();
            let (size_bytes, file_count) = if exists && !is_on_c_drive(raw) {
                // Monitor on another drive: the C:\ walk never touched it.
                let s = walk_dir_fast(p);
                (s.size_bytes, s.file_count)
            } else {
                (0, 0)
            };
            monitor_entries.push(MonitorEntry {
                path: raw.clone(),
                size_bytes,
                file_count,
                exists,
                advice,
                advice_reason,
            });
            let (done, total) = scan.note_done();
            on_progress(done, total);
        }
    }
    monitor_entries.sort_by(|a, b| b.size_bytes.cmp(&a.size_bytes));

    let large = scan.collector.finish();

    MonitorSnapshot {
        timestamp: chrono::Local::now().to_rfc3339(),
        scan_type: "full".into(),
        drive_total,
        drive_used,
        monitor_dirs: monitor_entries,
        large_files: large,
    }
}

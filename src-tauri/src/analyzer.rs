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
    let mut out = Vec::new();
    #[cfg(windows)]
    {
        out = list_children_win(path);
    }
    #[cfg(not(windows))]
    {
        if let Ok(rd) = std::fs::read_dir(path) {
            for entry in rd.flatten() {
                let p = entry.path();
                let ft = match entry.file_type() { Ok(t) => t, Err(_) => continue };
                if ft.is_dir() {
                    let s = walk_dir_fast_fallback(&p);
                    out.push(MonitorEntry { path: p.to_string_lossy().into_owned(), size_bytes: s.size_bytes, file_count: s.file_count, exists: true });
                } else if ft.is_file() {
                    let sz = entry.metadata().map(|m| m.len()).unwrap_or(0);
                    out.push(MonitorEntry { path: p.to_string_lossy().into_owned(), size_bytes: sz, file_count: 1, exists: true });
                }
            }
        }
    }
    out.sort_by(|a, b| b.size_bytes.cmp(&a.size_bytes));
    out
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
}

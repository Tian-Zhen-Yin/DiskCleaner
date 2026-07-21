use crate::models::{CleanCategory, CleanResult, ScanResult};
use crate::paths::{paths_for, CleanTarget};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

pub fn scan_category(category: CleanCategory) -> ScanResult {
    let (size_bytes, file_count) = scan_targets(paths_for(category));
    ScanResult {
        category,
        size_bytes,
        file_count,
    }
}

fn scan_targets(targets: Vec<CleanTarget>) -> (u64, u64) {
    let mut size_bytes: u64 = 0;
    let mut file_count: u64 = 0;
    for target in targets {
        match target {
            CleanTarget::Dir(root) | CleanTarget::RecycleBin(root) => {
                if !root.exists() {
                    continue;
                }
                for entry in WalkDir::new(&root)
                    .follow_links(false)
                    .into_iter()
                    .filter_map(|e| e.ok())
                {
                    if entry.file_type().is_file() {
                        if let Ok(md) = entry.metadata() {
                            size_bytes = size_bytes.saturating_add(md.len());
                            file_count = file_count.saturating_add(1);
                        }
                    }
                }
            }
            CleanTarget::Files { dir, pattern } => {
                for path in match_files(&dir, &pattern) {
                    if let Ok(md) = std::fs::metadata(&path) {
                        size_bytes = size_bytes.saturating_add(md.len());
                        file_count = file_count.saturating_add(1);
                    }
                }
            }
            CleanTarget::File(path) => {
                if let Ok(md) = std::fs::metadata(&path) {
                    size_bytes = size_bytes.saturating_add(md.len());
                    file_count = file_count.saturating_add(1);
                }
            }
        }
    }
    (size_bytes, file_count)
}

pub fn clean_category(category: CleanCategory) -> CleanResult {
    clean_category_with_progress(category, |_, _| {})
}

pub fn clean_category_with_progress<F>(category: CleanCategory, mut on_progress: F) -> CleanResult
where
    F: FnMut(u64, u64),
{
    let (freed_bytes, removed_count, errors) = clean_targets(paths_for(category), &mut on_progress);
    CleanResult {
        category,
        freed_bytes,
        removed_count,
        errors,
    }
}

fn clean_targets<F>(
    targets: Vec<CleanTarget>,
    on_progress: &mut F,
) -> (u64, u64, Vec<String>)
where
    F: FnMut(u64, u64),
{
    let mut freed_bytes: u64 = 0;
    let mut removed_count: u64 = 0;
    let mut errors: Vec<String> = Vec::new();

    for target in targets {
        match target {
            CleanTarget::Dir(root) => {
                if !root.exists() {
                    continue;
                }
                clean_dir_contents(
                    &root,
                    &mut freed_bytes,
                    &mut removed_count,
                    &mut errors,
                    on_progress,
                );
            }
            CleanTarget::Files { dir, pattern } => {
                for path in match_files(&dir, &pattern) {
                    let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                    match std::fs::remove_file(&path) {
                        Ok(_) => {
                            freed_bytes = freed_bytes.saturating_add(size);
                            removed_count = removed_count.saturating_add(1);
                            on_progress(freed_bytes, removed_count);
                        }
                        Err(e) => push_error(&mut errors, &path, e),
                    }
                }
            }
            CleanTarget::File(path) => {
                let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                match std::fs::remove_file(&path) {
                    Ok(_) => {
                        freed_bytes = freed_bytes.saturating_add(size);
                        removed_count = removed_count.saturating_add(1);
                        on_progress(freed_bytes, removed_count);
                    }
                    Err(e) => push_error(&mut errors, &path, e),
                }
            }
            CleanTarget::RecycleBin(root) => {
                // The Win32 API empties the bin atomically, so per-file progress
                // isn't available; report the pre-walked size as freed bytes.
                let size = directory_size(&root);
                match empty_recycle_bin() {
                    Ok(emptied) => {
                        if emptied {
                            freed_bytes = freed_bytes.saturating_add(size);
                            removed_count = removed_count.saturating_add(1);
                            on_progress(freed_bytes, removed_count);
                        }
                    }
                    Err(e) => {
                        if errors.len() < 50 {
                            errors.push(format!("SHEmptyRecycleBinW: {}", e));
                        }
                    }
                }
            }
        }
    }

    (freed_bytes, removed_count, errors)
}

fn match_files(dir: &Path, pattern: &str) -> Vec<PathBuf> {
    let full = format!("{}\\{}", dir.to_string_lossy(), pattern);
    match glob::glob(&full) {
        Ok(it) => it.filter_map(|e| e.ok()).collect(),
        Err(_) => Vec::new(),
    }
}

fn push_error(errors: &mut Vec<String>, path: &Path, e: std::io::Error) {
    if errors.len() < 50 {
        errors.push(format!("remove {}: {}", path.display(), e));
    }
}

#[cfg(windows)]
fn empty_recycle_bin() -> Result<bool, String> {
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::Shell::SHEmptyRecycleBinW;

    let root: Vec<u16> = "C:\\".encode_utf16().chain(std::iter::once(0)).collect();
    // SHERB_NOCONFIRMATION | SHERB_NOPROGRESSUI | SHERB_NOSOUND
    const FLAGS: u32 = 0x1 | 0x2 | 0x4;
    match unsafe {
        SHEmptyRecycleBinW(HWND(std::ptr::null_mut()), PCWSTR(root.as_ptr()), FLAGS)
    } {
        Ok(_) => Ok(true),
        Err(e) => {
            // SHEmptyRecycleBinW reports a non-fatal error when the bin is already
            // empty (E_FAIL / 0x80004005); treat that as "nothing to free".
            let msg = format!("{:?}", e);
            if msg.contains("0x80004005") || msg.contains("E_FAIL") {
                Ok(false)
            } else {
                Err(msg)
            }
        }
    }
}

#[cfg(not(windows))]
fn empty_recycle_bin() -> Result<bool, String> {
    Err("仅支持 Windows".to_string())
}

fn clean_dir_contents<F>(
    dir: &Path,
    freed_bytes: &mut u64,
    removed_count: &mut u64,
    errors: &mut Vec<String>,
    on_progress: &mut F,
) where
    F: FnMut(u64, u64),
{
    let rd = match std::fs::read_dir(dir) {
        Ok(r) => r,
        Err(e) => {
            if errors.len() < 50 {
                errors.push(format!("read_dir {}: {}", dir.display(), e));
            }
            return;
        }
    };

    for entry in rd.flatten() {
        let path = entry.path();
        let file_type = match entry.file_type() {
            Ok(t) => t,
            Err(_) => continue,
        };

        if file_type.is_dir() {
            let dir_size = directory_size(&path);
            match std::fs::remove_dir_all(&path) {
                Ok(_) => {
                    *freed_bytes = freed_bytes.saturating_add(dir_size);
                    *removed_count = removed_count.saturating_add(1);
                    on_progress(*freed_bytes, *removed_count);
                }
                Err(_) => {
                    clean_dir_contents(
                        path.as_path(),
                        freed_bytes,
                        removed_count,
                        errors,
                        on_progress,
                    );
                }
            }
        } else {
            let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
            match std::fs::remove_file(&path) {
                Ok(_) => {
                    *freed_bytes = freed_bytes.saturating_add(size);
                    *removed_count = removed_count.saturating_add(1);
                    on_progress(*freed_bytes, *removed_count);
                }
                Err(e) => push_error(errors, &path, e),
            }
        }
    }
}

fn directory_size(dir: &Path) -> u64 {
    let mut total: u64 = 0;
    for entry in WalkDir::new(dir)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if entry.file_type().is_file() {
            if let Ok(md) = entry.metadata() {
                total = total.saturating_add(md.len());
            }
        }
    }
    total
}

#[cfg(windows)]
pub fn get_disk_info(drive: &str) -> Result<crate::models::DiskInfo, String> {
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;

    let mut path = drive.trim_end_matches('\\').to_string();
    if !path.ends_with(':') {
        path.push(':');
    }
    path.push('\\');
    let wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();

    let mut free_to_caller: u64 = 0;
    let mut total: u64 = 0;
    let mut total_free: u64 = 0;

    let result = unsafe {
        GetDiskFreeSpaceExW(
            PCWSTR(wide.as_ptr()),
            Some(&mut free_to_caller),
            Some(&mut total),
            Some(&mut total_free),
        )
    };

    if result.is_err() {
        return Err(format!("GetDiskFreeSpaceExW failed for {}", drive));
    }

    Ok(crate::models::DiskInfo {
        total_bytes: total,
        free_bytes: total_free,
        used_bytes: total.saturating_sub(total_free),
    })
}

#[cfg(not(windows))]
pub fn get_disk_info(_drive: &str) -> Result<crate::models::DiskInfo, String> {
    Err("仅支持 Windows".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    fn make_tmp(prefix: &str) -> PathBuf {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let mut p = std::env::temp_dir();
        p.push(format!("dct_{}_{}_{}", prefix, nanos, n));
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    fn write(path: &Path, bytes: &[u8]) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, bytes).unwrap();
    }

    fn cleanup(p: &Path) {
        let _ = std::fs::remove_dir_all(p);
    }

    #[test]
    fn dir_target_clears_contents_but_keeps_root() {
        let root = make_tmp("dir");
        write(&root.join("a.txt"), b"hello"); // 5
        write(&root.join("sub").join("b.txt"), b"world!!"); // 7

        let (size, count) = scan_targets(vec![CleanTarget::Dir(root.clone())]);
        assert_eq!(size, 12);
        assert_eq!(count, 2);

        let mut progress = |_, _| {};
        let (freed, removed, errors) =
            clean_targets(vec![CleanTarget::Dir(root.clone())], &mut progress);
        assert_eq!(freed, 12);
        assert_eq!(removed, 2);
        assert!(errors.is_empty(), "{:?}", errors);

        // Root survives, contents gone.
        assert!(root.exists());
        assert!(root.read_dir().unwrap().next().is_none());

        cleanup(&root);
    }

    #[test]
    fn files_target_deletes_only_matching_files() {
        let dir = make_tmp("files");
        write(&dir.join("a.log"), b"AAA"); // 3
        write(&dir.join("b.log"), b"BBBB"); // 4
        write(&dir.join("c.txt"), b"CCCCC"); // 5, must survive

        let (size, count) = scan_targets(vec![CleanTarget::Files {
            dir: dir.clone(),
            pattern: "*.log".to_string(),
        }]);
        assert_eq!(size, 7);
        assert_eq!(count, 2);

        let mut progress = |_, _| {};
        let (freed, removed, errors) = clean_targets(
            vec![CleanTarget::Files {
                dir: dir.clone(),
                pattern: "*.log".to_string(),
            }],
            &mut progress,
        );
        assert_eq!(freed, 7);
        assert_eq!(removed, 2);
        assert!(errors.is_empty(), "{:?}", errors);
        assert!(!dir.join("a.log").exists());
        assert!(!dir.join("b.log").exists());
        assert!(dir.join("c.txt").exists());

        cleanup(&dir);
    }

    #[test]
    fn file_target_deletes_single_file() {
        let dir = make_tmp("file");
        let f = dir.join("Memory.dmp");
        write(&f, b"dumpy"); // 5

        let (size, count) = scan_targets(vec![CleanTarget::File(f.clone())]);
        assert_eq!(size, 5);
        assert_eq!(count, 1);

        let mut progress = |_, _| {};
        let (freed, removed, _errors) =
            clean_targets(vec![CleanTarget::File(f.clone())], &mut progress);
        assert_eq!(freed, 5);
        assert_eq!(removed, 1);
        assert!(!f.exists());

        cleanup(&dir);
    }
}

use crate::models::CleanCategory;
use std::path::PathBuf;

/// A single cleanable target. `paths_for` returns a list of these per category,
/// and the cleaner dispatches on the variant so we can safely handle whole
/// directories, glob-matched files, single files, and the recycle bin.
pub enum CleanTarget {
    /// Recursively clear the directory's contents, keeping the directory itself.
    Dir(PathBuf),
    /// Delete files inside `dir` matching the glob `pattern`; keep everything else.
    Files { dir: PathBuf, pattern: String },
    /// Delete a single file.
    File(PathBuf),
    /// Empty the recycle bin for the drive that contains `root` (C:\ by default).
    RecycleBin(PathBuf),
}

pub fn paths_for(category: CleanCategory) -> Vec<CleanTarget> {
    match category {
        CleanCategory::WindowsTemp => windows_temp_paths(),
        CleanCategory::BrowserCache => browser_cache_paths(),
        CleanCategory::WindowsUpdate => windows_update_paths(),
        CleanCategory::RecycleBin => recycle_bin_paths(),
        CleanCategory::Thumbnails => thumbnail_paths(),
        CleanCategory::ErrorReports => error_report_paths(),
        CleanCategory::MemoryDumps => memory_dump_paths(),
        CleanCategory::WindowsLogs => windows_log_paths(),
        CleanCategory::DeliveryOptimization => delivery_optimization_paths(),
        CleanCategory::Prefetch => prefetch_paths(),
        CleanCategory::DevCaches => dev_cache_paths(),
    }
}

fn system_root() -> String {
    std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".to_string())
}

fn existing(path: PathBuf) -> Option<PathBuf> {
    if path.exists() {
        Some(path)
    } else {
        None
    }
}

fn windows_temp_paths() -> Vec<CleanTarget> {
    let mut out = Vec::new();
    if let Ok(t) = std::env::var("TEMP") {
        let p = PathBuf::from(t);
        if p.exists() {
            out.push(CleanTarget::Dir(p));
        }
    }
    if let Ok(t) = std::env::var("TMP") {
        let p = PathBuf::from(t);
        if p.exists() && !out.iter().any(|x| matches!(x, CleanTarget::Dir(d) if *d == p)) {
            out.push(CleanTarget::Dir(p));
        }
    }
    let sys = system_root();
    if let Some(c) = existing(PathBuf::from(format!("{sys}\\Temp"))) {
        out.push(CleanTarget::Dir(c));
    }
    if let Some(local) = dirs::data_local_dir() {
        if let Some(c) = existing(local.join("CrashDumps")) {
            out.push(CleanTarget::Dir(c));
        }
    }
    out
}

fn browser_cache_paths() -> Vec<CleanTarget> {
    let mut out = Vec::new();
    let local = match dirs::data_local_dir() {
        Some(p) => p,
        None => return out,
    };

    // Chrome / Edge per-profile cache directories. All auto-rebuild on next launch.
    let chromium_subs = [
        "Cache",
        "Code Cache",
        "GPUCache",
        "Service Worker\\CacheStorage",
        "ShaderCache",
        "GrShaderCache",
        "DawnCache",
        "DawnGraphiteCache",
    ];

    for base in [
        local.join("Google").join("Chrome").join("User Data"),
        local.join("Microsoft").join("Edge").join("User Data"),
    ] {
        if base.exists() {
            for profile in profile_dirs(&base) {
                for sub in chromium_subs.iter() {
                    if let Some(c) = existing(profile.join(sub)) {
                        out.push(CleanTarget::Dir(c));
                    }
                }
            }
        }
    }

    if let Some(roaming) = dirs::data_dir() {
        let ff = roaming.join("Mozilla").join("Firefox").join("Profiles");
        if ff.exists() {
            if let Ok(rd) = std::fs::read_dir(&ff) {
                for entry in rd.flatten() {
                    for sub in ["cache2", "startupCache", "shaderCache"].iter() {
                        if let Some(c) = existing(entry.path().join(sub)) {
                            out.push(CleanTarget::Dir(c));
                        }
                    }
                }
            }
        }
    }

    out
}

fn profile_dirs(base: &std::path::Path) -> Vec<PathBuf> {
    let mut profiles = Vec::new();
    profiles.push(base.join("Default"));
    if let Ok(rd) = std::fs::read_dir(base) {
        for entry in rd.flatten() {
            if let Some(name) = entry.file_name().to_str() {
                if name.starts_with("Profile ") {
                    profiles.push(entry.path());
                }
            }
        }
    }
    profiles.into_iter().filter(|p| p.exists()).collect()
}

fn windows_update_paths() -> Vec<CleanTarget> {
    let sys = system_root();
    vec![
        PathBuf::from(format!("{sys}\\SoftwareDistribution\\Download")),
        PathBuf::from(format!("{sys}\\Logs\\CBS")),
        PathBuf::from(format!("{sys}\\Logs\\WindowsUpdate")),
    ]
    .into_iter()
    .filter(|p| p.exists())
    .map(CleanTarget::Dir)
    .collect()
}

fn recycle_bin_paths() -> Vec<CleanTarget> {
    let p = PathBuf::from("C:\\$Recycle.Bin");
    if p.exists() {
        vec![CleanTarget::RecycleBin(p)]
    } else {
        Vec::new()
    }
}

fn thumbnail_paths() -> Vec<CleanTarget> {
    let mut out = Vec::new();
    let local = match dirs::data_local_dir() {
        Some(p) => p,
        None => return out,
    };
    let explorer = local.join("Microsoft").join("Windows").join("Explorer");
    if explorer.exists() {
        out.push(CleanTarget::Files {
            dir: explorer.clone(),
            pattern: "thumbcache_*.db".to_string(),
        });
        out.push(CleanTarget::Files {
            dir: explorer,
            pattern: "iconcache_*.db".to_string(),
        });
    }
    if let Some(c) = existing(local.join("IconCache.db")) {
        out.push(CleanTarget::File(c));
    }
    out
}

fn error_report_paths() -> Vec<CleanTarget> {
    let mut out = Vec::new();
    let program_data =
        std::env::var("ProgramData").unwrap_or_else(|_| "C:\\ProgramData".to_string());
    if let Some(c) = existing(PathBuf::from(format!(
        "{program_data}\\Microsoft\\Windows\\WER"
    ))) {
        out.push(CleanTarget::Dir(c));
    }
    if let Some(local) = dirs::data_local_dir() {
        if let Some(c) = existing(local.join("Microsoft").join("Windows").join("WER")) {
            out.push(CleanTarget::Dir(c));
        }
    }
    out
}

fn memory_dump_paths() -> Vec<CleanTarget> {
    let sys = system_root();
    let mut out = Vec::new();
    if let Some(c) = existing(PathBuf::from(format!("{sys}\\Memory.dmp"))) {
        out.push(CleanTarget::File(c));
    }
    for sub in ["Minidump", "LiveKernelReports"] {
        if let Some(c) = existing(PathBuf::from(format!("{sys}\\{sub}"))) {
            out.push(CleanTarget::Dir(c));
        }
    }
    out
}

fn windows_log_paths() -> Vec<CleanTarget> {
    let sys = system_root();
    let mut out = Vec::new();
    if let Some(c) = existing(PathBuf::from(format!("{sys}\\Panther"))) {
        out.push(CleanTarget::Dir(c));
    }
    let inf = PathBuf::from(format!("{sys}\\inf"));
    if inf.exists() {
        out.push(CleanTarget::Files {
            dir: inf,
            pattern: "setupapi*.log".to_string(),
        });
    }
    let logs = PathBuf::from(format!("{sys}\\Logs"));
    if let Ok(rd) = std::fs::read_dir(&logs) {
        for entry in rd.flatten() {
            // CBS / WindowsUpdate belong to the WindowsUpdate category; skip them
            // here to avoid double-counting between the two categories.
            let skip = entry
                .file_name()
                .to_str()
                .map(|n| {
                    n.eq_ignore_ascii_case("CBS") || n.eq_ignore_ascii_case("WindowsUpdate")
                })
                .unwrap_or(false);
            if skip {
                continue;
            }
            let p = entry.path();
            if p.is_dir() {
                out.push(CleanTarget::Dir(p));
            }
        }
    }
    out
}

fn delivery_optimization_paths() -> Vec<CleanTarget> {
    let sys = system_root();
    if let Some(c) = existing(PathBuf::from(format!(
        "{sys}\\SoftwareDistribution\\DeliveryOptimization"
    ))) {
        vec![CleanTarget::Dir(c)]
    } else {
        Vec::new()
    }
}

fn prefetch_paths() -> Vec<CleanTarget> {
    let sys = system_root();
    if let Some(c) = existing(PathBuf::from(format!("{sys}\\Prefetch"))) {
        vec![CleanTarget::Dir(c)]
    } else {
        Vec::new()
    }
}

fn dev_cache_paths() -> Vec<CleanTarget> {
    let mut out = Vec::new();
    let local = match dirs::data_local_dir() {
        Some(p) => p,
        None => return out,
    };
    for sub in ["npm-cache", "pip\\Cache", "NuGet\\v3-cache"] {
        if let Some(c) = existing(local.join(sub)) {
            out.push(CleanTarget::Dir(c));
        }
    }
    if let Some(home) = dirs::home_dir() {
        let cargo = home.join(".cargo").join("registry");
        for sub in ["cache", "src"] {
            if let Some(c) = existing(cargo.join(sub)) {
                out.push(CleanTarget::Dir(c));
            }
        }
    }
    out
}

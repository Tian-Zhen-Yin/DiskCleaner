use crate::cleaner::clean_category;
use crate::history::HistoryStore;
use crate::models::{CleanCategory, ScheduleConfig, ScheduleFrequency};
use chrono::{Datelike, Duration, Local, NaiveTime, TimeZone};
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

pub struct Scheduler {
    handle: Mutex<Option<JoinHandle<()>>>,
}

impl Scheduler {
    pub fn new() -> Self {
        Self {
            handle: Mutex::new(None),
        }
    }

    pub async fn reload(
        self: &Arc<Self>,
        cfg: ScheduleConfig,
        history: Arc<HistoryStore>,
    ) {
        let mut guard = self.handle.lock().await;
        if let Some(h) = guard.take() {
            h.abort();
        }
        if !cfg.enabled || cfg.categories.is_empty() {
            return;
        }
        let categories = cfg.categories.clone();
        let frequency = cfg.frequency;
        let hour = cfg.hour;
        let minute = cfg.minute;

        let handle = tokio::spawn(async move {
            loop {
                let wait = duration_until_next(frequency, hour, minute);
                println!("[scheduler] next run in {:?}", wait);
                tokio::time::sleep(wait).await;
                println!("[scheduler] running scheduled cleanup");
                let mut results = Vec::new();
                for c in &categories {
                    let result = clean_category(*c);
                    println!(
                        "[scheduler] {:?}: freed={} removed={} errors={}",
                        c,
                        result.freed_bytes,
                        result.removed_count,
                        result.errors.len()
                    );
                    results.push(result);
                }
                let entries = crate::history::build_entries_from_results(&results);
                let _ = history.append(entries, "scheduler");
            }
        });
        *guard = Some(handle);
    }
}

fn duration_until_next(freq: ScheduleFrequency, hour: u32, minute: u32) -> std::time::Duration {
    let now = Local::now();
    let target_time = NaiveTime::from_hms_opt(hour.min(23), minute.min(59), 0)
        .unwrap_or_else(|| NaiveTime::from_hms_opt(3, 0, 0).unwrap());

    let mut target_date = now.date_naive();
    let mut target = Local
        .from_local_datetime(&target_date.and_time(target_time))
        .single()
        .unwrap_or(now);

    if target <= now {
        target_date = target_date.succ_opt().unwrap_or(target_date);
        target = Local
            .from_local_datetime(&target_date.and_time(target_time))
            .single()
            .unwrap_or(now + Duration::days(1));
    }

    if freq == ScheduleFrequency::Weekly {
        let target_weekday = now.weekday();
        while target.weekday() != target_weekday {
            target = target + Duration::days(1);
        }
        if target <= now {
            target = target + Duration::days(7);
        }
    }

    let diff = target - now;
    let secs = diff.num_seconds().max(60) as u64;
    std::time::Duration::from_secs(secs)
}

pub fn run_cli_clean(
    categories: &[CleanCategory],
) -> Vec<(CleanCategory, u64, u64, u64)> {
    println!("[cli] starting scheduled cleanup ({} items)", categories.len());
    let mut summary = Vec::new();
    for c in categories {
        let r = clean_category(*c);
        println!(
            "[cli] {:?}: freed={} bytes, removed={} files, errors={}",
            c, r.freed_bytes, r.removed_count, r.errors.len()
        );
        summary.push((*c, r.freed_bytes, r.removed_count, r.errors.len() as u64));
    }
    println!("[cli] done");
    summary
}

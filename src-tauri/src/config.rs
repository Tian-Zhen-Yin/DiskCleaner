use crate::models::ScheduleConfig;
use std::path::PathBuf;
use std::sync::Mutex;

pub struct ConfigStore {
    path: PathBuf,
    inner: Mutex<ScheduleConfig>,
}

impl ConfigStore {
    pub fn new() -> Self {
        let path = config_path();
        let cfg = load_from_disk(&path).unwrap_or_default();
        Self {
            path,
            inner: Mutex::new(cfg),
        }
    }

    pub fn get(&self) -> ScheduleConfig {
        self.inner.lock().unwrap().clone()
    }

    pub fn set(&self, cfg: ScheduleConfig) -> Result<(), String> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let json = serde_json::to_string_pretty(&cfg).map_err(|e| e.to_string())?;
        std::fs::write(&self.path, json).map_err(|e| e.to_string())?;
        *self.inner.lock().unwrap() = cfg;
        Ok(())
    }
}

fn load_from_disk(path: &PathBuf) -> Option<ScheduleConfig> {
    let s = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&s).ok()
}

pub fn config_path() -> PathBuf {
    let base = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
    base.join("DiskClearTool").join("config.json")
}

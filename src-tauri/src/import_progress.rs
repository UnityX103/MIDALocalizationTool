use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use serde_json::json;
use tauri::Emitter;

pub struct ImportProgress {
    pub id: String,
    cancelled: AtomicBool,
    last_event: Mutex<Instant>,
}

impl ImportProgress {
    pub fn new() -> Self {
        Self { id: uuid::Uuid::new_v4().to_string(), cancelled: AtomicBool::new(false),
            last_event: Mutex::new(Instant::now() - Duration::from_secs(1)) }
    }
    pub fn cancel(&self) { self.cancelled.store(true, Ordering::Relaxed); }
    pub fn report(&self, app: &tauri::AppHandle, fraction: f64, phase: &str) -> Result<(), String> {
        if self.cancelled.load(Ordering::Relaxed) { return Err("导入已取消".into()); }
        let mut last = self.last_event.lock().map_err(|error| error.to_string())?;
        if last.elapsed() >= Duration::from_millis(100) || fraction >= 1.0 {
            let _ = app.emit("mida-import-progress", json!({"requestId":self.id,"fraction":fraction,"phase":phase}));
            *last = Instant::now();
        }
        Ok(())
    }
}

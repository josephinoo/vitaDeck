use std::fs::{File, OpenOptions};
use std::io::Write;
use std::sync::{Mutex, OnceLock};

fn log_file() -> &'static Mutex<Option<File>> {
    static FILE: OnceLock<Mutex<Option<File>>> = OnceLock::new();
    FILE.get_or_init(|| {
        let _ = std::fs::create_dir_all("ux0:data/VitaDeck");
        let path = "ux0:data/VitaDeck/app.log";
        if std::fs::metadata(path).is_ok_and(|meta| meta.len() > 128 * 1024) {
            let _ = std::fs::remove_file("ux0:data/VitaDeck/app.previous.log");
            let _ = std::fs::rename(path, "ux0:data/VitaDeck/app.previous.log");
        }
        let file = OpenOptions::new().create(true).append(true).open(path).ok();
        Mutex::new(file)
    })
}

fn health_context() -> &'static Mutex<String> {
    static CONTEXT: OnceLock<Mutex<String>> = OnceLock::new();
    CONTEXT.get_or_init(|| Mutex::new("health context unavailable".to_owned()))
}

pub fn set_health_context(context: String) {
    if let Ok(mut current) = health_context().lock() {
        *current = context;
    }
}

pub fn current_health_context() -> String {
    health_context()
        .lock()
        .map(|context| context.clone())
        .unwrap_or_else(|_| "health context lock poisoned".to_owned())
}

pub fn log(msg: &str) {
    if let Ok(mut guard) = log_file().lock() {
        if let Some(file) = guard.as_mut() {
            let _ = writeln!(file, "[LOG] {}", msg);
        }
    }
    #[cfg(not(target_os = "vita"))]
    println!("[LOG] {}", msg);
}

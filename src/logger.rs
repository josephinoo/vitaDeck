use std::fs::{File, OpenOptions};
use std::io::Write;
use std::sync::{Mutex, OnceLock};

fn log_file() -> &'static Mutex<Option<File>> {
    static FILE: OnceLock<Mutex<Option<File>>> = OnceLock::new();
    FILE.get_or_init(|| {
        let _ = std::fs::create_dir_all("ux0:data/VitaDeck");
        let file = OpenOptions::new().create(true).append(true).open("ux0:data/VitaDeck/app.log").ok();
        Mutex::new(file)
    })
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

//! Crash-safe session marker. An uncleared marker on startup means the previous
//! process did not finish a normal shutdown, so network-heavy work is deferred.

const DATA_DIR: &str = "ux0:data/VitaDeck";
const SESSION_FILE: &str = "ux0:data/VitaDeck/session.lock";

pub fn begin() -> bool {
    let previous_unclean = std::path::Path::new(SESSION_FILE).exists();
    let _ = std::fs::create_dir_all(DATA_DIR);
    let _ = std::fs::write(SESSION_FILE, b"running\n");
    previous_unclean
}

pub fn finish_cleanly() {
    let _ = std::fs::remove_file(SESSION_FILE);
}

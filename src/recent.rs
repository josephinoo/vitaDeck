use std::fs::File;
use std::io::{Read, Write};

const DATA_DIR: &str = "ux0:data/VitaDeck/";
const RECENT_FILE: &str = "ux0:data/VitaDeck/recent.txt";
const MAX_ENTRIES: usize = 30;

pub struct RecentlyPlayed {
    title_ids: Vec<String>,
}

impl RecentlyPlayed {
    pub fn load() -> Self {
        let mut title_ids = Vec::new();
        if let Ok(mut file) = File::open(RECENT_FILE) {
            let mut text = String::new();
            if file.read_to_string(&mut text).is_ok() {
                title_ids = text.lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect();
            }
        }
        RecentlyPlayed { title_ids }
    }

    pub fn save(&self) {
        let _ = std::fs::create_dir_all(DATA_DIR);
        let Ok(mut file) = File::create(RECENT_FILE) else { return };
        for title_id in &self.title_ids {
            let _ = writeln!(file, "{}", title_id);
        }
    }

    pub fn touch(&mut self, title_id: &str) {
        self.title_ids.retain(|id| id != title_id);
        self.title_ids.insert(0, title_id.to_string());
        self.title_ids.truncate(MAX_ENTRIES);
        self.save();
    }

    pub fn order(&self) -> &[String] {
        &self.title_ids
    }
}

//! Local, launcher-owned game statistics. VitaDeck exits when it launches a
//! title, so these values deliberately represent launches, not guessed playtime.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

const DATA_DIR: &str = "ux0:data/VitaDeck";
const STATS_FILE: &str = "ux0:data/VitaDeck/game_stats.json";

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct GameStats {
    #[serde(default)]
    pub launch_count: u32,
    #[serde(default)]
    pub last_launched_at: u64,
}

#[derive(Default, Serialize, Deserialize)]
pub struct GameStatsStore {
    #[serde(default)]
    entries: HashMap<String, GameStats>,
}

impl GameStatsStore {
    pub fn load() -> Self {
        std::fs::read(STATS_FILE)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    pub fn get(&self, title_id: &str) -> Option<&GameStats> {
        self.entries.get(&normalize_id(title_id))
    }

    pub fn record_launch(&mut self, title_id: &str, timestamp: u64) {
        self.update_launch(title_id, timestamp);
        self.save();
    }

    fn update_launch(&mut self, title_id: &str, timestamp: u64) {
        let entry = self.entries.entry(normalize_id(title_id)).or_default();
        entry.launch_count = entry.launch_count.saturating_add(1);
        entry.last_launched_at = timestamp;
    }

    fn save(&self) {
        let _ = std::fs::create_dir_all(DATA_DIR);
        let Ok(bytes) = serde_json::to_vec(self) else { return };
        let temp = format!("{STATS_FILE}.part");
        if std::fs::write(&temp, bytes).is_ok() {
            let _ = std::fs::rename(temp, STATS_FILE);
        }
    }
}

fn normalize_id(title_id: &str) -> String {
    title_id.trim().to_ascii_uppercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launch_stats_are_normalized_and_monotonic() {
        let mut store = GameStatsStore::default();
        store.update_launch(" npug30038 ", 10);
        store.update_launch("NPUG30038", 20);
        assert_eq!(store.get("npug30038"), Some(&GameStats { launch_count: 2, last_launched_at: 20 }));
    }
}

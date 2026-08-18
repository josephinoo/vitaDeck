use crate::scanner::Game;
use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;
use std::time::SystemTime;

const SUBFOLDERS: &[&str] = &["PSVita", "PSP", "PS1"];
const COVERS_DIR: &str = "ux0:data/VitaDeck/COVERS";
const HERO_DIR: &str = "ux0:data/VitaDeck/HERO";
const LOGO_DIR: &str = "ux0:data/VitaDeck/LOGO";
const MUSIC_DIR: &str = "ux0:data/VitaDeck/MUSIC";

#[derive(Debug, Clone, Default)]
pub struct CacheStats {
    pub covers_bytes: u64,
    pub hero_bytes: u64,
    pub logo_bytes: u64,
    pub music_bytes: u64,
    pub total_bytes: u64,
    pub orphan_count: usize,
    pub orphan_bytes: u64,
}

#[derive(Debug)]
struct CachedFileInfo {
    path: PathBuf,
    size: u64,
    modified: SystemTime,
    stem_lower: String,
}

pub fn format_bytes(bytes: u64) -> String {
    if bytes >= 1_073_741_824 {
        format!("{:.1} GB", bytes as f64 / 1_073_741_824.0)
    } else if bytes >= 1_048_576 {
        format!("{:.1} MB", bytes as f64 / 1_048_576.0)
    } else if bytes >= 1024 {
        format!("{:.0} KB", bytes as f64 / 1024.0)
    } else {
        format!("{bytes} B")
    }
}

fn build_valid_keys(games: &[Game]) -> HashSet<String> {
    let mut set = HashSet::with_capacity(games.len() * 3);
    for g in games {
        set.insert(g.title_id.to_lowercase());
        set.insert(g.art_key.to_lowercase());
        set.insert(g.title.to_lowercase());
        let clean_title = g.title.replace(['/', '\\', ':', '*', '?', '"', '<', '>', '|'], "_").to_lowercase();
        set.insert(clean_title);
    }
    set
}

fn list_category_files(base_dir: &str) -> Vec<CachedFileInfo> {
    let mut result = Vec::new();
    for sub in SUBFOLDERS {
        let dir_path = format!("{}/{}", base_dir, sub);
        if let Ok(entries) = fs::read_dir(&dir_path) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() {
                    let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
                    let modified = entry.metadata().and_then(|m| m.modified()).unwrap_or(SystemTime::UNIX_EPOCH);
                    let stem_lower = path
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .map(|s| s.to_lowercase())
                        .unwrap_or_default();
                    result.push(CachedFileInfo {
                        path,
                        size,
                        modified,
                        stem_lower,
                    });
                }
            }
        }
    }
    result
}

pub fn compute_cache_stats(games: &[Game]) -> CacheStats {
    let valid_keys = build_valid_keys(games);
    let mut stats = CacheStats::default();

    let check_files = |base_dir: &str, byte_counter: &mut u64, orphan_cnt: &mut usize, orphan_b: &mut u64| {
        for file in list_category_files(base_dir) {
            *byte_counter += file.size;
            if !valid_keys.contains(&file.stem_lower) {
                *orphan_cnt += 1;
                *orphan_b += file.size;
            }
        }
    };

    check_files(COVERS_DIR, &mut stats.covers_bytes, &mut stats.orphan_count, &mut stats.orphan_bytes);
    check_files(HERO_DIR, &mut stats.hero_bytes, &mut stats.orphan_count, &mut stats.orphan_bytes);
    check_files(LOGO_DIR, &mut stats.logo_bytes, &mut stats.orphan_count, &mut stats.orphan_bytes);
    check_files(MUSIC_DIR, &mut stats.music_bytes, &mut stats.orphan_count, &mut stats.orphan_bytes);

    stats.total_bytes = stats.covers_bytes + stats.hero_bytes + stats.logo_bytes + stats.music_bytes;
    stats
}

pub fn clean_orphaned_cache(games: &[Game]) -> (usize, u64) {
    let valid_keys = build_valid_keys(games);
    let mut removed_count = 0;
    let mut bytes_freed = 0;

    let dirs = [COVERS_DIR, HERO_DIR, LOGO_DIR, MUSIC_DIR];
    for base in dirs {
        for file in list_category_files(base) {
            if !valid_keys.contains(&file.stem_lower) {
                if fs::remove_file(&file.path).is_ok() {
                    removed_count += 1;
                    bytes_freed += file.size;
                }
            }
        }
    }

    (removed_count, bytes_freed)
}

pub fn purge_music() -> (usize, u64) {
    let mut removed_count = 0;
    let mut bytes_freed = 0;

    for file in list_category_files(MUSIC_DIR) {
        if fs::remove_file(&file.path).is_ok() {
            removed_count += 1;
            bytes_freed += file.size;
        }
    }

    (removed_count, bytes_freed)
}

pub fn purge_all_cache() -> (usize, u64) {
    let mut removed_count = 0;
    let mut bytes_freed = 0;

    let dirs = [COVERS_DIR, HERO_DIR, LOGO_DIR, MUSIC_DIR];
    for base in dirs {
        for file in list_category_files(base) {
            if fs::remove_file(&file.path).is_ok() {
                removed_count += 1;
                bytes_freed += file.size;
            }
        }
    }

    (removed_count, bytes_freed)
}

pub fn enforce_cache_budget(games: &[Game], budget_mb: u32) -> u64 {
    if budget_mb == 0 {
        return 0; 
    }

    let budget_bytes = budget_mb as u64 * 1024 * 1024;
    let stats = compute_cache_stats(games);
    if stats.total_bytes <= budget_bytes {
        return 0;
    }

    let mut current_bytes = stats.total_bytes;
    let mut total_freed = 0;

    let mut evict_from_dir = |base_dir: &str| {
        if current_bytes <= budget_bytes {
            return;
        }
        let mut files = list_category_files(base_dir);
        files.sort_by_key(|f| f.modified);

        for file in files {
            if current_bytes <= budget_bytes {
                break;
            }
            if fs::remove_file(&file.path).is_ok() {
                current_bytes = current_bytes.saturating_sub(file.size);
                total_freed += file.size;
            }
        }
    };

    evict_from_dir(MUSIC_DIR);
    evict_from_dir(HERO_DIR);
    evict_from_dir(LOGO_DIR);
    evict_from_dir(COVERS_DIR);

    total_freed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scanner::System;

    #[test]
    fn test_format_bytes() {
        assert_eq!(format_bytes(500), "500 B");
        assert_eq!(format_bytes(1024), "1 KB");
        assert_eq!(format_bytes(10 * 1024 * 1024), "10.0 MB");
        assert_eq!(format_bytes(2 * 1024 * 1024 * 1024), "2.0 GB");
    }

    #[test]
    fn test_build_valid_keys() {
        let games = vec![
            Game {
                title_id: "PCSA00001".to_string(),
                art_key: "PCSA00001".to_string(),
                title: "Persona 4 Golden".to_string(),
                system: System::Vita,
                cover_bytes: None,
                hero_bytes: None,
                logo_bytes: None,
                has_box_art: true,
                has_hero: true,
                has_logo: true,
                is_game: Some(true),
                music_path: None,
                music_resolved: false,
                has_bubble: true,
                file_path: None,
            }
        ];

        let keys = build_valid_keys(&games);
        assert!(keys.contains("pcsa00001"));
        assert!(keys.contains("persona 4 golden"));
        assert!(!keys.contains("random_game"));
    }
}


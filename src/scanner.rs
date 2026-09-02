use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
#[cfg(target_os = "vita")]
use std::ffi::CString;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
#[cfg(target_os = "vita")]
use vitasdk_sys::{sceIoDclose, sceIoDopen, sceIoDread, sceIoGetstat, SceIoDirent, SceIoStat};

const APP_ROOTS: &[&str] = &["ux0:app", "gro0:app", "grw0:app", "uma0:app", "ur0:app"];
const PSPEMU_PARTITIONS: &[&str] = &["ux0", "uma0", "ur0", "imc0"];
const COVERS_DIR: &str = "ux0:data/VitaDeck/COVERS/";
const HERO_DIR: &str = "ux0:data/VitaDeck/HERO/";
const LOGO_DIR: &str = "ux0:data/VitaDeck/LOGO/";
const MUSIC_DIR: &str = "ux0:data/VitaDeck/MUSIC/";
const API_CACHE_DIR: &str = "ux0:data/VitaDeck/APICACHE/";
pub const GAMES_CACHE_FILE: &str = "ux0:data/VitaDeck/APICACHE/games_manifest.json";
const OWN_TITLE_ID: &str = "VITADECK1";
const OVERRIDES_FILE: &str = "ux0:data/VitaDeck/overrides.dat";

pub const BOX_ART_SOURCE: &str = "box art";

pub type ImageBytes = (bool, Vec<u8>);

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum System {
    Vita,
    Psp,
    Psx,
}

impl System {
    pub fn covers_folder(self) -> &'static str {
        match self {
            System::Vita => "PSVita",
            System::Psp => "PSP",
            System::Psx => "PS1",
        }
    }

    pub fn platform_param(self) -> &'static str {
        match self {
            System::Vita => "psv",
            System::Psp => "psp",
            System::Psx => "psx",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            System::Vita => "PS Vita",
            System::Psp => "PSP",
            System::Psx => "PS1",
        }
    }
}

#[derive(Clone)]
pub struct Game {
    pub title_id: String,
    pub art_key: String,
    pub title: String,
    pub system: System,
    pub cover_bytes: Option<ImageBytes>,
    pub hero_bytes: Option<ImageBytes>,
    pub logo_bytes: Option<ImageBytes>,
    pub has_box_art: bool,
    pub has_hero: bool,
    pub has_logo: bool,
    pub is_game: Option<bool>,
    pub music_path: Option<String>,

    pub music_resolved: bool,
    pub has_bubble: bool,
    pub file_path: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct CachedGame {
    pub title_id: String,
    pub art_key: String,
    pub title: String,
    pub system: System,
    pub has_box_art: bool,
    pub has_hero: bool,
    pub has_logo: bool,
    pub is_game: Option<bool>,
    pub music_path: Option<String>,
    pub music_resolved: bool,
    #[serde(default = "default_true")]
    pub has_bubble: bool,
    #[serde(default)]
    pub file_path: Option<String>,
}

fn default_true() -> bool {
    true
}

pub fn load_cached_games() -> Option<Vec<Game>> {
    let bytes = std::fs::read(GAMES_CACHE_FILE).ok().filter(|b| !b.is_empty())?;
    let cached: Vec<CachedGame> = serde_json::from_slice(&bytes).ok()?;
    if cached.is_empty() {
        return None;
    }
    Some(
        cached
            .into_iter()
            .map(|c| Game {
                title_id: c.title_id,
                art_key: c.art_key,
                title: c.title,
                system: c.system,
                cover_bytes: None,
                hero_bytes: None,
                logo_bytes: None,
                has_box_art: c.has_box_art,
                has_hero: c.has_hero,
                has_logo: c.has_logo,
                is_game: c.is_game,
                music_path: c.music_path,
                music_resolved: c.music_resolved,
                has_bubble: c.has_bubble,
                file_path: c.file_path,
            })
            .collect(),
    )
}

pub fn save_cached_games(games: &[Game]) {
    let cached: Vec<CachedGame> = games
        .iter()
        .map(|g| CachedGame {
            title_id: g.title_id.clone(),
            art_key: g.art_key.clone(),
            title: g.title.clone(),
            system: g.system,
            has_box_art: g.has_box_art,
            has_hero: g.has_hero,
            has_logo: g.has_logo,
            is_game: g.is_game,
            music_path: g.music_path.clone(),
            music_resolved: g.music_resolved,
            has_bubble: g.has_bubble,
            file_path: g.file_path.clone(),
        })
        .collect();
    if let Ok(bytes) = serde_json::to_vec(&cached) {
        let _ = std::fs::create_dir_all("ux0:data/VitaDeck/APICACHE/");
        let tmp = format!("{}.part", GAMES_CACHE_FILE);
        if std::fs::write(&tmp, &bytes).is_ok() {
            let _ = std::fs::rename(&tmp, GAMES_CACHE_FILE);
        }
    }
}

fn find_music(dir: &str) -> Option<String> {
    let path = format!("{}/music.mp3", dir);
    sce_file_exists(&path).then_some(path)
}

pub fn scan_installed_games(disabled_dirs: &[String], custom_dirs: &[String]) -> (Vec<Game>, Vec<String>) {
    let mut games = Vec::new();
    let mut log = Vec::new();

    for folder in ["PSVita", "PSP", "PS1"] {
        let _ = std::fs::create_dir_all(format!("{}{}", COVERS_DIR, folder));
        let _ = std::fs::create_dir_all(format!("{}{}", HERO_DIR, folder));
        let _ = std::fs::create_dir_all(format!("{}{}", LOGO_DIR, folder));
        let _ = std::fs::create_dir_all(format!("{}{}", MUSIC_DIR, folder));
        let _ = std::fs::create_dir_all(format!("{}{}", API_CACHE_DIR, folder));
    }

    ensure_overrides_file();
    let pspemu = index_pspemu(&mut log, disabled_dirs, custom_dirs);
    let overrides = load_overrides();
    let matched = scan_vita_apps(&mut games, &mut log, &pspemu, &overrides);
    emit_unbubbled_pspemu_games(&mut games, &mut log, &pspemu, &matched);

    if games.is_empty() {
        log.push("ERROR: No games found in app/.".to_string());
    }

    games.sort_by(|a, b| a.title.to_lowercase().cmp(&b.title.to_lowercase()));

    write_scan_log(&log);

    (games, log)
}

struct PspemuEntry {
    art_key: String,
    title: String,
    icon0: Option<ImageBytes>,
    system_hint: System,
    source_dir: Option<String>,
    file_path: String,
}

struct PspemuIndex {
    entries: HashMap<String, PspemuEntry>,
    has_pbp: HashSet<String>,
}

fn is_junk_name(name: &str) -> bool {
    let base = name.rsplit_once('/').map(|(_, n)| n).unwrap_or(name);
    base.starts_with('.')
        || base.eq_ignore_ascii_case("Thumbs.db")
        || base.eq_ignore_ascii_case("desktop.ini")
}

/// Discover immediate subdirectories of `dir` that are "category" folders —
/// i.e. directories that don't themselves look like a PSP game dir (no direct
/// EBOOT.PBP). Used both for recursion and for the settings folder picker.
fn category_subdirs(dir: &str) -> Vec<String> {
    let Ok(names) = list_dir(dir) else { return Vec::new() };
    let mut out = Vec::new();
    for name in names {
        if is_junk_name(&name) {
            continue;
        }
        let child = format!("{}/{}", dir, name);
        if !is_dir(&child) {
            continue;
        }
        if sce_file_exists(&format!("{}/EBOOT.PBP", child)) {
            continue;
        }
        out.push(child);
    }
    out.sort();
    out
}

fn category_subdirs_recursive(root: &str, max_depth: usize) -> Vec<String> {
    let mut found = Vec::new();
    let mut frontier = vec![(root.to_string(), 0usize)];
    while let Some((dir, depth)) = frontier.pop() {
        if depth >= max_depth {
            continue;
        }
        for child in category_subdirs(&dir) {
            frontier.push((child.clone(), depth + 1));
            found.push(child);
        }
    }
    found.sort();
    found.dedup();
    found
}

fn is_dir_disabled(dir: &str, disabled_dirs: &[String]) -> bool {
    disabled_dirs.iter().any(|d| d == dir)
}

fn index_pbp_dir(
    game_root: &str,
    entries: &mut HashMap<String, PspemuEntry>,
    has_pbp: &mut HashSet<String>,
    log: &mut Vec<String>,
) {
    let Ok(names) = list_dir(game_root) else { return };
    log.push(format!("{} -> {} entries", game_root, names.len()));

    for title_id in names {
        if is_junk_name(&title_id) {
            continue;
        }
        let pbp = format!("{}/{}/EBOOT.PBP", game_root, title_id);
        if !sce_file_exists(&pbp) {
            continue;
        }

        let pbp_data = read_pbp_sections(&pbp);
        let sfo_fields = pbp_data.as_ref().and_then(|d| {
            parse_sfo_fields(&d.param_sfo, &["DISC_ID", "TITLE_ID", "CATEGORY", "TITLE", "STITLE"])
        });

        let system_hint = classify_pspemu(&title_id, sfo_fields.as_deref());
        let art_key = art_key_from_sfo(sfo_fields.as_deref()).unwrap_or_else(|| {
            sanitize_sony_title_id(&title_id).unwrap_or_else(|| title_id.clone())
        });
        let title = sfo_fields
            .as_deref()
            .and_then(|f| {
                f.iter().find(|(k, v)| {
                    (k == "TITLE" || k == "STITLE") && !v.trim().is_empty()
                })
            })
            .map(|(_, v)| normalize_title(v))
            .unwrap_or_else(|| title_id.clone());

        let icon0 = pbp_data
            .as_ref()
            .filter(|d| !d.icon0.is_empty())
            .map(|d| (true, d.icon0.clone()));

        let source_dir = Some(format!("{}/{}", game_root, title_id));

        has_pbp.insert(title_id.clone());
        entries.insert(title_id, PspemuEntry { art_key, title, icon0, system_hint, source_dir, file_path: pbp });
    }
}

fn index_iso_dir(
    iso_root: &str,
    entries: &mut HashMap<String, PspemuEntry>,
    log: &mut Vec<String>,
) {
    let Ok(names) = list_dir(iso_root) else { return };
    let mut count = 0;
    for name in names {
        if is_junk_name(&name) {
            continue;
        }
        let lower = name.to_lowercase();
        if !lower.ends_with(".iso") && !lower.ends_with(".cso") {
            continue;
        }
        let title = clean_rom_name(&name);
        if title.is_empty() {
            continue;
        }
        let sanitized_id = sanitize_sony_title_id(&name).or_else(|| sanitize_sony_title_id(&title));
        let art_key = sanitized_id.clone().unwrap_or_else(|| title.clone());
        let stem = name.rsplit_once('.').map(|(stem, _)| stem.to_string()).unwrap_or_else(|| name.clone());
        if is_junk_name(&stem) {
            continue;
        }

        let iso_path = format!("{}/{}", iso_root, name);
        entries.entry(stem).or_insert(PspemuEntry {
            art_key,
            title,
            icon0: None,
            system_hint: System::Psp,
            source_dir: None,
            file_path: iso_path,
        });
        count += 1;
    }
    log.push(format!("{} -> {} iso/cso entries", iso_root, count));
}

fn index_pspemu(log: &mut Vec<String>, disabled_dirs: &[String], custom_dirs: &[String]) -> PspemuIndex {
    let mut entries = HashMap::new();
    let mut has_pbp = HashSet::new();

    for part in PSPEMU_PARTITIONS {
        let pspemu_dir = format!("{}:pspemu", part);
        if !sce_file_exists(&pspemu_dir) {
            continue;
        }

        let game_root = format!("{}:pspemu/PSP/GAME", part);
        index_pbp_dir(&game_root, &mut entries, &mut has_pbp, log);
        for sub in category_subdirs_recursive(&game_root, 3) {
            if is_dir_disabled(&sub, disabled_dirs) {
                log.push(format!("{} -> skipped (disabled)", sub));
                continue;
            }
            index_pbp_dir(&sub, &mut entries, &mut has_pbp, log);
        }

        let iso_root = format!("{}:pspemu/ISO", part);
        index_iso_dir(&iso_root, &mut entries, log);
        for sub in category_subdirs_recursive(&iso_root, 3) {
            if is_dir_disabled(&sub, disabled_dirs) {
                log.push(format!("{} -> skipped (disabled)", sub));
                continue;
            }
            index_iso_dir(&sub, &mut entries, log);
        }
    }

    for dir in custom_dirs {
        if is_dir_disabled(dir, disabled_dirs) { continue; }
        if !sce_file_exists(dir) {
            log.push(format!("{} -> custom path unavailable", dir));
            continue;
        }
        index_pbp_dir(dir, &mut entries, &mut has_pbp, log);
        index_iso_dir(dir, &mut entries, log);
    }

    PspemuIndex { entries, has_pbp }
}

/// Enumerate every category subfolder (depth 1) under the pspemu PSP/GAME and
/// ISO roots across all partitions, for the settings folder picker.
pub fn discover_scan_folders() -> Vec<String> {
    let mut out = Vec::new();
    for part in PSPEMU_PARTITIONS {
        let pspemu_dir = format!("{}:pspemu", part);
        if !sce_file_exists(&pspemu_dir) {
            continue;
        }
        out.extend(category_subdirs_recursive(&format!("{}:pspemu/PSP/GAME", part), 3));
        out.extend(category_subdirs_recursive(&format!("{}:pspemu/ISO", part), 3));
    }
    out.sort();
    out.dedup();
    out
}

fn ensure_overrides_file() {
    if sce_file_exists(OVERRIDES_FILE) {
        return;
    }
    let _ = std::fs::create_dir_all("ux0:data/VitaDeck/");
    let _ = std::fs::write(OVERRIDES_FILE, b"");
}

fn load_overrides() -> HashMap<String, u8> {
    let mut map = HashMap::new();
    let Some(bytes) = read_file(OVERRIDES_FILE) else { return map };
    let text = String::from_utf8_lossy(&bytes);
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((id, val)) = line.split_once('=') {
            if let Ok(n) = val.trim().parse::<u8>() {
                map.insert(id.trim().to_uppercase(), n);
            }
        }
    }
    map
}

fn classify_app(root: &str, dir_name: &str, pspemu: &PspemuIndex, overrides: &HashMap<String, u8>) -> (System, bool) {
    if let Some(&n) = overrides.get(dir_name) {
        return match n {
            1 => (System::Vita, false),
            2 => (System::Psp, false),
            3 => (System::Psx, false),
            4 => (System::Vita, true),
            _ => (System::Vita, false),
        };
    }

    if dir_name.starts_with("PCS") && !dir_name.starts_with("PCSI") {
        return (System::Vita, false);
    }

    if sce_file_exists(&format!("{}/{}/data/boot.bin", root, dir_name)) {
        if let Some((_, entry)) = find_pspemu_entry(pspemu, dir_name) {
            return (entry.system_hint, false);
        }
        if pspemu.has_pbp.contains(dir_name) {
            return (System::Psx, false);
        }
        return (System::Psp, false);
    }

    (System::Vita, false)
}

fn find_pspemu_entry<'a>(pspemu: &'a PspemuIndex, bubble_id: &str) -> Option<(&'a str, &'a PspemuEntry)> {
    if let Some((key, entry)) = pspemu.entries.get_key_value(bubble_id) {
        return Some((key.as_str(), entry));
    }
    let needle = sanitize_sony_title_id(bubble_id)
        .unwrap_or_else(|| bubble_id.to_uppercase());
    pspemu.entries.iter().find_map(|(key, entry)| {
        if entry.art_key.eq_ignore_ascii_case(&needle) || key.eq_ignore_ascii_case(&needle) {
            Some((key.as_str(), entry))
        } else {
            None
        }
    })
}

fn scan_vita_apps(
    games: &mut Vec<Game>,
    log: &mut Vec<String>,
    pspemu: &PspemuIndex,
    overrides: &HashMap<String, u8>,
) -> HashSet<String> {
    let mut matched = HashSet::new();
    let mut seen_title_ids = HashSet::new();

    for root in APP_ROOTS {
        let names = match list_dir(root) {
            Ok(names) => {
                log.push(format!("{} -> {} entries", root, names.len()));
                names
            }
            Err(code) => {
                log.push(format!("sceIoDopen(\"{}\") failed: code {}", root, code));
                continue;
            }
        };

        let (mut wrong_len, mut no_sfo, mut skipped_dup) = (0, 0, 0);

        for title_id in names {
            if title_id == OWN_TITLE_ID || title_id.len() != 9 {
                if title_id.len() != 9 {
                    wrong_len += 1;
                }
                continue;
            }

            if !seen_title_ids.insert(title_id.clone()) {
                skipped_dup += 1;
                log.push(format!("SKIP duplicate {}/{}", root, title_id));
                continue;
            }

            let dir = format!("{}/{}", root, title_id);
            if !sce_file_exists(&format!("{}/sce_sys/param.sfo", dir)) {
                no_sfo += 1;
                continue;
            }

            let local_title = parse_sfo_title_from_file(&format!("{}/sce_sys/param.sfo", dir))
                .filter(|t| !t.trim().is_empty());

            let sfo_cat = parse_sfo_category_from_file(&format!("{}/sce_sys/param.sfo", dir));

            let (system, force_hidden) = classify_app(root, &title_id, pspemu, overrides);
            let pspemu_hit = find_pspemu_entry(pspemu, &title_id);
            if let Some((key, entry)) = pspemu_hit {
                matched.insert(key.to_string());
                matched.insert(title_id.clone());
                matched.insert(entry.art_key.clone());
            }

            let art_key = if system == System::Vita {
                title_id.clone()
            } else {
                pspemu_hit
                    .map(|(_, e)| e.art_key.clone())
                    .or_else(|| sanitize_sony_title_id(&title_id))
                    .unwrap_or_else(|| title_id.clone())
            };

            let title = local_title
                .clone()
                .filter(|t| t != &title_id)
                .or_else(|| pspemu_hit.map(|(_, e)| e.title.clone()).filter(|t| t != &title_id))
                .or_else(|| local_title.clone())
                .unwrap_or_else(|| title_id.clone());

            let is_utility = is_known_utility(&title_id, &title, sfo_cat.as_deref())
                || is_known_utility(&art_key, &title, sfo_cat.as_deref());
            let is_game = if force_hidden || is_utility { Some(false) } else { None };

            let cache_path_png = format!("{}.png", cover_cache_path(system, &art_key));
            let cache_path_jpg = format!("{}.jpg", cover_cache_path(system, &art_key));
            let title_path_png = format!("{}.png", cover_cache_path_by_title(system, &title));
            let title_path_jpg = format!("{}.jpg", cover_cache_path_by_title(system, &title));

            let has_cover = if sce_file_exists(&cache_path_png) || sce_file_exists(&cache_path_jpg) {
                true
            } else if let Some(path) = [&title_path_png, &title_path_jpg]
                .into_iter()
                .find(|p| sce_file_exists(p))
            {
                if let Some(bytes) = read_file(path) {
                    persist_cover_bytes(system, &art_key, &(path.ends_with(".png"), bytes));
                }
                true
            } else {
                let (bytes, art_source) = load_vita_cover(&title_id);
                match bytes {
                    Some(bytes) => {
                        if art_source != BOX_ART_SOURCE {
                            persist_cover_bytes(system, &art_key, &bytes);
                        }
                        true
                    }
                    None => match pspemu_hit.and_then(|(_, e)| e.icon0.as_ref()) {
                        Some(icon) => {
                            persist_cover_bytes(system, &art_key, icon);
                            true
                        }
                        None => false,
                    },
                }
            };

            let mut has_hero = cached_hero_exists(system, &art_key);
            if !has_hero {
                if let Some(pic) = load_appmeta_pic0(&title_id) {
                    persist_hero_bytes(system, &art_key, &pic);
                    has_hero = true;
                }
            }

            log.push(format!(
                "OK  {} {}/{} art_key={} -> \"{}\"",
                system.label(), root, title_id, art_key, title
            ));

            let music_path = find_music(&dir);
            let music_resolved = music_path.is_some() || cached_music_exists(system, &art_key);
            let has_logo = cached_logo_exists(system, &art_key);
            let file_path = pspemu_hit.map(|(_, e)| e.file_path.clone()).or_else(|| Some(dir.clone()));
            games.push(Game {
                title_id,
                art_key,
                title,
                system,
                has_box_art: has_cover,
                cover_bytes: None,
                hero_bytes: None,
                logo_bytes: None,
                has_hero,
                has_logo,
                is_game,
                music_path,
                music_resolved,
                has_bubble: true,
                file_path,
            });
        }

        log.push(format!(
            "{}: {} invalid length, {} missing param.sfo, {} duplicates skipped",
            root, wrong_len, no_sfo, skipped_dup
        ));
    }

    matched
}

fn load_appmeta_pic0(title_id: &str) -> Option<ImageBytes> {
    for path in [
        format!("ur0:appmeta/{}/pic0.png", title_id),
        format!("ux0:appmeta/{}/pic0.png", title_id),
    ] {
        if let Some(bytes) = read_file(&path) {
            return Some((is_png(&bytes), bytes));
        }
    }
    None
}

fn emit_unbubbled_pspemu_games(
    games: &mut Vec<Game>,
    log: &mut Vec<String>,
    pspemu: &PspemuIndex,
    matched: &HashSet<String>,
) {
    for (key, entry) in &pspemu.entries {
        if is_junk_name(key) {
            continue;
        }
        if matched.contains(key) || matched.contains(&entry.art_key) {
            continue;
        }

        let system = entry.system_hint;
        let art_key = entry.art_key.clone();
        let title = entry.title.clone();

        let cache_path_png = format!("{}.png", cover_cache_path(system, &art_key));
        let cache_path_jpg = format!("{}.jpg", cover_cache_path(system, &art_key));
        let has_cover = if sce_file_exists(&cache_path_png) || sce_file_exists(&cache_path_jpg) {
            true
        } else if let Some(icon) = &entry.icon0 {
            persist_cover_bytes(system, &art_key, icon);
            true
        } else {
            false
        };

        let is_utility = is_known_utility(&art_key, &title, None) || is_known_utility(key, &title, None);
        let is_game = if is_utility { Some(false) } else { None };

        let music_path = entry.source_dir.as_deref().and_then(find_music);
        let music_resolved = music_path.is_some() || cached_music_exists(system, &art_key);
        let has_hero = cached_hero_exists(system, &art_key);
        let has_logo = cached_logo_exists(system, &art_key);

        log.push(format!(
            "OK  {} pspemu/{} art_key={} -> \"{}\" [sin burbuja]",
            system.label(), key, art_key, title
        ));

        games.push(Game {
            title_id: key.clone(),
            art_key,
            title,
            system,
            has_box_art: has_cover,
            cover_bytes: None,
            hero_bytes: None,
            logo_bytes: None,
            has_hero,
            has_logo,
            is_game,
            music_path,
            music_resolved,
            has_bubble: false,
            file_path: Some(entry.file_path.clone()),
        });
    }
}

pub fn clean_rom_name(filename: &str) -> String {
    let stem = filename.rsplit_once('.').map(|(s, _)| s).unwrap_or(filename);
    let mut out = String::with_capacity(stem.len());
    let mut depth = 0i32;
    for c in stem.chars() {
        match c {
            '[' | '(' => depth += 1,
            ']' | ')' => depth = (depth - 1).max(0),
            '_' | '.' if depth == 0 => out.push(' '),
            '®' | '™' | '©' => {}
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn normalize_search_title(title: &str) -> String {
    let clean = title
        .replace(['®', '™', '©'], "")
        .replace(['_', '-', ':', '/'], " ");
    clean_rom_name(&clean)
}

fn classify_pspemu(title_id: &str, sfo_fields: Option<&[(String, String)]>) -> System {
    if let Some(fields) = sfo_fields {
        if let Some((_, category)) = fields.iter().find(|(k, _)| k == "CATEGORY") {
            let cat = category.trim();
            if cat == "ME" || cat == "MA" {
                return System::Psx;
            }
            if cat == "MG" || cat == "UG" || cat == "EG" || cat == "MS" {
                return System::Psp;
            }
        }
    }

    const PSX_PREFIXES: &[&str] = &["SL", "SC", "SI", "SE", "ES", "HP", "CP", "PA"];
    if PSX_PREFIXES.iter().any(|p| title_id.starts_with(p)) {
        System::Psx
    } else {
        System::Psp
    }
}

fn art_key_from_sfo(sfo_fields: Option<&[(String, String)]>) -> Option<String> {
    let fields = sfo_fields?;
    let disc_id = fields
        .iter()
        .find(|(k, _)| k == "DISC_ID")
        .or_else(|| fields.iter().find(|(k, _)| k == "TITLE_ID"))
        .map(|(_, v)| v.trim())
        .filter(|v| !v.is_empty())?;

    Some(disc_id.replace('-', "").to_uppercase())
}

pub fn cover_cache_path(system: System, art_key: &str) -> String {
    format!("{}{}/{}", COVERS_DIR, system.covers_folder(), art_key)
}

pub fn cover_cache_path_by_title(system: System, title: &str) -> String {
    format!("{}{}/{}", COVERS_DIR, system.covers_folder(), sanitize_cover_filename(title))
}

fn sanitize_cover_filename(title: &str) -> String {
    title.replace(['/', '\\'], "-")
}

pub fn hero_cache_path(system: System, art_key: &str) -> String {
    format!("{}{}/{}", HERO_DIR, system.covers_folder(), art_key)
}

pub fn logo_cache_path(system: System, art_key: &str) -> String {
    format!("{}{}/{}", LOGO_DIR, system.covers_folder(), art_key)
}

pub fn music_cache_path(system: System, art_key: &str) -> String {
    format!("{}{}/{}", MUSIC_DIR, system.covers_folder(), art_key)
}

fn cached_music_exists(system: System, art_key: &str) -> bool {
    sce_file_exists(&format!("{}.mp3", music_cache_path(system, art_key)))
}

fn cached_hero_exists(system: System, art_key: &str) -> bool {
    let base = hero_cache_path(system, art_key);
    sce_file_exists(&format!("{}.png", base)) || sce_file_exists(&format!("{}.jpg", base))
}

pub fn cached_logo_exists(system: System, art_key: &str) -> bool {
    let base = logo_cache_path(system, art_key);
    sce_file_exists(&format!("{}.png", base)) || sce_file_exists(&format!("{}.jpg", base))
}

pub fn resolved_music_path(game: &Game) -> Option<String> {
    if let Some(path) = &game.music_path {
        return Some(path.clone());
    }
    cached_music_exists(game.system, &game.art_key).then(|| format!("{}.mp3", music_cache_path(game.system, &game.art_key)))
}

fn is_png(bytes: &[u8]) -> bool {
    bytes.len() >= 8 && &bytes[0..8] == b"\x89PNG\r\n\x1a\n"
}

pub fn load_vita_cover(title_id: &str) -> (Option<ImageBytes>, &'static str) {
    let base = cover_cache_path(System::Vita, title_id);
    let candidates = [
        (format!("{}.png", base), BOX_ART_SOURCE),
        (format!("{}.jpg", base), BOX_ART_SOURCE),
        (format!("ur0:appmeta/{}/icon0.png", title_id), "ur0:appmeta"),
        (format!("ux0:appmeta/{}/icon0.png", title_id), "ux0:appmeta"),
        (format!("ux0:app/{}/sce_sys/icon0.png", title_id), "sce_sys"),
        (format!("gro0:app/{}/sce_sys/icon0.png", title_id), "sce_sys"),
        (format!("grw0:app/{}/sce_sys/icon0.png", title_id), "sce_sys"),
        (format!("uma0:app/{}/sce_sys/icon0.png", title_id), "sce_sys"),
        (format!("ur0:app/{}/sce_sys/icon0.png", title_id), "sce_sys"),
    ];

    for (path, source) in candidates {
        if let Some(bytes) = read_file(&path) {
            let is_png_file = is_png(&bytes);
            return (Some((is_png_file, bytes)), source);
        }
    }

    (None, "no icon")
}

pub fn load_cached_hero(system: System, art_key: &str) -> Option<ImageBytes> {
    load_texture_bytes(&hero_cache_path(system, art_key))
}

pub fn load_cached_logo(system: System, art_key: &str) -> Option<ImageBytes> {
    load_texture_bytes(&logo_cache_path(system, art_key))
}

pub fn load_cached_cover(system: System, art_key: &str) -> Option<ImageBytes> {
    if let Some(bytes) = load_texture_bytes(&cover_cache_path(system, art_key)) {
        return Some(bytes);
    }
    match system {
        System::Vita => load_vita_cover(art_key).0,
        System::Psp | System::Psx => {
            for part in PSPEMU_PARTITIONS {
                let eboot = format!("{}:pspemu/PSP/GAME/{}/EBOOT.PBP", part, art_key);
                if let Some(pbp) = read_pbp_sections(&eboot) {
                    if !pbp.icon0.is_empty() {
                        return Some((true, pbp.icon0));
                    }
                }
            }
            None
        }
    }
}

fn persist_cover_bytes(system: System, art_key: &str, bytes: &ImageBytes) {
    let base = cover_cache_path(system, art_key);
    if let Some(parent) = std::path::Path::new(&base).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let ext = if bytes.0 { "png" } else { "jpg" };
    let path = format!("{}.{}", base, ext);
    let tmp = format!("{}.part", path);
    if std::fs::write(&tmp, &bytes.1).is_err() {
        return;
    }
    let _ = std::fs::rename(&tmp, &path);
}

fn persist_hero_bytes(system: System, art_key: &str, bytes: &ImageBytes) {
    let base = hero_cache_path(system, art_key);
    if let Some(parent) = std::path::Path::new(&base).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let ext = if bytes.0 { "png" } else { "jpg" };
    let path = format!("{}.{}", base, ext);
    let tmp = format!("{}.part", path);
    if std::fs::write(&tmp, &bytes.1).is_err() {
        return;
    }
    let _ = std::fs::rename(&tmp, &path);
}

fn load_texture_bytes(base_path_no_ext: &str) -> Option<ImageBytes> {
    for (ext, is_png) in [("png", true), ("jpg", false), ("jpeg", false)] {
        let path = format!("{}.{}", base_path_no_ext, ext);
        let Some(bytes) = read_file(&path) else { continue };
        if crate::textures::source_exceeds_decode_budget(&bytes) {
            let _ = std::fs::remove_file(&path);
            continue;
        }
        return Some((is_png, bytes));
    }
    None
}

fn read_file(path: &str) -> Option<Vec<u8>> {
    std::fs::read(path).ok().filter(|b| !b.is_empty())
}

pub struct PbpSections {
    param_sfo: Vec<u8>,
    icon0: Vec<u8>,
}

fn read_pbp_sections(path: &str) -> Option<PbpSections> {
    let mut file = File::open(path).ok()?;

    let mut header = [0u8; 0x28];
    file.read_exact(&mut header).ok()?;
    if &header[0..4] != b"\0PBP" {
        return None;
    }

    let offset_at = |i: usize| -> u32 {
        u32::from_le_bytes(header[8 + i * 4..12 + i * 4].try_into().unwrap())
    };
    let (sfo_off, icon0_off, icon1_off) = (
        offset_at(0),
        offset_at(1),
        offset_at(2),
    );

    let read_range = |file: &mut File, start: u32, end: u32| -> Vec<u8> {
        if end <= start {
            return Vec::new();
        }
        let len = (end - start) as usize;
        if len > 4 * 1024 * 1024 || file.seek(SeekFrom::Start(start as u64)).is_err() {
            return Vec::new();
        }
        let mut buf = vec![0u8; len];
        match file.read_exact(&mut buf) {
            Ok(()) => buf,
            Err(_) => Vec::new(),
        }
    };

    let param_sfo = read_range(&mut file, sfo_off, icon0_off);
    let icon0 = read_range(&mut file, icon0_off, icon1_off);

    Some(PbpSections { param_sfo, icon0 })
}

#[cfg(target_os = "vita")]
fn list_dir(path: &str) -> Result<Vec<String>, i32> {
    let Ok(c_path) = CString::new(path) else { return Err(-1) };
    let mut names = Vec::new();

    unsafe {
        let fd = sceIoDopen(c_path.as_ptr());
        if fd < 0 {
            return Err(fd);
        }

        loop {
            let mut dirent: SceIoDirent = std::mem::zeroed();
            if sceIoDread(fd, &mut dirent) <= 0 {
                break;
            }
            let name = dirent_name(&dirent);
            if !name.is_empty() && name != "." && name != ".." {
                names.push(name);
            }
        }

        sceIoDclose(fd);
    }

    Ok(names)
}

#[cfg(not(target_os = "vita"))]
fn list_dir(path: &str) -> Result<Vec<String>, i32> {
    let Ok(entries) = std::fs::read_dir(path) else { return Err(-1) };
    let mut names = Vec::new();
    for entry in entries.flatten() {
        if let Ok(name) = entry.file_name().into_string() {
            names.push(name);
        }
    }
    Ok(names)
}

#[cfg(target_os = "vita")]
fn dirent_name(dirent: &SceIoDirent) -> String {
    let bytes: &[u8] = unsafe {
        std::slice::from_raw_parts(dirent.d_name.as_ptr() as *const u8, dirent.d_name.len())
    };
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}

#[cfg(target_os = "vita")]
fn sce_file_exists(path: &str) -> bool {
    let Ok(c_path) = CString::new(path) else { return false };
    unsafe {
        let mut stat: SceIoStat = std::mem::zeroed();
        sceIoGetstat(c_path.as_ptr(), &mut stat) >= 0
    }
}

#[cfg(not(target_os = "vita"))]
fn sce_file_exists(path: &str) -> bool {
    std::path::Path::new(path).exists()
}

#[cfg(target_os = "vita")]
fn is_dir(path: &str) -> bool {
    let Ok(c_path) = CString::new(path) else { return false };
    unsafe {
        let mut stat: SceIoStat = std::mem::zeroed();
        if sceIoGetstat(c_path.as_ptr(), &mut stat) < 0 {
            return false;
        }
        (stat.st_mode & vitasdk_sys::SCE_S_IFMT as i32) == vitasdk_sys::SCE_S_IFDIR as i32
    }
}

#[cfg(not(target_os = "vita"))]
fn is_dir(path: &str) -> bool {
    std::path::Path::new(path).is_dir()
}

pub fn write_append_log(line: &str) {
    let _ = std::fs::create_dir_all("ux0:data/VitaDeck/");
    if let Ok(mut f) = File::options().create(true).append(true).open("ux0:data/VitaDeck/scan_log.txt") {
        use std::io::Write;
        let _ = writeln!(f, "{}", line);
    }
}

fn write_scan_log(log: &[String]) {
    let _ = std::fs::create_dir_all("ux0:data/VitaDeck/");
    if let Ok(mut f) = File::create("ux0:data/VitaDeck/scan_log.txt") {
        use std::io::Write;
        for line in log {
            let _ = writeln!(f, "{}", line);
        }
    }
}

fn parse_sfo_title_from_file(path: &str) -> Option<String> {
    let mut file = File::open(path).ok()?;
    let mut data = Vec::new();
    file.read_to_end(&mut data).ok()?;
    parse_sfo_title(&data)
}

fn parse_sfo_title(data: &[u8]) -> Option<String> {
    let fields = parse_sfo_fields(data, &["TITLE", "STITLE"])?;
    fields
        .into_iter()
        .find(|(_, v)| !v.trim().is_empty())
        .map(|(_, v)| normalize_title(&v))
}

/// SFO strings occasionally contain invisible controls or malformed UTF-8.
/// `from_utf8_lossy` has already made malformed sequences safe; remove only
/// controls that cannot be rendered in a game title and collapse whitespace.
fn normalize_title(raw: &str) -> String {
    raw.chars()
        .map(|c| if c.is_control() || c.is_whitespace() { ' ' } else { c })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn parse_sfo_fields(data: &[u8], wanted: &[&str]) -> Option<Vec<(String, String)>> {
    if data.len() < 20 || &data[0..4] != b"\0PSF" {
        return None;
    }

    let key_table_offset = u32::from_le_bytes(data[8..12].try_into().unwrap()) as usize;
    let data_table_offset = u32::from_le_bytes(data[12..16].try_into().unwrap()) as usize;
    let entries_count = u32::from_le_bytes(data[16..20].try_into().unwrap()) as usize;

    let mut found = Vec::new();
    let mut current_entry = 20;
    for _ in 0..entries_count {
        if current_entry + 16 > data.len() {
            break;
        }

        let key_offset =
            u16::from_le_bytes(data[current_entry..current_entry + 2].try_into().unwrap()) as usize;
        let data_val_offset = u32::from_le_bytes(
            data[current_entry + 12..current_entry + 16].try_into().unwrap(),
        ) as usize;

        let key_start = key_table_offset.saturating_add(key_offset);
        let data_start = data_table_offset.saturating_add(data_val_offset);
        if key_start >= data.len() || data_start > data.len() {
            current_entry += 16;
            continue;
        }

        let key = read_cstr(data, key_start);
        if wanted.contains(&key.as_str()) {
            found.push((key, read_cstr(data, data_start)));
        }

        current_entry += 16;
    }

    found.sort_by_key(|(k, _)| wanted.iter().position(|w| w == k).unwrap_or(usize::MAX));

    Some(found)
}

fn read_cstr(data: &[u8], start: usize) -> String {
    let end = data[start..]
        .iter()
        .position(|&b| b == 0)
        .map(|p| start + p)
        .unwrap_or(data.len());
    String::from_utf8_lossy(&data[start..end]).into_owned()
}

fn parse_sfo_category_from_file(path: &str) -> Option<String> {
    let mut file = File::open(path).ok()?;
    let mut data = Vec::new();
    file.read_to_end(&mut data).ok()?;
    parse_sfo_fields(&data, &["CATEGORY"])?
        .into_iter()
        .find(|(k, _)| k == "CATEGORY")
        .map(|(_, v)| v)
}

pub fn sanitize_sony_title_id(raw: &str) -> Option<String> {
    let s = raw
        .trim_end_matches(".iso")
        .trim_end_matches(".ISO")
        .trim_end_matches(".cso")
        .trim_end_matches(".CSO")
        .replace(['-', '_', ' '], "")
        .to_uppercase();

    if s.len() == 9
        && s.as_bytes()[..4].iter().all(|b| b.is_ascii_alphabetic())
        && s.as_bytes()[4..].iter().all(|b| b.is_ascii_digit())
    {
        Some(s)
    } else {
        None
    }
}

pub fn is_known_utility(title_id: &str, title: &str, category: Option<&str>) -> bool {
    let sanitized = sanitize_sony_title_id(title_id).unwrap_or_else(|| title_id.to_uppercase());

    const KNOWN_UTILITY_IDS: &[&str] = &[
        "PSPEMUCFW", "SKGB4TF1X", "SKGD3PL0Y", "VITASHELL", "VITADBDLD",
        "AUTOPLUG2", "PKGJ00000", "SKGTLSE12", "SHRKBR33D", "VGCF00001",
        "VITAHBSRT", "BHBB00001", "VHBB00001", "CTMANAGER", "PSVIDENT0",
        "OPENNOWV0", "VITAFORGE", "RETROFLOW", "RETROLNCR", "RETROVITA",
        "PCSI00011",
    ];

    if KNOWN_UTILITY_IDS.contains(&sanitized.as_str()) || title_id.to_uppercase().starts_with("SKG") {
        return true;
    }

    if let Some(cat) = category {
        let cat_trimmed = cat.trim();
        if cat_trimmed.eq_ignore_ascii_case("sd") || cat_trimmed.eq_ignore_ascii_case("gda") || cat_trimmed.eq_ignore_ascii_case("Utility") {
            return true;
        }
    }

    let lower_title = title.to_lowercase();
    const UTILITY_KEYWORDS: &[&str] = &[
        "adrenaline", "batteryfixer", "vitadeploy", "vitashell", "autoplugin",
        "homebrew browser", "custom themes manager", "psvident", "vitagrafix",
        "vitadb", "pkgj", "itls-enso", "sharkbr33d", "psm runtime",
        "retroflow", "retroarch",
    ];

    for kw in UTILITY_KEYWORDS {
        if lower_title.contains(kw) {
            return true;
        }
    }

    false
}

#[cfg(all(test, not(target_os = "vita")))]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn unique_tmp_dir(name: &str) -> String {
        let pid = std::process::id();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("vitadeck_test_{}_{}_{}", name, pid, nanos));
        std::fs::create_dir_all(&dir).expect("create tmp dir");
        dir.to_string_lossy().into_owned()
    }

    #[test]
    fn category_subdirs_finds_non_game_directories() {
        let root = unique_tmp_dir("category_subdirs");
        std::fs::create_dir_all(format!("{}/CAT_PSP", root)).unwrap();
        std::fs::create_dir_all(format!("{}/SLUS12345", root)).unwrap();
        std::fs::write(format!("{}/SLUS12345/EBOOT.PBP", root), b"fake").unwrap();

        let subs = category_subdirs(&root);
        assert_eq!(subs, vec![format!("{}/CAT_PSP", root)]);

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn index_pbp_dir_recurses_into_category_folder() {
        let root = unique_tmp_dir("index_pbp_dir");
        let game_root = format!("{}/PSP/GAME", root);
        std::fs::create_dir_all(&game_root).unwrap();
        std::fs::create_dir_all(format!("{}/CAT_PSX/SLUS01234", game_root)).unwrap();
        std::fs::write(format!("{}/CAT_PSX/SLUS01234/EBOOT.PBP", game_root), b"fake").unwrap();

        let mut entries = HashMap::new();
        let mut has_pbp = HashSet::new();
        let mut log = Vec::new();

        index_pbp_dir(&game_root, &mut entries, &mut has_pbp, &mut log);
        assert!(entries.is_empty(), "top-level scan should not find the category folder's game");

        for sub in category_subdirs(&game_root) {
            index_pbp_dir(&sub, &mut entries, &mut has_pbp, &mut log);
        }
        assert!(entries.contains_key("SLUS01234"));

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn index_iso_dir_recurses_into_category_folder() {
        let root = unique_tmp_dir("index_iso_dir");
        let iso_root = format!("{}/ISO", root);
        std::fs::create_dir_all(format!("{}/CAT_Minis", iso_root)).unwrap();
        std::fs::write(format!("{}/CAT_Minis/MyGame.iso", iso_root), b"fake").unwrap();

        let mut entries = HashMap::new();
        let mut log = Vec::new();

        index_iso_dir(&iso_root, &mut entries, &mut log);
        assert!(entries.is_empty());

        for sub in category_subdirs(&iso_root) {
            index_iso_dir(&sub, &mut entries, &mut log);
        }
        assert!(entries.contains_key("MyGame"));

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn category_discovery_supports_nested_cat_folders() {
        let root = unique_tmp_dir("nested_categories");
        let game_root = format!("{}/PSP/GAME", root);
        std::fs::create_dir_all(format!("{}/CAT_Retro/CAT_PSX/SLUS01234", game_root)).unwrap();
        std::fs::write(
            format!("{}/CAT_Retro/CAT_PSX/SLUS01234/EBOOT.PBP", game_root),
            b"fake",
        )
        .unwrap();

        let folders = category_subdirs_recursive(&game_root, 3);
        assert!(folders.contains(&format!("{}/CAT_Retro", game_root)));
        assert!(folders.contains(&format!("{}/CAT_Retro/CAT_PSX", game_root)));

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn disabled_dir_check_matches_exact_path() {
        let disabled = vec!["ux0:pspemu/ISO/CAT_PSP".to_string()];
        assert!(is_dir_disabled("ux0:pspemu/ISO/CAT_PSP", &disabled));
        assert!(!is_dir_disabled("ux0:pspemu/ISO/CAT_PSX", &disabled));
    }

    #[test]
    fn normalizes_titles_without_losing_common_game_symbols() {
        assert_eq!(
            normalize_title(" ACE  COMBAT™\0\tJOINT\nASSAULT "),
            "ACE COMBAT™ JOINT ASSAULT"
        );
    }
}

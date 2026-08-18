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

pub fn scan_installed_games() -> (Vec<Game>, Vec<String>) {
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
    let pspemu = index_pspemu(&mut log);
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

fn index_pspemu(log: &mut Vec<String>) -> PspemuIndex {
    let mut entries = HashMap::new();
    let mut has_pbp = HashSet::new();

    for part in PSPEMU_PARTITIONS {
        let pspemu_dir = format!("{}:pspemu", part);
        if !sce_file_exists(&pspemu_dir) {
            continue;
        }

        let game_root = format!("{}:pspemu/PSP/GAME", part);
        if let Ok(names) = list_dir(&game_root) {
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
                    .map(|(_, v)| v.clone())
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

        let iso_root = format!("{}:pspemu/ISO", part);
        if let Ok(names) = list_dir(&iso_root) {
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
            }
        }
    }

    PspemuIndex { entries, has_pbp }
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

        let looks_like_id = sanitize_sony_title_id(key).is_some()
            || sanitize_sony_title_id(&art_key).is_some();
        let has_pbp_meta = entry.source_dir.is_some();
        if !looks_like_id && !has_pbp_meta {
            let cache_png = format!("{}.png", cover_cache_path(system, &art_key));
            let cache_jpg = format!("{}.jpg", cover_cache_path(system, &art_key));
            if !sce_file_exists(&cache_png) && !sce_file_exists(&cache_jpg) {
                log.push(format!("SKIP unbubbled ISO without TitleID: {}", key));
                continue;
            }
        }

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
        .map(|(_, v)| v)
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

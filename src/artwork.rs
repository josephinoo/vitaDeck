use crate::net;
use crate::scanner::{cover_cache_path, hero_cache_path, logo_cache_path, music_cache_path, Game, System};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::mpsc::Sender;

pub struct ArtJob {
    pub title_id: String,
    pub art_key: String,
    pub title: String,
    pub system: System,

    pub known_cover_url: Option<String>,
    pub known_screenshot_url: Option<String>,
    pub known_logo_url: Option<String>,

    pub known_music_url: Option<String>,
}

#[derive(Clone)]
pub struct ImageData {
    pub is_png: bool,
    pub bytes: Vec<u8>,
}

pub struct ArtResult {
    pub title_id: String,
    pub cover: Option<ImageData>,
    pub hero: Option<ImageData>,
    pub logo: Option<ImageData>,

    pub music_downloaded: bool,
    pub error: Option<String>,

    pub job_complete: bool,

    pub images_ok: bool,
}

const MAX_JSON_BYTES: usize = 64 * 1024;
const MAX_IMAGE_BYTES: usize = 2 * 1024 * 1024;

const MAX_MUSIC_BYTES: usize = 12 * 1024 * 1024;

const PREFERRED_IMAGE_BYTES: usize = 800 * 1024;

fn asset_url(path: &str) -> String {
    if path.starts_with("http://") || path.starts_with("https://") {
        path.to_string()
    } else {
        format!("{}/assets/{}", net::API_BASE, path.trim_start_matches('/'))
    }
}

fn is_jpeg(bytes: &[u8]) -> bool {
    bytes.len() >= 3 && bytes[0] == 0xFF && bytes[1] == 0xD8 && bytes[2] == 0xFF
}

fn is_png(bytes: &[u8]) -> bool {
    bytes.len() >= 8 && &bytes[0..8] == b"\x89PNG\r\n\x1a\n"
}

fn fetch_image(url: &str) -> Option<ImageData> {
    let bytes = match net::download_to_vec(url, MAX_IMAGE_BYTES) {
        Ok(b) => b,
        Err(err) => {
            crate::scanner::write_append_log(&format!("fetch_image failed for {}: {}", url, err));
            return None;
        }
    };
    if is_png(&bytes) {
        Some(ImageData { is_png: true, bytes })
    } else if is_jpeg(&bytes) {
        Some(ImageData { is_png: false, bytes })
    } else {
        crate::scanner::write_append_log(&format!(
            "fetch_image invalid format (len: {}) for {}",
            bytes.len(),
            url
        ));
        None
    }
}

pub fn save_to_cache(image: &ImageData, base_path_no_ext: &str) {
    let ext = if image.is_png { "png" } else { "jpg" };
    let path = format!("{}.{}", base_path_no_ext, ext);
    let tmp = format!("{}.part", path);
    if std::fs::write(&tmp, &image.bytes).is_err() {
        return;
    }

    let _ = std::fs::rename(&tmp, &path);
}

fn fetch_and_cache_music(url: &str, system: System, art_key: &str) -> bool {
    let path = format!("{}.mp3", music_cache_path(system, art_key));
    let tmp = format!("{}.part", path);
    if let Some(parent) = std::path::Path::new(&path).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Err(err) = net::download_to_file(url, &tmp, MAX_MUSIC_BYTES) {
        crate::scanner::write_append_log(&format!("fetch_music failed for {}: {}", url, err));
        let _ = std::fs::remove_file(&tmp);
        return false;
    }
    if !looks_like_mp3(&tmp) {
        crate::scanner::write_append_log(&format!(
            "fetch_music invalid mp3 header for {} (cached as {})",
            url, tmp
        ));
        let _ = std::fs::remove_file(&tmp);
        return false;
    }
    std::fs::rename(&tmp, &path).is_ok()
}

fn looks_like_mp3(path: &str) -> bool {
    let Ok(mut file) = std::fs::File::open(path) else {
        return false;
    };
    let mut header = [0u8; 10];
    let Ok(n) = std::io::Read::read(&mut file, &mut header) else {
        return false;
    };
    if n >= 3 && &header[0..3] == b"ID3" {
        return true;
    }
    n >= 2 && header[0] == 0xFF && (header[1] & 0xE0) == 0xE0
}

pub fn resolve(job: ArtJob, tx: &Sender<ArtResult>) {
    if !net::wifi_available() {
        let _ = tx.send(ArtResult {
            title_id: job.title_id,
            cover: None,
            hero: None,
            logo: None,
            music_downloaded: false,
            error: Some("no wifi".to_string()),
            job_complete: true,
            images_ok: false,
        });
        return;
    }

    let cover = job.known_cover_url.as_deref().and_then(fetch_image);
    let hero = job.known_screenshot_url.as_deref().and_then(fetch_image);
    let logo = job.known_logo_url.as_deref().and_then(fetch_image);

    if let Some(cover) = &cover {
        save_to_cache(cover, &cover_cache_path(job.system, &job.art_key));
    }
    if let Some(hero) = &hero {
        save_to_cache(hero, &hero_cache_path(job.system, &job.art_key));
    }
    if let Some(logo) = &logo {
        save_to_cache(logo, &logo_cache_path(job.system, &job.art_key));
    }

    let images_ok = cover.is_some() || hero.is_some() || logo.is_some();
    let music_url = job.known_music_url.clone();

    let _ = tx.send(ArtResult {
        title_id: job.title_id.clone(),
        cover,
        hero,
        logo,
        music_downloaded: false,
        error: if !images_ok && music_url.is_none() {
            let err_msg = format!("no art found for {} [{}]", job.title, job.art_key);
            crate::scanner::write_append_log(&err_msg);
            Some(err_msg)
        } else {
            None
        },
        job_complete: true,
        images_ok,
    });

    if let Some(url) = music_url {
        let tx = tx.clone();
        let title_id = job.title_id.clone();
        let system = job.system;
        let art_key = job.art_key.clone();
        let images_ok = images_ok;
        std::thread::spawn(move || {
            let music_downloaded = fetch_and_cache_music(&url, system, &art_key);
            if !music_downloaded {
                crate::scanner::write_append_log(&format!(
                    "music download failed for {} [{}]",
                    title_id, art_key
                ));
            }
            let _ = tx.send(ArtResult {
                title_id,
                cover: None,
                hero: None,
                logo: None,
                music_downloaded,
                error: None,

                job_complete: false,
                images_ok,
            });
        });
    }
}

const VITADECK_LOOKUP_CACHE: &str = "ux0:data/VitaDeck/APICACHE/vitadeck_lookup.json";

#[derive(Deserialize)]
struct GameDetail {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    cleaned_name: Option<String>,
    #[serde(default)]
    title_id: Option<String>,
    #[serde(default)]
    platform: Option<String>,
    #[serde(default)]
    cover_path: Option<String>,
    #[serde(default)]
    logos: Vec<String>,
    #[serde(default)]
    heroes: Vec<String>,
    #[serde(default)]
    musics: Vec<String>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct LookupResult {
    pub is_game: bool,
    #[serde(default)]
    pub canonical_title: Option<String>,
    #[serde(default)]
    pub cover_url: Option<String>,

    #[serde(default)]
    pub screenshot_url: Option<String>,

    #[serde(default)]
    pub logo_url: Option<String>,

    #[serde(default)]
    pub music_url: Option<String>,
}

pub struct LookupTarget {
    pub art_key: String,
    pub title_id: String,
    pub title: String,
    pub system: System,
}

impl LookupTarget {
    pub fn from_game(game: &Game) -> Self {
        Self {
            art_key: game.art_key.clone(),
            title_id: game.title_id.clone(),
            title: game.title.clone(),
            system: game.system,
        }
    }
}

fn games_checksum(games: &[LookupTarget]) -> u64 {
    const LOOKUP_CACHE_VERSION: u64 = 0x0A_00;
    let mut hash: u64 = 0xcbf29ce484222325 ^ LOOKUP_CACHE_VERSION; 
    for g in games {
        for byte in g.art_key.bytes().chain(g.title_id.bytes()) {
            hash ^= byte as u64;
            hash = hash.wrapping_mul(0x100000001b3);
        }
    }
    hash
}

fn select_best_variant(paths: &[String]) -> Option<String> {
    select_best_variant_preferring(paths, PreferFormat::Jpeg)
}

fn select_best_logo_variant(paths: &[String]) -> Option<String> {

    select_best_variant_preferring(paths, PreferFormat::Png)
}

#[derive(Clone, Copy)]
enum PreferFormat {
    Jpeg,
    Png,
}

fn path_is_vita(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    name.contains("_vita.")
}

fn format_penalty(path: &str, prefer: PreferFormat) -> u8 {
    let lower = path.to_ascii_lowercase();
    let is_jpg = lower.ends_with(".jpg") || lower.ends_with(".jpeg");
    let is_png = lower.ends_with(".png");
    match prefer {
        PreferFormat::Jpeg => {
            if is_jpg {
                0
            } else if is_png {
                1
            } else {
                2
            }
        }
        PreferFormat::Png => {
            if is_png {
                0
            } else if is_jpg {
                3
            } else {
                2
            }
        }
    }
}

fn select_best_variant_preferring(paths: &[String], prefer: PreferFormat) -> Option<String> {
    let candidates: Vec<&String> = paths
        .iter()
        .filter(|p| {
            let lower = p.to_ascii_lowercase();
            !lower.ends_with(".webp") && !lower.ends_with(".avif")
        })
        .collect();
    if candidates.is_empty() {
        return None;
    }

    let mut vita: Vec<&&String> = candidates.iter().filter(|p| path_is_vita(p)).collect();
    if !vita.is_empty() {
        vita.sort_by_key(|p| format_penalty(p, prefer));
        return Some(asset_url(vita[0]));
    }

    if candidates.len() == 1 {
        return Some(asset_url(candidates[0]));
    }

    let mut best: Option<(usize, u8, String)> = None;
    for path in &candidates {
        let url = asset_url(path);
        let penalty = format_penalty(path, prefer);
        let Some(size) = net::fetch_content_length(&url) else {
            continue;
        };
        let cand = (size, penalty, url);
        best = Some(match best.take() {
            None => cand,
            Some(cur) => {
                let cur_preferred = cur.0 <= PREFERRED_IMAGE_BYTES;
                let new_preferred = cand.0 <= PREFERRED_IMAGE_BYTES;
                if new_preferred && !cur_preferred {
                    cand
                } else if new_preferred == cur_preferred && (cand.0, cand.1) < (cur.0, cur.1) {
                    cand
                } else {
                    cur
                }
            }
        });
    }

    best.map(|(_, _, url)| url)
        .or_else(|| candidates.first().map(|p| asset_url(p)))
}

fn lookup_one(title_id: &str) -> Option<LookupResult> {
    let url = format!("{}/api/games/{}", net::API_BASE, title_id);
    let bytes = net::download_to_vec(&url, MAX_JSON_BYTES).ok()?;
    let detail: GameDetail = serde_json::from_slice(&bytes).ok()?;
    let canonical_title = detail
        .cleaned_name
        .or(detail.name)
        .or(detail.title)
        .filter(|t| !t.trim().is_empty());

    Some(LookupResult {
        is_game: true,
        canonical_title,
        cover_url: detail.cover_path.as_deref().filter(|s| !s.trim().is_empty()).map(asset_url),
        screenshot_url: select_best_variant(&detail.heroes),
        logo_url: select_best_logo_variant(&detail.logos),
        music_url: select_best_variant(&detail.musics),
    })
}

fn url_encode(s: &str) -> String {
    let mut encoded = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(b as char);
            }
            b' ' => encoded.push_str("%20"),
            _ => encoded.push_str(&format!("%{:02X}", b)),
        }
    }
    encoded
}

fn is_title_match(target: &str, candidate: &str) -> bool {
    let t = target.to_lowercase();
    let c = candidate.to_lowercase();
    if t.is_empty() || c.is_empty() {
        return false;
    }
    if t == c || t.contains(&c) || c.contains(&t) {
        return true;
    }

    let t_words: Vec<&str> = t.split_whitespace().filter(|w| w.len() > 2).collect();
    let c_words: Vec<&str> = c.split_whitespace().filter(|w| w.len() > 2).collect();
    if !t_words.is_empty() && !c_words.is_empty() {
        let matches = t_words.iter().filter(|tw| c_words.contains(tw)).count();
        if matches >= (t_words.len().min(c_words.len()) + 1) / 2 {
            return true;
        }
    }
    false
}

#[derive(Deserialize)]
struct GamesListResponse {
    #[serde(default)]
    games: Vec<GameDetail>,
}

fn lookup_by_query(query: &str, system: System) -> Option<LookupResult> {
    let clean_q = crate::scanner::normalize_search_title(query);
    if clean_q.trim().is_empty() {
        return None;
    }
    let encoded = url_encode(clean_q.trim());
    let url = format!(
        "{}/api/games?q={}&platform={}&limit=10",
        net::API_BASE,
        encoded,
        system.platform_param()
    );
    let bytes = net::download_to_vec(&url, MAX_JSON_BYTES).ok()?;

    let mut details: Vec<GameDetail> = Vec::new();
    if let Ok(list) = serde_json::from_slice::<GamesListResponse>(&bytes) {
        details = list.games;
    } else if let Ok(list) = serde_json::from_slice::<Vec<GameDetail>>(&bytes) {
        details = list;
    } else if let Ok(detail) = serde_json::from_slice::<GameDetail>(&bytes) {
        details.push(detail);
    }

    if details.is_empty() {
        return None;
    }

    for detail in &details {
        let cand_name = detail
            .cleaned_name
            .as_deref()
            .or(detail.name.as_deref())
            .or(detail.title.as_deref())
            .unwrap_or("");
        if !cand_name.is_empty() && !is_title_match(&clean_q, cand_name) {
            continue;
        }
        if let Some(tid) = detail
            .title_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            if let Some(full) = lookup_one(tid) {
                if full.cover_url.is_some() || full.screenshot_url.is_some() || full.logo_url.is_some() {
                    return Some(full);
                }
            }
        }
    }

    for detail in &details {
        if let Some(tid) = detail
            .title_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            if let Some(full) = lookup_one(tid) {
                return Some(full);
            }
        }
    }

    let first = details.into_iter().next()?;
    let canonical_title = first.cleaned_name.or(first.name).or(first.title);
    Some(LookupResult {
        is_game: true,
        canonical_title,
        cover_url: first.cover_path.as_deref().filter(|s| !s.trim().is_empty()).map(asset_url),
        screenshot_url: select_best_variant(&first.heroes),
        logo_url: select_best_logo_variant(&first.logos),
        music_url: select_best_variant(&first.musics),
    })
}

pub fn lookup_batch(games: &[LookupTarget]) -> HashMap<String, LookupResult> {
    let checksum = games_checksum(games);

    if let Some(bytes) = std::fs::read(VITADECK_LOOKUP_CACHE).ok().filter(|b| !b.is_empty()) {
        if let Ok((cached_checksum, results)) =
            serde_json::from_slice::<(u64, HashMap<String, LookupResult>)>(&bytes)
        {
            if cached_checksum == checksum {
                return results;
            }
        }
    }

    if !net::wifi_available() {
        return HashMap::new();
    }

    let mut results = HashMap::with_capacity(games.len());
    for game in games {
        let title_id = crate::scanner::sanitize_sony_title_id(&game.title_id)
            .or_else(|| crate::scanner::sanitize_sony_title_id(&game.art_key))
            .or_else(|| crate::scanner::sanitize_sony_title_id(&game.title));

        let mut result = title_id.as_deref().and_then(lookup_one);

        if result.is_none() || result.as_ref().map(|r| r.cover_url.is_none()).unwrap_or(false) {
            let search_query = if let Some(r) = &result {
                r.canonical_title.as_deref().unwrap_or(&game.title)
            } else {
                &game.title
            };

            if let Some(fallback) = lookup_by_query(search_query, game.system) {
                if let Some(existing) = result.as_mut() {
                    if existing.canonical_title.is_none() {
                        existing.canonical_title = fallback.canonical_title;
                    }
                    if existing.cover_url.is_none() {
                        existing.cover_url = fallback.cover_url;
                    }
                    if existing.screenshot_url.is_none() {
                        existing.screenshot_url = fallback.screenshot_url;
                    }
                    if existing.logo_url.is_none() {
                        existing.logo_url = fallback.logo_url;
                    }
                    if existing.music_url.is_none() {
                        existing.music_url = fallback.music_url;
                    }
                } else {
                    result = Some(fallback);
                }
            }
        }

        if let Some(res) = result {
            results.insert(game.art_key.clone(), res);
        }
    }

    if !results.is_empty() {
        let _ = std::fs::create_dir_all("ux0:data/VitaDeck/APICACHE/");
        if let Ok(bytes) = serde_json::to_vec(&(checksum, &results)) {
            let tmp = format!("{}.part", VITADECK_LOOKUP_CACHE);
            if std::fs::write(&tmp, &bytes).is_ok() {
                let _ = std::fs::rename(&tmp, VITADECK_LOOKUP_CACHE);
            }
        }
    }

    results
}

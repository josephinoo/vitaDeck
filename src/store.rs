use crate::bgdl::{self, BGDL_TYPE_GAME};
use crate::licensing;
use crate::scanner::{Game, System};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::mpsc::{channel, Receiver};

pub const STORE_API_URL: &str = "https://vitaforge.josephinoo.dev/api/v1/vitadeck/store";
pub const STORE_VERSION_URL: &str = "https://vitaforge.josephinoo.dev/api/v1/vitadeck/store/version";
pub const VITAFORGE_ORIGIN: &str = "https://vitaforge.josephinoo.dev";
const MAX_STORE_BYTES: usize = 12 * 1024 * 1024;
const MAX_VERSION_BYTES: usize = 4 * 1024;
pub const MAX_SCREENSHOTS_KEPT: usize = 8;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoreItem {
    #[serde(default)]
    pub title_id: Option<String>,
    #[serde(default)]
    pub content_id: Option<String>,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub category: String,
    #[serde(rename = "type", default)]
    pub kind: String,
    #[serde(default)]
    pub download_url: String,
    #[serde(default)]
    pub zrif: Option<String>,
    #[serde(default)]
    pub size: Option<String>,
    #[serde(default)]
    pub region: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub cover_url: Option<String>,
    #[serde(default)]
    pub icon_url: Option<String>,
    #[serde(default)]
    pub icon_hash: Option<String>,
    #[serde(default)]
    pub background_url: Option<String>,
    #[serde(default)]
    pub screenshot_urls: Vec<String>,
    #[serde(default)]
    pub description: Option<String>,
}

impl StoreItem {
    pub fn has_valid_icon(&self) -> bool {
        self.icon_hash
            .as_deref()
            .map(|h| {
                let t = h.trim();
                !t.is_empty() && t != "default"
            })
            .unwrap_or(false)
    }

    pub fn resolved_cover_url(&self) -> Option<String> {
        if let Some(url) = nonempty(self.cover_url.as_deref()) {
            return Some(absolute_url(url));
        }
        if let Some(url) = self
            .screenshot_urls
            .iter()
            .find_map(|s| nonempty(Some(s.as_str())))
        {
            return Some(absolute_url(url));
        }
        if self.has_valid_icon() {
            if let Some(url) = nonempty(self.icon_url.as_deref()) {
                return Some(absolute_url(url));
            }
        }
        None
    }

    pub fn resolved_icon_url(&self) -> Option<String> {
        if !self.has_valid_icon() {
            return None;
        }
        nonempty(self.icon_url.as_deref()).map(absolute_url)
    }

    pub fn resolved_background_url(&self) -> Option<String> {
        nonempty(self.background_url.as_deref()).map(absolute_url)
    }

    pub fn resolved_screenshot_urls(&self) -> Vec<String> {
        self.screenshot_urls
            .iter()
            .filter_map(|s| nonempty(Some(s.as_str())))
            .map(absolute_url)
            .take(MAX_SCREENSHOTS_KEPT)
            .collect()
    }

    pub fn description_text(&self) -> Option<&str> {
        nonempty(self.description.as_deref())
    }

    pub fn size_label(&self) -> Option<String> {
        let n: u64 = nonempty(self.size.as_deref())?.parse().ok()?;
        if n >= 1_000_000_000 {
            Some(format!("{:.1} GB", n as f64 / 1_000_000_000.0))
        } else if n >= 1_000_000 {
            Some(format!("{:.0} MB", n as f64 / 1_000_000.0))
        } else if n >= 1_000 {
            Some(format!("{:.0} KB", n as f64 / 1_000.0))
        } else {
            Some(format!("{n} B"))
        }
    }
}

fn nonempty(s: Option<&str>) -> Option<&str> {
    s.map(str::trim).filter(|s| !s.is_empty())
}

pub fn absolute_url(path: &str) -> String {
    let trimmed = path.trim();
    let joined = if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        trimmed.to_string()
    } else if trimmed.starts_with('/') {
        format!("{VITAFORGE_ORIGIN}{trimmed}")
    } else {
        format!("{VITAFORGE_ORIGIN}/{trimmed}")
    };
    force_format(&joined)
}

/// Stable public artwork route. Unlike catalog-relative paths this remains
/// valid across catalog cache revisions and works for every PS Vita title ID.
pub fn cover_url_for_title_id(title_id: &str) -> Option<String> {
    let id = normalize_title_id(title_id);
    if id.len() != 9 || !id.bytes().all(|byte| byte.is_ascii_alphanumeric()) {
        return None;
    }
    Some(format!("{VITAFORGE_ORIGIN}/api/v1/images/cover/{id}?size=large&format=jpeg"))
}

fn image_format_for(url: &str) -> &'static str {
    if url.contains("/images/icon/") {
        "png"
    } else {
        "jpeg"
    }
}

fn force_format(url: &str) -> String {
    if url.contains("/scraped_assets/") || !url.contains("/images/") {
        return url.to_string();
    }
    let wanted = image_format_for(url);
    let Some(start) = url.find("format=") else {
        let separator = if url.contains('?') { '&' } else { '?' };
        return format!("{url}{separator}format={wanted}");
    };
    let value_start = start + "format=".len();
    let value_end = url[value_start..].find('&').map_or(url.len(), |i| value_start + i);
    format!("{}{wanted}{}", &url[..value_start], &url[value_end..])
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotResponse {
    pub data: Vec<StoreItem>,
}

#[derive(Deserialize)]
struct VersionResponse {
    #[serde(default)]
    etag: String,
}

pub struct StoreManager {
    pub items: Vec<StoreItem>,
    pub loading: bool,
    title_index: HashMap<String, usize>,
    rx: Option<Receiver<Option<LoadedCatalog>>>,
}

struct LoadedCatalog {
    items: Vec<StoreItem>,
    games: Vec<Game>,
    title_index: HashMap<String, usize>,
}

impl LoadedCatalog {
    fn new(items: Vec<StoreItem>) -> Self {
        let title_index = items
            .iter()
            .enumerate()
            .filter_map(|(index, item)| {
                item.title_id
                    .as_deref()
                    .map(normalize_title_id)
                    .map(|title_id| (title_id, index))
            })
            .collect();
        let games = games_from_items(&items);
        Self { items, games, title_index }
    }
}

fn normalize_title_id(title_id: &str) -> String {
    title_id.trim().to_ascii_uppercase()
}

impl Default for StoreManager {
    fn default() -> Self {
        Self::new()
    }
}

fn cache_path(name: &str) -> String {
    #[cfg(target_os = "vita")]
    {
        format!("ux0:data/VitaDeck/{name}")
    }
    #[cfg(not(target_os = "vita"))]
    {
        format!("data/{name}")
    }
}

impl StoreManager {
    pub fn new() -> Self {
        let (tx, rx) = channel();

        std::thread::spawn(move || {
            let _ = tx.send(Self::load_or_fetch_remote());
        });

        Self {
            // Disk parsing and the version probe both stay off the UI thread.
            items: Vec::new(),
            loading: true,
            title_index: HashMap::new(),
            rx: Some(rx),
        }
    }

    pub fn tick(&mut self) -> Option<Vec<Game>> {
        if let Some(rx) = &self.rx {
            if let Ok(maybe_catalog) = rx.try_recv() {
                self.loading = false;
                self.rx = None;
                if let Some(catalog) = maybe_catalog {
                    self.items = catalog.items;
                    self.title_index = catalog.title_index;
                    return Some(catalog.games);
                }
            }
        }
        None
    }

    pub fn item_by_title_id(&self, title_id: &str) -> Option<&StoreItem> {
        self.index_by_title_id(title_id).and_then(|index| self.items.get(index))
    }

    pub fn index_by_title_id(&self, title_id: &str) -> Option<usize> {
        self.title_index
            .get(title_id)
            .or_else(|| self.title_index.get(&normalize_title_id(title_id)))
            .copied()
    }

    pub fn load_cache() -> Option<Vec<StoreItem>> {
        let path = cache_path("store_catalog.json");
        let file = std::fs::File::open(&path).ok()?;
        let reader = std::io::BufReader::new(file);
        let items: Vec<StoreItem> = if let Ok(resp) = serde_json::from_reader::<_, SnapshotResponse>(reader) {
            resp.data
        } else {
            let file = std::fs::File::open(path).ok()?;
            serde_json::from_reader(std::io::BufReader::new(file)).ok()?
        };
        Some(Self::filter_items(items))
    }

    pub fn save_cache(items: &[StoreItem]) {
        let path = cache_path("store_catalog.json");
        if let Some(parent) = std::path::Path::new(&path).parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(bytes) = serde_json::to_vec(items) {
            let _ = std::fs::write(path, bytes);
        }
    }

    fn load_etag() -> Option<String> {
        let raw = std::fs::read_to_string(cache_path("store_catalog.etag")).ok()?;
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    }

    fn save_etag(etag: &str) {
        let path = cache_path("store_catalog.etag");
        if let Some(parent) = std::path::Path::new(&path).parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(path, etag);
    }

    fn fetch_etag() -> Option<String> {
        let bytes = match crate::net::download_to_vec(STORE_VERSION_URL, MAX_VERSION_BYTES) {
            Ok(bytes) => bytes,
            Err(error) => {
                crate::logger::log(&format!("StoreManager: version probe request failed: {error}"));
                return None;
            }
        };
        let resp: VersionResponse = match serde_json::from_slice(&bytes) {
            Ok(response) => response,
            Err(error) => {
                crate::logger::log(&format!("StoreManager: invalid version response: {error}"));
                return None;
            }
        };
        nonempty(Some(resp.etag.as_str())).map(|s| s.to_string())
    }

    fn is_ps_vita_game(item: &StoreItem) -> bool {
        if item.download_url.is_empty() {
            return false;
        }
        let title_id = item.title_id.as_deref().unwrap_or("").trim().to_uppercase();
        if title_id.is_empty() {
            return false;
        }

        let kind = item.kind.trim().to_lowercase();
        let cat = item.category.trim().to_lowercase();
        let is_psv_type = kind == "psv_game" || kind == "vita_app" || cat == "ps vita game";

        let is_vita_title_id = title_id.starts_with("PCS")
            || is_psv_type
            || (title_id.len() == 9
                && !title_id.starts_with("NPU")
                && !title_id.starts_with("NPE")
                && !title_id.starts_with("NPH")
                && !title_id.starts_with("NPJ")
                && !title_id.starts_with("NPA")
                && !title_id.starts_with("ULU")
                && !title_id.starts_with("ULE")
                && !title_id.starts_with("ULJ")
                && !title_id.starts_with("UCU")
                && !title_id.starts_with("UCE"));
        if !is_vita_title_id {
            return false;
        }

        let name_lower = item.name.to_lowercase();
        if name_lower.contains("(minis)")
            || name_lower.contains("(psp)")
            || name_lower.contains("(psx)")
            || name_lower.contains("(ps1)")
            || name_lower.contains("[dlc]")
            || name_lower.contains("theme")
        {
            return false;
        }

        if kind == "psp_game" || kind == "psp_app" || kind == "psx_game" || kind == "plugin" || kind == "theme" {
            return false;
        }

        if cat == "theme"
            || cat == "themes"
            || cat == "plugin"
            || cat == "plugins"
            || cat == "tool"
            || cat == "tools"
            || cat == "psp game"
            || cat == "ps1 game"
        {
            return false;
        }

        true
    }

    fn filter_items(items: Vec<StoreItem>) -> Vec<StoreItem> {
        let mut seen = std::collections::HashSet::new();
        let mut filtered: Vec<StoreItem> = items
            .into_iter()
            .filter(Self::is_ps_vita_game)
            .filter(|item| {
                if let Some(id) = &item.title_id {
                    let norm = id.trim().to_uppercase();
                    seen.insert(norm)
                } else {
                    false
                }
            })
            .collect();
        filtered.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        filtered
    }

    fn load_or_fetch_remote() -> Option<LoadedCatalog> {
        crate::logger::log("StoreManager: fetching store catalog");
        let cached = Self::load_cache();
        let remote_etag = Self::fetch_etag();
        match &remote_etag {
            Some(etag)
                if Self::load_etag().as_ref() == Some(etag)
                    && cached.as_ref().is_some_and(|c| !c.is_empty()) =>
            {
                crate::logger::log("StoreManager: catalog etag unchanged, using cache");
                return cached.map(LoadedCatalog::new);
            }
            None if cached.as_ref().is_some_and(|c| !c.is_empty()) => {
                crate::logger::log("StoreManager: version probe failed, keeping cache");
                return cached.map(LoadedCatalog::new);
            }
            None => crate::logger::log("StoreManager: version probe failed, downloading store"),
            Some(_) => crate::logger::log("StoreManager: catalog changed, downloading store"),
        }

        let raw_path = cache_path("store_catalog.raw.json");
        if let Some(parent) = std::path::Path::new(&raw_path).parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Err(e) = crate::net::download_to_file(STORE_API_URL, &raw_path, MAX_STORE_BYTES) {
            crate::logger::log(&format!("StoreManager: fetch_remote error: {e}"));
            return cached.map(LoadedCatalog::new);
        }

        let file = match std::fs::File::open(&raw_path) {
            Ok(f) => f,
            Err(e) => {
                crate::logger::log(&format!("StoreManager: open store catalog error: {e}"));
                return cached.map(LoadedCatalog::new);
            }
        };
        let items = match serde_json::from_reader::<_, SnapshotResponse>(std::io::BufReader::new(file)) {
            Ok(snapshot) => snapshot.data,
            Err(e) => {
                crate::logger::log(&format!("StoreManager: failed to parse store catalog: {e}"));
                return cached.map(LoadedCatalog::new);
            }
        };
        let filtered = Self::filter_items(items);
        crate::logger::log(&format!(
            "StoreManager: loaded {} unique PS Vita games from store",
            filtered.len()
        ));
        Self::save_cache(&filtered);
        if let Some(etag) = remote_etag {
            Self::save_etag(&etag);
        }
        let _ = std::fs::remove_file(&raw_path);
        Some(LoadedCatalog::new(filtered))
    }

    pub fn to_games(&self) -> Vec<Game> {
        games_from_items(&self.items)
    }

    pub fn download_and_install(&self, title_id: &str) -> anyhow::Result<()> {
        let item = self
            .item_by_title_id(title_id)
            .ok_or_else(|| anyhow::anyhow!("Game '{title_id}' not found in store catalog"))?;

        let content_id = licensing::resolve_content_id(
            title_id,
            item.content_id.as_deref(),
            &item.download_url,
            item.region.as_deref(),
        )?;

        let rif = licensing::get_license(
            title_id,
            item.content_id.as_deref(),
            &item.download_url,
            item.zrif.as_deref(),
            item.region.as_deref(),
        );

        if let Some(rif_bytes) = &rif {
            let _ = licensing::install_license(&content_id, rif_bytes);
        }

        bgdl::start_bgdl(&item.name, &item.download_url, rif.as_deref(), BGDL_TYPE_GAME)?;
        crate::logger::log(&format!("StoreManager: enqueued download for '{}' ({title_id})", item.name));
        Ok(())
    }
}

fn games_from_items(items: &[StoreItem]) -> Vec<Game> {
    items
        .iter()
        .filter_map(|item| {
            let title_id = item.title_id.as_ref()?.clone();
            Some(Game {
                system: System::Vita,
                title: item.name.clone(),
                title_id: title_id.clone(),
                art_key: title_id,
                cover_bytes: None,
                hero_bytes: None,
                logo_bytes: None,
                has_box_art: false,
                has_hero: false,
                has_logo: false,
                has_bubble: false,
                file_path: None,
                music_path: None,
                music_resolved: false,
                is_game: Some(true),
            })
        })
        .collect()
}

#[cfg(test)]
mod catalog_tests {
    use super::*;

    #[test]
    fn loaded_catalog_builds_games_and_case_insensitive_index() {
        let item: StoreItem = serde_json::from_str(
            r#"{"title_id":"PCSE00001","name":"Test","region":"US","download_url":"https://example.com/game"}"#,
        )
        .unwrap();
        let catalog = LoadedCatalog::new(vec![item]);
        assert_eq!(catalog.games.len(), 1);
        assert_eq!(catalog.title_index.get("PCSE00001"), Some(&0));
        assert_eq!(normalize_title_id(" pcse00001 "), "PCSE00001");
    }

    #[test]
    fn large_catalog_is_fully_indexed_before_delivery() {
        let items: Vec<StoreItem> = (0..4_000)
            .map(|index| {
                serde_json::from_str(&format!(
                    r#"{{"title_id":"PCSE{index:05}","name":"Game {index}","region":"US","download_url":"https://example.com/{index}"}}"#
                ))
                .unwrap()
            })
            .collect();
        let catalog = LoadedCatalog::new(items);
        assert_eq!(catalog.games.len(), 4_000);
        assert_eq!(catalog.title_index.len(), 4_000);
        assert_eq!(catalog.title_index.get("PCSE03999"), Some(&3_999));
    }
}

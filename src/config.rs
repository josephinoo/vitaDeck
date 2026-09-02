use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use crate::i18n::Locale;

pub const CONFIG_FILE: &str = "ux0:data/VitaDeck/config.json";

pub const BUDGET_OPTIONS_MB: &[u32] = &[50, 100, 150, 300, 0]; 
pub const STORE_REGIONS: &[&str] = &["US", "EU", "JP", "ASIA", "INT"];

/// Persistent ordering for installed-library tabs. Store results keep their
/// catalogue order because it is curated separately by the service.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum LibrarySort {
    #[default]
    Name,
    MostPlayed,
    RecentlyPlayed,
    System,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum LibraryView {
    #[default]
    CoverFlow,
    List,
}

impl LibraryView {
    pub const fn next(self) -> Self {
        match self {
            Self::CoverFlow => Self::List,
            Self::List => Self::CoverFlow,
        }
    }

    pub const fn message_key(self) -> &'static str {
        match self {
            Self::CoverFlow => "view-coverflow",
            Self::List => "view-list",
        }
    }
}

impl LibrarySort {
    pub fn next(self) -> Self {
        match self {
            Self::Name => Self::MostPlayed,
            Self::MostPlayed => Self::RecentlyPlayed,
            Self::RecentlyPlayed => Self::System,
            Self::System => Self::Name,
        }
    }

    pub const fn message_key(self) -> &'static str {
        match self {
            Self::Name => "sort-name",
            Self::MostPlayed => "sort-most-played",
            Self::RecentlyPlayed => "sort-recent",
            Self::System => "sort-system",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default = "default_download_bgm")]
    pub download_bgm: bool,
    #[serde(default = "default_cache_budget_mb")]
    pub cache_budget_mb: u32,
    #[serde(default)]
    pub client_id: String,
    #[serde(default)]
    pub locale: Locale,
    #[serde(default)]
    pub disabled_scan_dirs: Vec<String>,
    #[serde(default)]
    pub tab_order: Vec<String>,
    #[serde(default)]
    pub store_regions: Vec<String>,
    #[serde(default)]
    pub library_sort: LibrarySort,
    #[serde(default)]
    pub library_view: LibraryView,
    #[serde(default)]
    pub custom_scan_dirs: Vec<String>,
    #[serde(default)]
    pub hidden_title_ids: Vec<String>,
    #[serde(default)]
    pub title_overrides: HashMap<String, String>,
}

fn default_download_bgm() -> bool {
    true
}

fn default_cache_budget_mb() -> u32 {
    150
}

impl Default for Config {
    fn default() -> Self {
        Self {
            download_bgm: default_download_bgm(),
            cache_budget_mb: default_cache_budget_mb(),
            client_id: String::new(),
            locale: Locale::default(),
            disabled_scan_dirs: Vec::new(),
            tab_order: Vec::new(),
            store_regions: Vec::new(),
            library_sort: LibrarySort::default(),
            library_view: LibraryView::default(),
            custom_scan_dirs: Vec::new(),
            hidden_title_ids: Vec::new(),
            title_overrides: HashMap::new(),
        }
    }
}

fn generate_client_id() -> String {
    let mut bytes = [0u8; 16];
    #[cfg(target_os = "vita")]
    unsafe {
        vitasdk_sys::sceKernelGetRandomNumber(bytes.as_mut_ptr() as *mut _, bytes.len() as u32);
    }
    #[cfg(not(target_os = "vita"))]
    {
        use std::time::{SystemTime, UNIX_EPOCH};
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        let seed = nanos ^ (std::process::id() as u128);
        bytes.copy_from_slice(&seed.to_le_bytes());
    }
    let hex: String = bytes.iter().map(|b| format!("{:02x}", b)).collect();
    format!("vitadeck-{hex}")
}

impl Config {
    pub fn load() -> Self {
        let mut config: Config = File::open(CONFIG_FILE)
            .ok()
            .and_then(|mut file| {
                let mut contents = String::new();
                file.read_to_string(&mut contents).ok()?;
                serde_json::from_str(&contents).ok()
            })
            .unwrap_or_default();

        let mut changed = false;
        if config.client_id.is_empty() {
            config.client_id = generate_client_id();
            changed = true;
        }
        let original_regions = config.store_regions.clone();
        config.store_regions.retain(|region| {
            STORE_REGIONS.iter().any(|allowed| region.eq_ignore_ascii_case(allowed))
        });
        for region in &mut config.store_regions {
            region.make_ascii_uppercase();
        }
        config.store_regions.sort();
        config.store_regions.dedup();
        if config.store_regions != original_regions {
            changed = true;
        }
        // A region is a temporary Store view, not a library preference. Starting
        // from a known "all" view prevents a previously chosen region from
        // making the Store appear incomplete after restarting the app.
        if !config.store_regions.is_empty() {
            config.store_regions.clear();
            changed = true;
        }
        if changed {
            config.save();
        }
        config
    }

    pub fn save(&self) {
        let _ = std::fs::create_dir_all("ux0:data/VitaDeck");
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let tmp = format!("{}.part", CONFIG_FILE);
            if std::fs::write(&tmp, json.as_bytes()).is_ok() {
                let _ = std::fs::rename(&tmp, CONFIG_FILE);
            }
        }
    }

    pub fn budget_label(&self) -> &'static str {
        match self.cache_budget_mb {
            50 => "50 MB",
            100 => "100 MB",
            150 => "150 MB",
            300 => "300 MB",
            0 => "budget-unlimited",
            _ => "budget-custom",
        }
    }

    pub fn toggle_scan_dir(&mut self, path: &str) {
        if let Some(idx) = self.disabled_scan_dirs.iter().position(|d| d == path) {
            self.disabled_scan_dirs.remove(idx);
        } else {
            self.disabled_scan_dirs.push(path.to_string());
        }
        self.save();
    }

    pub fn add_custom_scan_dir(&mut self, path: &str) -> bool {
        let path = path.trim().replace('\\', "/");
        if path.is_empty() || self.custom_scan_dirs.iter().any(|entry| entry.eq_ignore_ascii_case(&path)) {
            return false;
        }
        self.custom_scan_dirs.push(path);
        self.custom_scan_dirs.sort();
        self.save();
        true
    }

    pub fn remove_custom_scan_dir(&mut self, index: usize) -> bool {
        if index >= self.custom_scan_dirs.len() { return false; }
        self.custom_scan_dirs.remove(index);
        self.save();
        true
    }

    pub fn next_budget_option(&mut self) {
        let current = self.cache_budget_mb;
        let idx = BUDGET_OPTIONS_MB.iter().position(|&b| b == current).unwrap_or(2);
        let next_idx = (idx + 1) % BUDGET_OPTIONS_MB.len();
        self.cache_budget_mb = BUDGET_OPTIONS_MB[next_idx];
        self.save();
    }

    pub fn store_region_selected(&self, region: &str) -> bool {
        self.store_regions.iter().any(|selected| selected == region)
    }

    pub fn toggle_store_region(&mut self, region: &str) {
        if !STORE_REGIONS.contains(&region) {
            return;
        }
        if let Some(index) = self.store_regions.iter().position(|selected| selected == region) {
            self.store_regions.remove(index);
        } else {
            self.store_regions.push(region.to_owned());
            self.store_regions.sort();
        }
        self.save();
    }

    pub fn clear_store_regions(&mut self) {
        if !self.store_regions.is_empty() {
            self.store_regions.clear();
            self.save();
        }
    }

    pub fn cycle_library_sort(&mut self) {
        self.library_sort = self.library_sort.next();
        self.save();
    }

    pub fn cycle_library_view(&mut self) {
        self.library_view = self.library_view.next();
        self.save();
    }

    pub fn title_override(&self, title_id: &str) -> Option<&str> {
        self.title_overrides
            .get(&title_id.trim().to_ascii_uppercase())
            .map(String::as_str)
    }

    pub fn set_title_override(&mut self, title_id: &str, title: &str) {
        let key = title_id.trim().to_ascii_uppercase();
        let title = title.trim();
        if title.is_empty() {
            self.title_overrides.remove(&key);
        } else {
            self.title_overrides.insert(key, title.to_string());
        }
        self.save();
    }

    pub fn title_is_hidden(&self, title_id: &str) -> bool {
        self.hidden_title_ids
            .iter()
            .any(|id| id.eq_ignore_ascii_case(title_id.trim()))
    }

    pub fn toggle_hidden_title(&mut self, title_id: &str) {
        let key = title_id.trim().to_ascii_uppercase();
        if let Some(index) = self
            .hidden_title_ids
            .iter()
            .position(|id| id.eq_ignore_ascii_case(&key))
        {
            self.hidden_title_ids.remove(index);
        } else {
            self.hidden_title_ids.push(key);
            self.hidden_title_ids.sort();
        }
        self.save();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_config_defaults_to_all_store_regions() {
        let config: Config = serde_json::from_str(
            r#"{"download_bgm":true,"cache_budget_mb":150,"client_id":"existing","locale":"en-US"}"#,
        )
        .unwrap();
        assert!(config.store_regions.is_empty());
    }
}

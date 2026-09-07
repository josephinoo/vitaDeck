use crate::artwork::{self, ArtJob, ArtResult};
use crate::collections::Collections;
use crate::input::{AppCommand, InputCommand};
use crate::i18n::{Locale, Localizer};
use crate::net;
use crate::recent::RecentlyPlayed;
use crate::scanner::{
    load_cached_cover, load_cached_hero, load_cached_logo, resolved_music_path, scan_installed_games, Game,
    ImageBytes, System,
};
use crate::store::StoreItem;
use crate::stats::GameStatsStore;
use crate::textures::TextureCache;
use crate::ui::{card_row_stride, Mode, FOOTER_H, GRID_COLS, HEADER_H, SCREEN_H, SCREEN_W};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::sync::{mpsc, Arc, Mutex};

const SCROLL_LERP: f32 = 0.22;

const MAX_ART_JOBS_IN_FLIGHT: usize = 2;

// Store artwork is served remotely.  Keep it deliberately serial so a fast
// scroll cannot turn into a burst that gets the Vita's IP rate-limited.
const STORE_ART_JOBS_IN_FLIGHT: usize = 1;
const STORE_SCROLL_SETTLE_FRAMES: u64 = 12;
const STORE_ART_INTERVAL_FRAMES: u64 = 18;
const ART_DOWNLOAD_WORKERS: usize = 2;

const COVER_KEEP_RADIUS: usize = 18;

const ART_LOOKAHEAD: usize = 10;

const STORE_SHOT_BUDGET: usize = 2;
const STORE_SHOT_KEEP_RADIUS: usize = 2;
const MEMORY_POLL_FRAMES: u64 = 30;
const MAX_ART_RESULTS_PER_FRAME: usize = 2;
const MAX_LOCAL_ART_RESULTS_PER_FRAME: usize = 2;
const ART_RELEASE_INTERVAL_FRAMES: u64 = 12;
const WIFI_POLL_INTERVAL_FRAMES: u64 = 60;

#[derive(Clone, Debug, PartialEq, Eq)]
enum TabTarget {
    All,
    Vita,
    Psp,
    Ps1,
    Recent,
    Store,
    Collection(String),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ArtState {
    Pending,
    Done,
    Failed,
}

#[derive(Clone, Default)]
struct KnownArt {
    cover_url: Option<String>,
    screenshot_url: Option<String>,
    logo_url: Option<String>,
    music_url: Option<String>,
}

struct LocalArtRequest {
    title_id: String,
    art_key: String,
    system: System,
    cover: bool,
    hero: bool,
    logo: bool,
}

struct LocalArtResult {
    title_id: String,
    art_key: String,
    cover: Option<ImageBytes>,
    hero: Option<ImageBytes>,
    logo: Option<ImageBytes>,
    cover_requested: bool,
    hero_requested: bool,
    logo_requested: bool,
}

#[derive(Clone)]
pub struct DownloadConfirmState {
    pub title_id: String,
    pub title: String,
    pub selected_choice: usize,
}

pub struct StoreDetailState {
    pub title_id: String,
    pub screenshot_urls: Vec<String>,
    pub screenshots: Vec<Option<ImageBytes>>,
    pending: HashSet<usize>,
    pub lightbox: Option<usize>,
    pub selected_shot: usize,
    pub hero_bytes: Option<ImageBytes>,
    looking_up_heroes: bool,
}

pub struct App {
    pub games: Vec<Game>,
    installed_title_ids: HashSet<String>,
    pub store: crate::store::StoreManager,
    pub store_games: Vec<Game>,
    store_filtered_count: usize,
    pub collections: Collections,
    pub recent: RecentlyPlayed,
    stats: GameStatsStore,
    game_index_by_title_id: HashMap<String, usize>,
    tab_counts: Vec<usize>,
    safe_mode: bool,
    pub mode: Mode,
    pub active_tab: usize,
    pub selected: usize,
    pub picker_index: usize,
    pub current_scroll: f32,
    pub tabs: Vec<String>,
    tab_targets: Vec<TabTarget>,
    pub visible: Vec<usize>,
    pub net_line: String,
    pub scan_log: Vec<String>,
    texture_cache: RefCell<TextureCache>,
    art_state: HashMap<String, ArtState>,

    known_art: HashMap<String, KnownArt>,

    art_jobs_in_flight: usize,
    store_last_selected: Option<usize>,
    store_next_art_frame: u64,
    request_tx: Option<mpsc::SyncSender<ArtJob>>,
    done_rx: Option<mpsc::Receiver<ArtResult>>,
    scan_rx: Option<mpsc::Receiver<(Vec<Game>, Vec<String>)>>,
    lookup_rx: Option<mpsc::Receiver<HashMap<String, artwork::LookupResult>>>,
    net_ready: bool,
    wifi_connected_cached: bool,
    dl_ok: u32,
    dl_fail: u32,
    last_error: String,
    pub audio: crate::audio::AudioEngine,

    music_selected: Option<usize>,
    music_settle_frames: u32,
    pub search_query: String,
    pub search_active: bool,
    pub ime: crate::ime::ImeDialog,
    pub launch_notice: Option<(String, String)>,
    local_art_tx: mpsc::SyncSender<LocalArtRequest>,
    local_art_rx: mpsc::Receiver<LocalArtResult>,
    local_art_pending: HashSet<String>,

    pub store_detail: Option<StoreDetailState>,
    shot_tx: Option<mpsc::SyncSender<(String, i32, String)>>,
    shot_rx: Option<mpsc::Receiver<(String, i32, Option<ImageBytes>)>>,
    hero_lookup_rx: Option<mpsc::Receiver<(String, Vec<String>)>>,
    hero_lookup_tx: mpsc::Sender<(String, Vec<String>)>,

    pub config: crate::config::Config,
    pub i18n: Localizer,
    pub cache_stats: crate::cache_manager::CacheStats,
    pub settings_selected: usize,
    tab_before_settings: usize,
    music_requested: std::collections::HashSet<String>,
    music_retry_after: HashMap<String, u64>,
    frame_counter: u64,
    pub runtime: crate::runtime::VitaRuntime,
    pub cache_notice: Option<String>,
    cache_notice_started_frame: u64,
    pub download_confirm: Option<DownloadConfirmState>,
    // BGDL owns transfers once submitted. Keep a small in-session mirror so
    // an accidental double press cannot enqueue the same title twice.
    queued_downloads: HashSet<String>,
    pub scan_folders: Vec<String>,
    collection_create_pending: bool,
    rename_pending: Option<String>,
    loaded_store_indices: HashSet<usize>,
}

impl App {
    pub fn new(recovered_from_crash: bool) -> Self {
        crate::logger::log("App::new: start");
        let net_status = net::init();
        let net_ready = net_status.is_ok();
        // An uncleared session marker is common when the Vita suspends or the
        // application is closed from the system menu. It must not turn off
        // artwork or background music on the next normal launch.
        let network_work_enabled = net_ready;
        let net_line = match &net_status {
            Ok(()) => "NET ok".to_string(),
            Err(e) => format!("NET error: {}", e),
        };
        let wifi_connected_cached = net_ready && net::wifi_available();
        crate::logger::log(&format!("App::new: net={}", net_line));

        let cached_games = crate::scanner::load_cached_games();
        let mut games = cached_games.unwrap_or_default();
        let installed_title_ids = games
            .iter()
            .map(|game| game.title_id.trim().to_ascii_uppercase())
            .collect();

        let config = crate::config::Config::load();
        apply_library_preferences(&mut games, &config);
        // Configure VitaForge before any background network work begins.
        crate::net::set_client_id(config.client_id.clone());
        let disabled_scan_dirs = config.disabled_scan_dirs.clone();
        let custom_scan_dirs = config.custom_scan_dirs.clone();

        let (scan_tx, scan_rx) = mpsc::channel();
        let (lookup_tx, lookup_rx) = mpsc::channel();
        std::thread::spawn(move || {
            crate::logger::log("Background scan thread started");
            let (scanned_games, scan_log) = scan_installed_games(&disabled_scan_dirs, &custom_scan_dirs);
            crate::logger::log(&format!("Scan finished. Found {} games.", scanned_games.len()));
            crate::scanner::save_cached_games(&scanned_games);
            let targets: Option<Vec<artwork::LookupTarget>> = network_work_enabled
                .then(|| scanned_games.iter().map(artwork::LookupTarget::from_game).collect());
            let _ = scan_tx.send((scanned_games, scan_log));

            if let Some(targets) = targets {
                let res = artwork::lookup_batch(&targets);
                let _ = lookup_tx.send(res);
            }
        });

        let scan_log: Vec<String> = Vec::new();
        let mut known_art: HashMap<String, KnownArt> = HashMap::new();

        crate::logger::log("App::new: loading collections/recent/config");
        let collections = Collections::load();
        let recent = RecentlyPlayed::load();
        let stats = GameStatsStore::load();
        let i18n = Localizer::new(config.locale);
        let cache_stats = crate::cache_manager::compute_cache_stats(&games);
        let store = crate::store::StoreManager::new();
        let store_games = store.to_games();
        for item in &store.items {
            if let Some(title_id) = &item.title_id {
                known_art.insert(title_id.clone(), store_known_art(item));
            }
        }
        let (tabs, tab_targets) = build_tabs(&collections, &i18n, &config.tab_order);
        let visible = filter_games(
            &games,
            &store_games,
            &store,
            &collections,
            &recent,
            tab_targets.first(),
            "",
            &config.store_regions,
        );
        crate::logger::log("App::new: collections and config OK");

        let (request_tx, done_rx) = if net_ready {
            let (request_tx, request_rx) = mpsc::sync_channel::<ArtJob>(4);
            let (done_tx, done_rx) = mpsc::channel::<ArtResult>();
            let request_rx = Arc::new(Mutex::new(request_rx));
            for _ in 0..ART_DOWNLOAD_WORKERS {
                let request_rx = Arc::clone(&request_rx);
                let done_tx = done_tx.clone();
                std::thread::spawn(move || loop {
                    let job = match request_rx.lock() {
                        Ok(rx) => rx.recv(),
                        Err(_) => return,
                    };
                    let Ok(job) = job else { return };
                    artwork::resolve(job, &done_tx);
                });
            }
            (Some(request_tx), Some(done_rx))
        } else {
            (None, None)
        };

        let (local_art_tx, local_art_request_rx) = mpsc::sync_channel::<LocalArtRequest>(4);
        let (local_art_result_tx, local_art_rx) = mpsc::channel::<LocalArtResult>();
        std::thread::spawn(move || {
            while let Ok(request) = local_art_request_rx.recv() {
                let cover = request.cover.then(|| load_cached_cover(request.system, &request.art_key)).flatten();
                let hero = request.hero.then(|| load_cached_hero(request.system, &request.art_key)).flatten();
                let logo = request.logo.then(|| load_cached_logo(request.system, &request.art_key)).flatten();
                let _ = local_art_result_tx.send(LocalArtResult {
                    title_id: request.title_id,
                    art_key: request.art_key,
                    cover,
                    hero,
                    logo,
                    cover_requested: request.cover,
                    hero_requested: request.hero,
                    logo_requested: request.logo,
                });
            }
        });
        let (shot_tx, shot_req_rx) = mpsc::sync_channel::<(String, i32, String)>(2);
        let (shot_done_tx, shot_rx) = mpsc::channel();
        std::thread::spawn(move || {
            while let Ok((title_id, index, url)) = shot_req_rx.recv() {
                let bytes = artwork::fetch_image(&url).map(|img| (img.is_png, img.bytes));
                let _ = shot_done_tx.send((title_id, index, bytes));
            }
        });
        let (hero_lookup_tx, hero_lookup_rx) = mpsc::channel();

        let mut app = App {
            games,
            installed_title_ids,
            store,
            store_games,
            store_filtered_count: visible.len(),
            collections,
            recent,
            stats,
            game_index_by_title_id: HashMap::new(),
            tab_counts: Vec::new(),
            safe_mode: false,
            mode: Mode::Browse,
            active_tab: 0,
            selected: 0,
            picker_index: 0,
            current_scroll: 0.0,
            tabs,
            tab_targets,
            visible,
            net_line,
            scan_log,
            texture_cache: RefCell::new(TextureCache::default()),
            art_state: HashMap::new(),
            known_art,
            art_jobs_in_flight: 0,
            store_last_selected: None,
            store_next_art_frame: 0,
            request_tx,
            done_rx,
            scan_rx: Some(scan_rx),
            lookup_rx: network_work_enabled.then_some(lookup_rx),
            net_ready,
            wifi_connected_cached,
            dl_ok: 0,
            dl_fail: 0,
            last_error: String::new(),
            audio: {
                crate::logger::log("App::new: initializing audio engine");
                let engine = crate::audio::AudioEngine::new();
                crate::logger::log("App::new: audio engine OK");
                engine
            },
            music_selected: None,
            music_settle_frames: 0,
            search_query: String::new(),
            search_active: false,
            ime: crate::ime::ImeDialog::new(),
            launch_notice: None,
            local_art_tx,
            local_art_rx,
            local_art_pending: HashSet::new(),
            store_detail: None,
            shot_tx: Some(shot_tx),
            shot_rx: Some(shot_rx),
            hero_lookup_rx: Some(hero_lookup_rx),
            hero_lookup_tx,
            config,
            i18n,
            cache_stats,
            settings_selected: 0,
            tab_before_settings: 0,
            music_requested: std::collections::HashSet::new(),
            music_retry_after: HashMap::new(),
            frame_counter: 0,
            runtime: crate::runtime::VitaRuntime::new(),
            cache_notice: None,
            cache_notice_started_frame: 0,
            download_confirm: None,
            queued_downloads: HashSet::new(),
            scan_folders: Vec::new(),
            collection_create_pending: false,
            rename_pending: None,
            loaded_store_indices: HashSet::new(),
        };
        app.rebuild_library_indexes();
        let _ = recovered_from_crash;
        app
    }

    pub fn is_loading(&self) -> bool {
        self.games.is_empty() && (self.scan_rx.is_some() || self.lookup_rx.is_some())
    }

    fn normalized_title_id(title_id: &str) -> String {
        title_id.trim().to_ascii_uppercase()
    }

    /// Rebuild indexes only after a library or collection mutation, never from
    /// the render path. This removes repeated O(games × collections) work from
    /// the tab bar and recent list.
    fn rebuild_library_indexes(&mut self) {
        self.game_index_by_title_id.clear();
        for (index, game) in self.games.iter().enumerate() {
            self.game_index_by_title_id
                .insert(Self::normalized_title_id(&game.title_id), index);
        }
        self.tab_counts = self
            .tab_targets
            .iter()
            .map(|target| self.count_target(target))
            .collect();
    }

    fn count_target(&self, target: &TabTarget) -> usize {
        match target {
            TabTarget::Recent => self
                .recent
                .order()
                .iter()
                .filter(|id| self.game_index_by_title_id.contains_key(&Self::normalized_title_id(id)))
                .count(),
            TabTarget::Store => self.store_filtered_count,
            TabTarget::Collection(name) => self
                .collection_index_by_name(name)
                .map_or(0, |collection_idx| {
                    self.games
                        .iter()
                        .filter(|game| self.collections.contains(collection_idx, &game.title_id))
                        .count()
                }),
            target => {
                let system = match target {
                    TabTarget::Vita => Some(System::Vita),
                    TabTarget::Psp => Some(System::Psp),
                    TabTarget::Ps1 => Some(System::Psx),
                    TabTarget::All => None,
                    _ => return 0,
                };
                self.games
                    .iter()
                    .filter(|game| game.is_game != Some(false))
                    .filter(|game| system.map_or(true, |current| game.system == current))
                    .count()
            }
        }
    }

    pub fn cache_notice_age_frames(&self) -> Option<u64> {
        self.cache_notice
            .as_ref()
            .map(|_| self.frame_counter.saturating_sub(self.cache_notice_started_frame))
    }

    fn show_cache_notice(&mut self, message: String) {
        self.cache_notice = Some(message);
        self.cache_notice_started_frame = self.frame_counter;
    }

    pub fn text(&self, key: &str) -> &str {
        self.i18n.text(key)
    }

    pub fn locale_name(&self, locale: Locale) -> &str {
        self.text(match locale {
            Locale::EnUs => "language-name-en-US",
            Locale::EsEs => "language-name-es-ES",
            Locale::PtBr => "language-name-pt-BR",
            Locale::FrFr => "language-name-fr-FR",
            Locale::ItIt => "language-name-it-IT",
            Locale::DeDe => "language-name-de-DE",
        })
    }

    pub fn budget_label(&self) -> &str {
        self.text(self.config.budget_label())
    }

    pub fn texture_cache_get(
        &self,
        ctx: &egui::Context,
        key: &str,
        bytes: &ImageBytes,
    ) -> Option<egui::TextureHandle> {
        self.texture_cache.borrow_mut().get_or_decode(ctx, key, bytes)
    }

    pub fn texture_bytes_in_use(&self) -> usize {
        self.texture_cache.borrow().bytes_in_use()
    }

    pub fn texture_budget_bytes(&self) -> usize {
        self.texture_cache.borrow().budget_bytes()
    }

    pub fn texture_decode_pending(&self) -> usize {
        self.texture_cache.borrow().pending_count()
    }

    pub fn art_is_pending(&self, title_id: &str) -> bool {
        matches!(self.art_state.get(title_id), Some(ArtState::Pending) | None)
    }

    pub fn store_item(&self, title_id: &str) -> Option<&StoreItem> {
        self.store.item_by_title_id(title_id)
    }

    pub fn store_region_summary(&self) -> String {
        match self.config.store_regions.as_slice() {
            [] => self.text("all-regions-short").to_owned(),
            [region] => region.clone(),
            regions => {
                let mut args = fluent_bundle::FluentArgs::new();
                args.set("count", regions.len() as i64);
                self.i18n.format("regions-count", Some(&args))
            }
        }
    }

    pub fn is_title_installed(&self, title_id: &str) -> bool {
        self.installed_title_ids.contains(&title_id.trim().to_ascii_uppercase())
    }

    pub fn game_launch_count(&self, title_id: &str) -> u32 {
        self.stats.get(title_id).map_or(0, |stats| stats.launch_count)
    }

    pub fn clock_line(&self) -> String {
        #[cfg(target_os = "vita")]
        unsafe {
            let mut time: vitasdk_sys::SceDateTime = core::mem::zeroed();
            vitasdk_sys::sceRtcGetCurrentClockLocalTime(&mut time);
            format!("{:02}:{:02}", time.hour, time.minute)
        }
        #[cfg(not(target_os = "vita"))]
        "12:00".to_string()
    }

    pub fn battery_pct(&self) -> i32 {
        #[cfg(target_os = "vita")]
        unsafe { vitasdk_sys::scePowerGetBatteryLifePercent().clamp(0, 100) }
        #[cfg(not(target_os = "vita"))]
        100
    }

    pub fn wifi_connected(&self) -> bool {
        self.wifi_connected_cached
    }

    pub fn tab_is_collection(&self) -> bool {
        !self.is_settings() && matches!(self.tab_targets.get(self.active_tab), Some(TabTarget::Collection(_)))
    }

    pub fn active_tab_can_delete(&self) -> bool {
        let Some(TabTarget::Collection(name)) = self.tab_targets.get(self.active_tab) else { return false };
        self.collection_index_by_name(name).is_some_and(|index| !self.collections.is_default(index))
    }

    fn collection_index_by_name(&self, name: &str) -> Option<usize> {
        self.collections.items.iter().position(|collection| collection.name.eq_ignore_ascii_case(name))
    }

    fn persist_tab_order(&mut self) {
        self.config.tab_order = self.tab_targets.iter().map(tab_token).collect();
        self.config.save();
    }

    fn rebuild_tabs(&mut self) {
        let active = self.tab_targets.get(self.active_tab).cloned();
        let (tabs, targets) = build_tabs(&self.collections, &self.i18n, &self.config.tab_order);
        self.tabs = tabs;
        self.tab_targets = targets;
        self.active_tab = active
            .and_then(|target| self.tab_targets.iter().position(|candidate| candidate == &target))
            .unwrap_or(0)
            .min(self.tab_targets.len().saturating_sub(1));
        self.rebuild_library_indexes();
    }

    fn move_active_tab(&mut self, delta: i32) {
        let len = self.tab_targets.len();
        if len < 2 {
            return;
        }
        let next = (self.active_tab as i32 + delta).clamp(0, len as i32 - 1) as usize;
        if next == self.active_tab {
            return;
        }
        self.tab_targets.swap(self.active_tab, next);
        self.tabs.swap(self.active_tab, next);
        self.tab_counts.swap(self.active_tab, next);
        self.active_tab = next;
        self.persist_tab_order();
        self.audio.play(crate::audio::SoundEffect::Navigate);
    }

    fn delete_active_custom_tab(&mut self) {
        let Some(TabTarget::Collection(name)) = self.tab_targets.get(self.active_tab).cloned() else { return };
        let Some(index) = self.collection_index_by_name(&name) else { return };
        if !self.collections.delete_custom(index) {
            return;
        }
        self.tab_targets.remove(self.active_tab);
        self.tabs.remove(self.active_tab);
        if self.active_tab < self.tab_counts.len() {
            self.tab_counts.remove(self.active_tab);
        }
        self.active_tab = self.active_tab.min(self.tab_targets.len().saturating_sub(1));
        self.persist_tab_order();
        self.refilter_visible();
        self.audio.play(crate::audio::SoundEffect::CloseModal);
    }

    fn pump_art_queue(&mut self) {
        if !self.net_ready {
            return;
        }
        if self.is_store_tab() {
            // Navigation is often much faster than an HTTP round trip.  Do
            // not start a cover until the cursor has stopped briefly, then
            // admit just one request every few frames.  Together with the
            // one-job Store budget this eliminates stale queued requests.
            if self.store_last_selected != Some(self.selected) {
                self.store_last_selected = Some(self.selected);
                self.store_next_art_frame = self
                    .frame_counter
                    .saturating_add(STORE_SCROLL_SETTLE_FRAMES);
                return;
            }
            if self.frame_counter < self.store_next_art_frame {
                return;
            }
        } else {
            self.store_last_selected = None;
        }
        if self.art_jobs_in_flight >= self.art_jobs_budget() {
            return;
        }
        let game_index = self.next_art_candidate();
        let Some(game_index) = game_index else { return };

        let is_store = self.is_store_tab();
        let target_pool = if is_store { &self.store_games } else { &self.games };
        let Some(game) = target_pool.get(game_index) else { return };

        let known = self.known_art.get(&game.art_key).cloned().unwrap_or_default();
        let known_music_url = if self.safe_mode || !self.config.download_bgm || game.music_resolved || is_store {
            None
        } else {
            known.music_url
        };

        let store_item = if is_store {
            self.store.item_by_title_id(&game.title_id)
        } else {
            None
        };
        let final_cover_url = if is_store {
            // The catalog artwork URLs can expire or point at a CDN that
            // rejects the Vita TLS client.  The title-ID API is stable and
            // carries our X-Client-ID header, so use it first.  The catalog
            // URL remains a second chance below.
            crate::store::cover_url_for_title_id(&game.title_id)
                .or_else(|| store_item.and_then(|i| i.resolved_cover_url()))
                .or(known.cover_url)
        } else {
            known.cover_url.or_else(|| {
                store_item
                    .and_then(|i| i.cover_url.as_deref())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(crate::store::absolute_url)
            })
        };
        // Store screenshots are fetched lazily by the detail view. Fetching
        // them while rendering the grid doubles request count and can replace
        // a portrait cover with a landscape gameplay image.
        let known_screenshot_url = if is_store { None } else { known.screenshot_url };
        let known_icon_url = if is_store {
            // `resolve` tries this after `known_cover_url`.  Supplying the
            // catalog cover here gives Store a real fallback if the stable
            // title-ID endpoint has no asset, rather than falling straight
            // to the small icon.
            store_item
                .and_then(|i| i.resolved_cover_url())
                .or_else(|| store_item.and_then(|i| i.resolved_icon_url()))
        } else {
            None
        };

        let job = ArtJob {
            title_id: game.title_id.clone(),
            art_key: game.art_key.clone(),
            title: game.title.clone(),
            system: game.system,
            known_cover_url: final_cover_url,
            known_screenshot_url,
            known_logo_url: if is_store { None } else { known.logo_url },
            known_icon_url,
            known_music_url,
            store_only: is_store,
            skip_disk: is_store,
            music_only: false,
        };
        self.art_state.insert(job.title_id.clone(), ArtState::Pending);
        if let Some(tx) = &self.request_tx {
            if tx.try_send(job).is_ok() {
                self.art_jobs_in_flight += 1;
                if is_store {
                    self.store_next_art_frame = self
                        .frame_counter
                        .saturating_add(STORE_ART_INTERVAL_FRAMES);
                }
            }
        }
    }

    fn apply_memory_pressure(&mut self, pressure: crate::runtime::MemoryPressure) {
        use crate::runtime::MemoryPressure::*;
        match pressure {
            Normal => {
                self.texture_cache.borrow_mut().set_pressure_fraction(1.0);
            }
            Warning => {
                self.texture_cache.borrow_mut().set_pressure_fraction(0.75);
            }
            Aggressive => {
                self.texture_cache.borrow_mut().set_pressure_fraction(0.45);
                self.drop_art_outside_keep_set();
            }
            Emergency => {
                self.texture_cache.borrow_mut().set_pressure_fraction(0.2);
                self.drop_art_outside_keep_set();
                self.audio.set_music(None);
                self.music_selected = None;
            }
        }
    }

    fn drop_art_outside_keep_set(&mut self) {
        let keep = self.cover_keep_set();
        let selected_index = self.visible.get(self.selected).copied();
        for (index, game) in self.store_games.iter_mut().enumerate() {
            if Some(index) != selected_index && !keep.contains(&index) {
                game.cover_bytes = None;
                game.hero_bytes = None;
                game.logo_bytes = None;
            }
        }
        for (index, game) in self.games.iter_mut().enumerate() {
            if Some(index) != selected_index && !keep.contains(&index) {
                game.cover_bytes = None;
                game.hero_bytes = None;
                game.logo_bytes = None;
            }
        }
    }

    fn art_lookahead(&self) -> usize {
        use crate::runtime::MemoryPressure::*;
        match self.runtime.pressure() {
            Normal => ART_LOOKAHEAD,
            Warning | Aggressive | Emergency => 0,
        }
    }

    fn art_jobs_budget(&self) -> usize {
        use crate::runtime::MemoryPressure::*;
        let base = if self.is_store_tab() {
            STORE_ART_JOBS_IN_FLIGHT
        } else {
            MAX_ART_JOBS_IN_FLIGHT
        };
        match self.runtime.pressure() {
            Normal => base,
            Warning => base.min(2),
            Aggressive => 1,
            Emergency => 0,
        }
    }

    fn store_keep_slots(&self) -> (usize, usize) {
        if self.visible.is_empty() {
            return (0, 0);
        }
        let stride = card_row_stride(SCREEN_W).max(1.0);
        let view_h = (SCREEN_H - HEADER_H - FOOTER_H).max(1.0);
        let first_row = (self.current_scroll / stride).floor().max(0.0) as usize;
        let start_row = first_row.saturating_sub(1);
        let rows_on_screen = ((view_h / stride).ceil() as usize).max(1);
        let start = start_row.saturating_mul(GRID_COLS);
        let end = (first_row + rows_on_screen + 2)
            .saturating_mul(GRID_COLS)
            .saturating_add(self.art_lookahead())
            .min(self.visible.len());
        (start, end)
    }

    fn next_art_candidate(&mut self) -> Option<usize> {
        let is_store = self.is_store_tab();
        let target_pool = if is_store { &self.store_games } else { &self.games };

        if self.visible.is_empty() {
            return None;
        }

        if self.runtime.pressure() >= crate::runtime::MemoryPressure::Aggressive {
            let index = *self.visible.get(self.selected)?;
            let game = target_pool.get(index)?;
            return (game.cover_bytes.is_none() && !self.art_state.contains_key(&game.title_id))
                .then_some(index);
        }

        let (start, end) = if is_store {
            self.store_keep_slots()
        } else {
            let start = self.selected.saturating_sub(2);
            let end = (self.selected + 4 + self.art_lookahead()).min(self.visible.len());
            (start, end)
        };

        for slot in start..end {
            let Some(&index) = self.visible.get(slot) else { continue };
            let Some(game) = target_pool.get(index) else { continue };
            if game.cover_bytes.is_none() && !self.art_state.contains_key(&game.title_id) {
                return Some(index);
            }
        }

        None
    }

    pub fn refilter_visible(&mut self) {
        if self.is_settings() {
            self.visible = Vec::new();
            self.selected = 0;
            return;
        }
        let selected_index = self.visible.get(self.selected).copied();
        self.visible = filter_games(
            &self.games,
            &self.store_games,
            &self.store,
            &self.collections,
            &self.recent,
            self.tab_targets.get(self.active_tab),
            &self.search_query,
            &self.config.store_regions,
        );
        if !self.is_store_tab() {
            self.visible.retain(|&index| {
                index < self.games.len()
                    && self.games
                        .get(index)
                        .is_some_and(|game| !self.config.title_is_hidden(&game.title_id))
            });
        }
        // The dedicated RECENT tab has an explicit user-history order and the
        // Store follows its catalogue order. Other local tabs use the saved
        // library order.
        if !self.is_store_tab()
            && !matches!(self.tab_targets.get(self.active_tab), Some(TabTarget::Recent))
        {
            let sort = self.config.library_sort;
            let stats = &self.stats;
            self.visible.sort_by(|left, right| {
                let (Some(a), Some(b)) = (self.games.get(*left), self.games.get(*right)) else {
                    return std::cmp::Ordering::Equal;
                };
                let by_name = || a.title.to_lowercase().cmp(&b.title.to_lowercase());
                match sort {
                    crate::config::LibrarySort::Name => by_name(),
                    crate::config::LibrarySort::MostPlayed => stats
                        .get(&b.title_id).map(|v| v.launch_count).unwrap_or(0)
                        .cmp(&stats.get(&a.title_id).map(|v| v.launch_count).unwrap_or(0))
                        .then_with(by_name),
                    crate::config::LibrarySort::RecentlyPlayed => stats
                        .get(&b.title_id).map(|v| v.last_launched_at).unwrap_or(0)
                        .cmp(&stats.get(&a.title_id).map(|v| v.last_launched_at).unwrap_or(0))
                        .then_with(by_name),
                    crate::config::LibrarySort::System => system_sort_key(a.system)
                        .cmp(&system_sort_key(b.system))
                        .then_with(by_name),
                }
            });
        }
        if self.is_store_tab() {
            self.store_filtered_count = self.visible.len();
            if let Some(slot) = self
                .tab_targets
                .iter()
                .position(|target| matches!(target, TabTarget::Store))
            {
                if let Some(count) = self.tab_counts.get_mut(slot) {
                    *count = self.store_filtered_count;
                }
            }
        }
        self.selected = selected_index
            .and_then(|idx| self.visible.iter().position(|&i| i == idx))
            .unwrap_or(0)
            .min(self.visible.len().saturating_sub(1));
    }

    fn apply_pending_lookup(&mut self) {
        if let Some(rx) = &self.scan_rx {
            if let Ok((mut scanned_games, scan_log)) = rx.try_recv() {
                self.scan_rx = None;
                self.scan_log = scan_log;
                apply_library_preferences(&mut scanned_games, &self.config);

                let list_changed = self.games.len() != scanned_games.len()
                    || self.games.iter().zip(scanned_games.iter()).any(|(a, b)| {
                        a.title_id != b.title_id
                            || a.art_key != b.art_key
                            || a.title != b.title
                            || a.system != b.system
                            || a.has_bubble != b.has_bubble
                            || a.file_path != b.file_path
                    });

                if list_changed || self.games.is_empty() {
                    // Repair caches created by the old permissive API matcher.
                    // If the on-device SFO now says a completely unrelated
                    // title for the same key, its downloaded media is unsafe.
                    for scanned in &scanned_games {
                        if let Some(previous) = self.games.iter().find(|game| game.art_key == scanned.art_key) {
                            if previous.title != scanned.title
                                && !artwork::is_title_match(&previous.title, &scanned.title)
                            {
                                let removed = crate::cache_manager::remove_game_art(
                                    scanned.system,
                                    &scanned.art_key,
                                );
                                if removed > 0 {
                                    crate::logger::log(&format!(
                                        "removed {removed} mismatched cached assets for {}: '{}' -> '{}'",
                                        scanned.art_key, previous.title, scanned.title
                                    ));
                                }
                            }
                        }
                    }
                    self.games = scanned_games;
                    self.rebuild_library_indexes();
                    self.refilter_visible();
                } else {
                    // The lightweight manifest can be older than the image files.
                    // Keep the live list, but refresh its cache flags from the scan so
                    // the artwork preloader does not fetch files already on disk.
                    for (game, scanned) in self.games.iter_mut().zip(scanned_games.iter()) {
                        game.has_box_art = scanned.has_box_art;
                        game.has_hero = scanned.has_hero;
                        game.has_logo = scanned.has_logo;
                        game.music_resolved = scanned.music_resolved;
                        game.music_path = scanned.music_path.clone();
                        game.has_bubble = scanned.has_bubble;
                        game.file_path = scanned.file_path.clone();
                    }
                }

                let games_clone = self.games.clone();
                let budget_mb = self.config.cache_budget_mb;
                std::thread::spawn(move || {
                    crate::cache_manager::clean_orphaned_cache(&games_clone);
                    crate::cache_manager::enforce_cache_budget(&games_clone, budget_mb);
                });
                self.installed_title_ids = self
                    .games
                    .iter()
                    .map(|game| game.title_id.trim().to_ascii_uppercase())
                    .collect();
                self.rebuild_library_indexes();
            }
        }

        if let Some(rx) = &self.lookup_rx {
            if let Ok(lookup) = rx.try_recv() {
                self.lookup_rx = None;
                let mut titles_updated = false;

                for game in &mut self.games {
                    if let Some(result) = lookup.get(&game.art_key) {
                        if game.is_game != Some(false) && result.is_game {
                            game.is_game = Some(true);
                        }
                        if let Some(canonical) = &result.canonical_title {
                            let cur_is_id = crate::scanner::sanitize_sony_title_id(&game.title).is_some()
                                || game.title == game.title_id
                                || game.title.to_lowercase().ends_with(".iso")
                                || game.title.to_lowercase().ends_with(".cso");
                            if cur_is_id || (!canonical.trim().is_empty() && (game.title.len() < canonical.len() || crate::scanner::sanitize_sony_title_id(&game.art_key).is_some())) {
                                game.title = canonical.clone();
                                titles_updated = true;
                            }
                        }
                        self.known_art.insert(
                            game.art_key.clone(),
                            KnownArt {
                                cover_url: result.cover_url.clone(),
                                screenshot_url: result.screenshot_url.clone(),
                                logo_url: result.logo_url.clone(),
                                music_url: result.music_url.clone(),
                            },
                        );
                    }
                }

                if titles_updated {
                    self.games.sort_by(|a, b| a.title.to_lowercase().cmp(&b.title.to_lowercase()));
                    crate::scanner::save_cached_games(&self.games);
                }

                self.art_state.clear();

                // Artwork is loaded lazily for the visible window. Scheduling
                // the complete library here makes large collections download
                // and retain several images per game immediately after boot.

                if !self.is_settings() {
                    self.refilter_visible();
                }
            }
        }
    }

    pub fn tick(&mut self, ctx: &egui::Context) {
        self.frame_counter = self.frame_counter.wrapping_add(1);
        if self.frame_counter % WIFI_POLL_INTERVAL_FRAMES == 0 {
            self.wifi_connected_cached = self.net_ready && net::wifi_available();
        }
        // 220 ms enter + 2.8 s hold + 220 ms exit at the target 60 FPS.
        if self.cache_notice_age_frames().is_some_and(|age| age > 195) {
            self.cache_notice = None;
        }
        self.texture_cache.borrow_mut().pump(ctx);
        let (pressure, changed) = if self.frame_counter % MEMORY_POLL_FRAMES == 0 {
            self.runtime.tick()
        } else {
            (self.runtime.pressure(), false)
        };
        if changed {
            crate::logger::log(&format!(
                "memory pressure -> {} ({} MB free)",
                pressure.label(),
                self.runtime.free_memory_bytes() / (1024 * 1024)
            ));
            self.apply_memory_pressure(pressure);
        }
        if self.frame_counter % MEMORY_POLL_FRAMES == 0 {
            let health = format!(
                "HEALTH frame={} mode={:?} free_mb={} pressure={} textures_kb={} decode_pending={} art_in_flight={} local_pending={} visible={} selected={}",
                self.frame_counter,
                self.mode,
                self.runtime.free_memory_bytes() / (1024 * 1024),
                pressure.label(),
                self.texture_bytes_in_use() / 1024,
                self.texture_decode_pending(),
                self.art_jobs_in_flight,
                self.local_art_pending.len(),
                self.visible.len(),
                self.selected,
            );
            crate::logger::set_health_context(health.clone());
            if self.frame_counter % 600 == 0 {
                crate::logger::log(&health);
            }
        }

        self.apply_pending_lookup();

        if let Some(res) = self.ime.poll() {
            match res {
                crate::ime::ImeResult::Confirmed(text) => {
                    let trimmed = text.trim().to_string();
                    if let Some(title_id) = self.rename_pending.take() {
                        self.config.set_title_override(&title_id, &trimmed);
                        apply_library_preferences(&mut self.games, &self.config);
                        self.refilter_visible();
                    } else if self.collection_create_pending {
                        self.collection_create_pending = false;
                        if self.collections.create(&trimmed) {
                            self.rebuild_tabs();
                            self.picker_index = self.collections.items.len().saturating_sub(1);
                            if let Some(title_id) = self
                                .visible
                                .get(self.selected)
                                .and_then(|&index| self.games.get(index))
                                .map(|game| game.title_id.clone())
                            {
                                self.collections.toggle(self.picker_index, &title_id);
                            }
                        }
                    } else if trimmed.is_empty() {
                        self.search_query.clear();
                        self.search_active = false;
                    } else {
                        self.search_query = trimmed;
                        self.search_active = true;
                    }
                    self.refilter_visible();
                }
                crate::ime::ImeResult::Canceled => {
                    self.collection_create_pending = false;
                    self.rename_pending = None;
                    if self.search_query.is_empty() {
                        self.search_active = false;
                    }
                }
            }
        }

        if let Some(store_games) = self.store.tick() {
            for item in &self.store.items {
                if let Some(title_id) = &item.title_id {
                    self.known_art.insert(title_id.clone(), store_known_art(item));
                }
            }
            self.store_games = store_games;
            if self.is_store_tab() {
                self.refilter_visible();
            } else if self.search_query.is_empty() && self.config.store_regions.is_empty() {
                self.store_filtered_count = self.store_games.len();
            } else {
                self.store_filtered_count = filter_store_games(
                    &self.store_games,
                    &self.store,
                    &self.search_query,
                    &self.config.store_regions,
                )
                .len();
            }
        }

        let target_scroll = if self.is_store_tab() {
            let row_stride = card_row_stride(crate::ui::SCREEN_W);
            let selected_row = self.selected / GRID_COLS;
            if selected_row > 1 {
                (selected_row - 1) as f32 * row_stride
            } else {
                0.0
            }
        } else {
            self.selected as f32 * crate::ui::TILE_SPACING
        };
        self.current_scroll += (target_scroll - self.current_scroll) * SCROLL_LERP;
        if (target_scroll - self.current_scroll).abs() < 0.5 {
            self.current_scroll = target_scroll;
        }

        self.pump_art_queue();

        if let Some(rx) = &self.done_rx {
            for _ in 0..MAX_ART_RESULTS_PER_FRAME {
                let Ok(result) = rx.try_recv() else { break };

                let ArtResult {
                    title_id,
                    canonical_title,
                    cover,
                    hero,
                    logo,
                    cover_ok,
                    hero_ok,
                    logo_ok,
                    music_downloaded,
                    error,
                    job_complete,
                    images_ok,
                } = result;

                if job_complete {
                    self.art_jobs_in_flight = self.art_jobs_in_flight.saturating_sub(1);
                }
                let selected_title_id =
                    self.visible.get(self.selected).and_then(|&i| self.games.get(i)).map(|g| g.title_id.clone());

                // Catalog indices are normally aligned with store_games, but
                // a cached catalog can be older than the game list.  Route a
                // downloaded cover by title ID in the actual render list so
                // valid images are never silently discarded.
                let store_index = self
                    .store_games
                    .iter()
                    .position(|game| game.title_id.eq_ignore_ascii_case(&title_id));
                let store_wants_cover = store_index
                    .is_some_and(|index| self.is_store_tab() && self.cover_keep_set().contains(&index));
                let mut cover = cover;
                let mut title_updated = false;

                if let Some(game) = self.games.iter_mut().find(|g| g.title_id == title_id) {
                    if let Some(canonical) = canonical_title
                        .as_deref()
                        .map(str::trim)
                        .filter(|title| !title.is_empty())
                    {
                        let current_is_id = crate::scanner::sanitize_sony_title_id(&game.title).is_some()
                            || game.title.eq_ignore_ascii_case(&game.title_id);
                        if current_is_id && !canonical.eq_ignore_ascii_case(&game.title) {
                            game.title = canonical.to_string();
                            title_updated = true;
                        }
                    }
                    if let Some(is_png) = cover.as_ref().map(|c| c.is_png) {
                        let bytes = if store_wants_cover {
                            cover.as_ref().map(|c| c.bytes.clone()).unwrap_or_default()
                        } else {
                            cover.take().map(|c| c.bytes).unwrap_or_default()
                        };
                        game.cover_bytes = Some((is_png, bytes));
                        self.texture_cache.borrow_mut().invalidate(&format!("{}:{}:cover", game.system.label(), game.title_id));
                    }
                    if cover_ok {
                        game.has_box_art = true;
                    }
                    if let Some(hero) = hero {
                        game.hero_bytes = Some((hero.is_png, hero.bytes));
                        self.texture_cache.borrow_mut().invalidate(&format!("{}:{}:hero", game.system.label(), game.title_id));
                    }
                    if hero_ok {
                        game.has_hero = true;
                    }
                    if let Some(logo) = logo {
                        game.logo_bytes = Some((logo.is_png, logo.bytes));
                        self.texture_cache.borrow_mut().invalidate(&format!("{}:{}:logo", game.system.label(), game.title_id));
                    }
                    if logo_ok {
                        game.has_logo = true;
                    }
                    if music_downloaded {
                        game.music_resolved = true;
                    }
                }

                if title_updated {
                    crate::scanner::save_cached_games(&self.games);
                }

                let mut store_discarded = false;
                if let Some(index) = store_index {
                    if store_wants_cover {
                        if let Some(game) = self.store_games.get_mut(index) {
                            if let Some(cover_bytes) = cover.take() {
                                game.cover_bytes = Some((cover_bytes.is_png, cover_bytes.bytes));
                                self.loaded_store_indices.insert(index);
                                self.texture_cache.borrow_mut().invalidate(&format!(
                                    "{}:{}:cover",
                                    game.system.label(),
                                    game.title_id
                                ));
                            }
                            if cover_ok {
                                game.has_box_art = true;
                            }
                        }
                    } else {
                        store_discarded = true;
                        if job_complete {
                            self.art_state.remove(&title_id);
                        }
                    }
                }

                if music_downloaded && selected_title_id.as_deref() == Some(title_id.as_str()) {
                    self.music_settle_frames = 0;
                }

                if music_downloaded {
                    self.music_requested.remove(&title_id);
                    self.music_retry_after.remove(&title_id);
                } else if !job_complete {
                    self.music_requested.remove(&title_id);
                    self.music_retry_after
                        .insert(title_id.clone(), self.frame_counter.saturating_add(8 * 60));
                }

                if job_complete {
                    if !store_discarded {
                        if images_ok || music_downloaded {
                            self.dl_ok += 1;
                            self.art_state.insert(title_id.clone(), ArtState::Done);
                        } else if error.is_some() {
                            self.dl_fail += 1;
                            self.art_state.insert(title_id.clone(), ArtState::Failed);
                            if let Some(game) = self.games.iter().find(|g| g.title_id == title_id) {
                                if game.cover_bytes.is_none() {
                                    if let Some(reason) = &error {
                                        self.last_error = format!("{} {}", title_id, reason);
                                    }
                                }
                            }
                        }
                    }
                } else if music_downloaded {

                    self.art_state
                        .entry(title_id.clone())
                        .and_modify(|s| *s = ArtState::Done)
                        .or_insert(ArtState::Done);
                } else {

                    if let Some(state) = self.art_state.get_mut(&title_id) {
                        if *state == ArtState::Pending {
                            *state = ArtState::Failed;
                            self.dl_fail += 1;
                        }
                    }
                }

                self.net_line = format!(
                    "NET ok  wifi:{}  ok:{} fail:{}  {}",
                    if self.wifi_connected_cached { "yes" } else { "no" },
                    self.dl_ok,
                    self.dl_fail,
                    self.last_error
                );
            }
        }

        let selected_index = self.visible.get(self.selected).copied();

        for _ in 0..MAX_LOCAL_ART_RESULTS_PER_FRAME {
            let Ok(result) = self.local_art_rx.try_recv() else { break };
            self.local_art_pending.remove(&format!("{}:{}:{}:{}", result.title_id,
                result.cover_requested as u8, result.hero_requested as u8, result.logo_requested as u8));
            if let Some(game) = self.games.iter_mut().find(|g| g.art_key == result.art_key) {
                if let Some(h) = result.hero {
                    game.hero_bytes = Some(h);
                }
                if let Some(l) = result.logo {
                    game.logo_bytes = Some(l);
                }
                if let Some(c) = result.cover.clone() {
                    game.cover_bytes = Some(c);
                }
                if result.cover_requested && game.cover_bytes.is_none() { game.has_box_art = false; }
                if result.hero_requested && game.hero_bytes.is_none() { game.has_hero = false; }
                if result.logo_requested && game.logo_bytes.is_none() { game.has_logo = false; }
            }
            if let Some(index) = self.store.index_by_title_id(&result.title_id) {
                if let Some(game) = self.store_games.get_mut(index) {
                    if let Some(c) = result.cover {
                        game.cover_bytes = Some(c);
                        self.loaded_store_indices.insert(index);
                    }
                }
            }
        }

        if self.music_selected != selected_index {
            self.music_selected = selected_index;
            self.music_settle_frames = 0;
        } else if self.music_settle_frames <= Self::MUSIC_SETTLE_FRAMES {
            self.music_settle_frames += 1;
        }

        if self.music_settle_frames == Self::ART_SETTLE_FRAMES {
            if let Some(selected_index) = selected_index {
                let system = self.games.get(selected_index).map(|g| g.system);
                let art_key = self.games.get(selected_index).map(|g| g.art_key.clone());
                let title_id = self.games.get(selected_index).map(|g| g.title_id.clone());
                let (needs_hero, needs_logo) = self
                    .games
                    .get(selected_index)
                    .map(|g| {
                        (
                            g.hero_bytes.is_none() && g.has_hero,
                            g.logo_bytes.is_none() && g.has_logo,
                        )
                    })
                    .unwrap_or((false, false));

                if let (Some(system), Some(art_key), Some(title_id)) = (system, art_key, title_id) {
                    self.request_local_art(title_id, art_key, system, false, needs_hero, needs_logo);
                }
            }
        }

        let keep = self.cover_keep_set();
        if !self.is_store_tab() {
            for index in keep {
                let game = self.games.get(index).filter(|g| g.cover_bytes.is_none() && g.has_box_art).cloned();
                if let Some(game) = game {
                    self.request_local_art(game.title_id, game.art_key, game.system, true, false, false);
                    break;
                }
            }
        } else {
            for index in keep {
                let game = self.store_games.get(index).filter(|g| g.cover_bytes.is_none()).cloned();
                if let Some(game) = game {
                    self.request_local_art(game.title_id, game.art_key, game.system, true, false, false);
                    break;
                }
            }
        }

        // The Store may contain thousands of entries. Releasing at 5 Hz keeps
        // memory bounded without scanning the complete catalog every frame.
        if self.frame_counter % ART_RELEASE_INTERVAL_FRAMES == 0 {
            self.release_offscreen_art(selected_index);
        }
        self.pump_store_media();
        self.pump_music(selected_index);
    }

    const MUSIC_SETTLE_FRAMES: u32 = 18;

    const ART_SETTLE_FRAMES: u32 = 14;

    fn pump_music(&mut self, selected_index: Option<usize>) {
        if self.safe_mode || self.is_settings() || self.is_store_tab() || selected_index.is_none() {
            self.audio.set_music(None);
            self.music_selected = None;
            self.music_settle_frames = 0;
            return;
        }

        if self.music_settle_frames < Self::MUSIC_SETTLE_FRAMES {
            return;
        }

        let Some(index) = selected_index else { return };
        let Some(game) = self.games.get(index) else { return };

        if let Some(path) = resolved_music_path(game) {
            self.audio.set_music(Some(path.as_str()));
            return;
        }

        self.audio.set_music(None);
        self.request_music_download(index);
    }

    fn request_music_download(&mut self, index: usize) {
        if !self.config.download_bgm
            || !self.net_ready
            || !self.wifi_connected_cached
            || self.runtime.pressure() != crate::runtime::MemoryPressure::Normal
        {
            return;
        }
        let Some(game) = self.games.get(index) else { return };
        if self
            .music_retry_after
            .get(&game.title_id)
            .is_some_and(|&retry_at| self.frame_counter < retry_at)
        {
            return;
        }
        if self.music_requested.contains(&game.title_id) {
            return;
        }
        let job = ArtJob {
            title_id: game.title_id.clone(),
            art_key: game.art_key.clone(),
            title: game.title.clone(),
            system: game.system,
            known_cover_url: None,
            known_screenshot_url: None,
            known_logo_url: None,
            known_icon_url: None,
            known_music_url: self.known_art.get(&game.art_key).and_then(|k| k.music_url.clone()),
            store_only: false,
            skip_disk: false,
            music_only: true,
        };
        let title_id = job.title_id.clone();
        if let Some(tx) = &self.request_tx {
            if tx.try_send(job).is_ok() {
                self.music_requested.insert(title_id);
            }
        }
    }

    fn cover_keep_set(&self) -> std::collections::HashSet<usize> {
        let mut keep = std::collections::HashSet::new();
        if self.is_store_tab() {
            let (start, end) = self.store_keep_slots();
            for slot in start..end {
                if let Some(&index) = self.visible.get(slot) {
                    keep.insert(index);
                }
            }
            return keep;
        }
        let end = (self.selected + COVER_KEEP_RADIUS).min(self.visible.len().saturating_sub(1));
        let start = self.selected.saturating_sub(COVER_KEEP_RADIUS);
        if start <= end {
            for slot in start..=end {
                if let Some(&index) = self.visible.get(slot) {
                    keep.insert(index);
                }
            }
        }
        keep
    }

    fn request_local_art(&mut self, title_id: String, art_key: String, system: System, cover: bool, hero: bool, logo: bool) {
        let key = format!("{}:{}:{}:{}", title_id, cover as u8, hero as u8, logo as u8);
        if !self.local_art_pending.insert(key.clone()) {
            return;
        }
        let request = LocalArtRequest {
            title_id,
            art_key,
            system,
            cover,
            hero,
            logo,
        };
        if self.local_art_tx.try_send(request).is_err() {
            self.local_art_pending.remove(&key);
        }
    }

    fn drop_store_cover(&mut self, index: usize) {
        let Some(game) = self.store_games.get_mut(index) else { return };
        let cover_key = format!("{}:{}:cover", game.system.label(), game.title_id);
        let hero_key = format!("{}:{}:hero", game.system.label(), game.title_id);
        game.cover_bytes = None;
        game.hero_bytes = None;
        let mut cache = self.texture_cache.borrow_mut();
        cache.invalidate(&cover_key);
        cache.invalidate(&hero_key);
        let title_id = game.title_id.clone();
        if !matches!(self.art_state.get(&title_id), Some(ArtState::Pending)) {
            self.art_state.remove(&title_id);
        }
    }

    fn release_offscreen_art(&mut self, selected_index: Option<usize>) {
        let cover_keep = self.cover_keep_set();
        if self.is_store_tab() {
            let drop: Vec<usize> = self
                .loaded_store_indices
                .iter()
                .copied()
                .filter(|&index| Some(index) != selected_index && !cover_keep.contains(&index))
                .collect();
            for index in drop {
                self.loaded_store_indices.remove(&index);
                self.drop_store_cover(index);
            }
            return;
        }
        for (index, game) in self.games.iter_mut().enumerate() {
            if Some(index) == selected_index {
                continue;
            }
            if game.hero_bytes.is_some() {
                game.hero_bytes = None;
            }
            if game.logo_bytes.is_some() {
                game.logo_bytes = None;
            }
            if game.cover_bytes.is_some() && !cover_keep.contains(&index) {
                game.cover_bytes = None;
            }
        }
    }

    pub fn handle_command(&mut self, command: AppCommand) {
        use crate::audio::SoundEffect;
        match command {
            AppCommand::Input(input) => self.handle_input(input),
            AppCommand::TabPrev => {
                if self.mode == Mode::TabOrganizer {
                    self.move_active_tab(-1);
                } else {
                    self.audio.play(SoundEffect::TabSwitch);
                    self.change_tab(-1);
                }
            }
            AppCommand::TabNext => {
                if self.mode == Mode::TabOrganizer {
                    self.move_active_tab(1);
                } else {
                    self.audio.play(SoundEffect::TabSwitch);
                    self.change_tab(1);
                }
            }
            AppCommand::OpenSettings => {
                let settings_index = self.settings_tab_index();
                if self.active_tab == settings_index {
                    self.audio.play(SoundEffect::CloseModal);
                    self.set_tab(self.tab_before_settings);
                } else {
                    self.audio.play(SoundEffect::TabSwitch);
                    self.tab_before_settings = self.active_tab;
                    self.settings_selected = 0;
                    self.set_tab(settings_index);
                }
            }
            AppCommand::SelectTab(index) => {
                if index != self.active_tab {
                    self.audio.play(SoundEffect::TabSwitch);
                }
                self.set_tab(index);
            }
            AppCommand::OpenCollectionPicker => {
                if self.mode == Mode::StoreDetail {
                    self.audio.play(SoundEffect::OpenModal);
                    self.toggle_store_lightbox();
                } else if self.mode == Mode::CollectionPicker {
                    self.handle_command(AppCommand::CreateCollection);
                } else if self.mode == Mode::Browse && !self.visible.is_empty() {
                    self.audio.play(SoundEffect::OpenModal);
                    self.mode = Mode::CollectionPicker;
                    self.picker_index = 0;
                }
            }
            AppCommand::OpenGameOptions => {
                if self.mode == Mode::Browse && !self.is_store_tab() && !self.visible.is_empty() {
                    self.mode = Mode::GameOptions;
                    self.picker_index = 0;
                    self.audio.play(SoundEffect::OpenModal);
                }
            }
            AppCommand::RenameSelectedGame => {
                let Some(game) = self
                    .visible
                    .get(self.selected)
                    .and_then(|&index| self.games.get(index))
                    .cloned()
                else { return };
                let title = self.text("rename-game").to_owned();
                if self.ime.open(&title, &game.title, 64) {
                    self.rename_pending = Some(game.title_id);
                }
            }
            AppCommand::ToggleHideSelectedGame => {
                if let Some(title_id) = self
                    .visible
                    .get(self.selected)
                    .and_then(|&index| self.games.get(index))
                    .map(|game| game.title_id.clone())
                {
                    self.config.toggle_hidden_title(&title_id);
                    self.mode = Mode::Browse;
                    self.refilter_visible();
                    self.audio.play(SoundEffect::Confirm);
                }
            }
            AppCommand::RemoveFromCollection => {
                if self.mode == Mode::TabOrganizer {
                    self.delete_active_custom_tab();
                } else if self.is_store_tab() {
                    self.handle_command(AppCommand::OpenStoreFilters);
                } else if self.mode == Mode::Browse && !self.tab_is_collection() {
                    self.toggle_selected_favorite();
                } else {
                    self.audio.play(SoundEffect::Confirm);
                    self.remove_selected_from_collection();
                }
            }
            AppCommand::CreateCollection => {
                if self.mode != Mode::CollectionPicker || self.ime.is_active() {
                    return;
                }
                let title = self.text("new-tab-title").to_owned();
                self.collection_create_pending = self.ime.open(&title, "", 18);
            }
            AppCommand::ToggleSearch => {
                if self.mode == Mode::Browse {
                    self.audio.play(SoundEffect::OpenModal);
                    self.mode = Mode::TabOrganizer;
                }
            }
            AppCommand::CycleLibrarySort => {
                if self.mode != Mode::Browse || self.is_store_tab() {
                    return;
                }
                self.config.cycle_library_sort();
                self.refilter_visible();
                self.audio.play(SoundEffect::Confirm);
                self.show_cache_notice(format!(
                    "{}: {}",
                    self.text("settings-library-sort"),
                    self.text(self.config.library_sort.message_key())
                ));
            }
            AppCommand::CycleLibraryView => {
                if self.mode != Mode::Browse || self.is_store_tab() {
                    return;
                }
                self.config.cycle_library_view();
                self.current_scroll = 0.0;
                self.audio.play(SoundEffect::Confirm);
                self.show_cache_notice(format!(
                    "{}: {}",
                    self.text("view"),
                    self.text(self.config.library_view.message_key())
                ));
            }
            AppCommand::SearchStore => {
                if !self.is_store_tab()
                    || !matches!(self.mode, Mode::Browse | Mode::StoreFilters)
                    || self.ime.is_active()
                {
                    return;
                }
                self.audio.play(SoundEffect::OpenModal);
                let title = self.text("search-store").to_owned();
                self.ime.open(&title, &self.search_query, 64);
            }
            AppCommand::OpenStoreFilters => {
                if self.is_store_tab() && self.mode == Mode::Browse {
                    self.audio.play(SoundEffect::OpenModal);
                    self.mode = Mode::StoreFilters;
                    self.picker_index = 0;
                }
            }
            AppCommand::ClearStoreRegions => {
                self.config.clear_store_regions();
                self.selected = 0;
                self.current_scroll = 0.0;
                self.refilter_visible();
                self.audio.play(SoundEffect::Confirm);
            }
            AppCommand::ToggleStoreRegion(index) => {
                if let Some(region) = crate::config::STORE_REGIONS.get(index) {
                    self.config.toggle_store_region(region);
                    self.selected = 0;
                    self.current_scroll = 0.0;
                    self.refilter_visible();
                    self.audio.play(SoundEffect::Confirm);
                }
            }
            AppCommand::SetDownloadChoice(choice) => {
                if let Some(confirm) = &mut self.download_confirm {
                    confirm.selected_choice = choice;
                    self.audio.play(SoundEffect::Navigate);
                }
            }
            AppCommand::ConfirmDownload(yes) => {
                if let Some(confirm) = self.download_confirm.take() {
                    if yes {
                        self.execute_download(&confirm.title_id);
                    } else {
                        self.audio.play(SoundEffect::CloseModal);
                    }
                }
            }
            AppCommand::Confirm => {
                if let Some(confirm) = self.download_confirm.take() {
                    if confirm.selected_choice == 0 {
                        self.execute_download(&confirm.title_id);
                    } else {
                        self.audio.play(SoundEffect::CloseModal);
                    }
                    return;
                }
                self.audio.play(SoundEffect::Confirm);
                if self.mode == Mode::TabOrganizer {
                    self.mode = Mode::Browse;
                    self.refilter_visible();
                } else if self.mode == Mode::ScanFolders {
                    self.handle_command(AppCommand::ToggleScanFolder(self.picker_index));
                } else if self.mode == Mode::StoreFilters {
                    match self.picker_index {
                        0 => self.handle_command(AppCommand::SearchStore),
                        1 => self.handle_command(AppCommand::ClearStoreRegions),
                        index => self.handle_command(AppCommand::ToggleStoreRegion(index - 2)),
                    }
                } else if self.mode == Mode::GameOptions {
                    match self.picker_index {
                        0 => self.handle_command(AppCommand::RenameSelectedGame),
                        1 => self.handle_command(AppCommand::ToggleHideSelectedGame),
                        _ => self.mode = Mode::Browse,
                    }
                } else if self.is_settings() {
                    match self.settings_selected {
                        0 => self.trigger_rescan(),
                        1 => self.handle_command(AppCommand::ToggleDownloadBgm),
                        2 => self.handle_command(AppCommand::CycleLanguage),
                        3 => self.handle_command(AppCommand::OpenScanFolders),
                        4 => self.handle_command(AppCommand::CycleCacheBudget),
                        5 => self.handle_command(AppCommand::CycleLibrarySort),
                        6 => self.handle_command(AppCommand::CleanOrphanCache),
                        7 => self.handle_command(AppCommand::PurgeMusicCache),
                        8 => self.handle_command(AppCommand::PurgeAllCache),
                        _ => {}
                    }
                } else if self.mode == Mode::CollectionPicker {
                    self.toggle_picker_row(self.picker_index);
                } else if self.mode == Mode::StoreDetail {
                    self.confirm_store_detail();
                } else if self.is_store_tab() {
                    self.open_store_detail();
                } else {
                    self.launch_selected();
                }
            }
            AppCommand::Rescan => {
                self.audio.play(SoundEffect::Confirm);
                self.trigger_rescan();
            }
            AppCommand::ToggleDownloadBgm => {
                self.audio.play(SoundEffect::Confirm);
                self.config.download_bgm = !self.config.download_bgm;
                self.config.save();
                if self.config.download_bgm {
                    self.music_requested.clear();
                    self.music_retry_after.clear();
                } else {
                    self.audio.set_music(None);
                    self.music_selected = None;
                }
                let notice = self.i18n.format_one(
                    "notice-bgm", "status", self.text(if self.config.download_bgm { "status-enabled" } else { "status-disabled" }).to_owned(),
                );
                self.show_cache_notice(notice);
            }
            AppCommand::CycleLanguage => {
                self.audio.play(SoundEffect::Confirm);
                self.config.locale = self.config.locale.next();
                self.config.save();
                self.i18n = Localizer::new(self.config.locale);
                self.rebuild_tabs();
                let notice = self.i18n.format_one(
                    "notice-language", "language", self.locale_name(self.config.locale).to_owned(),
                );
                self.show_cache_notice(notice);
            }
            AppCommand::CycleCacheBudget => {
                self.audio.play(SoundEffect::Confirm);
                self.config.next_budget_option();
                let freed = crate::cache_manager::enforce_cache_budget(&self.games, self.config.cache_budget_mb);
                self.cache_stats = crate::cache_manager::compute_cache_stats(&self.games);
                let mut args = fluent_bundle::FluentArgs::new();
                args.set("limit", self.budget_label().to_owned());
                args.set("freed", crate::cache_manager::format_bytes(freed));
                let notice = self.i18n.format("notice-cache", Some(&args));
                self.show_cache_notice(notice);
            }
            AppCommand::CleanOrphanCache => {
                self.audio.play(SoundEffect::Confirm);
                let (count, freed) = crate::cache_manager::clean_orphaned_cache(&self.games);
                self.cache_stats = crate::cache_manager::compute_cache_stats(&self.games);
                let mut args = fluent_bundle::FluentArgs::new();
                args.set("count", count as i64);
                args.set("freed", crate::cache_manager::format_bytes(freed));
                let notice = self.i18n.format("notice-orphans", Some(&args));
                self.show_cache_notice(notice);
            }
            AppCommand::PurgeMusicCache => {
                self.audio.play(SoundEffect::Confirm);
                let (count, freed) = crate::cache_manager::purge_music();
                self.cache_stats = crate::cache_manager::compute_cache_stats(&self.games);
                for g in &mut self.games {
                    g.music_resolved = false;
                }
                self.music_requested.clear();
                self.music_retry_after.clear();
                self.audio.set_music(None);
                let mut args = fluent_bundle::FluentArgs::new();
                args.set("count", count as i64);
                args.set("freed", crate::cache_manager::format_bytes(freed));
                let notice = self.i18n.format("notice-music-purged", Some(&args));
                self.show_cache_notice(notice);
            }
            AppCommand::PurgeAllCache => {
                self.audio.play(SoundEffect::Confirm);
                let (count, freed) = crate::cache_manager::purge_all_cache();
                self.cache_stats = crate::cache_manager::compute_cache_stats(&self.games);
                self.art_state.clear();
                for g in &mut self.games {
                    g.cover_bytes = None;
                    g.hero_bytes = None;
                    g.logo_bytes = None;
                    g.music_resolved = false;
                }
                self.music_requested.clear();
                self.music_retry_after.clear();
                self.audio.set_music(None);
                let mut args = fluent_bundle::FluentArgs::new();
                args.set("count", count as i64);
                args.set("freed", crate::cache_manager::format_bytes(freed));
                let notice = self.i18n.format("notice-cache-purged", Some(&args));
                self.show_cache_notice(notice);
            }
            AppCommand::TogglePickerRow(index) => {
                self.audio.play(SoundEffect::Confirm);
                self.toggle_picker_row(index);
            }
            AppCommand::OpenScanFolders => {
                self.audio.play(SoundEffect::OpenModal);
                self.scan_folders = crate::scanner::discover_scan_folders();
                self.mode = Mode::ScanFolders;
                self.picker_index = 0;
            }
            AppCommand::ToggleScanFolder(index) => {
                if let Some(path) = self.scan_folders.get(index) {
                    self.audio.play(SoundEffect::Confirm);
                    self.config.toggle_scan_dir(path);
                    let notice = self.text("notice-scan-folders").to_string();
                    self.show_cache_notice(notice);
                }
            }
            AppCommand::AddCustomScanFolder | AppCommand::RemoveCustomScanFolder(_) => {}
            AppCommand::Back => {
                if self.download_confirm.is_some() {
                    self.audio.play(SoundEffect::CloseModal);
                    self.download_confirm = None;
                    return;
                }
                if self.mode == Mode::StoreDetail {
                    if self.store_detail.as_ref().and_then(|d| d.lightbox).is_some() {
                        self.audio.play(SoundEffect::CloseModal);
                        if let Some(detail) = &mut self.store_detail {
                            detail.lightbox = None;
                        }
                    } else {
                        self.audio.play(SoundEffect::CloseModal);
                        self.close_store_detail();
                    }
                } else if self.mode == Mode::StoreFilters {
                    self.audio.play(SoundEffect::CloseModal);
                    self.mode = Mode::Browse;
                } else if self.mode == Mode::GameOptions {
                    self.audio.play(SoundEffect::CloseModal);
                    self.mode = Mode::Browse;
                } else if self.search_active {
                    self.audio.play(SoundEffect::CloseModal);
                    self.search_active = false;
                    self.search_query.clear();
                    self.refilter_visible();
                } else if self.mode == Mode::CollectionPicker {
                    self.audio.play(SoundEffect::CloseModal);
                    self.mode = Mode::Browse;
                    self.rebuild_tabs();
                } else if self.mode == Mode::ScanFolders {
                    self.audio.play(SoundEffect::CloseModal);
                    self.mode = Mode::Browse;
                } else if self.mode == Mode::TabOrganizer {
                    self.audio.play(SoundEffect::CloseModal);
                    self.mode = Mode::Browse;
                    self.refilter_visible();
                } else if self.is_settings() {
                    self.audio.play(SoundEffect::CloseModal);
                    self.set_tab(self.tab_before_settings);
                }
            }
            AppCommand::SelectVisibleSlot(slot) => {
                if slot < self.visible.len() {
                    if slot != self.selected {
                        self.audio.play(SoundEffect::Navigate);
                    }
                    self.selected = slot;
                }
            }
            AppCommand::Quit => {
                if self.download_confirm.is_some() {
                    self.download_confirm = None;
                } else if self.mode == Mode::StoreDetail {
                    self.close_store_detail();
                } else if self.mode == Mode::Browse {
                    // `process::exit` skips normal cleanup, so remove the
                    // crash marker explicitly before leaving from this input
                    // route. Otherwise every normal Quit looks like a crash
                    // on the next launch.
                    crate::session::finish_cleanly();
                    std::process::exit(0);
                }
            }
        }
    }

    fn handle_input(&mut self, input: InputCommand) {
        use crate::audio::SoundEffect;
        if let Some(confirm) = &mut self.download_confirm {
            match input {
                InputCommand::MoveLeft => {
                    if confirm.selected_choice > 0 {
                        confirm.selected_choice = 0;
                        self.audio.play(SoundEffect::Navigate);
                    }
                }
                InputCommand::MoveRight => {
                    if confirm.selected_choice < 1 {
                        confirm.selected_choice = 1;
                        self.audio.play(SoundEffect::Navigate);
                    }
                }
                _ => {}
            }
            return;
        }
        match self.mode {
            Mode::Browse => match input {
                InputCommand::MoveLeft => {
                    if self.selected > 0 {
                        self.selected -= 1;
                        self.audio.play(SoundEffect::Navigate);
                    }
                }
                InputCommand::MoveRight => {
                    if self.selected + 1 < self.visible.len() {
                        self.selected += 1;
                        self.audio.play(SoundEffect::Navigate);
                    }
                }
                InputCommand::MoveUp => {
                    if self.is_settings() {
                        if self.settings_selected > 0 {
                            self.settings_selected -= 1;
                            self.audio.play(SoundEffect::Navigate);
                        }
                    } else if self.is_store_tab() {
                        if self.selected > 0 {
                            self.selected = self.selected.saturating_sub(GRID_COLS);
                            self.audio.play(SoundEffect::Navigate);
                        }
                    } else if self.config.library_view == crate::config::LibraryView::List && self.selected > 0 {
                        self.selected -= 1;
                        self.audio.play(SoundEffect::Navigate);
                    }
                }
                InputCommand::MoveDown => {
                    if self.is_settings() {
                        if self.settings_selected < 8 {
                            self.settings_selected += 1;
                            self.audio.play(SoundEffect::Navigate);
                        }
                    } else if self.is_store_tab() {
                        if self.selected + 1 < self.visible.len() {
                            self.selected = (self.selected + GRID_COLS).min(self.visible.len().saturating_sub(1));
                            self.audio.play(SoundEffect::Navigate);
                        }
                    } else if self.config.library_view == crate::config::LibraryView::List
                        && self.selected + 1 < self.visible.len()
                    {
                        self.selected += 1;
                        self.audio.play(SoundEffect::Navigate);
                    }
                }
            },
            Mode::CollectionPicker => {
                let rows = self.collections.items.len();
                match input {
                    InputCommand::MoveUp => {
                        if self.picker_index > 0 {
                            self.picker_index -= 1;
                            self.audio.play(SoundEffect::Navigate);
                        }
                    }
                    InputCommand::MoveDown => {
                        if self.picker_index + 1 < rows {
                            self.picker_index += 1;
                            self.audio.play(SoundEffect::Navigate);
                        }
                    }
                    InputCommand::MoveLeft | InputCommand::MoveRight => {}
                }
            }
            Mode::ScanFolders => {
                let rows = self.scan_folders.len();
                match input {
                    InputCommand::MoveUp => {
                        if self.picker_index > 0 {
                            self.picker_index -= 1;
                            self.audio.play(SoundEffect::Navigate);
                        }
                    }
                    InputCommand::MoveDown => {
                        if self.picker_index + 1 < rows {
                            self.picker_index += 1;
                            self.audio.play(SoundEffect::Navigate);
                        }
                    }
                    InputCommand::MoveLeft | InputCommand::MoveRight => {}
                }
            }
            Mode::TabOrganizer => match input {
                InputCommand::MoveLeft => self.move_active_tab(-1),
                InputCommand::MoveRight => self.move_active_tab(1),
                InputCommand::MoveUp | InputCommand::MoveDown => {}
            },
            Mode::StoreFilters => {
                let rows = crate::config::STORE_REGIONS.len() + 2;
                match input {
                    InputCommand::MoveUp => {
                        if self.picker_index > 0 {
                            self.picker_index -= 1;
                            self.audio.play(SoundEffect::Navigate);
                        }
                    }
                    InputCommand::MoveDown => {
                        if self.picker_index + 1 < rows {
                            self.picker_index += 1;
                            self.audio.play(SoundEffect::Navigate);
                        }
                    }
                    InputCommand::MoveLeft | InputCommand::MoveRight => {}
                }
            }
            Mode::GameOptions => match input {
                InputCommand::MoveUp if self.picker_index > 0 => {
                    self.picker_index -= 1;
                    self.audio.play(SoundEffect::Navigate);
                }
                InputCommand::MoveDown if self.picker_index < 2 => {
                    self.picker_index += 1;
                    self.audio.play(SoundEffect::Navigate);
                }
                _ => {}
            },
            Mode::StoreDetail => {
                let n = self.store_detail.as_ref().map(|d| d.screenshot_urls.len()).unwrap_or(0);
                if n == 0 {
                    return;
                }
                match input {
                    InputCommand::MoveLeft => {
                        if let Some(detail) = &mut self.store_detail {
                            if let Some(idx) = detail.lightbox {
                                if idx > 0 {
                                    detail.lightbox = Some(idx - 1);
                                    detail.selected_shot = idx - 1;
                                    self.audio.play(SoundEffect::Navigate);
                                }
                            } else if detail.selected_shot > 0 {
                                detail.selected_shot -= 1;
                                self.audio.play(SoundEffect::Navigate);
                            }
                        }
                    }
                    InputCommand::MoveRight => {
                        if let Some(detail) = &mut self.store_detail {
                            if let Some(idx) = detail.lightbox {
                                if idx + 1 < n {
                                    detail.lightbox = Some(idx + 1);
                                    detail.selected_shot = idx + 1;
                                    self.audio.play(SoundEffect::Navigate);
                                }
                            } else if detail.selected_shot + 1 < n {
                                detail.selected_shot += 1;
                                self.audio.play(SoundEffect::Navigate);
                            }
                        }
                    }
                    InputCommand::MoveUp | InputCommand::MoveDown => {}
                }
            }
        }
    }

    pub fn tab_count(&self, tab: usize) -> usize {
        self.tab_counts.get(tab).copied().unwrap_or(0)
    }

    pub fn is_store_tab(&self) -> bool {
        matches!(self.tab_targets.get(self.active_tab), Some(TabTarget::Store))
    }

    pub fn settings_tab_index(&self) -> usize {
        self.tabs.len()
    }

    pub fn is_settings(&self) -> bool {
        self.active_tab == self.settings_tab_index()
    }

    fn change_tab(&mut self, delta: i32) {
        if self.mode != Mode::Browse {
            return;
        }
        let total = self.settings_tab_index() as i32;
        if total <= 0 {
            return;
        }
        let current = self.active_tab.min(self.settings_tab_index() - 1) as i32;
        let next = (current + delta).rem_euclid(total);
        self.set_tab(next as usize);
    }

    fn set_tab(&mut self, index: usize) {
        if index > self.settings_tab_index() || self.mode != Mode::Browse {
            return;
        }
        let leaving_store = self.is_store_tab()
            && !matches!(self.tab_targets.get(index), Some(TabTarget::Store));
        self.active_tab = index;
        self.selected = 0;
        self.current_scroll = 0.0;
        if leaving_store {
            let drop: Vec<usize> = self
                .store_games
                .iter()
                .enumerate()
                .filter(|(_, game)| game.cover_bytes.is_some() || game.hero_bytes.is_some())
                .map(|(i, _)| i)
                .collect();
            for i in drop {
                self.drop_store_cover(i);
            }
        }
        self.refilter_visible();
    }

    fn open_store_detail(&mut self) {
        let Some(game) = self
            .visible
            .get(self.selected)
            .and_then(|&i| self.store_games.get(i))
        else {
            return;
        };
        let title_id = game.title_id.clone();
        let item = self.store.item_by_title_id(&title_id);
        let urls = item.map(|i| i.resolved_screenshot_urls()).unwrap_or_default();
        let hero_url = item
            .and_then(|i| i.resolved_background_url())
            .or_else(|| urls.first().cloned());
        let looking_up = urls.is_empty();
        let n = urls.len();
        self.store_detail = Some(StoreDetailState {
            title_id: title_id.clone(),
            screenshot_urls: urls,
            screenshots: vec![None; n],
            pending: HashSet::new(),
            lightbox: None,
            selected_shot: 0,
            hero_bytes: None,
            looking_up_heroes: looking_up,
        });
        self.mode = Mode::StoreDetail;
        if let Some(url) = hero_url {
            if let Some(tx) = &self.shot_tx {
                let _ = tx.try_send((title_id.clone(), -1, url));
            }
        }
        if looking_up {
            let tx = self.hero_lookup_tx.clone();
            std::thread::spawn(move || {
                let urls = artwork::lookup_hero_urls(&title_id);
                let _ = tx.send((title_id, urls));
            });
        }
    }

    fn close_store_detail(&mut self) {
        if let Some(detail) = self.store_detail.take() {
            let mut cache = self.texture_cache.borrow_mut();
            cache.invalidate(&format!("PS Vita:{}:hero", detail.title_id));
            for i in 0..detail.screenshot_urls.len() {
                cache.invalidate(&format!("PS Vita:{}:shot{i}", detail.title_id));
            }
        }
        self.mode = Mode::Browse;
    }

    fn toggle_store_lightbox(&mut self) {
        let Some(detail) = &mut self.store_detail else { return };
        if detail.screenshot_urls.is_empty() {
            return;
        }
        if detail.lightbox.is_some() {
            detail.lightbox = None;
        } else {
            let idx = detail.selected_shot.min(detail.screenshot_urls.len().saturating_sub(1));
            detail.lightbox = Some(idx);
        }
    }

    fn confirm_store_detail(&mut self) {
        let Some(title_id) = self.store_detail.as_ref().map(|d| d.title_id.clone()) else {
            return;
        };
        if self.is_title_installed(&title_id) {
            self.launch_selected();
            return;
        }
        let title = self
            .store
            .item_by_title_id(&title_id)
            .map(|i| i.name.clone())
            .unwrap_or_else(|| title_id.clone());

        self.audio.play(crate::audio::SoundEffect::OpenModal);
        self.download_confirm = Some(DownloadConfirmState {
            title_id,
            title,
            selected_choice: 0,
        });
    }

    pub fn execute_download(&mut self, title_id: &str) {
        let queue_key = title_id.trim().to_ascii_uppercase();
        let title = self
            .store
            .item_by_title_id(title_id)
            .map(|i| i.name.clone())
            .unwrap_or_else(|| title_id.to_string());
        if self.queued_downloads.contains(&queue_key) {
            self.audio.play(crate::audio::SoundEffect::Navigate);
            self.launch_notice = Some((title_id.to_string(), format!("{}: {}", title, self.text("notice-download-already-queued"))));
            return;
        }
        match self.store.download_and_install(title_id) {
            Ok(()) => {
                self.queued_downloads.insert(queue_key);
                self.audio.play(crate::audio::SoundEffect::LaunchGame);
                let mut args = fluent_bundle::FluentArgs::new();
                args.set("title", title);
                self.launch_notice = Some((
                    title_id.to_string(),
                    self.i18n.format("notice-download-enqueued", Some(&args)),
                ));
            }
            Err(e) => {
                self.audio.play(crate::audio::SoundEffect::CloseModal);
                let mut args = fluent_bundle::FluentArgs::new();
                args.set("error", e.to_string());
                self.launch_notice = Some((title_id.to_string(), self.i18n.format("error-download", Some(&args))));
            }
        }
    }

    fn pump_store_media(&mut self) {
        if let Some(rx) = &self.hero_lookup_rx {
            if let Ok((title_id, urls)) = rx.try_recv() {
                if let Some(detail) = &mut self.store_detail {
                    if detail.title_id == title_id && detail.looking_up_heroes {
                        detail.looking_up_heroes = false;
                        if !urls.is_empty() {
                            let n = urls.len();
                            detail.screenshot_urls = urls;
                            detail.screenshots = vec![None; n];
                            detail.pending.clear();
                            if detail.hero_bytes.is_none() {
                                if let Some(url) = detail.screenshot_urls.first().cloned() {
                                    if let Some(tx) = &self.shot_tx {
                                        let _ = tx.try_send((title_id, -1, url));
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        if let Some(rx) = &self.shot_rx {
            while let Ok((title_id, index, bytes)) = rx.try_recv() {
                let Some(detail) = &mut self.store_detail else { continue };
                if detail.title_id != title_id {
                    continue;
                }
                if index < 0 {
                    if let Some(bytes) = bytes {
                        detail.hero_bytes = Some(bytes);
                    }
                } else {
                    let i = index as usize;
                    detail.pending.remove(&i);
                    if i < detail.screenshots.len() {
                        detail.screenshots[i] = bytes;
                    }
                }
            }
        }

        if self.mode != Mode::StoreDetail {
            return;
        }
        let (title_id, n, focus, in_flight) = {
            let Some(detail) = &self.store_detail else { return };
            if detail.screenshot_urls.is_empty() {
                return;
            }
            let n = detail.screenshot_urls.len();
            (
                detail.title_id.clone(),
                n,
                detail.lightbox.unwrap_or(detail.selected_shot).min(n.saturating_sub(1)),
                detail.pending.len(),
            )
        };
        if in_flight >= STORE_SHOT_BUDGET {
            return;
        }
        let mut order = Vec::with_capacity(n);
        order.push(focus);
        if focus > 0 {
            order.push(focus - 1);
        }
        if focus + 1 < n {
            order.push(focus + 1);
        }
        for offset in 2..=STORE_SHOT_KEEP_RADIUS {
            if focus >= offset && !order.contains(&(focus - offset)) {
                order.push(focus - offset);
            }
            if focus + offset < n && !order.contains(&(focus + offset)) {
                order.push(focus + offset);
            }
        }

        if let Some(detail) = &mut self.store_detail {
            for (i, slot) in detail.screenshots.iter_mut().enumerate() {
                if slot.is_some() && !order.contains(&i) {
                    *slot = None;
                }
            }
        }

        let mut budget = STORE_SHOT_BUDGET - in_flight;
        for i in order {
            if budget == 0 {
                break;
            }
            let Some(detail) = &mut self.store_detail else { break };
            if detail.screenshots.get(i).and_then(|s| s.as_ref()).is_some() {
                continue;
            }
            if detail.pending.contains(&i) {
                continue;
            }
            let Some(url) = detail.screenshot_urls.get(i).cloned() else { continue };
            if let Some(tx) = &self.shot_tx {
                if tx.try_send((title_id.clone(), i as i32, url)).is_ok() {
                    detail.pending.insert(i);
                    budget -= 1;
                }
            }
        }
    }
}

fn booter_exists(title_id: &str) -> bool {
    for part in &["ux0", "ur0", "uma0", "imc0"] {
        let app_path = format!("{}:app/{}", part, title_id);
        let sfo_path = format!("{}:app/{}/sce_sys/param.sfo", part, title_id);
        let eboot_path = format!("{}:app/{}/eboot.bin", part, title_id);
        if std::path::Path::new(&app_path).exists()
            || std::path::Path::new(&sfo_path).exists()
            || std::path::Path::new(&eboot_path).exists()
        {
            return true;
        }
    }
    false
}

fn find_adrenaline_booters() -> Vec<&'static str> {
    let mut found = Vec::new();
    for title_id in &["PSPEMUCFW", "PSPEMU001", "ADRLAUNCH", "RETROLNCR"] {
        if booter_exists(title_id) {
            found.push(*title_id);
        }
    }
    found
}

fn to_ms0_path(game_file_path: &str) -> Option<String> {
    let normalized = game_file_path.replace('\\', "/");
    if let Some((_, rest)) = normalized.split_once(":pspemu/") {
        return Some(format!("ms0:/{}", rest));
    }
    if let Some(idx) = normalized.find("pspemu/") {
        return Some(format!("ms0:/{}", &normalized[idx + 7..]));
    }
    None
}

fn current_unix_time() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}

fn prepare_adrenaline_boot(game_file_path: &str, booter_id: &str) -> Result<String, String> {
    if !std::path::Path::new(game_file_path).exists() {
        return Err(format!("game path not found: {}", game_file_path));
    }
    let Some(ms0_path) = to_ms0_path(game_file_path) else {
        return Err(format!("invalid PSP path for Adrenaline: {}", game_file_path));
    };

    let bubblesdb_dir = "ux0:adrbblbooter/bubblesdb";
    std::fs::create_dir_all(bubblesdb_dir)
        .map_err(|e| format!("failed create bubblesdb dir: {}", e))?;
    let bubblesdb_file = format!("{}/{}.txt", bubblesdb_dir, booter_id);
    std::fs::write(&bubblesdb_file, &ms0_path)
        .map_err(|e| format!("failed write bubblesdb: {}", e))?;

    let app_data_dir = format!("ux0:app/{}/data", booter_id);
    std::fs::create_dir_all(&app_data_dir)
        .map_err(|e| format!("failed create app data dir: {}", e))?;
    let boot_inf_file = format!("{}/boot.inf", app_data_dir);
    let boot_inf_content = format!("PATH={}\nDRIVER=INFERNO\nEXECUTE=EBOOT.BIN\n", ms0_path);
    std::fs::write(&boot_inf_file, boot_inf_content)
        .map_err(|e| format!("failed write boot.inf: {}", e))?;

    let boot_bin_file = format!("{}/boot.bin", app_data_dir);
    std::fs::write(&boot_bin_file, ms0_path.as_bytes())
        .map_err(|e| format!("failed write boot.bin: {}", e))?;

    if !std::path::Path::new(&boot_inf_file).exists() || !std::path::Path::new(&boot_bin_file).exists() {
        return Err("boot payload missing after write".to_string());
    }

    Ok(ms0_path)
}

impl App {
    fn launch_selected(&mut self) {
        let current_game = if self.is_store_tab() {
            self.visible.get(self.selected).and_then(|&i| self.store_games.get(i))
        } else {
            self.visible.get(self.selected).and_then(|&i| self.games.get(i))
        };
        let Some(game) = current_game else { return };
        let launched_title_id = game.title_id.clone();

        let mut launch_candidates = vec![game.title_id.clone()];

        if game.system != System::Vita {
            if !game.has_bubble {
                let Some(file_path) = game.file_path.clone() else {
                    self.launch_notice = Some((
                        game.title_id.clone(),
                        self.text("error-launch").to_string(),
                    ));
                    return;
                };
                let mut errors = Vec::new();
                launch_candidates.clear();
                for booter_id in find_adrenaline_booters() {
                    match prepare_adrenaline_boot(&file_path, booter_id) {
                        Ok(ms0_path) => {
                            crate::logger::log(&format!(
                                "launch_selected: using booter {} for {} -> {}",
                                booter_id, game.title_id, ms0_path
                            ));
                            launch_candidates.push(booter_id.to_string());
                        }
                        Err(e) => {
                            errors.push(format!("{}: {}", booter_id, e));
                        }
                    }
                }
                if launch_candidates.is_empty() {
                    crate::logger::log(&format!(
                        "launch_selected: no valid adrenaline booter for {} ({})",
                        game.title_id,
                        errors.join(" | ")
                    ));
                    self.launch_notice = Some((
                        game.title_id.clone(),
                        self.text("error-launch").to_string(),
                    ));
                    return;
                }
            }
        }

        self.launch_notice = None;
        self.recent.touch(&launched_title_id);
        self.stats.record_launch(&launched_title_id, current_unix_time());
        self.rebuild_library_indexes();
        crate::logger::log(&format!(
            "launch_selected: {} (candidates: {})",
            launched_title_id,
            launch_candidates.join(",")
        ));
        self.audio.play(crate::audio::SoundEffect::LaunchGame);
        self.audio.set_music(None);
        self.music_selected = None;
        self.music_settle_frames = 0;

        #[cfg(target_os = "vita")]
        {
            for target_title_id in &launch_candidates {
                let uri = format!("psgm:play?titleid={}", target_title_id);
                if let Ok(c_uri) = std::ffi::CString::new(uri) {
                    std::thread::sleep(std::time::Duration::from_millis(150));
                    unsafe {
                        let rc = vitasdk_sys::sceAppMgrLaunchAppByUri(0x20000, c_uri.as_ptr());
                        if rc >= 0 {
                            crate::session::finish_cleanly();
                            vitasdk_sys::sceKernelExitProcess(0);
                        }
                        crate::logger::log(&format!(
                            "launch_selected: sceAppMgrLaunchAppByUri failed target={} rc=0x{:08X}",
                            target_title_id, rc as u32
                        ));
                    }
                }
            }
            self.launch_notice = Some((
                launched_title_id,
                self.text("error-launch").to_string(),
            ));
        }

        if matches!(self.tab_targets.get(self.active_tab), Some(TabTarget::Recent)) {
            self.refilter_visible();
        }
    }

    fn remove_selected_from_collection(&mut self) {
        let Some(TabTarget::Collection(name)) = self.tab_targets.get(self.active_tab) else { return };
        let Some(collection_idx) = self.collection_index_by_name(name) else { return };
        let Some(game) = self.visible.get(self.selected).and_then(|&i| self.games.get(i)) else { return };
        let title_id = game.title_id.clone();
        if self.collections.remove(collection_idx, &title_id) {
            self.rebuild_library_indexes();
            self.refilter_visible();
        }
    }

    /// Square is the fast SteamOS-style favourite action outside a collection.
    /// It works for both installed titles and Store items by title ID.
    fn toggle_selected_favorite(&mut self) {
        let selected_id = if self.is_store_tab() {
            self.visible
                .get(self.selected)
                .and_then(|&index| self.store_games.get(index))
        } else {
            self.visible
                .get(self.selected)
                .and_then(|&index| self.games.get(index))
        }
        .map(|game| game.title_id.clone());
        let Some(title_id) = selected_id else { return };
        let Some(index) = self.collections.items.iter().position(|collection| {
            collection.name.eq_ignore_ascii_case("Favorites")
                || collection.name.eq_ignore_ascii_case("Favoritos")
        }) else {
            return;
        };
        self.collections.toggle(index, &title_id);
        self.rebuild_library_indexes();
        self.refilter_visible();
        self.audio.play(crate::audio::SoundEffect::Confirm);
    }

    fn toggle_picker_row(&mut self, index: usize) {
        if self.mode != Mode::CollectionPicker {
            return;
        }
        let Some(game) = self.visible.get(self.selected).and_then(|&i| self.games.get(i)) else { return };
        let title_id = game.title_id.clone();
        self.collections.toggle(index, &title_id);
        self.picker_index = index;
        self.rebuild_library_indexes();
        self.refilter_visible();
    }

    pub fn trigger_rescan(&mut self) {
        crate::logger::log("App::trigger_rescan requested");
        let _ = std::fs::remove_file(crate::scanner::GAMES_CACHE_FILE);
        let net_ready = self.net_ready;
        let disabled_scan_dirs = self.config.disabled_scan_dirs.clone();
        let custom_scan_dirs = self.config.custom_scan_dirs.clone();
        let (scan_tx, scan_rx) = mpsc::channel();
        let (lookup_tx, lookup_rx) = mpsc::channel();
        std::thread::spawn(move || {
            crate::logger::log("Background rescan thread started");
            let (scanned_games, scan_log) = scan_installed_games(&disabled_scan_dirs, &custom_scan_dirs);
            crate::scanner::save_cached_games(&scanned_games);
            let targets: Option<Vec<artwork::LookupTarget>> = net_ready
                .then(|| scanned_games.iter().map(artwork::LookupTarget::from_game).collect());
            let _ = scan_tx.send((scanned_games, scan_log));

            if let Some(targets) = targets {
                let res = artwork::lookup_batch(&targets);
                let _ = lookup_tx.send(res);
            }
        });
        self.scan_rx = Some(scan_rx);
        self.lookup_rx = Some(lookup_rx);
    }
}

fn tab_token(target: &TabTarget) -> String {
    match target {
        TabTarget::All => "all".to_owned(),
        TabTarget::Vita => "vita".to_owned(),
        TabTarget::Psp => "psp".to_owned(),
        TabTarget::Ps1 => "ps1".to_owned(),
        TabTarget::Recent => "recent".to_owned(),
        TabTarget::Store => "store".to_owned(),
        TabTarget::Collection(name) => format!("collection:{name}"),
    }
}

fn tab_label(target: &TabTarget, i18n: &Localizer) -> String {
    match target {
        TabTarget::All => i18n.text("tab-all").to_owned(),
        TabTarget::Vita => "PS VITA".to_owned(),
        TabTarget::Psp => "PSP".to_owned(),
        TabTarget::Ps1 => "PS1".to_owned(),
        TabTarget::Recent => i18n.text("tab-recent").to_owned(),
        TabTarget::Store => i18n.text("tab-store").to_owned(),
        TabTarget::Collection(name) if name.eq_ignore_ascii_case("Favorites") || name.eq_ignore_ascii_case("Favoritos") => {
            i18n.text("tab-favorites").to_owned()
        }
        TabTarget::Collection(name) => name.to_uppercase(),
    }
}

fn build_tabs(collections: &Collections, i18n: &Localizer, saved_order: &[String]) -> (Vec<String>, Vec<TabTarget>) {
    let mut available = vec![
        TabTarget::All,
        TabTarget::Vita,
        TabTarget::Psp,
        TabTarget::Ps1,
        TabTarget::Recent,
        TabTarget::Store,
    ];
    available.extend(collections.items.iter().map(|c| TabTarget::Collection(c.name.clone())));

    let mut ordered = Vec::with_capacity(available.len());
    for token in saved_order {
        if let Some(index) = available.iter().position(|target| tab_token(target).eq_ignore_ascii_case(token)) {
            ordered.push(available.remove(index));
        }
    }
    ordered.extend(available);
    let labels = ordered.iter().map(|target| tab_label(target, i18n)).collect();
    (labels, ordered)
}

fn filter_games(
    games: &[Game],
    store_games: &[Game],
    store: &crate::store::StoreManager,
    collections: &Collections,
    recent: &RecentlyPlayed,
    target: Option<&TabTarget>,
    search: &str,
    store_regions: &[String],
) -> Vec<usize> {
    let raw: Vec<usize> = match target {
        Some(TabTarget::Recent) => recent
            .order()
            .iter()
            .filter_map(|title_id| games.iter().position(|g| &g.title_id == title_id))
            .collect(),
        Some(TabTarget::Store) => (0..store_games.len()).collect(),
        Some(TabTarget::Collection(name)) => {
            let collection_idx = collections.items.iter().position(|c| c.name.eq_ignore_ascii_case(name));
            games
                .iter()
                .enumerate()
                .filter(|(_, g)| collection_idx.is_some_and(|idx| collections.contains(idx, &g.title_id)))
                .map(|(i, _)| i)
                .collect()
        }
        Some(target) => {
            let system = match target {
                TabTarget::Vita => Some(System::Vita),
                TabTarget::Psp => Some(System::Psp),
                TabTarget::Ps1 => Some(System::Psx),
                _ => None,
            };
            games
                .iter()
                .enumerate()
                .filter(|(_, g)| g.is_game != Some(false))
                .filter(|(_, g)| system.map_or(true, |s| g.system == s))
                .map(|(i, _)| i)
                .collect()
        }
        None => Vec::new(),
    };

    if !matches!(target, Some(TabTarget::Store)) {
        return raw;
    }

    filter_store_games(store_games, store, search, store_regions)
}

fn system_sort_key(system: System) -> u8 {
    match system {
        System::Vita => 0,
        System::Psp => 1,
        System::Psx => 2,
    }
}

fn filter_store_games(
    store_games: &[Game],
    store: &crate::store::StoreManager,
    search: &str,
    store_regions: &[String],
) -> Vec<usize> {
    let query = search.trim().to_lowercase();
    (0..store_games.len())
        .filter(|&i| {
            store_games.get(i).is_some_and(|game| {
                let matches_query = query.is_empty() || contains_ignore_case(&game.title, &query);
                // “All” must be a direct view of `store_games`. Requiring a
                // second lookup here made the whole Store empty when an old
                // cache had an incomplete title index.
                let matches_region = store_regions.is_empty()
                    || store
                        .item_by_title_id(&game.title_id)
                        // Missing metadata is not a reason to hide a game.
                        .map(|item| store_item_matches_regions(item, store_regions))
                        .unwrap_or(true);
                matches_query && matches_region
            })
        })
        .collect()
}

fn store_item_matches_regions(item: &StoreItem, selected_regions: &[String]) -> bool {
    if selected_regions.is_empty() {
        return true;
    }
    let Some(region) = item.region.as_deref() else {
        // Older catalog entries are not region-tagged. They must remain
        // discoverable when a Store region is selected.
        return true;
    };
    let actual = canonical_store_region(region);
    selected_regions
        .iter()
        .filter_map(|selected| canonical_store_region(selected))
        .any(|selected| actual == Some(selected))
}

fn canonical_store_region(region: &str) -> Option<&'static str> {
    let normalized = region.trim().to_ascii_uppercase();
    if normalized.is_empty() {
        return None;
    }
    let words: Vec<&str> = normalized
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|word| !word.is_empty())
        .collect();
    let has = |word: &str| words.iter().any(|candidate| *candidate == word);
    if has("US") || has("USA") || has("UNITED") || has("AMERICA") || has("NA") {
        Some("US")
    } else if has("EU") || has("EUR") || has("EUROPE") || has("EUROPEAN") {
        Some("EU")
    } else if has("JP") || has("JPN") || has("JAPAN") || has("JAPANESE") {
        Some("JP")
    } else if has("ASIA") || has("ASIAN") || has("KR") || has("KOR") || has("KOREA") || has("CN") || has("CHINA") {
        Some("ASIA")
    } else if has("INT") || has("INTERNATIONAL") || has("GLOBAL") || has("WORLD") || has("WORLDWIDE") {
        Some("INT")
    } else {
        None
    }
}

fn contains_ignore_case(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }
    if haystack.len() < needle.len() {
        return false;
    }
    let hay = haystack.as_bytes();
    let ned = needle.as_bytes();
    if haystack.is_ascii() && needle.is_ascii() {
        return hay
            .windows(ned.len())
            .any(|w| w.eq_ignore_ascii_case(ned));
    }
    haystack.to_lowercase().contains(needle)
}

fn apply_library_preferences(games: &mut [Game], config: &crate::config::Config) {
    for game in games {
        if let Some(title) = config.title_override(&game.title_id) {
            game.title = title.to_string();
        }
    }
}

fn store_known_art(item: &StoreItem) -> KnownArt {
    KnownArt {
        cover_url: item
            .cover_url
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(crate::store::absolute_url),
        screenshot_url: item.resolved_screenshot_urls().into_iter().next(),
        logo_url: None,
        music_url: None,
    }
}

#[cfg(test)]
mod tab_tests {
    use super::*;

    #[test]
    fn saved_tab_order_is_restored_and_stale_entries_are_ignored() {
        let collections = Collections {
            items: vec![
                crate::collections::Collection { name: "Favorites".to_owned(), title_ids: HashSet::new() },
                crate::collections::Collection { name: "HOMEBREW".to_owned(), title_ids: HashSet::new() },
            ],
        };
        let i18n = Localizer::new(Locale::EnUs);
        let saved = vec![
            "store".to_owned(),
            "collection:HOMEBREW".to_owned(),
            "collection:DELETED".to_owned(),
            "all".to_owned(),
        ];

        let (_, targets) = build_tabs(&collections, &i18n, &saved);
        assert_eq!(targets[0], TabTarget::Store);
        assert_eq!(targets[1], TabTarget::Collection("HOMEBREW".to_owned()));
        assert_eq!(targets[2], TabTarget::All);
        assert_eq!(targets.len(), 8);
    }

    #[test]
    fn store_region_filter_supports_multiple_regions_and_all() {
        let item: StoreItem = serde_json::from_str(
            r#"{"title_id":"PCSE00001","name":"Test","region":"US","download_url":"https://example.com/game"}"#,
        )
        .unwrap();
        assert!(store_item_matches_regions(&item, &[]));
        assert!(store_item_matches_regions(&item, &["EU".to_owned(), "US".to_owned()]));
        assert!(!store_item_matches_regions(&item, &["JP".to_owned(), "ASIA".to_owned()]));
    }

    #[test]
    fn store_region_filter_accepts_catalog_region_names() {
        let selected = ["US".to_owned()];
        for region in ["US", "USA", "United States", "North America"] {
            let item: StoreItem = serde_json::from_str(&format!(
                r#"{{"title_id":"PCSE00001","name":"Test","region":"{region}","download_url":"https://example.com/game"}}"#
            )).unwrap();
            assert!(store_item_matches_regions(&item, &selected), "{region}");
        }
    }
}

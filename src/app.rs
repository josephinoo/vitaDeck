use crate::artwork::{self, ArtJob, ArtResult};
use crate::collections::Collections;
use crate::input::{AppCommand, InputCommand};
use crate::net;
use crate::recent::RecentlyPlayed;
use crate::scanner::{
    load_cached_cover, load_cached_hero, load_cached_logo, resolved_music_path, scan_installed_games, Game,
    ImageBytes, System,
};
use crate::store::StoreItem;
use crate::textures::TextureCache;
use crate::ui::{card_row_stride, Mode, FOOTER_H, GRID_COLS, HEADER_H, SCREEN_H, SCREEN_W};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::sync::mpsc;

const SCROLL_LERP: f32 = 0.22;

const MAX_ART_JOBS_IN_FLIGHT: usize = 3;

const STORE_ART_JOBS_IN_FLIGHT: usize = 2;

const PRELOAD_MAX_ART_JOBS_IN_FLIGHT: usize = 3;

const COVER_KEEP_RADIUS: usize = 18;

const ART_LOOKAHEAD: usize = 10;

const STORE_SHOT_BUDGET: usize = 2;
const STORE_SHOT_KEEP_RADIUS: usize = 2;
const MEMORY_POLL_FRAMES: u64 = 30;

const SYSTEM_TABS: [(&str, Option<System>); 4] = [
    ("ALL", None),
    ("PS VITA", Some(System::Vita)),
    ("PSP", Some(System::Psp)),
    ("PS1", Some(System::Psx)),
];
const RECENT_TAB_INDEX: usize = SYSTEM_TABS.len();
pub const STORE_TAB_INDEX: usize = SYSTEM_TABS.len() + 1;

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
    pub store: crate::store::StoreManager,
    pub store_games: Vec<Game>,
    pub collections: Collections,
    pub recent: RecentlyPlayed,
    pub mode: Mode,
    pub active_tab: usize,
    pub selected: usize,
    pub picker_index: usize,
    pub current_scroll: f32,
    pub tabs: Vec<String>,
    pub visible: Vec<usize>,
    pub net_line: String,
    pub scan_log: Vec<String>,
    texture_cache: RefCell<TextureCache>,
    art_state: HashMap<String, ArtState>,

    known_art: HashMap<String, KnownArt>,

    art_jobs_in_flight: usize,
    request_tx: Option<mpsc::Sender<ArtJob>>,
    done_rx: Option<mpsc::Receiver<ArtResult>>,
    scan_rx: Option<mpsc::Receiver<(Vec<Game>, Vec<String>)>>,
    lookup_rx: Option<mpsc::Receiver<HashMap<String, artwork::LookupResult>>>,
    net_ready: bool,
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
    local_art_tx: mpsc::Sender<(String, Option<ImageBytes>, Option<ImageBytes>, Option<ImageBytes>)>,
    local_art_rx: mpsc::Receiver<(String, Option<ImageBytes>, Option<ImageBytes>, Option<ImageBytes>)>,

    preload_pending: std::collections::HashSet<String>,
    preload_total: usize,
    pub store_detail: Option<StoreDetailState>,
    shot_tx: Option<mpsc::Sender<(String, i32, String)>>,
    shot_rx: Option<mpsc::Receiver<(String, i32, Option<ImageBytes>)>>,
    hero_lookup_rx: Option<mpsc::Receiver<(String, Vec<String>)>>,
    hero_lookup_tx: mpsc::Sender<(String, Vec<String>)>,

    pub config: crate::config::Config,
    pub cache_stats: crate::cache_manager::CacheStats,
    pub settings_selected: usize,
    tab_before_settings: usize,
    music_requested: std::collections::HashSet<String>,
    music_retry_after: HashMap<String, u64>,
    frame_counter: u64,
    pub runtime: crate::runtime::VitaRuntime,
    pub cache_notice: Option<String>,
    pub download_confirm: Option<DownloadConfirmState>,
}

impl App {
    pub fn new() -> Self {
        crate::logger::log("App::new: start");
        let net_status = net::init();
        let net_ready = net_status.is_ok();
        let net_line = match &net_status {
            Ok(()) => "NET ok".to_string(),
            Err(e) => format!("NET error: {}", e),
        };
        crate::logger::log(&format!("App::new: net={}", net_line));

        let cached_games = crate::scanner::load_cached_games();
        let games = cached_games.unwrap_or_default();

        let (scan_tx, scan_rx) = mpsc::channel();
        let (lookup_tx, lookup_rx) = mpsc::channel();
        std::thread::spawn(move || {
            crate::logger::log("Background scan thread started");
            let (scanned_games, scan_log) = scan_installed_games();
            crate::logger::log(&format!("Scan finished. Found {} games.", scanned_games.len()));
            crate::scanner::save_cached_games(&scanned_games);
            let targets: Option<Vec<artwork::LookupTarget>> = net_ready
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
        let config = crate::config::Config::load();
        crate::net::set_client_id(config.client_id.clone());
        let cache_stats = crate::cache_manager::compute_cache_stats(&games);
        let store = crate::store::StoreManager::new();
        let store_games = store.to_games();
        for item in &store.items {
            if let Some(title_id) = &item.title_id {
                known_art.insert(title_id.clone(), store_known_art(item));
            }
        }
        let tabs = build_tabs(&collections);
        let visible = filter_games(&games, &store_games, &collections, &recent, 0, "");
        crate::logger::log("App::new: collections and config OK");

        let (request_tx, done_rx) = if net_ready {
            let (request_tx, request_rx) = mpsc::channel::<ArtJob>();
            let (done_tx, done_rx) = mpsc::channel::<ArtResult>();
            std::thread::spawn(move || {
                while let Ok(job) = request_rx.recv() {

                    artwork::resolve(job, &done_tx);
                }
            });
            (Some(request_tx), Some(done_rx))
        } else {
            (None, None)
        };

        let (local_art_tx, local_art_rx) = mpsc::channel();
        let (shot_tx, shot_req_rx) = mpsc::channel::<(String, i32, String)>();
        let (shot_done_tx, shot_rx) = mpsc::channel();
        std::thread::spawn(move || {
            while let Ok((title_id, index, url)) = shot_req_rx.recv() {
                let bytes = artwork::fetch_image(&url).map(|img| (img.is_png, img.bytes));
                let _ = shot_done_tx.send((title_id, index, bytes));
            }
        });
        let (hero_lookup_tx, hero_lookup_rx) = mpsc::channel();

        App {
            games,
            store,
            store_games,
            collections,
            recent,
            mode: Mode::Browse,
            active_tab: 0,
            selected: 0,
            picker_index: 0,
            current_scroll: 0.0,
            tabs,
            visible,
            net_line,
            scan_log,
            texture_cache: RefCell::new(TextureCache::default()),
            art_state: HashMap::new(),
            known_art,
            art_jobs_in_flight: 0,
            request_tx,
            done_rx,
            scan_rx: Some(scan_rx),
            lookup_rx: Some(lookup_rx),
            net_ready,
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
            preload_pending: std::collections::HashSet::new(),
            preload_total: 0,
            store_detail: None,
            shot_tx: Some(shot_tx),
            shot_rx: Some(shot_rx),
            hero_lookup_rx: Some(hero_lookup_rx),
            hero_lookup_tx,
            config,
            cache_stats,
            settings_selected: 0,
            tab_before_settings: 0,
            music_requested: std::collections::HashSet::new(),
            music_retry_after: HashMap::new(),
            frame_counter: 0,
            runtime: crate::runtime::VitaRuntime::new(),
            cache_notice: None,
            download_confirm: None,
        }
    }

    pub fn is_loading(&self) -> bool {
        (self.games.is_empty() && (self.scan_rx.is_some() || self.lookup_rx.is_some()))
            || !self.preload_pending.is_empty()
    }

    pub fn preload_progress(&self) -> Option<(usize, usize)> {
        if self.preload_total == 0 || self.preload_pending.is_empty() {
            None
        } else {
            Some((self.preload_total - self.preload_pending.len(), self.preload_total))
        }
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

    pub fn art_is_pending(&self, title_id: &str) -> bool {
        matches!(self.art_state.get(title_id), Some(ArtState::Pending) | None)
    }

    pub fn store_item(&self, title_id: &str) -> Option<&StoreItem> {
        self.store.item_by_title_id(title_id)
    }

    pub fn is_title_installed(&self, title_id: &str) -> bool {
        self.games
            .iter()
            .any(|g| g.title_id == title_id && g.has_bubble)
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
        net::wifi_available()
    }

    pub fn tab_is_collection(&self) -> bool {
        !self.is_settings() && collection_index_of_tab(self.active_tab).is_some()
    }

    fn pump_art_queue(&mut self) {
        if !self.net_ready {
            return;
        }
        let preloading = !self.preload_pending.is_empty();
        if self.art_jobs_in_flight >= self.art_jobs_budget() {
            return;
        }
        if !net::wifi_available() {
            return;
        }

        let game_index = if preloading { self.next_preload_candidate() } else { self.next_art_candidate() };
        let Some(game_index) = game_index else { return };

        let is_store = self.is_store_tab();
        let target_pool = if is_store { &self.store_games } else { &self.games };
        let Some(game) = target_pool.get(game_index) else { return };

        let known = self.known_art.get(&game.art_key).cloned().unwrap_or_default();
        let known_music_url = if !self.config.download_bgm || game.music_resolved || preloading || is_store { None } else { known.music_url };

        let store_item = if is_store {
            self.store.item_by_title_id(&game.title_id)
        } else {
            None
        };
        let final_cover_url = if is_store {
            store_item.and_then(|i| i.resolved_cover_url()).or(known.cover_url)
        } else {
            known.cover_url.or_else(|| {
                store_item
                    .and_then(|i| i.cover_url.as_deref())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(crate::store::absolute_url)
            })
        };
        let known_screenshot_url = if is_store {
            store_item.and_then(|i| i.resolved_screenshot_urls().into_iter().next())
        } else {
            known.screenshot_url
        };
        let known_icon_url = if is_store {
            store_item.and_then(|i| i.resolved_icon_url())
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
            if tx.send(job).is_ok() {
                self.art_jobs_in_flight += 1;
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
            Warning => ART_LOOKAHEAD / 2,
            Aggressive | Emergency => 0,
        }
    }

    fn art_jobs_budget(&self) -> usize {
        use crate::runtime::MemoryPressure::*;
        let base = if !self.preload_pending.is_empty() {
            PRELOAD_MAX_ART_JOBS_IN_FLIGHT
        } else if self.is_store_tab() {
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

    fn next_preload_candidate(&mut self) -> Option<usize> {
        for (index, game) in self.games.iter().enumerate() {
            if self.preload_pending.contains(&game.title_id) && !self.art_state.contains_key(&game.title_id) {
                return Some(index);
            }
        }
        None
    }

    fn store_keep_slots(&self) -> (usize, usize) {
        if self.visible.is_empty() {
            return (0, 0);
        }
        let stride = card_row_stride(SCREEN_W).max(1.0);
        let view_h = (SCREEN_H - HEADER_H - FOOTER_H).max(1.0);
        let first_row = (self.current_scroll / stride).floor().max(0.0) as usize;
        let rows_on_screen = ((view_h / stride).ceil() as usize).max(1);
        let start = first_row.saturating_mul(GRID_COLS);
        let end = start
            .saturating_add((rows_on_screen + 1).saturating_mul(GRID_COLS))
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
            &self.collections,
            &self.recent,
            self.active_tab,
            &self.search_query,
        );
        self.selected = selected_index
            .and_then(|idx| self.visible.iter().position(|&i| i == idx))
            .unwrap_or(0)
            .min(self.visible.len().saturating_sub(1));
    }

    fn apply_pending_lookup(&mut self) {
        if let Some(rx) = &self.scan_rx {
            if let Ok((scanned_games, scan_log)) = rx.try_recv() {
                self.scan_rx = None;
                self.scan_log = scan_log;

                let list_changed = self.games.len() != scanned_games.len()
                    || self.games.iter().zip(scanned_games.iter()).any(|(a, b)| a.art_key != b.art_key);

                if list_changed || self.games.is_empty() {
                    self.games = scanned_games;
                    self.refilter_visible();
                }

                crate::cache_manager::clean_orphaned_cache(&self.games);
                crate::cache_manager::enforce_cache_budget(&self.games, self.config.cache_budget_mb);
                self.cache_stats = crate::cache_manager::compute_cache_stats(&self.games);
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

                if self.net_ready {
                    let known_art = &self.known_art;
                    self.preload_pending = self
                        .games
                        .iter()
                        .filter(|g| {
                            let Some(known) = known_art.get(&g.art_key) else { return false };
                            (known.cover_url.is_some() && !g.has_box_art)
                                || (known.screenshot_url.is_some() && !g.has_hero)
                                || (known.logo_url.is_some() && !g.has_logo)
                                || (known.music_url.is_some() && !g.music_resolved)
                        })
                        .map(|g| g.title_id.clone())
                        .collect();
                    self.preload_total = self.preload_pending.len();
                }

                if !self.is_settings() {
                    self.refilter_visible();
                }
            }
        }
    }

    pub fn tick(&mut self, ctx: &egui::Context) {
        self.frame_counter = self.frame_counter.wrapping_add(1);
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

        self.apply_pending_lookup();

        if let Some(res) = self.ime.poll() {
            match res {
                crate::ime::ImeResult::Confirmed(text) => {
                    let trimmed = text.trim().to_string();
                    if trimmed.is_empty() {
                        self.search_query.clear();
                        self.search_active = false;
                    } else {
                        self.search_query = trimmed;
                        self.search_active = true;
                    }
                    self.refilter_visible();
                }
                crate::ime::ImeResult::Canceled => {
                    if self.search_query.is_empty() {
                        self.search_active = false;
                    }
                }
            }
        }

        if self.store.tick() || (self.store_games.is_empty() && !self.store.items.is_empty()) {
            for item in &self.store.items {
                if let Some(title_id) = &item.title_id {
                    self.known_art.insert(title_id.clone(), store_known_art(item));
                }
            }
            self.store_games = self.store.to_games();
            if self.is_store_tab() {
                self.refilter_visible();
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
            while let Ok(result) = rx.try_recv() {

                let ArtResult {
                    title_id,
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
                    self.preload_pending.remove(&title_id);
                }
                let selected_title_id =
                    self.visible.get(self.selected).and_then(|&i| self.games.get(i)).map(|g| g.title_id.clone());

                let store_index = self.store_games.iter().position(|g| g.title_id == title_id);
                let store_wants_cover = store_index
                    .is_some_and(|index| self.is_store_tab() && self.cover_keep_set().contains(&index));
                let mut cover = cover;

                if let Some(game) = self.games.iter_mut().find(|g| g.title_id == title_id) {
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

                let mut store_discarded = false;
                if let Some(index) = store_index {
                    if store_wants_cover {
                        if let Some(game) = self.store_games.get_mut(index) {
                            if let Some(cover_bytes) = cover.take() {
                                game.cover_bytes = Some((cover_bytes.is_png, cover_bytes.bytes));
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
                    if net::wifi_available() { "yes" } else { "no" },
                    self.dl_ok,
                    self.dl_fail,
                    self.last_error
                );
            }
        }

        let selected_index = self.visible.get(self.selected).copied();

        while let Ok((art_key, hero, logo, cover)) = self.local_art_rx.try_recv() {
            if let Some(game) = self.games.iter_mut().find(|g| g.art_key == art_key) {
                if let Some(h) = hero {
                    game.hero_bytes = Some(h);
                }
                if let Some(l) = logo {
                    game.logo_bytes = Some(l);
                }
                if let Some(c) = cover {
                    game.cover_bytes = Some(c);
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
                    let hero = if needs_hero { load_cached_hero(system, &art_key) } else { None };
                    let logo = if needs_logo { load_cached_logo(system, &art_key) } else { None };

                    if let Some(game) = self.games.get_mut(selected_index) {
                        if needs_hero && hero.is_none() {
                            game.has_hero = false;
                            self.art_state.remove(&title_id);
                        }
                        if needs_logo && logo.is_none() {
                            game.has_logo = false;
                            self.art_state.remove(&title_id);
                        }
                    }

                    if hero.is_some() || logo.is_some() {
                        let _ = self.local_art_tx.send((art_key, hero, logo, None));
                    }
                }
            }
        }

        if !self.is_store_tab() {
            for index in self.cover_keep_set() {
                if let Some(game) = self.games.get_mut(index) {
                    if game.cover_bytes.is_none() && game.has_box_art {
                        game.cover_bytes = load_cached_cover(game.system, &game.art_key);
                        if game.cover_bytes.is_none() {
                            game.has_box_art = false;
                        }
                    }
                }
            }
        } else {
            for index in self.cover_keep_set() {
                let Some(game) = self.store_games.get(index) else { continue };
                if game.cover_bytes.is_some() || !self.is_title_installed(&game.title_id) {
                    continue;
                }
                let (system, art_key, title_id) = (game.system, game.art_key.clone(), game.title_id.clone());
                let local = load_cached_cover(system, &art_key)
                    .or_else(|| crate::scanner::load_vita_cover(&title_id).0);
                if let Some(bytes) = local {
                    if let Some(game) = self.store_games.get_mut(index) {
                        game.cover_bytes = Some(bytes);
                    }
                }
            }
        }

        self.release_offscreen_art(selected_index);
        self.pump_store_media();
        self.pump_music(selected_index);
    }

    const MUSIC_SETTLE_FRAMES: u32 = 18;

    const ART_SETTLE_FRAMES: u32 = 2;

    fn pump_music(&mut self, selected_index: Option<usize>) {
        if self.is_settings() || self.is_store_tab() || selected_index.is_none() {
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
        if !self.config.download_bgm || !self.net_ready || !net::wifi_available() {
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
            if tx.send(job).is_ok() {
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

    fn drop_store_cover(&mut self, index: usize) {
        let Some(game) = self.store_games.get_mut(index) else { return };
        game.cover_bytes = None;
        game.hero_bytes = None;
        let title_id = game.title_id.clone();
        if !matches!(self.art_state.get(&title_id), Some(ArtState::Pending)) {
            self.art_state.remove(&title_id);
        }
    }

    fn release_offscreen_art(&mut self, selected_index: Option<usize>) {
        let cover_keep = self.cover_keep_set();
        if self.is_store_tab() {
            let drop: Vec<usize> = self
                .store_games
                .iter()
                .enumerate()
                .filter(|(index, game)| {
                    Some(*index) != selected_index
                        && (game.cover_bytes.is_some() || game.hero_bytes.is_some())
                        && !cover_keep.contains(index)
                })
                .map(|(index, _)| index)
                .collect();
            for index in drop {
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
                self.audio.play(SoundEffect::TabSwitch);
                self.change_tab(-1);
            }
            AppCommand::TabNext => {
                self.audio.play(SoundEffect::TabSwitch);
                self.change_tab(1);
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
                } else if !self.visible.is_empty() {
                    self.audio.play(SoundEffect::OpenModal);
                    self.mode = Mode::CollectionPicker;
                    self.picker_index = 0;
                }
            }
            AppCommand::RemoveFromCollection => {
                self.audio.play(SoundEffect::Confirm);
                self.remove_selected_from_collection();
            }
            AppCommand::ToggleSearch => {
                if !self.is_store_tab() {
                    return;
                }
                if self.search_active && !self.search_query.is_empty() {
                    self.audio.play(SoundEffect::Confirm);
                    self.search_query.clear();
                    self.search_active = false;
                    self.refilter_visible();
                } else {
                    self.audio.play(SoundEffect::OpenModal);
                    let title = if self.is_store_tab() {
                        "Search the store"
                    } else {
                        "Search games"
                    };
                    self.ime.open(title, &self.search_query, 64);
                    self.search_active = true;
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
                if self.is_settings() {
                    match self.settings_selected {
                        0 => self.trigger_rescan(),
                        1 => self.handle_command(AppCommand::ToggleDownloadBgm),
                        2 => self.handle_command(AppCommand::CycleCacheBudget),
                        3 => self.handle_command(AppCommand::CleanOrphanCache),
                        4 => self.handle_command(AppCommand::PurgeMusicCache),
                        5 => self.handle_command(AppCommand::PurgeAllCache),
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
                self.cache_notice = Some(format!(
                    "Background music: {}",
                    if self.config.download_bgm { "Enabled" } else { "Disabled" }
                ));
            }
            AppCommand::CycleCacheBudget => {
                self.audio.play(SoundEffect::Confirm);
                self.config.next_budget_option();
                let freed = crate::cache_manager::enforce_cache_budget(&self.games, self.config.cache_budget_mb);
                self.cache_stats = crate::cache_manager::compute_cache_stats(&self.games);
                self.cache_notice = Some(format!(
                    "Cache limit: {} ({} freed)",
                    self.config.budget_label(),
                    crate::cache_manager::format_bytes(freed)
                ));
            }
            AppCommand::CleanOrphanCache => {
                self.audio.play(SoundEffect::Confirm);
                let (count, freed) = crate::cache_manager::clean_orphaned_cache(&self.games);
                self.cache_stats = crate::cache_manager::compute_cache_stats(&self.games);
                self.cache_notice = Some(format!(
                    "Orphans removed: {} files ({})",
                    count,
                    crate::cache_manager::format_bytes(freed)
                ));
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
                self.cache_notice = Some(format!(
                    "Music purged: {} files ({})",
                    count,
                    crate::cache_manager::format_bytes(freed)
                ));
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
                self.cache_notice = Some(format!(
                    "All cache purged: {} files ({})",
                    count,
                    crate::cache_manager::format_bytes(freed)
                ));
            }
            AppCommand::TogglePickerRow(index) => {
                self.audio.play(SoundEffect::Confirm);
                self.toggle_picker_row(index);
            }
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
                } else if self.search_active {
                    self.audio.play(SoundEffect::CloseModal);
                    self.search_active = false;
                    self.search_query.clear();
                    self.refilter_visible();
                } else if self.mode == Mode::CollectionPicker {
                    self.audio.play(SoundEffect::CloseModal);
                    self.mode = Mode::Browse;
                    self.tabs = build_tabs(&self.collections);
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
                    }
                }
                InputCommand::MoveDown => {
                    if self.is_settings() {
                        if self.settings_selected < 5 {
                            self.settings_selected += 1;
                            self.audio.play(SoundEffect::Navigate);
                        }
                    } else if self.is_store_tab() {
                        if self.selected + 1 < self.visible.len() {
                            self.selected = (self.selected + GRID_COLS).min(self.visible.len().saturating_sub(1));
                            self.audio.play(SoundEffect::Navigate);
                        }
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
        if tab == RECENT_TAB_INDEX {
            self.recent
                .order()
                .iter()
                .filter(|title_id| self.games.iter().any(|g| &g.title_id == *title_id))
                .count()
        } else if tab == STORE_TAB_INDEX {
            self.store_games.len()
        } else {
            match collection_index_of_tab(tab) {
                Some(collection_idx) => self
                    .games
                    .iter()
                    .filter(|g| self.collections.contains(collection_idx, &g.title_id))
                    .count(),
                None => {
                    let system = SYSTEM_TABS.get(tab).and_then(|(_, s)| *s);
                    self.games
                        .iter()
                        .filter(|g| g.is_game != Some(false))
                        .filter(|g| system.map_or(true, |s| g.system == s))
                        .count()
                }
            }
        }
    }

    pub fn is_store_tab(&self) -> bool {
        self.active_tab == STORE_TAB_INDEX
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
        let leaving_store = self.is_store_tab() && index != STORE_TAB_INDEX;
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
                let _ = tx.send((title_id.clone(), -1, url));
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
        let title = self
            .store
            .item_by_title_id(title_id)
            .map(|i| i.name.clone())
            .unwrap_or_else(|| title_id.to_string());
        match self.store.download_and_install(title_id) {
            Ok(()) => {
                self.audio.play(crate::audio::SoundEffect::LaunchGame);
                self.launch_notice = Some((
                    title_id.to_string(),
                    format!("Enqueued download for '{title}'. Added to LiveArea downloads!"),
                ));
            }
            Err(e) => {
                self.audio.play(crate::audio::SoundEffect::CloseModal);
                self.launch_notice = Some((title_id.to_string(), format!("Download error: {e}")));
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
                                        let _ = tx.send((title_id, -1, url));
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
                if tx.send((title_id.clone(), i as i32, url)).is_ok() {
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

        let mut launch_candidates = vec![game.title_id.clone()];

        if game.system != System::Vita {
            if !game.has_bubble {
                let Some(file_path) = game.file_path.clone() else {
                    self.launch_notice = Some((
                        game.title_id.clone(),
                        "No se encontró la ruta local del juego PSP/PS1".to_string(),
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
                        "No se pudo preparar el launcher de Adrenaline (revisa PSPEMUCFW/PSPEMU001)".to_string(),
                    ));
                    return;
                }
            }
        }

        self.launch_notice = None;
        self.recent.touch(&game.title_id);
        crate::logger::log(&format!(
            "launch_selected: {} (candidates: {})",
            game.title_id,
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
                game.title_id.clone(),
                "No se pudo lanzar el juego (falló AppMgr en todos los launchers)".to_string(),
            ));
        }

        if self.active_tab == RECENT_TAB_INDEX {
            self.refilter_visible();
        }
    }

    fn remove_selected_from_collection(&mut self) {
        let Some(collection_idx) = collection_index_of_tab(self.active_tab) else { return };
        let Some(game) = self.visible.get(self.selected).and_then(|&i| self.games.get(i)) else { return };
        let title_id = game.title_id.clone();
        if self.collections.remove(collection_idx, &title_id) {
            self.refilter_visible();
        }
    }

    fn toggle_picker_row(&mut self, index: usize) {
        if self.mode != Mode::CollectionPicker {
            return;
        }
        let Some(game) = self.visible.get(self.selected).and_then(|&i| self.games.get(i)) else { return };
        let title_id = game.title_id.clone();
        self.collections.toggle(index, &title_id);
        self.picker_index = index;
        self.refilter_visible();
    }

    pub fn trigger_rescan(&mut self) {
        crate::logger::log("App::trigger_rescan requested");
        let _ = std::fs::remove_file(crate::scanner::GAMES_CACHE_FILE);
        let net_ready = self.net_ready;
        let (scan_tx, scan_rx) = mpsc::channel();
        let (lookup_tx, lookup_rx) = mpsc::channel();
        std::thread::spawn(move || {
            crate::logger::log("Background rescan thread started");
            let (scanned_games, scan_log) = scan_installed_games();
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

fn build_tabs(collections: &Collections) -> Vec<String> {
    SYSTEM_TABS
        .iter()
        .map(|(name, _)| name.to_string())
        .chain(std::iter::once("RECENTLY PLAYED".to_string()))
        .chain(std::iter::once("STORE".to_string()))
        .chain(collections.items.iter().map(|c| c.name.to_uppercase()))
        .collect()
}

fn collection_index_of_tab(tab: usize) -> Option<usize> {
    if tab <= STORE_TAB_INDEX {
        None
    } else {
        Some(tab - STORE_TAB_INDEX - 1)
    }
}

fn filter_games(
    games: &[Game],
    store_games: &[Game],
    collections: &Collections,
    recent: &RecentlyPlayed,
    tab: usize,
    search: &str,
) -> Vec<usize> {
    let raw: Vec<usize> = if tab == RECENT_TAB_INDEX {
        recent
            .order()
            .iter()
            .filter_map(|title_id| games.iter().position(|g| &g.title_id == title_id))
            .collect()
    } else if tab == STORE_TAB_INDEX {
        (0..store_games.len()).collect()
    } else {
        match collection_index_of_tab(tab) {
            Some(collection_idx) => games
                .iter()
                .enumerate()
                .filter(|(_, g)| collections.contains(collection_idx, &g.title_id))
                .map(|(i, _)| i)
                .collect(),
            None => {
                let system = SYSTEM_TABS.get(tab).and_then(|(_, s)| *s);
                games
                    .iter()
                    .enumerate()
                    .filter(|(_, g)| g.is_game != Some(false))
                    .filter(|(_, g)| system.map_or(true, |s| g.system == s))
                    .map(|(i, _)| i)
                    .collect()
            }
        }
    };

    if tab != STORE_TAB_INDEX {
        return raw;
    }

    let target_list = store_games;
    let query = search.trim().to_lowercase();
    if query.is_empty() {
        raw
    } else {
        raw.into_iter()
            .filter(|&i| target_list.get(i).is_some_and(|g| contains_ignore_case(&g.title, &query)))
            .collect()
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

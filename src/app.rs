use crate::artwork::{self, ArtJob, ArtResult};
use crate::collections::Collections;
use crate::input::{AppCommand, InputCommand};
use crate::net;
use crate::recent::RecentlyPlayed;
use crate::scanner::{
    load_cached_cover, load_cached_hero, load_cached_logo, resolved_music_path, scan_installed_games, Game,
    ImageBytes, System,
};
use crate::textures::TextureCache;
use crate::ui::{Mode, TILE_SPACING};
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::mpsc;

const SCROLL_LERP: f32 = 0.22;

const MAX_ART_JOBS_IN_FLIGHT: usize = 1;

const COVER_KEEP_RADIUS: usize = 8;

const SYSTEM_TABS: [(&str, Option<System>); 4] = [
    ("ALL", None),
    ("PS VITA", Some(System::Vita)),
    ("PSP", Some(System::Psp)),
    ("PS1", Some(System::Psx)),
];
const RECENT_TAB_INDEX: usize = SYSTEM_TABS.len();

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

pub struct App {
    pub games: Vec<Game>,
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
    /// (title_id, message) shown under the hero title when the selected game has no
    /// Adrenaline bubble to launch — cleared implicitly once a different game is selected.
    pub launch_notice: Option<(String, String)>,
    local_art_tx: mpsc::Sender<(String, Option<ImageBytes>, Option<ImageBytes>, Option<ImageBytes>)>,
    local_art_rx: mpsc::Receiver<(String, Option<ImageBytes>, Option<ImageBytes>, Option<ImageBytes>)>,
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
            let _ = scan_tx.send((scanned_games.clone(), scan_log));

            if net_ready {
                let targets: Vec<artwork::LookupTarget> =
                    scanned_games.iter().map(artwork::LookupTarget::from_game).collect();
                let res = artwork::lookup_batch(&targets);
                let _ = lookup_tx.send(res);
            }
        });

        let scan_log: Vec<String> = Vec::new();
        let known_art: HashMap<String, KnownArt> = HashMap::new();

        crate::logger::log("App::new: loading collections/recent");
        let collections = Collections::load();
        let recent = RecentlyPlayed::load();
        let tabs = build_tabs(&collections);
        let visible = filter_games(&games, &collections, &recent, 0, "");
        crate::logger::log("App::new: collections OK");

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

        App {
            games,
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
            launch_notice: None,
            local_art_tx,
            local_art_rx,
        }
    }

    pub fn is_loading(&self) -> bool {
        self.games.is_empty() && (self.scan_rx.is_some() || self.lookup_rx.is_some())
    }

    pub fn texture_cache_get(
        &self,
        ctx: &egui::Context,
        key: &str,
        bytes: &ImageBytes,
    ) -> Option<egui::TextureHandle> {
        self.texture_cache.borrow_mut().get_or_decode(ctx, key, bytes)
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
        if !self.net_ready || self.art_jobs_in_flight >= MAX_ART_JOBS_IN_FLIGHT {
            return;
        }
        if !net::wifi_available() {
            return;
        }

        let Some(game_index) = self.next_art_candidate() else { return };
        let Some(game) = self.games.get(game_index) else { return };

        let known = self.known_art.get(&game.art_key).cloned().unwrap_or_default();

        let known_music_url = if game.music_resolved { None } else { known.music_url };

        if known.cover_url.is_none()
            && known.screenshot_url.is_none()
            && known.logo_url.is_none()
            && known_music_url.is_none()
        {
            let title_id = game.title_id.clone();
            self.art_state.insert(title_id, ArtState::Failed);
            return;
        }

        let job = ArtJob {
            title_id: game.title_id.clone(),
            art_key: game.art_key.clone(),
            title: game.title.clone(),
            system: game.system,
            known_cover_url: known.cover_url,
            known_screenshot_url: known.screenshot_url,
            known_logo_url: known.logo_url,
            known_music_url,
        };
        self.art_state.insert(job.title_id.clone(), ArtState::Pending);
        if let Some(tx) = &self.request_tx {
            if tx.send(job).is_ok() {
                self.art_jobs_in_flight += 1;
            }
        }
    }

    fn next_art_candidate(&mut self) -> Option<usize> {

        let needs_art = |game: &Game| {
            !(game.has_box_art && game.has_hero && game.has_logo && game.music_resolved)
        };

        let near_end = (self.selected + 3).min(self.visible.len().saturating_sub(1));
        let near_start = self.selected.saturating_sub(2);
        if near_start > near_end {
            return None;
        }

        let ordered = std::iter::once(self.selected).chain(near_start..=near_end);
        for slot in ordered {
            let Some(&index) = self.visible.get(slot) else { continue };
            let Some(game) = self.games.get(index) else { continue };
            if needs_art(game) && !self.art_state.contains_key(&game.title_id) {
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

                if !self.is_settings() {
                    self.refilter_visible();
                }
            }
        }
    }

    pub fn tick(&mut self, _ctx: &egui::Context) {
        self.apply_pending_lookup();

        let target_scroll = self.selected as f32 * TILE_SPACING;
        self.current_scroll += (target_scroll - self.current_scroll) * SCROLL_LERP;
        if (target_scroll - self.current_scroll).abs() < 1.0 {
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

                if let Some(game) = self.games.iter_mut().find(|g| g.title_id == title_id) {
                    if let Some(cover) = cover {
                        game.cover_bytes = Some((cover.is_png, cover.bytes));
                        game.has_box_art = true;
                        self.texture_cache.borrow_mut().invalidate(&format!("{}:{}:cover", game.system.label(), game.title_id));
                    }
                    if let Some(hero) = hero {
                        game.hero_bytes = Some((hero.is_png, hero.bytes));
                        game.has_hero = true;
                        self.texture_cache.borrow_mut().invalidate(&format!("{}:{}:hero", game.system.label(), game.title_id));
                    }
                    if let Some(logo) = logo {
                        game.logo_bytes = Some((logo.is_png, logo.bytes));
                        game.has_logo = true;
                        self.texture_cache.borrow_mut().invalidate(&format!("{}:{}:logo", game.system.label(), game.title_id));
                    }
                    if music_downloaded {
                        game.music_resolved = true;
                    }
                }

                if music_downloaded && selected_title_id.as_deref() == Some(title_id.as_str()) {
                    self.music_settle_frames = 0;
                }

                if job_complete {
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

        for index in self.cover_keep_set() {
            if let Some(game) = self.games.get(index) {
                if game.cover_bytes.is_none() && game.has_box_art {

                }
            }
        }

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

        self.release_offscreen_art(selected_index);
        self.pump_music(selected_index);
    }

    const MUSIC_SETTLE_FRAMES: u32 = 18;

    const ART_SETTLE_FRAMES: u32 = 2;

    fn pump_music(&mut self, selected_index: Option<usize>) {
        if self.is_settings() || selected_index.is_none() {
            self.audio.set_music(None);
            self.music_selected = None;
            self.music_settle_frames = 0;
            return;
        }

        if self.music_settle_frames < Self::MUSIC_SETTLE_FRAMES {
            return;
        }
        let path = selected_index.and_then(|i| self.games.get(i)).and_then(resolved_music_path);
        self.audio.set_music(path.as_deref());
    }

    fn cover_keep_set(&self) -> std::collections::HashSet<usize> {
        let mut keep = std::collections::HashSet::new();
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

    fn release_offscreen_art(&mut self, selected_index: Option<usize>) {
        let cover_keep = self.cover_keep_set();
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
            AppCommand::SelectTab(index) => {
                if index != self.active_tab {
                    self.audio.play(SoundEffect::TabSwitch);
                }
                self.set_tab(index);
            }
            AppCommand::OpenCollectionPicker => {
                if !self.visible.is_empty() {
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
                self.audio.play(SoundEffect::OpenModal);
                self.search_active = !self.search_active;
                if !self.search_active {
                    self.search_query.clear();
                }
                self.refilter_visible();
            }
            AppCommand::Confirm => {
                self.audio.play(SoundEffect::Confirm);
                if self.is_settings() {
                    self.trigger_rescan();
                } else if self.mode == Mode::CollectionPicker {
                    self.toggle_picker_row(self.picker_index);
                } else {
                    self.launch_selected();
                }
            }
            AppCommand::Rescan => {
                self.audio.play(SoundEffect::Confirm);
                self.trigger_rescan();
            }
            AppCommand::TogglePickerRow(index) => {
                self.audio.play(SoundEffect::Confirm);
                self.toggle_picker_row(index);
            }
            AppCommand::Back => {
                if self.search_active {
                    self.audio.play(SoundEffect::CloseModal);
                    self.search_active = false;
                    self.search_query.clear();
                    self.refilter_visible();
                } else if self.mode == Mode::CollectionPicker {
                    self.audio.play(SoundEffect::CloseModal);
                    self.mode = Mode::Browse;
                    self.tabs = build_tabs(&self.collections);
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
                if self.mode == Mode::Browse {
                    std::process::exit(0);
                }
            }
        }
    }

    fn handle_input(&mut self, input: InputCommand) {
        use crate::audio::SoundEffect;
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
                    if self.selected > 0 {
                        self.selected = self.selected.saturating_sub(5);
                        self.audio.play(SoundEffect::Navigate);
                    }
                }
                InputCommand::MoveDown => {
                    if self.selected + 1 < self.visible.len() {
                        self.selected = (self.selected + 5).min(self.visible.len().saturating_sub(1));
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
        }
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
        let total = self.settings_tab_index() as i32 + 1;
        let next = (self.active_tab as i32 + delta).rem_euclid(total);
        self.set_tab(next as usize);
    }

    fn set_tab(&mut self, index: usize) {
        if index > self.settings_tab_index() || self.mode != Mode::Browse {
            return;
        }
        self.active_tab = index;
        self.selected = 0;
        self.current_scroll = 0.0;
        self.refilter_visible();
    }

    fn launch_selected(&mut self) {
        let Some(game) = self.visible.get(self.selected).and_then(|&i| self.games.get(i)) else { return };

        if !game.has_bubble {
            self.launch_notice = Some((
                game.title_id.clone(),
                format!("Crea una burbuja con ABM (BubbleID = {}) para poder lanzar este juego", game.title_id),
            ));
            return;
        }
        self.launch_notice = None;

        self.recent.touch(&game.title_id);
        self.audio.set_music(None);
        self.music_selected = None;
        self.music_settle_frames = 0;

        #[cfg(target_os = "vita")]
        {
            let uri = format!("psgm:play?titleid={}", game.title_id);
            if let Ok(c_uri) = std::ffi::CString::new(uri) {
                unsafe {
                    vitasdk_sys::sceAppMgrLaunchAppByUri(0, c_uri.as_ptr());
                }
                std::process::exit(0);
            }
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
            let _ = scan_tx.send((scanned_games.clone(), scan_log));

            if net_ready {
                let targets: Vec<artwork::LookupTarget> =
                    scanned_games.iter().map(artwork::LookupTarget::from_game).collect();
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
        .chain(collections.items.iter().map(|c| c.name.to_uppercase()))
        .collect()
}

fn collection_index_of_tab(tab: usize) -> Option<usize> {
    if tab <= RECENT_TAB_INDEX {
        None
    } else {
        Some(tab - RECENT_TAB_INDEX - 1)
    }
}

fn filter_games(
    games: &[Game],
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

    let query = search.trim().to_lowercase();
    if query.is_empty() {
        raw
    } else {
        raw.into_iter()
            .filter(|&i| {
                let g = &games[i];
                g.title.to_lowercase().contains(&query)
                    || g.title_id.to_lowercase().contains(&query)
                    || g.art_key.to_lowercase().contains(&query)
            })
            .collect()
    }
}

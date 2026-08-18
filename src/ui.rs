use crate::app::App;
use crate::input::AppCommand;
use crate::scanner::{Game, ImageBytes};
use egui::{Color32, CornerRadius, FontId, Pos2, Rect, Sense, Stroke, Vec2};
use std::sync::OnceLock;

pub const SCREEN_W: f32 = 960.0 / 1.3;
#[allow(dead_code)]
pub const SCREEN_H: f32 = 544.0 / 1.3;

pub const GRID_COLS: usize = 6;
pub const CARD_ASPECT: f32 = 1.44; 

pub const HEADER_H: f32 = 58.0;
pub const FOOTER_H: f32 = 32.0;
pub const MARGIN_X: f32 = 24.0;
pub const COL_GAP: f32 = 10.0;
pub const ROW_GAP: f32 = 10.0;

pub const TILE_W: f32 = 84.0;
pub const TILE_H: f32 = 118.0;
pub const TILE_SPACING: f32 = 96.0;
pub const TILE_SELECTED_SCALE: f32 = 1.08;

pub const COL_BG: Color32 = Color32::from_rgb(18, 22, 30);
pub const COL_HEADER_BG: Color32 = Color32::from_rgba_premultiplied(14, 18, 25, 245);
pub const COL_FOOTER_BG: Color32 = Color32::from_rgba_premultiplied(13, 16, 23, 245);

pub const COL_CARD_BG: Color32 = Color32::from_rgb(26, 32, 44);
pub const COL_CARD_BORDER: Color32 = Color32::from_rgb(42, 50, 68);
pub const COL_CARD_SELECTED_BORDER: Color32 = Color32::from_rgb(228, 236, 248);

pub const COL_TAB_ACTIVE: Color32 = Color32::from_rgb(46, 56, 76);
pub const COL_TEXT: Color32 = Color32::from_rgb(240, 243, 248);
pub const COL_TEXT_DIM: Color32 = Color32::from_rgb(148, 158, 174);
pub const COL_TEXT_MUTED: Color32 = Color32::from_rgb(105, 115, 130);

pub const COL_SEARCH_BG: Color32 = Color32::from_rgba_premultiplied(28, 34, 46, 240);
pub const COL_SEARCH_BORDER: Color32 = Color32::from_rgba_premultiplied(50, 60, 80, 240);

pub const COL_BADGE_INSTALLED: Color32 = Color32::from_rgb(46, 204, 113);
pub const COL_BADGE_FAV: Color32 = Color32::from_rgb(245, 158, 11);

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Browse,
    CollectionPicker,
    StoreDetail,
}

const JP_FONT: &[u8] = include_bytes!("../assets/fonts/NotoSansJP-Subset.otf");
const NO_COVER_PNG: &[u8] = include_bytes!("../assets/no-cover.png");

pub fn apply_theme(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        "noto-jp".to_owned(),
        std::sync::Arc::new(egui::FontData::from_static(JP_FONT)),
    );

    for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        fonts
            .families
            .entry(family)
            .or_default()
            .push("noto-jp".to_owned());
    }
    ctx.set_fonts(fonts);

    ctx.set_visuals(egui::Visuals::dark());
    let mut style = (*ctx.style()).clone();
    style.spacing.item_spacing = Vec2::new(6.0, 4.0);
    style.visuals.window_shadow = egui::epaint::Shadow::NONE;
    ctx.set_style(style);
}

pub fn card_dimensions(screen_width: f32) -> (f32, f32) {
    let available_w = screen_width - (MARGIN_X * 2.0) - (COL_GAP * (GRID_COLS as f32 - 1.0));
    let card_w = (available_w / GRID_COLS as f32).max(50.0);
    let card_h = (card_w * CARD_ASPECT).round();
    (card_w.round(), card_h)
}

pub fn card_row_stride(screen_width: f32) -> f32 {
    let (_, card_h) = card_dimensions(screen_width);
    card_h + ROW_GAP
}

fn label_at(ui: &mut egui::Ui, pos: Pos2, text: &str, size: f32, color: Color32) {
    ui.painter().text(pos, egui::Align2::LEFT_TOP, text, FontId::proportional(size), color);
}

fn label_mid(ui: &mut egui::Ui, left: f32, center_y: f32, text: &str, size: f32, color: Color32) {
    ui.painter().text(
        Pos2::new(left, center_y),
        egui::Align2::LEFT_CENTER,
        text,
        FontId::proportional(size),
        color,
    );
}

fn label_center(ui: &mut egui::Ui, center_x: f32, center_y: f32, text: &str, size: f32, color: Color32) {
    ui.painter().text(
        Pos2::new(center_x, center_y),
        egui::Align2::CENTER_CENTER,
        text,
        FontId::proportional(size),
        color,
    );
}

fn filled_rect(ui: &mut egui::Ui, rect: Rect, color: Color32) {
    ui.painter().rect_filled(rect, CornerRadius::ZERO, color);
}

fn rounded_panel(ui: &mut egui::Ui, rect: Rect, color: Color32) {
    let r = rect.height().min(24.0) / 2.0;
    ui.painter().rect_filled(rect, CornerRadius::same(r as u8), color);
}

pub fn build_ui(ctx: &egui::Context, app: &App) -> Vec<AppCommand> {
    let mut commands = Vec::new();
    egui::CentralPanel::default()
        .frame(egui::Frame::NONE.fill(COL_BG))
        .show(ctx, |ui| {
            let screen = ui.max_rect();
            filled_rect(ui, screen, COL_BG);
            preload_button_icons(ui);

            if app.is_loading() {
                draw_full_loading_screen(ui, app, screen);
            } else if app.is_settings() {
                draw_settings(ui, app, screen, &mut commands);
                draw_header(ui, app, screen, &mut commands);
                draw_footer(ui, app);
            } else if app.mode == Mode::StoreDetail {
                draw_store_detail(ui, app, screen);
                draw_footer(ui, app);
            } else {
                if app.visible.is_empty() {
                    draw_empty_state(ui, app, screen);
                } else if app.is_store_tab() {
                    draw_game_grid(ui, app, screen, &mut commands);
                } else {
                    draw_backdrop(ui, app, screen);
                    draw_corner_art(ui, app, screen);
                    draw_hero_text(ui, app, screen);
                    draw_cover_row(ui, app, screen, &mut commands);
                }

                draw_header(ui, app, screen, &mut commands);
                draw_footer(ui, app);

                if app.mode == Mode::CollectionPicker {
                    draw_collection_picker(ui, app, screen, &mut commands);
                }
            }

            if app.download_confirm.is_some() {
                draw_download_confirm_modal(ui, app, screen, &mut commands);
            }

        });
    commands
}

fn draw_full_loading_screen(ui: &mut egui::Ui, app: &App, screen: Rect) {
    let time = ui.ctx().input(|i| i.time);
    let dots = ".".repeat(1 + (time as usize) % 3);
    let bar_width = 280.0;
    let bar_height = 10.0;

    let logo_rect = Rect::from_center_size(screen.center() - egui::vec2(0.0, 72.0), Vec2::splat(56.0));
    draw_builtin_icon(
        ui,
        logo_rect,
        "builtin:icon_playstation",
        include_bytes!("../assets/icons/icon-playstation.png"),
    );

    let preload = app.preload_progress();
    let text = match preload {
        Some((done, total)) => format!("Downloading artwork  {done} / {total}"),
        None => format!("Scanning and indexing games from memory{dots}"),
    };
    let progress_frac = match preload {
        Some((done, total)) => done as f32 / total.max(1) as f32,
        None => (time as f32 * 2.0).sin() * 0.5 + 0.5,
    };

    let font = FontId::proportional(20.0);
    let galley = ui.painter().layout_no_wrap(text, font, Color32::WHITE);
    let pos = screen.center() - egui::vec2(galley.rect.size().x / 2.0, 25.0);
    ui.painter().galley(pos, galley, Color32::WHITE);

    let bar_rect = Rect::from_center_size(screen.center() + egui::vec2(0.0, 15.0), egui::vec2(bar_width, bar_height));
    ui.painter().rect_filled(bar_rect, CornerRadius::same(5), Color32::from_rgb(40, 44, 52));

    let fill_frac = if preload.is_some() { progress_frac } else { 0.15 + progress_frac * 0.75 };
    let fill_rect = Rect::from_min_size(bar_rect.min, egui::vec2(bar_width * fill_frac.clamp(0.0, 1.0), bar_height));
    ui.painter().rect_filled(fill_rect, CornerRadius::same(5), Color32::from_rgb(80, 160, 240));

    let subtext = "Please wait while the data loads...";
    let subfont = FontId::proportional(13.0);
    let subgalley = ui.painter().layout_no_wrap(subtext.to_string(), subfont, COL_TEXT_DIM);
    let subpos = screen.center() - egui::vec2(subgalley.rect.size().x / 2.0, -35.0);
    ui.painter().galley(subpos, subgalley, COL_TEXT_DIM);
}

fn texture_key(game: &Game, kind: &str) -> String {
    format!("{}:{}:{kind}", game.system.label(), game.title_id)
}

fn draw_game_grid(ui: &mut egui::Ui, app: &App, screen: Rect, commands: &mut Vec<AppCommand>) {
    let (card_w, card_h) = card_dimensions(screen.width());
    let grid_top = screen.top() + HEADER_H + 6.0;
    let grid_bottom = screen.bottom() - FOOTER_H - 4.0;

    let clip_rect = Rect::from_min_max(
        Pos2::new(screen.left(), grid_top - 6.0),
        Pos2::new(screen.right(), grid_bottom + 6.0),
    );

    let painter = ui.painter().with_clip_rect(clip_rect);
    let games_pool = if app.is_store_tab() {
        &app.store_games
    } else {
        &app.games
    };

    let stride = card_h + ROW_GAP;
    let first_row = (app.current_scroll / stride).floor().max(0.0) as usize;
    let view_h = (grid_bottom - grid_top).max(1.0);
    let last_row = ((app.current_scroll + view_h) / stride).ceil() as usize;
    let first_slot = first_row.saturating_mul(GRID_COLS);
    let last_slot = last_row
        .saturating_add(1)
        .saturating_mul(GRID_COLS)
        .min(app.visible.len());

    for slot in first_slot..last_slot {
        let Some(&game_index) = app.visible.get(slot) else { continue };
        let Some(game) = games_pool.get(game_index) else { continue };

        let col = slot % GRID_COLS;
        let row = slot / GRID_COLS;

        let tile_x = screen.left() + MARGIN_X + col as f32 * (card_w + COL_GAP);
        let tile_y = grid_top + row as f32 * (card_h + ROW_GAP) - app.current_scroll;

        if tile_y + card_h < grid_top - 20.0 || tile_y > grid_bottom + 20.0 {
            continue;
        }

        let base_rect = Rect::from_min_size(Pos2::new(tile_x, tile_y), Vec2::new(card_w, card_h));
        draw_game_card_with_painter(ui, &painter, app, game, slot, base_rect, commands);
    }

    if let Some((_title_id, notice)) = &app.launch_notice {
        let notice_rect = Rect::from_center_size(
            Pos2::new(screen.center().x, screen.bottom() - FOOTER_H - 24.0),
            Vec2::new((notice.len() as f32 * 6.5 + 32.0).min(screen.width() - 40.0), 28.0),
        );
        ui.painter().rect_filled(notice_rect, CornerRadius::same(14), Color32::from_rgba_premultiplied(20, 24, 34, 245));
        ui.painter().rect_stroke(notice_rect, CornerRadius::same(14), Stroke::new(1.0_f32, Color32::from_rgb(56, 189, 248)), egui::StrokeKind::Inside);
        ui.painter().text(
            notice_rect.center(),
            egui::Align2::CENTER_CENTER,
            notice,
            FontId::proportional(11.5),
            Color32::from_rgb(240, 243, 248),
        );
    }
}

fn draw_game_card_with_painter(
    ui: &mut egui::Ui,
    painter: &egui::Painter,
    app: &App,
    game: &Game,
    slot: usize,
    base_rect: Rect,
    commands: &mut Vec<AppCommand>,
) {
    let selected = slot == app.selected;
    let card_rect = if selected {
        base_rect.expand2(Vec2::new(base_rect.width() * 0.035, base_rect.height() * 0.035))
    } else {
        base_rect
    };
    let rounding = CornerRadius::same(6);

    let mut cover_drawn = false;
    if let Some(bytes) = game.cover_bytes.as_ref() {
        if let Some(handle) = app.texture_cache_get(ui.ctx(), &texture_key(game, "cover"), bytes) {
            draw_texture_cover_with_painter(painter, &handle, card_rect);
            cover_drawn = true;
        }
    }
    if !cover_drawn {
        if app.art_is_pending(&game.title_id) && game.cover_bytes.is_none() {
            draw_card_placeholder_with_painter(painter, game, card_rect, rounding);
        } else {
            draw_no_cover_with_painter(ui, app, painter, card_rect, rounding);
        }
    }

    if selected {
        painter.rect_stroke(
            card_rect,
            rounding,
            Stroke::new(2.5_f32, COL_CARD_SELECTED_BORDER),
            egui::StrokeKind::Outside,
        );
    } else {
        painter.rect_stroke(
            card_rect,
            rounding,
            Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(255, 255, 255, 18)),
            egui::StrokeKind::Inside,
        );
    }

    let is_installed = if app.is_store_tab() {
        app.games.iter().any(|g| g.title_id == game.title_id && (g.has_bubble || g.system == crate::scanner::System::Vita))
    } else {
        game.has_bubble || game.system == crate::scanner::System::Vita
    };
    if is_installed {
        draw_installed_badge_with_painter(painter, card_rect);
    }

    let is_favorite = app.collections.items.iter().any(|c| {
        (c.name.eq_ignore_ascii_case("Favorites") || c.name.eq_ignore_ascii_case("Favoritos"))
            && c.title_ids.iter().any(|id| id == &game.title_id)
    });
    if is_favorite {
        draw_favorite_badge_with_painter(painter, card_rect);
    }

    let response = ui.interact(card_rect, ui.id().with(("tile", slot)), Sense::click());
    if response.clicked() {
        commands.push(AppCommand::SelectVisibleSlot(slot));
    }
}

fn draw_installed_badge_with_painter(painter: &egui::Painter, card_rect: Rect) {
    let center = Pos2::new(card_rect.right() - 11.0, card_rect.bottom() - 11.0);
    let radius = 7.5;
    painter.circle_filled(center, radius + 1.2, Color32::from_rgb(14, 18, 26));
    painter.circle_filled(center, radius, COL_BADGE_INSTALLED);
    let stroke = Stroke::new(1.6_f32, Color32::WHITE);
    let p1 = center + Vec2::new(-3.5, 0.2);
    let p2 = center + Vec2::new(-1.0, 2.8);
    let p3 = center + Vec2::new(3.5, -2.4);
    painter.line_segment([p1, p2], stroke);
    painter.line_segment([p2, p3], stroke);
}

fn draw_favorite_badge_with_painter(painter: &egui::Painter, card_rect: Rect) {
    let center = Pos2::new(card_rect.right() - 11.0, card_rect.top() + 11.0);
    let radius = 7.5;
    painter.circle_filled(center, radius + 1.2, Color32::from_rgb(14, 18, 26));
    painter.circle_filled(center, radius, COL_BADGE_FAV);
    painter.text(
        center,
        egui::Align2::CENTER_CENTER,
        "★",
        FontId::proportional(9.0),
        Color32::WHITE,
    );
}

fn draw_card_placeholder_with_painter(
    painter: &egui::Painter,
    game: &Game,
    rect: Rect,
    rounding: CornerRadius,
) {
    painter.rect_filled(rect, rounding, COL_CARD_BG);
    painter.rect_stroke(rect, rounding, Stroke::new(1.0_f32, COL_CARD_BORDER), egui::StrokeKind::Inside);

    let badge_h = 15.0;
    let badge_rect = Rect::from_min_size(rect.min + Vec2::new(6.0, 6.0), Vec2::new(rect.width() - 12.0, badge_h));
    let badge_col = match game.system {
        crate::scanner::System::Vita => Color32::from_rgb(14, 116, 144),
        crate::scanner::System::Psp => Color32::from_rgb(3, 105, 161),
        crate::scanner::System::Psx => Color32::from_rgb(109, 40, 217),
    };
    painter.rect_filled(badge_rect, CornerRadius::same(3), badge_col);
    painter.text(
        badge_rect.center(),
        egui::Align2::CENTER_CENTER,
        game.system.label(),
        FontId::proportional(9.0),
        Color32::WHITE,
    );

    let text_pos = rect.min + Vec2::new(6.0, 26.0);
    painter.text(
        text_pos,
        egui::Align2::LEFT_TOP,
        short_title(&game.title),
        FontId::proportional(11.0),
        Color32::from_rgb(220, 226, 236),
    );
}

fn short_title(title: &str) -> &str {
    const MAX_CHARS: usize = 36;
    match title.char_indices().nth(MAX_CHARS) {
        Some((idx, _)) => &title[..idx],
        None => title,
    }
}

fn draw_header(ui: &mut egui::Ui, app: &App, screen: Rect, commands: &mut Vec<AppCommand>) {
    let header_rect = Rect::from_min_size(screen.min, Vec2::new(screen.width(), HEADER_H));
    filled_rect(ui, header_rect, COL_HEADER_BG);

    let row1_y = header_rect.top() + 14.0;

    if app.is_store_tab() {
        let search_w = 210.0;
        let search_h = 20.0;
        let search_rect = Rect::from_min_size(
            Pos2::new(header_rect.left() + MARGIN_X, row1_y - search_h / 2.0),
            Vec2::new(search_w, search_h),
        );
        ui.painter().rect_filled(search_rect, CornerRadius::same(10), COL_SEARCH_BG);
        ui.painter().rect_stroke(
            search_rect,
            CornerRadius::same(10),
            Stroke::new(1.0_f32, if app.search_active { Color32::from_rgb(56, 189, 248) } else { COL_SEARCH_BORDER }),
            egui::StrokeKind::Inside,
        );

        let search_label = if app.search_query.is_empty() {
            "Search the store..."
        } else {
            &app.search_query
        };
        let search_text_color = if app.search_query.is_empty() { COL_TEXT_MUTED } else { COL_TEXT };
        label_mid(ui, search_rect.left() + 12.0, row1_y, search_label, 10.5, search_text_color);

        if ui.interact(search_rect, ui.id().with("search_box_click"), Sense::click()).clicked() {
            commands.push(AppCommand::ToggleSearch);
        }
    }

    let right_pad = screen.right() - MARGIN_X;

    let ps_rect = Rect::from_center_size(Pos2::new(right_pad - 10.0, row1_y), Vec2::splat(16.0));
    draw_builtin_icon(
        ui,
        ps_rect,
        "builtin:icon_playstation",
        include_bytes!("../assets/icons/icon-playstation.png"),
    );

    let clock_text = app.clock_line();
    let clock_w = ui
        .fonts(|f| f.layout_no_wrap(clock_text.clone(), FontId::proportional(11.0), COL_TEXT))
        .size()
        .x;
    let clock_right = ps_rect.left() - 10.0;
    label_mid(ui, clock_right - clock_w, row1_y, &clock_text, 11.0, COL_TEXT);

    let battery_pos = Pos2::new(clock_right - clock_w - 28.0, row1_y - 4.5);
    let battery_pct = app.battery_pct();
    draw_battery_icon(ui, battery_pos, battery_pct as f32);

    let wifi_pos = Pos2::new(battery_pos.x - 14.0, row1_y);
    draw_wifi_icon(ui, wifi_pos, app.wifi_connected());

    let row2_y = header_rect.top() + 41.0;

    let l1_rect = Rect::from_min_size(
        Pos2::new(header_rect.left() + MARGIN_X, row2_y - 11.0),
        Vec2::new(24.0, 22.0),
    );
    draw_shoulder_glyph(ui, l1_rect, "L1");
    if ui.interact(l1_rect, ui.id().with("l1_click"), Sense::click()).clicked() {
        commands.push(AppCommand::TabPrev);
    }

    let r1_rect = Rect::from_min_size(
        Pos2::new(header_rect.right() - MARGIN_X - 24.0, row2_y - 11.0),
        Vec2::new(24.0, 22.0),
    );
    draw_shoulder_glyph(ui, r1_rect, "R1");
    if ui.interact(r1_rect, ui.id().with("r1_click"), Sense::click()).clicked() {
        commands.push(AppCommand::TabNext);
    }

    let gear_rect = Rect::from_center_size(Pos2::new(header_rect.right() - MARGIN_X - 44.0, row2_y), Vec2::splat(18.0));
    let gear_bg = if app.is_settings() {
        Color32::from_rgb(56, 189, 248)
    } else {
        Color32::from_rgba_unmultiplied(255, 255, 255, 20)
    };
    ui.painter().rect_filled(gear_rect.expand(3.0), CornerRadius::same(5), gear_bg);
    draw_builtin_icon(
        ui,
        gear_rect,
        "builtin:icon_settings",
        include_bytes!("../assets/icons/icon-settings.png"),
    );
    if ui.interact(gear_rect.expand(3.0), ui.id().with("settings_click"), Sense::click()).clicked() {
        commands.push(AppCommand::OpenSettings);
    }

    let left_bound = l1_rect.right() + 14.0;
    let right_bound = gear_rect.left() - 14.0;
    let available_w = (right_bound - left_bound).max(10.0);

    let mut tab_measures = Vec::new();
    let mut total_pills_w = 0.0;

    for (i, tab_name) in app.tabs.iter().enumerate() {
        let count = app.tab_count(i);
        let display_name = match tab_name.as_str() {
            "RECENTLY PLAYED" => "RECENT",
            other => other,
        };

        let name_w = ui
            .fonts(|f| f.layout_no_wrap(display_name.to_string(), FontId::proportional(11.0), COL_TEXT))
            .size()
            .x;
        let count_str = format!("{count}");
        let count_w = ui
            .fonts(|f| f.layout_no_wrap(count_str.clone(), FontId::proportional(11.0), COL_TEXT))
            .size()
            .x;

        let pill_w = name_w + count_w + 22.0;
        total_pills_w += pill_w;
        tab_measures.push((display_name, count_str, name_w, count_w, pill_w));
    }

    let num_tabs = tab_measures.len();
    let (mut tab_x, gap) = if num_tabs > 1 && available_w > total_pills_w {
        let max_gap = 36.0_f32;
        let computed_gap = ((available_w - total_pills_w) / (num_tabs - 1) as f32).min(max_gap);
        let used_w = total_pills_w + computed_gap * (num_tabs - 1) as f32;
        let start = left_bound + (available_w - used_w) / 2.0;
        (start, computed_gap)
    } else {
        (left_bound, 14.0_f32)
    };

    for (i, (display_name, count_str, name_w, _count_w, pill_w)) in tab_measures.into_iter().enumerate() {
        let is_active = !app.is_settings() && app.active_tab == i;
        let pill_rect = Rect::from_min_size(Pos2::new(tab_x, row2_y - 11.0), Vec2::new(pill_w, 22.0));

        if is_active {
            ui.painter().rect_filled(pill_rect, CornerRadius::same(11), COL_TAB_ACTIVE);
        }

        let name_color = if is_active { Color32::WHITE } else { COL_TEXT_DIM };
        let count_color = if is_active { Color32::from_rgb(180, 195, 215) } else { Color32::from_rgb(115, 125, 140) };

        label_mid(ui, tab_x + 9.0, row2_y, display_name, 11.0, name_color);
        label_mid(ui, tab_x + 9.0 + name_w + 4.0, row2_y, &count_str, 11.0, count_color);

        if ui.interact(pill_rect, ui.id().with(("tab_pill", i)), Sense::click()).clicked() {
            commands.push(AppCommand::SelectTab(i));
        }

        tab_x += pill_w + gap;
    }
}

fn draw_shoulder_glyph(ui: &mut egui::Ui, rect: Rect, label: &str) {
    let (name, bytes): (&str, &[u8]) = match label {
        "L1" => (
            "builtin:btn_l1",
            include_bytes!("../assets/buttons/outline-L1.png"),
        ),
        "R1" => (
            "builtin:btn_r1",
            include_bytes!("../assets/buttons/outline-R1.png"),
        ),
        _ => {
            rounded_panel(ui, rect, Color32::from_rgba_unmultiplied(255, 255, 255, 22));
            return;
        }
    };

    if !draw_builtin_icon(ui, rect, name, bytes) {
        rounded_panel(ui, rect, Color32::from_rgba_unmultiplied(255, 255, 255, 22));
        label_mid(ui, rect.center().x - 6.0, rect.center().y, label, 10.0, COL_TEXT_DIM);
    }
}

fn draw_wifi_icon(ui: &mut egui::Ui, center: Pos2, connected: bool) {
    let color = if connected { COL_TEXT } else { Color32::from_rgb(90, 96, 106) };
    let bar = COL_HEADER_BG;
    for i in (0..3).rev() {
        let r = 2.6 + i as f32 * 2.8;
        ui.painter().circle_filled(center, r, color);
        ui.painter().circle_filled(center, r - 1.3, bar);
    }
    let mask = Rect::from_min_size(Pos2::new(center.x - 10.0, center.y), Vec2::new(20.0, 10.0));
    filled_rect(ui, mask, bar);
    ui.painter().circle_filled(center, 1.4, color);
}

fn draw_battery_icon(ui: &mut egui::Ui, pos: Pos2, pct: f32) {
    let w = 20.0;
    let h = 10.0;
    let outline = Rect::from_min_size(pos, Vec2::new(w, h));
    ui.painter().rect_filled(outline, CornerRadius::same(2), COL_TEXT);
    ui.painter().rect_filled(
        Rect::from_min_size(Pos2::new(pos.x + w, pos.y + 2.5), Vec2::new(2.0, h - 5.0)),
        CornerRadius::same(1),
        COL_TEXT,
    );
    let inner = outline.shrink(1.8);
    filled_rect(ui, inner, COL_HEADER_BG);
    let fill_color = if pct <= 20.0 { Color32::from_rgb(220, 60, 60) } else { Color32::from_rgb(90, 220, 120) };
    let fill_w = (inner.width() * (pct / 100.0)).max(0.0);
    filled_rect(ui, Rect::from_min_size(inner.min, Vec2::new(fill_w, inner.height())), fill_color);
}

#[derive(Clone, Copy)]
enum Glyph {
    Cross,
    Circle,
    Triangle,
    Square,
    Select,
    Start,
}

impl Glyph {
    fn color(self) -> Color32 {
        match self {
            Glyph::Cross => Color32::from_rgb(0x38, 0xbd, 0xf8),
            Glyph::Circle => Color32::from_rgb(0xf8, 0x71, 0x71),
            Glyph::Triangle => Color32::from_rgb(0x34, 0xd3, 0x99),
            Glyph::Square => Color32::from_rgb(0xf4, 0x72, 0xb6),
            Glyph::Select => Color32::from_rgb(0x94, 0xa3, 0xb8),
            Glyph::Start => Color32::from_rgb(0x94, 0xa3, 0xb8),
        }
    }
}

fn draw_footer(ui: &mut egui::Ui, app: &App) {
    let screen = ui.max_rect();
    let bottom_bar_y = screen.bottom() - FOOTER_H;
    let rect = Rect::from_min_size(Pos2::new(screen.left(), bottom_bar_y), Vec2::new(screen.width(), FOOTER_H));
    filled_rect(ui, rect, COL_FOOTER_BG);

    let mid_y = rect.center().y;

    let menu_rect = Rect::from_center_size(
        Pos2::new(rect.left() + MARGIN_X + 32.0, mid_y),
        Vec2::new(64.0, 20.0),
    );
    rounded_panel(ui, menu_rect, Color32::from_rgba_unmultiplied(255, 255, 255, 24));
    let menu_text = "VITA";
    label_mid(ui, menu_rect.left() + 10.0, mid_y, menu_text, 10.5, COL_TEXT);
    label_mid(ui, menu_rect.right() + 8.0, mid_y, "MENU", 11.0, COL_TEXT_DIM);

    let confirm_label = if app.mode == Mode::StoreDetail {
        if let Some(detail) = &app.store_detail {
            if app.is_title_installed(&detail.title_id) {
                "LAUNCH"
            } else {
                "INSTALL"
            }
        } else {
            "INSTALL"
        }
    } else if app.is_store_tab() {
        "DETAILS"
    } else if selected_game(app).is_some_and(|g| g.system == crate::scanner::System::Vita) {
        "LAUNCH"
    } else {
        "SELECT"
    };

    let search_label = if app.search_active && !app.search_query.is_empty() {
        "CLEAR SEARCH"
    } else {
        "SEARCH"
    };

    let hints: Vec<(Glyph, &str)> = if app.is_settings() {
        let action = match app.settings_selected {
            0 => "RESCAN",
            1 => "TOGGLE",
            2 => "CHANGE",
            3 => "CLEAN",
            4 => "PURGE MUSIC",
            _ => "PURGE ALL",
        };
        vec![
            (Glyph::Cross, action),
            (Glyph::Circle, "BACK"),
            (Glyph::Start, "CLOSE"),
        ]
    } else if app.mode == Mode::CollectionPicker {
        vec![(Glyph::Cross, "TOGGLE"), (Glyph::Circle, "BACK")]
    } else if app.mode == Mode::StoreDetail {
        let mut hints = Vec::new();
        if app.store_detail.as_ref().is_some_and(|d| d.lightbox.is_some()) {
            hints.push((Glyph::Triangle, "CLOSE"));
        } else if app.store_detail.as_ref().is_some_and(|d| !d.screenshot_urls.is_empty()) {
            hints.push((Glyph::Triangle, "VIEW"));
        }
        hints.push((Glyph::Cross, confirm_label));
        hints.push((Glyph::Circle, "BACK"));
        hints
    } else {
        let mut hints = Vec::new();
        if app.tab_is_collection() {
            hints.push((Glyph::Square, "REMOVE"));
        }
        if !app.visible.is_empty() {
            hints.push((Glyph::Triangle, "COLLECTIONS"));
        }
        if app.is_store_tab() {
            hints.push((Glyph::Select, search_label));
        }
        if !app.visible.is_empty() {
            hints.push((Glyph::Cross, confirm_label));
        }
        hints.push((Glyph::Start, "SETTINGS"));
        hints
    };

    let mut right_x = screen.right() - MARGIN_X;
    for (glyph, label) in hints.iter().rev() {
        let label_w = ui
            .fonts(|f| f.layout_no_wrap(label.to_string(), FontId::proportional(11.5), COL_TEXT_DIM))
            .size()
            .x;
        let icon_w = match glyph {
            Glyph::Select => 26.0,
            _ => 16.0,
        };

        let total_item_w = icon_w + 4.0 + label_w;
        let item_left = right_x - total_item_w;

        draw_button_glyph(ui, Pos2::new(item_left + icon_w / 2.0, mid_y), *glyph);
        label_mid(ui, item_left + icon_w + 4.0, mid_y, label, 11.5, COL_TEXT_DIM);

        right_x = item_left - 14.0;
    }
}

fn draw_button_glyph(ui: &mut egui::Ui, center: Pos2, glyph: Glyph) {
    let size = match glyph {
        Glyph::Select => Vec2::new(26.0, 14.0),
        _ => Vec2::splat(16.0),
    };
    let rect = Rect::from_center_size(center, size);

    let maybe_asset = match glyph {
        Glyph::Cross => Some((
            "builtin:btn_cross",
            include_bytes!("../assets/buttons/outline-blue-cross.png").as_slice(),
        )),
        Glyph::Circle => Some((
            "builtin:btn_circle",
            include_bytes!("../assets/buttons/outline-red-circle.png").as_slice(),
        )),
        Glyph::Triangle => Some((
            "builtin:btn_triangle",
            include_bytes!("../assets/buttons/outline-green-triangle.png").as_slice(),
        )),
        Glyph::Square => Some((
            "builtin:btn_square",
            include_bytes!("../assets/buttons/outline-purple-square.png").as_slice(),
        )),
        Glyph::Select => Some((
            "builtin:btn_select",
            include_bytes!("../assets/buttons/outline-select.png").as_slice(),
        )),
        Glyph::Start => None,
    };

    if let Some((name, bytes)) = maybe_asset {
        if draw_builtin_icon(ui, rect, name, bytes) {
            return;
        }
    }

    draw_fallback_glyph(ui, center, glyph);
}

fn draw_builtin_icon(ui: &mut egui::Ui, rect: Rect, name: &str, bytes: &[u8]) -> bool {
    let handle = ui.ctx().data_mut(|d| d.get_temp::<egui::TextureHandle>(egui::Id::new(name)));
    let handle = match handle {
        Some(h) => h,
        None => {
            let Ok(img) = image::load_from_memory(bytes) else {
                return false;
            };
            let mut rgba = img.to_rgba8();

            for pix in rgba.pixels_mut() {
                let [r, g, b, a] = pix.0;
                if a > 0 && r < 18 && g < 18 && b < 18 {
                    pix.0 = [0, 0, 0, 0];
                }
            }
            let color_img = egui::ColorImage::from_rgba_unmultiplied(
                [rgba.width() as usize, rgba.height() as usize],
                rgba.as_raw(),
            );
            let h = ui.ctx().load_texture(name, color_img, egui::TextureOptions::LINEAR);
            ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new(name), h.clone()));
            h
        }
    };
    ui.painter().image(
        handle.id(),
        rect,
        Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
        Color32::WHITE,
    );
    true
}

fn preload_button_icons(ui: &mut egui::Ui) {
    const ICONS: &[(&str, &[u8])] = &[
        ("builtin:btn_cross", include_bytes!("../assets/buttons/outline-blue-cross.png")),
        ("builtin:btn_circle", include_bytes!("../assets/buttons/outline-red-circle.png")),
        ("builtin:btn_triangle", include_bytes!("../assets/buttons/outline-green-triangle.png")),
        ("builtin:btn_square", include_bytes!("../assets/buttons/outline-purple-square.png")),
        ("builtin:btn_select", include_bytes!("../assets/buttons/outline-select.png")),
        ("builtin:btn_l1", include_bytes!("../assets/buttons/outline-L1.png")),
        ("builtin:btn_r1", include_bytes!("../assets/buttons/outline-R1.png")),
    ];
    for &(name, bytes) in ICONS {
        let already = ui
            .ctx()
            .data_mut(|d| d.get_temp::<egui::TextureHandle>(egui::Id::new(name)).is_some());
        if already {
            continue;
        }
        let Ok(img) = image::load_from_memory(bytes) else {
            continue;
        };
        let mut rgba = img.to_rgba8();
        for pix in rgba.pixels_mut() {
            let [r, g, b, a] = pix.0;
            if a > 0 && r < 18 && g < 18 && b < 18 {
                pix.0 = [0, 0, 0, 0];
            }
        }
        let color_img = egui::ColorImage::from_rgba_unmultiplied(
            [rgba.width() as usize, rgba.height() as usize],
            rgba.as_raw(),
        );
        let h = ui.ctx().load_texture(name, color_img, egui::TextureOptions::LINEAR);
        ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new(name), h));
    }
}

fn draw_fallback_glyph(ui: &mut egui::Ui, center: Pos2, glyph: Glyph) {
    let radius = 7.0;
    let stroke = Stroke::new(1.8_f32, glyph.color());
    let painter = ui.painter();
    match glyph {
        Glyph::Cross => {
            let arm = radius * 0.62;
            painter.line_segment([center + Vec2::new(-arm, -arm), center + Vec2::new(arm, arm)], stroke);
            painter.line_segment([center + Vec2::new(arm, -arm), center + Vec2::new(-arm, arm)], stroke);
        }
        Glyph::Circle => {
            painter.circle_stroke(center, radius * 0.72, stroke);
        }
        Glyph::Triangle => {
            let r = radius * 0.8;
            let p0 = center + Vec2::new(0.0, -r);
            let p1 = center + Vec2::new(r * 0.87, r * 0.5);
            let p2 = center + Vec2::new(-r * 0.87, r * 0.5);
            painter.line_segment([p0, p1], stroke);
            painter.line_segment([p1, p2], stroke);
            painter.line_segment([p2, p0], stroke);
        }
        Glyph::Square => {
            let s = radius * 0.62;
            painter.rect_stroke(
                Rect::from_center_size(center, Vec2::splat(s * 2.0)),
                CornerRadius::ZERO,
                stroke,
                egui::StrokeKind::Outside,
            );
        }
        Glyph::Select => {
            let rect = Rect::from_center_size(center, Vec2::new(13.0, 6.5));
            painter.rect_stroke(
                rect,
                CornerRadius::same(2),
                stroke,
                egui::StrokeKind::Outside,
            );
        }
        Glyph::Start => {
            let rect = Rect::from_center_size(center, Vec2::new(13.0, 6.5));
            painter.rect_stroke(rect, CornerRadius::same(2), stroke, egui::StrokeKind::Outside);
            painter.line_segment(
                [Pos2::new(center.x, rect.top() + 1.5), Pos2::new(center.x, rect.bottom() - 1.5)],
                stroke,
            );
        }
    }
}

fn draw_collection_picker(ui: &mut egui::Ui, app: &App, screen: Rect, commands: &mut Vec<AppCommand>) {
    let panel_w = 260.0_f32;
    let panel_h = (app.collections.items.len() as f32 * 36.0 + 50.0).min(screen.height() - 80.0);
    let panel_rect = Rect::from_center_size(screen.center(), Vec2::new(panel_w, panel_h));

    filled_rect(ui, screen, Color32::from_rgba_unmultiplied(0, 0, 0, 160));
    rounded_panel(ui, panel_rect, Color32::from_rgb(18, 20, 26));
    ui.painter().rect_stroke(panel_rect, CornerRadius::same(10), Stroke::new(1.0_f32, Color32::from_rgb(45, 52, 65)), egui::StrokeKind::Inside);

    label_at(ui, Pos2::new(panel_rect.left() + 20.0, panel_rect.top() + 14.0), "COLLECTIONS", 13.0, COL_TEXT_DIM);

    let Some(game) = selected_game(app) else { return };
    let row_start_y = panel_rect.top() + 42.0;

    for (i, collection) in app.collections.items.iter().enumerate() {
        let y = row_start_y + i as f32 * 36.0;
        if y + 32.0 > panel_rect.bottom() - 8.0 {
            break;
        }
        let row_rect = Rect::from_min_size(Pos2::new(panel_rect.left() + 10.0, y), Vec2::new(panel_w - 20.0, 32.0));
        let selected = i == app.picker_index;
        if selected {
            rounded_panel(ui, row_rect, Color32::from_rgba_unmultiplied(56, 189, 248, 40));
            ui.painter().rect_stroke(row_rect, CornerRadius::same(6), Stroke::new(1.5_f32, Color32::from_rgb(56, 189, 248)), egui::StrokeKind::Inside);
        }

        let is_member = app.collections.contains(i, &game.title_id);
        let check_box = Rect::from_min_size(Pos2::new(row_rect.left() + 10.0, row_rect.top() + 8.0), Vec2::new(16.0, 16.0));
        ui.painter().rect_stroke(check_box, CornerRadius::same(3), Stroke::new(1.5_f32, if is_member { Color32::from_rgb(56, 189, 248) } else { COL_TEXT_DIM }), egui::StrokeKind::Inside);
        if is_member {
            ui.painter().rect_filled(check_box.shrink(3.0), CornerRadius::same(1), Color32::from_rgb(56, 189, 248));
        }

        let is_favorites = collection.name.eq_ignore_ascii_case("Favorites")
            || collection.name.eq_ignore_ascii_case("Favoritos");
        let name_x = if is_favorites {
            let icon_rect = Rect::from_center_size(
                Pos2::new(row_rect.left() + 44.0, row_rect.center().y),
                Vec2::splat(14.0),
            );
            draw_builtin_icon(
                ui,
                icon_rect,
                "builtin:icon_favorites",
                include_bytes!("../assets/icons/icon-favorites.png"),
            );
            row_rect.left() + 56.0
        } else {
            row_rect.left() + 36.0
        };
        label_at(ui, Pos2::new(name_x, row_rect.top() + 8.0), &collection.name, 13.0, if selected { COL_TEXT } else { COL_TEXT_DIM });

        let response = ui.interact(row_rect, ui.id().with(("collection_row", i)), Sense::click());
        if response.clicked() {
            commands.push(AppCommand::TogglePickerRow(i));
        }
    }
}

fn draw_download_confirm_modal(
    ui: &mut egui::Ui,
    app: &App,
    screen: Rect,
    commands: &mut Vec<AppCommand>,
) {
    let Some(confirm) = &app.download_confirm else { return };
    let panel_w = 400.0_f32;
    let panel_h = 200.0_f32;
    let panel_rect = Rect::from_center_size(screen.center(), Vec2::new(panel_w, panel_h));

    filled_rect(ui, screen, Color32::from_rgba_unmultiplied(0, 0, 0, 190));

    rounded_panel(ui, panel_rect, Color32::from_rgb(18, 22, 30));
    ui.painter().rect_stroke(
        panel_rect,
        CornerRadius::same(12),
        Stroke::new(1.5_f32, Color32::from_rgb(56, 189, 248)),
        egui::StrokeKind::Inside,
    );

    let center_x = panel_rect.center().x;

    let header_y = panel_rect.top() + 22.0;
    label_center(ui, center_x, header_y, "CONFIRM DOWNLOAD", 13.5, Color32::from_rgb(56, 189, 248));

    let prompt_y = header_y + 26.0;
    label_center(ui, center_x, prompt_y, "Download this game to your console?", 11.5, COL_TEXT_DIM);

    let name_y = prompt_y + 30.0;
    let title_display = if confirm.title.chars().count() > 36 {
        let truncated: String = confirm.title.chars().take(33).collect();
        format!("{}...", truncated)
    } else {
        confirm.title.clone()
    };
    label_center(ui, center_x, name_y, &title_display, 14.0, COL_TEXT);

    let id_y = name_y + 20.0;
    label_center(ui, center_x, id_y, &format!("[{}]", confirm.title_id), 11.0, COL_TEXT_MUTED);

    let btn_w = 150.0_f32;
    let btn_h = 34.0_f32;
    let btn_gap = 16.0_f32;
    let btn_y = panel_rect.bottom() - btn_h / 2.0 - 18.0;
    let btn_offset = (btn_w + btn_gap) / 2.0;

    let left_btn_rect = Rect::from_center_size(
        Pos2::new(center_x - btn_offset, btn_y),
        Vec2::new(btn_w, btn_h),
    );
    let right_btn_rect = Rect::from_center_size(
        Pos2::new(center_x + btn_offset, btn_y),
        Vec2::new(btn_w, btn_h),
    );

    let sel_yes = confirm.selected_choice == 0;
    let sel_no = confirm.selected_choice == 1;

    let yes_bg = if sel_yes {
        Color32::from_rgb(16, 140, 70)
    } else {
        Color32::from_rgba_unmultiplied(255, 255, 255, 14)
    };
    rounded_panel(ui, left_btn_rect, yes_bg);
    if sel_yes {
        ui.painter().rect_stroke(
            left_btn_rect,
            CornerRadius::same(6),
            Stroke::new(2.0_f32, Color32::from_rgb(74, 222, 128)),
            egui::StrokeKind::Inside,
        );
    }
    label_center(
        ui,
        left_btn_rect.center().x,
        left_btn_rect.center().y,
        "YES, DOWNLOAD",
        12.0,
        if sel_yes { Color32::WHITE } else { COL_TEXT_DIM },
    );
    if ui
        .interact(left_btn_rect, ui.id().with("btn_download_yes"), Sense::click())
        .clicked()
    {
        commands.push(AppCommand::ConfirmDownload(true));
    }

    let no_bg = if sel_no {
        Color32::from_rgb(180, 40, 40)
    } else {
        Color32::from_rgba_unmultiplied(255, 255, 255, 14)
    };
    rounded_panel(ui, right_btn_rect, no_bg);
    if sel_no {
        ui.painter().rect_stroke(
            right_btn_rect,
            CornerRadius::same(6),
            Stroke::new(2.0_f32, Color32::from_rgb(248, 113, 113)),
            egui::StrokeKind::Inside,
        );
    }
    label_center(
        ui,
        right_btn_rect.center().x,
        right_btn_rect.center().y,
        "CANCEL",
        12.0,
        if sel_no { Color32::WHITE } else { COL_TEXT_DIM },
    );
    if ui
        .interact(right_btn_rect, ui.id().with("btn_download_no"), Sense::click())
        .clicked()
    {
        commands.push(AppCommand::ConfirmDownload(false));
    }
}

fn draw_settings(ui: &mut egui::Ui, app: &App, screen: Rect, commands: &mut Vec<AppCommand>) {
    let left = screen.left() + MARGIN_X;
    let col_w = 420.0;
    let right_col = left + col_w + 20.0;
    let mut y_left = screen.top() + HEADER_H + 16.0;
    let mut y_right = screen.top() + HEADER_H + 16.0;

    let title_icon = Rect::from_center_size(Pos2::new(left + 12.0, y_left + 10.0), Vec2::splat(22.0));
    draw_builtin_icon(
        ui,
        title_icon,
        "builtin:icon_settings",
        include_bytes!("../assets/icons/icon-settings.png"),
    );
    label_at(ui, Pos2::new(left + 30.0, y_left), "General & Library", 20.0, COL_TEXT);
    y_left += 30.0;

    let vita_games = app.games.iter().filter(|g| g.system == crate::scanner::System::Vita).count();
    let psp_games = app.games.iter().filter(|g| g.system == crate::scanner::System::Psp).count();
    let psx_games = app.games.iter().filter(|g| g.system == crate::scanner::System::Psx).count();

    let free_mb = app.runtime.free_memory_bytes() / (1024 * 1024);
    let tex_used_kb = app.texture_bytes_in_use() / 1024;
    let tex_budget_kb = app.texture_budget_bytes() / 1024;

    let rows = [
        ("Version".to_string(), format!("VitaDeck {}", env!("CARGO_PKG_VERSION"))),
        ("Library".to_string(), format!("{} PS Vita  ·  {} PSP  ·  {} PS1", vita_games, psp_games, psx_games)),
        ("Collections".to_string(), format!("{}", app.collections.items.len())),
        ("Wi-Fi".to_string(), if app.wifi_connected() { "connected".to_string() } else { "offline".to_string() }),
        ("Free RAM".to_string(), format!("{} MB  ·  {}", free_mb, app.runtime.pressure().label())),
        ("Textures".to_string(), format!("{} KB / {} KB", tex_used_kb, tex_budget_kb)),
    ];

    for (label, value) in rows {
        label_at(ui, Pos2::new(left, y_left), &label, 12.0, COL_TEXT_DIM);
        label_at(ui, Pos2::new(left + 110.0, y_left), &value, 12.0, COL_TEXT);
        y_left += 20.0;
    }

    y_left += 10.0;

    let btn_rescan = Rect::from_min_size(Pos2::new(left, y_left), Vec2::new(col_w, 32.0));
    let sel_0 = app.settings_selected == 0;
    let bg_col_0 = if sel_0 { Color32::from_rgba_unmultiplied(56, 189, 248, 60) } else { Color32::from_rgba_unmultiplied(56, 189, 248, 20) };
    let border_col_0 = if sel_0 { Color32::from_rgb(56, 189, 248) } else { Color32::from_rgba_unmultiplied(56, 189, 248, 100) };
    rounded_panel(ui, btn_rescan, bg_col_0);
    ui.painter().rect_stroke(btn_rescan, CornerRadius::same(6), Stroke::new(if sel_0 { 2.0_f32 } else { 1.0_f32 }, border_col_0), egui::StrokeKind::Inside);
    label_at(ui, Pos2::new(btn_rescan.left() + 16.0, btn_rescan.center().y - 7.0), "⟳ RESCAN LIBRARY", 12.0, Color32::WHITE);
    if ui.interact(btn_rescan, ui.id().with("rescan_btn"), Sense::click()).clicked() {
        commands.push(AppCommand::Rescan);
    }
    y_left += 40.0;

    let card_bgm = Rect::from_min_size(Pos2::new(left, y_left), Vec2::new(col_w, 42.0));
    let sel_1 = app.settings_selected == 1;
    let bg_col_1 = if sel_1 { Color32::from_rgba_unmultiplied(255, 255, 255, 25) } else { Color32::from_rgba_unmultiplied(255, 255, 255, 10) };
    let border_col_1 = if sel_1 { Color32::from_rgb(56, 189, 248) } else { Color32::from_rgba_unmultiplied(255, 255, 255, 20) };
    rounded_panel(ui, card_bgm, bg_col_1);
    ui.painter().rect_stroke(card_bgm, CornerRadius::same(6), Stroke::new(if sel_1 { 2.0_f32 } else { 1.0_f32 }, border_col_1), egui::StrokeKind::Inside);
    label_at(ui, Pos2::new(card_bgm.left() + 12.0, card_bgm.top() + 6.0), "Download BGM (Music)", 12.0, COL_TEXT);
    label_at(ui, Pos2::new(card_bgm.left() + 12.0, card_bgm.top() + 23.0), "Disabled saves ~3 MB per game", 10.0, COL_TEXT_DIM);
    let bgm_status = if app.config.download_bgm { "[ ON ]" } else { "[ OFF ]" };
    let bgm_color = if app.config.download_bgm { Color32::from_rgb(74, 222, 128) } else { Color32::from_rgb(248, 113, 113) };
    label_at(ui, Pos2::new(card_bgm.right() - 60.0, card_bgm.top() + 12.0), bgm_status, 12.0, bgm_color);
    if ui.interact(card_bgm, ui.id().with("bgm_card"), Sense::click()).clicked() {
        commands.push(AppCommand::ToggleDownloadBgm);
    }
    y_left += 50.0;

    let card_budget = Rect::from_min_size(Pos2::new(left, y_left), Vec2::new(col_w, 42.0));
    let sel_2 = app.settings_selected == 2;
    let bg_col_2 = if sel_2 { Color32::from_rgba_unmultiplied(255, 255, 255, 25) } else { Color32::from_rgba_unmultiplied(255, 255, 255, 10) };
    let border_col_2 = if sel_2 { Color32::from_rgb(56, 189, 248) } else { Color32::from_rgba_unmultiplied(255, 255, 255, 20) };
    rounded_panel(ui, card_budget, bg_col_2);
    ui.painter().rect_stroke(card_budget, CornerRadius::same(6), Stroke::new(if sel_2 { 2.0_f32 } else { 1.0_f32 }, border_col_2), egui::StrokeKind::Inside);
    label_at(ui, Pos2::new(card_budget.left() + 12.0, card_budget.top() + 6.0), "Disk Cache Limit", 12.0, COL_TEXT);
    label_at(ui, Pos2::new(card_budget.left() + 12.0, card_budget.top() + 23.0), "Auto-evicts oldest music & hero art", 10.0, COL_TEXT_DIM);
    label_at(ui, Pos2::new(card_budget.right() - 85.0, card_budget.top() + 12.0), app.config.budget_label(), 12.0, Color32::from_rgb(56, 189, 248));
    if ui.interact(card_budget, ui.id().with("budget_card"), Sense::click()).clicked() {
        commands.push(AppCommand::CycleCacheBudget);
    }

    label_at(ui, Pos2::new(right_col, y_right), "Storage & Optimization", 20.0, COL_TEXT);
    y_right += 30.0;

    let stats = &app.cache_stats;
    let cache_rows = [
        ("Covers & Boxes".to_string(), crate::cache_manager::format_bytes(stats.covers_bytes)),
        ("Hero & Logos".to_string(), crate::cache_manager::format_bytes(stats.hero_bytes + stats.logo_bytes)),
        ("Background Music".to_string(), crate::cache_manager::format_bytes(stats.music_bytes)),
        ("Total Used".to_string(), crate::cache_manager::format_bytes(stats.total_bytes)),
        ("Orphaned Files".to_string(), format!("{} ({})", stats.orphan_count, crate::cache_manager::format_bytes(stats.orphan_bytes))),
    ];

    for (label, value) in cache_rows {
        label_at(ui, Pos2::new(right_col, y_right), &label, 12.0, COL_TEXT_DIM);
        label_at(ui, Pos2::new(right_col + 130.0, y_right), &value, 12.0, COL_TEXT);
        y_right += 20.0;
    }

    y_right += 10.0;

    let btn_clean = Rect::from_min_size(Pos2::new(right_col, y_right), Vec2::new(col_w, 32.0));
    let sel_3 = app.settings_selected == 3;
    let bg_col_3 = if sel_3 { Color32::from_rgba_unmultiplied(234, 179, 8, 40) } else { Color32::from_rgba_unmultiplied(234, 179, 8, 15) };
    let border_col_3 = if sel_3 { Color32::from_rgb(234, 179, 8) } else { Color32::from_rgba_unmultiplied(234, 179, 8, 80) };
    rounded_panel(ui, btn_clean, bg_col_3);
    ui.painter().rect_stroke(btn_clean, CornerRadius::same(6), Stroke::new(if sel_3 { 2.0_f32 } else { 1.0_f32 }, border_col_3), egui::StrokeKind::Inside);
    label_at(ui, Pos2::new(btn_clean.left() + 16.0, btn_clean.center().y - 7.0), "CLEAN ORPHANED CACHE", 12.0, Color32::WHITE);
    if ui.interact(btn_clean, ui.id().with("clean_orphans_btn"), Sense::click()).clicked() {
        commands.push(AppCommand::CleanOrphanCache);
    }
    y_right += 38.0;

    let btn_purge_music = Rect::from_min_size(Pos2::new(right_col, y_right), Vec2::new(col_w, 32.0));
    let sel_4 = app.settings_selected == 4;
    let bg_col_4 = if sel_4 { Color32::from_rgba_unmultiplied(244, 63, 94, 40) } else { Color32::from_rgba_unmultiplied(244, 63, 94, 15) };
    let border_col_4 = if sel_4 { Color32::from_rgb(244, 63, 94) } else { Color32::from_rgba_unmultiplied(244, 63, 94, 80) };
    rounded_panel(ui, btn_purge_music, bg_col_4);
    ui.painter().rect_stroke(btn_purge_music, CornerRadius::same(6), Stroke::new(if sel_4 { 2.0_f32 } else { 1.0_f32 }, border_col_4), egui::StrokeKind::Inside);
    label_at(ui, Pos2::new(btn_purge_music.left() + 16.0, btn_purge_music.center().y - 7.0), "PURGE MUSIC CACHE", 12.0, Color32::WHITE);
    if ui.interact(btn_purge_music, ui.id().with("purge_music_btn"), Sense::click()).clicked() {
        commands.push(AppCommand::PurgeMusicCache);
    }
    y_right += 38.0;

    let btn_purge_all = Rect::from_min_size(Pos2::new(right_col, y_right), Vec2::new(col_w, 32.0));
    let sel_5 = app.settings_selected == 5;
    let bg_col_5 = if sel_5 { Color32::from_rgba_unmultiplied(239, 68, 68, 40) } else { Color32::from_rgba_unmultiplied(239, 68, 68, 15) };
    let border_col_5 = if sel_5 { Color32::from_rgb(239, 68, 68) } else { Color32::from_rgba_unmultiplied(239, 68, 68, 80) };
    rounded_panel(ui, btn_purge_all, bg_col_5);
    ui.painter().rect_stroke(btn_purge_all, CornerRadius::same(6), Stroke::new(if sel_5 { 2.0_f32 } else { 1.0_f32 }, border_col_5), egui::StrokeKind::Inside);
    label_at(ui, Pos2::new(btn_purge_all.left() + 16.0, btn_purge_all.center().y - 7.0), "PURGE ALL CACHE", 12.0, Color32::WHITE);
    if ui.interact(btn_purge_all, ui.id().with("purge_all_btn"), Sense::click()).clicked() {
        commands.push(AppCommand::PurgeAllCache);
    }

    if let Some(notice) = &app.cache_notice {
        let toast_rect = Rect::from_min_size(Pos2::new(left, screen.bottom() - FOOTER_H - 30.0), Vec2::new(screen.width() - (MARGIN_X * 2.0), 24.0));
        rounded_panel(ui, toast_rect, Color32::from_rgba_unmultiplied(16, 185, 129, 40));
        ui.painter().rect_stroke(toast_rect, CornerRadius::same(4), Stroke::new(1.0_f32, Color32::from_rgb(16, 185, 129)), egui::StrokeKind::Inside);
        label_at(ui, Pos2::new(toast_rect.left() + 12.0, toast_rect.top() + 4.0), notice, 11.0, Color32::WHITE);
    }
}

fn draw_empty_state(ui: &mut egui::Ui, app: &App, screen: Rect) {
    label_at(ui, screen.min + Vec2::new(MARGIN_X, HEADER_H + 40.0), "No games found in this tab", 18.0, Color32::from_rgb(255, 210, 90));

    let mut line_y = HEADER_H + 70.0;
    for line in app.scan_log.iter().take(8) {
        label_at(ui, screen.min + Vec2::new(MARGIN_X, line_y), line, 12.0, Color32::from_rgb(200, 200, 200));
        line_y += 15.0;
    }
    label_at(
        ui,
        screen.min + Vec2::new(MARGIN_X, line_y + 6.0),
        "Full log at ux0:data/VitaDeck/scan_log.txt",
        11.0,
        Color32::from_rgb(140, 140, 140),
    );
}

fn no_cover_bytes() -> &'static ImageBytes {
    static BYTES: OnceLock<ImageBytes> = OnceLock::new();
    BYTES.get_or_init(|| (true, NO_COVER_PNG.to_vec()))
}

fn draw_no_cover_with_painter(
    ui: &egui::Ui,
    app: &App,
    painter: &egui::Painter,
    rect: Rect,
    rounding: CornerRadius,
) {
    painter.rect_filled(rect, rounding, Color32::from_rgb(214, 214, 214));
    if let Some(handle) = app.texture_cache_get(ui.ctx(), "builtin:no-cover", no_cover_bytes()) {
        draw_texture_contain_with_painter(painter, &handle, rect);
    }
    let font_size = (rect.width() * 0.2).clamp(11.0, 16.0);
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        "No Cover",
        FontId::proportional(font_size),
        Color32::from_rgb(24, 24, 24),
    );
}

fn draw_texture_contain_with_painter(painter: &egui::Painter, handle: &egui::TextureHandle, rect: Rect) {
    let size = handle.size_vec2();
    if size.x <= 0.0 || size.y <= 0.0 || rect.width() <= 0.0 || rect.height() <= 0.0 {
        return;
    }
    let scale = (rect.width() / size.x).min(rect.height() / size.y);
    let drawn = Vec2::new(size.x * scale, size.y * scale);
    let min = Pos2::new(rect.center().x - drawn.x / 2.0, rect.center().y - drawn.y / 2.0);
    painter.image(
        handle.id(),
        Rect::from_min_size(min, drawn),
        Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
        Color32::WHITE,
    );
}

fn draw_store_detail(ui: &mut egui::Ui, app: &App, screen: Rect) {
    let Some(detail) = &app.store_detail else { return };
    let Some(game) = app
        .store_games
        .iter()
        .find(|g| g.title_id == detail.title_id)
    else {
        return;
    };
    let item = app.store_item(&detail.title_id);

    let hero_h = screen.height() * 0.46;
    let hero_rect = Rect::from_min_size(screen.min, Vec2::new(screen.width(), hero_h));
    filled_rect(ui, hero_rect, COL_CARD_BG);

    let mut hero_drawn = false;
    if let Some(bytes) = &detail.hero_bytes {
        if let Some(handle) = app.texture_cache_get(ui.ctx(), &texture_key(game, "hero"), bytes) {
            draw_texture_cover_with_painter(ui.painter(), &handle, hero_rect);
            hero_drawn = true;
        }
    }
    let cover_bytes = game.cover_bytes.as_ref();
    if !hero_drawn {
        if let Some(bytes) = cover_bytes {
            if let Some(handle) = app.texture_cache_get(ui.ctx(), &texture_key(game, "cover"), bytes) {
                draw_texture_cover_with_painter(ui.painter(), &handle, hero_rect);
                hero_drawn = true;
            }
        }
    }
    if hero_drawn {
        filled_rect(ui, hero_rect, Color32::from_rgba_unmultiplied(0, 0, 0, 120));
    }

    let cover_h = hero_h * 0.72;
    let cover_w = cover_h / CARD_ASPECT;
    let cover_rect = Rect::from_min_size(
        Pos2::new(screen.left() + MARGIN_X, hero_rect.bottom() - cover_h * 0.55),
        Vec2::new(cover_w, cover_h),
    );
    ui.painter().rect_filled(cover_rect, CornerRadius::same(6), COL_CARD_BG);
    let mut cover_drawn = false;
    if let Some(bytes) = cover_bytes {
        if let Some(handle) = app.texture_cache_get(ui.ctx(), &texture_key(game, "cover"), bytes) {
            draw_texture_cover_with_painter(ui.painter(), &handle, cover_rect);
            cover_drawn = true;
        }
    }
    if !cover_drawn {
        draw_no_cover_with_painter(ui, app, ui.painter(), cover_rect, CornerRadius::same(6));
    }
    ui.painter().rect_stroke(
        cover_rect,
        CornerRadius::same(6),
        Stroke::new(1.5_f32, COL_CARD_SELECTED_BORDER),
        egui::StrokeKind::Outside,
    );

    let text_left = cover_rect.right() + 14.0;
    let mut text_y = cover_rect.top() + 4.0;
    ui.painter().text(
        Pos2::new(text_left, text_y),
        egui::Align2::LEFT_TOP,
        &game.title,
        FontId::proportional(18.0),
        COL_TEXT,
    );
    text_y += 24.0;

    let mut meta = Vec::new();
    meta.push(game.title_id.clone());
    if let Some(item) = item {
        if let Some(region) = item.region.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            meta.push(region.to_string());
        }
        if let Some(version) = item.version.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            meta.push(format!("v{version}"));
        }
        if let Some(size) = item.size_label() {
            meta.push(size);
        }
    }
    ui.painter().text(
        Pos2::new(text_left, text_y),
        egui::Align2::LEFT_TOP,
        meta.join("  ·  "),
        FontId::proportional(12.0),
        COL_TEXT_DIM,
    );
    text_y += 18.0;

    if let Some(author) = item.and_then(|i| i.author.as_deref().map(str::trim).filter(|s| !s.is_empty())) {
        ui.painter().text(
            Pos2::new(text_left, text_y),
            egui::Align2::LEFT_TOP,
            author,
            FontId::proportional(12.0),
            COL_TEXT_MUTED,
        );
        text_y += 16.0;
    }

    let cta_label = if app.is_title_installed(&game.title_id) {
        "LAUNCH"
    } else {
        "INSTALL"
    };
    let cta_rect = Rect::from_min_size(Pos2::new(text_left, text_y + 4.0), Vec2::new(110.0, 26.0));
    ui.painter().rect_filled(cta_rect, CornerRadius::same(6), Color32::from_rgb(56, 189, 248));
    ui.painter().text(
        cta_rect.center(),
        egui::Align2::CENTER_CENTER,
        cta_label,
        FontId::proportional(12.0),
        Color32::from_rgb(12, 18, 28),
    );

    let shot_top = cover_rect.bottom() + 12.0;
    let shot_h = 72.0;
    let shot_w = 128.0;
    if !detail.screenshot_urls.is_empty() {
        let mut x = screen.left() + MARGIN_X;
        for (index, maybe_bytes) in detail.screenshots.iter().enumerate() {
            if x + shot_w > screen.right() - MARGIN_X {
                break;
            }
            let shot_rect = Rect::from_min_size(Pos2::new(x, shot_top), Vec2::new(shot_w, shot_h));
            ui.painter().rect_filled(shot_rect, CornerRadius::same(5), COL_CARD_BG);
            if let Some(bytes) = maybe_bytes {
                let key = format!("{}:{}:shot{index}", game.system.label(), game.title_id);
                if let Some(handle) = app.texture_cache_get(ui.ctx(), &key, bytes) {
                    draw_texture_cover_with_painter(ui.painter(), &handle, shot_rect);
                }
            }
            let stroke = if detail.selected_shot == index {
                Stroke::new(2.0_f32, COL_CARD_SELECTED_BORDER)
            } else {
                Stroke::new(1.0_f32, COL_CARD_BORDER)
            };
            ui.painter().rect_stroke(shot_rect, CornerRadius::same(5), stroke, egui::StrokeKind::Inside);
            x += shot_w + 8.0;
        }
    }

    let desc_top = if detail.screenshot_urls.is_empty() {
        shot_top
    } else {
        shot_top + shot_h + 10.0
    };
    if let Some(desc) = item.and_then(|i| i.description_text()) {
        let desc_rect = Rect::from_min_max(
            Pos2::new(screen.left() + MARGIN_X, desc_top),
            Pos2::new(screen.right() - MARGIN_X, screen.bottom() - FOOTER_H - 6.0),
        );
        let font = FontId::proportional(12.0);
        let galley = ui.fonts(|f| f.layout(desc.to_string(), font, COL_TEXT_DIM, desc_rect.width()));
        ui.painter().with_clip_rect(desc_rect).galley(desc_rect.min, galley, COL_TEXT_DIM);
    }

    if let Some((_tid, notice)) = &app.launch_notice {
        let notice_rect = Rect::from_center_size(
            Pos2::new(screen.center().x, screen.bottom() - FOOTER_H - 24.0),
            Vec2::new((notice.len() as f32 * 6.5 + 32.0).min(screen.width() - 40.0), 28.0),
        );
        ui.painter().rect_filled(
            notice_rect,
            CornerRadius::same(14),
            Color32::from_rgba_premultiplied(20, 24, 34, 245),
        );
        ui.painter().rect_stroke(
            notice_rect,
            CornerRadius::same(14),
            Stroke::new(1.0_f32, Color32::from_rgb(56, 189, 248)),
            egui::StrokeKind::Inside,
        );
        ui.painter().text(
            notice_rect.center(),
            egui::Align2::CENTER_CENTER,
            notice,
            FontId::proportional(11.5),
            COL_TEXT,
        );
    }

    if let Some(index) = detail.lightbox {
        ui.painter().rect_filled(screen, CornerRadius::ZERO, Color32::from_rgba_unmultiplied(0, 0, 0, 200));
        let box_w = 720.0 / 1.3;
        let box_h = 405.0 / 1.3;
        let box_rect = Rect::from_center_size(screen.center(), Vec2::new(box_w, box_h));
        ui.painter().rect_filled(box_rect, CornerRadius::same(8), COL_CARD_BG);
        if let Some(Some(bytes)) = detail.screenshots.get(index) {
            let key = format!("{}:{}:shot{index}", game.system.label(), game.title_id);
            if let Some(handle) = app.texture_cache_get(ui.ctx(), &key, bytes) {
                draw_texture_cover_with_painter(ui.painter(), &handle, box_rect);
            }
        }
        ui.painter().rect_stroke(
            box_rect,
            CornerRadius::same(8),
            Stroke::new(1.5_f32, COL_CARD_SELECTED_BORDER),
            egui::StrokeKind::Outside,
        );
    }
}

fn selected_game(app: &App) -> Option<&Game> {
    if app.is_store_tab() {
        app.visible.get(app.selected).and_then(|&i| app.store_games.get(i))
    } else {
        app.visible.get(app.selected).and_then(|&i| app.games.get(i))
    }
}

fn cover_row_top(screen: Rect) -> f32 {
    screen.top() + HEADER_H + 12.0
}

fn cover_row_bottom(screen: Rect) -> f32 {
    cover_row_top(screen) + TILE_H
}

fn draw_backdrop(ui: &mut egui::Ui, app: &App, screen: Rect) {
    let Some(game) = selected_game(app) else { return };
    let Some(bytes) = game.hero_bytes.as_ref() else { return };
    let Some(handle) = app.texture_cache_get(ui.ctx(), &texture_key(game, "hero"), bytes) else { return };

    draw_texture_cover_with_painter(ui.painter(), &handle, screen);
    filled_rect(ui, screen, Color32::from_rgba_unmultiplied(0, 0, 0, 160));

    horizontal_gradient_rect(
        ui,
        Rect::from_min_max(screen.left_top(), Pos2::new(screen.left() + screen.width() * 0.55, screen.bottom())),
        Color32::from_rgba_unmultiplied(0, 0, 0, 195),
        Color32::TRANSPARENT,
    );
    vertical_gradient_rect(
        ui,
        Rect::from_min_max(screen.left_top(), Pos2::new(screen.right(), cover_row_bottom(screen) + 30.0)),
        Color32::from_rgba_unmultiplied(0, 0, 0, 210),
        Color32::TRANSPARENT,
    );
}

fn draw_corner_art(ui: &mut egui::Ui, app: &App, screen: Rect) {
    let Some(game) = selected_game(app) else { return };
    let Some(bytes) = game.logo_bytes.as_ref() else { return };
    let Some(handle) = app.texture_cache_get(ui.ctx(), &texture_key(game, "logo"), bytes) else { return };

    let box_rect = Rect::from_min_max(
        Pos2::new(screen.left() + screen.width() * 0.52, cover_row_bottom(screen) + 16.0),
        Pos2::new(screen.right() - 28.0, screen.bottom() - FOOTER_H - 12.0),
    );
    draw_texture_fit(ui, &handle, box_rect, false);
}

fn draw_texture_fit(ui: &mut egui::Ui, handle: &egui::TextureHandle, box_rect: Rect, allow_upscale: bool) {
    let size = handle.size_vec2();
    if size.x <= 0.0 || size.y <= 0.0 || box_rect.width() <= 0.0 || box_rect.height() <= 0.0 {
        return;
    }
    let mut scale = (box_rect.width() / size.x).min(box_rect.height() / size.y);
    if !allow_upscale {
        scale = scale.min(1.0);
    }
    let drawn = Vec2::new(size.x * scale, size.y * scale);
    let min = Pos2::new(
        box_rect.center().x - drawn.x / 2.0,
        box_rect.bottom() - drawn.y,
    );
    ui.painter().image(
        handle.id(),
        Rect::from_min_size(min, drawn),
        Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
        Color32::WHITE,
    );
}

fn horizontal_gradient_rect(ui: &mut egui::Ui, rect: Rect, left: Color32, right: Color32) {
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(rect.left_top(), left);
    mesh.colored_vertex(rect.right_top(), right);
    mesh.colored_vertex(rect.right_bottom(), right);
    mesh.colored_vertex(rect.left_bottom(), left);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    ui.painter().add(egui::Shape::mesh(mesh));
}

fn vertical_gradient_rect(ui: &mut egui::Ui, rect: Rect, top: Color32, bottom: Color32) {
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(rect.left_top(), top);
    mesh.colored_vertex(rect.right_top(), top);
    mesh.colored_vertex(rect.right_bottom(), bottom);
    mesh.colored_vertex(rect.left_bottom(), bottom);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    ui.painter().add(egui::Shape::mesh(mesh));
}

fn draw_hero_text(ui: &mut egui::Ui, app: &App, screen: Rect) {
    let Some(game) = selected_game(app) else { return };
    let title_y = screen.bottom() - FOOTER_H - 74.0;
    label_at(ui, Pos2::new(screen.left() + MARGIN_X, title_y), &game.title, 22.0, COL_TEXT);

    let mut subtitle = game.system.label().to_string();
    let memberships = app.collections.memberships(&game.title_id);
    if !memberships.is_empty() {
        subtitle.push_str("  ·  ");
        subtitle.push_str(&memberships.join(", "));
    }
    label_at(ui, Pos2::new(screen.left() + MARGIN_X, title_y + 26.0), &subtitle, 13.0, COL_TEXT_DIM);

    if let Some((title_id, message)) = &app.launch_notice {
        if title_id == &game.title_id {
            label_at(
                ui,
                Pos2::new(screen.left() + MARGIN_X, title_y + 46.0),
                message,
                12.5,
                Color32::from_rgb(250, 204, 21),
            );
        }
    }
}

fn draw_cover_row(ui: &mut egui::Ui, app: &App, screen: Rect, commands: &mut Vec<AppCommand>) {
    let row_left = screen.left() + MARGIN_X;
    let row_right = screen.right() - MARGIN_X;
    let tile_y = cover_row_top(screen);

    for (slot, &game_index) in app.visible.iter().enumerate() {
        let Some(game) = app.games.get(game_index) else { continue };

        let x = (row_left + slot as f32 * TILE_SPACING - app.current_scroll).round();
        if x + TILE_W < row_left || x > row_right {
            continue;
        }

        let selected = slot == app.selected;
        let (w, h) = if selected {
            (TILE_W * TILE_SELECTED_SCALE, TILE_H * TILE_SELECTED_SCALE)
        } else {
            (TILE_W, TILE_H)
        };

        let y = tile_y - (h - TILE_H) / 2.0;
        let rect = Rect::from_min_size(Pos2::new(x, y), Vec2::new(w, h));
        let rounding = CornerRadius::same(6);

        let mut cover_drawn = false;
        if let Some(bytes) = &game.cover_bytes {
            if let Some(handle) = app.texture_cache_get(ui.ctx(), &texture_key(game, "cover"), bytes) {
                draw_texture_cover_with_painter(ui.painter(), &handle, rect);
                cover_drawn = true;
            }
        }

        if !cover_drawn {
            draw_card_placeholder_with_painter(ui.painter(), game, rect, rounding);
        }

        if selected {
            ui.painter().rect_stroke(
                rect,
                rounding,
                Stroke::new(2.5_f32, COL_CARD_SELECTED_BORDER),
                egui::StrokeKind::Outside,
            );
        } else {
            ui.painter().rect_stroke(
                rect,
                rounding,
                Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(255, 255, 255, 18)),
                egui::StrokeKind::Inside,
            );
        }

        if game.has_bubble || game.system == crate::scanner::System::Vita {
            draw_installed_badge_with_painter(ui.painter(), rect);
        }

        let is_favorite = app.collections.items.iter().any(|c| {
            (c.name.eq_ignore_ascii_case("Favorites") || c.name.eq_ignore_ascii_case("Favoritos"))
                && c.title_ids.iter().any(|id| id == &game.title_id)
        });
        if is_favorite {
            draw_favorite_badge_with_painter(ui.painter(), rect);
        }

        let response = ui.interact(rect, ui.id().with(("tile", slot)), Sense::click());
        if response.clicked() {
            commands.push(AppCommand::SelectVisibleSlot(slot));
        }
    }
}

fn draw_texture_cover_with_painter(painter: &egui::Painter, handle: &egui::TextureHandle, rect: Rect) {
    let size = handle.size_vec2();
    if size.x <= 0.0 || size.y <= 0.0 {
        return;
    }
    let scale = (rect.width() / size.x).max(rect.height() / size.y);
    let part_w = (rect.width() / scale).min(size.x);
    let part_h = (rect.height() / scale).min(size.y);
    let uv_min = Pos2::new((size.x - part_w) / 2.0 / size.x, (size.y - part_h) / 2.0 / size.y);
    let uv_max = Pos2::new(uv_min.x + part_w / size.x, uv_min.y + part_h / size.y);
    painter.image(handle.id(), rect, Rect::from_min_max(uv_min, uv_max), Color32::WHITE);
}

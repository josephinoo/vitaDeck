use crate::app::App;
use crate::input::AppCommand;
use crate::scanner::Game;
use egui::{Color32, CornerRadius, FontId, Pos2, Rect, Sense, Stroke, Vec2};

pub const SCREEN_W: f32 = 960.0 / 1.3;
pub const SCREEN_H: f32 = 544.0 / 1.3;

const TOP_BAR_H: f32 = 30.0;
const BOTTOM_BAR_H: f32 = 32.0;

const ROW_LEFT: f32 = 26.0;
const TILE_W: f32 = 84.0;
const TILE_H: f32 = 116.0;
pub const TILE_SPACING: f32 = 96.0;
const TILE_SELECTED_SCALE: f32 = 1.06;

const COL_PANEL: Color32 = Color32::from_rgb(38, 43, 51);
const COL_PANEL_LIGHT: Color32 = Color32::from_rgb(58, 65, 76);
const COL_TEXT: Color32 = Color32::from_rgb(238, 240, 244);
const COL_TEXT_DIM: Color32 = Color32::from_rgb(158, 166, 178);
const COL_ACCENT: Color32 = Color32::WHITE;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Browse,
    CollectionPicker,
}

pub fn apply_theme(ctx: &egui::Context) {
    ctx.set_visuals(egui::Visuals::dark());
    let mut style = (*ctx.style()).clone();
    style.spacing.item_spacing = Vec2::new(6.0, 4.0);
    style.visuals.window_shadow = egui::epaint::Shadow::NONE;
    ctx.set_style(style);
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

fn filled_rect(ui: &mut egui::Ui, rect: Rect, color: Color32) {
    ui.painter().rect_filled(rect, CornerRadius::ZERO, color);
}

fn rounded_panel(ui: &mut egui::Ui, rect: Rect, color: Color32) {
    let r = rect.height().min(24.0) / 2.0;
    ui.painter().rect_filled(rect, CornerRadius::same(r as u8), color);
}

pub fn build_ui(ctx: &egui::Context, app: &App) -> Vec<AppCommand> {
    let mut commands = Vec::new();
    egui::CentralPanel::default().frame(egui::Frame::NONE.fill(Color32::BLACK)).show(ctx, |ui| {
        let screen = ui.max_rect();
        filled_rect(ui, screen, Color32::BLACK);
        preload_button_icons(ui);

        if app.is_loading() {
            draw_full_loading_screen(ui, screen);
        } else {
            if app.is_settings() {
                draw_settings(ui, app, screen, &mut commands);
            } else {
                draw_backdrop(ui, app, screen);
                draw_corner_art(ui, app, screen);
                if app.visible.is_empty() {
                    draw_empty_state(ui, app, screen);
                } else {
                    draw_hero_text(ui, app, screen);
                    draw_cover_row(ui, app, screen, &mut commands);
                }
            }

            draw_top_bar(ui, app, screen, &mut commands);
            draw_bottom_bar(ui, app);

            if app.mode == Mode::CollectionPicker {
                draw_collection_picker(ui, app, screen, &mut commands);
            }
        }
    });
    commands
}

fn draw_full_loading_screen(ui: &mut egui::Ui, screen: Rect) {
    let time = ui.ctx().input(|i| i.time);
    let dots = ".".repeat(1 + (time as usize) % 3);
    let progress = (time as f32 * 2.0).sin() * 0.5 + 0.5;
    let bar_width = 280.0;
    let bar_height = 10.0;

    let logo_rect = Rect::from_center_size(screen.center() - egui::vec2(0.0, 72.0), Vec2::splat(56.0));
    draw_builtin_icon(
        ui,
        logo_rect,
        "builtin:icon_playstation",
        include_bytes!("../assets/icons/icon-playstation.png"),
    );

    let text = format!("Scanning and indexing games from memory{dots}");
    let font = FontId::proportional(20.0);
    let galley = ui.painter().layout_no_wrap(text, font, Color32::WHITE);
    let pos = screen.center() - egui::vec2(galley.rect.size().x / 2.0, 25.0);
    ui.painter().galley(pos, galley, Color32::WHITE);

    let bar_rect = Rect::from_center_size(screen.center() + egui::vec2(0.0, 15.0), egui::vec2(bar_width, bar_height));
    ui.painter().rect_filled(bar_rect, CornerRadius::same(5), Color32::from_rgb(40, 44, 52));

    let fill_rect = Rect::from_min_size(bar_rect.min, egui::vec2(bar_width * (0.15 + progress * 0.75), bar_height));
    ui.painter().rect_filled(fill_rect, CornerRadius::same(5), Color32::from_rgb(80, 160, 240));

    let subtext = "Please wait while the data loads...";
    let subfont = FontId::proportional(13.0);
    let subgalley = ui.painter().layout_no_wrap(subtext.to_string(), subfont, COL_TEXT_DIM);
    let subpos = screen.center() - egui::vec2(subgalley.rect.size().x / 2.0, -35.0);
    ui.painter().galley(subpos, subgalley, COL_TEXT_DIM);
}

fn draw_loading_overlay(ui: &mut egui::Ui, app: &App, screen: Rect) {
    let dots = ".".repeat(1 + (ui.ctx().input(|i| i.time) as usize) % 3);
    let text = format!("Loading data for {} games{dots}", app.games.len());
    let text_w = ui.fonts(|f| f.layout_no_wrap(text.clone(), FontId::proportional(14.0), COL_TEXT)).size().x;

    let pad = Vec2::new(14.0, 10.0);
    let size = Vec2::new(text_w + pad.x * 2.0, 20.0 + pad.y * 2.0);
    let pos = Pos2::new(screen.right() - size.x - 16.0, screen.top() + TOP_BAR_H + 16.0);
    let rect = Rect::from_min_size(pos, size);

    ui.painter().rect_filled(rect, CornerRadius::same(10), Color32::from_rgba_unmultiplied(10, 11, 14, 235));
    label_at(ui, rect.min + pad, &text, 14.0, COL_TEXT);
}

fn selected_game(app: &App) -> Option<&Game> {
    app.visible.get(app.selected).and_then(|&i| app.games.get(i))
}

fn texture_key(game: &Game, kind: &str) -> String {
    format!("{}:{}:{kind}", game.system.label(), game.title_id)
}

fn draw_backdrop(ui: &mut egui::Ui, app: &App, screen: Rect) {
    let Some(game) = selected_game(app) else { return };
    let Some(bytes) = game.hero_bytes.as_ref() else { return };
    let Some(handle) = app.texture_cache_get(ui.ctx(), &texture_key(game, "hero"), bytes) else { return };

    draw_texture_cover(ui, &handle, screen);
    filled_rect(ui, screen, Color32::from_rgba_unmultiplied(0, 0, 0, 165));

    horizontal_gradient_rect(
        ui,
        Rect::from_min_max(screen.left_top(), Pos2::new(screen.left() + screen.width() * 0.55, screen.bottom())),
        Color32::from_rgba_unmultiplied(0, 0, 0, 190),
        Color32::TRANSPARENT,
    );
    vertical_gradient_rect(
        ui,
        Rect::from_min_max(screen.left_top(), Pos2::new(screen.right(), cover_row_bottom(screen) + 30.0)),
        Color32::from_rgba_unmultiplied(0, 0, 0, 205),
        Color32::TRANSPARENT,
    );
}

fn draw_corner_art(ui: &mut egui::Ui, app: &App, screen: Rect) {
    let Some(game) = selected_game(app) else { return };
    let Some(bytes) = game.logo_bytes.as_ref() else { return };
    let Some(handle) = app.texture_cache_get(ui.ctx(), &texture_key(game, "logo"), bytes) else { return };

    let box_rect = Rect::from_min_max(
        Pos2::new(screen.left() + screen.width() * 0.52, cover_row_bottom(screen) + 24.0),
        Pos2::new(screen.right() - 28.0, screen.bottom() - BOTTOM_BAR_H - 22.0),
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

fn cover_row_top(screen: Rect) -> f32 {
    screen.top() + TOP_BAR_H + 16.0
}

fn cover_row_bottom(screen: Rect) -> f32 {
    cover_row_top(screen) + TILE_H
}

fn draw_hero_text(ui: &mut egui::Ui, app: &App, screen: Rect) {
    let Some(game) = selected_game(app) else { return };
    let title_y = screen.bottom() - BOTTOM_BAR_H - 96.0;
    label_at(ui, Pos2::new(screen.left() + ROW_LEFT, title_y), &game.title, 24.0, COL_TEXT);

    let mut subtitle = game.system.label().to_string();
    let memberships = app.collections.memberships(&game.title_id);
    if !memberships.is_empty() {
        subtitle.push_str("  -  ");
        subtitle.push_str(&memberships.join(", "));
    }
    label_at(ui, Pos2::new(screen.left() + ROW_LEFT, title_y + 30.0), &subtitle, 14.0, COL_TEXT_DIM);

    if let Some((title_id, message)) = &app.launch_notice {
        if title_id == &game.title_id {
            label_at(
                ui,
                Pos2::new(screen.left() + ROW_LEFT, title_y + 52.0),
                message,
                13.0,
                Color32::from_rgb(250, 204, 21),
            );
        }
    }
}

fn draw_cover_row(ui: &mut egui::Ui, app: &App, screen: Rect, commands: &mut Vec<AppCommand>) {
    let row_left = screen.left() + ROW_LEFT;
    let row_right = screen.right() - 20.0;
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

        let rounding = CornerRadius::same(8);
        if let Some(bytes) = &game.cover_bytes {
            if let Some(handle) = app.texture_cache_get(ui.ctx(), &texture_key(game, "cover"), bytes) {
                draw_texture_cover(ui, &handle, rect);
            } else {
                draw_placeholder_tile(ui, game, rect, rounding);
            }
        } else {
            draw_placeholder_tile(ui, game, rect, rounding);
        }

        if selected {
            ui.painter().rect_stroke(rect, rounding, Stroke::new(3.0_f32, Color32::from_rgb(56, 189, 248)), egui::StrokeKind::Outside);
        }

        let response = ui.interact(rect, ui.id().with(("tile", slot)), Sense::click());
        if response.clicked() {
            commands.push(AppCommand::SelectVisibleSlot(slot));
        }
    }
}

fn draw_placeholder_tile(ui: &mut egui::Ui, game: &Game, rect: Rect, rounding: CornerRadius) {
    ui.painter().rect_filled(rect, rounding, Color32::from_rgb(26, 32, 40));
    ui.painter().rect_stroke(rect, rounding, Stroke::new(1.0_f32, Color32::from_rgb(50, 58, 70)), egui::StrokeKind::Inside);

    let badge_rect = Rect::from_min_size(rect.min + Vec2::new(6.0, 6.0), Vec2::new(rect.width() - 12.0, 16.0));
    let badge_col = match game.system {
        crate::scanner::System::Vita => Color32::from_rgb(14, 116, 144),
        crate::scanner::System::Psp => Color32::from_rgb(3, 105, 161),
        crate::scanner::System::Psx => Color32::from_rgb(109, 40, 217),
    };
    ui.painter().rect_filled(badge_rect, CornerRadius::same(4), badge_col);
    ui.painter().text(badge_rect.center(), egui::Align2::CENTER_CENTER, game.system.label(), FontId::proportional(10.0), Color32::WHITE);

    let text_pos = rect.min + Vec2::new(6.0, 26.0);
    label_at(ui, text_pos, short_title(&game.title), 11.0, Color32::from_rgb(220, 226, 236));
}

fn short_title(title: &str) -> &str {
    const MAX_CHARS: usize = 40;
    match title.char_indices().nth(MAX_CHARS) {
        Some((idx, _)) => &title[..idx],
        None => title,
    }
}

fn draw_top_bar(ui: &mut egui::Ui, app: &App, screen: Rect, commands: &mut Vec<AppCommand>) {
    let rect = Rect::from_min_size(screen.min, Vec2::new(screen.width(), TOP_BAR_H));
    filled_rect(ui, rect, Color32::from_rgba_unmultiplied(10, 11, 14, 235));

    let ps_rect = Rect::from_center_size(
        Pos2::new(rect.left() + 16.0, rect.center().y),
        Vec2::splat(18.0),
    );
    draw_builtin_icon(
        ui,
        ps_rect,
        "builtin:icon_playstation",
        include_bytes!("../assets/icons/icon-playstation.png"),
    );

    let l1_rect = Rect::from_min_size(Pos2::new(ps_rect.right() + 6.0, rect.top() + 6.0), Vec2::new(28.0, TOP_BAR_H - 12.0));
    draw_shoulder_glyph(ui, l1_rect, "L1");

    let mid_y = rect.center().y;
    let mut x_pos = l1_rect.right() + 10.0;

    for (i, tab_name) in app.tabs.iter().enumerate() {
        let is_active = !app.is_settings() && app.active_tab == i;
        let color = if is_active { COL_TEXT } else { COL_TEXT_DIM };
        let display_name = match tab_name.as_str() {
            "RECENTLY PLAYED" => "RECENT",
            other => other,
        };
        let is_favorites = display_name.eq_ignore_ascii_case("FAVORITES")
            || display_name.eq_ignore_ascii_case("FAVORITOS");
        let icon_slot = if is_favorites { 16.0 } else { 0.0 };
        let label_w = ui
            .fonts(|f| f.layout_no_wrap(display_name.to_string(), FontId::proportional(13.0), color))
            .size()
            .x
            .max(30.0);
        let tab_left = x_pos;

        if is_favorites {
            let icon_rect = Rect::from_center_size(
                Pos2::new(tab_left + 7.0, mid_y),
                Vec2::splat(14.0),
            );
            draw_builtin_icon(
                ui,
                icon_rect,
                "builtin:icon_favorites",
                include_bytes!("../assets/icons/icon-favorites.png"),
            );
        }

        label_mid(ui, tab_left + icon_slot, mid_y, display_name, 13.0, color);

        let tab_w = label_w + icon_slot;
        let tab_rect = Rect::from_center_size(
            Pos2::new(tab_left + tab_w / 2.0, mid_y),
            Vec2::new(tab_w, TOP_BAR_H - 8.0),
        );
        if ui.interact(tab_rect, ui.id().with(("nav_tab", i)), Sense::click()).clicked() {
            commands.push(AppCommand::SelectTab(i));
        }

        if is_active {
            let line_y = rect.bottom() - 2.0_f32;
            ui.painter().line_segment(
                [Pos2::new(tab_left, line_y), Pos2::new(tab_left + tab_w, line_y)],
                Stroke::new(2.0_f32, Color32::from_rgb(56, 189, 248)),
            );
        }

        x_pos += tab_w + 14.0;
    }

    let is_settings_active = app.is_settings();
    let settings_color = if is_settings_active { COL_TEXT } else { COL_TEXT_DIM };
    let settings_icon_rect = Rect::from_center_size(
        Pos2::new(x_pos + 7.0, mid_y),
        Vec2::splat(14.0),
    );
    draw_builtin_icon(
        ui,
        settings_icon_rect,
        "builtin:icon_settings",
        include_bytes!("../assets/icons/icon-settings.png"),
    );
    label_mid(ui, x_pos + 16.0, mid_y, "SETTINGS", 13.0, settings_color);
    let settings_label_w = ui
        .fonts(|f| f.layout_no_wrap("SETTINGS".to_string(), FontId::proportional(13.0), settings_color))
        .size()
        .x
        .max(40.0);
    let settings_w = settings_label_w + 16.0;
    let settings_rect = Rect::from_center_size(
        Pos2::new(x_pos + settings_w / 2.0, mid_y),
        Vec2::new(settings_w, TOP_BAR_H - 8.0),
    );
    if ui.interact(settings_rect, ui.id().with("nav_settings"), Sense::click()).clicked() {
        commands.push(AppCommand::SelectTab(app.settings_tab_index()));
    }
    if is_settings_active {
        let line_y = rect.bottom() - 2.0;
        ui.painter().line_segment(
            [Pos2::new(x_pos, line_y), Pos2::new(x_pos + settings_w, line_y)],
            Stroke::new(2.0_f32, Color32::from_rgb(56, 189, 248)),
        );
    }

    let r1_rect = Rect::from_min_size(Pos2::new(x_pos + settings_w + 8.0, rect.top() + 6.0), Vec2::new(28.0, TOP_BAR_H - 12.0));
    draw_shoulder_glyph(ui, r1_rect, "R1");

    label_mid(ui, rect.right() - 44.0, mid_y, &app.clock_line(), 14.0, COL_TEXT);

    let battery_pct = app.battery_pct();
    let battery_pos = Pos2::new(rect.right() - 118.0, mid_y - 6.0);
    draw_battery_icon(ui, battery_pos, battery_pct as f32);
    label_mid(ui, battery_pos.x + 24.0, mid_y, &format!("{battery_pct}%"), 12.0, COL_TEXT);

    draw_wifi_icon(ui, Pos2::new(battery_pos.x - 22.0, mid_y), app.wifi_connected());

    if app.search_active {
        let search_rect = Rect::from_min_size(
            Pos2::new(screen.left() + 20.0, rect.bottom() + 4.0),
            Vec2::new(screen.width() - 40.0, 30.0),
        );
        ui.painter().rect_filled(
            search_rect,
            CornerRadius::same(6),
            Color32::from_rgba_unmultiplied(20, 24, 32, 245),
        );
        ui.painter().rect_stroke(
            search_rect,
            CornerRadius::same(6),
            Stroke::new(1.5, Color32::from_rgb(56, 189, 248)),
            egui::StrokeKind::Inside,
        );

        let query_text = if app.search_query.is_empty() {
            "SEARCH GAMES... [SELECT: CLOSE]"
        } else {
            &app.search_query
        };
        let query_color = if app.search_query.is_empty() {
            COL_TEXT_DIM
        } else {
            Color32::WHITE
        };
        label_at(
            ui,
            Pos2::new(search_rect.left() + 10.0, search_rect.center().y - 7.0),
            &format!("SEARCH: {}", query_text),
            13.0,
            query_color,
        );
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
            let text_w = label.len() as f32 * 6.0;
            label_at(
                ui,
                Pos2::new(rect.center().x - text_w / 2.0, rect.center().y - 6.0),
                label,
                11.0,
                COL_TEXT_DIM,
            );
            return;
        }
    };

    if !draw_builtin_icon(ui, rect, name, bytes) {
        rounded_panel(ui, rect, Color32::from_rgba_unmultiplied(255, 255, 255, 22));
        let text_w = label.len() as f32 * 6.0;
        label_at(
            ui,
            Pos2::new(rect.center().x - text_w / 2.0, rect.center().y - 6.0),
            label,
            11.0,
            COL_TEXT_DIM,
        );
    }
}

fn draw_wifi_icon(ui: &mut egui::Ui, center: Pos2, connected: bool) {
    let color = if connected { COL_TEXT } else { Color32::from_rgb(110, 116, 126) };
    let bar = Color32::from_rgba_unmultiplied(16, 19, 24, 255);
    for i in (0..3).rev() {
        let r = 3.0 + i as f32 * 3.2;
        ui.painter().circle_filled(center, r, color);
        ui.painter().circle_filled(center, r - 1.4, bar);
    }
    let mask = Rect::from_min_size(Pos2::new(center.x - 11.0, center.y), Vec2::new(22.0, 11.0));
    filled_rect(ui, mask, bar);
    ui.painter().circle_filled(center, 1.6, color);
}

fn draw_battery_icon(ui: &mut egui::Ui, pos: Pos2, pct: f32) {
    let w = 22.0;
    let h = 11.0;
    let outline = Rect::from_min_size(pos, Vec2::new(w, h));
    ui.painter().rect_filled(outline, CornerRadius::same(2), COL_TEXT);
    ui.painter().rect_filled(Rect::from_min_size(Pos2::new(pos.x + w, pos.y + 2.5), Vec2::new(2.5, h - 5.0)), CornerRadius::same(1), COL_TEXT);
    let inner = outline.shrink(2.0);
    filled_rect(ui, inner, Color32::from_rgba_unmultiplied(16, 19, 24, 255));
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
}

impl Glyph {
    fn color(self) -> Color32 {
        match self {
            Glyph::Cross => Color32::from_rgb(0x38, 0xbd, 0xf8),
            Glyph::Circle => Color32::from_rgb(0xf8, 0x71, 0x71),
            Glyph::Triangle => Color32::from_rgb(0x34, 0xd3, 0x99),
            Glyph::Square => Color32::from_rgb(0xf4, 0x72, 0xb6),
            Glyph::Select => Color32::from_rgb(0x94, 0xa3, 0xb8),
        }
    }
}

fn draw_bottom_bar(ui: &mut egui::Ui, app: &App) {
    let screen = ui.max_rect();
    let bottom_bar_y = screen.bottom() - BOTTOM_BAR_H;
    let rect = Rect::from_min_size(Pos2::new(screen.left(), bottom_bar_y), Vec2::new(screen.width(), BOTTOM_BAR_H));
    filled_rect(ui, rect, Color32::from_rgba_unmultiplied(12, 14, 18, 240));

    let confirm_label = if selected_game(app).is_some_and(|g| g.system == crate::scanner::System::Vita) {
        "LAUNCH"
    } else {
        "SELECT"
    };

    let search_label = if app.search_active { "CLOSE SEARCH" } else { "SEARCH" };

    let hints: &[(Glyph, &str)] = if app.is_settings() {
        &[(Glyph::Cross, "RESCAN LIBRARY"), (Glyph::Circle, "BACK")]
    } else if app.mode == Mode::CollectionPicker {
        &[(Glyph::Cross, "TOGGLE"), (Glyph::Circle, "BACK")]
    } else if app.tab_is_collection() {
        &[
            (Glyph::Square, "REMOVE"),
            (Glyph::Triangle, "COLLECTION"),
            (Glyph::Select, search_label),
            (Glyph::Cross, confirm_label),
            (Glyph::Circle, "BACK"),
        ]
    } else {
        &[
            (Glyph::Triangle, "COLLECTION"),
            (Glyph::Select, search_label),
            (Glyph::Cross, confirm_label),
            (Glyph::Circle, "BACK"),
        ]
    };

    let mut x = rect.left() + 16.0;
    let mid_y = rect.center().y;

    for (glyph, label) in hints {
        let icon_w = match glyph {
            Glyph::Select => 32.0,
            _ => 18.0,
        };

        let icon_y = match glyph {
            Glyph::Select => mid_y + 1.0,
            _ => mid_y,
        };
        draw_button_glyph(ui, Pos2::new(x + icon_w / 2.0, icon_y), *glyph);
        label_mid(ui, x + icon_w + 4.0, mid_y, label, 12.0, COL_TEXT_DIM);
        x += icon_w + 4.0 + label.len() as f32 * 6.2 + 16.0;
    }
}

fn draw_button_glyph(ui: &mut egui::Ui, center: Pos2, glyph: Glyph) {

    let size = match glyph {
        Glyph::Select => Vec2::new(32.0, 16.0),
        _ => Vec2::splat(18.0),
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
    let radius = 8.0;
    let stroke = Stroke::new(2.0_f32, glyph.color());
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
            let rect = Rect::from_center_size(center, Vec2::new(14.0, 7.0));
            painter.rect_stroke(
                rect,
                CornerRadius::same(2),
                stroke,
                egui::StrokeKind::Outside,
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

fn draw_settings(ui: &mut egui::Ui, app: &App, screen: Rect, commands: &mut Vec<AppCommand>) {
    let left = screen.left() + ROW_LEFT;
    let mut y = screen.top() + TOP_BAR_H + 40.0;

    let title_icon = Rect::from_center_size(Pos2::new(left + 12.0, y + 10.0), Vec2::splat(22.0));
    draw_builtin_icon(
        ui,
        title_icon,
        "builtin:icon_settings",
        include_bytes!("../assets/icons/icon-settings.png"),
    );
    label_at(ui, Pos2::new(left + 30.0, y), "Settings", 24.0, COL_TEXT);
    y += 40.0;

    let vita_games = app.games.iter().filter(|g| g.system == crate::scanner::System::Vita).count();
    let psp_games = app.games.iter().filter(|g| g.system == crate::scanner::System::Psp).count();
    let psx_games = app.games.iter().filter(|g| g.system == crate::scanner::System::Psx).count();

    let rows = [
        ("Version".to_string(), format!("VitaDeck {}", env!("CARGO_PKG_VERSION"))),
        ("Library".to_string(), format!("{} PS Vita  ·  {} PSP  ·  {} PS1", vita_games, psp_games, psx_games)),
        ("Collections".to_string(), format!("{}", app.collections.items.len())),
        ("Network".to_string(), app.net_line.clone()),
        ("Wi-Fi".to_string(), if app.wifi_connected() { "connected".to_string() } else { "offline".to_string() }),
    ];

    for (label, value) in rows {
        label_at(ui, Pos2::new(left, y), &label, 13.0, COL_TEXT_DIM);
        label_at(ui, Pos2::new(left + 130.0, y), &value, 13.0, COL_TEXT);
        y += 24.0;
    }

    y += 20.0;
    let btn_rect = Rect::from_min_size(Pos2::new(left, y), Vec2::new(200.0, 36.0));
    rounded_panel(ui, btn_rect, Color32::from_rgba_unmultiplied(56, 189, 248, 30));
    ui.painter().rect_stroke(btn_rect, CornerRadius::same(6), Stroke::new(1.5, Color32::from_rgb(56, 189, 248)), egui::StrokeKind::Inside);
    label_at(ui, Pos2::new(btn_rect.left() + 24.0, btn_rect.center().y - 7.0), "RESCAN LIBRARY", 13.0, Color32::WHITE);

    if ui.interact(btn_rect, ui.id().with("rescan_btn"), Sense::click()).clicked() {
        commands.push(AppCommand::Rescan);
    }
}

fn draw_empty_state(ui: &mut egui::Ui, app: &App, screen: Rect) {
    label_at(ui, screen.min + Vec2::new(40.0, 100.0), "No games found in this tab", 18.0, Color32::from_rgb(255, 210, 90));

    let mut line_y = 130.0;
    for line in app.scan_log.iter().take(8) {
        label_at(ui, screen.min + Vec2::new(40.0, line_y), line, 12.0, Color32::from_rgb(200, 200, 200));
        line_y += 15.0;
    }
    label_at(
        ui,
        screen.min + Vec2::new(40.0, line_y + 6.0),
        "Full log at ux0:data/VitaDeck/scan_log.txt",
        11.0,
        Color32::from_rgb(140, 140, 140),
    );
}

fn draw_texture_cover(ui: &mut egui::Ui, handle: &egui::TextureHandle, rect: Rect) {
    let size = handle.size_vec2();
    if size.x <= 0.0 || size.y <= 0.0 {
        return;
    }
    let scale = (rect.width() / size.x).max(rect.height() / size.y);
    let part_w = (rect.width() / scale).min(size.x);
    let part_h = (rect.height() / scale).min(size.y);
    let uv_min = Pos2::new((size.x - part_w) / 2.0 / size.x, (size.y - part_h) / 2.0 / size.y);
    let uv_max = Pos2::new(uv_min.x + part_w / size.x, uv_min.y + part_h / size.y);
    ui.painter().image(handle.id(), rect, Rect::from_min_max(uv_min, uv_max), Color32::WHITE);
}

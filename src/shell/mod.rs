#[link(name = "SDL2", kind = "static")]
unsafe extern "C" {}

#[cfg(target_os = "vita")]
#[link(name = "vitaGL", kind = "static")]
#[link(name = "vita2d", kind = "static")]
#[link(name = "mathneon", kind = "static")]
unsafe extern "C" {}
mod egui_painter;
mod surface;

use crate::app::App;
use crate::input::{
    held_stick_direction, map_controller_button_event, map_pointer_event, open_first_controller,
    register_vita_controller_mapping,
};
use anyhow::Result;
use std::time::{Duration, Instant};
use surface::{VitaSurface, HEIGHT, WIDTH};

const UI_SCALE: f32 = 1.3;
const ACTIVE_FRAME_TIME: Duration = Duration::from_millis(16);
const FRAME_BUDGET: Duration = Duration::from_micros(16_667);
const SEVERE_FRAME: Duration = Duration::from_millis(33);
const FRAME_REPORT_INTERVAL: u32 = 300;
const STICK_REPEAT_DELAY: Duration = Duration::from_millis(200);
const STICK_REPEAT_INTERVAL: Duration = Duration::from_millis(70);

#[derive(Default)]
struct FrameProfiler {
    frames: u32,
    over_budget: u32,
    max_total: Duration,
    max_cpu: Duration,
}

impl FrameProfiler {
    fn record(&mut self, total: Duration, cpu: Duration, paint: surface::FramePaintStats) {
        self.frames += 1;
        self.max_total = self.max_total.max(total);
        self.max_cpu = self.max_cpu.max(cpu);
        if total > FRAME_BUDGET { self.over_budget += 1; }
        if total > SEVERE_FRAME {
            crate::logger::log(&format!(
                "slow frame {:.1}ms (cpu {:.1}ms, textures {:.1}ms, geometry {:.1}ms, present {:.1}ms)",
                total.as_secs_f64() * 1000.0,
                cpu.as_secs_f64() * 1000.0,
                paint.texture_apply_secs * 1000.0,
                paint.geometry_secs * 1000.0,
                paint.present_secs * 1000.0,
            ));
        }
        if self.frames >= FRAME_REPORT_INTERVAL {
            crate::logger::log(&format!(
                "frames: {}/{} over 16.7ms; max {:.1}ms total / {:.1}ms cpu",
                self.over_budget,
                self.frames,
                self.max_total.as_secs_f64() * 1000.0,
                self.max_cpu.as_secs_f64() * 1000.0,
            ));
            *self = Self::default();
        }
    }
}

pub fn run(mut app: App) -> Result<()> {
    crate::logger::log("shell::run: sdl2::init()");
    let sdl = sdl2::init().map_err(anyhow::Error::msg)?;
    crate::logger::log("shell::run: video subsystem");
    let video = sdl.video().map_err(anyhow::Error::msg)?;
    let _ = register_vita_controller_mapping(&sdl);
    crate::logger::log("shell::run: game controller");
    let controllers = sdl.game_controller().map_err(anyhow::Error::msg)?;
    let mut controller = open_first_controller(&controllers);
    let mut event_pump = sdl.event_pump().map_err(anyhow::Error::msg)?;
    crate::logger::log("shell::run: VitaSurface::new()");
    let mut surface = VitaSurface::new(&video)?;
    crate::logger::log("shell::run: egui context");

    let egui_ctx = egui::Context::default();

    crate::ui::apply_theme(&egui_ctx);
    crate::logger::log("shell::run: entrando al loop principal");
    let start_time = Instant::now();
    let mut pointer_pos = egui::Pos2::ZERO;
    let mut held_direction = None;
    let mut held_since = Instant::now();
    let mut last_repeat_at = Instant::now();

    let mut frame_count: u32 = 0;
    let mut frame_profiler = FrameProfiler::default();

    loop {
        let frame_started_at = Instant::now();
        let log_frame = frame_count < 3;
        if log_frame {
            crate::logger::log(&format!("frame {}: poll events", frame_count));
        }
        let mut egui_events = Vec::new();
        let mut direct_commands = Vec::new();
        let screen_points = (WIDTH as f32 / UI_SCALE, HEIGHT as f32 / UI_SCALE);
        for event in event_pump.poll_iter() {
            if let Some(egui_event) = map_pointer_event(&event, screen_points, UI_SCALE, &mut pointer_pos) {
                egui_events.push(egui_event);
            }
            if let Some(command) = map_controller_button_event(&event) {
                direct_commands.push(command);
            }
            match event {
                sdl2::event::Event::Quit { .. }
                | sdl2::event::Event::AppWillEnterBackground { .. }
                | sdl2::event::Event::AppDidEnterBackground { .. } => {
                    #[cfg(target_os = "vita")]
                    unsafe {
                        crate::session::finish_cleanly();
                        vitasdk_sys::sceKernelExitProcess(0);
                    }
                    #[cfg(not(target_os = "vita"))]
                    {
                        crate::session::finish_cleanly();
                        return Ok(());
                    }
                }
                sdl2::event::Event::ControllerDeviceAdded { .. } if controller.is_none() => {
                    controller = open_first_controller(&controllers);
                }
                sdl2::event::Event::ControllerDeviceRemoved { .. } => controller = None,
                _ => {}
            }
        }
        match held_stick_direction(controller.as_ref()) {
            Some(direction) if held_direction == Some(direction) => {
                if held_since.elapsed() >= STICK_REPEAT_DELAY && last_repeat_at.elapsed() >= STICK_REPEAT_INTERVAL {
                    direct_commands.push(direction.into());
                    last_repeat_at = Instant::now();
                }
            }
            Some(direction) => {
                direct_commands.push(direction.into());
                held_direction = Some(direction);
                held_since = Instant::now();
                last_repeat_at = Instant::now();
            }
            None => held_direction = None,
        }
        for command in direct_commands {
            app.handle_command(command);
        }
        if log_frame {
            crate::logger::log(&format!("frame {}: tick", frame_count));
        }
        let cpu_started_at = Instant::now();
        app.tick(&egui_ctx);
        if log_frame {
            crate::logger::log(&format!("frame {}: egui run", frame_count));
        }
        let raw_input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(WIDTH as f32 / UI_SCALE, HEIGHT as f32 / UI_SCALE),
            )),
            viewport_id: egui::ViewportId::ROOT,
            viewports: std::iter::once((
                egui::ViewportId::ROOT,
                egui::ViewportInfo { native_pixels_per_point: Some(UI_SCALE), ..Default::default() },
            ))
            .collect(),
            time: Some(start_time.elapsed().as_secs_f64()),
            predicted_dt: ACTIVE_FRAME_TIME.as_secs_f32(),
            events: egui_events,
            ..Default::default()
        };
        let mut ui_commands = Vec::new();
        let full_output = egui_ctx.run(raw_input, |ctx| {
            ui_commands = crate::ui::build_ui(ctx, &app);
        });
        for command in ui_commands {
            app.handle_command(command);
        }
        if log_frame {
            crate::logger::log(&format!("frame {}: tessellate", frame_count));
        }
        let clipped_primitives = egui_ctx.tessellate(full_output.shapes, full_output.pixels_per_point);
        if log_frame {
            crate::logger::log(&format!("frame {}: draw_scene", frame_count));
        }
        surface.draw_scene();
        if log_frame {
            let tex_set = full_output.textures_delta.set.len();
            let tex_free = full_output.textures_delta.free.len();
            let prims = clipped_primitives.len();
            crate::logger::log(&format!(
                "frame {}: paint_egui (prims={}, tex_set={}, tex_free={})",
                frame_count, prims, tex_set, tex_free
            ));
        }

        let paint = surface.paint_egui(full_output.pixels_per_point, &clipped_primitives, &full_output.textures_delta)?;
        let total = frame_started_at.elapsed();
        let cpu = cpu_started_at.elapsed().saturating_sub(Duration::from_secs_f64(paint.present_secs));
        frame_profiler.record(total, cpu, paint);
        if log_frame {
            crate::logger::log(&format!("frame {}: DONE ✓", frame_count));
        }
        frame_count += 1;
    }
}

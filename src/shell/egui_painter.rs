use anyhow::Result;
use sdl2::pixels::PixelFormatEnum;
use sdl2::rect::Rect;
use sdl2::render::BlendMode;
use std::collections::HashMap;

const RETRIES_PER_FRAME: usize = 2;

const MAX_UPLOAD_ATTEMPTS: u32 = 8;
const BACKOFF_RETRY_INTERVAL: std::time::Duration = std::time::Duration::from_millis(250);

const NEW_TEXTURES_PER_FRAME: usize = 2;

const MAX_PENDING_UPLOADS: usize = 16;

fn is_font_texture(id: egui::TextureId) -> bool {

    id == egui::TextureId::default()
}

fn is_small_ui_texture(size: [usize; 2]) -> bool {
    size[0] <= 160 && size[1] <= 160
}
#[derive(Default)]
pub struct SdlEguiPainter {
    textures: HashMap<egui::TextureId, SdlEguiTexture>,
    retired: Vec<SdlEguiTexture>,
    pending: HashMap<egui::TextureId, PendingUpload>,
    vertices: Vec<sdl2::render::Vertex>,
    indices: Vec<i32>,
    scratch: Vec<u8>,
}
struct SdlEguiTexture {
    texture: Option<sdl2::render::Texture>,
    uv_scale: egui::Vec2,
}
impl Drop for SdlEguiTexture {
    fn drop(&mut self) {
        if let Some(texture) = self.texture.take() {
            unsafe { texture.destroy() };
        }
    }
}
struct PendingUpload {
    size: [usize; 2],
    pos: Option<[usize; 2]>,
    pixels: Vec<u8>,
    attempts: u32,
    next_retry_at: std::time::Instant,
}
#[derive(Default, Clone, Copy)]
pub struct PaintStats {
    pub texture_apply_secs: f64,
    pub geometry_secs: f64,
    pub draw_calls: u32,
    pub textures_uploaded: u32,
    pub vertices_drawn: u32,
}
impl SdlEguiPainter {
    pub fn paint(
        &mut self,
        canvas: &mut sdl2::render::Canvas<sdl2::video::Window>,
        screen_size: [u32; 2],
        pixels_per_point: f32,
        primitives: &[egui::ClippedPrimitive],
        textures_delta: &egui::TexturesDelta,
    ) -> Result<PaintStats> {
        let texture_apply_started_at = std::time::Instant::now();
        let textures_uploaded = self.apply_textures(canvas, textures_delta);
        let texture_apply_secs = texture_apply_started_at.elapsed().as_secs_f64();
        let geometry_started_at = std::time::Instant::now();
        let mut draw_calls = 0u32;
        let mut vertices_drawn = 0u32;
        let mut current_clip: Option<sdl2::rect::Rect> = None;
        let mut current_texture_id: Option<egui::TextureId> = None;
        for clipped_primitive in primitives {
            let Some(clip_rect) =
                Self::sdl_clip_rect(clipped_primitive.clip_rect, screen_size, pixels_per_point)
            else {
                continue;
            };
            let egui::epaint::Primitive::Mesh(mesh) = &clipped_primitive.primitive else {
                continue;
            };
            if mesh.indices.is_empty() || mesh.vertices.is_empty() {
                continue;
            }
            let uv_scale = match self.textures.get(&mesh.texture_id) {
                Some(t) => t.uv_scale,
                None if mesh.texture_id != egui::TextureId::default() => {
                    self.flush_batch(canvas, current_texture_id, &mut draw_calls, &mut vertices_drawn);
                    current_clip = None;
                    current_texture_id = None;
                    continue;
                }
                None => egui::vec2(1.0, 1.0),
            };
            let same_batch = current_clip == Some(clip_rect) && current_texture_id == Some(mesh.texture_id);
            if !same_batch {
                self.flush_batch(canvas, current_texture_id, &mut draw_calls, &mut vertices_drawn);
                canvas.set_clip_rect(clip_rect);
                current_clip = Some(clip_rect);
                current_texture_id = Some(mesh.texture_id);
            }
            let base_index = self.vertices.len() as u32;
            self.vertices.extend(
                mesh.vertices
                    .iter()
                    .map(|vertex| Self::sdl_vertex(vertex, pixels_per_point, uv_scale)),
            );
            self.indices.extend(mesh.indices.iter().map(|&i| (base_index + i) as i32));
        }
        self.flush_batch(canvas, current_texture_id, &mut draw_calls, &mut vertices_drawn);
        let geometry_secs = geometry_started_at.elapsed().as_secs_f64();
        canvas.set_clip_rect(None);
        for texture_id in &textures_delta.free {
            self.pending.remove(texture_id);
            self.destroy_texture(*texture_id);
        }
        Ok(PaintStats { texture_apply_secs, geometry_secs, draw_calls, textures_uploaded, vertices_drawn })
    }
    fn is_new_creation(&self, texture_id: egui::TextureId, pos: Option<[usize; 2]>) -> bool {
        pos.is_none() || !self.textures.contains_key(&texture_id)
    }
    fn destroy_texture(&mut self, id: egui::TextureId) {
        if let Some(texture) = self.textures.remove(&id) {
            // SDL may still have queued geometry referencing this texture.
            // Keep it alive through RenderPresent, then let GXM synchronize
            // destruction after the frame has been submitted.
            self.retired.push(texture);
        }
    }
    pub fn after_present(&mut self) {
        self.retired.clear();
    }
    pub fn resource_counts(&self) -> (usize, usize, usize) {
        (self.textures.len(), self.pending.len(),
            self.pending.values().map(|upload| upload.pixels.len()).sum())
    }
    fn flush_batch(
        &mut self,
        canvas: &mut sdl2::render::Canvas<sdl2::video::Window>,
        texture_id: Option<egui::TextureId>,
        draw_calls: &mut u32,
        vertices_drawn: &mut u32,
    ) {
        if self.indices.is_empty() || self.vertices.is_empty() {
            self.vertices.clear();
            self.indices.clear();
            return;
        }
        let texture_ref = texture_id
            .and_then(|id| self.textures.get(&id))
            .and_then(|t| t.texture.as_ref());
        if let Err(err) = canvas.render_geometry(&self.vertices, texture_ref, &self.indices) {
            eprintln!("skipped a draw call: {err}");
        } else {
            *draw_calls += 1;
            *vertices_drawn += self.vertices.len() as u32;
        }
        self.vertices.clear();
        self.indices.clear();
    }
    fn apply_textures(
        &mut self,
        canvas: &mut sdl2::render::Canvas<sdl2::video::Window>,
        textures_delta: &egui::TexturesDelta,
    ) -> u32 {
        let mut uploaded = 0u32;
        let mut new_creations = 0usize;

        if let Some(font_id) = self
            .pending
            .keys()
            .copied()
            .find(|id| is_font_texture(*id))
        {
            let now = std::time::Instant::now();
            if let Some(upload) = self.pending.remove(&font_id) {
                if upload.next_retry_at <= now {
                    self.upload(
                        canvas,
                        font_id,
                        upload.size,
                        upload.pos,
                        &upload.pixels,
                        upload.attempts,
                    );
                    new_creations += 1;
                    uploaded += 1;
                } else {
                    self.pending.insert(font_id, upload);
                }
            }
        }

        if !self.pending.is_empty() {
            let now = std::time::Instant::now();
            let retry: Vec<egui::TextureId> = self
                .pending
                .iter()
                .filter(|(id, upload)| !is_font_texture(**id) && upload.next_retry_at <= now)
                .map(|(id, _)| *id)
                .take(RETRIES_PER_FRAME.min(NEW_TEXTURES_PER_FRAME.saturating_sub(new_creations)))
                .collect();
            for texture_id in retry {
                let Some(upload) = self.pending.remove(&texture_id) else { continue };
                self.upload(canvas, texture_id, upload.size, upload.pos, &upload.pixels, upload.attempts);
                new_creations += 1;
                uploaded += 1;
            }
        }
        let mut scratch = std::mem::take(&mut self.scratch);

        let mut ordered: Vec<(egui::TextureId, &egui::epaint::ImageDelta)> =
            textures_delta.set.iter().map(|(id, d)| (*id, d)).collect();
        ordered.sort_by_key(|(id, d)| {
            let size = d.image.size();
            let priority = is_font_texture(*id) || is_small_ui_texture(size);
            !priority
        });

        for (texture_id, delta) in ordered {
            scratch.clear();
            Self::fill_sdl_rgba(&delta.image, &mut scratch);
            let is_new = self.is_new_creation(texture_id, delta.pos);
            let size = delta.image.size();
            let priority = is_font_texture(texture_id) || is_small_ui_texture(size);
            if is_new && !priority && new_creations >= NEW_TEXTURES_PER_FRAME {
                self.enqueue_pending(
                    texture_id,
                    size,
                    delta.pos,
                    &scratch,
                    0,
                    std::time::Instant::now(),
                );
                continue;
            }
            if is_new {
                new_creations += 1;
            }
            self.upload(canvas, texture_id, size, delta.pos, &scratch, 0);
            uploaded += 1;
        }
        self.scratch = scratch;
        uploaded
    }
    fn defer_or_give_up(
        &mut self,
        texture_id: egui::TextureId,
        size: [usize; 2],
        pos: Option<[usize; 2]>,
        pixels: &[u8],
        attempts: u32,
    ) {
        let attempts = attempts + 1;
        if attempts >= MAX_UPLOAD_ATTEMPTS {
            eprintln!("giving up on a {}x{} texture after {attempts} attempts", size[0], size[1]);
            return;
        }
        self.enqueue_pending(
            texture_id,
            size,
            pos,
            pixels,
            attempts,
            std::time::Instant::now() + BACKOFF_RETRY_INTERVAL,
        );
    }

    fn enqueue_pending(
        &mut self,
        texture_id: egui::TextureId,
        size: [usize; 2],
        pos: Option<[usize; 2]>,
        pixels: &[u8],
        attempts: u32,
        next_retry_at: std::time::Instant,
    ) {

        if self.pending.contains_key(&texture_id) {
            self.pending.insert(
                texture_id,
                PendingUpload {
                    size,
                    pos,
                    pixels: pixels.to_vec(),
                    attempts,
                    next_retry_at,
                },
            );
            return;
        }
        while self.pending.len() >= MAX_PENDING_UPLOADS {

            let victim = self
                .pending
                .iter()
                .filter(|(id, _)| !is_font_texture(**id))
                .max_by_key(|(_, u)| u.pixels.len())
                .map(|(id, _)| *id);
            let Some(victim) = victim else { break };
            self.pending.remove(&victim);
            eprintln!("dropped pending texture upload (queue full)");
        }
        self.pending.insert(
            texture_id,
            PendingUpload {
                size,
                pos,
                pixels: pixels.to_vec(),
                attempts,
                next_retry_at,
            },
        );
    }
    fn upload(
        &mut self,
        canvas: &mut sdl2::render::Canvas<sdl2::video::Window>,
        texture_id: egui::TextureId,
        size: [usize; 2],
        pos: Option<[usize; 2]>,
        pixels: &[u8],
        attempts: u32,
    ) {
        let [width, height] = size;
        if pos.is_none() || !self.textures.contains_key(&texture_id) {

            let texture = if is_font_texture(texture_id) {
                canvas.create_texture_streaming(PixelFormatEnum::RGBA32, width as u32, height as u32)
            } else {
                canvas.create_texture_static(PixelFormatEnum::RGBA32, width as u32, height as u32)
            };
            let mut texture = match texture {
                Ok(texture) => texture,
                Err(err) => {
                    eprintln!("no room for a {width}x{height} texture, will retry: {err}");
                    self.defer_or_give_up(texture_id, size, pos, pixels, attempts);
                    return;
                }
            };
            texture.set_blend_mode(BlendMode::Blend);
            if let Err(err) = texture.update(Rect::new(0, 0, width as u32, height as u32), pixels, width * 4) {
                eprintln!("couldn't upload a texture, will retry: {err}");
                unsafe { texture.destroy() };
                self.defer_or_give_up(texture_id, size, pos, pixels, attempts);
                return;
            }
            self.destroy_texture(texture_id);
            self.textures.insert(texture_id, SdlEguiTexture { texture: Some(texture), uv_scale: egui::vec2(1.0, 1.0) });
            return;
        }
        let Some([x, y]) = pos else {
            eprintln!("partial texture update with no position, skipped");
            return;
        };
        let Some(existing) = self.textures.get_mut(&texture_id) else {
            eprintln!("partial update for a texture that no longer exists, skipped");
            return;
        };
        if let Some(existing_tex) = existing.texture.as_mut() {
            if let Err(err) =
                existing_tex.update(Rect::new(x as i32, y as i32, width as u32, height as u32), pixels, width * 4)
            {
                eprintln!("couldn't patch a texture: {err}");
            }
        }
    }
    fn fill_sdl_rgba(image: &egui::ImageData, out: &mut Vec<u8>) {
        match image {
            egui::ImageData::Color(image) => {

                let bytes: &[u8] = unsafe {
                    std::slice::from_raw_parts(image.pixels.as_ptr() as *const u8, image.pixels.len() * 4)
                };
                out.extend_from_slice(bytes);
            }
            egui::ImageData::Font(image) => {
                out.reserve(image.pixels.len() * 4);
                for &coverage in &image.pixels {
                    let a = (coverage * 255.0).round().clamp(0.0, 255.0) as u8;
                    out.extend_from_slice(&[255, 255, 255, a]);
                }
            }
        }
    }
    fn sdl_vertex(
        vertex: &egui::epaint::Vertex,
        pixels_per_point: f32,
        uv_scale: egui::Vec2,
    ) -> sdl2::render::Vertex {
        let [r, g, b, a] = vertex.color.to_array();
        sdl2::render::Vertex {
            position: sdl2::rect::FPoint::new(
                vertex.pos.x * pixels_per_point,
                vertex.pos.y * pixels_per_point,
            ),
            color: sdl2::pixels::Color::RGBA(r, g, b, a),
            tex_coord: sdl2::rect::FPoint::new(
                (vertex.uv.x * uv_scale.x).clamp(0.0, 1.0),
                (vertex.uv.y * uv_scale.y).clamp(0.0, 1.0),
            ),
        }
    }
    #[cfg(test)]
    pub(crate) fn uv_for_test(uv: egui::Vec2, uv_scale: egui::Vec2) -> (f32, f32) {
        let vertex = egui::epaint::Vertex {
            pos: egui::Pos2::ZERO,
            uv: uv.to_pos2(),
            color: egui::Color32::WHITE,
        };
        let v = Self::sdl_vertex(&vertex, 1.0, uv_scale);
        (v.tex_coord.x(), v.tex_coord.y())
    }
    fn sdl_clip_rect(
        clip_rect: egui::Rect,
        [screen_width, screen_height]: [u32; 2],
        pixels_per_point: f32,
    ) -> Option<sdl2::rect::Rect> {
        let min_x = (clip_rect.min.x * pixels_per_point)
            .floor()
            .clamp(0.0, screen_width as f32) as i32;
        let min_y = (clip_rect.min.y * pixels_per_point)
            .floor()
            .clamp(0.0, screen_height as f32) as i32;
        let max_x = (clip_rect.max.x * pixels_per_point)
            .ceil()
            .clamp(0.0, screen_width as f32) as i32;
        let max_y = (clip_rect.max.y * pixels_per_point)
            .ceil()
            .clamp(0.0, screen_height as f32) as i32;
        let width = (max_x - min_x).max(0) as u32;
        let height = (max_y - min_y).max(0) as u32;
        if width == 0 || height == 0 {
            None
        } else {
            Some(sdl2::rect::Rect::new(min_x, min_y, width, height))
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires SDL_VIDEODRIVER=dummy; run separately from other SDL tests"]
    fn rapid_texture_turnover_retires_only_after_present() {
        let sdl = sdl2::init().unwrap();
        let video = sdl.video().unwrap();
        let window = video.window("texture lifecycle", 64, 64).hidden().build().unwrap();
        let mut canvas = window.into_canvas().software().build().unwrap();
        let mut painter = SdlEguiPainter::default();
        for frame in 1..=500 {
            let id = egui::TextureId::Managed(frame);
            let mut mesh = egui::Mesh::with_texture(id);
            let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(32.0, 32.0));
            mesh.add_rect_with_uv(rect, egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
            let delta = egui::TexturesDelta {
                set: vec![(id, egui::epaint::ImageDelta::full(
                    egui::ColorImage::new([32, 32], egui::Color32::WHITE), egui::TextureOptions::LINEAR))],
                free: vec![id],
            };
            let primitives = [egui::ClippedPrimitive { clip_rect: rect,
                primitive: egui::epaint::Primitive::Mesh(mesh) }];
            painter.paint(&mut canvas, [64, 64], 1.0, &primitives, &delta).unwrap();
            assert!(painter.textures.is_empty());
            assert_eq!(painter.retired.len(), 1);
            canvas.present();
            painter.after_present();
            assert!(painter.retired.is_empty());
            assert!(painter.pending.is_empty());
        }
    }
    #[test]
    fn texture_coordinates_stay_inside_the_range_sdl_accepts() {
        let (x, y) = SdlEguiPainter::uv_for_test(egui::vec2(-1e-7, 1.0000002), egui::vec2(1.0, 1.0));
        assert_eq!((x, y), (0.0, 1.0));
        assert!(x.is_sign_positive(), "a negative zero is still out of bounds for SDL");
        let (x, y) = SdlEguiPainter::uv_for_test(egui::vec2(0.25, 0.78), egui::vec2(1.0, 1.0));
        assert_eq!((x, y), (0.25, 0.78));
    }

    #[test]
    fn test_enqueue_pending_eviction() {
        let mut painter = SdlEguiPainter::default();
        let now = std::time::Instant::now();
        // Insert MAX_PENDING_UPLOADS textures
        for i in 1..=MAX_PENDING_UPLOADS {
            let id = egui::TextureId::User(i as u64);
            painter.enqueue_pending(id, [10, 10], None, &[0; 100], 0, now);
        }
        assert_eq!(painter.pending.len(), MAX_PENDING_UPLOADS);

        // Enqueue an extra texture, which triggers eviction of the largest/victim
        let extra_id = egui::TextureId::User(999);
        painter.enqueue_pending(extra_id, [20, 20], None, &[0; 400], 0, now);
        assert_eq!(painter.pending.len(), MAX_PENDING_UPLOADS);

        // Verifying that removing an evicted ID returns None and does not panic
        let missing = egui::TextureId::User(1);
        let removed = painter.pending.remove(&missing);
        // It could be Some or None depending on which was evicted, but handling None cleanly with ? or match is safe
        let _ = removed;
    }
}

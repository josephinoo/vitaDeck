use crate::runtime::LruCache;
use crate::scanner::ImageBytes;
use std::collections::HashSet;
use std::io::Cursor;
use std::sync::mpsc::{channel, Receiver, Sender};

const MAX_CACHE_BYTES: usize = 10 * 1024 * 1024;

const MAX_DECODE_PENDING: usize = 6;
const MAX_DECODE_RESULTS_PER_FRAME: usize = 1;

#[derive(Clone, Copy)]
enum TextureKind {
    Cover,
    Hero,
    Logo,
    Screenshot,
}

impl TextureKind {
    fn from_key(key: &str) -> Self {
        if key.ends_with(":hero") {
            TextureKind::Hero
        } else if key.ends_with(":logo") {
            TextureKind::Logo
        } else if key.ends_with(":shot") {
            TextureKind::Screenshot
        } else {
            TextureKind::Cover
        }
    }

    fn max_decoded_side(self) -> u32 {
        match self {
            TextureKind::Cover => 256,
            TextureKind::Hero => 960,
            TextureKind::Logo => 320,
            TextureKind::Screenshot => 480,
        }
    }
}

pub struct TextureCache {
    handles: LruCache<String, egui::TextureHandle>,
    failed: HashSet<String>,
    pending: HashSet<String>,
    decode_tx: Sender<(String, TextureKind, ImageBytes)>,
    decode_rx: Receiver<(String, Option<egui::ColorImage>)>,
}

impl Default for TextureCache {
    fn default() -> Self {
        let (req_tx, req_rx) = channel::<(String, TextureKind, ImageBytes)>();
        let (res_tx, res_rx) = channel::<(String, Option<egui::ColorImage>)>();

        std::thread::spawn(move || {
            while let Ok((key, kind, bytes)) = req_rx.recv() {
                let image = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    decode(&bytes, kind)
                }))
                .unwrap_or(None);
                if res_tx.send((key, image)).is_err() {
                    break;
                }
            }
        });

        Self {
            handles: LruCache::new(MAX_CACHE_BYTES),
            failed: HashSet::new(),
            pending: HashSet::new(),
            decode_tx: req_tx,
            decode_rx: res_rx,
        }
    }
}

impl TextureCache {

    pub fn pump(&mut self, ctx: &egui::Context) {
        for _ in 0..MAX_DECODE_RESULTS_PER_FRAME {
            let Ok((key, maybe_image)) = self.decode_rx.try_recv() else { break };
            self.pending.remove(&key);
            match maybe_image {
                Some(image) => {
                    let cost = image.size[0] * image.size[1] * 4;
                    let handle = ctx.load_texture(&key, image, egui::TextureOptions::LINEAR);
                    self.handles.insert(key, handle, cost);
                }
                None => {
                    self.failed.insert(key);
                }
            }
        }
    }

    pub fn get_or_decode(
        &mut self,
        _ctx: &egui::Context,
        key: &str,
        bytes: &ImageBytes,
    ) -> Option<egui::TextureHandle> {
        if let Some(handle) = self.handles.get(key) {
            return Some(handle.clone());
        }
        if self.failed.contains(key) {
            return None;
        }
        if !self.pending.contains(key) && self.pending.len() < MAX_DECODE_PENDING {
            let kind = TextureKind::from_key(key);
            if self.decode_tx.send((key.to_string(), kind, bytes.clone())).is_ok() {
                self.pending.insert(key.to_string());
            }
        }
        None
    }

    pub fn set_pressure_fraction(&mut self, fraction: f32) {
        let budget = ((MAX_CACHE_BYTES as f32) * fraction.clamp(0.05, 1.0)) as usize;
        self.handles.set_max_cost_bytes(budget);
    }

    pub fn bytes_in_use(&self) -> usize {
        self.handles.total_cost_bytes()
    }

    pub fn budget_bytes(&self) -> usize {
        self.handles.max_cost_bytes()
    }

    pub fn invalidate(&mut self, key: &str) {
        self.handles.remove(key);
        self.failed.remove(key);
        self.pending.remove(key);
    }
}

const MAX_SOURCE_SIDE: u32 = 2048;

pub fn source_exceeds_decode_budget(bytes: &[u8]) -> bool {
    peek_image_dimensions(bytes)
        .map(|(w, h)| w > MAX_SOURCE_SIDE || h > MAX_SOURCE_SIDE)
        .unwrap_or(false)
}

fn peek_image_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    let mut reader = image::ImageReader::new(Cursor::new(bytes));
    if bytes.len() >= 8 && &bytes[0..8] == b"\x89PNG\r\n\x1a\n" {
        reader.set_format(image::ImageFormat::Png);
    } else if bytes.len() >= 3 && &bytes[0..3] == b"\xff\xd8\xff" {
        reader.set_format(image::ImageFormat::Jpeg);
    } else {
        reader = reader.with_guessed_format().ok()?;
    }
    reader.into_dimensions().ok()
}

fn decode((is_png, bytes): &ImageBytes, kind: TextureKind) -> Option<egui::ColorImage> {
    if bytes.is_empty() {
        return None;
    }

    let is_png_header = bytes.len() >= 8 && &bytes[0..8] == b"\x89PNG\r\n\x1a\n";
    let is_jpg_header = bytes.len() >= 3 && &bytes[0..3] == b"\xff\xd8\xff";

    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_SOURCE_SIDE);
    limits.max_image_height = Some(MAX_SOURCE_SIDE);

    let mut reader = if is_png_header || *is_png {
        let mut r = image::ImageReader::new(Cursor::new(bytes.as_slice()));
        r.set_format(image::ImageFormat::Png);
        r
    } else if is_jpg_header {
        let mut r = image::ImageReader::new(Cursor::new(bytes.as_slice()));
        r.set_format(image::ImageFormat::Jpeg);
        r
    } else {
        image::ImageReader::new(Cursor::new(bytes.as_slice()))
            .with_guessed_format()
            .unwrap_or_else(|_| image::ImageReader::new(Cursor::new(bytes.as_slice())))
    };
    reader.limits(limits);

    let mut image = reader.decode().ok()?;

    let max_side = kind.max_decoded_side();
    if image.width() > max_side || image.height() > max_side {
        image = image.resize(max_side, max_side, image::imageops::FilterType::Triangle);
    }
    let rgba = image.to_rgba8();
    let size = [rgba.width() as usize, rgba.height() as usize];
    Some(egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw()))
}

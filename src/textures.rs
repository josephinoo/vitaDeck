use crate::scanner::ImageBytes;
use std::collections::{HashMap, HashSet};
use std::io::Cursor;
use std::sync::mpsc::{channel, Receiver, Sender};

const MAX_CACHE_BYTES: usize = 10 * 1024 * 1024;

const MAX_DECODE_PENDING: usize = 6;

struct Entry {
    handle: egui::TextureHandle,
    bytes: usize,
    last_used: u64,
}

pub struct TextureCache {
    handles: HashMap<String, Entry>,
    failed: HashSet<String>,
    pending: HashSet<String>,
    clock: u64,
    bytes_in_use: usize,
    decode_tx: Sender<(String, ImageBytes)>,
    decode_rx: Receiver<(String, Option<egui::ColorImage>)>,
}

impl Default for TextureCache {
    fn default() -> Self {
        let (req_tx, req_rx) = channel::<(String, ImageBytes)>();
        let (res_tx, res_rx) = channel::<(String, Option<egui::ColorImage>)>();

        std::thread::spawn(move || {
            while let Ok((key, bytes)) = req_rx.recv() {
                let image = decode(&bytes);
                let _ = res_tx.send((key, image));
            }
        });

        Self {
            handles: HashMap::new(),
            failed: HashSet::new(),
            pending: HashSet::new(),
            clock: 0,
            bytes_in_use: 0,
            decode_tx: req_tx,
            decode_rx: res_rx,
        }
    }
}

impl TextureCache {

    pub fn pump(&mut self, ctx: &egui::Context) {
        while let Ok((key, maybe_image)) = self.decode_rx.try_recv() {
            self.pending.remove(&key);
            match maybe_image {
                Some(image) => {
                    let cost = image.size[0] * image.size[1] * 4;
                    let handle = ctx.load_texture(&key, image, egui::TextureOptions::LINEAR);
                    self.clock += 1;
                    self.handles.insert(
                        key.clone(),
                        Entry {
                            handle,
                            bytes: cost,
                            last_used: self.clock,
                        },
                    );
                    self.bytes_in_use += cost;
                    self.evict_until_under_budget(&key);
                }
                None => {
                    self.failed.insert(key);
                }
            }
        }
    }

    pub fn get_or_decode(
        &mut self,
        ctx: &egui::Context,
        key: &str,
        bytes: &ImageBytes,
    ) -> Option<egui::TextureHandle> {
        self.pump(ctx);

        self.clock += 1;
        let now = self.clock;

        if let Some(entry) = self.handles.get_mut(key) {
            entry.last_used = now;
            return Some(entry.handle.clone());
        }
        if self.failed.contains(key) {
            return None;
        }
        if !self.pending.contains(key) && self.pending.len() < MAX_DECODE_PENDING {
            self.pending.insert(key.to_string());
            let _ = self.decode_tx.send((key.to_string(), bytes.clone()));
        }
        None
    }

    fn evict_until_under_budget(&mut self, keep: &str) {
        while self.bytes_in_use > MAX_CACHE_BYTES {
            let victim = self
                .handles
                .iter()
                .filter(|(k, _)| k.as_str() != keep)
                .min_by_key(|(_, e)| e.last_used)
                .map(|(k, _)| k.clone());
            let Some(victim) = victim else { break };
            if let Some(entry) = self.handles.remove(&victim) {
                self.bytes_in_use = self.bytes_in_use.saturating_sub(entry.bytes);
            }
        }
    }

    pub fn invalidate(&mut self, key: &str) {
        if let Some(entry) = self.handles.remove(key) {
            self.bytes_in_use = self.bytes_in_use.saturating_sub(entry.bytes);
        }
        self.failed.remove(key);
        self.pending.remove(key);
    }
}

const MAX_DECODED_SIDE: u32 = 640;

const MAX_SOURCE_SIDE: u32 = 1280;

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

fn decode((is_png, bytes): &ImageBytes) -> Option<egui::ColorImage> {
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

    if image.width() > MAX_DECODED_SIDE || image.height() > MAX_DECODED_SIDE {
        image = image.resize(
            MAX_DECODED_SIDE,
            MAX_DECODED_SIDE,
            image::imageops::FilterType::Triangle,
        );
    }
    let rgba = image.to_rgba8();
    let size = [rgba.width() as usize, rgba.height() as usize];
    Some(egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw()))
}

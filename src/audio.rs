#![allow(dead_code, unused_imports)]

use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Condvar, Mutex};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SoundEffect {
    Navigate,
    Confirm,
    TabSwitch,
    OpenModal,
    CloseModal,
    LaunchGame,
}

#[derive(Clone)]
pub struct WavSound {
    pub sample_rate: u32,
    pub channels: u16,
    pub samples: Vec<i16>,
}

impl WavSound {
    pub fn parse(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 44 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
            return None;
        }
        let mut pos = 12;
        let mut fmt_found = false;
        let mut channels = 2u16;
        let mut sample_rate = 44100u32;

        while pos + 8 <= bytes.len() {
            let chunk_id = &bytes[pos..pos + 4];
            let chunk_size = u32::from_le_bytes(bytes[pos + 4..pos + 8].try_into().ok()?) as usize;
            pos += 8;

            if chunk_id == b"fmt " && chunk_size >= 16 && pos + chunk_size <= bytes.len() {
                channels = u16::from_le_bytes(bytes[pos + 2..pos + 4].try_into().ok()?);
                sample_rate = u32::from_le_bytes(bytes[pos + 4..pos + 8].try_into().ok()?);
                fmt_found = true;
            } else if chunk_id == b"data" && pos + chunk_size <= bytes.len() {
                if !fmt_found {
                    return None;
                }
                let pcm_bytes = &bytes[pos..pos + chunk_size];
                let mut samples = Vec::with_capacity(pcm_bytes.len() / 2);
                for chunk in pcm_bytes.chunks_exact(2) {
                    samples.push(i16::from_le_bytes([chunk[0], chunk[1]]));
                }
                return Some(WavSound {
                    sample_rate,
                    channels,
                    samples,
                });
            }
            pos += chunk_size;
        }
        None
    }

    fn to_mono_48000(&self) -> WavSound {
        let channels = self.channels.max(1) as usize;
        let mono: Vec<i16> = self
            .samples
            .chunks(channels)
            .map(|frame| {
                let sum: i32 = frame.iter().map(|&s| s as i32).sum();
                (sum / channels as i32) as i16
            })
            .collect();

        if self.sample_rate == SFX_SAMPLE_RATE {
            return WavSound {
                sample_rate: SFX_SAMPLE_RATE,
                channels: 1,
                samples: mono,
            };
        }

        let ratio = SFX_SAMPLE_RATE as f64 / self.sample_rate as f64;
        let out_len = ((mono.len() as f64) * ratio).round() as usize;
        let mut resampled = Vec::with_capacity(out_len);
        for i in 0..out_len {
            let src_pos = i as f64 / ratio;
            let idx = src_pos as usize;
            let frac = src_pos - idx as f64;
            let a = mono.get(idx).copied().unwrap_or(0) as f64;
            let b = mono.get(idx + 1).copied().unwrap_or(a as i16) as f64;
            resampled.push((a + (b - a) * frac) as i16);
        }
        WavSound {
            sample_rate: SFX_SAMPLE_RATE,
            channels: 1,
            samples: resampled,
        }
    }
}

const SFX_SAMPLE_RATE: u32 = 48000;
const SFX_GRAIN_SIZE: usize = 512;

#[derive(Debug)]
pub enum MusicCmd {
    Play(String),
    Stop,
}

pub struct AudioEngine {
    tx: mpsc::Sender<SoundEffect>,
    music_tx: mpsc::Sender<MusicCmd>,
    music_path: Option<String>,
}

impl AudioEngine {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel::<SoundEffect>();
        std::thread::spawn(move || {
            let mut sounds = HashMap::new();
            load_sound(&mut sounds, SoundEffect::Navigate, "deck_ui_navigation.wav");
            load_sound(&mut sounds, SoundEffect::Confirm, "deck_ui_default_activation.wav");
            load_sound(&mut sounds, SoundEffect::TabSwitch, "deck_ui_tab_transition_01.wav");
            load_sound(&mut sounds, SoundEffect::OpenModal, "deck_ui_show_modal.wav");
            load_sound(&mut sounds, SoundEffect::CloseModal, "deck_ui_hide_modal.wav");
            load_sound(&mut sounds, SoundEffect::LaunchGame, "deck_ui_launch_game.wav");

            let sounds: HashMap<SoundEffect, WavSound> =
                sounds.into_iter().map(|(k, v)| (k, v.to_mono_48000())).collect();

            let port = open_sfx_port();

            while let Ok(mut effect) = rx.recv() {
                while let Ok(next) = rx.try_recv() {
                    effect = next;
                }
                if let Some(sound) = sounds.get(&effect) {
                    play_sound_on_hardware(port, sound);
                }
            }
        });

        let (music_tx, music_rx) = mpsc::channel::<MusicCmd>();
        start_music_subsystem(music_rx);

        AudioEngine {
            tx,
            music_tx,
            music_path: None,
        }
    }

    pub fn play(&self, effect: SoundEffect) {
        let _ = self.tx.send(effect);
    }

    pub fn set_music(&mut self, path: Option<&str>) {
        if self.music_path.as_deref() == path {
            return;
        }
        self.music_path = path.map(str::to_string);
        let _ = match path {
            Some(p) => self.music_tx.send(MusicCmd::Play(p.to_string())),
            None => self.music_tx.send(MusicCmd::Stop),
        };
    }
}

fn load_sound(map: &mut HashMap<SoundEffect, WavSound>, effect: SoundEffect, filename: &str) {
    let paths = [
        format!("app0:sounds/{}", filename),
        format!("assets/sounds/{}", filename),
        format!("ux0:app/VITADECK1/sounds/{}", filename),
    ];
    for path in &paths {
        if let Ok(mut file) = File::open(path) {
            let mut data = Vec::new();
            if file.read_to_end(&mut data).is_ok() {
                if let Some(sound) = WavSound::parse(&data) {
                    map.insert(effect, sound);
                    return;
                }
            }
        }
    }
    crate::logger::log(&format!("audio: failed to load sound {filename}"));
}

#[cfg(target_os = "vita")]
fn open_sfx_port() -> i32 {
    use vitasdk_sys::*;
    unsafe {
        let port = sceAudioOutOpenPort(
            SCE_AUDIO_OUT_PORT_TYPE_MAIN,
            SFX_GRAIN_SIZE as i32,
            SFX_SAMPLE_RATE as i32,
            SCE_AUDIO_OUT_MODE_MONO as u32,
        );
        if port < 0 {
            crate::logger::log(&format!("audio: sceAudioOutOpenPort (SFX) failed (0x{:08X})", port as u32));
        }
        port
    }
}

#[cfg(not(target_os = "vita"))]
fn open_sfx_port() -> i32 {
    0
}

#[cfg(target_os = "vita")]
fn play_sound_on_hardware(port: i32, sound: &WavSound) {
    use vitasdk_sys::*;
    if port < 0 {
        return;
    }
    unsafe {
        for chunk in sound.samples.chunks(SFX_GRAIN_SIZE) {
            if chunk.len() == SFX_GRAIN_SIZE {
                sceAudioOutOutput(port, chunk.as_ptr() as *const _);
            } else {
                let mut padded = [0i16; SFX_GRAIN_SIZE];
                padded[..chunk.len()].copy_from_slice(chunk);
                sceAudioOutOutput(port, padded.as_ptr() as *const _);
            }
        }
    }
}

#[cfg(not(target_os = "vita"))]
fn play_sound_on_hardware(_port: i32, _sound: &WavSound) {}

pub struct PcmRingBuffer {
    buffer: Vec<i16>,
    capacity: usize,
    read_pos: usize,
    write_pos: usize,
    count: usize,
}

impl PcmRingBuffer {
    pub fn new(capacity: usize) -> Self {
        Self {
            buffer: vec![0i16; capacity],
            capacity,
            read_pos: 0,
            write_pos: 0,
            count: 0,
        }
    }

    pub fn clear(&mut self) {
        self.read_pos = 0;
        self.write_pos = 0;
        self.count = 0;
    }

    pub fn available_write(&self) -> usize {
        self.capacity - self.count
    }

    pub fn available_read(&self) -> usize {
        self.count
    }

    pub fn write_samples(&mut self, samples: &[i16]) -> usize {
        let to_write = samples.len().min(self.capacity - self.count);
        if to_write == 0 {
            return 0;
        }

        let first_chunk = to_write.min(self.capacity - self.write_pos);
        self.buffer[self.write_pos..self.write_pos + first_chunk].copy_from_slice(&samples[..first_chunk]);

        let second_chunk = to_write - first_chunk;
        if second_chunk > 0 {
            self.buffer[..second_chunk].copy_from_slice(&samples[first_chunk..to_write]);
        }

        self.write_pos = (self.write_pos + to_write) % self.capacity;
        self.count += to_write;
        to_write
    }

    pub fn read_samples(&mut self, out: &mut [i16]) -> usize {
        let to_read = out.len().min(self.count);
        if to_read == 0 {
            return 0;
        }

        let first_chunk = to_read.min(self.capacity - self.read_pos);
        out[..first_chunk].copy_from_slice(&self.buffer[self.read_pos..self.read_pos + first_chunk]);

        let second_chunk = to_read - first_chunk;
        if second_chunk > 0 {
            out[first_chunk..to_read].copy_from_slice(&self.buffer[..second_chunk]);
        }

        self.read_pos = (self.read_pos + to_read) % self.capacity;
        self.count -= to_read;
        to_read
    }
}

pub struct SharedAudioQueue {
    pub ring: Mutex<PcmRingBuffer>,
    pub cond_read: Condvar,
    pub cond_write: Condvar,
    pub playing: AtomicBool,
    pub flush_requested: AtomicBool,
    pub sample_rate: std::sync::atomic::AtomicU32,
}

impl SharedAudioQueue {
    pub fn new(capacity: usize) -> Self {
        Self {
            ring: Mutex::new(PcmRingBuffer::new(capacity)),
            cond_read: Condvar::new(),
            cond_write: Condvar::new(),
            playing: AtomicBool::new(false),
            flush_requested: AtomicBool::new(false),
            sample_rate: std::sync::atomic::AtomicU32::new(44100),
        }
    }
}

struct AlignedBuf<T> {
    storage: Vec<T>,
    offset: usize,
    len: usize,
}

impl<T: Copy + Default> AlignedBuf<T> {
    fn new(len: usize, align_bytes: usize) -> Self {
        let align_elems = (align_bytes / std::mem::size_of::<T>()).max(1);
        let storage = vec![T::default(); len + align_elems];
        let addr = storage.as_ptr() as usize;
        let aligned = (addr + align_bytes - 1) & !(align_bytes - 1);
        let offset = (aligned - addr) / std::mem::size_of::<T>();
        Self {
            storage,
            offset,
            len,
        }
    }

    fn as_mut_ptr(&mut self) -> *mut T {
        unsafe { self.storage.as_mut_ptr().add(self.offset) }
    }

    fn as_mut_slice(&mut self) -> &mut [T] {
        &mut self.storage[self.offset..self.offset + self.len]
    }

    fn as_slice(&self) -> &[T] {
        &self.storage[self.offset..self.offset + self.len]
    }
}

#[cfg(target_os = "vita")]
fn skip_id3_tag(file: &mut File) {
    use std::io::{Seek, SeekFrom};
    let mut header = [0u8; 10];
    let start = match file.read(&mut header) {
        Ok(n) if n == 10 => crate::mp3::id3_tag_len(&header).unwrap_or(0),
        _ => 0,
    };
    let _ = file.seek(SeekFrom::Start(start));
}

const BGM_OUT_SAMPLES_PER_CH: usize = 1024;
const BGM_OUT_CHANNELS: usize = 2;
const BGM_OUT_CHUNK_SAMPLES: usize = BGM_OUT_SAMPLES_PER_CH * BGM_OUT_CHANNELS;
const PCM_RING_CAPACITY_SAMPLES: usize = 65536;
const MUSIC_READ_CHUNK: usize = 16 * 1024;
const MUSIC_WINDOW_TARGET: usize = 64 * 1024;

#[cfg(target_os = "vita")]
fn start_music_subsystem(music_rx: mpsc::Receiver<MusicCmd>) {
    let queue = Arc::new(SharedAudioQueue::new(PCM_RING_CAPACITY_SAMPLES));
    let playback_queue = Arc::clone(&queue);

    std::thread::spawn(move || {
        audio_output_thread_main(playback_queue);
    });

    std::thread::spawn(move || {
        audio_decoder_thread_main(music_rx, queue);
    });
}

#[cfg(not(target_os = "vita"))]
fn start_music_subsystem(music_rx: mpsc::Receiver<MusicCmd>) {
    std::thread::spawn(move || {
        while music_rx.recv().is_ok() {}
    });
}

#[cfg(target_os = "vita")]
fn audio_output_thread_main(queue: Arc<SharedAudioQueue>) {
    use vitasdk_sys::*;

    crate::logger::log("audio: audio output thread started");

    let mut current_sample_rate: u32 = 44100;
    let mut port: i32 = -1;

    let mut out_buffer = [0i16; BGM_OUT_CHUNK_SAMPLES];
    let mut logged_underrun = false;

    loop {
        if queue.flush_requested.swap(false, Ordering::SeqCst) {
            if let Ok(mut ring) = queue.ring.lock() {
                ring.clear();
                queue.cond_write.notify_all();
            }
        }

        let is_playing = queue.playing.load(Ordering::SeqCst);
        let requested_sample_rate = queue.sample_rate.load(Ordering::SeqCst);

        if is_playing && port >= 0 && requested_sample_rate > 0 && requested_sample_rate != current_sample_rate {
            unsafe { sceAudioOutReleasePort(port) };
            port = -1;
            crate::logger::log(&format!(
                "audio: sample rate changed from {}Hz to {}Hz, recreating port",
                current_sample_rate, requested_sample_rate
            ));
            current_sample_rate = requested_sample_rate;
        } else if requested_sample_rate > 0 {
            current_sample_rate = requested_sample_rate;
        }

        if !is_playing {
            if port >= 0 {
                unsafe { sceAudioOutReleasePort(port) };
                port = -1;
                crate::logger::log("audio: audio port closed (stopped)");
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
            continue;
        }

        if port < 0 {
            unsafe {
                port = sceAudioOutOpenPort(
                    SCE_AUDIO_OUT_PORT_TYPE_BGM,
                    BGM_OUT_SAMPLES_PER_CH as i32,
                    current_sample_rate as i32,
                    SCE_AUDIO_OUT_MODE_STEREO as u32,
                );
                if port < 0 {
                    crate::logger::log(&format!(
                        "audio: sceAudioOutOpenPort(BGM, {}Hz) failed: 0x{:08X}; trying MAIN fallback",
                        current_sample_rate, port as u32
                    ));
                    port = sceAudioOutOpenPort(
                        SCE_AUDIO_OUT_PORT_TYPE_MAIN,
                        BGM_OUT_SAMPLES_PER_CH as i32,
                        current_sample_rate as i32,
                        SCE_AUDIO_OUT_MODE_STEREO as u32,
                    );
                }
                if port >= 0 {
                    let mut vol = [SCE_AUDIO_OUT_MAX_VOL as i32; 2];
                    sceAudioOutSetVolume(
                        port,
                        SCE_AUDIO_VOLUME_FLAG_L_CH | SCE_AUDIO_VOLUME_FLAG_R_CH,
                        vol.as_mut_ptr(),
                    );
                    crate::logger::log(&format!(
                        "audio: audio port opened successfully (port={}, freq={}Hz, len={})",
                        port, current_sample_rate, BGM_OUT_SAMPLES_PER_CH
                    ));
                } else {
                    crate::logger::log(&format!(
                        "audio: fatal - failed to open any audio port: 0x{:08X}",
                        port as u32
                    ));
                    std::thread::sleep(std::time::Duration::from_millis(100));
                    continue;
                }
            }
        }

        let read_count = if let Ok(mut ring) = queue.ring.lock() {
            let count = ring.read_samples(&mut out_buffer);
            if count < BGM_OUT_CHUNK_SAMPLES {
                out_buffer[count..].fill(0);
                if is_playing && !logged_underrun && count == 0 {
                    crate::logger::log(&format!(
                        "audio: buffer underrun (avail={})",
                        ring.available_read()
                    ));
                    logged_underrun = true;
                }
            } else {
                logged_underrun = false;
            }
            if ring.available_write() >= (BGM_OUT_CHUNK_SAMPLES * 2) {
                queue.cond_write.notify_all();
            }
            count
        } else {
            0
        };
        let _ = read_count;

        if port >= 0 {
            unsafe {
                let out_rc = sceAudioOutOutput(port, out_buffer.as_ptr() as *const _);
                if out_rc < 0 {
                    crate::logger::log(&format!(
                        "audio: sceAudioOutOutput failed: 0x{:08X}",
                        out_rc as u32
                    ));
                }
            }
        }
    }
}

#[cfg(target_os = "vita")]
fn audio_decoder_thread_main(rx: mpsc::Receiver<MusicCmd>, queue: Arc<SharedAudioQueue>) {
    use std::mem::size_of;
    use vitasdk_sys::*;

    crate::logger::log("audio: audio decoder thread started");

    unsafe {
        let load = sceSysmoduleLoadModule(SCE_SYSMODULE_AUDIOCODEC);
        if load < 0 {
            crate::logger::log(&format!(
                "audio: sceSysmoduleLoadModule(AUDIOCODEC) returned 0x{:08X}",
                load as u32
            ));
        }

        let mut init_param: SceAudiodecInitParam = std::mem::zeroed();
        init_param.mp3 = SceAudiodecInitStreamParam {
            size: size_of::<SceAudiodecInitStreamParam>() as u32,
            totalStreams: SCE_AUDIODEC_MP3_MAX_STREAMS,
        };
        let init_rc = sceAudiodecInitLibrary(SCE_AUDIODEC_TYPE_MP3, &mut init_param);
        if init_rc < 0 && (init_rc as u32) != 0x807F0003 {
            crate::logger::log(&format!(
                "audio: sceAudiodecInitLibrary failed: 0x{:08X} (fatal)",
                init_rc as u32
            ));
            while rx.recv().is_ok() {}
            return;
        } else if (init_rc as u32) == 0x807F0003 {
            crate::logger::log("audio: sceAudiodecInitLibrary returned ALREADY_INITIALIZED (0x807F0003)");
        } else {
            crate::logger::log("audio: sceAudiodecInitLibrary initialized successfully");
        }

        let align = SCE_AUDIODEC_ALIGNMENT_SIZE as usize;
        let mut es_buf = AlignedBuf::<u8>::new(SCE_AUDIODEC_MP3_MAX_ES_SIZE as usize, align);
        let pcm_len = (SCE_AUDIODEC_MP3_MAX_SAMPLES * SCE_AUDIODEC_MP3_MAX_CH_IN_DECODER) as usize;
        let mut pcm_buf = AlignedBuf::<i16>::new(pcm_len, align);
        let mut current_track: Option<String> = None;
        let mut stereo_converter = vec![0i16; pcm_len * 2];

        loop {
            let path = match current_track.take() {
                Some(p) => p,
                None => match rx.recv() {
                    Ok(MusicCmd::Play(p)) => p,
                    Ok(MusicCmd::Stop) => {
                        queue.playing.store(false, Ordering::SeqCst);
                        queue.flush_requested.store(true, Ordering::SeqCst);
                        continue;
                    }
                    Err(_) => break,
                },
            };

            crate::logger::log(&format!("audio: requested track: {}", path));
            queue.flush_requested.store(true, Ordering::SeqCst);
            queue.playing.store(true, Ordering::SeqCst);

            let outcome = decode_and_feed_mp3(
                &path,
                &rx,
                &queue,
                &mut es_buf,
                &mut pcm_buf,
                &mut stereo_converter,
            );

            match outcome {
                DecodeOutcome::PlayNext(next) => {
                    current_track = Some(next);
                }
                DecodeOutcome::LoopSame => {
                    current_track = Some(path);
                }
                DecodeOutcome::Stopped => {
                    queue.playing.store(false, Ordering::SeqCst);
                    current_track = None;
                }
            }
        }

        sceAudiodecTermLibrary(SCE_AUDIODEC_TYPE_MP3);
    }
}

enum DecodeOutcome {
    PlayNext(String),
    LoopSame,
    Stopped,
}

#[cfg(target_os = "vita")]
fn decode_and_feed_mp3(
    path: &str,
    rx: &mpsc::Receiver<MusicCmd>,
    queue: &Arc<SharedAudioQueue>,
    es_buf: &mut AlignedBuf<u8>,
    pcm_buf: &mut AlignedBuf<i16>,
    stereo_converter: &mut [i16],
) -> DecodeOutcome {
    use std::mem::size_of;
    use vitasdk_sys::*;

    let Ok(mut file) = File::open(path) else {
        crate::logger::log(&format!("audio: failed to open file: {}", path));
        return DecodeOutcome::Stopped;
    };
    skip_id3_tag(&mut file);

    let mut window: Vec<u8> = Vec::with_capacity(MUSIC_WINDOW_TARGET + MUSIC_READ_CHUNK);
    let mut read_buf = [0u8; MUSIC_READ_CHUNK];
    let mut eof = false;
    let mut produced_audio = false;
    let mut logged_header = false;
    let mut ctrl: Option<(SceAudiodecCtrl, Box<SceAudiodecInfo>)> = None;
    let mut decoder_created = false;

    let outcome = 'stream: loop {
        match rx.try_recv() {
            Ok(MusicCmd::Play(next)) => break 'stream DecodeOutcome::PlayNext(next),
            Ok(MusicCmd::Stop) => break 'stream DecodeOutcome::Stopped,
            Err(_) => {}
        }

        if !eof && window.len() < MUSIC_WINDOW_TARGET {
            match file.read(&mut read_buf) {
                Ok(0) => eof = true,
                Ok(n) => window.extend_from_slice(&read_buf[..n]),
                Err(_) => eof = true,
            }
        }

        let Some(offset) = crate::mp3::find_next_frame(&window, 0) else {
            if eof {
                break 'stream if produced_audio {
                    DecodeOutcome::LoopSame
                } else {
                    DecodeOutcome::Stopped
                };
            }
            if window.len() > 16 {
                let keep = 16.min(window.len());
                window.drain(0..window.len() - keep);
            }
            continue;
        };

        let Some(frame) = crate::mp3::parse_frame_header(&window[offset..]) else {
            window.drain(0..offset + 1);
            continue;
        };

        if !logged_header {
            queue.sample_rate.store(frame.sample_rate, Ordering::SeqCst);
            crate::logger::log(&format!(
                "audio: MP3 header parsed - rate={}Hz ch={} ver={} frame_len={} path={}",
                frame.sample_rate, frame.channels, frame.version, frame.len, path
            ));
            logged_header = true;
        }

        if !decoder_created {
            let mut info: Box<SceAudiodecInfo> = Box::new(unsafe { std::mem::zeroed() });
            info.mp3.size = size_of::<SceAudiodecInfoMp3>() as u32;
            info.mp3.ch = frame.channels;
            info.mp3.version = frame.version;

            let mut c: SceAudiodecCtrl = unsafe { std::mem::zeroed() };
            c.size = size_of::<SceAudiodecCtrl>() as u32;
            c.wordLength = SCE_AUDIODEC_WORD_LENGTH_16BITS;
            c.pEs = es_buf.as_mut_ptr();
            c.maxEsSize = es_buf.len as u32;
            c.pPcm = pcm_buf.as_mut_ptr() as *mut _;
            c.maxPcmSize = (pcm_buf.len * 2) as u32;
            c.pInfo = info.as_mut() as *mut SceAudiodecInfo;

            let create_rc = unsafe { sceAudiodecCreateDecoder(&mut c, SCE_AUDIODEC_TYPE_MP3) };
            if create_rc < 0 {
                crate::logger::log(&format!(
                    "audio: sceAudiodecCreateDecoder failed: 0x{:08X} for {} (ch={}, ver={})",
                    create_rc as u32, path, frame.channels, frame.version
                ));
                break 'stream DecodeOutcome::Stopped;
            }
            ctrl = Some((c, info));
            decoder_created = true;
        }

        if window.len() < offset + frame.len {
            if eof {
                break 'stream if produced_audio {
                    DecodeOutcome::LoopSame
                } else {
                    DecodeOutcome::Stopped
                };
            }
            match file.read(&mut read_buf) {
                Ok(0) => eof = true,
                Ok(n) => window.extend_from_slice(&read_buf[..n]),
                Err(_) => eof = true,
            }
            continue;
        }

        let frame_bytes = &window[offset..offset + frame.len];
        if let Some((c, info)) = ctrl.as_mut() {
            unsafe {
                es_buf.as_mut_slice()[..frame_bytes.len()].copy_from_slice(frame_bytes);

                info.mp3.size = size_of::<SceAudiodecInfoMp3>() as u32;
                info.mp3.ch = frame.channels;
                info.mp3.version = frame.version;

                c.pEs = es_buf.as_mut_ptr();
                c.inputEsSize = frame.len as u32;
                c.maxEsSize = es_buf.len as u32;
                c.pPcm = pcm_buf.as_mut_ptr() as *mut _;
                c.maxPcmSize = (pcm_buf.len * 2) as u32;
                c.pInfo = info.as_mut() as *mut SceAudiodecInfo;

                let decode_rc = sceAudiodecDecode(c);
                if decode_rc >= 0 && c.outputPcmSize > 0 {
                    let sample_count = (c.outputPcmSize / 2) as usize;
                    let raw_samples = &pcm_buf.as_slice()[..sample_count];

                    let stereo_slice: &[i16] = if frame.channels == 1 {
                        let mut out_idx = 0;
                        for &s in raw_samples {
                            stereo_converter[out_idx] = s;
                            stereo_converter[out_idx + 1] = s;
                            out_idx += 2;
                        }
                        &stereo_converter[..sample_count * 2]
                    } else {
                        raw_samples
                    };

                    let mut written_total = 0;
                    while written_total < stereo_slice.len() {
                        match rx.try_recv() {
                            Ok(MusicCmd::Play(next)) => break 'stream DecodeOutcome::PlayNext(next),
                            Ok(MusicCmd::Stop) => break 'stream DecodeOutcome::Stopped,
                            Err(_) => {}
                        }

                        if let Ok(mut ring) = queue.ring.lock() {
                            let written = ring.write_samples(&stereo_slice[written_total..]);
                            written_total += written;
                            if written > 0 {
                                queue.cond_read.notify_one();
                            }
                        }

                        if written_total < stereo_slice.len() {
                            std::thread::sleep(std::time::Duration::from_millis(5));
                        }
                    }
                    produced_audio = true;
                } else if decode_rc < 0 {
                    crate::logger::log(&format!(
                        "audio: sceAudiodecDecode error: 0x{:08X} in {}",
                        decode_rc as u32, path
                    ));
                }
            }
        }

        window.drain(0..offset + frame.len);
    };

    if let Some((mut c, _)) = ctrl {
        unsafe {
            let del_rc = sceAudiodecDeleteDecoder(&mut c);
            if del_rc < 0 {
                crate::logger::log(&format!("audio: sceAudiodecDeleteDecoder error: 0x{:08X}", del_rc as u32));
            }
        };
    }

    outcome
}

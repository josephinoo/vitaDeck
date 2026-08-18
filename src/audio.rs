use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::sync::mpsc;

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
            return WavSound { sample_rate: SFX_SAMPLE_RATE, channels: 1, samples: mono };
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
        WavSound { sample_rate: SFX_SAMPLE_RATE, channels: 1, samples: resampled }
    }
}

enum MusicCmd {
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
        std::thread::spawn(move || music_thread_main(music_rx));

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
    crate::scanner::write_append_log(&format!("audio: failed to load sound {filename}"));
}

const SFX_SAMPLE_RATE: u32 = 48000;
const SFX_GRAIN_SIZE: usize = 512;

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
            crate::scanner::write_append_log(&format!("audio: sceAudioOutOpenPort failed ({port})"));
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
                let mut padded = vec![0i16; SFX_GRAIN_SIZE];
                padded[..chunk.len()].copy_from_slice(chunk);
                sceAudioOutOutput(port, padded.as_ptr() as *const _);
            }
        }
    }
}

#[cfg(not(target_os = "vita"))]
fn play_sound_on_hardware(_port: i32, _sound: &WavSound) {

}

const MUSIC_READ_CHUNK: usize = 32 * 1024;
const MUSIC_WINDOW_TARGET: usize = 96 * 1024;

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

    fn as_ptr(&self) -> *const T {
        unsafe { self.storage.as_ptr().add(self.offset) }
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

enum TrackOutcome {
    PlayNext(String),
    Idle,
    LoopSame,
}

#[cfg(target_os = "vita")]
fn music_thread_main(rx: mpsc::Receiver<MusicCmd>) {
    use std::mem::size_of;
    use vitasdk_sys::*;

    unsafe {
        let load = sceSysmoduleLoadModule(SCE_SYSMODULE_AUDIOCODEC);
        if load < 0 {
            crate::scanner::write_append_log(&format!(
                "audio: sceSysmoduleLoadModule(AUDIOCODEC) failed: 0x{:08X}",
                load as u32
            ));
        }

        let mut init_param: SceAudiodecInitParam = std::mem::zeroed();
        init_param.mp3 = SceAudiodecInitStreamParam {
            size: size_of::<SceAudiodecInitStreamParam>() as u32,
            totalStreams: 1,
        };
        let init_rc = sceAudiodecInitLibrary(SCE_AUDIODEC_TYPE_MP3, &mut init_param);
        if init_rc < 0 && (init_rc as u32) != 0x807F0003 {
            crate::scanner::write_append_log(&format!(
                "audio: sceAudiodecInitLibrary failed: 0x{:08X} (fatal, no music this session)",
                init_rc as u32
            ));
            while rx.recv().is_ok() {}
            return;
        } else if (init_rc as u32) == 0x807F0003 {
            crate::scanner::write_append_log(
                "audio: sceAudiodecInitLibrary returned ALREADY_INITIALIZED; continuing with existing state",
            );
        }

        let align = SCE_AUDIODEC_ALIGNMENT_SIZE as usize;
        let mut es_buf = AlignedBuf::<u8>::new(SCE_AUDIODEC_MP3_MAX_ES_SIZE as usize, align);
        let pcm_len = (SCE_AUDIODEC_MP3_MAX_SAMPLES * SCE_AUDIODEC_MP3_MAX_CH_IN_DECODER) as usize;
        let mut pcm_buf = AlignedBuf::<i16>::new(pcm_len, align);
        let mut info: SceAudiodecInfo = std::mem::zeroed();
        info.mp3.size = size_of::<SceAudiodecInfoMp3>() as u32;
        info.mp3.ch = 2;
        info.mp3.version = SCE_AUDIODEC_MP3_MPEG_VERSION_1 as u32;

        let mut ctrl: SceAudiodecCtrl = std::mem::zeroed();
        ctrl.size = size_of::<SceAudiodecCtrl>() as u32;
        ctrl.wordLength = SCE_AUDIODEC_WORD_LENGTH_16BITS;
        ctrl.pEs = es_buf.as_mut_ptr();
        ctrl.maxEsSize = es_buf.len as u32;
        ctrl.pPcm = pcm_buf.as_mut_ptr() as *mut _;
        ctrl.maxPcmSize = (pcm_buf.len * 2) as u32;
        ctrl.pInfo = &mut info;

        let create_rc = sceAudiodecCreateDecoder(&mut ctrl, SCE_AUDIODEC_TYPE_MP3);
        if create_rc < 0 {
            crate::scanner::write_append_log(&format!(
                "audio: sceAudiodecCreateDecoder failed: 0x{:08X} (fatal, no music this session)",
                create_rc as u32
            ));
            sceAudiodecTermLibrary(SCE_AUDIODEC_TYPE_MP3);
            while rx.recv().is_ok() {}
            return;
        }

        let mut current: Option<String> = None;
        loop {
            let path = match current.take() {
                Some(p) => p,
                None => match rx.recv() {
                    Ok(MusicCmd::Play(p)) => p,
                    Ok(MusicCmd::Stop) | Err(_) => continue,
                },
            };

            match decode_track(&path, &rx, &mut ctrl, &mut es_buf, &mut pcm_buf, align) {
                TrackOutcome::PlayNext(p) => current = Some(p),
                TrackOutcome::Idle => current = None,
                TrackOutcome::LoopSame => current = Some(path),
            }
        }
    }
}

#[cfg(target_os = "vita")]
fn decode_track(
    path: &str,
    rx: &mpsc::Receiver<MusicCmd>,
    ctrl: &mut vitasdk_sys::SceAudiodecCtrl,
    es_buf: &mut AlignedBuf<u8>,
    pcm_buf: &mut AlignedBuf<i16>,
    align: usize,
) -> TrackOutcome {
    use std::mem::size_of;
    use vitasdk_sys::*;

    fn check_interrupt(rx: &mpsc::Receiver<MusicCmd>) -> Option<TrackOutcome> {
        match rx.try_recv() {
            Ok(MusicCmd::Play(p)) => Some(TrackOutcome::PlayNext(p)),
            Ok(MusicCmd::Stop) => Some(TrackOutcome::Idle),
            Err(_) => None,
        }
    }

    let Ok(mut file) = File::open(path) else {
        crate::scanner::write_append_log(&format!("audio: failed to open {}", path));
        return TrackOutcome::Idle;
    };
    unsafe { skip_id3_tag(&mut file) };

    unsafe {
        let clear_rc = sceAudiodecClearContext(ctrl);
        if clear_rc < 0 {
            crate::scanner::write_append_log(&format!(
                "audio: sceAudiodecClearContext failed: 0x{:08X} path={} (continuing anyway)",
                clear_rc as u32, path
            ));
        }
    }

    let mut window: Vec<u8> = Vec::new();
    let mut read_buf = vec![0u8; MUSIC_READ_CHUNK];
    let mut eof = false;
    let mut pcm_accumulator: AlignedBuf<i16> = AlignedBuf::new(8192, align);
    let mut accum_len: usize = 0;
    let mut port: i32 = -1;
    let mut logged_decode_fail = false;
    let mut produced_audio = false;

    const GRAIN_SIZE: usize = 1024;

    let outcome = 'decode: loop {
        if let Some(outcome) = check_interrupt(rx) {
            break 'decode outcome;
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
                break 'decode if produced_audio { TrackOutcome::LoopSame } else { TrackOutcome::Idle };
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
        if window.len() < offset + frame.len {
            if eof {
                break 'decode if produced_audio { TrackOutcome::LoopSame } else { TrackOutcome::Idle };
            }
            match file.read(&mut read_buf) {
                Ok(0) => eof = true,
                Ok(n) => window.extend_from_slice(&read_buf[..n]),
                Err(_) => eof = true,
            }
            continue;
        }

        let frame_bytes = &window[offset..offset + frame.len];
        unsafe {
            es_buf.as_mut_slice()[..frame_bytes.len()].copy_from_slice(frame_bytes);

            let mut info: SceAudiodecInfo = std::mem::zeroed();
            info.mp3.size = size_of::<SceAudiodecInfoMp3>() as u32;
            info.mp3.ch = frame.channels;
            info.mp3.version = frame.version;
            ctrl.pEs = es_buf.as_mut_ptr();
            ctrl.inputEsSize = frame.len as u32;
            ctrl.maxEsSize = es_buf.len as u32;
            ctrl.pPcm = pcm_buf.as_mut_ptr() as *mut _;
            ctrl.maxPcmSize = (pcm_buf.len * 2) as u32;
            ctrl.pInfo = &mut info;

            let decode_rc = sceAudiodecDecode(ctrl);
            if decode_rc >= 0 && ctrl.outputPcmSize > 0 {
                let total_samples = (ctrl.outputPcmSize / 2) as usize;
                let channels = info.mp3.ch.max(1) as usize;
                let pcm_slice = &pcm_buf.as_slice()[..total_samples];

                if accum_len + total_samples > pcm_accumulator.len {
                    let mut bigger = AlignedBuf::<i16>::new(accum_len + total_samples + 4096, align);
                    bigger.as_mut_slice()[..accum_len].copy_from_slice(&pcm_accumulator.as_slice()[..accum_len]);
                    pcm_accumulator = bigger;
                }
                pcm_accumulator.as_mut_slice()[accum_len..accum_len + total_samples].copy_from_slice(pcm_slice);
                accum_len += total_samples;
                produced_audio = true;

                if port < 0 {
                    let mode = if channels == 1 { SCE_AUDIO_OUT_MODE_MONO } else { SCE_AUDIO_OUT_MODE_STEREO };
                    port = sceAudioOutOpenPort(SCE_AUDIO_OUT_PORT_TYPE_BGM, GRAIN_SIZE as i32, frame.sample_rate as i32, mode as u32);
                    if port < 0 {
                        crate::scanner::write_append_log(&format!(
                            "audio: sceAudioOutOpenPort(BGM) failed: 0x{:08X} path={} (trying MAIN fallback)",
                            port as u32, path
                        ));
                        port = sceAudioOutOpenPort(
                            SCE_AUDIO_OUT_PORT_TYPE_MAIN,
                            GRAIN_SIZE as i32,
                            frame.sample_rate as i32,
                            mode as u32,
                        );
                        if port < 0 {
                            crate::scanner::write_append_log(&format!(
                                "audio: sceAudioOutOpenPort(MAIN fallback) failed: 0x{:08X} path={}",
                                port as u32, path
                            ));
                            break 'decode TrackOutcome::Idle;
                        }
                    }
                }

                let chunk_size = GRAIN_SIZE * channels;
                while accum_len >= chunk_size {
                    if port >= 0 {
                        let out_rc = sceAudioOutOutput(port, pcm_accumulator.as_ptr() as *const _);
                        if out_rc < 0 && !logged_decode_fail {
                            crate::scanner::write_append_log(&format!("audio: sceAudioOutOutput failed: 0x{:08X}", out_rc as u32));
                            logged_decode_fail = true;
                        }
                    }
                    let rest = accum_len - chunk_size;
                    if rest > 0 {
                        let slice = pcm_accumulator.as_mut_slice();
                        slice.copy_within(chunk_size..chunk_size + rest, 0);
                    }
                    accum_len = rest;
                }
            } else if decode_rc < 0 && !logged_decode_fail {
                crate::scanner::write_append_log(&format!("audio: sceAudiodecDecode failed: 0x{:08X} path={}", decode_rc as u32, path));
                logged_decode_fail = true;
            }
        }

        window.drain(0..offset + frame.len);
    };

    if port >= 0 {
        unsafe { sceAudioOutReleasePort(port) };
    }

    if !produced_audio {
        crate::scanner::write_append_log(&format!(
            "audio: {} produced no decodable audio; keeping file for diagnostics/retry",
            path
        ));
    }

    outcome
}

#[cfg(not(target_os = "vita"))]
fn music_thread_main(rx: mpsc::Receiver<MusicCmd>) {
    while rx.recv().is_ok() {}
}

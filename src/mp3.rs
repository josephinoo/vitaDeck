
#![allow(dead_code)]

const BITRATE_KBPS_V1_L3: [u16; 16] = [0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 0];
const BITRATE_KBPS_V2_L3: [u16; 16] = [0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160, 0];
const SAMPLE_RATE_V1: [u32; 3] = [44100, 48000, 32000];
const SAMPLE_RATE_V2: [u32; 3] = [22050, 24000, 16000];
const SAMPLE_RATE_V2_5: [u32; 3] = [11025, 12000, 8000];

pub struct FrameInfo {

    pub len: usize,

    pub sample_rate: u32,

    pub channels: u32,

    pub version: u32,
}

pub fn parse_frame_header(bytes: &[u8]) -> Option<FrameInfo> {
    if bytes.len() < 4 {
        return None;
    }

    if bytes[0] != 0xFF || (bytes[1] & 0xE0) != 0xE0 {
        return None;
    }

    let version_bits = (bytes[1] >> 3) & 0x03;
    let layer_bits = (bytes[1] >> 1) & 0x03;
    if layer_bits != 0x01 {
        return None; 
    }
    if version_bits == 0x01 {
        return None; 
    }

    let bitrate_index = (bytes[2] >> 4) & 0x0F;
    let sample_rate_index = (bytes[2] >> 2) & 0x03;
    let padding = (bytes[2] >> 1) & 0x01;
    if bitrate_index == 0 || bitrate_index == 15 || sample_rate_index == 3 {
        return None; 
    }

    let is_v1 = version_bits == 0x03;
    let bitrate_kbps = if is_v1 {
        BITRATE_KBPS_V1_L3[bitrate_index as usize]
    } else {
        BITRATE_KBPS_V2_L3[bitrate_index as usize]
    } as u32;
    let sample_rate = match version_bits {
        0x03 => SAMPLE_RATE_V1[sample_rate_index as usize],
        0x02 => SAMPLE_RATE_V2[sample_rate_index as usize],
        _ => SAMPLE_RATE_V2_5[sample_rate_index as usize],
    };
    if bitrate_kbps == 0 || sample_rate == 0 {
        return None;
    }

    let channel_mode = (bytes[3] >> 6) & 0x03;
    let channels = if channel_mode == 3 { 1 } else { 2 };
    let version = match version_bits {
        0x03 => 3, 
        0x02 => 2, 
        _ => 0,    
    };

    let coeff = if is_v1 { 144 } else { 72 };
    let len = (coeff * bitrate_kbps * 1000 / sample_rate + padding as u32) as usize;
    if len < 4 {
        return None;
    }

    Some(FrameInfo { len, sample_rate, channels, version })
}

pub fn id3_tag_len(bytes: &[u8]) -> Option<u64> {
    if bytes.len() < 10 || &bytes[0..3] != b"ID3" {
        return None;
    }
    let size = ((bytes[6] as u64 & 0x7F) << 21)
        | ((bytes[7] as u64 & 0x7F) << 14)
        | ((bytes[8] as u64 & 0x7F) << 7)
        | (bytes[9] as u64 & 0x7F);
    let footer = if bytes[5] & 0x10 != 0 { 10 } else { 0 };
    Some(10 + size + footer)
}

pub fn find_next_frame(bytes: &[u8], start: usize) -> Option<usize> {
    let mut pos = start;

    if let Some(tag_len) = id3_tag_len(&bytes[pos.min(bytes.len())..]) {
        let skip = tag_len as usize;
        if pos + skip >= bytes.len() {
            return None;
        }
        pos += skip;
    }
    while pos + 4 <= bytes.len() {
        if let Some(frame) = parse_frame_header(&bytes[pos..]) {
            let next = pos + frame.len;
            if next + 4 > bytes.len() || parse_frame_header(&bytes[next..]).is_some() {
                return Some(pos);
            }
        }
        pos += 1;
    }
    None
}

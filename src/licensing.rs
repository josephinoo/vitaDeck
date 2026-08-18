use anyhow::{bail, Result};

pub fn extract_content_id_from_url(url: &str) -> Option<String> {
    let bytes = url.as_bytes();
    if bytes.len() < 36 {
        return None;
    }
    for i in 0..=bytes.len() - 36 {
        let sub = &url[i..i + 36];
        let sub_bytes = sub.as_bytes();
        if sub_bytes[6] == b'-'
            && sub_bytes[16] == b'_'
            && sub_bytes[17] == b'0'
            && sub_bytes[18] == b'0'
            && sub_bytes[19] == b'-'
            && sub_bytes[0].is_ascii_uppercase()
            && sub_bytes[1].is_ascii_uppercase()
        {
            return Some(sub.to_string());
        }
    }
    None
}

pub fn resolve_content_id(
    title_id: &str,
    content_id: Option<&str>,
    download_url: &str,
    region: Option<&str>,
) -> Result<String> {
    if let Some(id) = content_id.filter(|s| !s.is_empty()) {
        return Ok(id.to_owned());
    }
    if let Some(id_from_url) = extract_content_id_from_url(download_url) {
        return Ok(id_from_url);
    }
    if title_id.is_empty() {
        bail!("no content ID or title ID available");
    }
    let prefix = match region {
        Some(r) if r.eq_ignore_ascii_case("EU") => "EP",
        Some(r) if r.eq_ignore_ascii_case("JP") => "JP",
        Some(r) if r.eq_ignore_ascii_case("ASIA") => "UP",
        _ => "UP",
    };
    Ok(format!("{prefix}0001-{title_id}_00-0000000000000000"))
}

pub fn create_fake_license(content_id: &str) -> Vec<u8> {
    let mut rif = vec![0u8; 512];
    rif[0x08..0x10].copy_from_slice(&0x0123456789ABCDEFu64.to_le_bytes());
    let cid = content_id.as_bytes();
    let cid_len = cid.len().min(0x30 - 1);
    rif[0x10..0x10 + cid_len].copy_from_slice(&cid[..cid_len]);
    rif[0x70..0x70 + 0x28].fill(0xFF);
    rif
}

unsafe extern "C" {
    fn pkgi_zrif_decode(
        zrif: *const std::ffi::c_char,
        rif: *mut u8,
        err: *mut std::ffi::c_char,
        err_size: u32,
    ) -> i32;
}

pub fn decode_zrif(zrif_str: &str) -> Option<Vec<u8>> {
    let c_zrif = std::ffi::CString::new(zrif_str).ok()?;
    let mut rif = vec![0u8; 1024];
    let mut err_buf = [0i8; 256];
    let res = unsafe {
        pkgi_zrif_decode(
            c_zrif.as_ptr(),
            rif.as_mut_ptr(),
            err_buf.as_mut_ptr(),
            err_buf.len() as u32,
        )
    };
    if res > 0 {
        rif.truncate(res as usize);
        Some(rif)
    } else {
        None
    }
}

pub fn get_license(
    title_id: &str,
    content_id: Option<&str>,
    download_url: &str,
    zrif: Option<&str>,
    region: Option<&str>,
) -> Option<Vec<u8>> {
    if let Some(zrif) = zrif.filter(|s| !s.is_empty()) {
        if let Some(rif_bytes) = decode_zrif(zrif) {
            return Some(rif_bytes);
        }
    }
    if let Ok(cid) = resolve_content_id(title_id, content_id, download_url, region) {
        return Some(create_fake_license(&cid));
    }
    None
}

pub fn install_license(content_id: &str, rif_bytes: &[u8]) -> Result<()> {
    let license_dir = "ux0:license/app";
    let _ = std::fs::create_dir_all(license_dir);
    let path = format!("{license_dir}/{content_id}.rif");
    std::fs::write(&path, rif_bytes)?;
    Ok(())
}

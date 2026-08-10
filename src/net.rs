use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

pub const API_BASE: &str = env!("VITADECK_API_BASE");

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const READ_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_REDIRECTS: u32 = 10;
const USER_AGENT: &str = "VitaDeck";

fn tls_config() -> Arc<rustls::ClientConfig> {
    static CONFIG: OnceLock<Arc<rustls::ClientConfig>> = OnceLock::new();
    CONFIG
        .get_or_init(|| {
            let mut roots = rustls::RootCertStore::empty();
            roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
            Arc::new(
                rustls::ClientConfig::builder()
                    .with_root_certificates(roots)
                    .with_no_client_auth(),
            )
        })
        .clone()
}

pub fn init() -> Result<(), String> {
    #[cfg(target_os = "vita")]
    unsafe {
        use vitasdk_sys::*;
        let _ = sceSysmoduleLoadModule(SCE_SYSMODULE_NET);
        let mut param = SceNetInitParam {
            memory: Box::leak(vec![0u8; 1024 * 1024].into_boxed_slice()).as_mut_ptr() as *mut _,
            size: 1024 * 1024,
            flags: 0,
        };
        let _ = sceNetInit(&mut param);
        let _ = sceNetCtlInit();
    }
    Ok(())
}

pub fn wifi_available() -> bool {
    #[cfg(target_os = "vita")]
    unsafe {
        use vitasdk_sys::*;
        let mut state: i32 = 0;
        sceNetCtlInetGetState(&mut state) >= 0 && state == SCE_NETCTL_STATE_CONNECTED as i32
    }
    #[cfg(not(target_os = "vita"))]
    true
}

struct Url {
    https: bool,
    host: String,
    port: u16,
    path: String,
}

fn parse_url(url: &str) -> Result<Url, String> {
    let (https, rest) = if let Some(r) = url.strip_prefix("https://") {
        (true, r)
    } else if let Some(r) = url.strip_prefix("http://") {
        (false, r)
    } else {
        return Err(format!("unsupported URL scheme: {url}"));
    };

    let (authority, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    let (host, port) = match authority.rsplit_once(':') {
        Some((h, p)) => (h, p.parse::<u16>().map_err(|_| "bad port".to_string())?),
        None => (authority, if https { 443 } else { 80 }),
    };

    Ok(Url { https, host: host.to_string(), port, path: path.to_string() })
}

fn request_once(
    url: &Url,
    max_bytes: usize,
    method: &str,
) -> Result<(u16, Option<String>, Option<usize>, Vec<u8>), String> {
    let tcp = TcpStream::connect((url.host.as_str(), url.port)).map_err(|e| e.to_string())?;

    #[cfg(not(target_os = "vita"))]
    {
        tcp.set_read_timeout(Some(READ_TIMEOUT)).ok();
        tcp.set_write_timeout(Some(CONNECT_TIMEOUT)).ok();
    }

    let request = format!(
        "{} {} HTTP/1.1\r\nHost: {}\r\nUser-Agent: {}\r\nAccept: application/json, */*\r\nConnection: close\r\n\r\n",
        method, url.path, url.host, USER_AGENT
    );

    let mut reader: Box<dyn Read> = if url.https {
        let server_name = rustls_pki_types::ServerName::try_from(url.host.clone())
            .map_err(|_| "invalid hostname".to_string())?;
        let mut conn = rustls::ClientConnection::new(tls_config(), server_name)
            .map_err(|e| e.to_string())?;
        let mut tcp = tcp;
        {
            let mut tls = rustls::Stream::new(&mut conn, &mut tcp);
            tls.write_all(request.as_bytes()).map_err(|e| e.to_string())?;
        }
        Box::new(RustlsOwnedStream { conn, sock: tcp })
    } else {
        let mut tcp = tcp;
        tcp.write_all(request.as_bytes()).map_err(|e| e.to_string())?;
        Box::new(tcp)
    };

    read_response(&mut reader, max_bytes, method == "HEAD")
}

struct RustlsOwnedStream {
    conn: rustls::ClientConnection,
    sock: TcpStream,
}

impl Read for RustlsOwnedStream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        rustls::Stream::new(&mut self.conn, &mut self.sock).read(buf)
    }
}

fn read_response(
    reader: &mut dyn Read,
    max_bytes: usize,
    head_only: bool,
) -> Result<(u16, Option<String>, Option<usize>, Vec<u8>), String> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    let header_end = loop {
        if let Some(pos) = find_subslice(&buf, b"\r\n\r\n") {
            break pos;
        }
        let n = reader.read(&mut chunk).map_err(|e| e.to_string())?;
        if n == 0 {
            return Err("connection closed before headers completed".to_string());
        }
        buf.extend_from_slice(&chunk[..n]);
        if buf.len() > 32 * 1024 {
            return Err("response headers too large".to_string());
        }
    };

    let header_text = String::from_utf8_lossy(&buf[..header_end]);
    let mut lines = header_text.split("\r\n");
    let status_line = lines.next().unwrap_or("");
    let status: u16 = status_line.split_whitespace().nth(1).and_then(|s| s.parse().ok()).unwrap_or(0);

    let mut content_length: Option<usize> = None;
    let mut chunked = false;
    let mut location: Option<String> = None;
    for line in lines {
        let Some((key, val)) = line.split_once(':') else { continue };
        let key = key.trim().to_ascii_lowercase();
        let val = val.trim();
        match key.as_str() {
            "content-length" => content_length = val.parse().ok(),
            "transfer-encoding" if val.eq_ignore_ascii_case("chunked") => chunked = true,
            "location" => location = Some(val.to_string()),
            _ => {}
        }
    }

    if head_only {
        return Ok((status, location, content_length, Vec::new()));
    }

    let mut body = buf[header_end + 4..].to_vec();

    if chunked {
        body = read_chunked(reader, body, max_bytes)?;
    } else if let Some(len) = content_length {
        if len > max_bytes {
            return Err("response exceeds max size".to_string());
        }
        while body.len() < len {
            let n = reader.read(&mut chunk).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            body.extend_from_slice(&chunk[..n]);
            if body.len() > max_bytes {
                return Err("response exceeds max size".to_string());
            }
        }
    } else {

        loop {
            let n = reader.read(&mut chunk).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            body.extend_from_slice(&chunk[..n]);
            if body.len() > max_bytes {
                return Err("response exceeds max size".to_string());
            }
        }
    }

    Ok((status, location, content_length, body))
}

fn read_chunked(reader: &mut dyn Read, mut leftover: Vec<u8>, max_bytes: usize) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    let mut chunk = [0u8; 4096];

    loop {

        let size_end = loop {
            if let Some(pos) = find_subslice(&leftover, b"\r\n") {
                break pos;
            }
            let n = reader.read(&mut chunk).map_err(|e| e.to_string())?;
            if n == 0 {
                return Err("connection closed mid-chunk-header".to_string());
            }
            leftover.extend_from_slice(&chunk[..n]);
        };
        let size_line = String::from_utf8_lossy(&leftover[..size_end]);
        let size_str = size_line.split(';').next().unwrap_or("").trim();
        let size = usize::from_str_radix(size_str, 16).map_err(|_| "bad chunk size".to_string())?;
        leftover.drain(..size_end + 2);

        if size == 0 {
            break;
        }
        if out.len() + size > max_bytes {
            return Err("response exceeds max size".to_string());
        }

        while leftover.len() < size + 2 {
            let n = reader.read(&mut chunk).map_err(|e| e.to_string())?;
            if n == 0 {
                return Err("connection closed mid-chunk".to_string());
            }
            leftover.extend_from_slice(&chunk[..n]);
        }
        out.extend_from_slice(&leftover[..size]);
        leftover.drain(..size + 2); 
    }

    Ok(out)
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

pub fn request(url: &str, max_bytes: usize, method: &str) -> Result<(Vec<u8>, Option<usize>), String> {
    let mut current = url.to_string();
    for _ in 0..MAX_REDIRECTS {
        let parsed = parse_url(&current)?;
        let (status, location, content_length, body) = request_once(&parsed, max_bytes, method)?;
        match status {
            200..=299 => return Ok((body, content_length)),
            301 | 302 | 303 | 307 | 308 => {
                let Some(loc) = location else { return Err(format!("redirect ({status}) with no Location")) };
                current = if loc.starts_with("http://") || loc.starts_with("https://") {
                    loc
                } else if let Some(rest) = loc.strip_prefix('/') {
                    format!("{}://{}:{}/{}", if parsed.https { "https" } else { "http" }, parsed.host, parsed.port, rest)
                } else {
                    loc
                };
            }
            _ => return Err(format!("HTTP {status}")),
        }
    }
    Err("too many redirects".to_string())
}

pub fn download_to_vec(url: &str, max_bytes: usize) -> Result<Vec<u8>, String> {
    request(url, max_bytes, "GET").map(|(body, _)| body)
}

pub fn download_to_file(url: &str, path: &str, max_bytes: usize) -> Result<(), String> {
    let mut current = url.to_string();
    for _ in 0..MAX_REDIRECTS {
        let parsed = parse_url(&current)?;
        let tcp = TcpStream::connect((parsed.host.as_str(), parsed.port)).map_err(|e| e.to_string())?;
        #[cfg(not(target_os = "vita"))]
        {
            tcp.set_read_timeout(Some(READ_TIMEOUT)).ok();
            tcp.set_write_timeout(Some(CONNECT_TIMEOUT)).ok();
        }

        let request = format!(
            "GET {} HTTP/1.1\r\nHost: {}\r\nUser-Agent: {}\r\nAccept: */*\r\nConnection: close\r\n\r\n",
            parsed.path, parsed.host, USER_AGENT
        );

        let mut reader: Box<dyn Read> = if parsed.https {
            let server_name = rustls_pki_types::ServerName::try_from(parsed.host.clone())
                .map_err(|_| "invalid hostname".to_string())?;
            let mut conn = rustls::ClientConnection::new(tls_config(), server_name)
                .map_err(|e| e.to_string())?;
            let mut tcp = tcp;
            {
                let mut tls = rustls::Stream::new(&mut conn, &mut tcp);
                tls.write_all(request.as_bytes()).map_err(|e| e.to_string())?;
            }
            Box::new(RustlsOwnedStream { conn, sock: tcp })
        } else {
            let mut tcp = tcp;
            tcp.write_all(request.as_bytes()).map_err(|e| e.to_string())?;
            Box::new(tcp)
        };

        let (status, location, content_length, leftover, chunked) =
            read_response_headers(&mut reader)?;
        match status {
            200..=299 => {
                if let Some(len) = content_length {
                    if len > max_bytes {
                        return Err("response exceeds max size".to_string());
                    }
                }
                let mut file = std::fs::File::create(path).map_err(|e| e.to_string())?;
                stream_body_to_writer(
                    &mut reader,
                    &mut file,
                    leftover,
                    content_length,
                    chunked,
                    max_bytes,
                )?;
                return Ok(());
            }
            301 | 302 | 303 | 307 | 308 => {
                let Some(loc) = location else {
                    return Err(format!("redirect ({status}) with no Location"));
                };
                current = if loc.starts_with("http://") || loc.starts_with("https://") {
                    loc
                } else if let Some(rest) = loc.strip_prefix('/') {
                    format!(
                        "{}://{}:{}/{}",
                        if parsed.https { "https" } else { "http" },
                        parsed.host,
                        parsed.port,
                        rest
                    )
                } else {
                    loc
                };
            }
            _ => return Err(format!("HTTP {status}")),
        }
    }
    Err("too many redirects".to_string())
}

pub fn fetch_content_length(url: &str) -> Option<usize> {
    request(url, 0, "HEAD").ok().and_then(|(_, len)| len)
}

fn read_response_headers(
    reader: &mut dyn Read,
) -> Result<(u16, Option<String>, Option<usize>, Vec<u8>, bool), String> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    let header_end = loop {
        if let Some(pos) = find_subslice(&buf, b"\r\n\r\n") {
            break pos;
        }
        let n = reader.read(&mut chunk).map_err(|e| e.to_string())?;
        if n == 0 {
            return Err("connection closed before headers completed".to_string());
        }
        buf.extend_from_slice(&chunk[..n]);
        if buf.len() > 32 * 1024 {
            return Err("response headers too large".to_string());
        }
    };

    let header_text = String::from_utf8_lossy(&buf[..header_end]);
    let mut lines = header_text.split("\r\n");
    let status_line = lines.next().unwrap_or("");
    let status: u16 = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);

    let mut content_length: Option<usize> = None;
    let mut chunked = false;
    let mut location: Option<String> = None;
    for line in lines {
        let Some((key, val)) = line.split_once(':') else { continue };
        let key = key.trim().to_ascii_lowercase();
        let val = val.trim();
        match key.as_str() {
            "content-length" => content_length = val.parse().ok(),
            "transfer-encoding" if val.eq_ignore_ascii_case("chunked") => chunked = true,
            "location" => location = Some(val.to_string()),
            _ => {}
        }
    }

    let leftover = buf[header_end + 4..].to_vec();
    Ok((status, location, content_length, leftover, chunked))
}

fn stream_body_to_writer(
    reader: &mut dyn Read,
    writer: &mut dyn Write,
    mut leftover: Vec<u8>,
    content_length: Option<usize>,
    chunked: bool,
    max_bytes: usize,
) -> Result<(), String> {
    let mut written = 0usize;
    let mut chunk = [0u8; 8192];

    if chunked {
        let body = read_chunked(reader, leftover, max_bytes)?;
        writer.write_all(&body).map_err(|e| e.to_string())?;
        return Ok(());
    }

    if !leftover.is_empty() {
        if leftover.len() > max_bytes {
            return Err("response exceeds max size".to_string());
        }
        writer.write_all(&leftover).map_err(|e| e.to_string())?;
        written += leftover.len();
        leftover.clear();
    }

    if let Some(len) = content_length {
        while written < len {
            let want = (len - written).min(chunk.len());
            let n = reader.read(&mut chunk[..want]).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            writer.write_all(&chunk[..n]).map_err(|e| e.to_string())?;
            written += n;
            if written > max_bytes {
                return Err("response exceeds max size".to_string());
            }
        }
    } else {
        loop {
            let n = reader.read(&mut chunk).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            written += n;
            if written > max_bytes {
                return Err("response exceeds max size".to_string());
            }
            writer.write_all(&chunk[..n]).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

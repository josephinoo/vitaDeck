use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::Read;

pub const CONFIG_FILE: &str = "ux0:data/VitaDeck/config.json";

pub const BUDGET_OPTIONS_MB: &[u32] = &[50, 100, 150, 300, 0]; 

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default = "default_download_bgm")]
    pub download_bgm: bool,
    #[serde(default = "default_cache_budget_mb")]
    pub cache_budget_mb: u32,
    #[serde(default)]
    pub client_id: String,
}

fn default_download_bgm() -> bool {
    true
}

fn default_cache_budget_mb() -> u32 {
    150
}

impl Default for Config {
    fn default() -> Self {
        Self {
            download_bgm: default_download_bgm(),
            cache_budget_mb: default_cache_budget_mb(),
            client_id: String::new(),
        }
    }
}

fn generate_client_id() -> String {
    let mut bytes = [0u8; 16];
    #[cfg(target_os = "vita")]
    unsafe {
        vitasdk_sys::sceKernelGetRandomNumber(bytes.as_mut_ptr() as *mut _, bytes.len() as u32);
    }
    #[cfg(not(target_os = "vita"))]
    {
        use std::time::{SystemTime, UNIX_EPOCH};
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        let seed = nanos ^ (std::process::id() as u128);
        bytes.copy_from_slice(&seed.to_le_bytes());
    }
    let hex: String = bytes.iter().map(|b| format!("{:02x}", b)).collect();
    format!("vitadeck-{hex}")
}

impl Config {
    pub fn load() -> Self {
        let mut config: Config = File::open(CONFIG_FILE)
            .ok()
            .and_then(|mut file| {
                let mut contents = String::new();
                file.read_to_string(&mut contents).ok()?;
                serde_json::from_str(&contents).ok()
            })
            .unwrap_or_default();

        if config.client_id.is_empty() {
            config.client_id = generate_client_id();
            config.save();
        }
        config
    }

    pub fn save(&self) {
        let _ = std::fs::create_dir_all("ux0:data/VitaDeck");
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let tmp = format!("{}.part", CONFIG_FILE);
            if std::fs::write(&tmp, json.as_bytes()).is_ok() {
                let _ = std::fs::rename(&tmp, CONFIG_FILE);
            }
        }
    }

    pub fn budget_label(&self) -> &'static str {
        match self.cache_budget_mb {
            50 => "50 MB",
            100 => "100 MB",
            150 => "150 MB",
            300 => "300 MB",
            0 => "Unlimited",
            _ => "Custom",
        }
    }

    pub fn next_budget_option(&mut self) {
        let current = self.cache_budget_mb;
        let idx = BUDGET_OPTIONS_MB.iter().position(|&b| b == current).unwrap_or(2);
        let next_idx = (idx + 1) % BUDGET_OPTIONS_MB.len();
        self.cache_budget_mb = BUDGET_OPTIONS_MB[next_idx];
        self.save();
    }
}

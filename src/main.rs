#[cfg(target_os = "vita")]
mod vita_runtime {
    #[used]
    #[unsafe(export_name = "sceUserMainThreadStackSize")]
    pub static SCE_USER_MAIN_THREAD_STACK_SIZE: u32 = 4 * 1024 * 1024;

    #[used]
    #[unsafe(export_name = "_newlib_heap_size_user")]
    pub static NEWLIB_HEAP_SIZE_USER: u32 = 192 * 1024 * 1024;
}

mod app;
mod artwork;
mod audio;
mod bgdl;
mod cache_manager;
mod collections;
mod config;
mod ime;
mod input;
mod licensing;
mod logger;
mod mp3;
mod net;
mod recent;
mod scanner;
mod shell;
mod store;
mod textures;
mod ui;
pub mod runtime;

use app::App;

fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        let _ = std::fs::create_dir_all("ux0:data/VitaDeck");
        use std::io::Write;
        if let Ok(mut file) =
            std::fs::OpenOptions::new().create(true).append(true).open("ux0:data/VitaDeck/panic.log")
        {
            let _ = writeln!(file, "=== panic ===\n{info}");
        }
    }));
}

fn main() -> anyhow::Result<()> {
    install_panic_hook();
    logger::log("=== VitaDeck starting ===");

    let app = App::new();
    logger::log("App::new completed successfully");
    shell::run(app)
}

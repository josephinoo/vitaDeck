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
mod i18n;
mod input;
mod licensing;
mod logger;
mod mp3;
mod net;
mod recent;
mod scanner;
mod session;
mod shell;
mod stats;
mod store;
mod textures;
mod ui;
pub mod runtime;

use app::App;

fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        crate::logger::log(&format!("PANIC: {info}\n{}", crate::logger::current_health_context()));
        let _ = std::fs::create_dir_all("ux0:data/VitaDeck");
        use std::io::Write;
        if let Ok(mut file) =
            std::fs::OpenOptions::new().create(true).append(true).open("ux0:data/VitaDeck/panic.log")
        {
            let _ = writeln!(
                file,
                "=== panic ===\n{info}\n{}",
                crate::logger::current_health_context()
            );
        }
    }));
}

fn main() -> anyhow::Result<()> {
    install_panic_hook();
    let recovered_from_crash = session::begin();
    if recovered_from_crash {
        logger::log("previous VitaDeck session ended unexpectedly; restoring normal media loading");
    }
    logger::log("=== VitaDeck starting ===");
    logger::log("build: store-texture-retirement-v2");

    let app = App::new(recovered_from_crash);
    logger::log("App::new completed successfully");
    shell::run(app)
}

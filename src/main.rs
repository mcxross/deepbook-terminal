use crate::widgets::Terminal;
use std::io::Write;

#[macro_use]
mod macros;

pub mod app;
pub mod data;
pub mod kline;
pub mod logger;
pub mod openapi;
#[cfg_attr(target_family = "windows", path = "os/windows.rs")]
#[cfg_attr(target_family = "unix", path = "os/unix.rs")]
pub mod os;
pub mod render;
pub mod systems;
pub mod ui;
pub mod utils;
pub mod widgets;

mod views;

#[derive(Clone, Debug, Default)]
pub struct Args {
    pub logout: bool,
}

#[tokio::main]
async fn main() {
    let _guard = logger::init();
    tracing::info!("App started");

    let quote_receiver = match openapi::init_contexts().await {
        Ok(receiver) => receiver,
        Err(e) => {
            openapi::print_config_guide();
            eprintln!("\nError details: {e}");
            return;
        }
    };

    tracing::info!("Surflux context initialized successfully");

    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        Terminal::exit_full_screen();
        hook(info);
    }));

    let _ = std::io::stdout().write_all(b"\n");
    let _ = std::io::stdout().flush();

    Terminal::enter_full_screen();
    let args = Args::default();
    app::run(args, quote_receiver).await;
    Terminal::exit_full_screen();
}

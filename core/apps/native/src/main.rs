#[cfg(feature = "automation")]
mod automation;

#[cfg(not(feature = "automation"))]
mod automation {
    use std::net::SocketAddr;
    use std::path::PathBuf;

    use gpui::{App, WindowHandle};

    use crate::app::ShellView;

    #[derive(Clone, Debug, Default)]
    pub struct AutomationConfig {
        pub addr: Option<SocketAddr>,
        pub screenshot_dir: Option<PathBuf>,
        pub fixture: Option<String>,
    }

    pub fn start(_app: &mut App, _window: WindowHandle<ShellView>, _config: AutomationConfig) {}
}
mod app;
mod theme;

use std::net::SocketAddr;
use std::path::PathBuf;

use clap::Parser;

#[derive(Parser, Debug)]
#[command(name = "ctx-native", version, about = "ctx native client")]
struct Cli {
    #[arg(long, value_name = "DIR")]
    screenshot_dir: Option<PathBuf>,
    #[arg(long, value_name = "NAME")]
    fixture: Option<String>,
    #[arg(long, value_name = "WIDTHxHEIGHT")]
    window_size: Option<app::WindowSize>,
    #[arg(long, value_name = "HOST:PORT")]
    automation_addr: Option<SocketAddr>,
}

fn main() {
    let cli = Cli::parse();
    let options = app::AppOptions {
        window_size: cli.window_size,
        automation: automation::AutomationConfig {
            addr: cli.automation_addr,
            screenshot_dir: cli.screenshot_dir,
            fixture: cli.fixture,
        },
    };
    app::run(options);
}

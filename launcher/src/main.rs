#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
use rac_rs_launcher::cfg::LauncherCfg;
use rac_rs_launcher::platform;
use rac_rs_launcher::ui::{theme, LauncherApp};
use std::time::{Duration, Instant};

const ICON_PNG: &[u8] = include_bytes!("../assets/rac-rs-256.png");

fn icon() -> Option<egui::IconData> {
    let img = image::load_from_memory_with_format(ICON_PNG, image::ImageFormat::Png).ok()?.to_rgba8();
    let (width, height) = img.dimensions();
    Some(egui::IconData { rgba: img.into_raw(), width, height })
}

fn main() {
    let info = LauncherApp::open_args();
    let restarted = info.restarted;
    // One launcher per window; after a self-update the old process may still be closing, so retry for a while.
    let until = Instant::now() + Duration::from_secs(if restarted { 10 } else { 0 });
    let guard = loop {
        if let Some(g) = platform::single_instance("Local\\RacRsLauncher") {
            break Some(g);
        }
        if Instant::now() >= until {
            break None;
        }
        std::thread::sleep(Duration::from_millis(200));
    };
    if guard.is_none() {
        rfd::MessageDialog::new()
            .set_title("RAC-RS Launcher")
            .set_description("The launcher is already open. Look for its window in the taskbar.")
            .set_level(rfd::MessageLevel::Info)
            .show();
        return;
    }
    let cfg = LauncherCfg::load(&info.data_dir);
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("RAC-RS Launcher")
        .with_inner_size([cfg.width.max(860.0), cfg.height.max(580.0)])
        .with_min_inner_size([860.0, 580.0]);
    if let Some(i) = icon() {
        viewport = viewport.with_icon(i);
    }
    let opts = eframe::NativeOptions { viewport, ..Default::default() };
    let result = eframe::run_native(
        "RAC-RS Launcher",
        opts,
        Box::new(move |cc| {
            theme::apply(&cc.egui_ctx);
            let mut app = LauncherApp::build_app(&info);
            app.restarted = restarted;
            Ok(Box::new(LauncherApp::new(app)))
        }),
    );
    if let Err(e) = result {
        rfd::MessageDialog::new().set_title("RAC-RS Launcher").set_description(format!("The launcher could not start: {e}")).set_level(rfd::MessageLevel::Error).show();
    }
    drop(guard);
}

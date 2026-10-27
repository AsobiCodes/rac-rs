use rac_rs_launcher::cfg::LauncherCfg;
use rac_rs_launcher::platform;
use rac_rs_launcher::ui::{theme, LauncherApp};

const ICON_PNG: &[u8] = include_bytes!("../assets/rac-rs-256.png");

fn icon() -> Option<egui::IconData> {
    let img = image::load_from_memory_with_format(ICON_PNG, image::ImageFormat::Png).ok()?.to_rgba8();
    let (width, height) = img.dimensions();
    Some(egui::IconData { rgba: img.into_raw(), width, height })
}

fn main() {
    let info = LauncherApp::open_args();
    // One launcher per window.
    let guard = platform::single_instance("rac-rs-launcher");
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
            Ok(Box::new(LauncherApp::new(LauncherApp::build_app(&info))))
        }),
    );
    if let Err(e) = result {
        rfd::MessageDialog::new().set_title("RAC-RS Launcher").set_description(format!("The launcher could not start: {e}")).set_level(rfd::MessageLevel::Error).show();
    }
    drop(guard);
}

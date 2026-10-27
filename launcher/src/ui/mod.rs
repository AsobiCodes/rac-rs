//! The egui user interface of the launcher.
pub mod app;
pub mod cover;
pub mod page_backup;
pub mod page_game;
pub mod page_install;
pub mod page_library;
pub mod page_mods;
pub mod page_mods_master;
pub mod page_mods_script;
pub mod page_net;
pub mod page_settings;
pub mod presets;
pub mod profile_picker;
pub mod rows;
pub mod settings_presets;
pub mod sidebar;
pub mod state;
pub mod state_backup;
pub mod state_lib;
pub mod state_net;
pub mod theme;

pub use app::LauncherApp;

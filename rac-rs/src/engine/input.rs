//! Input devices (engine split stage E5, 2026-10-27; from rac1 retail): which keyboard key or controller button drives which PS2
//! pad function (`binds`), and the gamepads as a PS2 pad (`pad`: an XInput controller, `pspad`: PlayStation controllers over
//! HID). A game turns these into its pad words; the bindings page and the device prompts are the game's.
pub mod binds;
pub mod pad;
pub mod pspad;
mod xinput;

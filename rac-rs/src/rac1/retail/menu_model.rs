//! The retail menu bridge of the modern screens (v0.5, screens stage A): what the page menu on screen IS (`pages::route`, `view::view`),
//! its texts decoded per language (`text`), the addresses it is read from on the PAL and USA disc (`addrs`), and how a screen's choice
//! reaches the game (`cmd::resolve` + `cmd::submit`; the worker side joins the pad in `Retail::tick_keys`). The matching hide
//! switches are `modern_menu::hide`. Nothing here draws, and no switch is set yet, so the picture and the input are unchanged.
pub mod addrs;
pub mod cmd;
pub mod extra;
pub mod pages;
pub mod pcpage;
pub mod text;
pub mod view;
#[cfg(test)]
mod tests;

pub use cmd::{resolve, submit, submit_quiet, Command, Intent};
pub use pages::{route, Page, Route};
pub use text::{Bank, Btn, RichText, Span, Strings};
pub use view::{view, PageView};

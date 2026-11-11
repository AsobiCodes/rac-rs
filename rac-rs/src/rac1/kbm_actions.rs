//! R&C1's keyboard + mouse action table (v0.5.1; design `V051_KBM_DESIGN.md`, engine `engine/input/kbm`): the rows of the Controls page
//! with their outputs, defaults (exactly the controls the port had before the system: Space, F, LMB / G, T / MMB, ...), the labels in
//! the game font (`pcctl.rs LABELS` conventions: bytes for the accented letters), the Ship context and the Menu layer (the fixed
//! menu keys of `retail/menu_keys.rs`).
//!
//! Retail meanings checked (decomp/notes): R1 and R2 both crouch, L1 and L2 both start the look state (hero_crouch_ledge.md), so the
//! labels carry the pad name; R3 opens the map like Select (help text 2013 "Press the SELECT button or the R3 button to bring up the
//! map"), so its label is "Map (R3)"; no retail use of L3 was found, its label is plain "L3". The labels are the game's own words where it has them (the pause tab and the
//! Controls help of the same language: Quick Select 0x4ee4, Jump 0x5258, Crouch 0x5253, Look around 0x5256, OmniWrench 0x4e46, Fire weapon
//! = the first half of 0x5255, Camera 0x4f23, Map 0x4ef5; `decomp/notes/aridia_data/help_text.csv`); Pause, Movement, D-pad and the
//! PC rows have no retail word.

use rac_engine::input::kbm::{ActionDef, Input, Layer, MenuKey, MenuOut, Mouse, Output, Table, Wheel};
use winit::keyboard::KeyCode as K;

/// PC action ids (`Output::Pc`): Aim = the modern camera / strafe aiming (`modcam::ui_aim`), Zoom = the camera zoom (`modcam::ui_zoom`).
pub const AIM: u8 = 0;
pub const ZOOM_IN: u8 = 1;
pub const ZOOM_OUT: u8 = 2;
/// The context bit of a ship fight / turret (`shipmouse`): the ship's fire button is Cross.
pub const SHIP: u8 = 0;

/// Sections of the Controls page.
pub const SECTIONS: usize = 6;

/// No controller slots (stick rows, keyboard-only PC rows, the Ship layer).
const NO_PAD: u8 = 255;
/// `pad`: the controller button of today's default (`binds::PHYS` index: A 0, B 1, X 2, Y 3, LB 4, RB 5, Start 6, Back 7, L3 8, R3 9,
/// D-pad up / down / left / right 10..13, LT 14, RT 15); `follows`: the old pad function the slot follows in an old `pad=` file.
const fn a(id: &'static str, section: u8, layer: Layer, output: Output, d0: Option<Input>, d1: Option<Input>, keep_one: bool, pad: u8, follows: u16) -> ActionDef {
	let ok = pad != NO_PAD;
	ActionDef { id, section, layer, output, defaults: [d0, d1], keep_one, pad_ok: ok, pad_defaults: [if ok { Some(pad) } else { None }, None], pad_follows: follows }
}
const fn mk(input: Input, out: MenuOut, unless_sticks: bool, stick_shadow: bool) -> MenuKey {
	MenuKey { input, out, unless_sticks, stick_shadow }
}
const fn key(k: K) -> Input {
	Input::Key(k)
}

pub static ACTIONS: [ActionDef; 28] = [
	a("jump", 0, Layer::Game, Output::Pad(0x40), Some(Input::Key(K::Space)), None, false, 0, 0),
	a("wrench", 0, Layer::Game, Output::Pad(0x80), Some(Input::Key(K::KeyF)), None, false, 2, 0),
	a("fire", 0, Layer::Game, Output::Pad(0x20), Some(Input::Mouse(Mouse::Left)), Some(Input::Key(K::KeyG)), false, 1, 0),
	a("quick_select", 0, Layer::Game, Output::Pad(0x10), Some(Input::Key(K::KeyT)), Some(Input::Mouse(Mouse::Middle)), false, 3, 0),
	a("look", 0, Layer::Game, Output::Pad(4), Some(Input::Key(K::KeyC)), None, false, 4, 0),
	a("crouch", 0, Layer::Game, Output::Pad(8), Some(Input::Key(K::ControlLeft)), None, false, 5, 0),
	a("look2", 1, Layer::Game, Output::Pad(1), Some(Input::Key(K::KeyV)), None, false, 14, 0),
	a("crouch2", 1, Layer::Game, Output::Pad(2), Some(Input::Key(K::KeyB)), None, false, 15, 0),
	a("l3", 1, Layer::Game, Output::Pad(0x200), Some(Input::Key(K::ShiftLeft)), None, false, 8, 0),
	a("r3", 1, Layer::Game, Output::Pad(0x400), Some(Input::Key(K::KeyR)), None, false, 9, 0),
	a("pause", 1, Layer::Game, Output::Pad(0x800), Some(Input::Key(K::Escape)), Some(Input::Key(K::Enter)), true, 6, 0),
	a("map", 1, Layer::Game, Output::Pad(0x100), Some(Input::Key(K::KeyM)), None, false, 7, 0),
	a("forward", 2, Layer::Game, Output::Stick(3, -1), Some(Input::Key(K::KeyW)), None, false, NO_PAD, 0),
	a("back", 2, Layer::Game, Output::Stick(3, 1), Some(Input::Key(K::KeyS)), None, false, NO_PAD, 0),
	a("strafe_left", 2, Layer::Game, Output::Stick(2, -1), Some(Input::Key(K::KeyA)), None, false, NO_PAD, 0),
	a("strafe_right", 2, Layer::Game, Output::Stick(2, 1), Some(Input::Key(K::KeyD)), None, false, NO_PAD, 0),
	a("camera_up", 3, Layer::Game, Output::Stick(1, -1), Some(Input::Key(K::KeyX)), None, false, NO_PAD, 0),
	a("camera_down", 3, Layer::Game, Output::Stick(1, 1), Some(Input::Key(K::KeyZ)), None, false, NO_PAD, 0),
	a("camera_left", 3, Layer::Game, Output::Stick(0, -1), Some(Input::Key(K::KeyE)), None, false, NO_PAD, 0),
	a("camera_right", 3, Layer::Game, Output::Stick(0, 1), Some(Input::Key(K::KeyQ)), None, false, NO_PAD, 0),
	a("up", 4, Layer::Game, Output::Pad(0x1000), Some(Input::Key(K::ArrowUp)), None, false, 10, 0),
	a("down", 4, Layer::Game, Output::Pad(0x4000), Some(Input::Key(K::ArrowDown)), None, false, 11, 0),
	a("left", 4, Layer::Game, Output::Pad(0x8000), Some(Input::Key(K::ArrowLeft)), None, false, 12, 0),
	a("right", 4, Layer::Game, Output::Pad(0x2000), Some(Input::Key(K::ArrowRight)), None, false, 13, 0),
	a("aim", 5, Layer::Game, Output::Pc(AIM), Some(Input::Mouse(Mouse::Right)), None, false, 15, 2),
	a("zoom_in", 5, Layer::Game, Output::Pc(ZOOM_IN), Some(Input::Wheel(Wheel::Up)), None, false, NO_PAD, 0),
	a("zoom_out", 5, Layer::Game, Output::Pc(ZOOM_OUT), Some(Input::Wheel(Wheel::Down)), None, false, NO_PAD, 0),
	a("ship_fire", 5, Layer::Context(SHIP), Output::Pad(0x40), Some(Input::Mouse(Mouse::Left)), None, false, NO_PAD, 0),
];

/// The Menu layer: the keys of the modern menu scheme (Numpad Enter is Enter). They are removed from the game bindings while a
/// screen is up (`MenuKey` shadows); Q / E also leave the sticks. The Pad bits are the d-pad and L1 / R1 of `mouse_menu`.
pub static MENU: [MenuKey; 18] = [
	mk(key(K::Space), MenuOut::Confirm, false, false),
	mk(key(K::Enter), MenuOut::Confirm, false, false),
	mk(key(K::Escape), MenuOut::Back, false, false),
	mk(key(K::Backspace), MenuOut::Back, false, false),
	mk(key(K::Tab), MenuOut::Alt, false, false),
	mk(key(K::KeyQ), MenuOut::Pad(4), false, true),
	mk(key(K::KeyE), MenuOut::Pad(8), false, true),
	mk(key(K::KeyZ), MenuOut::None, false, false),
	mk(key(K::KeyX), MenuOut::None, false, false),
	mk(key(K::Delete), MenuOut::None, false, false),
	mk(key(K::ArrowUp), MenuOut::Pad(0x1000), false, false),
	mk(key(K::ArrowDown), MenuOut::Pad(0x4000), false, false),
	mk(key(K::ArrowLeft), MenuOut::Pad(0x8000), false, false),
	mk(key(K::ArrowRight), MenuOut::Pad(0x2000), false, false),
	mk(key(K::KeyW), MenuOut::Pad(0x1000), true, false),
	mk(key(K::KeyS), MenuOut::Pad(0x4000), true, false),
	mk(key(K::KeyA), MenuOut::Pad(0x8000), true, false),
	mk(key(K::KeyD), MenuOut::Pad(0x2000), true, false),
];

pub static TABLE: Table = Table { actions: &ACTIONS, menu: &MENU };

/// Makes the live bindings use this table (idempotent; the pack hands it over in a running game, tests call this).
pub fn install() {
	rac_engine::input::kbm::ensure_table(&TABLE);
}

type T5 = [&'static [u8]; 5];

/// Labels in En, Fr, De, Es, It, per action, in `ACTIONS` order.
#[rustfmt::skip]
pub static LABELS: [T5; 28] = [
	[b"Jump", b"Sauter", b"Springen", b"Saltar", b"Salta"],
	[b"OmniWrench", b"Super Cl\x8c", b"Omni-Schl\xa7ssel", b"Omnillave", b"Onnichiave"],
	[b"Fire weapon", b"Attaque d'armes", b"Waffen feuern", b"Disparar", b"Spara"],
	[b"Quick Select", b"S\x8clect-prompt", b"Schnellauswahl", b"Selecci\x9cn r\x84pida", b"Scelta Rapida"],
	[b"Look around (L1)", b"Observer (L1)", b"Herumschauen (L1)", b"Mirar alrededor (L1)", b"Guarda intorno (L1)"],
	[b"Crouch (R1)", b"S'accroupir (R1)", b"Hocke (R1)", b"Agachar (R1)", b"Accovacciati (R1)"],
	[b"Look around (L2)", b"Observer (L2)", b"Herumschauen (L2)", b"Mirar alrededor (L2)", b"Guarda intorno (L2)"],
	[b"Crouch (R2)", b"S'accroupir (R2)", b"Hocke (R2)", b"Agachar (R2)", b"Accovacciati (R2)"],
	[b"L3", b"L3", b"L3", b"L3", b"L3"],
	[b"Map (R3)", b"Carte (R3)", b"Karte (R3)", b"Mapa (R3)", b"Mappa (R3)"],
	[b"Pause", b"Pause", b"Pause", b"Pausa", b"Pausa"],
	[b"Map", b"Carte", b"Karte", b"Mapa", b"Mappa"],
	[b"Move forward", b"Avancer", b"Vorw\x87rts", b"Avanzar", b"Avanti"],
	[b"Move back", b"Reculer", b"R\xa7ckw\x87rts", b"Retroceder", b"Indietro"],
	[b"Move left", b"Aller \x85 gauche", b"Nach links", b"Ir a la izq.", b"Vai a sinistra"],
	[b"Move right", b"Aller \x85 droite", b"Nach rechts", b"Ir a la der.", b"Vai a destra"],
	[b"Camera up", b"Cam\x8cra haut", b"Kamera hoch", b"C\x84mara arriba", b"Telecamera su"],
	[b"Camera down", b"Cam\x8cra bas", b"Kamera runter", b"C\x84mara abajo", b"Telecamera gi\xa5"],
	[b"Camera left", b"Cam\x8cra gauche", b"Kamera links", b"C\x84mara izq.", b"Telecamera sinistra"],
	[b"Camera right", b"Cam\x8cra droite", b"Kamera rechts", b"C\x84mara der.", b"Telecamera destra"],
	[b"D-pad up", b"Croix haut", b"Steuerkreuz hoch", b"Cruceta arriba", b"Croce su"],
	[b"D-pad down", b"Croix bas", b"Steuerkreuz runter", b"Cruceta abajo", b"Croce gi\xa5"],
	[b"D-pad left", b"Croix gauche", b"Steuerkreuz links", b"Cruceta izq.", b"Croce sinistra"],
	[b"D-pad right", b"Croix droite", b"Steuerkreuz rechts", b"Cruceta der.", b"Croce destra"],
	[b"Aim (PC)", b"Viser (PC)", b"Zielen (PC)", b"Apuntar (PC)", b"Mira (PC)"],
	[b"Zoom in (PC)", b"Zoom + (PC)", b"Zoom + (PC)", b"Zoom + (PC)", b"Zoom + (PC)"],
	[b"Zoom out (PC)", b"Zoom - (PC)", b"Zoom - (PC)", b"Zoom - (PC)", b"Zoom - (PC)"],
	[b"Ship fire", b"Tir vaisseau", b"Schiffsfeuer", b"Disparo nave", b"Fuoco nave"],
];

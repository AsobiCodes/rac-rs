//! Shared, live player setting for the optional volumetric smoke (viewer/volsmoke.rs; with Lighting Enhanced).
//! Level 0 = Off (today's picture, default, zero cost), 1 Low, 2 Medium, 3 High. `RAC_SMOKE=off|low|medium|high|0..3` overrides
//! the setting (tests and the picture pass; unset = the setting). The pack's rows say which smoke the game has (`smoke_rows.rs`).
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::OnceLock;

static LEVEL: AtomicU8 = AtomicU8::new(0);

/// The highest level (High).
pub const MAX: u8 = 3;

/// Sets the player's level (values above `MAX` count as High).
pub fn set(level: u8) {
	LEVEL.store(level.min(MAX), Ordering::Relaxed);
}

/// The text of `RAC_SMOKE` as a level; None when it is not a level name.
pub fn parse(s: &str) -> Option<u8> {
	match s.trim().to_ascii_lowercase().as_str() {
		"0" | "off" | "classic" => Some(0),
		"1" | "low" => Some(1),
		"2" | "medium" | "med" => Some(2),
		"3" | "high" | "on" => Some(3),
		_ => None,
	}
}

fn env() -> Option<u8> {
	static E: OnceLock<Option<u8>> = OnceLock::new();
	*E.get_or_init(|| std::env::var("RAC_SMOKE").ok().and_then(|v| parse(&v)))
}

/// The level in force: `RAC_SMOKE` when set, else the player's setting.
pub fn level() -> u8 {
	env().unwrap_or_else(|| LEVEL.load(Ordering::Relaxed))
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn names_parse_and_levels_clamp() {
		assert_eq!((parse("off"), parse("Low"), parse(" 2 "), parse("HIGH"), parse("x")), (Some(0), Some(1), Some(2), Some(3), None));
		set(9);
		assert_eq!(LEVEL.load(Ordering::Relaxed), MAX);
		set(0);
		assert_eq!(LEVEL.load(Ordering::Relaxed), 0);
	}
}

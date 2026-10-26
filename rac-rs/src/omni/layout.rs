//! Which disc the mods are told they run on, and the data layout behind it.
//!
//! Our PAL v1.00 disc has the DATA layout of the binding's "ntsc" tables (pad 0x13C940, hero 0x13F350, items 0x13D4C0), not of its "pal" tables
//! (those are PAL v2.00, data +0x100): measured by the stage 2c-3 probe, see HANDOFF_2c3.md. So:
//! - native mods pick their address tables by the host's `disc` string: on this disc they are told "ntsc" (`RAC_OMNI_DISC=pal|ntsc` overrides);
//! - script mods keep "pal" when no native mod is present (so `games: [pal]` mods load), and then the binding the loader gets has its "pal"
//!   column replaced by the "ntsc" one (`pal_as_ntsc`), so `hero.*` and `bolts` read the right words either way;
//! - the host's own base_address / supports_field answers (host.rs) always use the ntsc column.

use super::json::Json;

/// (the disc string for the loader, the line to say in the run log if any). `usa`: the USA disc runs; `env`: RAC_OMNI_DISC; `any_native`: a mod
/// folder with a native library exists.
pub fn disc_string(usa: bool, env: Option<&str>, any_native: bool) -> (&'static str, Option<&'static str>) {
	match env.map(|e| e.trim().to_ascii_lowercase()).as_deref() {
		Some("pal") => return ("pal", Some("RAC_OMNI_DISC=pal: mods are told the pal disc (v2.00 addresses: wrong for this game, a debug switch)")),
		Some("ntsc") => return ("ntsc", Some("RAC_OMNI_DISC=ntsc: mods are told the ntsc disc")),
		_ => {}
	}
	if usa {
		("ntsc", None)
	} else if any_native {
		("ntsc", Some("PAL v1.00 disc: native mods get the ntsc address layout (same data layout)"))
	} else {
		("pal", None)
	}
}

/// Sets the "pal" value of `obj` to its "ntsc" value (when both exist).
fn pal_from_ntsc(obj: &mut Json) {
	if let Json::Obj(v) = obj {
		let Some(ntsc) = v.iter().find(|(k, _)| k == "ntsc").map(|(_, x)| x.clone()) else { return };
		if let Some(p) = v.iter_mut().find(|(k, _)| k == "pal") {
			p.1 = ntsc;
		}
	}
}

/// The binding text with every base and every field address of the "pal" column set to the "ntsc" one (hook symbols stay as they are).
pub fn pal_as_ntsc(text: &str) -> Result<String, String> {
	let mut j = super::json::parse(text)?;
	if let Json::Obj(top) = &mut j {
		for (k, v) in top.iter_mut() {
			match (k.as_str(), v) {
				("bases", Json::Obj(bases)) => bases.iter_mut().for_each(|(_, b)| pal_from_ntsc(b)),
				("fields", Json::Arr(fields)) => {
					for f in fields.iter_mut() {
						if let Json::Obj(fv) = f {
							fv.iter_mut().filter(|(k, _)| k == "address").for_each(|(_, a)| pal_from_ntsc(a));
						}
					}
				}
				_ => {}
			}
		}
	}
	Ok(j.to_text())
}

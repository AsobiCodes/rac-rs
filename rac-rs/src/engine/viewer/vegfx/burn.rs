//! [PC] Vegetation burns (stage 3 of the v0.5 fire extras): the grass clumps and plants vegfx.rs builds (Vegetation Moderate / Lush) catch
//! fire inside a ground fire after an explosion (firefx_blast.rs), stay burnt for a while and regrow. Retail's own shrubs and
//! billboards are untouched; nothing is written to retail RAM.
//!
//! CPU half: the burn MAP (vegfx/burnmap.rs, lane ngrass; it replaced the list of at most 16 round areas): a top-down grid around the
//! camera holding each texel's burn start on the fire's GAME clock (`FireFx.game.now()`, firefx_gametime.rs), painted by the
//! Pyrocitor's ground touches and every new ground fire patch (`BlastFire::ignite`) and kept after the patch is gone (a patch lives
//! ~11 s, the grass stays burnt ~60 s and regrows over ~15 s). Fire Low / Off, Classic lighting or Vegetation Off feed nothing (the
//! map is cleared, the uniform has count 0).
//! GPU half: `BurnU` is the tail of vegfx.rs' uniform (`VegU.burn`); the vertex shaders (vegfx/shader.rs `veg_burn`) read the map
//! texel under each blade or plant root (a ragged edge: a little jitter). A plant on a burnt texel lights up to `LAG_MAX` s after it
//! (its own hash), flames at its tip while it chars from the tip down, stays black, shorter and bent, then regrows.
//! No spreading beyond the painted texels; no per-frame CPU loop over plants. The timeline is `timeline` (the shader's mirror).
//! Dev knobs (`RAC_TUNE`, ENV.md): veg_burn_margin veg_burn_reach veg_burnt_time veg_regrow_time veg_char_h veg_tip_flame veg_burnt_h
//! veg_ember.

use bytemuck::{Pod, Zeroable};

use super::burnmap::LAG_MAX;

fn tune(name: &str, d: f32) -> f32 {
	crate::devtune::f32(name, d)
}

/// The timeline of an area, in seconds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::viewer) struct Times {
	/// Flames and charring (the ground fire's own burn time).
	pub burn: f32,
	/// Burnt, nothing moves.
	pub burnt: f32,
	/// Regrowth.
	pub regrow: f32,
}

impl Times {
	pub(in crate::viewer) fn tuned(burn: f32) -> Times {
		Times { burn, burnt: tune("veg_burnt_time", 60.0).max(1.0), regrow: tune("veg_regrow_time", 15.0).max(1.0) }
	}

	/// Age (s) after which a burn is gone for every plant.
	pub(in crate::viewer) fn total(&self) -> f32 {
		self.burn + self.burnt + self.regrow + LAG_MAX
	}
}

/// The uniform tail: row 0 = (1 = the map has a live burn / 0 = none, burn s, burnt s, regrow s), row 1 = the look (char height as a
/// fraction of the plant, tip flame height m, height left when burnt, ember strength), then the map (vegfx/burnmap.rs `pack`):
/// (world x, y of texel (0, 0), 1 / texel, texels per side), (now relative to the map's epoch, 0, 0, 0).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(in crate::viewer) struct BurnU {
	pub head: [f32; 4],
	pub look: [f32; 4],
	pub map: [[f32; 4]; 2],
	/// The Pyrocitor's touch spots on the grass (firefx_jet_grass.rs): jh.x = count (0 = the shader's jet loop never runs); per spot
	/// (place, reach) and (strength, stream direction xy, 0). Not a burn term: vegfx/shader.rs `veg_touch` reads them.
	pub jh: [f32; 4],
	pub jet: [[f32; 4]; 2 * MAX_JET_SPOTS],
}

/// The most Pyrocitor touch spots in the uniform.
pub(in crate::viewer) const MAX_JET_SPOTS: usize = 8;

impl BurnU {
	/// No burn: count 0, the shaders do nothing.
	pub(in crate::viewer) fn none() -> BurnU {
		BurnU::zeroed()
	}
}

/// The state of a plant at an age after its area lit (the shader's `veg_burn` mirror): char 0..1, tip flame 0..1.
#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq)]
struct State {
	char: f32,
	flame: f32,
}

#[cfg(test)]
fn ss(a: f32, b: f32, x: f32) -> f32 {
	let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
	t * t * (3.0 - 2.0 * t)
}

/// The mirror of the shader's timeline for a plant lit `t` seconds ago (None = not lit yet, or regrown).
#[cfg(test)]
fn timeline(t: f32, times: &Times) -> Option<State> {
	let regrow_at = times.burn + times.burnt;
	if t <= 0.0 || t > regrow_at + times.regrow {
		return None;
	}
	let char = ss(0.0, times.burn, t) * (1.0 - ss(0.0, 1.0, (t - regrow_at) / times.regrow));
	Some(State { char, flame: ss(0.0, 0.25, t) * (1.0 - ss(0.45 * times.burn, times.burn, t)) })
}

#[cfg(test)]
mod tests {
	use super::*;

	const T: Times = Times { burn: 3.0, burnt: 60.0, regrow: 15.0 };

	#[test]
	fn the_timeline_chars_stays_burnt_then_regrows_without_a_jump() {
		assert_eq!(timeline(0.0, &T), None, "not lit yet");
		let mid = timeline(1.5, &T).unwrap();
		assert!(mid.char > 0.3 && mid.char < 0.7 && mid.flame > 0.9, "{mid:?}");
		let done = timeline(3.0, &T).unwrap();
		assert!((done.char - 1.0).abs() < 1e-5 && done.flame < 1e-5, "burnt, flames out: {done:?}");
		assert_eq!(timeline(40.0, &T).unwrap().char, 1.0, "burnt for the whole 60 s");
		assert_eq!(timeline(63.0, &T).unwrap().char, 1.0, "regrowth starts after the burnt time");
		let half = timeline(63.0 + 7.5, &T).unwrap();
		assert!((half.char - 0.5).abs() < 1e-5, "half regrown at half the time: {half:?}");
		assert_eq!(timeline(78.1, &T), None, "regrown");
		// No jump anywhere (a step of 1/50 s changes the char by little) and the char only rises, plateaus, then falls.
		let mut last = 0.0;
		for k in 1..4000 {
			let c = timeline(k as f32 * 0.02, &T).map_or(0.0, |s| s.char);
			assert!((c - last).abs() < 0.05, "jump at {k}: {last} -> {c}");
			last = c;
		}
	}

	#[test]
	fn the_uniform_has_the_map_rows() {
		assert_eq!(std::mem::size_of::<BurnU>(), 16 * (2 + 2 + 1 + 2 * MAX_JET_SPOTS));
		let none = BurnU::none();
		assert_eq!((none.head, none.look, none.map), ([0.0; 4], [0.0; 4], [[0.0; 4]; 2]), "no burn: count 0, nothing else");
	}

	/// The veg shader with the burn part must validate and write HLSL (DX12 parity, like the level shader test in metalenv.rs).
	#[test]
	fn the_veg_shader_with_the_burn_part_validates_and_writes_hlsl() {
		let src = format!("{}{}", crate::viewer::volfog::soft::level_source(crate::viewer::SHADER, false), super::super::shader::WGSL);
		let m = naga::front::wgsl::parse_str(&src).unwrap_or_else(|e| panic!("{}", e.emit_to_string(&src)));
		let info = naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all()).validate(&m).expect("WGSL validates");
		for entry in ["vs_veg_blade", "vs_veg_mesh", "vs_veg_leaf", "fs_veg_blade", "fs_veg_mesh"] {
			assert!(m.entry_points.iter().any(|e| e.name == entry), "{entry}");
		}
		#[cfg(windows)]
		{
			let out = crate::viewer::metalpipe::hlsl(&src, true);
			assert!(out.contains("veg_burn"), "the burn function is in the HLSL");
		}
		let _ = info;
	}

	/// The WGSL names the timeline's constants of `timeline` (flames rise in 0.25 s, die from 45 % of the burn time), its uniform tail
	/// has the Rust struct's shape (2 vec4 + 2 map vec4) and it reads the burn map texture.
	#[test]
	fn the_shader_mirrors_the_timeline_and_the_uniform_shape() {
		let w = super::super::shader::WGSL;
		assert!(w.contains("veg_ss(0.0, 0.25, t) * (1.0 - veg_ss(0.45 * burn, burn, t))"), "flame timeline");
		assert!(w.contains("let p = veg_ss(0.0, burn, t);") && w.contains("let g = veg_ss(0.0, 1.0, (t - regrow_at) / veg.burn.head.w);"), "char and regrow");
		assert!(w.contains("map: array<vec4<f32>, 2>"), "uniform map rows");
		assert!(w.contains("@group(3) @binding(1) var veg_burnmap: texture_2d<f32>;") && w.contains("textureLoad(veg_burnmap"), "the map texture");
		assert!(w.contains(&format!("* {LAG_MAX:?};")), "the plant lag is LAG_MAX");
	}

	/// The lines (after `fn veg_fog`, i.e. in the vertex and fragment entry points) that use a burn term (`veg_burn(`, `veg_char(`,
	/// `veg_glow(`, `veg.burn.look`, `bn.x|y|z|w`) outside a block guarded by the uniform branch on the burnt count (`if (burning)` /
	/// `if (veg.burn.head.x > 0.0)`). A guarded one-line `if (..) { .. }` counts. Comment lines and the `o.bn = ..` varying are skipped.
	fn unguarded_burn_lines(wgsl: &str) -> Vec<String> {
		let from = wgsl.find("fn veg_fog").expect("fn veg_fog marks the start of the entry points");
		let (mut stack, mut bad): (Vec<bool>, Vec<String>) = (Vec::new(), Vec::new());
		for line in wgsl[from..].lines() {
			let t = line.trim();
			let guard = t.contains("if (burning)") || t.contains("if (veg.burn.head.x > 0.0)");
			let uses = ["veg_burn(", "veg_char(", "veg_glow(", "veg.burn.look", "bn.x", "bn.y", "bn.z", "bn.w"].iter().any(|k| t.contains(k));
			if uses && !t.starts_with("//") && !t.starts_with("o.bn") && !guard_or_inside(guard, &stack) {
				bad.push(t.to_string());
			}
			for ch in t.chars() {
				match ch {
					'{' => stack.push(guard),
					'}' => {
						stack.pop();
					}
					_ => {}
				}
			}
		}
		bad
	}

	fn guard_or_inside(guard: bool, stack: &[bool]) -> bool {
		guard || stack.iter().any(|g| *g)
	}

	/// m2 of the fireplus review: with no burnt area the grass shader must run the pre-burn code, so no burn call (and nothing computed
	/// only for it) may sit outside the uniform branch on the burnt count, in the vertex or the fragment stage.
	#[test]
	fn every_burn_term_of_the_veg_shader_sits_behind_the_burnt_count_branch() {
		let w = super::super::shader::WGSL;
		let bad = unguarded_burn_lines(w);
		assert!(bad.is_empty(), "burn terms outside `if (burning)` / `if (veg.burn.head.x > 0.0)`: {bad:?}");
		// The count-0 paths are the pre-burn expressions, and the branches exist in all four entry points.
		// (The fire light of flit.wgsl wraps the lit colour: `flit_add(lit, albedo, world, normal)`, no burn term in it.)
		assert!(w.contains("return vec4<f32>(mix(u.fog_colour.rgb, flit_add(i.c * k, i.c, i.w, vec3<f32>(0.0)), fog), 1.0);"), "blade fragment, no burn");
		assert!(w.contains("return vec4<f32>(mix(u.fog_colour.rgb, flit_add(t.rgb * shade, t.rgb, i.w, vec3<f32>(0.0)), fog), a);"), "mesh fragment, no burn");
		assert!(w.contains("var height = ib.y * f * touch.z * smoothstep(0.8, 2.4, d);"), "blade height, no burn");
		assert!(w.contains("var s = ib.y * mix(0.15, 1.0, f) * select(0.0, 1.0, f > 0.0) * touch.z * smoothstep(1.0, 3.0, d);"), "mesh scale, no burn");
		assert!(w.contains("let wind = lean * c.a * c.a * max(p.z, 0.0) * s;"), "mesh wind, no burn");
		assert_eq!(w.matches("let burning = veg.burn.head.x > 0.0;").count(), 2, "vs_veg_blade and vs_veg_mesh");
		assert_eq!(w.matches("if (veg.burn.head.x > 0.0) {").count(), 2, "fs_veg_blade and fs_veg_mesh");
		// The checker itself catches a call outside the branch (and accepts the guarded forms).
		let sample = "fn veg_fog() {}\nfn a() {\n let bn = veg_burn(c);\n if (burning) { z = bn.y; }\n if (veg.burn.head.x > 0.0) {\n  return veg_glow(i.bn);\n }\n return veg_char(c, i.bn);\n}\n";
		assert_eq!(unguarded_burn_lines(sample), vec!["let bn = veg_burn(c);".to_string(), "return veg_char(c, i.bn);".to_string()]);
	}
}

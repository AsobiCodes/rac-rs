//! [PC] The effects layer, waited for late (lane lows, v0.4.2). The VU1 thread builds a field's effects layer (the queue-A quads
//! and water pools of the VU1 owners, retail/efx_vu.rs) while the game thread runs the next field. `Retail::tick` used to wait
//! for it right after taking the field (t_wfx: 6.5 ms of the laptop's slowest frames on Pokitaru, laptop session 2026-10-29).
//! Now nothing reads queue A, queue B or the pools before `fx_late` (frame_upload.rs, after the hero, the props and their
//! upload), so the render thread does that work meanwhile.
//! Frame smoothing splits its per-field and per-frame work for it: `field` keeps the field and decides the cut at once and
//! leaves `still::hold`, the pairing and the weld to `fx_late`; `frame` blends every other list and leaves queue A / queue B to
//! `fx_late`. Every step still runs on the same lists in the same order, so the picture is the same.
//! RAC_FX_LATE=0 (A/B): the old order, the layer waited for right after the tick.

use super::*;

/// What `fx_late` still owes for the newest field (`State::late`).
pub(super) struct Late {
	/// The game's number of the field (logs).
	pub(super) field_no: u32,
	/// The alpha of the display frame `frame` blended since without queue A / queue B (None: it drew the field unblended).
	pub(super) alpha: Option<f32>,
}

/// RAC_FX_LATE=0: the effects layer is waited for right after each tick (`frame_ticks`), the order before lane lows.
pub(in crate::viewer) fn wait_in_tick() -> bool {
	static OFF: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
	*OFF.get_or_init(|| std::env::var("RAC_FX_LATE").is_ok_and(|v| v == "0"))
}

/// Waits for the effects layer of the field taken last if it is still out (`Retail::resolve_fx`; span t_wfx), then does what
/// `field` and `frame` left for it. Called before the frame's first reader of queue A, queue B or the pools (frame_upload.rs)
/// and before each tick (`frame_ticks`). Only a layer this `r` still had out is put into the kept field (`layer`): a `Late` left
/// from a game since dropped (the front end before the upload ran) is paired without one, never with another game's lists.
pub(in crate::viewer) fn fx_late(r: &mut Retail) {
	let layer = r.fx_pending();
	let ns = r.resolve_fx();
	if ns > 0 {
		crate::viewer::profile::span_add("t_wfx", ns as f32 / 1e6);
	}
	let _s = crate::viewer::profile::span("i_late");
	ST.with(|s| finish(&mut s.borrow_mut(), r, layer));
}

fn finish(s: &mut State, r: &mut Retail, layer: bool) {
	let Some(late) = s.late.take() else {
		return;
	};
	let prev = s.prev.take();
	let Some(mut f) = s.cur.take() else {
		s.prev = prev;
		return;
	};
	if layer {
		take_layer(&mut f, r.smooth_src().0);
	}
	pair_new(s, prev, f, late.field_no);
	// The display frame blended so far (`frame`, this frame's post phase) gets its queue A / queue B (and the pools it copies).
	if let (Some(t), Some(o)) = (late.alpha, r.smooth_fx_mut()) {
		patch(s, o, t);
	}
}

/// `Retail::resolve_fx` put the layer on the end of the snapshot's (`src`) queue-A and pool lists that `f` was copied from.
fn take_layer(f: &mut Field, src: Option<&RetailFx>) {
	if let (Some(c), Some(src)) = (f.fx.as_mut(), src) {
		// The kept copy is the game's lists before the layer (plus nothing): a longer copy would be another field's.
		debug_assert!(src.qa.len() >= c.qa.len() && src.pools.len() >= c.pools.len(), "effects layer: kept field is not a prefix");
		c.qa.extend_from_slice(src.qa.get(c.qa.len()..).unwrap_or_default());
		c.pools.extend_from_slice(src.pools.get(c.pools.len()..).unwrap_or_default());
	}
}

/// `o`, which `frame` blended without queue A / queue B, gets them (and the pools it copied before the layer was in), blended
/// at `t` as `frame` blends them when the layer is in.
fn patch(s: &State, o: &mut RetailFx, t: f32) {
	if let (Some(a), Some(c)) = (s.prev.as_ref().and_then(|p| p.fx.as_ref()), s.cur.as_ref().and_then(|c| c.fx.as_ref())) {
		(o.qa, o.qb, o.pools) = (c.qa.clone(), c.qb.clone(), c.pools.clone());
		blend_quads(o, a, t, s);
	}
}

/// Queue A / queue B of `o` (a copy of the newest field's, with `blend_fx`'s other lists) drawn at `t` between the field
/// before (`a`) and it: the moving quads' colours (`State::quads`), the standing ones (`State::still`), the welded corners.
pub(super) fn blend_quads(o: &mut RetailFx, a: &RetailFx, t: f32, s: &State) {
	lerp_quads(&a.qa, &mut o.qa, t, &s.quads[0]);
	lerp_quads(&a.qb, &mut o.qb, t, &s.quads[1]);
	still::lerp_still(&a.qa, &mut o.qa, t, &s.still[0]);
	still::lerp_still(&a.qb, &mut o.qb, t, &s.still[1]);
	weld::apply(&mut o.qa, &mut o.qb, t, &s.weld);
}

#[cfg(test)]
mod tests {
	use super::*;

	/// Four fields of queue-A / queue-B quads (standing, moving, coming and going kinds, as `still_kinds_matches_plain_rule`
	/// makes them), each with an effects layer of its own on the end of queue A. The late order (`cut_of` when the field comes,
	/// `frame`'s blend without the quads, then `take_layer`, `pair_new`, `patch`) gives the display frame what the order before
	/// lane lows gave it: the layer in before `advance`, `blend_fx` with the pairs, `still::lerp_still`, `weld::apply`.
	#[test]
	fn late_layer_draws_the_same() {
		let mut seed = 0x1357_9bdfu32;
		let mut rnd = move || {
			seed ^= seed << 13;
			seed ^= seed >> 17;
			seed ^= seed << 5;
			(seed % 10_000) as f32 / 10_000.0
		};
		let base: Vec<(usize, [f32; 3])> = (0..400).map(|i| (i % 6, [rnd() * 8.0, rnd() * 8.0, rnd() * 2.0])).collect();
		let quad = |kind: usize, o: [f32; 3], c: f32| WorldQuad {
			pos: [o, [o[0] + 0.4, o[1], o[2]], [o[0], o[1] + 0.4, o[2]], [o[0], o[1] + 0.4, o[2]]],
			uv: [[c, 0.5]; 4],
			colour: [[c, 1.0 - c, 0.5, 1.0]; 4],
			tex: 40 + kind / 2,
			additive: kind % 2 == 1,
		};
		let mut fields = Vec::new();
		for n in 0..4 {
			let mut lists: [Vec<WorldQuad>; 3] = Default::default();
			for (i, &(kind, o)) in base.iter().enumerate() {
				// Kinds 0-1 stand (jitter), 2-3 move, 4-5 come and go; every third quad is the late layer's, every fifth queue B.
				let mv = match kind {
					0 | 1 => 0.02 * rnd(),
					2 | 3 => 0.3 * n as f32,
					_ if (i + n) % 3 == 0 => continue,
					_ => 0.1,
				};
				let q = quad(kind, [o[0] + mv, o[1], o[2]], rnd());
				lists[if i % 5 == 0 { 1 } else if i % 3 == 0 { 2 } else { 0 }].push(q);
			}
			fields.push(lists);
		}
		let field = |l: &[Vec<WorldQuad>; 3], with_layer: bool| {
			let mut qa = l[0].clone();
			if with_layer {
				qa.extend_from_slice(&l[2]);
			}
			let fx = RetailFx { qa, qb: l[1].clone(), ..Default::default() };
			Field { fx: Some(fx), cam: Some(([0.0; 3], 0.0, 0.0, 1.0)), ..Default::default() }
		};
		let (mut old, mut new) = (State::default(), State::default());
		for (n, l) in fields.iter().enumerate() {
			let alpha = 0.3 + 0.1 * n as f32;
			// Before lane lows: the layer in at the tick, `advance`, the whole blend at once.
			advance(&mut old, field(l, true), n as u32);
			// Late: the field without its layer, the blend of the other lists, then the layer and the quads.
			let mut f = field(l, false);
			let prev = cut_of(&mut new, &mut f, n as u32);
			(new.prev, new.cur, new.late) = (prev, Some(f), Some(Late { field_no: n as u32, alpha: Some(alpha) }));
			let mut o = match (new.prev.as_ref().and_then(|p| p.fx.as_ref()), new.cur.as_ref().and_then(|c| c.fx.as_ref())) {
				(Some(a), Some(b)) => Some(blend_fx(a, b, alpha, &Default::default())),
				_ => None,
			};
			let mut f = new.cur.take().unwrap();
			let prev = new.prev.take();
			take_layer(&mut f, field(l, true).fx.as_ref());
			pair_new(&mut new, prev, f, n as u32);
			new.late = None;
			if let Some(o) = o.as_mut() {
				patch(&new, o, alpha);
			}
			assert_eq!(old.cut, new.cut, "field {n}");
			assert_eq!(old.quads, new.quads, "field {n}");
			let (a, b) = (old.prev.as_ref().and_then(|p| p.fx.as_ref()), old.cur.as_ref().and_then(|c| c.fx.as_ref()));
			let want = a.zip(b).map(|(a, b)| {
				let mut w = blend_fx(a, b, alpha, &old.quads);
				still::lerp_still(&a.qa, &mut w.qa, alpha, &old.still[0]);
				still::lerp_still(&a.qb, &mut w.qb, alpha, &old.still[1]);
				weld::apply(&mut w.qa, &mut w.qb, alpha, &old.weld);
				w
			});
			let bits = |q: &[WorldQuad]| q.iter().map(|q| (q.pos.map(|p| p.map(f32::to_bits)), q.colour.map(|c| c.map(f32::to_bits)), q.uv.map(|u| u.map(f32::to_bits)), q.tex, q.additive)).collect::<Vec<_>>();
			assert_eq!(want.is_some(), o.is_some(), "field {n}");
			if let (Some(w), Some(o)) = (want, o) {
				assert!(n > 0 && !w.qa.is_empty() && !w.qb.is_empty());
				assert_eq!(bits(&w.qa), bits(&o.qa), "queue A, field {n}");
				assert_eq!(bits(&w.qb), bits(&o.qb), "queue B, field {n}");
			}
		}
		assert!(old.quads.iter().any(|q| !q.is_empty()) && old.still.iter().any(|q| !q.is_empty()), "the fixture blends moving and standing quads");
	}
}

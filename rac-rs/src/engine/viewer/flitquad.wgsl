// [PC] Lane fshade (b), user 2026-11-08 ("Yes, both"): the fire-shadow term at half resolution, shared inside each 2x2 pixel quad
// (flitquad.rs `fire_text` puts this before flit.wgsl's `flit_light` in the fire modules on a device with subgroups). Every lane takes the quad's centre: the mean world
// point and the mean unit normal (`n` unflipped: flit_shadow turns it to the map's side itself) of its 4 pixels (helper lanes
// extrapolate along the triangle; worker 4 first took lane 0's point and normal: its extrapolated normal darkened silhouettes, max
// 121), lane q looks map q up (the up-to-4 maps over the 4 lanes: one lookup per pixel instead of up to 4) and the 4 results go to
// every lane (worker 3's emulated look pp\w3_crop_hpic.png, there with the top-left point and each pixel's own normal). The light loop
// then finds every map read (no lazy read). Returns -1 (the lazy per-pixel reads, the full-rate picture) where the quad is not whole:
// a lane discarded (SPIR-V OpKill) or not in this call (another branch), or a helper lane the ballot leaves out (DX12); a lane whose
// quad lane is not q = id & 3 is caught by the index each result carries. Quad-uniform flow: the ballot, the OR of the cell masks,
// the sums and the broadcasts run in the same block on every lane of a whole quad (the sums are bitwise equal on all 4: a + b = b + a).
// The subgroup invocation id, stored first by every fire-lit fragment entry point (flitquad.rs `entries`); never set = per pixel.
var<private> flq_sgi: u32 = 0xffffffffu;

fn flq_bits(b: vec4<u32>) -> u32 {
	let wi = flq_sgi >> 5u;
	let bw = select(select(b.x, b.y, wi == 1u), select(b.z, b.w, wi == 3u), wi >= 2u);
	return (bw >> (flq_sgi & 28u)) & 15u;
}

fn flit_quad(w: vec3<f32>, n: vec3<f32>, has_n: bool, m: u32) -> vec4<f32> {
	let none = vec4<f32>(-1.0);
	if (flq_sgi == 0xffffffffu) { return none; }
	if (flq_bits(subgroupBallot(true)) != 15u) { return none; }
	// No light in any of the 4 pixels' cells: nothing to read (the per-pixel loop has nothing either).
	var mq = m | quadSwapX(m);
	mq = mq | quadSwapY(mq);
	if (mq == 0u) { return none; }
	let q = flq_sgi & 3u;
	var ws = w + quadSwapX(w);
	ws = (ws + quadSwapY(ws)) * 0.25;
	var ns = n + quadSwapX(n);
	ns = ns + quadSwapY(ns);
	let hs = has_n && dot(ns, ns) > 1e-8;
	let nu = ns / sqrt(max(dot(ns, ns), 1e-16));
	// No spread/normal guard (worker 5: a per-pixel fallback for quads whose points or normals spread wide changed 0.15 % of the
	// pixels at the 4-map view, silhouette rims only, max diff vs today 120 with and without it, and cost ~0.5 ms there).
	let v = vec2<f32>(flit_shadow(q, ws, nu, hs), f32(q));
	let whole = flq_bits(subgroupBallot(true)) == 15u;
	let r0 = quadBroadcast(v, 0u);
	let r1 = quadBroadcast(v, 1u);
	let r2 = quadBroadcast(v, 2u);
	let r3 = quadBroadcast(v, 3u);
	if (!whole || any(vec4<f32>(r0.y, r1.y, r2.y, r3.y) != vec4<f32>(0.0, 1.0, 2.0, 3.0))) { return none; }
	return vec4<f32>(r0.x, r1.x, r2.x, r3.x);
}

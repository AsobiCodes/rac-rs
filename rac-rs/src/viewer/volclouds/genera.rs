//! [PC] The four cloud genera of the volumetric sky (WGSL, after layer.rs `BLOCK` and `sky::LOOKUP`, before layer.rs `CLOUDS`), each
//! with its own shape, altitude, thickness and coverage (`params::Look`):
//! - low and vertical, in the marched shell (`shell_density`): cumulus, the weather map's heaps (banks of merged big heaps with
//!   blue gaps, medium and small heaps round them; the coverage picks how many), wide and flat-topped with rounded shoulders, a
//!   soft flat base that rolls a little, ragged holes inside the banks (broken cloud overhead), the outline frayed over a broad
//!   rim; on levels with vertical development a few broad towers rise from the big heaps (`Look::tower`). And the stratocumulus
//!   deck, broad patches in soft rolls across the wind, low in the shell. Mostly coarse billows (broad, soft, painterly lobes),
//!   eroded by curled Worley detail at the thin edges.
//!   On the storm levels (`Look::form`) cumulonimbus towers on their own cell grid take the towers' place (`storm_cell`).
//! - middle (`mid_density`): the altocumulus / altostratus band, rippled wave bands across the wind with cells along each ripple,
//!   in patches; a sheet where the ripples are too fine to see.
//! - high (`high_opacity`): the thin veil of cirrus streaks (noise stretched along the wind and bent) and cirrocumulus, small cells
//!   in patches.
//!
//! The noise volumes have no mip levels, so detail finer than a grid pixel at that distance (`footprint`) fades to its mean: far
//! clouds keep their mass without shimmering.

/// WGSL: the shared helpers and the density of each genus.
pub const GENERA: &str = r#"
fn sat(x: f32) -> f32 { return clamp(x, 0.0, 1.0); }
fn remap(x: f32, a: f32, b: f32, c: f32, d: f32) -> f32 { return c + (x - a) / (b - a) * (d - c); }
fn centre() -> vec3<f32> { return vec3<f32>(cu.cam[0].x, cu.cam[0].y, cu.shell.w); }
// Height fraction in the cloud shell (0 base .. 1 the tallest towers' top).
fn height_of(p: vec3<f32>) -> f32 { return (length(p - centre()) - cu.shell.y) / (cu.shell.z - cu.shell.y); }
fn across(v: vec2<f32>) -> vec2<f32> { return vec2<f32>(-v.y, v.x); }
// Patches of a broad noise field v (about 0.5 +- 0.15) covering about the share c of the sky.
fn cover_of(v: f32, c: f32) -> f32 {
	if (c <= 0.0) { return 0.0; }
	let thr = 0.5 + (0.5 - c) * 0.45;
	return smoothstep(thr - 0.04, thr + 0.12, v);
}

// The width (world units) a grid pixel covers at distance t.
fn footprint(t: f32) -> f32 { return t * 2.0 * length(cu.cam[2].xyz) * cu.size.z; }

// Density, the height inside the cloud there (0 base .. 1 top; darker undersides, wisps low and billows high), the envelope
// (above 0 inside a cloud's outline before erosion: the march takes fine steps there) and the skin (1 at a billow's surface, 0
// deep inside: the sky lights the outer skin of each lobe more, so the lobes of a shaded underside still read).
struct Den { d: f32, hr: f32, e: f32, k: f32 };

// A point in the wind's frame (x along the wind): the weather map's rows of heaps (cloud streets) run along the wind.
fn wind_frame(v: vec2<f32>) -> vec2<f32> { return vec2<f32>(dot(v, cu.flow.xy), dot(v, across(cu.flow.xy))); }

// A slow, smooth seed field (periods of several weather tiles) that shifts where each region of the sky reads the shape volumes:
// heaps and towers far apart never share one silhouette (the volumes tile every few kilometres).
fn region_seed(pr: vec2<f32>) -> vec2<f32> {
	let a = textureSampleLevel(weather, nsamp, pr / (cu.scale.x * 3.7) + vec2<f32>(0.61, 0.29), 0.0).b;
	let b = textureSampleLevel(weather, nsamp, pr.yx / (cu.scale.x * 4.3) + vec2<f32>(0.13, 0.87), 0.0).b;
	return vec2<f32>(a, b);
}

// [Storm] The cumulonimbus form (`form.x`, Hoven / Oltanis) in place of the fair-weather towers: towers on their own grid of cells
// (one candidate per cell, at a random place in its middle; the grid never repeats), each on a tall heap, of its own height and
// width, rising from the shell's base as a column whose sides swell with the coarse 3D lobes up to a rounded cauliflower crown,
// all leaning with a slow lean field (more toward the top: the shear differs from region to region). The heaps round them keep one
// base (no flat-bottomed blocks floating at different heights, no brims).
struct Storm { top: f32, dist: f32, rb: f32 };
fn storm_hash(c: vec2<i32>) -> vec4<f32> {
	var v = vec4<u32>(bitcast<u32>(c.x), bitcast<u32>(c.y), 0x2545f491u, 0x9e3779b9u) * 1664525u + 1013904223u;
	v.x += v.y * v.w; v.y += v.z * v.x; v.z += v.x * v.y; v.w += v.y * v.z;
	v = v ^ (v >> vec4<u32>(16u));
	v.x += v.y * v.w; v.y += v.z * v.x; v.z += v.x * v.y; v.w += v.y * v.z;
	return vec4<f32>(v) * 2.3283064e-10;
}
// The tower whose cell holds pr at height h: its top (shell fraction; 0 = none near), the horizontal distance from its axis and its
// base radius (world units). wv = the smooth warp field there (two slow Perlin taps): the lean.
fn storm_cell(pr: vec2<f32>, h: f32, wv: vec2<f32>) -> Storm {
	let thick = cu.shell.z - cu.shell.y;
	let pl = pr + (wv - 0.5) * (h * h * thick * 1.6);
	let g = cu.scale.y * 1.6;
	let c = vec2<i32>(floor(pl / g));
	let a = storm_hash(c);
	let b = storm_hash(c * 3 + vec2<i32>(1, 2));
	let ctr = (vec2<f32>(c) + 0.35 + a.xy * 0.3) * g;
	let rb = mix(1000.0, 1700.0, b.x) * cu.scale.y / 4500.0;
	let d = length(pl - ctr);
	if (d >= rb * 1.9) { return Storm(0.0, d, rb); }
	// (Mostly on the heaps (their outline at the tower's axis), a few alone in the gaps.)
	let wc = textureSampleLevel(weather, nsamp, ctr / cu.scale.x, 0.0).r;
	let on = cu.genus.w * step(b.y, 0.55 + 0.4 * smoothstep(0.05, 0.4, wc));
	return Storm(select(0.0, 0.006 + on * mix(0.42, 0.95, a.z), on > 0.0), d, rb);
}
// The tower's field at height h (on the heaps' field's scale: the surface at 0.4) from the coarse shape sample s and the billows
// (the coarse billows) bil; below 0 = no tower here.
fn storm_field(c: Storm, h: f32, hd: f32, s: vec4<f32>, bil: f32) -> f32 {
	if (c.top <= max(hd, 0.05)) { return -2.0; }
	let ht = sat((h - 0.006) / (c.top - 0.006));
	// (Slightly narrower up the column, the crown a rounded dome of turrets; the coarse lobes swell and narrow the sides.)
	let cr = sat((ht - 0.5) / 0.5);
	let rr = c.rb * mix(1.0, 0.8, ht) * sqrt(max(1.0 - cr * cr, 0.0)) * (1.0 + (s.g - 0.45) * 0.6 + (s.r - 0.5) * 0.5);
	let edge = smoothstep(0.26, 0.33, c.dist / (cu.scale.y * 1.6));
	return 0.4 + (1.0 - c.dist / max(rr, 1.0)) * 1.6 + (bil - 0.48) * 2.6 * mix(0.8, 1.3, ht) - edge * 2.0;
}

// Cumulus and stratocumulus at world point p, height fraction h, grid pixel footprint fp; `cheap` leaves the detail out.
fn shell_density(p: vec3<f32>, h: f32, fp: f32, cheap: bool) -> Den {
	var o = Den(0.0, 1.0, 0.0, 1.0);
	if (h <= 0.0 || h >= 1.0) { return o; }
	let wd = cu.flow.xy;
	// Drifting with the wind, the tops leaning downwind (shear).
	let pw = p.xy - wd * (h * cu.wind.w) - cu.wind.xy;
	let pr = wind_frame(pw);
	// A coarse shape sample (it breaks the deck's patches; the heaps' outlines are bent by a smooth warp: lumpy, not round).
	let ws = textureSampleLevel(shape, nsamp, vec3<f32>(pr / (cu.scale.y * 1.7), 0.15), 0.0);
	// (A smooth warp, from the Perlin deck field, gentle enough never to fold: the Worley cells' creases, or a warp steeper than
	// its wavelength, pinch the outlines into straight vertical fins.)
	let wq = pr / cu.scale.x;
	let wv = vec2<f32>(textureSampleLevel(weather, nsamp, wq, 0.0).b, textureSampleLevel(weather, nsamp, wq.yx + vec2<f32>(0.43, 0.71), 0.0).b);
	let warp = (wv - 0.5) * cu.scale.y * 0.3;
	let w = textureSampleLevel(weather, nsamp, (pr + warp) / cu.scale.x, 0.0);
	// Cumulus: the heaps the coverage shows (the weather map holds their union: R near, A with the far coverage, which takes over
	// with distance so the clouds pile into a band low in the sky, as on the painted skies) and G, the union of their domes' heights
	// (each heap's own: a small one never takes a big neighbour's height), the biggest towering with the level's vertical
	// development, on a flat base that lifts only a little toward the edge. Heaps too small for the march steps at this distance
	// fade out (no hit-and-miss flicker).
	let thick = cu.shell.z - cu.shell.y;
	// (G holds the domes of the heaps shown near; the far band's extra heaps take a bank's dome from their own outline.)
	let fband = smoothstep(12000.0, 40000.0, length(p - cu.cam[0].xyz));
	let r = mix(w.r, w.a, fband);
	// (Heaps too small for the grid at this distance sink: the dome is lowered by a few grid pixels, so a small heap shrinks and
	// goes smoothly; a mask by the local height would cut its thin rim off in a vertical wall.)
	let hgt = max(mix(w.g * cu.scale.x, max(w.g * cu.scale.x, w.a * (2.0 - w.a) * thick * 0.3) * 0.6, fband) - 3.0 * fp, 0.0);
	let lc = smoothstep(0.0, 2.0 * fp, hgt);
	let hgn = hgt / max(cu.scale.x / 16800.0, 0.1);
	let hd = min(hgt / thick, 0.85);
	// (Towers: only on the tall big heaps, a few narrow ones round the coarse cells' points (not the whole heap rising as one
	// column), their tops in big lumps; the level's vertical development sets how many and how tall.)
	let tw = cu.genus.w * smoothstep(380.0, 700.0, hgn);
	let rs = region_seed(pr);
	let wt = textureSampleLevel(shape, nsamp, vec3<f32>(pr / (cu.scale.y * 4.0) + rs * 5.0, 0.45 + rs.x * 0.4), 0.0).g;
	let spot = sat((wt + (cu.genus.w - 0.5) * 0.1 - 0.5) / 0.3);
	let stm = cu.form.x > 0.0;
	var sc = Storm(0.0, 0.0, 1.0);
	if (stm) { sc = storm_cell(pr, h, wv); }
	let towr = select(min(tw * tw * (1.0 - (1.0 - spot) * (1.0 - spot)) * 0.45, 1.0 - hd) * smoothstep(0.15, 0.5, r), 0.0, stm);
	let dome = select(hd + towr * 1.3, max(hd, sc.top), stm);
	// (The base lifts toward the edge and rolls with the smooth warp field: no flat, even ceiling seen from below.)
	// (Not under a tower: a tall heap whose base lifts in one part reads as a flat brim over a stem, a hat.)
	let b0 = select(0.006 + hd * 0.12 * (1.0 - r) + (sat(wv.y - 0.3) * 0.2 + sat(0.65 - wv.x) * 0.3) * hd * (1.0 - 0.85 * sat(tw * 2.0)), 0.006 + hd * 0.03 * (1.0 - r), stm);
	// A finer breakup field (low = a hole), mostly inside the merged banks: gaps and ragged holes, broken cloud overhead.
	let wbk = textureSampleLevel(weather, nsamp, pr / (cu.scale.x * 0.13) + vec2<f32>(0.17, 0.53), 0.0).b;
	let hob = mix(0.35, 1.0, smoothstep(0.45, 0.85, r));
	let env = smoothstep(b0, b0 + 0.05, h) * (1.0 - smoothstep(dome * 0.95, dome * 1.25, h));
	// Stratocumulus: the deck in rolls across the wind, their tops rounded, the bottom softly uneven.
	let roll = 0.5 + 0.5 * sin((dot(pw, across(wd)) / cu.flow.w + w.b * 2.5) * 6.2831853);
	let scov = cu.cover.y;
	// The deck's patches broken into lumps and holes by the same coarse noise (no flat lens with a smooth outline).
	let lsc = cover_of(w.b * mix(0.94, 1.03, roll) + (ws.r - 0.5) * 0.16, scov);
	let th = cu.flow.z * (0.55 + 0.45 * roll);
	let psc = smoothstep(0.004 + 0.02 * (1.0 - roll), th * 0.3, h) * (1.0 - smoothstep(th * 0.45, th, h));
	o.e = max(lc * env * step(0.001, r), lsc * psc);
	if (stm && h < sc.top * 1.1) { o.e = max(o.e, smoothstep(b0, b0 + 0.05, h)); }
	if (o.e <= 0.0) { return o; }
	let tm = cu.wind.z;
	let q = vec3<f32>(pw, p.z) / cu.scale.y + vec3<f32>(rs * 6.0, tm * cu.scale.w);
	let s = textureSampleLevel(shape, nsamp, q, 0.0);
	// Billows: inverted Worley cells (round lobes) swell the outline into cauliflower, at a size that follows the heap's (a finer
	// copy of the shape volume for the small and medium heaps), the finer sizes fading to their mean where the grid cannot resolve
	// them.
	let sf = textureSampleLevel(shape, nsamp, q * 3.3 + vec3<f32>(0.37, 0.11, 0.0), 0.0);
	// (Faded by the march step there, not the grid pixel only: cells finer than the steps only add noise. A step is several grid
	// pixels long, so this covers the footprint too.)
	let st = max(thick, length(p - cu.cam[0].xyz) * 1.2) * 0.5 / cu.march.x;
	let fa = smoothstep(cu.scale.y * 0.015, cu.scale.y * 0.04, st);
	let fb = smoothstep(cu.scale.y * 0.03, cu.scale.y * 0.08, st);
	let fc = smoothstep(cu.scale.y * 0.009, cu.scale.y * 0.024, st);
	// (Mostly the coarse lobes: broad, soft, painterly forms; the finer cells only add a little texture.)
	let bbig = s.g * 0.68 + mix(s.b, 0.48, fb) * 0.24 + mix(s.a, 0.48, fa) * 0.08;
	let bsmall = mix(sf.g, 0.48, fa) * 0.75 + mix(sf.b, 0.48, fc) * 0.25;
	// (A storm's heaps all take the coarse lobes: no smooth lens among the towers.)
	let bil = select(mix(bsmall, bbig, smoothstep(120.0, 350.0, hgn)), bbig, stm);
	let low = mix(s.g * 0.625 + s.b * 0.25 + s.a * 0.125, s.g * 0.7 + 0.15, fb);
	let base = sat(remap(s.r, -(1.0 - low), 1.0, 0.0, 1.0));
	// The heap's field: 1 in its core, falling to 0 at its radius and toward the dome's top (the billows decide where the
	// surface lies inside that band: lobes and turrets on top, lumpy sides, the flat base kept).
	let dtop = hd + towr * (0.85 + 0.3 * ws.g);
	let hn = sat((h - b0) / max(dtop - b0, 0.01));
	// (The field falls off from the heap's side, its dome and its base at about the same rate in world units, so the billows swell
	// into round lobes, neither vertical teeth nor flat-topped boxes.)
	let vx = max(dtop - b0, 0.01);
	let fcu = sat((dtop - h) / min(vx * 0.5, 450.0 * cu.scale.y / 3000.0 / thick));
	let gate = smoothstep(0.03, 0.3, fcu);
	// (The base ramp has one height in world units: a ramp that grew with the column would lift the middle of the base over its
	// edges, a concave underside with hanging rims: mushrooms.)
	// (A storm's base is deeper and lumpy: a soft, uneven dark underside, no flat disc seen from below.)
	let wb = select(200.0, 320.0 * (1.0 + (0.5 - s.g) * 0.9), stm) * cu.scale.y / 3000.0 / thick;
	let pb = sat((b0 + wb - h) / wb);
	// (And it falls off toward the heap's outline over a broad rim, so the billows fray the edge softly: no sharp round outline. The
	// outline draws in toward the top (rounded shoulders, no vertical walls under a flat lid) and a little at the base, and swells
	// and shrinks with the coarse 3D lobes, so a side is never one 2D shape pulled straight up.)
	let side = sat((r - 0.3 * hn * hn - select(0.18, 0.05, stm) * (1.0 - hn) * (1.0 - hn) * (1.0 - hn) + (s.g - 0.45) * 0.3) / 0.3);
	// (The holes thin the field through the cloud's whole height, their walls swelling with the coarse 3D lobes: a base lifted
	// by a 2D field would end in a cut vertical wall.)
	let ho = smoothstep(0.56, 0.3, wbk + (s.r - 0.5) * 0.25) * hob;
	var dfield = min(fcu, side * 1.2) - ho * 1.1 + ((bil - 0.48) * 2.6 + (base - 0.5) * 0.3) * gate * mix(0.6, 1.0, side) - pb * pb * 1.6;
	var hnl = hn;
	var lct = lc;
	if (stm) {
		let tf = storm_field(sc, h, hd, s, bbig) - ho * 0.6 - pb * pb * 1.6;
		if (tf > dfield) { dfield = tf; hnl = sat((h - b0) / max(sc.top - b0, 0.01)); lct = 1.0; }
	}
	// (A fairly crisp surface: solid painted masses, the thin edges frayed by the erosion below.)
	let dcu = sat((dfield - 0.4) / (0.26 + cu.form.w)) * lct;
	let dsc = sat(remap(base * psc, 1.0 - lsc, 1.0, 0.0, 1.0)) * lsc * 0.75;
	var d = max(dcu, dsc);
	o.hr = select(sat(h / max(th, 0.01)), hnl, dcu >= dsc);
	o.k = select(1.0, sat(1.0 - (dfield - 0.4) / (0.5 + cu.form.w)), dcu >= dsc);
	if (d <= 0.0) { return o; }
	if (!cheap) {
		// Detail erosion, curled by the shape's own cells: frayed wisps at the base and the thin edges, small billows higher;
		// its mean where the grid cannot resolve it.
		let ef = (1.0 - smoothstep(cu.scale.z * 0.06, cu.scale.z * 0.25, fp)) * (1.0 - smoothstep(cu.scale.z * 0.25, cu.scale.z * 0.8, st));
		var m = 0.5;
		if (ef > 0.0) {
			let curl = (s.gb - 0.5) * 0.8;
			let dq = vec3<f32>(pw + curl * cu.scale.z, p.z) / cu.scale.z + vec3<f32>(0.0, 0.0, tm * cu.scale.w * 3.0);
			let dn = textureSampleLevel(detail, nsamp, dq, 0.0).rgb;
			let hf = dn.r * 0.625 + dn.g * 0.25 + dn.b * 0.125;
			m = mix(0.5, mix(hf, 1.0 - hf, sat(o.hr * 3.0)), ef);
		}
		// (A graded level's bodies are softer, less frayed: `alt.w`.)
		d = sat(remap(d, m * cu.cover.w * mix(1.0, 0.6, cu.alt.w) * (1.0 - 0.5 * d), 1.0, 0.0, 1.0));
	}
	o.d = d;
	return o;
}

// Altocumulus: rippled wave bands across the wind (cells along each ripple) in patches, in the band alt.x .. alt.x + alt.y.
fn mid_density(p: vec3<f32>, fp: f32) -> f32 {
	let hm = (length(p - centre()) - cu.alt.x) / cu.alt.y;
	if (hm <= 0.0 || hm >= 1.0) { return 0.0; }
	let wd = cu.flow.xy;
	let pw = p.xy - cu.wind.xy * 1.3;
	let w = textureSampleLevel(weather, nsamp, pw / (cu.scale.x * 2.3) + vec2<f32>(0.31, 0.77), 0.0);
	let lc = cover_of(w.b, cu.genus.x);
	if (lc <= 0.0) { return 0.0; }
	let lam = cu.flow.w * 0.25;
	let warp = textureSampleLevel(shape, nsamp, vec3<f32>(pw / (lam * 14.0), 0.25), 0.0).r;
	let band = 0.5 + 0.5 * sin((dot(pw, across(wd)) / lam + warp * 4.0) * 6.2831853);
	let cells = textureSampleLevel(shape, nsamp, vec3<f32>(dot(pw, wd) / (lam * 9.0), dot(pw, across(wd)) / (lam * 6.0), 0.6 + hm * 0.05), 0.0).g;
	// Far away the ripples fade into the coarse lumps of their warp (soft patches, not a flat sheet with a crisp outline).
	let v = mix(band * (0.15 + 0.85 * cells), warp * 0.75, smoothstep(lam * 0.08, lam * 0.4, fp));
	let prof = 4.0 * hm * (1.0 - hm);
	// Only the ripples' crests even in a patch's middle (broken cells, not a filled oval), thinning out toward its edge.
	return sat(remap(v * prof, 1.0 - 0.6 * lc, 1.0, 0.0, 1.0)) * sqrt(lc) * 0.85;
}

// The high veil's opacity at p (on the sphere alt.z): cirrus streaks and cirrocumulus cells.
fn high_opacity(p: vec3<f32>, fp: f32) -> f32 {
	let wd = cu.flow.xy;
	let pw = p.xy - cu.wind.xy * 2.0;
	let w = textureSampleLevel(weather, nsamp, pw / (cu.scale.x * 3.1) + vec2<f32>(0.63, 0.19), 0.0);
	var ci = 0.0;
	if (cu.genus.y > 0.0) {
		let bend = textureSampleLevel(shape, nsamp, vec3<f32>(pw / (cu.scale.x * 1.1), 0.4), 0.0).r;
		let su = vec2<f32>(dot(pw, wd) / (cu.scale.x * 1.2), dot(pw, across(wd)) / (cu.scale.x * 0.1) + bend * 1.2);
		let st = textureSampleLevel(shape, nsamp, vec3<f32>(su, 0.8), 0.0);
		let fine = mix(st.b, 0.5, smoothstep(cu.scale.x * 0.004, cu.scale.x * 0.016, fp));
		// Toward a patch's edge the streaks thin out one by one (no oval outline).
		let lci = cover_of(w.b, cu.genus.y);
		ci = sat((st.r * 0.75 + fine * 0.25 - (0.66 - 0.21 * lci)) / 0.3) * sqrt(lci);
	}
	var cc = 0.0;
	if (cu.genus.z > 0.0) {
		// A field of small grains of two sizes in faint ripples across the wind; toward a patch's edge the grains thin out and
		// shrink (no outline of round cells).
		let cs = cu.flow.w * 0.16;
		let bend = textureSampleLevel(shape, nsamp, vec3<f32>(pw / (cs * 20.0), 0.55), 0.0).r;
		let rip = 0.5 + 0.5 * sin((dot(pw, across(wd)) / (cs * 1.7) + bend * 3.0) * 6.2831853);
		let g1 = textureSampleLevel(detail, nsamp, vec3<f32>(pw / (cs * 4.0), 0.3), 0.0).g;
		let g2 = textureSampleLevel(detail, nsamp, vec3<f32>(pw / (cs * 2.3), 0.7), 0.0).b;
		let grains = mix((g1 * 0.65 + g2 * 0.35) * (0.88 + 0.12 * rip), 0.4, smoothstep(cs * 0.1, cs * 0.45, fp));
		let wc = textureSampleLevel(weather, nsamp, pw / (cu.scale.x * 2.6) + vec2<f32>(0.27, 0.52), 0.0).b;
		let lc = cover_of(wc, cu.genus.z);
		cc = sat((grains - (0.58 - 0.16 * lc)) / 0.16) * sqrt(lc);
	}
	return max(ci * 0.5, cc * 0.5);
}
"#;

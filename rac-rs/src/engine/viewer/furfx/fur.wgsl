// [PC] Fur shells (viewer/furfx.rs). Appended to the level shader (SHADER + volfog WORLD + moby_add): `VOut`, `vs_main`, `u`, `tex`,
// `samp`, `sun_lit` are the level pass's own, so a shell is lit like the skin under it.
//
// One instanced draw per fur texture run: instance k is shell k + 1 of N (the skin is shell 0, drawn opaque before). A vertex moves
// along its (welded) normal by len * h (h = (k + 1) / N) and its tip leans down along the surface by comb * h^2 (combed fur). The
// shells are blended (premultiplied alpha, inner to outer, no depth write) after the opaque scene. Strand coverage comes from a
// small tiling noise texture with mipmaps (furfx/noise.rs), sampled trilinear + anisotropic at two scales: fine strands up close,
// a smooth plush layer from afar, nothing that shimmers. The opacity is spread over the N shells so every quality level shows the
// same fur; where the surface turns away from the view the shells get denser (the soft fuzzy outline). The game's fur map
// (binding 3, `FurMask::map`) scales the length and the outline per texel and sets the tone (a face: short fur that keeps the skin's
// shading on the muzzle, none on the eye rims, long fur on the cheeks).

struct FurPart {
	// x: length in m at the top shell, y: noise tiles per uv unit, z: comb (tip lean down the surface, m), w: shells N
	a: vec4<f32>,
	// rgb: tip colour multiplier, w: root shade (0 = none, 1 = black)
	b: vec4<f32>,
	// x: opacity (1 = the strand coverage), y: sheen strength, z: style (0 Soft, 1 Fluffy), w: weld flag (1 = shared normals)
	c: vec4<f32>,
	// x: outline (halo) opacity boost, y: root self-shadow (0 = none), z: strand highlight strength, w: highlight exponent
	d: vec4<f32>,
	// x: tip lift (towards the colour's lighter shade), y: strand sharpening (coverage reaching full opacity at this value, 0 = off),
	// z: strand shading (darker strand edges)
	e: vec4<f32>,
}
@group(2) @binding(0) var<uniform> fp: FurPart;
@group(2) @binding(1) var<storage, read> fur_vb: array<f32>;
@group(2) @binding(2) var<storage, read> fur_weld: array<u32>;
// r: fur, g: length scale and b: halo scale (x 255 / 128: furspec ZONE_ONE), a: tone (1 = the part's look, 0 = the skin's shading)
@group(2) @binding(3) var fur_mask: texture_2d<f32>;
@group(2) @binding(4) var fur_samp: sampler;
@group(2) @binding(5) var fur_noise: texture_2d<f32>;

@vertex fn vs_fur(@location(0) p: vec3<f32>, @location(1) uv: vec2<f32>, @location(2) n: vec3<f32>, @location(3) c: vec4<f32>, @location(4) l: vec4<f32>, @builtin(vertex_index) vi: u32, @builtin(instance_index) sh: u32) -> VOut {
	let n_shells = max(fp.a.w, 1.0);
	let h = (f32(sh) + 1.0) / n_shells;
	var nn = n;
	if (fp.c.w > 0.5) {
		let b = fur_weld[vi] * 16u + 5u;
		nn = vec3<f32>(fur_vb[b], fur_vb[b + 1u], fur_vb[b + 2u]);
	}
	let nl = length(nn);
	nn = select(vec3<f32>(0.0, 0.0, 1.0), nn / max(nl, 1.0e-6), nl > 1.0e-4);
	// Comb: gravity projected on the surface; where the surface faces straight up or down there is no lean.
	let down = vec3<f32>(0.0, 0.0, -1.0);
	let t = down - nn * dot(down, nn);
	let comb = t * (fp.a.z / max(length(t), 0.3));
	// The fur map's length scale at this vertex: the shell's height above the skin in units of the part's length.
	let hl = h * textureSampleLevel(fur_mask, fur_samp, uv, 0.0).g * (255.0 / 128.0);
	// The hero's afterimage copies are not drawn at all (furfx/pick.rs `ghost_cut`), so every vertex here is real hero geometry.
	var o = vs_main(p + nn * (fp.a.x * hl) + comb * (h * hl), uv, n, c, l);
	o.fogf = hl;
	o.kmx = vec2<f32>(f32(sh), -1.0);
	return o;
}

// Strand coverage at height h (0 skin .. 1 longest tip) from the four noise channels (1/8, 3/8, 5/8, 7/8 of the length).
// `dx` / `dy`: the derivatives of `g`, taken before the early discard (an implicit-derivative sample may not follow a discard).
fn fur_cov(g: vec2<f32>, dx: vec2<f32>, dy: vec2<f32>, h: f32) -> f32 {
	let s = textureSampleGrad(fur_noise, fur_samp, g, dx, dy);
	let x = clamp(h * 4.0 - 0.5, 0.0, 3.5);
	let w = max(vec4<f32>(0.0), vec4<f32>(1.0) - abs(vec4<f32>(x) - vec4<f32>(0.0, 1.0, 2.0, 3.0)));
	return dot(s, w);
}

@fragment fn fs_fur(i: VOut) -> @location(0) vec4<f32> {
	let base = textureSample(tex, samp, i.uv);
	let fm = textureSample(fur_mask, fur_samp, i.uv);
	// Height in units of the strand length here (the vertices' scale can differ from this texel's): above 1 the strand has ended.
	let sf = fm.g * (255.0 / 128.0);
	let hr = i.fogf / max(sf, 0.02);
	let tone = fm.a;
	// Two octaves, the second rotated and finer, so the tile never repeats visibly.
	let g = i.uv * fp.a.y;
	let g2 = vec2<f32>(g.x * 0.8 - g.y * 0.6, g.x * 0.6 + g.y * 0.8) * 1.73 + vec2<f32>(0.37, 0.71);
	let gx = dpdx(g);
	let gy = dpdy(g);
	let g2x = dpdx(g2);
	let g2y = dpdy(g2);
	// Early out: a texel the alpha below zeroes anyway (no fur, cut-out skin, no strand length, past the strand's tip) skips the noise,
	// the light and the blend. Each test is where its factor in `a` is 0 (the tip's at most rounding above), so the `a < 0.004` test
	// below would drop the texel anyway: the picture is unchanged.
	if (fm.r <= 0.2 || base.a < 0.25 || sf <= 0.03 || hr >= 1.15) { discard; }
	let h = clamp(hr, 0.0, 1.0);
	let c1 = fur_cov(g, gx, gy, h);
	let c2 = fur_cov(g2, g2x, g2y, h);
	let cov0 = (1.0 - (1.0 - c1) * (1.0 - 0.7 * c2));
	// Sharper strands: thin tips reach full opacity (a smooth function of the filtered coverage: from afar it stays smooth).
	// Where a strand gets thinner than ~2 pixels (from afar, at grazing angles) the filtered coverage is an average and stays as is,
	// so the shells never turn into solid sheets.
	let px = max(length(gx), length(gy)) * 85.0;
	let sharp = mix(fp.e.y, 1.0, smoothstep(0.25, 0.6, px));
	let cov = select(cov0, smoothstep(0.0, sharp, cov0), fp.e.y > 0.0);
	// A zero interpolated normal (collapsed or degenerate vertices) would give NaN rim / density / alpha: fall back to up.
	let nlen = length(i.n);
	let nrm = select(vec3<f32>(0.0, 0.0, 1.0), i.n / max(nlen, 1.0e-6), nlen > 1.0e-4);
	let view = normalize(u.eye.xyz - i.w);
	let ndv = abs(dot(nrm, view));
	let rim = 1.0 - ndv;
	// The layer's opacity, spread over the shells (8 = reference count), denser at the outline, soft at the mask's edge.
	let dens = clamp(cov * fp.c.x * (1.0 + fp.d.x * fm.b * (255.0 / 128.0) * rim * rim), 0.0, 0.995);
	var a = 1.0 - pow(1.0 - dens, 8.0 / max(fp.a.w, 1.0));
	a *= smoothstep(0.2, 0.8, fm.r) * step(0.25, base.a) * smoothstep(0.03, 0.12, sf) * (1.0 - smoothstep(1.02, 1.15, hr));
	a *= 1.0 - smoothstep(1.0, 1.08, hr);
	if (a < 0.004) { discard; }
	// Colour: the skin's texel, a slight shade at the root, warmer and lighter towards the tip (by the tone); the tips also lift
	// towards the colour's own lighter shade, so the dark stripes show strands too.
	let tipc = base.rgb * fp.b.rgb + (sqrt(base.rgb) - base.rgb) * fp.e.x;
	let col = base.rgb * mix(vec3<f32>(1.0 - fp.b.w), mix(vec3<f32>(1.0), tipc / max(base.rgb, vec3<f32>(1.0e-3)), tone), h);
	// Depth: the deep layers sit in the coat's own shadow (by the tone: the muzzle keeps the skin's shading).
	// Each strand is lit across its round shape: its core bright, its edges darker (the strand's own coverage here).
	let occ = mix(1.0 - fp.d.y * tone, 1.0, pow(h, 0.7)) * mix(1.0 - fp.e.z * tone, 1.0, clamp(max(c1, c2) * 1.4, 0.0, 1.0));
	// Light: the skin's light, wrapped a little towards the unshadowed light so the fur never turns into dark grain.
	let sl = sun_lit(i);
	let lt = mix(sl, max(i.lit, sl), 0.4 * h * tone) * occ;
	var rgb = col * i.c.rgb * lt;
	// Warm sheen at grazing angles (light scattering through the strand tips).
	rgb += rgb * vec3<f32>(1.0, 0.93, 0.78) * (fp.c.y * rim * rim * (0.4 + 0.6 * h) * mix(0.3, 1.0, tone));
	// Strand highlight (Kajiya-Kay): the strand leans out along the normal and down the comb; a white lobe and a coloured,
	// wider one, only where the sun reaches (its share of the skin's light).
	if (fp.d.z > 0.0) {
		let down = vec3<f32>(0.0, 0.0, -1.0);
		let tc = down - nrm * dot(down, nrm);
		let tang = normalize(nrm + tc * (0.9 * h / max(length(tc), 0.3)) + vec3<f32>(0.0, 0.0, 1.0e-4));
		let ld = select(normalize(vec3<f32>(0.3, 0.2, 0.9)), -u.sun.xyz, u.sun.w > 0.0);
		let hv = normalize(ld + view + vec3<f32>(0.0, 0.0, 1.0e-4));
		let th = dot(tang, hv);
		let s1 = pow(max(sqrt(max(1.0 - th * th, 0.0)), 0.0), fp.d.w);
		let th2 = th + 0.12;
		let s2 = pow(max(sqrt(max(1.0 - th2 * th2, 0.0)), 0.0), fp.d.w * 0.35);
		let lum = dot(sl, vec3<f32>(0.3, 0.5, 0.2));
		rgb += (vec3<f32>(s1 * 0.5) + s2 * 0.6 * base.rgb) * fp.d.z * lum * cov * h * tone * max(dot(nrm, ld) + 0.3, 0.0);
	}
	// [PC] Fire / weapon light on the strand (flit.wgsl; post keeps only the far share). Fire light v2 (lane dlit3): the coat's own
	// occlusion `occ` darkens it in the deep layers as it does the sun's light (the fur keeps its depth and stripes); no sheen on
	// strands (Lambert on the skin normal, as the sun's diffuse).
	rgb = flit_add(rgb, col * i.c.rgb * occ, i.w, nrm);
	let dist = distance(i.w, u.eye.xyz);
	let f = clamp(u.fog.z + (dist - u.fog.x) * (u.fog.w - u.fog.z) / max(u.fog.y - u.fog.x, 0.001), min(u.fog.z, u.fog.w), 255.0) / 255.0;
	return vec4<f32>(mix(u.fog_colour.rgb, rgb, f) * a, a);
}

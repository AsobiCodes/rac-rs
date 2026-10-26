// [PC] Starfield (Enhanced only; viewer/starfield.rs): the retail sky star texture drawn as soft glowing points.
// Standalone module (the main level shader already uses @group(2) for other entry points). Group 0 = the level uniforms (only the first
// two matrices are read), group 1 = the star texture with its stars taken out (the dead texture) + the shell sampler, group 2 = frame
// values + the star mask: one texel per star at the star's texel (r, g = centroid offset in the 3x3, b = tint, a = sqrt of its energy).

struct U { view_proj: mat4x4<f32>, sky_view_proj: mat4x4<f32> };
// a = (time in seconds, twinkle amount, pixel scale = target height / 1080, gain), b = (halo gain, cross gain, 0, 0).
struct SU { a: vec4<f32>, b: vec4<f32> };
@group(0) @binding(0) var<uniform> u: U;
@group(1) @binding(0) var tex: texture_2d<f32>;
@group(1) @binding(1) var samp: sampler;
@group(2) @binding(0) var<uniform> su: SU;
@group(2) @binding(1) var starmask: texture_2d<f32>;
@group(2) @binding(2) var starlist: texture_2d<u32>; // per texel: (x + 1, y + 1) of the two nearest stars within 3 texels

struct VOut { @builtin(position) pos: vec4<f32>, @location(0) uv: vec2<f32>, @location(1) c: vec4<f32> };

@vertex fn vs_star(@location(0) p: vec3<f32>, @location(1) uv: vec2<f32>, @location(2) n: vec3<f32>, @location(3) c: vec4<f32>) -> VOut {
	var o: VOut;
	// As vs_sky: pinned to the far plane so the world always draws over it.
	o.pos = (u.sky_view_proj * vec4<f32>(p, 1.0)).xyww;
	o.uv = uv;
	o.c = c;
	return o;
}

fn hash3(p: vec2<i32>) -> vec3<f32> {
	var h = vec3<u32>(bitcast<u32>(p.x) * 0x9E3779B1u, bitcast<u32>(p.y) * 0x85EBCA6Bu, 0xC2B2AE35u);
	h = h ^ (h.yzx >> vec3<u32>(15u));
	h = h * vec3<u32>(0x2C1B3C6Du, 0x297A2D39u, 0x165667B1u);
	h = h ^ (h.zxy >> vec3<u32>(13u));
	h = h * vec3<u32>(0x27D4EB2Fu, 0x9E3779B1u, 0x85EBCA6Bu);
	h = h ^ (h >> vec3<u32>(16u));
	return vec3<f32>(h & vec3<u32>(0xFFFFFFu)) / 16777216.0;
}

fn tint(t: f32) -> vec3<f32> {
	let cool = vec3<f32>(0.62, 0.85, 1.0);
	let warm = vec3<f32>(1.0, 0.84, 0.62);
	return select(mix(vec3<f32>(1.0), warm, -t), mix(vec3<f32>(1.0), cool, t), t > 0.0);
}

// Cumulative normal (tanh form): the pixel-integrated Gaussian is Phi((x + 0.5) / s) - Phi((x - 0.5) / s), so a point that moves
// over the pixel grid keeps its energy (no crawl or shimmer when the camera turns).
fn phi(x: f32) -> f32 {
	return 0.5 + 0.5 * tanh(0.7978846 * (x + 0.044715 * x * x * x));
}

fn box_gauss(x: f32, s: f32) -> f32 {
	return phi((x + 0.5) / s) - phi((x - 0.5) / s);
}

@fragment fn fs_star(i: VOut) -> @location(0) vec4<f32> {
	let dim = vec2<f32>(textureDimensions(starmask));
	let p = i.uv * dim;
	let dx = dpdx(p);
	let dy = dpdy(p);
	let base = textureSample(tex, samp, i.uv) * i.c;
	var star = vec3<f32>(0.0);
	let det = dx.x * dy.y - dx.y * dy.x;
	let sc = su.a.z;
	if (abs(det) > 1e-9) {
		let isz = vec2<i32>(dim);
		let c0 = clamp(vec2<i32>(floor(p)), vec2<i32>(0), isz - vec2<i32>(1));
		let l = textureLoad(starlist, c0, 0);
		for (var k = 0u; k < 2u; k++) {
			if (l[2u * k] != 0u) {
				let n = vec2<i32>(i32(l[2u * k]) - 1, i32(l[2u * k + 1u]) - 1);
				let m = textureLoad(starmask, n, 0);
				if (m.a <= 0.0) {
					continue;
				}
				let f = m.a * m.a;
				let d = p - (vec2<f32>(n) + 0.5 + (m.rg - 0.5) * 2.0);
				let pu = (d.x * dy.y - d.y * dy.x) / det; // d = pu * dx + pv * dy: the offset in screen pixels
				let pv = (dx.x * d.y - dx.y * d.x) / det;
				let h = hash3(n);
				// Brightness: close to linear in the retail texel energy, so every star keeps its retail brightness (a faint one stays faint, no new stars).
				let b = min(su.a.w * 0.7 * pow(f, 0.85), 1.0);
				// Twinkle: one star in five dips now and then (periods 3 to 9 s), never a flicker.
				var tw = 1.0;
				if (h.x < 0.2) {
					let w = 6.2831853 / (3.0 + 6.0 * h.y);
					let ph = 0.5 + 0.5 * sin(su.a.x * w + 6.2831853 * h.z);
					tw = 1.0 - su.a.y * pow(ph, 4.0);
				}
				let s0 = (0.55 + 0.25 * b) * sc;
				let s1 = (1.5 + 1.9 * b) * sc;
				let k0 = box_gauss(0.0, s0);
				let core = box_gauss(pu, s0) * box_gauss(pv, s0) / (k0 * k0);
				let halo = su.b.x * 0.2 * exp(-(pu * pu + pv * pv) / (2.0 * s1 * s1));
				star += tint((m.b - 0.5) * 2.0) * (b * tw * (core + halo));
			}
		}
	}
	star = star * i.c.rgb * i.c.a;
	// Premultiplied: the dead texel as the sky shell blends it, the star light added on top (pipeline: One, OneMinusSrcAlpha).
	return vec4<f32>(base.rgb * base.a + star, base.a);
}

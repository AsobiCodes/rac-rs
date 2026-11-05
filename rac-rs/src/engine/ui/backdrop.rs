//! The blurred game behind the menu: the plan and the numbers of the blur (pure CPU, tested), the easing between the menu's
//! and the peek's blur / scrim, and the knobs. The GPU passes that use them are in `backdrop_gpu.rs`.
//!
//! The mockup blurs the paused game with CSS `blur(16px)`, a Gaussian of standard deviation 16 CSS px. A kernel that wide is
//! not run at full size: the window picture is halved `k` times with exact 2x2 averages (k from the radius, 1..=5), then blurred
//! with a fixed 15-tap Gaussian along x and y at that small size, then drawn back up with a bilinear filter under the scrim. The
//! radius is `blur_px * ui scale * knob` window pixels; the box averaging of the halvings is taken off the kernel's variance so
//! the result matches the CSS radius.
use super::theme;

/// Taps of one side of the blur kernel (offsets 0..=7, so 15 taps in all).
pub const TAPS: usize = 8;
/// The most halvings (1/32 size).
pub const MAX_K: u32 = 5;
/// The widest kernel the 15 taps carry well, in texels of the small level.
const SIGMA_MAX: f32 = 2.4;

/// Normalised one-sided Gaussian weights: `w[0] + 2 * (w[1] + ... + w[7]) == 1`. Sigma in texels; below 0.05 it is a single tap.
pub fn kernel(sigma: f32) -> [f32; TAPS] {
	let mut w = [0.0f64; TAPS];
	if sigma < 0.05 {
		w[0] = 1.0;
	} else {
		let s2 = 2.0 * (sigma as f64) * (sigma as f64);
		for (i, v) in w.iter_mut().enumerate() {
			*v = (-((i * i) as f64) / s2).exp();
		}
	}
	let sum = w[0] + 2.0 * w[1..].iter().sum::<f64>();
	let mut out = [0.0f32; TAPS];
	for (o, v) in out.iter_mut().zip(w) {
		*o = (v / sum) as f32;
	}
	out
}

/// Halvings and the kernel sigma (texels of the small level) for a blur of standard deviation `sigma_win` window pixels.
pub fn plan(sigma_win: f32) -> (u32, f32) {
	let sigma = sigma_win.max(0.0);
	let mut k = 1;
	while k < MAX_K && sigma / (1u32 << k) as f32 > SIGMA_MAX {
		k += 1;
	}
	let d = (1u32 << k) as f32;
	// The 2x2 averages of the halvings already blur like a box of width d (variance d^2 / 12).
	let rest = (sigma * sigma - d * d / 12.0).max(0.01);
	(k, rest.sqrt() / d)
}

/// `[blur in design units, scrim alpha]` eased over time. The target changes when the menu goes to the peek and back; the first
/// target after a reset is taken at once (the menu appears with its backdrop, like its panels).
#[derive(Clone, Copy, Debug, Default)]
pub struct Ease {
	from: [f32; 2],
	to: [f32; 2],
	t0: f64,
	started: bool,
}

impl Ease {
	pub fn target(&mut self, to: [f32; 2], now: f64, dur: f32) {
		if !self.started {
			*self = Ease { from: to, to, t0: now, started: true };
		} else if to != self.to {
			self.from = self.value(now, dur);
			self.to = to;
			self.t0 = now;
		}
	}

	pub fn value(&self, now: f64, dur: f32) -> [f32; 2] {
		let u = if dur <= 0.0 { 1.0 } else { ((now - self.t0) as f32 / dur).clamp(0.0, 1.0) };
		let e = u * u * (3.0 - 2.0 * u);
		[self.from[0] + (self.to[0] - self.from[0]) * e, self.from[1] + (self.to[1] - self.from[1]) * e]
	}

	/// The menu closed: the next one starts at its target.
	pub fn reset(&mut self) {
		self.started = false;
	}
}

/// Seconds on a wall clock (the UI runs at display rate and reads no field data, so frame smoothing cannot make it flicker).
pub fn now_secs() -> f64 {
	static T0: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
	T0.get_or_init(std::time::Instant::now).elapsed().as_secs_f64()
}

/// Look knobs (`RAC_TUNE=name=value,...`, ENV.md): the blur radius and the scrim strength as factors of the mockup's numbers,
/// the ease time, the text coverage exponent (below 1 = bolder), the backdrop dither strength.
pub struct Knobs {
	pub blur: f32,
	pub scrim: f32,
	pub ease_s: f32,
	pub text_gamma: f32,
	pub dither: f32,
}

pub fn knobs() -> Knobs {
	use crate::devtune::f32 as k;
	Knobs { blur: k("ui_blur", 1.0), scrim: k("ui_scrim", 1.0), ease_s: k("ui_ease_s", theme::EASE_S), text_gamma: k("ui_text_gamma", 0.8), dither: k("ui_dither", 1.0) }
}

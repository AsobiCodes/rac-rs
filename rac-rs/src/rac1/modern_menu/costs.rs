//! Speed-impact levels of the rows, in ONE table (rows with `Cost::None` show no meter). MEASURED 2026-11-04 (user: "lets update every
//! setting to match its real impact"; the first guesses had Metal Low and GI High): dev PC (RTX 5070 Ti), the 1280 x 720 test window,
//! build exes/v05_t11a, GPU time per frame (RAC_GPU_PASSES=60, median of ~140 lines) with every row at its top setting, then one row
//! at a time Off / Classic, in three scenes: Kerwan streets (fg1 3_0044, 1.98 ms), the Novalis burning buildings (fg1 1_0018,
//! 2.85 ms), Pokitaru's sea (p0459, 3.70 ms). Tool: scratch_perf_tools/t10/cost_bench.ps1 + cost_gp.py. A row's level is its largest
//! share of the frame in the scenes where it draws: Low under 5 %, Medium 5..15 %, High over 15 %.
use rac_engine::ui::Cost;

pub const RSCALE: Cost = Cost::High;
/// The FSR passes themselves (they save more than they cost; the quality row decides the size).
pub const UPSCALER: Cost = Cost::Low;
pub const UPSCALE_Q: Cost = Cost::High;
pub const SMOOTHING: Cost = Cost::Low;
/// Enhanced against Classic: every Enhanced pass (2.5 / 3.4 ms of 2.85 / 3.70; ~90 %).
pub const LIGHTING: Cost = Cost::High;
/// 0.04 / 0.02 / 0.09 ms (under 3 %).
pub const AO: Cost = Cost::Low;
/// 0.57 / 0.77 / 0.94 ms (25..29 %).
pub const SHADOWS: Cost = Cost::High;
/// 0.09 / 0.10 / 0.10 ms (under 5 %).
pub const GI: Cost = Cost::Low;
/// 0.00 / 0.00 / 0.07 ms.
pub const SUNRAYS: Cost = Cost::Low;
/// High against Low: 0.02 / 0.22 / 0.38 ms (up to 10 %).
pub const DRAW: Cost = Cost::Medium;
/// Lush against Off: 0.12 / 0.57 / 0.30 ms (Novalis 20 %).
pub const VEGETATION: Cost = Cost::High;
/// High against Classic: 0.20 ms on Novalis, 0.92 ms on Pokitaru's sea (25 %).
pub const WATER: Cost = Cost::High;
/// 0.00 / 0.12 ms (3 %).
pub const WAVES: Cost = Cost::Low;
/// Only while the camera is under water (underwater.rs: one full-screen pass and a copy of the picture). Measured (lane nuw, dev PC,
/// fg1 1_0038 under water, Water High, 300 frames): the pass 0.102 ms of a 1.017 ms GPU sum, 0.922 ms with the row Off; under half
/// of the Water passes in the same frames (bed, shade, mirror, positions: 0.235 ms, Medium): Low.
pub const UNDERWATER: Cost = Cost::Low;
/// High against Classic: 0.05 / 0.06 / 0.16 ms (up to 4 %).
pub const FOG: Cost = Cost::Low;
/// High against Classic: 0.20 / 0.26 / 0.10 ms (up to 10 %).
pub const CLOUDS: Cost = Cost::Medium;
/// High against Classic: 0.00 / 0.08 ms (3 %).
pub const PARTICLES: Cost = Cost::Low;
/// 0.68 / 0.77 / 1.64 ms (27..44 %): the costliest effect.
pub const METAL: Cost = Cost::High;
/// The burning buildings of Novalis: 0.07 ms (3 %).
pub const FIRE: Cost = Cost::Low;
/// The burning buildings of Novalis: 0.23 ms (8 %).
pub const SMOKE: Cost = Cost::Medium;
/// 0.00 / 0.06 ms (2 %).
pub const FUR: Cost = Cost::Low;

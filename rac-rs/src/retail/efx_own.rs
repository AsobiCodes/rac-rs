//! Exact ownership of the draw-queue callbacks' packets (EFFECTS LAYER).
//!
//! The draw-queue callbacks (queue A: count 0x15f424, queue B: count 0x15f428; one runner pair per level overlay) put
//! their packets into ONE linear DMA buffer through a bump cursor (a global per level). The cursor value when a
//! callback starts and when the next one starts (or the runner has returned) bounds exactly what that callback wrote.
//!
//! Loop-free, record-only probes (stage 2): nothing here runs retail code itself or holds a host frame across it. Each
//! probe is a hand that only notes (entry, return address, cursor) and then hands over to whatever ran there before
//! (the compiled retail body, or a hand-written function already registered at that entry, called as it would be).
//! Probes sit at: the runner entries (start of a queue run, with the queue's fn list), every callback entry seen in
//! the queues (counted only when the return address is the runner's loop), and the first call after the runner
//! returns (per level: the callee and its return address, read from the world draw function). A queue run is
//! accepted only when the callbacks seen equal the queue's list; otherwise its records carry `ok = false`.
//! Stage 1 replaced the runner loop by a hand that called the callbacks: on Rilgar the laser callback's nested run
//! never came back to that host frame (the oracle's nested run missed the return and went on into the game), so the
//! hand stayed stuck and later woke up in another field; probes cannot get stuck.
//!
//! Callback probes are created when a new callback shows up in a queue (field end), as hands over the first two
//! instruction words (hash taken from RAM then), and the recompiled table is rebuilt so they take effect next field.
//! RAC_EFX_OWN=0 leaves every probe out. RAC_EFX_LOG=1 (retail.rs): one line per field with the records.
use ee_oracle::cpu::Cpu;
use ee_oracle::rec::{hands, hash, register_hand, Hand, Table};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use crate::retail::disc::va;

/// Level number byte (RAM).
const LEVEL: u32 = 0x15ed84;

/// One callback run: queue (0 = A, 1 = B), index in the queue, fn, arg, packet cursor at its start / at its end, and
/// whether the whole queue run it belongs to was seen exactly (start, every callback of the list, end).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Own {
	pub queue: u8,
	pub idx: u16,
	pub fnp: u32,
	pub arg: u32,
	pub c0: u32,
	pub c1: u32,
	pub ok: bool,
	/// The retail view the queue run drew with (`view_words`, read at the runner's entry).
	pub view: [u32; 20],
	/// 1 + index of the fx slot table snapshot taken when the queue run ended (`slot_snap`); 0 = none.
	pub slots: u32,
}

/// One level's runner: entry, the loop's return address (the callback's `ra`), count word, fn and arg arrays.
struct Runner {
	queue: u8,
	entry: u32,
	ret: u32,
	count: u32,
	fns: u32,
	args: u32,
}

/// One level: number, packet cursor global, the two runners, and the end probes (callee entry, return address) = the
/// first call after each runner call in the world draw function (no cursor bump in between).
struct Level {
	level: u8,
	cursor: u32,
	runners: [Runner; 4],
	ends: &'static [(u32, u32)],
}

// Per level [C, scratch_efx/lv/levels.py from each level's overlay.elf]: the world draw is the twin of Rilgar 0x2271a0
// (same layout on all 19 levels): runner A = jal at +0x328, B = jal at +0x3f4; end probes = the next calls at +0x33c,
// +0x40c (only when 0x15f5a4 == 0) and +0x420 (ra = site + 8); runner count / fn / arg words from the runner's own
// lui/lw/addiu; cursor = the global the world draw prologue reads at -0x4de8 (same word at -0x4dc0, -0x4d58, -0x164).
static LEVELS: [Level; 19] = [
	// veldin: world draw 0x1f8538
	Level {
		level: 0,
		cursor: 0x161180,
		runners: [
			Runner { queue: 0, entry: 0x1f93b0, ret: 0x1f93fc, count: 0x15f424, fns: 0x16d880, args: 0x16d980 },
			Runner { queue: 1, entry: 0x1f9568, ret: 0x1f95b4, count: 0x15f428, fns: 0x16da80, args: 0x16db80 },
			Runner { queue: 2, entry: 0x1f9428, ret: 0x1f9474, count: 0x15f42c, fns: 0x16dc80, args: 0x16dd80 },
			Runner { queue: 3, entry: 0x1f94a0, ret: 0x1f94ec, count: 0x15f430, fns: 0x16de80, args: 0x16df80 },
		],
		ends: &[(0x2a0550, 0x1f887c), (0x1fe688, 0x1f894c), (0x2a0550, 0x1f8960), (0x1f90f8, 0x1f86c0), (0x1f90f8, 0x1f8794)],
	},
	// novalis: world draw 0x21a0b8
	Level {
		level: 1,
		cursor: 0x161140,
		runners: [
			Runner { queue: 0, entry: 0x21af30, ret: 0x21af7c, count: 0x15f424, fns: 0x16dc80, args: 0x16dd80 },
			Runner { queue: 1, entry: 0x21b0e8, ret: 0x21b134, count: 0x15f428, fns: 0x16de80, args: 0x16df80 },
			Runner { queue: 2, entry: 0x21afa8, ret: 0x21aff4, count: 0x15f42c, fns: 0x16e080, args: 0x16e180 },
			Runner { queue: 3, entry: 0x21b020, ret: 0x21b06c, count: 0x15f430, fns: 0x16e280, args: 0x16e380 },
		],
		ends: &[(0x2b4248, 0x21a3fc), (0x220518, 0x21a4cc), (0x2b4248, 0x21a4e0), (0x21ac78, 0x21a240), (0x21ac78, 0x21a314)],
	},
	// aridia: world draw 0x2093a0
	Level {
		level: 2,
		cursor: 0x161140,
		runners: [
			Runner { queue: 0, entry: 0x20a218, ret: 0x20a264, count: 0x15f424, fns: 0x16de00, args: 0x16df00 },
			Runner { queue: 1, entry: 0x20a420, ret: 0x20a46c, count: 0x15f428, fns: 0x16e000, args: 0x16e100 },
			Runner { queue: 2, entry: 0x20a2e0, ret: 0x20a32c, count: 0x15f42c, fns: 0x16e200, args: 0x16e300 },
			Runner { queue: 3, entry: 0x20a358, ret: 0x20a3a4, count: 0x15f430, fns: 0x16e400, args: 0x16e500 },
		],
		ends: &[(0x29fce0, 0x2096e4), (0x20f640, 0x2097b4), (0x29fce0, 0x2097c8), (0x209f60, 0x209528), (0x209f60, 0x2095fc)],
	},
	// kerwan: world draw 0x1f1e20
	Level {
		level: 3,
		cursor: 0x161140,
		runners: [
			Runner { queue: 0, entry: 0x1f2c98, ret: 0x1f2ce4, count: 0x15f424, fns: 0x16d900, args: 0x16da00 },
			Runner { queue: 1, entry: 0x1f2e50, ret: 0x1f2e9c, count: 0x15f428, fns: 0x16db00, args: 0x16dc00 },
			Runner { queue: 2, entry: 0x1f2d10, ret: 0x1f2d5c, count: 0x15f42c, fns: 0x16dd00, args: 0x16de00 },
			Runner { queue: 3, entry: 0x1f2d88, ret: 0x1f2dd4, count: 0x15f430, fns: 0x16df00, args: 0x16e000 },
		],
		ends: &[(0x28ce20, 0x1f2164), (0x1f7f70, 0x1f2234), (0x28ce20, 0x1f2248), (0x1f29e0, 0x1f1fa8), (0x1f29e0, 0x1f207c)],
	},
	// eudora: world draw 0x1eae20
	Level {
		level: 4,
		cursor: 0x161140,
		runners: [
			Runner { queue: 0, entry: 0x1ebc98, ret: 0x1ebce4, count: 0x15f424, fns: 0x16d980, args: 0x16da80 },
			Runner { queue: 1, entry: 0x1ebe50, ret: 0x1ebe9c, count: 0x15f428, fns: 0x16db80, args: 0x16dc80 },
			Runner { queue: 2, entry: 0x1ebd10, ret: 0x1ebd5c, count: 0x15f42c, fns: 0x16dd80, args: 0x16de80 },
			Runner { queue: 3, entry: 0x1ebd88, ret: 0x1ebdd4, count: 0x15f430, fns: 0x16df80, args: 0x16e080 },
		],
		ends: &[(0x2913c8, 0x1eb164), (0x1f1070, 0x1eb234), (0x2913c8, 0x1eb248), (0x1eb9e0, 0x1eafa8), (0x1eb9e0, 0x1eb07c)],
	},
	// rilgar: world draw 0x2271a0
	Level {
		level: 5,
		cursor: 0x161180,
		runners: [
			Runner { queue: 0, entry: 0x228018, ret: 0x228064, count: 0x15f424, fns: 0x16dc80, args: 0x16dd80 },
			Runner { queue: 1, entry: 0x228220, ret: 0x22826c, count: 0x15f428, fns: 0x16de80, args: 0x16df80 },
			Runner { queue: 2, entry: 0x2280e0, ret: 0x22812c, count: 0x15f42c, fns: 0x16e080, args: 0x16e180 },
			Runner { queue: 3, entry: 0x228158, ret: 0x2281a4, count: 0x15f430, fns: 0x16e280, args: 0x16e380 },
		],
		ends: &[(0x2c8c88, 0x2274e4), (0x22d340, 0x2275b4), (0x2c8c88, 0x2275c8), (0x227d60, 0x227328), (0x227d60, 0x2273fc)],
	},
	// nebula: world draw 0x2100a0
	Level {
		level: 6,
		cursor: 0x161140,
		runners: [
			Runner { queue: 0, entry: 0x210f18, ret: 0x210f64, count: 0x15f424, fns: 0x16df80, args: 0x16e080 },
			Runner { queue: 1, entry: 0x2110d0, ret: 0x21111c, count: 0x15f428, fns: 0x16e180, args: 0x16e280 },
			Runner { queue: 2, entry: 0x210f90, ret: 0x210fdc, count: 0x15f42c, fns: 0x16e380, args: 0x16e480 },
			Runner { queue: 3, entry: 0x211008, ret: 0x211054, count: 0x15f430, fns: 0x16e580, args: 0x16e680 },
		],
		ends: &[(0x2ad5c8, 0x2103e4), (0x2162f0, 0x2104b4), (0x2ad5c8, 0x2104c8), (0x210c60, 0x210228), (0x210c60, 0x2102fc)],
	},
	// umbris: world draw 0x2202a0
	Level {
		level: 7,
		cursor: 0x161140,
		runners: [
			Runner { queue: 0, entry: 0x221118, ret: 0x221164, count: 0x15f424, fns: 0x16d880, args: 0x16d980 },
			Runner { queue: 1, entry: 0x221320, ret: 0x22136c, count: 0x15f428, fns: 0x16da80, args: 0x16db80 },
			Runner { queue: 2, entry: 0x2211e0, ret: 0x22122c, count: 0x15f42c, fns: 0x16dc80, args: 0x16dd80 },
			Runner { queue: 3, entry: 0x221258, ret: 0x2212a4, count: 0x15f430, fns: 0x16de80, args: 0x16df80 },
		],
		ends: &[(0x2c7178, 0x2205e4), (0x226440, 0x2206b4), (0x2c7178, 0x2206c8), (0x220e60, 0x220428), (0x220e60, 0x2204fc)],
	},
	// batalia: world draw 0x20aec8
	Level {
		level: 8,
		cursor: 0x161140,
		runners: [
			Runner { queue: 0, entry: 0x20bd40, ret: 0x20bd8c, count: 0x15f424, fns: 0x16e000, args: 0x16e100 },
			Runner { queue: 1, entry: 0x20bef8, ret: 0x20bf44, count: 0x15f428, fns: 0x16e200, args: 0x16e300 },
			Runner { queue: 2, entry: 0x20bdb8, ret: 0x20be04, count: 0x15f42c, fns: 0x16e400, args: 0x16e500 },
			Runner { queue: 3, entry: 0x20be30, ret: 0x20be7c, count: 0x15f430, fns: 0x16e600, args: 0x16e700 },
		],
		ends: &[(0x2a88f8, 0x20b20c), (0x211118, 0x20b2dc), (0x2a88f8, 0x20b2f0), (0x20ba88, 0x20b050), (0x20ba88, 0x20b124)],
	},
	// gaspar: world draw 0x217820
	Level {
		level: 9,
		cursor: 0x161140,
		runners: [
			Runner { queue: 0, entry: 0x218698, ret: 0x2186e4, count: 0x15f424, fns: 0x16d980, args: 0x16da80 },
			Runner { queue: 1, entry: 0x2188a0, ret: 0x2188ec, count: 0x15f428, fns: 0x16db80, args: 0x16dc80 },
			Runner { queue: 2, entry: 0x218760, ret: 0x2187ac, count: 0x15f42c, fns: 0x16dd80, args: 0x16de80 },
			Runner { queue: 3, entry: 0x2187d8, ret: 0x218824, count: 0x15f430, fns: 0x16df80, args: 0x16e080 },
		],
		ends: &[(0x2bce20, 0x217b64), (0x21dcd0, 0x217c34), (0x2bce20, 0x217c48), (0x2183e0, 0x2179a8), (0x2183e0, 0x217a7c)],
	},
	// orxon: world draw 0x1ebb20
	Level {
		level: 10,
		cursor: 0x161140,
		runners: [
			Runner { queue: 0, entry: 0x1ec998, ret: 0x1ec9e4, count: 0x15f424, fns: 0x16dc80, args: 0x16dd80 },
			Runner { queue: 1, entry: 0x1ecb50, ret: 0x1ecb9c, count: 0x15f428, fns: 0x16de80, args: 0x16df80 },
			Runner { queue: 2, entry: 0x1eca10, ret: 0x1eca5c, count: 0x15f42c, fns: 0x16e080, args: 0x16e180 },
			Runner { queue: 3, entry: 0x1eca88, ret: 0x1ecad4, count: 0x15f430, fns: 0x16e280, args: 0x16e380 },
		],
		ends: &[(0x290fd0, 0x1ebe64), (0x1f1c70, 0x1ebf34), (0x290fd0, 0x1ebf48), (0x1ec6e0, 0x1ebca8), (0x1ec6e0, 0x1ebd7c)],
	},
	// pokitaru: world draw 0x229d20
	Level {
		level: 11,
		cursor: 0x161140,
		runners: [
			Runner { queue: 0, entry: 0x22ab98, ret: 0x22abe4, count: 0x15f424, fns: 0x16e200, args: 0x16e300 },
			Runner { queue: 1, entry: 0x22ad50, ret: 0x22ad9c, count: 0x15f428, fns: 0x16e400, args: 0x16e500 },
			Runner { queue: 2, entry: 0x22ac10, ret: 0x22ac5c, count: 0x15f42c, fns: 0x16e600, args: 0x16e700 },
			Runner { queue: 3, entry: 0x22ac88, ret: 0x22acd4, count: 0x15f430, fns: 0x16e800, args: 0x16e900 },
		],
		ends: &[(0x2c33d0, 0x22a064), (0x22ff70, 0x22a134), (0x2c33d0, 0x22a148), (0x22a8e0, 0x229ea8), (0x22a8e0, 0x229f7c)],
	},
	// hoven: world draw 0x21af28
	Level {
		level: 12,
		cursor: 0x161140,
		runners: [
			Runner { queue: 0, entry: 0x21bda0, ret: 0x21bdec, count: 0x15f424, fns: 0x16dc80, args: 0x16dd80 },
			Runner { queue: 1, entry: 0x21bfa8, ret: 0x21bff4, count: 0x15f428, fns: 0x16de80, args: 0x16df80 },
			Runner { queue: 2, entry: 0x21be68, ret: 0x21beb4, count: 0x15f42c, fns: 0x16e080, args: 0x16e180 },
			Runner { queue: 3, entry: 0x21bee0, ret: 0x21bf2c, count: 0x15f430, fns: 0x16e280, args: 0x16e380 },
		],
		ends: &[(0x2b6d60, 0x21b26c), (0x2214d8, 0x21b33c), (0x2b6d60, 0x21b350), (0x21bae8, 0x21b0b0), (0x21bae8, 0x21b184)],
	},
	// oltanis_orbit: world draw 0x203ea0
	Level {
		level: 13,
		cursor: 0x161140,
		runners: [
			Runner { queue: 0, entry: 0x204d18, ret: 0x204d64, count: 0x15f424, fns: 0x16db00, args: 0x16dc00 },
			Runner { queue: 1, entry: 0x204ed0, ret: 0x204f1c, count: 0x15f428, fns: 0x16dd00, args: 0x16de00 },
			Runner { queue: 2, entry: 0x204d90, ret: 0x204ddc, count: 0x15f42c, fns: 0x16df00, args: 0x16e000 },
			Runner { queue: 3, entry: 0x204e08, ret: 0x204e54, count: 0x15f430, fns: 0x16e100, args: 0x16e200 },
		],
		ends: &[(0x2aab60, 0x2041e4), (0x20a0f0, 0x2042b4), (0x2aab60, 0x2042c8), (0x204a60, 0x204028), (0x204a60, 0x2040fc)],
	},
	// oltanis: world draw 0x20b728
	Level {
		level: 14,
		cursor: 0x161180,
		runners: [
			Runner { queue: 0, entry: 0x20c5a0, ret: 0x20c5ec, count: 0x15f424, fns: 0x16df80, args: 0x16e080 },
			Runner { queue: 1, entry: 0x20c7f8, ret: 0x20c844, count: 0x15f428, fns: 0x16e180, args: 0x16e280 },
			Runner { queue: 2, entry: 0x20c668, ret: 0x20c6b4, count: 0x15f42c, fns: 0x16e380, args: 0x16e480 },
			Runner { queue: 3, entry: 0x20c730, ret: 0x20c77c, count: 0x15f430, fns: 0x16e580, args: 0x16e680 },
		],
		ends: &[(0x2a6900, 0x20ba6c), (0x211c28, 0x20bb3c), (0x2a6900, 0x20bb50), (0x20c2e8, 0x20b8b0), (0x20c2e8, 0x20b984)],
	},
	// quartu: world draw 0x1f1ba0
	Level {
		level: 15,
		cursor: 0x161140,
		runners: [
			Runner { queue: 0, entry: 0x1f2a18, ret: 0x1f2a64, count: 0x15f424, fns: 0x16de00, args: 0x16df00 },
			Runner { queue: 1, entry: 0x1f2c20, ret: 0x1f2c6c, count: 0x15f428, fns: 0x16e000, args: 0x16e100 },
			Runner { queue: 2, entry: 0x1f2a90, ret: 0x1f2adc, count: 0x15f42c, fns: 0x16e200, args: 0x16e300 },
			Runner { queue: 3, entry: 0x1f2b58, ret: 0x1f2ba4, count: 0x15f430, fns: 0x16e400, args: 0x16e500 },
		],
		ends: &[(0x28e420, 0x1f1ee4), (0x1f7d40, 0x1f1fb4), (0x28e420, 0x1f1fc8), (0x1f2760, 0x1f1d28), (0x1f2760, 0x1f1dfc)],
	},
	// kalebo: world draw 0x1f6d20
	Level {
		level: 16,
		cursor: 0x161180,
		runners: [
			Runner { queue: 0, entry: 0x1f7b98, ret: 0x1f7be4, count: 0x15f424, fns: 0x16dc00, args: 0x16dd00 },
			Runner { queue: 1, entry: 0x1f7d50, ret: 0x1f7d9c, count: 0x15f428, fns: 0x16de00, args: 0x16df00 },
			Runner { queue: 2, entry: 0x1f7c10, ret: 0x1f7c5c, count: 0x15f42c, fns: 0x16e000, args: 0x16e100 },
			Runner { queue: 3, entry: 0x1f7c88, ret: 0x1f7cd4, count: 0x15f430, fns: 0x16e200, args: 0x16e300 },
		],
		ends: &[(0x299428, 0x1f7064), (0x1fce70, 0x1f7134), (0x299428, 0x1f7148), (0x1f78e0, 0x1f6ea8), (0x1f78e0, 0x1f6f7c)],
	},
	// veldin_orbit: world draw 0x1f5520
	Level {
		level: 17,
		cursor: 0x161140,
		runners: [
			Runner { queue: 0, entry: 0x1f6398, ret: 0x1f63e4, count: 0x15f424, fns: 0x16e100, args: 0x16e200 },
			Runner { queue: 1, entry: 0x1f6550, ret: 0x1f659c, count: 0x15f428, fns: 0x16e300, args: 0x16e400 },
			Runner { queue: 2, entry: 0x1f6410, ret: 0x1f645c, count: 0x15f42c, fns: 0x16e500, args: 0x16e600 },
			Runner { queue: 3, entry: 0x1f6488, ret: 0x1f64d4, count: 0x15f430, fns: 0x16e700, args: 0x16e800 },
		],
		ends: &[(0x298730, 0x1f5864), (0x1fb670, 0x1f5934), (0x298730, 0x1f5948), (0x1f60e0, 0x1f56a8), (0x1f60e0, 0x1f577c)],
	},
	// veldin_finale: world draw 0x2011a0
	Level {
		level: 18,
		cursor: 0x161140,
		runners: [
			Runner { queue: 0, entry: 0x202018, ret: 0x202064, count: 0x15f424, fns: 0x16e200, args: 0x16e300 },
			Runner { queue: 1, entry: 0x2021d0, ret: 0x20221c, count: 0x15f428, fns: 0x16e400, args: 0x16e500 },
			Runner { queue: 2, entry: 0x202090, ret: 0x2020dc, count: 0x15f42c, fns: 0x16e600, args: 0x16e700 },
			Runner { queue: 3, entry: 0x202108, ret: 0x202154, count: 0x15f430, fns: 0x16e800, args: 0x16e900 },
		],
		ends: &[(0x2a0080, 0x2014e4), (0x2073f0, 0x2015b4), (0x2a0080, 0x2015c8), (0x201d60, 0x201328), (0x201d60, 0x2013fc)],
	},
];
/// `LEVELS` on the USA disc (tools/racrs_va.py: every address mapped in its level; generated, do not edit).
static LEVELS_USA: [Level; 19] = [
	// veldin: world draw 0x1f8538
	Level {
		level: 0,
		cursor: 0x1611c0,
		runners: [
			Runner { queue: 0, entry: 0x1f9430, ret: 0x1f947c, count: 0x15f424, fns: 0x16d880, args: 0x16d980 },
			Runner { queue: 1, entry: 0x1f95e8, ret: 0x1f9634, count: 0x15f428, fns: 0x16da80, args: 0x16db80 },
			Runner { queue: 2, entry: 0x1f94a8, ret: 0x1f94f4, count: 0x15f42c, fns: 0x16dc80, args: 0x16dd80 },
			Runner { queue: 3, entry: 0x1f9520, ret: 0x1f956c, count: 0x15f430, fns: 0x16de80, args: 0x16df80 },
		],
		ends: &[(0x2a0ba8, 0x1f88fc), (0x1fead8, 0x1f89cc), (0x2a0ba8, 0x1f89e0), (0x1f9178, 0x1f8740), (0x1f9178, 0x1f8814)],
	},
	// novalis: world draw 0x21a0b8
	Level {
		level: 1,
		cursor: 0x1611c0,
		runners: [
			Runner { queue: 0, entry: 0x21b030, ret: 0x21b07c, count: 0x15f424, fns: 0x16dd00, args: 0x16de00 },
			Runner { queue: 1, entry: 0x21b1e8, ret: 0x21b234, count: 0x15f428, fns: 0x16df00, args: 0x16e000 },
			Runner { queue: 2, entry: 0x21b0a8, ret: 0x21b0f4, count: 0x15f42c, fns: 0x16e100, args: 0x16e200 },
			Runner { queue: 3, entry: 0x21b120, ret: 0x21b16c, count: 0x15f430, fns: 0x16e300, args: 0x16e400 },
		],
		ends: &[(0x2b4918, 0x21a4fc), (0x2209e8, 0x21a5cc), (0x2b4918, 0x21a5e0), (0x21ad78, 0x21a340), (0x21ad78, 0x21a414)],
	},
	// aridia: world draw 0x2093a0
	Level {
		level: 2,
		cursor: 0x1611c0,
		runners: [
			Runner { queue: 0, entry: 0x20a318, ret: 0x20a364, count: 0x15f424, fns: 0x16de80, args: 0x16df80 },
			Runner { queue: 1, entry: 0x20a520, ret: 0x20a56c, count: 0x15f428, fns: 0x16e080, args: 0x16e180 },
			Runner { queue: 2, entry: 0x20a3e0, ret: 0x20a42c, count: 0x15f42c, fns: 0x16e280, args: 0x16e380 },
			Runner { queue: 3, entry: 0x20a458, ret: 0x20a4a4, count: 0x15f430, fns: 0x16e480, args: 0x16e580 },
		],
		ends: &[(0x2a03b0, 0x2097e4), (0x20fb10, 0x2098b4), (0x2a03b0, 0x2098c8), (0x20a060, 0x209628), (0x20a060, 0x2096fc)],
	},
	// kerwan: world draw 0x1f1e20
	Level {
		level: 3,
		cursor: 0x1611c0,
		runners: [
			Runner { queue: 0, entry: 0x1f2d98, ret: 0x1f2de4, count: 0x15f424, fns: 0x16d980, args: 0x16da80 },
			Runner { queue: 1, entry: 0x1f2f50, ret: 0x1f2f9c, count: 0x15f428, fns: 0x16db80, args: 0x16dc80 },
			Runner { queue: 2, entry: 0x1f2e10, ret: 0x1f2e5c, count: 0x15f42c, fns: 0x16dd80, args: 0x16de80 },
			Runner { queue: 3, entry: 0x1f2e88, ret: 0x1f2ed4, count: 0x15f430, fns: 0x16df80, args: 0x16e080 },
		],
		ends: &[(0x28d4f0, 0x1f2264), (0x1f8440, 0x1f2334), (0x28d4f0, 0x1f2348), (0x1f2ae0, 0x1f20a8), (0x1f2ae0, 0x1f217c)],
	},
	// eudora: world draw 0x1eae20
	Level {
		level: 4,
		cursor: 0x1611c0,
		runners: [
			Runner { queue: 0, entry: 0x1ebd98, ret: 0x1ebde4, count: 0x15f424, fns: 0x16da00, args: 0x16db00 },
			Runner { queue: 1, entry: 0x1ebf50, ret: 0x1ebf9c, count: 0x15f428, fns: 0x16dc00, args: 0x16dd00 },
			Runner { queue: 2, entry: 0x1ebe10, ret: 0x1ebe5c, count: 0x15f42c, fns: 0x16de00, args: 0x16df00 },
			Runner { queue: 3, entry: 0x1ebe88, ret: 0x1ebed4, count: 0x15f430, fns: 0x16e000, args: 0x16e100 },
		],
		ends: &[(0x291aa0, 0x1eb264), (0x1f1540, 0x1eb334), (0x291aa0, 0x1eb348), (0x1ebae0, 0x1eb0a8), (0x1ebae0, 0x1eb17c)],
	},
	// rilgar: world draw 0x2271a0
	Level {
		level: 5,
		cursor: 0x1611c0,
		runners: [
			Runner { queue: 0, entry: 0x228098, ret: 0x2280e4, count: 0x15f424, fns: 0x16dc80, args: 0x16dd80 },
			Runner { queue: 1, entry: 0x2282a0, ret: 0x2282ec, count: 0x15f428, fns: 0x16de80, args: 0x16df80 },
			Runner { queue: 2, entry: 0x228160, ret: 0x2281ac, count: 0x15f42c, fns: 0x16e080, args: 0x16e180 },
			Runner { queue: 3, entry: 0x2281d8, ret: 0x228224, count: 0x15f430, fns: 0x16e280, args: 0x16e380 },
		],
		ends: &[(0x2c92f8, 0x227564), (0x22d790, 0x227634), (0x2c92f8, 0x227648), (0x227de0, 0x2273a8), (0x227de0, 0x22747c)],
	},
	// nebula: world draw 0x2100a0
	Level {
		level: 6,
		cursor: 0x1611c0,
		runners: [
			Runner { queue: 0, entry: 0x211018, ret: 0x211064, count: 0x15f424, fns: 0x16e000, args: 0x16e100 },
			Runner { queue: 1, entry: 0x2111d0, ret: 0x21121c, count: 0x15f428, fns: 0x16e200, args: 0x16e300 },
			Runner { queue: 2, entry: 0x211090, ret: 0x2110dc, count: 0x15f42c, fns: 0x16e400, args: 0x16e500 },
			Runner { queue: 3, entry: 0x211108, ret: 0x211154, count: 0x15f430, fns: 0x16e600, args: 0x16e700 },
		],
		ends: &[(0x2adca0, 0x2104e4), (0x2167c0, 0x2105b4), (0x2adca0, 0x2105c8), (0x210d60, 0x210328), (0x210d60, 0x2103fc)],
	},
	// umbris: world draw 0x2202a0
	Level {
		level: 7,
		cursor: 0x1611c0,
		runners: [
			Runner { queue: 0, entry: 0x221218, ret: 0x221264, count: 0x15f424, fns: 0x16d900, args: 0x16da00 },
			Runner { queue: 1, entry: 0x221420, ret: 0x22146c, count: 0x15f428, fns: 0x16db00, args: 0x16dc00 },
			Runner { queue: 2, entry: 0x2212e0, ret: 0x22132c, count: 0x15f42c, fns: 0x16dd00, args: 0x16de00 },
			Runner { queue: 3, entry: 0x221358, ret: 0x2213a4, count: 0x15f430, fns: 0x16df00, args: 0x16e000 },
		],
		ends: &[(0x2c7850, 0x2206e4), (0x226910, 0x2207b4), (0x2c7850, 0x2207c8), (0x220f60, 0x220528), (0x220f60, 0x2205fc)],
	},
	// batalia: world draw 0x20aec8
	Level {
		level: 8,
		cursor: 0x1611c0,
		runners: [
			Runner { queue: 0, entry: 0x20be40, ret: 0x20be8c, count: 0x15f424, fns: 0x16e080, args: 0x16e180 },
			Runner { queue: 1, entry: 0x20bff8, ret: 0x20c044, count: 0x15f428, fns: 0x16e280, args: 0x16e380 },
			Runner { queue: 2, entry: 0x20beb8, ret: 0x20bf04, count: 0x15f42c, fns: 0x16e480, args: 0x16e580 },
			Runner { queue: 3, entry: 0x20bf30, ret: 0x20bf7c, count: 0x15f430, fns: 0x16e680, args: 0x16e780 },
		],
		ends: &[(0x2a8fc8, 0x20b30c), (0x2115e8, 0x20b3dc), (0x2a8fc8, 0x20b3f0), (0x20bb88, 0x20b150), (0x20bb88, 0x20b224)],
	},
	// gaspar: world draw 0x217820
	Level {
		level: 9,
		cursor: 0x1611c0,
		runners: [
			Runner { queue: 0, entry: 0x218798, ret: 0x2187e4, count: 0x15f424, fns: 0x16da00, args: 0x16db00 },
			Runner { queue: 1, entry: 0x2189a0, ret: 0x2189ec, count: 0x15f428, fns: 0x16dc00, args: 0x16dd00 },
			Runner { queue: 2, entry: 0x218860, ret: 0x2188ac, count: 0x15f42c, fns: 0x16de00, args: 0x16df00 },
			Runner { queue: 3, entry: 0x2188d8, ret: 0x218924, count: 0x15f430, fns: 0x16e000, args: 0x16e100 },
		],
		ends: &[(0x2bd4f8, 0x217c64), (0x21e1a0, 0x217d34), (0x2bd4f8, 0x217d48), (0x2184e0, 0x217aa8), (0x2184e0, 0x217b7c)],
	},
	// orxon: world draw 0x1ebb20
	Level {
		level: 10,
		cursor: 0x1611c0,
		runners: [
			Runner { queue: 0, entry: 0x1eca98, ret: 0x1ecae4, count: 0x15f424, fns: 0x16dd00, args: 0x16de00 },
			Runner { queue: 1, entry: 0x1ecc50, ret: 0x1ecc9c, count: 0x15f428, fns: 0x16df00, args: 0x16e000 },
			Runner { queue: 2, entry: 0x1ecb10, ret: 0x1ecb5c, count: 0x15f42c, fns: 0x16e100, args: 0x16e200 },
			Runner { queue: 3, entry: 0x1ecb88, ret: 0x1ecbd4, count: 0x15f430, fns: 0x16e300, args: 0x16e400 },
		],
		ends: &[(0x2916a8, 0x1ebf64), (0x1f2140, 0x1ec034), (0x2916a8, 0x1ec048), (0x1ec7e0, 0x1ebda8), (0x1ec7e0, 0x1ebe7c)],
	},
	// pokitaru: world draw 0x229d20
	Level {
		level: 11,
		cursor: 0x1611c0,
		runners: [
			Runner { queue: 0, entry: 0x22ac98, ret: 0x22ace4, count: 0x15f424, fns: 0x16e280, args: 0x16e380 },
			Runner { queue: 1, entry: 0x22ae50, ret: 0x22ae9c, count: 0x15f428, fns: 0x16e480, args: 0x16e580 },
			Runner { queue: 2, entry: 0x22ad10, ret: 0x22ad5c, count: 0x15f42c, fns: 0x16e680, args: 0x16e780 },
			Runner { queue: 3, entry: 0x22ad88, ret: 0x22add4, count: 0x15f430, fns: 0x16e880, args: 0x16e980 },
		],
		ends: &[(0x2c3aa0, 0x22a164), (0x230440, 0x22a234), (0x2c3aa0, 0x22a248), (0x22a9e0, 0x229fa8), (0x22a9e0, 0x22a07c)],
	},
	// hoven: world draw 0x21af28
	Level {
		level: 12,
		cursor: 0x1611c0,
		runners: [
			Runner { queue: 0, entry: 0x21bea0, ret: 0x21beec, count: 0x15f424, fns: 0x16dd00, args: 0x16de00 },
			Runner { queue: 1, entry: 0x21c0a8, ret: 0x21c0f4, count: 0x15f428, fns: 0x16df00, args: 0x16e000 },
			Runner { queue: 2, entry: 0x21bf68, ret: 0x21bfb4, count: 0x15f42c, fns: 0x16e100, args: 0x16e200 },
			Runner { queue: 3, entry: 0x21bfe0, ret: 0x21c02c, count: 0x15f430, fns: 0x16e300, args: 0x16e400 },
		],
		ends: &[(0x2b7430, 0x21b36c), (0x2219a8, 0x21b43c), (0x2b7430, 0x21b450), (0x21bbe8, 0x21b1b0), (0x21bbe8, 0x21b284)],
	},
	// oltanis_orbit: world draw 0x203ea0
	Level {
		level: 13,
		cursor: 0x1611c0,
		runners: [
			Runner { queue: 0, entry: 0x204e18, ret: 0x204e64, count: 0x15f424, fns: 0x16db80, args: 0x16dc80 },
			Runner { queue: 1, entry: 0x204fd0, ret: 0x20501c, count: 0x15f428, fns: 0x16dd80, args: 0x16de80 },
			Runner { queue: 2, entry: 0x204e90, ret: 0x204edc, count: 0x15f42c, fns: 0x16df80, args: 0x16e080 },
			Runner { queue: 3, entry: 0x204f08, ret: 0x204f54, count: 0x15f430, fns: 0x16e180, args: 0x16e280 },
		],
		ends: &[(0x2ab238, 0x2042e4), (0x20a5c0, 0x2043b4), (0x2ab238, 0x2043c8), (0x204b60, 0x204128), (0x204b60, 0x2041fc)],
	},
	// oltanis: world draw 0x20b728
	Level {
		level: 14,
		cursor: 0x1611c0,
		runners: [
			Runner { queue: 0, entry: 0x20c620, ret: 0x20c66c, count: 0x15f424, fns: 0x16df80, args: 0x16e080 },
			Runner { queue: 1, entry: 0x20c878, ret: 0x20c8c4, count: 0x15f428, fns: 0x16e180, args: 0x16e280 },
			Runner { queue: 2, entry: 0x20c6e8, ret: 0x20c734, count: 0x15f42c, fns: 0x16e380, args: 0x16e480 },
			Runner { queue: 3, entry: 0x20c7b0, ret: 0x20c7fc, count: 0x15f430, fns: 0x16e580, args: 0x16e680 },
		],
		ends: &[(0x2a6f50, 0x20baec), (0x212078, 0x20bbbc), (0x2a6f50, 0x20bbd0), (0x20c368, 0x20b930), (0x20c368, 0x20ba04)],
	},
	// quartu: world draw 0x1f1ba0
	Level {
		level: 15,
		cursor: 0x1611c0,
		runners: [
			Runner { queue: 0, entry: 0x1f2b18, ret: 0x1f2b64, count: 0x15f424, fns: 0x16de80, args: 0x16df80 },
			Runner { queue: 1, entry: 0x1f2d20, ret: 0x1f2d6c, count: 0x15f428, fns: 0x16e080, args: 0x16e180 },
			Runner { queue: 2, entry: 0x1f2b90, ret: 0x1f2bdc, count: 0x15f42c, fns: 0x16e280, args: 0x16e380 },
			Runner { queue: 3, entry: 0x1f2c58, ret: 0x1f2ca4, count: 0x15f430, fns: 0x16e480, args: 0x16e580 },
		],
		ends: &[(0x28eaf0, 0x1f1fe4), (0x1f8210, 0x1f20b4), (0x28eaf0, 0x1f20c8), (0x1f2860, 0x1f1e28), (0x1f2860, 0x1f1efc)],
	},
	// kalebo: world draw 0x1f6d20
	Level {
		level: 16,
		cursor: 0x1611c0,
		runners: [
			Runner { queue: 0, entry: 0x1f7c98, ret: 0x1f7ce4, count: 0x15f424, fns: 0x16dc80, args: 0x16dd80 },
			Runner { queue: 1, entry: 0x1f7e50, ret: 0x1f7e9c, count: 0x15f428, fns: 0x16de80, args: 0x16df80 },
			Runner { queue: 2, entry: 0x1f7d10, ret: 0x1f7d5c, count: 0x15f42c, fns: 0x16e080, args: 0x16e180 },
			Runner { queue: 3, entry: 0x1f7d88, ret: 0x1f7dd4, count: 0x15f430, fns: 0x16e280, args: 0x16e380 },
		],
		ends: &[(0x299b18, 0x1f7164), (0x1fd340, 0x1f7234), (0x299b18, 0x1f7248), (0x1f79e0, 0x1f6fa8), (0x1f79e0, 0x1f707c)],
	},
	// veldin_orbit: world draw 0x1f5520
	Level {
		level: 17,
		cursor: 0x1611c0,
		runners: [
			Runner { queue: 0, entry: 0x1f6498, ret: 0x1f64e4, count: 0x15f424, fns: 0x16e180, args: 0x16e280 },
			Runner { queue: 1, entry: 0x1f6650, ret: 0x1f669c, count: 0x15f428, fns: 0x16e380, args: 0x16e480 },
			Runner { queue: 2, entry: 0x1f6510, ret: 0x1f655c, count: 0x15f42c, fns: 0x16e580, args: 0x16e680 },
			Runner { queue: 3, entry: 0x1f6588, ret: 0x1f65d4, count: 0x15f430, fns: 0x16e780, args: 0x16e880 },
		],
		ends: &[(0x298e08, 0x1f5964), (0x1fbb40, 0x1f5a34), (0x298e08, 0x1f5a48), (0x1f61e0, 0x1f57a8), (0x1f61e0, 0x1f587c)],
	},
	// veldin_finale: world draw 0x2011a0
	Level {
		level: 18,
		cursor: 0x1611c0,
		runners: [
			Runner { queue: 0, entry: 0x202118, ret: 0x202164, count: 0x15f424, fns: 0x16e280, args: 0x16e380 },
			Runner { queue: 1, entry: 0x2022d0, ret: 0x20231c, count: 0x15f428, fns: 0x16e480, args: 0x16e580 },
			Runner { queue: 2, entry: 0x202190, ret: 0x2021dc, count: 0x15f42c, fns: 0x16e680, args: 0x16e780 },
			Runner { queue: 3, entry: 0x202208, ret: 0x202254, count: 0x15f430, fns: 0x16e880, args: 0x16e980 },
		],
		ends: &[(0x2a0750, 0x2015e4), (0x2078c0, 0x2016b4), (0x2a0750, 0x2016c8), (0x201e60, 0x201428), (0x201e60, 0x2014fc)],
	},
];

#[derive(Clone, Debug)]
enum Ev {
	Start { queue: u8, fns: Vec<(u32, u32)>, view: [u32; 20], c: u32 },
	Cb { fnp: u32, arg: u32, c: u32 },
	End { c: u32, slots: Vec<u64> },
}

static ON: AtomicBool = AtomicBool::new(true);
static EVS: Mutex<Vec<Ev>> = Mutex::new(Vec::new());
/// Probes registered: (entry, hash of the first 8 bytes).
static PROBES: Mutex<Option<HashSet<(u32, u64)>>> = Mutex::new(None);
/// The hand that was at an entry before the probe (fn, est), per (entry, level).
type Prior = Option<(fn(&mut Cpu) -> bool, u64)>;
static CHAIN: Mutex<Option<HashMap<(u32, u8), Prior>>> = Mutex::new(None);

fn level_of(cpu: &Cpu) -> Option<&'static Level> {
	let lv = cpu.mem.ram[LEVEL as usize];
	(*crate::retail::disc::pick(&LEVELS, &LEVELS_USA)).iter().find(|l| l.level == lv)
}

pub fn enabled() -> bool {
	ON.load(Ordering::Relaxed)
}

/// The hand that would run at `e` on this level without the probe (a registered hand whose code hash matches RAM).
fn prior(cpu: &Cpu, e: u32) -> Prior {
	let lv = cpu.mem.ram[LEVEL as usize];
	let mut g = CHAIN.lock().unwrap();
	let m = g.get_or_insert_with(HashMap::new);
	*m.entry((e, lv)).or_insert_with(|| {
		hands()
			.into_iter()
			.filter(|h| h.entry == e && h.f as *const () != probe as fn(&mut Cpu) -> bool as *const () && hash(&cpu.mem, h.entry, h.end) == h.hash)
			.last()
			.map(|h| (h.f, h.est))
	})
}

/// The fx slot table (16-byte records, +0 = the slot's cached TEX0) as it is now. Retail clears the cached TEX0s at the start
/// of every packet block (FUN_0021b9d0: `DAT_0016d580[i] = 0` for DAT_0015f41c slots) and re-uploads a texture on its next use, so
/// the table at the end of a field only holds the last block's slots; the snapshot at the end of a queue run holds the run's.
fn slot_table(cpu: &mut Cpu, level: u8) -> Vec<u64> {
	let base = va(0x16d580u32.wrapping_add_signed(*super::LEVEL_SHIFT.get(level as usize).unwrap_or(&0)));
	let n = cpu.mem.rd32(0x15f41c).min(0x100);
	let mut v = Vec::with_capacity(n as usize);
	for i in 0..n {
		v.push(cpu.mem.rd32(base + 16 * i) as u64 | (cpu.mem.rd32(base + 16 * i + 4) as u64) << 32);
	}
	v
}

/// The slot table snapshots of the last `field_end` (indexed by `Own.slots - 1`).
static SNAPS: Mutex<Vec<Vec<u64>>> = Mutex::new(Vec::new());

/// The fx slot whose TEX0 was `tex0` when the queue run of `o` ended (exact 64-bit match); None without a snapshot.
pub fn slot_of(o: &Own, tex0: u64) -> Option<Option<usize>> {
	let g = SNAPS.lock().ok()?;
	let s = g.get((o.slots as usize).checked_sub(1)?)?;
	Some(if tex0 == 0 { None } else { s.iter().position(|&x| x == tex0) })
}

/// The probe: note the event, then run what would have run (the prior hand, else the compiled retail body).
fn probe(cpu: &mut Cpu) -> bool {
	let e = cpu.pc;
	if let Some(l) = level_of(cpu) {
		let ra = cpu.r[31] as u32;
		let c = cpu.mem.rd32(l.cursor);
		let mut ev = Vec::new();
		for r in &l.runners {
			if e == r.entry {
				let n = (cpu.mem.rd32(r.count) as i32).clamp(0, 64) as u32;
				let fns = (0..n).map(|i| (cpu.mem.rd32(r.fns + 4 * i), cpu.mem.rd32(r.args + 4 * i))).collect();
				ev.push(Ev::Start { queue: r.queue, fns, view: view_words(cpu, l.level), c });
			}
			if ra == r.ret {
				ev.push(Ev::Cb { fnp: e, arg: cpu.r[4] as u32, c });
			}
		}
		if l.ends.iter().any(|&(f, a)| f == e && a == ra) {
			ev.push(Ev::End { c, slots: slot_table(cpu, l.level) });
		}
		if !ev.is_empty() {
			if let Ok(mut v) = EVS.lock() {
				v.extend(ev);
			}
		}
	}
	match prior(cpu, e) {
		Some((f, _)) => f(cpu),
		None => false,
	}
}

/// Before the first field of a loaded savestate (EE not run yet, before `Machine::enable_rec`, which then builds the
/// table with them): the probes `field_end` would only make at the end of that field, from the level and the queue lists
/// the savestate holds (the lists of the field it was saved in). Without them the first field has no records and draws
/// no effects layer (Veldin 18 goo strip at the cliff feet, Pokitaru sea over the island skirts: fullgame2 0252 field 1).
pub fn prime(cpu: &mut Cpu) {
	if enabled() {
		add_probes(cpu);
	}
}

/// Field end (EE parked): make probes for entries not covered yet (runners, end callees, the callbacks now in the
/// queues; the table is rebuilt so they apply from the next field) and return this field's records in call order.
pub fn field_end(cpu: &mut Cpu) -> Vec<Own> {
	let evs = EVS.lock().map(|mut v| std::mem::take(&mut *v)).unwrap_or_default();
	if let Ok(mut g) = SNAPS.lock() {
		g.clear();
	}
	if !enabled() {
		return Vec::new();
	}
	add_probes(cpu);
	assemble(&evs)
}

/// Probes for the entries of the current level not covered yet: runners, end callees and the callbacks now in the
/// queues. With a recompiled table in use it is rebuilt, so they apply from the next field.
fn add_probes(cpu: &mut Cpu) {
	let Some(l) = level_of(cpu) else { return };
	let mut want: Vec<u32> = l.runners.iter().map(|r| r.entry).chain(l.ends.iter().map(|e| e.0)).collect();
	for r in &l.runners {
		let n = (cpu.mem.rd32(r.count) as i32).clamp(0, 64) as u32;
		want.extend((0..n).map(|i| cpu.mem.rd32(r.fns + 4 * i)));
	}
	let mut add = Vec::new();
	{
		let mut g = PROBES.lock().unwrap();
		let set = g.get_or_insert_with(HashSet::new);
		for e in want {
			if e & 3 != 0 || !(0x100000..0x1f0_0000).contains(&e) {
				continue;
			}
			let h = hash(&cpu.mem, e, e + 8);
			if set.insert((e, h)) {
				let est = prior(cpu, e).map_or(0, |p| p.1);
				add.push(Hand { entry: e, end: e + 8, hash: h, est, f: probe });
			}
		}
	}
	if !add.is_empty() {
		let n = add.len();
		register_hand(Box::leak(add.into_boxed_slice()));
		if cpu.rec.is_some() {
			let t0 = std::time::Instant::now();
			let lv = cpu.mem.ram[LEVEL as usize] as u32;
			cpu.rec = Some(Box::new(Table::build(&cpu.mem, lv)));
			if std::env::var_os("RAC_EFX_LOG").is_some() {
				eprintln!("efxown: {n} probes added, table rebuilt in {:.1} ms", t0.elapsed().as_secs_f64() * 1e3);
			}
		} else if std::env::var_os("RAC_EFX_LOG").is_some() {
			eprintln!("efxown: {n} probes added before the table is built");
		}
	}
}

/// A queue run being assembled: queue, the queue's fn list, the callbacks seen, the view, the cursor at the runner's entry.
type Run = (u8, Vec<(u32, u32)>, Vec<Own>, [u32; 20], u32);

/// The retail view of a queue run, read at the runner's entry (the callbacks project with it): camera eye and basis rows
/// forward / left / up (retail.rs `camera_globals`), then the projection block of FUN_00219480 (Novalis; Rilgar twin
/// 0x226568 per gsfull/wide.rs) at Novalis's addresses plus the level's shift: near 0x16cee0, x / y scale 0x16cf00 /
/// 0x16cf14, z terms 0x16cf28 / 0x16cf38, screen offset vector 0x16cfe0 x / y / z (camera.md projection table) [C].
fn view_words(cpu: &Cpu, level: u8) -> [u32; 20] {
	let mut w = [0u32; 20];
	let ram = &cpu.mem.ram;
	let rd = |a: u32| ram.get(a as usize..a as usize + 4).map_or(0, |b| u32::from_le_bytes(b.try_into().unwrap()));
	let (Some((eye, basis, _)), Some(&d)) = (super::camera_globals(level as u32), super::LEVEL_SHIFT.get(level as usize)) else { return w };
	for k in 0..3u32 {
		let i = k as usize;
		w[i] = rd(eye + 4 * k);
		w[3 + i] = rd(basis + 4 * k);
		w[6 + i] = rd(basis + 0x10 + 4 * k);
		w[9 + i] = rd(basis + 0x20 + 4 * k);
	}
	for (i, a) in [va(0x16cee0u32), va(0x16cf00), va(0x16cf14), va(0x16cf28), va(0x16cf38), va(0x16cfe0), va(0x16cfe4), va(0x16cfe8)].into_iter().enumerate() {
		w[12 + i] = rd(a.wrapping_add_signed(d));
	}
	w
}

/// `seen` is `fns` with entries removed (same order).
fn is_subsequence(seen: &[(u32, u32)], fns: &[(u32, u32)]) -> bool {
	let mut it = fns.iter();
	seen.iter().all(|s| it.any(|f| f == s))
}

fn close(out: &mut Vec<Own>, run: Run, end: Option<u32>, slots: u32) {
	let (queue, fns, mut list, view, start) = run;
	let seen: Vec<(u32, u32)> = list.iter().map(|o| (o.fnp, o.arg)).collect();
	// Callbacks that no probe saw at the head of the list (a callback of code the level's recompiled set does not hold,
	// e.g. the held Mine Glove's: it runs in the interpreter, where hands do not apply): nothing runs between the runner's
	// entry and the first seen callback but them, so their packets are exactly [cursor at the runner's entry, first seen
	// callback's start) (or the end cursor when none was seen). The first unseen callback owns that range, the others
	// are empty, so the run stays exact and the seen callbacks keep their bounds.
	let miss = seen.first().map_or(fns.len(), |s| fns.iter().position(|f| f == s).unwrap_or(0));
	if end.is_some() && miss > 0 && is_subsequence(&seen, &fns) {
		let first = list.first().map_or(end.unwrap_or(start), |o| o.c0);
		if first >= start {
			for o in list.iter_mut() {
				o.idx += miss as u16;
			}
			let pre = fns[..miss].iter().enumerate().map(|(i, &(fnp, arg))| {
				let (c0, c1) = if i == 0 { (start, first) } else { (first, first) };
				Own { queue, idx: i as u16, fnp, arg, c0, c1, ok: false, view, slots }
			});
			list.splice(0..0, pre);
		}
	}
	// The runner also skips queue entries (a callback is never entered when its draw is not wanted: Quartu 0x2d8040 in
	// the tank room, its entry probe never fires, gsfull draws the water of the three callbacks after it), so the
	// callbacks seen need only be the queue's list in order with entries left out: an entry that is not entered writes
	// nothing, the cursor bounds still give every seen callback its own packets.
	let seen: Vec<(u32, u32)> = list.iter().map(|o| (o.fnp, o.arg)).collect();
	let ok = end.is_some() && is_subsequence(&seen, &fns);
	if !ok && std::env::var_os("RAC_EFX_LOG").is_some() {
		eprintln!("efxrun inexact: end {end:x?} list {:x?} seen {:x?}", fns, seen);
	}
	if let (Some(c), Some(last)) = (end, list.last_mut()) {
		last.c1 = c;
	}
	for o in list.iter_mut() {
		o.ok = ok && o.c1 != 0;
	}
	out.extend(list);
}

/// Events -> records. A run is exact when it starts at a runner entry, sees the callbacks of the queue's list in order
/// and ends at the end probe; the last callback ends at the end probe's cursor.
fn assemble(evs: &[Ev]) -> Vec<Own> {
	let mut out = Vec::new();
	let mut open: Option<Run> = None;
	for e in evs {
		match e {
			Ev::Start { queue, fns, view, c } => {
				if let Some(run) = open.take() {
					close(&mut out, run, None, 0);
				}
				open = Some((*queue, fns.clone(), Vec::new(), *view, *c));
			}
			Ev::Cb { fnp, arg, c } => {
				if let Some((q, _, list, view, _)) = open.as_mut() {
					if let Some(last) = list.last_mut() {
						last.c1 = *c;
					}
					let idx = list.len() as u16;
					list.push(Own { queue: *q, idx, fnp: *fnp, arg: *arg, c0: *c, c1: 0, ok: false, view: *view, slots: 0 });
				}
			}
			Ev::End { c, slots } => {
				if let Some(mut run) = open.take() {
					let mut g = SNAPS.lock().unwrap();
					g.push(slots.clone());
					for o in run.2.iter_mut() {
						o.slots = g.len() as u32;
					}
					let n = g.len() as u32;
					drop(g);
					close(&mut out, run, Some(*c), n);
				}
			}
		}
	}
	if let Some(run) = open.take() {
		close(&mut out, run, None, 0);
	}
	out
}

/// One callback's packets as the DMA engine reads them (oracle `mem.rs vif1_kick`, TTE on): per tag its two VIF words
/// (mark phase 2), then the block's data (mark phase 0), with the block's RAM source address. `ok`: the walk went
/// through cnt / ref / refs / next tags only and landed exactly on the callback's end cursor.
#[derive(Clone, Debug, Default)]
pub struct Pkt {
	pub words: Vec<u32>,
	pub marks: Vec<(usize, u32)>,
	/// (word index of the block's data, source address, qword count, tag id) per block.
	pub blocks: Vec<(usize, u32, u32, u8)>,
	pub ok: bool,
}

/// RAM or scratchpad qword (`dma_qw`: bit 31 of a DMA address selects the scratchpad).
fn qw(cpu: &Cpu, a: u32) -> u128 {
	if a & 0x8000_0000 != 0 {
		let i = (a & 0x3ff0) as usize;
		u128::from_le_bytes(cpu.mem.spr[i..i + 16].try_into().unwrap())
	} else {
		let i = (a & 0x01ff_fff0) as usize;
		u128::from_le_bytes(cpu.mem.ram[i..i + 16].try_into().unwrap())
	}
}

/// Walks one callback's range [c0, c1) of the packet buffer (at field end, before the kick reads it).
pub fn walk(cpu: &Cpu, o: &Own) -> Pkt {
	let mut p = Pkt { ..Default::default() };
	if !o.ok || o.c1 < o.c0 || o.c1 - o.c0 > 0x10_0000 {
		return p;
	}
	let mut tadr = o.c0;
	while tadr != o.c1 {
		if tadr < o.c0 || tadr > o.c1 || p.blocks.len() > 0x4000 {
			return p;
		}
		let t = qw(cpu, tadr);
		let tag = t as u64;
		let q = (tag & 0xffff) as u32;
		let id = ((tag >> 28) & 7) as u8;
		let addr = ((tag >> 32) as u32) & 0x7fff_fff0;
		let (src, next) = match id {
			1 => (tadr + 16, tadr + 16 + q * 16),
			2 => (tadr + 16, addr),
			3 | 4 => ((tag >> 32) as u32 & 0xffff_fff0, tadr + 16),
			_ => return p,
		};
		p.marks.push((p.words.len(), 2));
		p.words.push((t >> 64) as u32);
		p.words.push((t >> 96) as u32);
		p.blocks.push((p.words.len(), src, q, id));
		p.marks.push((p.words.len(), 0));
		for k in 0..q {
			let d = qw(cpu, src + 16 * k);
			p.words.extend([d as u32, (d >> 32) as u32, (d >> 64) as u32, (d >> 96) as u32]);
		}
		tadr = next;
	}
	p.ok = true;
	p
}

/// VIF commands in a packet stream (the words as the VIF reads them): counts of (MSCAL, MSCALF, MSCNT, DIRECT,
/// UNPACK, other) and whether the stream ends exactly at a command boundary (payloads do not run past the packet).
pub fn vif_census(words: &[u32]) -> ([u32; 6], bool) {
	let mut n = [0u32; 6];
	let mut i = 0usize;
	let (mut cl, mut wl) = (1u32, 1u32);
	while i < words.len() {
		let w = words[i];
		let cmd = (w >> 24) & 0x7f;
		let imm = w & 0xffff;
		let num = (w >> 16) & 0xff;
		i += 1;
		let len = match cmd {
			0x01 => {
				cl = imm & 0xff;
				wl = (imm >> 8) & 0xff;
				0
			}
			0x14 => { n[0] += 1; 0 }
			0x15 => { n[1] += 1; 0 }
			0x17 => { n[2] += 1; 0 }
			0x20 => 1,
			0x30 | 0x31 => 4,
			0x4a => { n[5] += 1; (if num == 0 { 256 } else { num }) as usize * 2 }
			0x50 | 0x51 => { n[3] += 1; (if imm == 0 { 0x10000 } else { imm }) as usize * 4 }
			0x60..=0x7f => {
				n[4] += 1;
				let (vn, vl) = ((cmd >> 2) & 3, cmd & 3);
				let num = if num == 0 { 256 } else { num };
				// Rows written: num; rows read from the stream: num when WL <= CL, else CL per WL-row block.
				let rows = if wl <= cl || wl == 0 { num } else { (num / wl) * cl + (num % wl).min(cl) };
				(((32 >> vl) * (vn + 1) * rows) as usize).div_ceil(32)
			}
			_ => 0,
		};
		i += len;
	}
	(n, i == words.len())
}

/// GIF content of the DIRECT payloads in a packet stream (PATH2: the EE built the GS packets itself, no VU1 program):
/// PRIM types, vertex kicks, screen x / y (pixels, XYOFFSET included) and z / q ranges, A+D registers written, and the
/// distinct TEX0 / ALPHA / TEST values. For the effects layer's facts log (RAC_EFX_LOG).
#[derive(Debug, Default)]
pub struct GifSum {
	pub prims: [u32; 8],
	pub kicks: u32,
	pub xy: [f32; 4],
	pub z: [u32; 2],
	pub q: [f32; 2],
	pub ad: Vec<u8>,
	pub tex0: Vec<u64>,
	pub alpha: Vec<u64>,
	pub test: Vec<u64>,
	pub flg: [u32; 3],
}

pub fn gif_census(words: &[u32]) -> GifSum {
	let mut g = GifSum { xy: [f32::MAX, f32::MAX, f32::MIN, f32::MIN], z: [u32::MAX, 0], q: [f32::MAX, f32::MIN], ..Default::default() };
	let qw = |i: usize| -> u128 { (0..4).fold(0u128, |a, k| a | (words.get(i + k).copied().unwrap_or(0) as u128) << (32 * k)) };
	let mut i = 0usize;
	let mut q = 1.0f32;
	let vert = |g: &mut GifSum, x: u32, y: u32, z: u32, q: f32| {
		g.kicks += 1;
		let (x, y) = (x as f32 / 16.0, y as f32 / 16.0);
		g.xy = [g.xy[0].min(x), g.xy[1].min(y), g.xy[2].max(x), g.xy[3].max(y)];
		g.z = [g.z[0].min(z), g.z[1].max(z)];
		g.q = [g.q[0].min(q), g.q[1].max(q)];
	};
	while i < words.len() {
		let w = words[i];
		let cmd = (w >> 24) & 0x7f;
		let imm = (w & 0xffff) as usize;
		i += 1;
		match cmd {
			0x50 | 0x51 => {
				let end = (i + if imm == 0 { 0x10000 } else { imm } * 4).min(words.len());
				while i + 4 <= end {
					let t = qw(i);
					i += 4;
					let nloop = (t & 0x7fff) as usize;
					let flg = ((t >> 58) & 3) as usize;
					let nreg = match ((t >> 60) & 15) as usize { 0 => 16, n => n };
					g.flg[flg.min(2)] += 1;
					if (t >> 46) & 1 != 0 {
						g.prims[((t >> 47) & 7) as usize] += 1;
					}
					let regs: Vec<u8> = (0..nreg).map(|k| ((t >> (64 + 4 * k)) & 15) as u8).collect();
					match flg {
						0 => {
							for _ in 0..nloop {
								for &r in &regs {
									if i + 4 > end {
										break;
									}
									let d = qw(i);
									i += 4;
									match r {
										0 => g.prims[(d & 7) as usize] += 1,
										2 => q = f32::from_bits((d >> 64) as u32),
										4 => vert(&mut g, (d & 0xffff) as u32, ((d >> 32) & 0xffff) as u32, ((d >> 68) & 0xff_ffff) as u32, q),
										5 => vert(&mut g, (d & 0xffff) as u32, ((d >> 32) & 0xffff) as u32, (d >> 64) as u32, q),
										14 => {
											let (a, v) = (((d >> 64) & 0xff) as u8, d as u64);
											if !g.ad.contains(&a) {
												g.ad.push(a);
											}
											let l = match a { 0x06 | 0x07 => &mut g.tex0, 0x42 | 0x43 => &mut g.alpha, 0x47 | 0x48 => &mut g.test, _ => continue };
											if !l.contains(&v) {
												l.push(v);
											}
										}
										_ => {}
									}
								}
							}
						}
						1 => {
							// REGLIST: one 64-bit word per register descriptor, the loop padded to a whole qword.
							let n = nloop * nreg;
							let dw = |k: usize| -> u64 { words.get(i + 2 * k).copied().unwrap_or(0) as u64 | (words.get(i + 2 * k + 1).copied().unwrap_or(0) as u64) << 32 };
							for k in 0..n {
								let d = dw(k);
								match regs[k % nreg] {
									0 => g.prims[(d & 7) as usize] += 1,
									1 => q = f32::from_bits((d >> 32) as u32),
									4 => vert(&mut g, (d & 0xffff) as u32, ((d >> 16) & 0xffff) as u32, ((d >> 32) & 0xff_ffff) as u32, q),
									5 => vert(&mut g, (d & 0xffff) as u32, ((d >> 16) & 0xffff) as u32, (d >> 32) as u32, q),
									6 | 7 => {
										if !g.tex0.contains(&d) {
											g.tex0.push(d);
										}
									}
									r => {
										if !g.ad.contains(&(0x80 | r)) {
											g.ad.push(0x80 | r);
										}
									}
								}
							}
							i += n.div_ceil(2) * 4;
						}
						_ => i += nloop * 4,
					}
				}
				i = end;
			}
			0x20 => i += 1,
			0x30 | 0x31 => i += 4,
			0x4a => i += (if (w >> 16) & 0xff == 0 { 256 } else { (w >> 16) & 0xff }) as usize * 2,
			0x60..=0x7f => {
				let (vn, vl) = ((cmd >> 2) & 3, cmd & 3);
				let num = match (w >> 16) & 0xff { 0 => 256, n => n };
				i += (((32 >> vl) * (vn + 1) * num) as usize).div_ceil(32);
			}
			_ => {}
		}
	}
	g
}

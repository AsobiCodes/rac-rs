//! The Galactic Map mission checklist (decomp/notes/level_transition.md "Mission checklist"): per-planet
//! records (Veldin overlay table 0x186c70 / Novalis 0x187070, 0x28 bytes each; both copies agree) whose
//! state comes from FUN_0024d818 / N FUN_002622f8 and the condition test FUN_0024d9b8 / N FUN_00262498.

use crate::save::Story;

/// One record: text id (+0x00), flags (+0x10: 1 hide once complete, 2 never listed, 4 needs the planet
/// visited), shown-when condition (+0x02, +0x04), complete-when condition (+0x08, +0x0c).
pub struct Mission {
	pub text: u16,
	flags: u8,
	show: (u8, u32),
	done: (u8, u32),
}

const fn m(text: u16, flags: u8, show: (u8, u32), done: (u8, u32)) -> Mission {
	Mission { text, flags, show, done }
}
use m as M;

/// Records by planet id (0..18).
pub const MISSIONS: [&[Mission]; 19] = [
	&[M(0x53b2, 0, (0, 0x0), (1, 0x3))],
	&[M(0x5320, 4, (0, 0x0), (1, 0x3)), M(0x5321, 4, (0, 0x0), (4, 0x3)), M(0x5322, 0, (4, 0x3), (1, 0x2)), M(0xffff, 2, (1, 0x3), (9, 0x0))],
	&[M(0x5329, 0, (0, 0x0), (4, 0x10)), M(0x532a, 0, (4, 0x10), (2, 0x1e)), M(0x532b, 0, (0, 0x0), (4, 0x11)), M(0x532c, 0, (4, 0x11), (2, 0x5)), M(0x532d, 0, (6, 0x14), (2, 0x1a))],
	&[M(0x5336, 0, (0, 0x0), (4, 0x14)), M(0x5337, 0, (4, 0x14), (2, 0x2)), M(0x5338, 4, (0, 0x0), (7, 0x0)), M(0x5339, 0, (6, 0x18), (1, 0x4)), M(0x533a, 0, (4, 0x18), (4, 0x15)), M(0x533b, 0, (4, 0x15), (2, 0xc)), M(0xffff, 2, (0, 0x0), (4, 0x15)), M(0xffff, 2, (0, 0x0), (1, 0x4))],
	&[M(0x533e, 0, (0, 0x0), (1, 0x6)), M(0x533f, 4, (0, 0x0), (3, 0x9))],
	&[M(0x5344, 1, (0, 0x0), (4, 0x26)), M(0x5344, 0, (4, 0x26), (6, 0x0)), M(0x5345, 0, (0, 0x0), (4, 0x27)), M(0x5346, 0, (4, 0x27), (1, 0x7)), M(0xffff, 2, (4, 0x28), (3, 0x17))],
	&[M(0x534d, 0, (0, 0x0), (7, 0x1)), M(0x534e, 0, (4, 0x2f), (2, 0x1d)), M(0x534f, 0, (6, 0x21), (6, 0x22)), M(0x5350, 0, (6, 0x22), (6, 0x24)), M(0x5351, 0, (6, 0x20), (2, 0x16)), M(0xffff, 2, (0, 0x0), (6, 0x22)), M(0xffff, 2, (0, 0x0), (4, 0x2f))],
	&[M(0x5355, 0, (0, 0x0), (6, 0x29)), M(0x5356, 0, (6, 0x29), (4, 0x34)), M(0x5357, 0, (4, 0x34), (1, 0x8))],
	&[M(0x535f, 0, (0, 0x0), (6, 0x2f)), M(0x5360, 0, (6, 0x2f), (4, 0x38)), M(0x5361, 0, (4, 0x38), (1, 0x9)), M(0x5362, 0, (0, 0x0), (4, 0x37)), M(0x5363, 0, (4, 0x37), (1, 0xa)), M(0x5364, 0, (1, 0xa), (4, 0x39)), M(0x5365, 0, (4, 0x39), (2, 0x1b))],
	&[M(0x5368, 0, (0, 0x0), (2, 0x7))],
	&[M(0x5370, 0, (0, 0x0), (2, 0x1c)), M(0x5371, 0, (2, 0x1c), (1, 0xb)), M(0x5373, 4, (0, 0x0), (7, 0x2)), M(0x5372, 0, (4, 0x3f), (1, 0xc)), M(0x5374, 0, (4, 0x40), (6, 0x4)), M(0xffff, 2, (6, 0x4), (6, 0x5)), M(0xffff, 2, (0, 0x0), (1, 0xb)), M(0xffff, 2, (2, 0x6), (1, 0xc)), M(0xffff, 2, (2, 0x6), (4, 0x40))],
	&[M(0x537c, 0, (0, 0x0), (7, 0x3)), M(0xffff, 2, (0, 0x0), (4, 0x42)), M(0x537d, 0, (4, 0x42), (5, 0x42)), M(0x537e, 0, (5, 0x42), (2, 0x3)), M(0x537f, 0, (2, 0x3), (2, 0x6)), M(0x5380, 0, (6, 0x55), (4, 0x44)), M(0xffff, 2, (0, 0x0), (4, 0x44)), M(0x5381, 0, (4, 0x44), (2, 0x23))],
	&[M(0x5385, 0, (0, 0x0), (1, 0xd)), M(0x5386, 4, (0, 0x0), (7, 0x4)), M(0xffff, 2, (0, 0x0), (4, 0x46)), M(0x5387, 0, (4, 0x46), (2, 0x4)), M(0xffff, 2, (0, 0x0), (6, 0x61))],
	&[M(0x538a, 0, (0, 0x0), (4, 0x4b)), M(0x538b, 0, (4, 0x4b), (1, 0xe))],
	&[M(0x5390, 0, (0, 0x0), (4, 0x58)), M(0xffff, 2, (0, 0x0), (4, 0x58)), M(0x5391, 0, (4, 0x58), (1, 0xf)), M(0x5392, 0, (0, 0x0), (7, 0x5)), M(0xffff, 2, (0, 0x0), (4, 0x57)), M(0x5393, 0, (4, 0x57), (3, 0x20)), M(0xffff, 2, (0, 0x0), (3, 0x15))],
	&[M(0xffff, 2, (0, 0x0), (1, 0x11)), M(0x539b, 0, (0, 0x0), (4, 0x5e)), M(0x539c, 0, (4, 0x5e), (1, 0x10)), M(0x539e, 4, (0, 0x0), (3, 0x22)), M(0x539f, 0, (6, 0x6e), (1, 0x11)), M(0xffff, 2, (0, 0x0), (3, 0x22))],
	&[M(0x53a7, 0, (0, 0x0), (7, 0x6)), M(0x53a8, 0, (6, 0x70), (4, 0x66)), M(0xffff, 2, (0, 0x0), (4, 0x66)), M(0x53a9, 0, (4, 0x66), (2, 0x1f)), M(0x53aa, 0, (6, 0x71), (3, 0x21)), M(0xffff, 2, (0, 0x0), (3, 0x21))],
	&[M(0x53ad, 0, (0, 0x0), (1, 0x12)), M(0x53ae, 4, (0, 0x0), (6, 0x78))],
	&[M(0x53b0, 0, (0, 0x0), (6, 0x7d))],
];

/// Kind-7 conditions: the record's callback (0x24dbf8.., disassembled; not in the decompile) is the AND
/// of these flag tests.
const CALLBACKS: [&[(u8, u32)]; 7] = [
	&[(4, 0x18), (6, 0x18)],
	&[(4, 0x2f), (6, 0x20), (6, 0x21)],
	&[(4, 0x40), (4, 0x3f)],
	&[(4, 0x42), (6, 0x55)],
	&[(4, 0x46), (6, 0x61)],
	&[(4, 0x57), (3, 0x15)],
	&[(2, 0x21), (2, 0x1f)],
];

/// FUN_0024d9b8: 0 always; 1 planet unlocked (0x13dd40); 2 item owned (0x13d4c0); 3 item new flag
/// (0x13d4e8); 4 dialogue started and 5 progressed (u32 at 0x13d5bc + n*16 nonzero / above 1, n <= 0x78);
/// 6 story byte (0x13d388); 7 callback; 8 gold bolt (0x14bec0 + level*4 + n, arg = level << 16 | n);
/// anything else false.
fn test(st: &Story, (kind, arg): (u8, u32)) -> bool {
	let a = arg as usize;
	let dialogue = || {
		let at = 0xc + a * 16;
		(arg <= 0x78).then(|| st.npc.get(at..at + 4).map(|b| i32::from_le_bytes([b[0], b[1], b[2], b[3]]))).flatten()
	};
	match kind {
		0 => true,
		1 => st.unlock.get(a).is_some_and(|&b| b != 0),
		2 => st.inv.have.get(a).copied().unwrap_or(false),
		3 => st.new_flags.get(a).is_some_and(|&b| b != 0),
		4 => dialogue().is_some_and(|v| v != 0),
		5 => dialogue().is_some_and(|v| v > 1),
		6 => st.a5.get(a).is_some_and(|&b| b != 0),
		7 => CALLBACKS.get(a).is_some_and(|c| c.iter().all(|&k| test(st, k))),
		8 => st.gold.get(a >> 16).and_then(|g| g.get(a & 0xffff)).is_some_and(|&b| b != 0),
		_ => false,
	}
}

/// The checklist of `planet` (FUN_0024dac8 with no descriptions): (text id, complete) per listed row,
/// and whether every countable record is complete (bit 31, 'All Missions Complete'). State per record:
/// 0 hidden (shown-when false, or flag 4 before the planet is visited), 1 active, 2 complete.
pub fn checklist(planet: usize, st: &Story) -> (Vec<(u32, bool)>, bool) {
	let Some(list) = MISSIONS.get(planet) else { return (Vec::new(), false) };
	let visited = st.visited.get(planet).is_some_and(|&v| v != 0);
	let (mut rows, mut all) = (Vec::new(), true);
	for r in list.iter() {
		let state = if r.flags & 4 != 0 && !visited || !test(st, r.show) { 0 } else if test(st, r.done) { 2 } else { 1 };
		if state != 2 && r.flags & 2 == 0 {
			all = false;
		}
		if r.flags & 2 == 0 && state != 0 && !(r.flags & 1 != 0 && state == 2) {
			rows.push((r.text as u32, state == 2));
		}
	}
	(rows, all)
}

#[cfg(test)]
mod tests {
	use super::*;

	/// Veldin lists 'reach Novalis' until planet 3 unlocks; Novalis's visit-gated rows wait for a visit,
	/// and its list completes once the gated rows are done.
	#[test]
	fn novalis_checklist() {
		let mut st = Story::default();
		assert_eq!(checklist(0, &st), (vec![(0x53b2, false)], false));
		assert_eq!(checklist(1, &st).0, vec![]);
		st.visited[1] = 1;
		assert_eq!(checklist(1, &st).0, vec![(0x5320, false), (0x5321, false)]);
		st.unlock[3] = 1;
		st.npc[0xc + 3 * 16] = 1;
		st.unlock[2] = 1;
		assert_eq!(checklist(1, &st), (vec![(0x5320, true), (0x5321, true), (0x5322, true)], true));
		assert_eq!(checklist(0, &st), (vec![(0x53b2, true)], true));
	}
}

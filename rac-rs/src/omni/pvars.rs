//! The host's `pvars` callback (`omni.pvars(moby)`): the variables of a moby's class, named by the binding.
//!
//! Data-driven: the moby's class id and pvar pointer are read at the offsets of the binding's `structs.moby` fields `class` and `pvar`, the class is looked
//! up in the binding's `pvars` section (`"<class id>": "enemy"` = the fields of `structs.enemy`, or a list of `{name, offset, type}`), and each variable
//! is read from `pvar + offset` and handed to the loader's `emit`. A class the section does not list, a moby outside RAM or a null pvar answer 0
//! (the loader gives the script nil). Only reads; nothing is written and no game code runs.

use super::binding::{Binding, PvarDef, Ty};
use super::cabi::PvarEmit;
use super::host::{cpu, state};
use std::ffi::{c_char, c_void, CString};

/// RAM below this is the executable image: a moby or pvar pointer there is garbage.
const HEAP_FLOOR: u32 = 0x0010_0000;

fn rd(ram: &[u8], addr: u32, n: usize) -> Option<&[u8]> {
	let a = addr as usize;
	ram.get(a..a.checked_add(n)?)
}

/// The value of a variable of type `ty` at `addr`, None for an unreadable address or a type a pvar cannot be (void, 64-bit, f64).
fn read_value(ram: &[u8], addr: u32, ty: Ty) -> Option<f64> {
	Some(match ty {
		Ty::U8 => f64::from(rd(ram, addr, 1)?[0]),
		Ty::I8 => f64::from(rd(ram, addr, 1)?[0] as i8),
		Ty::U16 => f64::from(u16::from_le_bytes(rd(ram, addr, 2)?.try_into().ok()?)),
		Ty::I16 => f64::from(i16::from_le_bytes(rd(ram, addr, 2)?.try_into().ok()?)),
		Ty::U32 => f64::from(u32::from_le_bytes(rd(ram, addr, 4)?.try_into().ok()?)),
		Ty::I32 => f64::from(i32::from_le_bytes(rd(ram, addr, 4)?.try_into().ok()?)),
		Ty::F32 => f64::from(f32::from_le_bytes(rd(ram, addr, 4)?.try_into().ok()?)),
		_ => return None,
	})
}

/// The variables of `moby` as (name, value): None when the binding has no pvars for its class (or the moby is not a readable record).
pub fn pvars_of(b: &Binding, ram: &[u8], moby: u32) -> Option<Vec<(String, f64)>> {
	if moby < HEAP_FLOOR {
		return None;
	}
	let class_off = b.struct_offset("moby", "class")?;
	let pvar_off = b.struct_offset("moby", "pvar")?;
	let class = i32::from(i16::from_le_bytes(rd(ram, moby.checked_add(class_off)?, 2)?.try_into().ok()?));
	let defs: &[PvarDef] = b.pvars_of(class)?;
	let pvar = u32::from_le_bytes(rd(ram, moby.checked_add(pvar_off)?, 4)?.try_into().ok()?);
	if pvar < HEAP_FLOOR {
		return None;
	}
	Some(defs.iter().filter_map(|d| Some((d.name.clone(), read_value(ram, pvar.checked_add(d.offset)?, d.ty)?))).collect())
}

/// `OmniHost::pvars`: 1 found (emit called per variable), 0 none.
pub unsafe extern "C" fn pvars_cb(user: *mut c_void, moby: u32, emit: PvarEmit, ctx: *mut c_void) -> i32 {
	let Some(c) = cpu() else { return 0 };
	let Some(vars) = pvars_of(&state(user).binding, &c.mem.ram, moby) else { return 0 };
	for (name, v) in vars {
		let Ok(n) = CString::new(name) else { continue };
		// SAFETY: `emit` is the loader's callback, `n` lives across the call, `ctx` is its own context.
		unsafe { emit(ctx, n.as_ptr() as *const c_char, v) };
	}
	1
}

#[cfg(test)]
mod tests {
	use super::*;

	const BINDING: &str = r#"{"game":"rac1","hooks":[],"fields":[],
	  "structs":{"moby":{"fields":[{"name":"class","offset":"0xa6","type":"i16"},{"name":"pvar","offset":"0x78","type":"u32"}]},
	             "enemy":{"fields":[{"name":"health","offset":"0x20","type":"f32"},{"name":"junk","offset":{"pal":"0x24","ntsc":"0x24"},"type":"u8"},{"name":"bad","offset":"zz","type":"u8"}]}},
	  "pvars":{"749":"enemy","12":[{"name":"x","offset":"0x4","type":"u16"}],"7":"nosuch","oops":"enemy"},"unknown_key":1}"#;

	fn ram_with(moby: usize, class: i16, pvar: u32, at: usize, bytes: &[u8]) -> Vec<u8> {
		let mut ram = vec![0u8; 0x40_0000];
		ram[moby + 0xa6..moby + 0xa8].copy_from_slice(&class.to_le_bytes());
		ram[moby + 0x78..moby + 0x7c].copy_from_slice(&pvar.to_le_bytes());
		ram[at..at + bytes.len()].copy_from_slice(bytes);
		ram
	}

	#[test]
	fn enemy_class_gives_health() {
		let b = Binding::parse(BINDING).unwrap();
		let mut bytes = 2.5f32.to_le_bytes().to_vec();
		bytes.extend_from_slice(&[9, 0, 0, 0]);
		let ram = ram_with(0x20_0000, 749, 0x30_0000, 0x30_0020, &bytes);
		assert_eq!(pvars_of(&b, &ram, 0x20_0000), Some(vec![("health".to_string(), 2.5), ("junk".to_string(), 9.0)]));
	}

	#[test]
	fn list_entry_and_misses() {
		let b = Binding::parse(BINDING).unwrap();
		let ram = ram_with(0x20_0000, 12, 0x30_0000, 0x30_0004, &[0x34, 0x12]);
		assert_eq!(pvars_of(&b, &ram, 0x20_0000), Some(vec![("x".to_string(), 4660.0)]));
		let other = ram_with(0x20_0000, 99, 0x30_0000, 0x30_0000, &[]);
		assert_eq!(pvars_of(&b, &other, 0x20_0000), None, "a class the section does not list");
		let unknown_struct = ram_with(0x20_0000, 7, 0x30_0000, 0x30_0000, &[]);
		assert_eq!(pvars_of(&b, &unknown_struct, 0x20_0000), None, "an entry naming a missing struct is dropped");
		assert_eq!(pvars_of(&b, &ram, 0), None, "null moby");
		assert_eq!(pvars_of(&b, &ram, 0x7fff_0000), None, "moby outside RAM");
		let nopvar = ram_with(0x20_0000, 749, 0, 0, &[]);
		assert_eq!(pvars_of(&b, &nopvar, 0x20_0000), None, "null pvar");
	}

	#[test]
	fn without_the_sections_nothing_is_found() {
		let b = Binding::parse(r#"{"game":"rac1","hooks":[]}"#).unwrap();
		assert_eq!(pvars_of(&b, &vec![0u8; 0x40_0000], 0x20_0000), None);
	}

	#[test]
	fn real_binding_lists_the_enemy_classes() {
		let b = Binding::parse(include_str!("rac1.json")).unwrap();
		assert_eq!(b.struct_offset("moby", "class"), Some(0xa6));
		assert_eq!(b.struct_offset("moby", "pvar"), Some(0x78));
		assert_eq!(b.struct_offset("enemy", "health"), Some(0x20));
		assert_eq!(b.pvars.len(), 94, "70 enemy classes (B3) + 18 weapon pvar classes (P2a) + 6 support weapon classes (Arsenal Mastery fields)");
		assert_eq!(b.struct_offset("held_morph", "time_left"), Some(0x18));
		assert_eq!(b.struct_offset("decoy", "health"), Some(0x48));
		assert_eq!(b.struct_offset("moby", "draw_dist"), Some(0x32));
		assert_eq!(b.pvars_of(749).map(|d| d.len()), Some(1));
		assert!(b.pvars_of(1440).is_some() && b.pvars_of(1).is_none());
	}
}

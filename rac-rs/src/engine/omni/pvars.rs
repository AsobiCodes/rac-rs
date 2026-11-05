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


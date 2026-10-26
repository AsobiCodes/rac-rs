//! The game binding (`rac1.json`, mod interface 1.3; THIRD_PARTY.md): which hook points and fields exist and where they live on the PAL and the
//! NTSC disc. The OmniLoader DLL gets the same text and owns the mods' side of it; this module reads the host's side: the base
//! addresses, the field names, and each hook's signature and symbol (whose trailing 8 hex digits are the retail entry address).

use super::json::{self, Json};

/// A value type of a hook signature.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ty {
	Void,
	I8,
	U8,
	I16,
	U16,
	I32,
	U32,
	I64,
	U64,
	F32,
	F64,
}

impl Ty {
	pub fn parse(s: &str) -> Option<Ty> {
		Some(match s.trim() {
			"void" => Ty::Void,
			"i8" => Ty::I8,
			"u8" => Ty::U8,
			"i16" => Ty::I16,
			"u16" => Ty::U16,
			"i32" => Ty::I32,
			"u32" => Ty::U32,
			"i64" => Ty::I64,
			"u64" => Ty::U64,
			"f32" => Ty::F32,
			"f64" => Ty::F64,
			_ => return None,
		})
	}
	pub fn is_float(self) -> bool {
		matches!(self, Ty::F32 | Ty::F64)
	}
}

/// `ret(arg,arg,...)`, such as `void(i32,f32,f32,f32)`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sig {
	pub ret: Ty,
	pub args: Vec<Ty>,
}

impl Sig {
	pub fn parse(s: &str) -> Result<Sig, String> {
		let (ret, rest) = s.split_once('(').ok_or_else(|| format!("signature {s:?}: no '('"))?;
		let inner = rest.strip_suffix(')').ok_or_else(|| format!("signature {s:?}: no ')'"))?;
		let ret = Ty::parse(ret).ok_or_else(|| format!("signature {s:?}: unknown return type {ret:?}"))?;
		let mut args = Vec::new();
		for a in inner.split(',').filter(|a| !a.trim().is_empty()) {
			args.push(Ty::parse(a).ok_or_else(|| format!("signature {s:?}: unknown type {a:?}"))?);
		}
		// The bridge reads integers from a0..a3,t0..t3 and floats from f12..f19 (hooks.rs `sig_supported`): 8 of each, so 11 parameters (3 floats, 8 integers) are fine.
		let floats = args.iter().filter(|t| t.is_float()).count();
		if floats > 8 || args.len() - floats > 8 {
			return Err(format!("signature {s:?}: more than 8 integer or 8 float arguments"));
		}
		Ok(Sig { ret, args })
	}
}

/// One hook point of the binding.
#[derive(Clone, Debug)]
pub struct HookDef {
	pub id: String,
	pub signature: String,
	pub pal: String,
	pub ntsc: String,
}

impl HookDef {
	/// The retail entry address named by the symbol of this disc (`func_L00_002056D8` -> 0x2056D8).
	pub fn entry(&self, ntsc: bool) -> Option<u32> {
		symbol_address(if ntsc { &self.ntsc } else { &self.pal })
	}
}

/// The address in a symbol name: its last 8 hex digits after an underscore.
pub fn symbol_address(sym: &str) -> Option<u32> {
	let tail = sym.rsplit('_').next()?;
	(tail.len() == 8).then(|| u32::from_str_radix(tail, 16).ok()).flatten()
}

fn hex(s: &str) -> Option<u32> {
	u32::from_str_radix(s.trim().trim_start_matches("0x").trim_start_matches("0X"), 16).ok()
}

/// A variable entry `{"name","offset","type"}`; the offset is a hex string, or `{"pal":..,"ntsc":..}` (the ntsc one is read: both discs we run have that layout).
fn pvar_def(f: &Json) -> Option<PvarDef> {
	let offset = match f.get("offset")? {
		Json::Str(s) => hex(s)?,
		o => o.get("ntsc").and_then(Json::str).and_then(hex)?,
	};
	let readonly = f.get("readonly").is_some_and(Json::bool);
	Some(PvarDef { name: f.get("name").and_then(Json::str)?.to_string(), offset, ty: Ty::parse(f.get("type").and_then(Json::str)?)?, readonly })
}

fn parse_structs(j: &Json) -> Vec<(String, Vec<PvarDef>)> {
	let mut out = Vec::new();
	for (name, s) in j.get("structs").map_or(&[][..], Json::entries) {
		out.push((name.clone(), s.get("fields").map_or(&[][..], Json::arr).iter().filter_map(pvar_def).collect()));
	}
	out
}

/// `"pvars": {"<class id>": "<struct name>" | [ {name, offset, type}, .. ]}`; an entry that cannot be read is skipped.
fn parse_pvars(j: &Json, structs: &[(String, Vec<PvarDef>)]) -> Vec<(i32, Vec<PvarDef>)> {
	let mut out = Vec::new();
	for (class, v) in j.get("pvars").map_or(&[][..], Json::entries) {
		let Ok(class) = class.trim().parse::<i32>() else { continue };
		let defs: Vec<PvarDef> = match v {
			Json::Str(name) => structs.iter().find(|s| &s.0 == name).map(|s| s.1.clone()).unwrap_or_default(),
			other => other.arr().iter().filter_map(pvar_def).collect(),
		};
		if !defs.is_empty() {
			out.push((class, defs));
		}
	}
	out
}

/// A named field of the game state: `base + offset`, or an absolute address per disc.
#[derive(Clone, Debug)]
pub struct FieldDef {
	pub name: String,
	pub base: Option<String>,
	pub offset: u32,
	pub pal: u32,
	pub ntsc: u32,
}

/// One variable of a class's pvar block (the binding's `pvars` section; `structs` fields have the same shape).
#[derive(Clone, Debug, PartialEq)]
pub struct PvarDef {
	pub name: String,
	pub offset: u32,
	pub ty: Ty,
	/// `"readonly": true` in the binding: the loader's `struct_set` refuses the field (a game-owned pointer, index or state).
	pub readonly: bool,
}

/// The `mobys` section: where the moby pool lives and what a slot looks like, for the host's moby walk (mobys.rs). The cells and the registry are
/// overlay-window addresses (window.rs); every other value is an offset or a constant. All keys are required; extra keys are ignored.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MobysDef {
	/// Window address of the pool cells: word `start_word` = first slot, `pool_start_word` = start of the dynamic pool, `end_word` = end of the pool.
	pub cells: u32,
	pub start_word: u32,
	pub pool_start_word: u32,
	pub end_word: u32,
	pub stride: u32,
	/// Slot state byte; `tail` ends the walk, `dead_from` and above are deleted slots.
	pub state: u32,
	pub dead_from: u32,
	pub tail: u32,
	pub class: u32,
	/// Position x (f32), then y, z at +4, +8.
	pub pos: u32,
	pub flags: u32,
	pub registered_flag: u32,
	pub has_component_flag: u32,
	pub class_ptr: u32,
	pub class_type: u32,
	pub enemy_type: i32,
	pub pvar: u32,
	/// Window address of the zero-terminated pointer array of the target registry.
	pub registry: u32,
	pub max_slots: u32,
}

/// The `skeletons.hero` section: the body the pose overrides (pose.rs) can re-pose. The names, chains and mirror pairs are the loader's
/// (it reads the same text); the host needs the class, the joint count, the pairs, the centre line and the chest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SkeletonDef {
	pub class: u32,
	pub joint_count: u32,
	/// Mirror pairs (a, b), both directions are the same pair.
	pub pairs: Vec<(u32, u32)>,
	/// Joints on the centre line (their own mirror image).
	pub centre: Vec<u32>,
	/// The chest joint (parent of both shoulders) and the attach point of the class that gives its world matrix.
	pub chest: u32,
	pub chest_attach: u32,
	/// Joint names (name, index), for the debug text.
	pub names: Vec<(String, u32)>,
}

#[derive(Clone, Debug, Default)]
pub struct Binding {
	pub hooks: Vec<HookDef>,
	/// The `structs` section: (struct name, its fields on the ntsc layout). Fields with an unreadable type or offset are skipped.
	pub structs: Vec<(String, Vec<PvarDef>)>,
	/// The `pvars` section: (class id, the variables the host hands `omni.pvars(moby)`), a class naming a struct resolved to its fields.
	pub pvars: Vec<(i32, Vec<PvarDef>)>,
	/// (name, PAL address, NTSC address)
	pub bases: Vec<(String, u32, u32)>,
	pub fields: Vec<FieldDef>,
	/// The `mobys` section; None = moby queries are not supported by this binding.
	pub mobys: Option<MobysDef>,
	/// The `skeletons.hero` section; None = no pose overrides.
	pub skeleton: Option<SkeletonDef>,
}

/// A number written as a hex string (`"0x100"`) or as a JSON number.
fn num_or_hex(j: &Json, key: &str) -> Option<u32> {
	match j.get(key)? {
		Json::Str(s) => hex(s),
		Json::Num(n) if *n >= 0.0 && *n <= f64::from(u32::MAX) => Some(*n as u32),
		_ => None,
	}
}

fn parse_mobys(j: &Json) -> Option<MobysDef> {
	let m = j.get("mobys")?;
	let n = |k: &str| num_or_hex(m, k);
	Some(MobysDef {
		cells: n("cells")?,
		start_word: n("start_word")?,
		pool_start_word: n("pool_start_word")?,
		end_word: n("end_word")?,
		stride: n("stride").filter(|&s| s >= 4)?,
		state: n("state")?,
		dead_from: n("dead_from")?,
		tail: n("tail")?,
		class: n("class")?,
		pos: n("pos")?,
		flags: n("flags")?,
		registered_flag: n("registered_flag")?,
		has_component_flag: n("has_component_flag")?,
		class_ptr: n("class_ptr")?,
		class_type: n("class_type")?,
		enemy_type: n("enemy_type")? as i32,
		pvar: n("pvar")?,
		registry: n("registry")?,
		max_slots: n("max_slots")?,
	})
}

fn parse_skeleton(j: &Json) -> Option<SkeletonDef> {
	let h = j.get("skeletons")?.get("hero")?;
	let n = |k: &str| num_or_hex(h, k);
	let nums = |a: &Json| -> Option<Vec<u32>> { a.arr().iter().map(|x| if let Json::Num(v) = x { Some(*v as u32) } else { None }).collect() };
	let joint_count = n("joint_count").filter(|&c| c > 0 && c <= 512)?;
	let pairs = h.get("mirror_pairs")?.arr().iter().map(|p| nums(p).filter(|p| p.len() == 2 && p.iter().all(|&x| x < joint_count)).map(|p| (p[0], p[1]))).collect::<Option<Vec<_>>>()?;
	let centre = nums(h.get("centre")?)?;
	let names = h.get("joints")?.entries().iter().filter_map(|(k, v)| if let Json::Num(i) = v { Some((k.clone(), *i as u32)) } else { None }).collect();
	let (chest, chest_attach) = (n("chest")?, n("chest_attach")?);
	(chest < joint_count && centre.iter().all(|&c| c < joint_count)).then_some(SkeletonDef { class: n("class")?, joint_count, pairs, centre, chest, chest_attach, names })
}

impl Binding {
	pub fn parse(text: &str) -> Result<Binding, String> {
		let j = json::parse(text)?;
		let mut b = Binding::default();
		for h in j.get("hooks").map_or(&[][..], Json::arr) {
			let sym = |disc: &str| h.get("symbols").and_then(|s| s.get(disc)).and_then(Json::str).unwrap_or("").to_string();
			b.hooks.push(HookDef {
				id: h.get("id").and_then(Json::str).ok_or("a hook without an id")?.to_string(),
				signature: h.get("signature").and_then(Json::str).unwrap_or("void()").to_string(),
				pal: sym("pal"),
				ntsc: sym("ntsc"),
			});
		}
		for (name, v) in j.get("bases").map_or(&[][..], Json::entries) {
			let at = |d: &str| v.get(d).and_then(Json::str).and_then(hex);
			if let (Some(p), Some(n)) = (at("pal"), at("ntsc")) {
				b.bases.push((name.clone(), p, n));
			}
		}
		for f in j.get("fields").map_or(&[][..], Json::arr) {
			let name = f.get("name").and_then(Json::str).ok_or("a field without a name")?.to_string();
			let addr = |d: &str| f.get("address").and_then(|a| a.get(d)).and_then(Json::str).and_then(hex).unwrap_or(0);
			b.fields.push(FieldDef {
				name,
				base: f.get("base").and_then(Json::str).map(str::to_string),
				offset: f.get("offset").and_then(Json::str).and_then(hex).unwrap_or(0),
				pal: addr("pal"),
				ntsc: addr("ntsc"),
			});
		}
		b.structs = parse_structs(&j);
		b.pvars = parse_pvars(&j, &b.structs);
		b.mobys = parse_mobys(&j);
		b.skeleton = parse_skeleton(&j);
		Ok(b)
	}

	/// The fields of a struct of the `structs` section.
	pub fn struct_fields(&self, name: &str) -> Option<&[PvarDef]> {
		self.structs.iter().find(|s| s.0 == name).map(|s| s.1.as_slice())
	}

	/// The offset of a field of a struct (`struct_offset("moby", "class")`).
	pub fn struct_offset(&self, st: &str, field: &str) -> Option<u32> {
		self.struct_fields(st)?.iter().find(|f| f.name == field).map(|f| f.offset)
	}

	/// A struct field of the `structs` section.
	pub fn struct_field(&self, st: &str, field: &str) -> Option<&PvarDef> {
		self.struct_fields(st)?.iter().find(|f| f.name == field)
	}

	/// The variables of a class's pvar block, or None for a class the section does not list.
	pub fn pvars_of(&self, class: i32) -> Option<&[PvarDef]> {
		self.pvars.iter().find(|p| p.0 == class).map(|p| p.1.as_slice())
	}

	pub fn base(&self, name: &str, ntsc: bool) -> Option<u32> {
		self.bases.iter().find(|b| b.0 == name).map(|b| if ntsc { b.2 } else { b.1 })
	}

	pub fn hook(&self, id: &str) -> Option<&HookDef> {
		self.hooks.iter().find(|h| h.id == id)
	}

	/// The binding knows this field and its base resolves on this disc.
	pub fn has_field(&self, name: &str, ntsc: bool) -> bool {
		self.fields.iter().any(|f| f.name == name && f.base.as_deref().map_or(if ntsc { f.ntsc } else { f.pal } != 0, |b| self.base(b, ntsc).is_some()))
	}
}

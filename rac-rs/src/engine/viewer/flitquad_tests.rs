//! Tests of flitquad.rs (lane fshade): the plain sources carry nothing of the lane, the fire edits apply to every maker's source
//! (both forced-early-depth choices), the twins validate with the capabilities the device gets, and every fire-lit entry stores the id.

use naga::valid::Capabilities as C;

/// What a device without `Features::SUBGROUP` gives naga.
const NO_SUB: C = C::all().difference(C::SUBGROUP).difference(C::SUBGROUP_BARRIER).difference(C::SUBGROUP_VERTEX_STAGE);
/// What the device with only `Features::SUBGROUP` asked (review W4 finding 6: not `all()`, no vertex-stage subgroup ops).
const SUB: C = C::all().difference(C::SUBGROUP_BARRIER).difference(C::SUBGROUP_VERTEX_STAGE);

fn validate(label: &str, src: &str, caps: C) -> Result<(naga::Module, naga::valid::ModuleInfo), String> {
	let m = naga::front::wgsl::parse_str(src).map_err(|e| format!("{label}: {}", e.emit_to_string(src)))?;
	let info = naga::valid::Validator::new(naga::valid::ValidationFlags::all(), caps).validate(&m).map_err(|e| format!("{label}: {e:?}"))?;
	Ok((m, info))
}

/// Every maker's plain source (setup.rs, furfx/pipes.rs, vegfx.rs), for both forced-early-depth choices, Vulkan and DX12 text.
fn sources() -> Vec<(String, String)> {
	let mut v = Vec::new();
	for early in [false, true] {
		let level = crate::viewer::metalfx::with_override(crate::viewer::volfog::soft::level_source(crate::viewer::SHADER, early));
		v.push((format!("level early={early}"), level.clone()));
		v.push((format!("level bindless early={early}"), crate::viewer::bindless::shader_source(&level)));
		v.push((format!("fur early={early}"), crate::viewer::furfx::pipes::source_with(early)));
		v.push((format!("veg early={early}"), crate::viewer::vegfx::pipes::source_with(early)));
	}
	// Review W7 item 1: the DX12 texts (texfilter.rs `frag_w_fix` runs on the source before `Modules::new`) take the same edits.
	let dx: Vec<_> = v.iter().map(|(l, s)| (format!("{l} dx12"), crate::viewer::texfilter::frag_w_for(s.clone(), true))).collect();
	v.extend(dx);
	v
}

#[test]
fn the_plain_sources_carry_nothing_of_the_lane() {
	// The sets without the lookup and the single-set pipelines are built from these: the text from before the lane (worker 6:
	// any edit moved texels in no-fire scenes on NVIDIA).
	for (label, src) in sources() {
		assert!(!src.contains("flit_quad") && !src.contains("flq_sgi") && !src.contains("two: bool"), "{label}");
		assert!(src.contains(super::CONE.0) && src.contains(super::CONE_RET.0), "{label}");
		validate(&label, &src, NO_SUB).unwrap();
	}
}

#[test]
fn the_fire_edits_apply_and_validate_without_subgroups() {
	for (label, src) in sources() {
		let (f, n, all) = super::fire_text(&src, false).unwrap_or_else(|| panic!("{label}: an anchor moved"));
		assert_eq!((n, all), (0, 0), "{label}");
		assert!(f.contains(super::CONE.1) && f.contains(super::TAP_SKIP) && f.contains(super::CONE_RET.1) && !f.contains("flit_quad"), "{label}");
		validate(&label, &f, NO_SUB).unwrap();
	}
}

/// The functions `f` calls, directly or not (by handle), into `out`.
fn calls(m: &naga::Module, f: &naga::Function, out: &mut std::collections::HashSet<naga::Handle<naga::Function>>) {
	fn block(m: &naga::Module, b: &naga::Block, out: &mut std::collections::HashSet<naga::Handle<naga::Function>>) {
		use naga::Statement as S;
		for s in b.iter() {
			match s {
				S::Call { function, .. } => {
					if out.insert(*function) {
						calls(m, &m.functions[*function], out);
					}
				}
				S::Block(b) => block(m, b, out),
				S::If { accept, reject, .. } => {
					block(m, accept, out);
					block(m, reject, out);
				}
				S::Switch { cases, .. } => cases.iter().for_each(|c| block(m, &c.body, out)),
				S::Loop { body, continuing, .. } => {
					block(m, body, out);
					block(m, continuing, out);
				}
				_ => {}
			}
		}
	}
	block(m, &f.body, out);
}

#[test]
fn every_twin_validates_with_subgroup_only_and_every_fire_lit_entry_takes_the_id() {
	for (label, src) in sources() {
		let (q, n, all) = super::fire_text(&src, true).unwrap_or_else(|| panic!("{label}: an anchor moved"));
		assert!(n >= 1 && n <= all, "{label}: {n} of {all}");
		assert!(validate(&label, &q, NO_SUB).is_err(), "{label}: the twin needs SUBGROUP");
		let (m, info) = validate(&label, &q, SUB).unwrap();
		let quad = m.functions.iter().find(|(_, f)| f.name.as_deref() == Some("flit_quad")).map(|(h, _)| h).expect("flit_quad");
		let mut lit = 0;
		for ep in m.entry_points.iter().filter(|e| e.stage == naga::ShaderStage::Fragment) {
			let mut reach = Default::default();
			calls(&m, &ep.function, &mut reach);
			if !reach.contains(&quad) {
				continue;
			}
			lit += 1;
			let id = ep.function.arguments.iter().any(|a| a.binding == Some(naga::Binding::BuiltIn(naga::BuiltIn::SubgroupInvocationId)));
			assert!(id, "{label}: {} reaches flit_quad without the subgroup id", ep.name);
		}
		assert!(lit >= 1, "{label}: no entry reaches flit_quad");
		// The fire set's constants are the defaults; the lookup-off overrides must also go through (review W4 finding 6).
		for pc in [&[][..], &[("FLS_ON", 0.0)][..]] {
			let pc: naga::back::PipelineConstants = pc.iter().map(|(k, v)| (k.to_string(), *v)).collect();
			naga::back::pipeline_constants::process_overrides(&m, &info, None, &pc).unwrap_or_else(|e| panic!("{label}: {e:?}"));
		}
	}
}

#[test]
fn the_probe_holds_the_twin_code_and_needs_subgroups() {
	let p = super::probe_source();
	assert!(p.contains(include_str!("flitquad.wgsl")));
	validate("probe", &p, SUB).unwrap();
	assert!(validate("probe", &p, NO_SUB).is_err());
}

#[test]
fn a_moved_anchor_gives_the_plain_lookup() {
	let src = crate::viewer::volfog::soft::level_source(crate::viewer::SHADER, true).replace(super::CONE_RET.0, "return acc / 4.0;");
	assert!(super::fire_text(&src, true).is_none());
}

#[cfg(windows)]
#[test]
fn the_twin_writes_hlsl_shader_model_6() {
	let src = crate::viewer::volfog::soft::level_source(crate::viewer::SHADER, true);
	let (m, info) = validate("level", &super::fire_text(&src, true).unwrap().0, SUB).unwrap();
	let (m, info) = naga::back::pipeline_constants::process_overrides(&m, &info, None, &Default::default()).expect("overrides");
	let opt = naga::back::hlsl::Options { shader_model: naga::back::hlsl::ShaderModel::V6_0, ..Default::default() };
	let mut out = String::new();
	naga::back::hlsl::Writer::new(&mut out, &opt, &Default::default()).write(&m, &info, None).expect("hlsl");
	assert!(out.contains("QuadReadLaneAt") && out.contains("WaveActiveBallot") && out.contains("WaveGetLaneIndex"));
}

/// Review W4 finding 7 (dev, by hand): RAC_FXC_DIR=DIR writes the SM 5.1 HLSL of the level fire module without subgroups (what FXC
/// compiles on DX12 without dxcompiler.dll) to DIR/level_fire_sm51.hlsl, for `fxc /T ps_5_1 /E <entry>`.
#[cfg(windows)]
#[test]
fn fxc_sweep_source() {
	let Some(dir) = std::env::var_os("RAC_FXC_DIR") else { return };
	let src = crate::viewer::metalfx::with_override(crate::viewer::volfog::soft::level_source(crate::viewer::SHADER, false));
	let (m, info) = validate("level fire", &super::fire_text(&src, false).unwrap().0, NO_SUB).unwrap();
	let (m, info) = naga::back::pipeline_constants::process_overrides(&m, &info, None, &Default::default()).expect("overrides");
	let opt = naga::back::hlsl::Options { shader_model: naga::back::hlsl::ShaderModel::V5_1, ..Default::default() };
	let mut out = String::new();
	let r = naga::back::hlsl::Writer::new(&mut out, &opt, &Default::default()).write(&m, &info, None).expect("hlsl");
	let names: Vec<String> = r.entry_point_names.into_iter().filter_map(|n| n.ok()).collect();
	std::fs::write(std::path::Path::new(&dir).join("level_fire_sm51.hlsl"), out).unwrap();
	std::fs::write(std::path::Path::new(&dir).join("entries.txt"), names.join("
")).unwrap();
}

use super::*;

fn picture(w: u32, h: u32, seed: u32) -> Vec<u8> {
	(0..w * h * 4).map(|i| (i.wrapping_mul(2654435761).wrapping_add(seed) >> 13) as u8).collect()
}

/// A CT32 picture gets a CT32 name (PSM 0, TW / TH of its size, no palette), a changed pixel changes it, and the name does not depend on
/// how often it is asked for.
#[test]
fn ct32_name_follows_the_pixels_and_the_size() {
	let px = picture(256, 128, 7);
	let n = ct32_name(&px, 256, 128).unwrap();
	assert_eq!((n.clut, n.bits, n.region_w, n.region_h), (0, 8 << 6 | 7 << 10, 0, 0));
	assert_eq!(ct32_name(&px, 256, 128), Some(n));
	let mut other = px.clone();
	other[4 * (100 * 256 + 200) + 1] ^= 1;
	assert_ne!(ct32_name(&other, 256, 128).unwrap().tex0, n.tex0);
	// 128x64 is another size: its own bits
	assert_eq!(ct32_name(&picture(128, 64, 1), 128, 64).unwrap().bits, 7 << 6 | 6 << 10);
}

#[test]
fn ct32_name_refuses_what_is_no_gs_upload() {
	assert!(ct32_name(&[0; 16], 2, 2).is_none(), "narrower than a page");
	assert!(ct32_name(&picture(64, 64, 0)[1..], 64, 64).is_none(), "short data");
	assert!(ct32_name(&picture(96, 64, 0), 96, 64).is_none(), "not a power of two");
}

/// Nothing to look up, nothing replaced: the helpers hand the texture back untouched.
#[test]
fn nothing_named_changes_nothing() {
	let t = Texture { w: 2, h: 1, rgba: vec![[1, 2, 3, 4], [5, 6, 7, 8]] };
	let mut v = [Texture { w: 2, h: 1, rgba: t.rgba.clone() }];
	assert_eq!(apply(&mut v, &[]), 0);
	assert_eq!(v[0].rgba, t.rgba);
	assert_eq!(replace(None, Texture { w: 2, h: 1, rgba: t.rgba.clone() }).rgba, t.rgba);
}

/// A picture asked for under two names (its PSMT8 and PSMT8H name, #66) takes the first one the packs have, once.
#[test]
fn an_index_with_two_names_takes_the_first_found() {
	let n = |k: u64| TexName::palette(k, 7, 0x1553);
	let file = |k: u64| PackFile { path: std::path::PathBuf::from(format!("{k}.png")), ext: super::super::name::Ext::Png, mod_idx: 0 };
	let has = |t: &TexName| [2u64, 3, 4].contains(&t.tex0).then(|| file(t.tex0));
	let got = first_found(&[(0, n(1)), (0, n(2)), (1, n(3)), (1, n(4)), (2, n(5))], has);
	assert_eq!(got, vec![(0, file(2)), (1, file(3))]);
}

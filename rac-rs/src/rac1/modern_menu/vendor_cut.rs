//! The modern vendor page hides the shop's six screens (v0.5 screens, lane v05c). The dark plates behind the modern page are not
//! menu quads: they are 3D geometry of the class-11 vendor moby (decomp/notes/vendor.md: "the class-11 model holds the six screen
//! frames"), drawn by the world pass, while the overlay (dropped for the Vendor screen) only holds the text. Each screen is an
//! assembly of its own: the metal frame, the black interior plate (8 faces of the body texture) and the hub parts behind it, all
//! skinned to that screen's block of six joints. Cutting only the frame left the black plates (and, with the plates gone, the hubs).
//! Pure: no statics. `frame_vertices` finds the screen vertices of a class, `collapse` moves them onto one point of a posed model so
//! every screen triangle is degenerate (nothing is drawn, nothing outside the model's bounds is touched); the host glue is
//! `host/handler/vendor_frames.rs` (called from `frame_upload` while `vendor_host::up()`). Only the pedestal stays.
//!
//! [C] measured on the disc (class 11 on levels 1, 3 and 11: the same mesh, 2327 vertices, 2128 faces) and in the exe (the black boxes
//! vanish with RAC_DRAWSKIP=props, the class-11 pose is the only vendor pose, the mesh has no flat or metal part): the joints come in
//! six blocks of six (0-5 ... 30-35, one per screen) and the pedestal's 37..44; every face has either all three vertices on a screen
//! joint (1664 faces: the frame texture 552, the body texture 1112) or all three on a pedestal joint (464 faces, the third texture),
//! none mixed, and no vertex is shared between the two sets. The faces on the corner joints alone (2-5 of each block) are the frame
//! (552) and 8 body faces per screen, the black plates; the other body faces (joints 1, 6, 7, ...) are the hubs behind the plates. The
//! texture id is the level's remapped id (`mobies.rs decode_class`), so the screens are found by joint, never by a texture number.
use crate::mobies::{MobyClass, Skin};
use crate::viewer::Posed;

/// The Gadgetron vendor's moby class (`game/vendor.rs` `VENDOR`).
pub const VENDOR_CLASS: u32 = 11;
/// Joints per screen: the vendor model's joints come in six blocks of six (0-5, 6-11, ... 30-35), one block per screen.
const BLOCK: u8 = 6;
const SCREENS: u8 = 6;

/// A joint of one of the six screens (0..36). The pedestal's joints (37..44) come after them.
pub fn is_screen_joint(j: u8) -> bool {
	j < BLOCK * SCREENS
}

/// The joint with the largest weight of a vertex (the first one on a tie).
pub fn dominant_joint(s: &Skin) -> u8 {
	let mut best = 0;
	for k in 1..3 {
		if s.weights[k] > s.weights[best] {
			best = k;
		}
	}
	s.joints[best]
}

/// The pure core of `frame_vertices`: `faces` as the mesh holds them (texture key, vertex indices), the vertices' `skin`, `n` vertices.
/// A face is a screen face when every one of its vertices has its dominant joint on a screen (whatever its texture: frame, black
/// plate and hub all qualify). The vertices of the screen faces are marked, except one a face that is not a screen face also uses
/// (moving that one would bend the pedestal; none on the real model).
pub fn frame_mask(faces: &[(u32, [u32; 3])], skin: &[Skin], n: usize) -> Vec<bool> {
	let on_screens = |f: &[u32; 3]| f.iter().all(|&v| skin.get(v as usize).is_some_and(|s| is_screen_joint(dominant_joint(s))));
	let mut out = vec![false; n];
	for (_, f) in faces.iter().filter(|(_, f)| on_screens(f)) {
		for &v in f {
			if let Some(m) = out.get_mut(v as usize) {
				*m = true;
			}
		}
	}
	for (_, f) in faces.iter().filter(|(_, f)| !on_screens(f)) {
		for &v in f {
			if let Some(m) = out.get_mut(v as usize) {
				*m = false;
			}
		}
	}
	out
}

/// The screen vertices of `class` (parallel to `mesh.positions`; metal packets, which follow the mesh in a posed model, are never screens).
pub fn frame_vertices(class: &MobyClass) -> Vec<bool> {
	frame_mask(&class.mesh.faces, &class.skin, class.mesh.positions.len())
}

/// `pose` with every masked vertex moved onto the first unmasked vertex's position (the screen triangles become points: nothing is
/// drawn, and the model's bounds do not grow). None = nothing to do: no masked vertex, no unmasked one to collapse onto, or a pose
/// shorter than the mask (not this class's pose). The normals are kept.
pub fn collapse(pose: &Posed, mask: &[bool]) -> Option<Posed> {
	let anchor = mask.iter().position(|m| !*m)?;
	if !mask.iter().any(|m| *m) || pose.0.len() < mask.len() || pose.1.len() < mask.len() {
		return None;
	}
	let at = pose.0[anchor];
	let mut ps = pose.0.clone();
	for (p, m) in ps.iter_mut().zip(mask) {
		if *m {
			*p = at;
		}
	}
	Some((ps, pose.1.clone()))
}

#[cfg(test)]
mod tests;

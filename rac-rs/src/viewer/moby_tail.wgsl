// Colour alpha below -1.5 is a baked colour: world geometry (terrain, ties, shrubs: light w -1..-5) or a moby's tinted tail packet
// (the tail colour of the moby, `vs_props_pose`: light w >= 0, the moby's light word). A tail packet has the GS state of the lit
// packets (ALPHA_1 0x44, TEST_1 ATE GEQUAL AREF 0x60 AFAIL RGB_ONLY): its texels below alpha 0x80 blend (and from alpha 0x60 write Z)
// in the moby pass like theirs. Qwark's TV screen is such a texel (alpha 0x24, no Z); drawn opaque it hid Qwark behind it.
fn moby_tail(i: VOut) -> bool {
	return i.c.a < -1.5 && i.l.w > -0.5;
}


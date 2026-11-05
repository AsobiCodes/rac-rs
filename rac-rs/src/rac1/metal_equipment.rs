//! Lower added metal sheen for Clank, head/foot equipment and held weapons.
//! Class identities: hero item / held updater tables (weapons_framework.md section 1),
//! Clank/backpack and boot attachments (retail/strafe.rs). Texture identities are the
//! selected metal images referenced by those classes across all PAL level remaps.

pub const CLASSES: &[u32] = &[10, 71, 87, 157, 163, 168, 173, 175, 176, 177, 180, 185, 188, 190, 192, 208, 229, 433, 454, 483, 562, 585, 601, 607, 608, 609, 618, 849, 1251, 1289, 1290];

pub const TEXTURES: &[u64] = &[
	0x0a7443f0f4ddfdd4, 0x191f01eb0383b7df, 0x27da47574d2f7067, 0x27f237a9f7a61f03,
	0x2a0eacec36d12d1e, 0x334afdd99bfff81d, 0x360fc451b08e95d6, 0x3a75873cefe0306a,
	0x46690799f74a8c16, 0x5f2b350f0d532290, 0x7214eb26814c3731, 0x7a00021e2c085007,
	0x7cfcb9dcbb535dc1, 0x7fcbec9ed1fe8790, 0x8487b7ad5060effa, 0x8c648c70e5fbbad4,
	0x9245e2b616723b92, 0xa44c7d11a0a37828, 0xc2a94a1de9a13492, 0xcdd22b07c31d668d,
	0xd5f65cd1f70e3864, 0xd97736b7c3dc063c, 0xe83b012d9288fb34, 0xeb4b113ef26976c8,
	0xfa227899a42da9e5, 0xfa6505c60e4ee651, 0xfc89d6bb186ea5fc,
 ];

#[cfg(test)]
mod tests {
    #[test]
    fn equipment_is_a_strict_subset_of_selected_metal() {
        let ids = super::TEXTURES;
        assert!(ids.windows(2).all(|w| w[0] < w[1]));
        assert!(super::CLASSES.windows(2).all(|w| w[0] < w[1]));
        assert!(ids.iter().all(|id| crate::metal_materials::METAL_TEXTURES.binary_search_by_key(id, |r| r.0).is_ok()));
        assert!(ids.binary_search(&0xfa227899a42da9e5).is_ok(), "wrench uses the softer response");
        assert!(ids.binary_search(&0xd049c79ef9c61bef).is_err(), "cloth stays unmarked");
    }
}

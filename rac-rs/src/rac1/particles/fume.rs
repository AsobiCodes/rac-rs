//! The fume effect: `FumeParams` (acceleration, size and colour ramps) and `Fume`, a drifting sprite particle built from them.
//! Leftover from the retired hand port; still compiled because retail mode and the renderer still use parts of `particles` (`Rng`, sprite and quad types). Do not extend.
use super::*;
use std::rc::Rc;
#[derive(Clone,Debug)]
pub struct FumeParams {
 pub accel:[f32;3],pub min:[f32;3],pub max:[f32;3],pub growth:f32,pub spin_rate:f32,pub alpha_rate:f32,
 pub restitution:f32,pub cap:f32,pub alpha_min:f32,pub alpha_max:f32,pub rgb_rate:[f32;3],pub rgb_min:[f32;3],pub rgb_max:[f32;3],
 pub flags:u32,pub size_min:f32,pub size_max:f32,pub spread:[f32;3],pub spin_range:[f32;2],pub size_range:[f32;2],
 pub rgb:[f32;3],pub alpha:f32,pub life:i16,pub count:u8,pub reload:u8,pub blend:u8,pub def:u8,pub offsets:[f32;3],
}
#[derive(Clone,Debug)]
pub struct Fume {
 pub pos:[f32;3],pub vel:[f32;3],pub p:Rc<FumeParams>,pub size:f32,pub spin:f32,pub alpha:f32,
 pub rgb_hi:[u8;3],pub rgb_lo:[u8;3],pub signs:u8,pub rot_byte:u8,pub life:i16,pub alpha_byte:u8,
}
impl Fume {
 pub(super) fn sprite(&self)->Sprite {
  Sprite {pos:self.pos,size:self.size*SIZE_SCALE,rot:self.rot_byte as f32*TAU/256.0,
   colour:[(self.rgb_hi[0]as f32/FULL).min(2.0),(self.rgb_hi[1]as f32/FULL).min(2.0),(self.rgb_hi[2]as f32/FULL).min(2.0),(self.alpha_byte as f32/FULL).clamp(0.0,1.0)],
   tex:TEX_SMOKE4,additive:self.p.blend&1!=0,pool:Some((self.p.blend>>5)*16+4), flat: false}
 }
}

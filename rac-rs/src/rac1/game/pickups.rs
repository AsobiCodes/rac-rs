//! Pickups of the old hand-port `Game`: `glint_cull` (frees glints outside the view volume, FUN_00220518) and `bolt_wrap`
//! (angle wrap); the rest is gone.
//! Leftover from the retired hand port; still compiled because retail mode uses part of `game::Game` (the prop pools). Do not extend.
use super::*;

impl Game {







}




impl Game {
 /// The view-volume free of FUN_00220518 (glint_pool_draw), run once per tick with that tick's camera (FRAME-1).
 pub(crate) fn glint_cull(&mut self,eye:V3,right:V3,up:V3,fwd:V3) {
  for g in &mut self.glints {if g.life>0 && crate::particles::glint_off_view(g.pos,eye,right,up,fwd) {*g=Glint::default();}}
 }
}

pub(super) fn bolt_wrap(mut a:f32)->f32 {let p=std::f32::consts::PI;let t=std::f32::consts::TAU;while a>=p {a-=t;}while a< -p {a+=t;}a}




use super::*;

pub const EMITTER:u32=27;
pub const DROPSHIP:u32=666;
pub const FIGHTER: u32 = 660;
pub const BOMBER: u32 = 688;
/// Bomber rocket 0x2ae (FUN_002f61d0).
pub const SHOT: u32 = 686;
/// KER-3: Kerwan's model-less flyer 0x34d (moby 925), whose class-table update is jet_common_flight 0x2935a8 (twin of
/// novalis 0x2f4a98); class 822 (KER-80a/80b) takes it over (decomp/notes/kerwan_environment.md KER-3).
pub const KER_JET: u32 = 845;
pub fn is_jet(c: u32) -> bool { matches!(c, FIGHTER | BOMBER | DROPSHIP | KER_JET) }

impl Game {
 pub fn set_jet_joint(&mut self, map:HashMap<u32,V3>) { self.jet_joint=map; }
 /// BRIDGE-9 world bounding sphere of actor i (retail actor +0x00, 1/1024 m units).
 pub(super) fn actor_bsphere(&self,i:usize)->Option<(V3,f32)> {
  let p=&self.props[i];
  let (seqs,cs)=self.seq_spheres.get(&p.class_id)?;
  let sp=seqs.get(p.anim.seq).or(seqs.first())?;
  let g=cs/1024.0*p.scale;
  let m=moby_axes(p.rot);
  let k:V3=std::array::from_fn(|a|(p.pos[a]+m[0][a]*sp[0]*g+m[1][a]*sp[1]*g+m[2][a]*sp[2]*g)*1024.0);
  Some((k,sp[3]*g*1024.0))
 }
 /// Per prop the world point the moby draw-distance fade measures depth to (1/1024 m): the BRIDGE-9 sphere centre, else the origin.
 pub fn fade_centres(&self)->Vec<V3> {
  (0..self.props.len()).map(|i|self.actor_bsphere(i).map(|s|s.0).unwrap_or_else(||self.props[i].pos.map(|c|c*1024.0))).collect()
 }
 /// Per prop the radius of that sphere (1/1024 m), None without one (REN-MOBY-DRAWN near / side tests).
 pub fn fade_radii(&self)->Vec<Option<f32>> {
  (0..self.props.len()).map(|i|self.actor_bsphere(i).map(|s|s.1)).collect()
 }
 pub fn set_seq_spheres(&mut self,m:HashMap<u32,(Vec<[f32;4]>,f32)>) { self.seq_spheres=m; }
}

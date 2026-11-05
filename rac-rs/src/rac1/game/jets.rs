//! Jet class ids (emitter, dropship, fighter, bomber, shot, Kerwan flyer), `is_jet`, and the old `Game` jet helpers (joint map,
//! actor bounding sphere, draw-distance fade point).
//! Leftover from the retired hand port; still compiled because retail mode uses part of `game::Game` (the prop pools). Do not extend.
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

/// The last class lookup of `seq_spheres` and the last rotation with its `moby_axes` (`Game::bsphere_memo`).
type SphereMemo<'a>=(Option<(u32,Option<&'a (Vec<[f32;4]>,f32)>)>,Option<([u32;3],[V3;3])>);

impl Game {
 pub fn set_jet_joint(&mut self, map:HashMap<u32,V3>) { self.jet_joint=map; }
 /// BRIDGE-9 world bounding sphere of actor i (retail actor +0x00, 1/1024 m units), with the last class lookup and the last rotation's axes kept in `memo` (pool slots of one class are
 /// contiguous and idle ones share a rotation): [PC] perf, the same inputs give the same sphere.
 fn bsphere_memo<'a>(&'a self,i:usize,memo:&mut SphereMemo<'a>)->Option<(V3,f32)> {
  let p=&self.props[i];
  let ent=match memo.0 { Some((c,e)) if c==p.class_id=>e, _=>{ let e=self.seq_spheres.get(&p.class_id); memo.0=Some((p.class_id,e)); e } };
  let (seqs,cs)=ent?;
  let sp=seqs.get(p.anim.seq).or(seqs.first())?;
  let g=cs/1024.0*p.scale;
  let key=p.rot.map(f32::to_bits);
  let m=match memo.1 { Some((k,m)) if k==key=>m, _=>{ let m=moby_axes(p.rot); memo.1=Some((key,m)); m } };
  let k:V3=std::array::from_fn(|a|(p.pos[a]+m[0][a]*sp[0]*g+m[1][a]*sp[1]*g+m[2][a]*sp[2]*g)*1024.0);
  Some((k,sp[3]*g*1024.0))
 }
 /// Per prop the world point the moby draw-distance fade measures depth to (1/1024 m): the BRIDGE-9 sphere centre, else the origin.
 /// And per prop the radius of that sphere (1/1024 m), None without one (REN-MOBY-DRAWN near / side tests): (centres, radii).
 /// [PC] perf: one pass, each actor's sphere computed once (the two lists used to be built by two passes of the same sphere).
 pub fn fade_spheres(&self)->(Vec<V3>,Vec<Option<f32>>) {
  let n=self.props.len();
  let (mut c,mut r)=(Vec::with_capacity(n),Vec::with_capacity(n));
  let mut memo:SphereMemo=(None,None);
  for i in 0..n {
   let s=self.bsphere_memo(i,&mut memo);
   c.push(s.map(|s|s.0).unwrap_or_else(||self.props[i].pos.map(|c|c*1024.0)));
   r.push(s.map(|s|s.1));
  }
  (c,r)
 }
 pub fn set_seq_spheres(&mut self,m:HashMap<u32,(Vec<[f32;4]>,f32)>) { self.seq_spheres=m; }
}

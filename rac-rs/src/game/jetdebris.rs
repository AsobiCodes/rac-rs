use super::*;



/// FUN_0030b8d8 (NOV-5.jets-D-draw): the camera-facing billboard of one debris piece at post-move `pos` / `rot`.
pub fn jet_debris_quad(eye:V3,pos:V3,rot:V3)->crate::particles::WorldQuad {
 let unit=|v:V3|{let l=length(v);if l==0.0{[0.0;3]}else{scale(v,1.0/l)}};
 let m=crate::particles::euler_m(rot);
 let p=add(add(scale(m[0],-0.258),scale(m[2],0.448)),pos);
 let r0=unit(sub(eye,p));let r1=unit(crate::physics::cross(r0,[0.0,0.0,-1.0]));let r2=crate::physics::cross(r0,r1);
 let corner=|y:f32,z:f32|add(add(scale(r1,0.4*y),scale(r2,0.4*z)),p);
 let v=[corner(-1.0,1.0),corner(-1.0,-1.0),corner(1.0,1.0),corner(1.0,-1.0)];
 crate::particles::WorldQuad{pos:[v[0],v[1],v[3],v[2]],uv:[[0.0,0.0],[0.0,1.0],[1.0,1.0],[1.0,0.0]],colour:[[1.9921875,0.25,0.25,0.25];4],tex:11,additive:true}
}


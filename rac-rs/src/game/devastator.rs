//! Devastator (item 11): the held class 157 updater (WPN-6.2a) with its smoke and muzzle bursts, the class 153
//! rocket pool, creator and registry (WPN-6.2b), the rocket flight (WPN-6.2c), its trail (WPN-6.2d) and the impact:
//! explosion, gold bounce and retarget (WPN-6.2d-x).
//!
//! The target component FUN_00270d90 (lift, radius, no-predict and speed bytes) needs no Rust carrier: every placed
//! registry member authors lift 0.5 and bytes +0xa/+0xb/+0xc = 0 and nothing writes them at run time, so the retail
//! component branches give exactly the "no component" results (WPN-6.2f1 (b) [C]). The impact geometry follows WPN-6.2f2
//! (BRIDGE-55): a face hit takes dir = v0 - v1 of the hit face (seg_collide 0x174290 - 0x1742a0), an actor hit the
//! primitive normal P - centre of the winning shape (`Game.last_hit_normal`); an OBB victim keeps -unit(vel) [I].



/// The rocket class.
pub const ROCKET: u32 = 153;
/// The held class.
pub const HELD: u32 = 157;

































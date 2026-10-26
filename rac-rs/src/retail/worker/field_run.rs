//! One field of the worker: the pad and the hooks before it (travel request, ship/turret mouse, PC options, replay), then the
//! EE runs the field.

use super::*;

impl Worker {
	pub(super) fn field_begin(&mut self, fin: FieldIn) -> std::time::Instant {
		let shipmouse_state = &mut self.shipmouse_state;
		let wperf = &mut self.wperf;
		let mut m = &mut *self.m;
		let play = &mut self.play;
		let booting = self.booting;
		let mut field = self.field;
		let front = self.front;
		let ll_last = self.ll_last;
		let mut ll_idle = self.ll_idle;
		fmv_bridge::hold_field();
		let t = std::time::Instant::now();
		ll_idle += t.duration_since(ll_last).as_nanos() as u64;
		wperf.lap(0);
		if let Some(to) = fin.travel.filter(|_| !disc::extras_off()) {
			m.cpu.mem.wr32(va(0x15f5a0), to);
			m.cpu.mem.wr32(va(0x15f5b8), 1);
			m.cpu.mem.wr32(va(0x15f550), 1);
		}
		// Ship fights: the mouse counts of the field become a virtual stick on top of the keyboard / pad sticks (shipmouse.rs);
		// the recording keeps the raw sticks + counts, so a replay re-derives the same bytes.
		let ship = !front && !booting && !disc::extras_off() && shipmouse::in_ship_fight(&m);
		// Turret sections (Batalia, Gaspar, Hoven): the same counts steer the turret through its stick (shipmouse.rs).
		let turret = if front || booting || disc::extras_off() { None } else { shipmouse::turret(&m, shipmouse::y_inverted(&m)) };
		let pad_sticks = shipmouse_state.step(ship, ship && shipmouse::y_inverted(&m), fin.mc.look, fin.sticks);
		let pad_sticks = shipmouse_state.step_turret(turret, fin.mc.look, pad_sticks);
		m.set_pad(fin.buttons, pad_sticks);
		// Button prompts: the device of this field (prompts.rs).
		prompts::begin_field(field, &mut m.cpu.mem);
		if !front && !booting && !disc::extras_off() {
			let mut fin2 = pcext::pre_field(&mut m, &fin);
			// The mouse steers the ship or the turret, not the modern camera, in a ship fight / turret section.
			if ship || turret.is_some() {
				fin2.mc.look = [0, 0];
			}
			modcam::pre_field(&mut m, &fin2);
			strafe::pre_field(&mut m, &fin2);
			aim::pre_field(&mut m, &fin2);
		} else if front {
			// The title's Options -> Graphics rows (pcext.rs `front_field`).
			pcext::front_field(&mut m);
			crate::retail::pcmods::front_field(&mut m.cpu.mem);
		}
		// Dev debug keys (retail/debugkeys.rs): inert unless RAC_DEBUG_KEYS=1; their writes happen here, between two fields.
		debugkeys::pre_field(&mut m, field, !front && !booting);
		if let Some(p) = &play {
			p.apply(&mut m.cpu, field);
		}
		// The mods (omni/input.rs) read this field's keyboard / mouse snapshot, in `omni::pad` and `omni::tick` below and in their hooks.
		crate::omni::input::set_field(&fin.omni);
		rec::field(field, &fin);
		if rec::trail_due(field) {
			rec::trail(field, &m.cpu.mem.ram[..]);
		}
		field += 1;
		if field % UNWIND_FIELDS == 0 {
			// The game thread is parked at the vblank wait: the flag unwinds its compiled frames there.
			m.cpu.rec_unwind = true;
		}
		wperf.lap(1);
		self.field = field;
		self.ll_idle = ll_idle;
		t
	}

	pub(super) fn field_ee(&mut self) -> (Result<(), String>, u64) {
		let wperf = &mut self.wperf;
		let m = &mut *self.m;
		let vush = &mut self.vush;
		let booting = self.booting;
		let front = self.front;
		let t_frame = std::time::Instant::now();
		let teed = !front && !booting && efx_draw::enabled();
		if teed {
			vush.begin(&mut m.cpu.mem.gsq);
		}
		let ic0 = m.cpu.icount;
		crate::omni::pad(&mut m.cpu);
		let r = m.frame();
		crate::omni::tick(&mut m.cpu);
		disc::rescope(&m.cpu.mem.ram);
		wperf.icd = m.cpu.icount.wrapping_sub(ic0);
		vush.end(&mut m.cpu.mem.gsq);
		let frame_ns = t_frame.elapsed().as_nanos() as u64;
		// RAC_TEST_SLOW_EE=F (tests): the worker idles (F - 1) times its field after it, as a CPU F times slower would be busy.
		if let Some(f) = test_slow_ee() {
			std::thread::sleep(std::time::Duration::from_nanos((frame_ns as f64 * (f - 1.0)) as u64));
		}
		wperf.lap(2);
		(r, frame_ns)
	}
}

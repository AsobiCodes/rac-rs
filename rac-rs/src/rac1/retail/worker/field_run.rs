//! One field of the worker: the pad and the hooks before it (travel request, ship/turret mouse, PC options, replay), then the
//! EE runs the field.

use super::*;

impl Worker {
	pub(super) fn field_begin(&mut self, mut fin: FieldIn) -> std::time::Instant {
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
		let turret = if front || booting || disc::extras_off() { None } else { shipmouse::turret(&m, shipmouse::game_y_inverted(&m)) };
		shipmouse_state.set_turret_flip(shipmouse::y_inverted(&m) != shipmouse::game_y_inverted(&m));
		let pad_sticks = shipmouse_state.step(ship, ship && shipmouse::y_inverted(&m), fin.mc.look, fin.sticks);
		let mut pad_sticks = shipmouse_state.step_turret(turret, fin.mc.look, pad_sticks);
		// Modern camera: the controller's right stick gets the mouse's sign for every camera, modcam's or retail's (modcam/sticks.rs);
		// the recording keeps the raw sticks, so a replay re-derives the same bytes. Not with the Classic camera, in the front end,
		// the title boot, a ship fight or a turret section.
		let cam_stick = !front && !booting && !disc::extras_off() && !ship && turret.is_none();
		if cam_stick {
			pad_sticks = modcam::sticks::camera_sticks(&m, pad_sticks, fin.mc.keys);
		}
		// The mouse's instant selection in the page menus (mouse_instant.rs): the cursor words it writes and the pad bits it adds
		// are in this field, before the pad is set (and recorded). Without a mouse request: the pad as it came.
		if !booting && !disc::extras_off() {
			fin.buttons = mouse_instant::field(&mut m, front, fin.buttons);
			// The Achievements row of the pause Options list (pcext/achrow.rs): a Cross on it (the instant click's own, a key, the pad) is
			// decided on RAM as it is now and taken out of the pad; the main thread opens the list. Before the pad is set and recorded.
			fin.buttons = pcext::achrow::worker_field(&m, front, fin.buttons);
			// Modern Options pages (modern_swallow.rs): Cross on the Graphics / Camera row opens the engine's page, the game never sees it.
			fin.buttons = modern_swallow::filter(&m, front, fin.buttons);
			// Quit to Desktop (pcext/quitrow.rs): Cross on its Options row arms the flag; Circle on the Quit Game page it opened asks the program to end and the game never sees it.
			fin.buttons = pcext::quitrow::filter(&m, front, fin.buttons);
			// Auto-equip gadgets (auto_gadget.rs, row On): Circle where a gadget is needed asks for it (request word) and is held
			// back until it is in hand; the weapon comes back after. Off: the pad as it came.
			fin.buttons = crate::retail::auto_gadget::filter(&mut m, front, field, fin.buttons);
		}
		m.set_pad(fin.buttons, pad_sticks);
		// Button prompts: the device of this field (prompts.rs).
		prompts::begin_field(field, &mut m.cpu.mem);
		if !front && !booting && !disc::extras_off() {
			// Quick Select pauses the game (qs_pause.rs, row On): the world steps are off for this field while the ring is up.
			let frozen = qs_pause::pre_field(&mut m, field);
			// Fast travel (fast_travel.rs, row On): the arrival warp stays black and ends once the level's reads are done.
			crate::retail::fast_travel::pre_field(&mut m, field, play.is_some());
			let mut fin2 = pcext::pre_field(&mut m, &fin);
			if cam_stick {
				// modcam turns on the same right stick bytes the game reads.
				fin2.sticks[..2].copy_from_slice(&pad_sticks[..2]);
			}
			// The mouse steers the ship or the turret, not the modern camera, in a ship fight / turret section; no turn in a frozen field.
			if ship || turret.is_some() || frozen {
				fin2.mc.look = [0, 0];
			}
			modcam::pre_field(&mut m, &fin2);
			// First person (retail's free-look camera): the same mouse counts turn its view (modcam/fp_look.rs).
			modcam::fp_look::pre_field(&mut m, &fin2);
			strafe::pre_field(&mut m, &fin2);
			aim::pre_field(&mut m, &fin2);
		} else if front {
			// The title's Options -> Graphics rows (pcext.rs `front_field`).
			pcext::front_field(&mut m);
			crate::retail::pcmods::front_field(&mut m.cpu.mem);
			// Fast travel after the title's Load Game (fast_load.rs, row On): the load's warp stays black and ends once the reads are done.
			if !booting && !disc::extras_off() {
				crate::retail::fast_load::front_field(&mut m, field, play.is_some());
			}
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
		// Every UNWIND_FIELDS fields the flag unwinds the game thread's compiled frames, only once it is parked in a vsync
		// wait: a long frame hands over the field mid-computation, maybe inside a PC hook's call (worker/unwind.rs).
		let parked = unwind::parked(m.cpu.pc, m.cpu.vblank_pc, ee_oracle::ver::addrs().poll_head);
		let (due, now) = unwind::step(self.unwind_due, field, parked);
		self.unwind_due = due;
		if now {
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

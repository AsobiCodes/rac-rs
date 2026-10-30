//! The winit event handler of the viewer (`impl ApplicationHandler for App`). The per-frame code of
//! `RedrawRequested` is cut into steps (`impl App` methods), one child file per job:
//! - cursor_vis.rs: when the OS cursor is shown (mouse in use) or hidden (pad / keyboard / captured)
//! - display.rs: test window, display mode
//! - frame_audio.rs: plays the frame's collected sounds, animation cues, music, ambience
//! - frame_hero.rs: hero pose, scene actors, hero effects
//! - frame_hud.rs: HUD state, HUD and pause menu draw
//! - frame_post.rs: shadows, sound events collected from the field's ticks (`frame_sound_events`), camera
//! - frame_props.rs: prop poses
//! - fx_skin.rs: the skinning of the joint-record props kept between frames
//! - frame_render.rs: final draw, capture, present
//! - frame_start.rs: Frame (locals shared by the steps), frame start, input
//! - frame_switch.rs: level switch, travel, messages, free camera keys
//! - frame_ticks.rs: the game ticks, per-tick render state
//! - frame_upload.rs: upload phase, scene camera put back, water hold
//! - frame_upload_props.rs: the props' CPU-only draw inputs of the upload phase (tints, class UV, fade spheres)
//! - hero_rows.rs: hero rows (tilt), joint override records
//! - play_input.rs: mouse button / key state changes
//! - ship_shadows.rs: ship pad / flight shadow quads
//! - sun_gate.rs: which camera-culled mobys the sun shadow pass can take (props phase)
//! - sound_loops.rs: sound stage, enemy cues, loop voices
use super::*;

mod cursor_vis;
mod display;
mod edge_fill;
pub(in crate::viewer) mod frame_clock;
mod frame_audio;
pub(in crate::viewer) mod fx_skin;
pub(in crate::viewer) mod pose_born;
mod frame_hero;
mod frame_hud;
mod frame_post;
mod frame_props;
mod frame_render;
mod frame_split;
mod frame_start;
mod frame_switch;
mod frame_ticks;
mod frame_upload;
mod frame_upload_props;
mod hero_rows;
pub(in crate::viewer) mod hud_hold;
pub(in crate::viewer) mod hud_place;
mod play_input;
mod pose_service;
mod ship_shadows;
mod sun_gate;
mod sound_loops;
use self::display::*;
use self::frame_ticks::*;
use self::frame_upload::*;
use self::hero_rows::*;
use self::ship_shadows::*;
use self::sun_gate::{SunGate, HISTORY_MARGIN};
use self::sound_loops::*;
use self::frame_start::*;

/// A retail movie ended while retail presents no picture yet (`Retail::holds`): its last picture stays on screen.
static MOVIE_HOLD: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

use super::herocache::{hero_cache_key, hero_check_compare, hero_check_on, hero_check_frame, next_gen, HeroCache};

/// Most frames the world stays black after a level switch (`App::arrival_black`).
const ARRIVAL_BLACK_MAX: u32 = 30;

impl ApplicationHandler for App {
	fn resumed(&mut self, el: &ActiveEventLoop) {
		if self.gpu.is_some() {
			return;
		}
		let mut attrs = Window::default_attributes().with_title(crate::app_name()).with_inner_size(winit::dpi::LogicalSize::new(1280, 720)).with_visible(!self.headless);
		// Window and taskbar icon: the logo, 64x64 raw RGBA embedded in the exe (assets/icon/rac-rs-64.rgba).
		if let Ok(icon) = winit::window::Icon::from_rgba(include_bytes!("../../assets/icon/rac-rs-64.rgba").to_vec(), 64, 64) {
			attrs = attrs.with_window_icon(Some(icon));
		}
		// Agent test runs (RAC_TEST_WINDOW=hidden, or the smoke check's RAC_RETAIL_SMOKE): the window is created off-screen, not activated and not in the taskbar, so
		// it never shows on the user's desktop while the swapchain keeps presenting. Without those variables the window is the normal one.
		if test_window() {
			attrs = attrs.with_active(false).with_position(winit::dpi::PhysicalPosition::new(-32000, -32000));
			// RAC_TEST_SIZE=WIDTHxHEIGHT (pixels): the test window's size (proof shots at other resolutions than 1280x720).
			if let Some((w, h)) = std::env::var("RAC_TEST_SIZE").ok().and_then(|v| v.split_once('x').and_then(|(a, b)| Some((a.trim().parse::<u32>().ok()?, b.trim().parse::<u32>().ok()?)))) {
				attrs = attrs.with_inner_size(winit::dpi::PhysicalSize::new(w, h));
			}
			#[cfg(windows)]
			{
				use winit::platform::windows::WindowAttributesExtWindows;
				attrs = attrs.with_skip_taskbar(true);
			}
		}
		let window = Arc::new(el.create_window(attrs).expect("window"));
		crate::runlog::set_window(&window);
		crate::awake::focus(window.has_focus());
		crate::awake::minimise(window.inner_size().width == 0 || window.inner_size().height == 0);
		crate::loadlog::say(|| "host: window created".into());
		crate::retail::gpu_capture_expect_window(); // the GS capture waits for this window's device (it shares it)
		self.gpu = Some(Gpu::new(window));
		super::gpu::renderer::start_done(); // [PC] the DX12 FXC start-up wait is over (renderer.rs)
		self.audio = if self.headless { None } else { audio::Audio::new() };
		if self.audio.is_none() {
			eprintln!("no audio output; running silent");
		}
		crate::loadlog::say(|| "host: audio ready".into());
		self.load_level();
		// The front end starts play itself (New Game / Load Game).
		if self.autoplay && self.front.is_none() {
			self.toggle_play();
		}
		self.last = Instant::now();
	}

	fn device_event(&mut self, _: &ActiveEventLoop, _: DeviceId, event: DeviceEvent) {
		if let DeviceEvent::MouseMotion { delta } = event {
			crate::retail::prompts::note_mouse_motion(delta.0, delta.1);
			cursor_vis::raw_motion(delta.0, delta.1);
			// OmniLoader mods (omni/input.rs): the raw motion goes to them; a mod that took the mouse or captured the cursor keeps it from the camera.
			crate::omni::input::note_mouse_motion(delta.0, delta.1);
			if self.retail.is_some() && crate::omni::input::mouse_owned() {
				return;
			}
			// [PC] DEV ONLY (RAC_DEV_TOOLS=1, retail/devtools.rs): the free camera takes the mouse while the window has the focus.
			if self.focused && self.retail.is_some() && crate::retail::devtools::ui_mouse(delta.0 as f32, delta.1 as f32) {
				return;
			}
			// Quick Select wheel by mouse (RAC_WHEEL_MOUSE=1, retail/wheel_mouse.rs): after the mod and dev-camera owners above.
			if self.retail.is_some() {
				crate::retail::wheel_mouse::motion(delta.0, delta.1);
			}
			if self.grabbed {
				if self.retail.is_some() {
					crate::retail::modcam::ui_mouse(delta.0 as f32, delta.1 as f32);
				}
				self.mouse_dx += delta.0 as f32;
				self.mouse_dy += delta.1 as f32;
			} else if self.looking {
				self.cam.yaw -= delta.0 as f32 * 0.003;
				self.mouse_dx += delta.0 as f32;
				self.cam.pitch = (self.cam.pitch - delta.1 as f32 * 0.003).clamp(-1.55, 1.55);
			}
		}
	}

	fn window_event(&mut self, el: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
		// Button prompts follow the device: a key or mouse button press, the wheel or clear mouse movement make the keyboard the device (retail/prompts.rs).
		crate::retail::prompts::note_window_event(&event);
		// The OS cursor shows only while the mouse is the device in use (cursor_vis.rs).
		cursor_vis::window_event(&event, self.grabbed);
		// OmniLoader mods (omni/input.rs): keys, buttons, wheel, cursor and focus; the game does not see the mouse presses of a mod that owns the mouse.
		crate::omni::input::note_window_event(&event, self.gpu.as_ref().map_or((1, 1), |g| g.win_size()));
		if self.retail.is_some() && crate::omni::input::swallows(&event) {
			return;
		}
		match event {
			WindowEvent::CloseRequested => {
				crate::gscap_guard::quit();
				el.exit()
			}
			WindowEvent::Resized(s) => {
				if let Some(g) = &mut self.gpu {
					g.resize(s.width, s.height);
				}
				crate::awake::minimise(s.width == 0 || s.height == 0);
			}
			WindowEvent::Focused(f) => {
				self.focused = f;
				// Gamepad input does not count as activity for Windows: the display stays awake while this window has the focus (awake.rs).
				crate::awake::focus(f);
				// Retail play: a key released while another window had the focus never reports its release here, so it
				// would stay held in every recorded field. Losing the focus lets go of every key and mouse button.
				if !f && crate::retail::active() {
					self.lose_focus(); // play_input.rs: every key and mouse button let go, a Sound Options bar drag ends
				}
			}
			WindowEvent::CursorMoved { position, .. } => {
				self.cursor = Some((position.x as f32, position.y as f32));
				// Retail page menus (retail/mouse_menu.rs): the cursor as fractions of the client area.
				// Aims only while the mouse is in use (cursor shown); a hidden-cursor desk bump just keeps the place for a click.
				if let (Some(r), Some(g)) = (self.retail.as_mut(), self.gpu.as_ref()) {
					let (fx, fy) = (position.x as f32 / g.win_size().0.max(1) as f32, position.y as f32 / g.win_size().1.max(1) as f32); // window pixels (render scale: config is the render size)
					if cursor_vis::mouse_in_use() { r.mouse_cursor(fx, fy) } else { r.mouse_track(fx, fy) }
				}
			}
			// Retail page menus: the wheel steps the list (retail/mouse_menu.rs).
			WindowEvent::MouseWheel { delta, .. } if self.retail.as_ref().is_some_and(|r| r.mouse_menu_active()) => {
				use winit::event::MouseScrollDelta;
				let y = match delta {
					MouseScrollDelta::LineDelta(_, y) => y,
					MouseScrollDelta::PixelDelta(p) => p.y as f32,
				};
				if let Some(r) = self.retail.as_mut() {
					r.mouse_wheel(y);
				}
			}
			WindowEvent::MouseWheel { delta, .. } => {
				use winit::event::MouseScrollDelta;
				let lines = match delta {
					MouseScrollDelta::LineDelta(_, y) => y,
					MouseScrollDelta::PixelDelta(p) => p.y as f32 / 80.0,
				};
				// [PC] DEV ONLY: the free camera's speed (retail/devtools.rs).
				if self.retail.is_some() && crate::retail::devtools::ui_wheel(lines) {
					return;
				}
				self.zoom_acc -= lines * 0.75;
				if self.retail.is_some() && self.grabbed {
					crate::retail::modcam::ui_zoom(lines);
				}
			}
			// Retail page menus: right click = the screen's back button, left click = select + Cross (the fire button is off).
			WindowEvent::MouseInput { state: ElementState::Pressed, button: MouseButton::Right, .. } if self.retail.as_ref().is_some_and(|r| r.mouse_menu_active()) => {
				if let Some(r) = self.retail.as_mut() {
					r.mouse_back();
				}
			}
			WindowEvent::MouseInput { state: ElementState::Pressed, button: MouseButton::Left, .. } if self.retail.as_ref().is_some_and(|r| r.mouse_menu_active()) => {
				self.fire_mouse = false;
				if let Some(r) = self.retail.as_mut() {
					r.mouse_click();
				}
			}
			// Retail + modern camera, or Camera Classic with Strafe aiming On (retail/modcam.rs): RMB held aims.
			WindowEvent::MouseInput { state, button: MouseButton::Right, .. } if self.retail.is_some() && crate::retail::modcam::classic::aim_button() && !crate::retail::devtools::free_on() => crate::retail::modcam::ui_aim(state == ElementState::Pressed),
			WindowEvent::MouseInput { state, button: MouseButton::Right, .. } => self.looking = state == ElementState::Pressed,
			// Play: left / middle click (see `App::play_mouse`); in retail a click shorter than a frame still reaches the game.
			WindowEvent::MouseInput { state, button: b @ (MouseButton::Left | MouseButton::Middle), .. } => self.play_mouse((b == MouseButton::Middle) as usize, state == ElementState::Pressed, true),
			WindowEvent::KeyboardInput { event, .. } => {
				let PhysicalKey::Code(k) = event.physical_key else { return };
				// [PC] DEV ONLY (RAC_DEV_TOOLS=1, retail/devtools.rs): F1 planet list (its keys go nowhere else while it is open),
				// F2 free camera and its own keys. Off: nothing is taken.
				if self.retail.is_some() && crate::retail::devtools::ui_key(k, event.state == ElementState::Pressed, event.repeat) {
					return;
				}
				// [PC] Debug mode showcase (retail/splitview.rs): F10 split view, F11 hide HUD, F12 labels, [ ] Home \ divider.
				if self.retail.is_some() && crate::retail::splitview::key(k, event.state == ElementState::Pressed, event.repeat) {
					return;
				}
				// Held keys are tracked everywhere (the Sound page steps volume while Left/Right is held).
				self.key_change(k, event.state == ElementState::Pressed, event.repeat);
				// Retail + modern camera: F7 toggles it (retail/modcam.rs).
				if k == KeyCode::F7 && event.state == ElementState::Pressed && !event.repeat && self.retail.is_some() && crate::retail::modcam::enabled() {
					crate::retail::modcam::ui_toggle();
				}
				// [PC] PREVIEW (viewer/vegfx.rs): F4 hides / shows the extra plants while RAC_VEG is on.
				if k == KeyCode::F4 && event.state == ElementState::Pressed && !event.repeat && super::vegfx::cfg().env.is_some_and(|v| v > 0) {
					super::vegfx::toggle();
				}
				// [PC] Frame smoothing test key (RAC_SMOOTH_KEY=1, test launchers only): F9 flips it for this session (interp.rs).
				if event.state == ElementState::Pressed && !event.repeat && self.retail.is_some() {
					super::interp::key(k, crate::retail::pcext::graphics().smoothing);
				}
				// Dev debug keys (RAC_DEBUG_KEYS=1, retail/debugkeys.rs): F5 immortality, F6 infinite ammo, F8 +10000 bolts.
				if event.state == ElementState::Pressed && !event.repeat && self.retail.is_some() && crate::retail::debugkeys::enabled() {
					// [PC] F3: draw distance Low / the Graphics row's level (viewer/fardraw.rs), for before / after.
					if k == KeyCode::F3 {
						super::fardraw::toggle();
					}
					crate::retail::debugkeys::ui_key(k);
				}
			}
			WindowEvent::RedrawRequested => {
				if let Some(g) = &mut self.gpu {
					g.rscale_sync(); // [PC] a changed render scale (menu / env) applies before anything reads the frame size
				}
				let mut f = self.frame_begin();
				self.frame_input(&mut f);
				self.frame_ticks(&mut f);
				self.frame_shadows(&mut f);
				self.frame_sound_events(&mut f);
				self.frame_camera(&mut f);
				self.pose_service();
				self.frame_hero(&mut f);
				self.frame_hero_scene(&mut f);
				self.frame_hero_fx(&mut f);
				self.frame_props(&mut f);
				self.frame_hud_state(&mut f);
				self.frame_hud_draw(&mut f);
				self.frame_upload(&mut f);
				self.frame_cam_restore(&mut f);
				self.frame_audio_events(&mut f);
				self.frame_audio_anim(&mut f);
				self.frame_audio_music(&mut f);
				self.frame_audio_ambience(&mut f);
				self.frame_switch(&mut f);
				self.frame_render(&mut f, el);
			}
			_ => {}
		}
	}
}

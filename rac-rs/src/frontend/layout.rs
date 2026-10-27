//! The front end's page layouts (panel rects and the row layout of each page): the panels of a page and where its rows go.
//! Moved out of frontend.rs for the size caps; `use super::*` reaches the same constants and types.

use super::*;

pub(super) fn layout(page: Page, n: usize, in_game: bool) -> Layout {
	let l = |panels: &[[f32; 4]], rows: Rows| Layout { panels: panels.to_vec(), title: Some(panels[0]), rows };
	match page {
		Page::Root => Layout { panels: ROOT_PANELS.to_vec(), title: None, rows: Rows::Each(ROOT_PANELS.to_vec()) },
		Page::Pause => {
			let b: Vec<[f32; 4]> = PAUSE_Y.iter().take(n).map(|&y| [155.0, y, 203.0, 39.0]).collect();
			Layout { panels: b.clone(), title: None, rows: Rows::Each(b) }
		}
		Page::Options => l(&[[193.0, 23.0, 127.0, 47.0], [193.0, 119.0, 127.0, 176.0], TITLE_BACK], Rows::Column([193.0, 119.0, 127.0, 176.0])),
		Page::Language => l(&[[175.0, 23.0, 163.0, 47.0], [181.0, 119.0, 151.0, 176.0], TITLE_BACK], Rows::Column([181.0, 119.0, 151.0, 176.0])),
		Page::Video => l(&[[155.0, 23.0, 203.0, 47.0], [58.0, 170.0, 396.0, 62.0], TITLE_BACK], Rows::Column([58.0, 170.0, 396.0, 62.0])),
		// Video's header and back panels, the Sound page's value panel (pcext.rs `title_options` copies its slot-3 sequence).
		Page::Graphics => l(&[[155.0, 23.0, 203.0, 47.0], [47.0, 125.0, 419.0, 164.0], TITLE_BACK], Rows::Column([47.0, 125.0, 419.0, 164.0])),
		Page::Sound if in_game => l(&[OPTION_TITLE, [53.0, 127.0, 406.0, 164.0], GAME_BACK], Rows::Column([53.0, 127.0, 406.0, 164.0])),
		Page::Sound => l(&[[155.0, 23.0, 203.0, 47.0], [47.0, 125.0, 419.0, 164.0], [191.0, 346.0, 131.0, 78.0]], Rows::Column([47.0, 125.0, 419.0, 164.0])),
		Page::NewGame | Page::LoadGame => {
			let more = [168.0, 254.0, 322.0, 48.0];
			l(&[SLOT_TITLE, SLOT_STRIP, SLOT_INFO, more, [172.0, 345.0, 316.0, 80.0]], Rows::Slots(SLOT_STRIP, (page == Page::NewGame).then_some(more)))
		}
		Page::SaveGame | Page::LoadSaved => l(&[SLOT_TITLE, SLOT_STRIP, SLOT_INFO, [168.0, 254.0, 322.0, 48.0], [265.0, 358.0, 131.0, 67.0]], Rows::Slots(SLOT_STRIP, None)),
		Page::Overwrite => l(&[SLOT_TITLE, SLOT_STRIP, [188.0, 94.0, 287.0, 238.0], [265.0, 358.0, 131.0, 67.0]], Rows::None),
		// Only slot 3 is on screen; header and buttons sit inside it [I].
		Page::Quit => Layout { panels: vec![[173.0, 125.0, 166.0, 168.0]], title: Some([173.0, 129.0, 166.0, 30.0]), rows: Rows::None },
		Page::GameOptions => l(&[[136.0, 23.0, 241.0, 47.0], [23.0, 134.0, 148.0, 150.0], [341.0, 134.0, 148.0, 150.0], [191.0, 91.0, 131.0, 235.0], GAME_BACK], Rows::Column([191.0, 91.0, 131.0, 235.0])),
		Page::Help => l(&[[152.0, 24.0, 209.0, 47.0], [21.0, 134.0, 155.0, 148.0], [336.0, 134.0, 155.0, 148.0], [195.0, 91.0, 123.0, 235.0], [195.0, 345.0, 123.0, 78.0]], Rows::Column([195.0, 91.0, 123.0, 235.0])),
		Page::HelpLog => l(&[[23.0, 24.0, 209.0, 47.0], [23.0, 92.0, 209.0, 329.0], HELP_BODY, [326.0, 382.0, 92.0, 42.0]], Rows::Column([23.0, 92.0, 209.0, 329.0])),
		Page::Controls => l(&[[24.0, 24.0, 148.0, 47.0], [21.0, 92.0, 471.0, 266.0], [381.0, 381.0, 111.0, 43.0]], Rows::Column([21.0, 92.0, 471.0, 266.0])),
		Page::Moves => l(&[[22.0, 24.0, 178.0, 47.0], [22.0, 82.0, 178.0, 286.0], [225.0, 278.0, 266.0, 144.0], [225.0, 23.0, 266.0, 235.0], [54.0, 385.0, 113.0, 40.0]], Rows::Column([22.0, 82.0, 178.0, 286.0])),
		Page::WeaponsHelp | Page::GadgetsHelp => {
			let mut p = vec![[24.0, 24.0, 209.0, 47.0], [23.0, 90.0, 58.0, 211.0]];
			if page == Page::WeaponsHelp {
				p.extend([[101.0, 90.0, 134.0, 97.0], [101.0, 204.0, 134.0, 97.0], [253.0, 24.0, 238.0, 235.0]]);
			} else {
				p.extend([[101.0, 90.0, 134.0, 211.0], [253.0, 25.0, 238.0, 175.0], [253.0, 220.0, 238.0, 39.0]]);
			}
			p.extend([[23.0, 322.0, 212.0, 40.0], [64.0, 381.0, 134.0, 41.0], [253.0, 278.0, 238.0, 144.0]]);
			l(&p, Rows::Column(if page == Page::GadgetsHelp { [23.0, 90.0, 58.0, 211.0] } else { [101.0, 90.0, 134.0, 211.0] }))
		}
		Page::Weapons => l(&[[24.0, 23.0, 212.0, 35.0], [263.0, 25.0, 224.0, 236.0], [24.0, 90.0, 212.0, 147.0], [24.0, 264.0, 212.0, 100.0], [263.0, 282.0, 227.0, 37.0], [264.0, 342.0, 90.0, 82.0], EQUIP_PANEL, [378.0, 342.0, 113.0, 82.0]], Rows::None),
		Page::Gadgets => {
			let lists = GADGET_PANELS.map(|g| g.1);
			let mut p = vec![[24.0, 23.0, 212.0, 35.0], [263.0, 25.0, 224.0, 236.0], [263.0, 282.0, 227.0, 37.0], [264.0, 342.0, 90.0, 82.0], [378.0, 342.0, 113.0, 82.0]];
			p.extend(GADGET_PANELS.map(|g| g.0));
			p.extend(lists);
			l(&p, Rows::Each(lists.to_vec()))
		}
		Page::QuickSelect => l(&[[24.0, 23.0, 212.0, 35.0], [263.0, 25.0, 224.0, 236.0], [23.0, 85.0, 215.0, 148.0], [61.0, 258.0, 138.0, 105.0], [263.0, 282.0, 227.0, 37.0], [264.0, 342.0, 90.0, 82.0], [21.0, 390.0, 219.0, 35.0], [378.0, 342.0, 113.0, 82.0]], Rows::None),
		Page::Items => l(&[[22.0, 23.0, 189.0, 47.0], [22.0, 89.0, 189.0, 148.0], [238.0, 258.0, 253.0, 103.0], [237.0, 24.0, 253.0, 213.0], [22.0, 315.0, 189.0, 106.0], [391.0, 383.0, 100.0, 39.0], [22.0, 258.0, 189.0, 34.0]], Rows::None),
		Page::Goodies => l(&[[21.0, 25.0, 192.0, 42.0], [21.0, 93.0, 192.0, 228.0], [239.0, 25.0, 251.0, 297.0], [56.0, 343.0, 121.0, 81.0], [239.0, 343.0, 251.0, 81.0]], Rows::Column([21.0, 93.0, 192.0, 228.0])),
		Page::HelpDesk | Page::Subtitles => l(&[OPTION_TITLE, [166.0, 175.0, 172.0, 67.0], GAME_BACK], Rows::Column([166.0, 175.0, 172.0, 67.0])),
		Page::Camera => l(&[OPTION_TITLE, [69.0, 176.0, 381.0, 91.0], GAME_BACK], Rows::Column([69.0, 176.0, 381.0, 91.0])),
		// Galactic Map (frame 0): header s0, planet title s1, planet list s3; the viewer draws the map in s2.
		// The planet name goes in s1 (left) and the area name in s0 (retail capture) [I].
		Page::Map => Layout { title: Some([23.0, 27.0, 172.0, 35.0]), ..l(&[[214.0, 27.0, 276.0, 35.0], [23.0, 27.0, 172.0, 35.0], MAP_PANEL, [23.0, 81.0, 172.0, 170.0], [23.0, 277.0, 172.0, 145.0]], Rows::None) },
		// Page 0xf (level_transition.md, Veldin 0x1b6088): title s0, list s1, map s2, legend s3 and planet
		// picture s4 (corrected 2026-10-04: s3/s4 were guessed at (20,350) / (183,292)).
		Page::Galactic => l(&[[21.0, 23.0, 470.0, 42.0], [20.0, 86.0, 144.0, 251.0], [183.0, 86.0, 307.0, 197.0], [22.0, 358.0, 144.0, 66.0], [183.0, 304.0, 307.0, 120.0]], Rows::None),
		// Page 0x10 (0x1b63f8): s0 area name, s1 planet name, s2 checklist, s3 globe, s4 gold bolts, s5 legend
		// (items 0x1b3868, 0x1b35f8, 0x1b64d0, 0x1b6550, 0x1b65b8, 0x1b6480).
		Page::GalacticInfo => Layout { title: None, ..l(&[[268.0, 23.0, 223.0, 42.0], [22.0, 23.0, 223.0, 42.0], [268.0, 88.0, 223.0, 242.0], [22.0, 88.0, 223.0, 242.0], [268.0, 351.0, 223.0, 72.0], [55.0, 351.0, 155.0, 72.0]], Rows::None) },
		Page::Title => Layout { panels: Vec::new(), title: None, rows: Rows::None },
	}
}

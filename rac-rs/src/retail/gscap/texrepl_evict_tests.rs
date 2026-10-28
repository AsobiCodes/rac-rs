use super::*;
use crate::retail::gscap::draw::cpu_repl::CpuRepl;
use crate::retail::gscap::draw::gpu_repl::Book;

fn nm(n: u64) -> TexName {
    TexName::palette(n, n + 1, 0x1dd3)
}

fn table(serial: u64) -> Arc<CpuRepl> {
    Arc::new(CpuRepl { w: 5, h: 5, serial, px: vec![[0; 4]; 25] })
}

fn entry(n: Option<u64>, word: u32) -> Entry {
    Entry { named: n.map(|n| (nm(n), [0; 4])), word, done: true, room: n.is_some() && word == 0, ..Default::default() }
}

fn need(w: u32, h: u32) -> Need {
    Need { size: (w, h), slot: hudrepl::test_slot(), rw: w, rh: h }
}

/// A bare `Book` as the field end's atlas: the real packer and planner, no GPU.
struct Bk {
    book: Book,
    placed: Vec<TexName>,
}

impl Atlas for Bk {
    fn evict(&mut self, needs: &[(TexName, (u32, u32))], keep_from: u64) -> (Vec<TexName>, Vec<TexName>) {
        let (rows, gave_up) = self.book.plan_keys(needs, keep_from);
        (self.book.free_rows(&rows), gave_up)
    }

    fn place(&mut self, name: &TexName, need: &Need) {
        if self.book.place(*name, need.size.0, need.size.1).is_ok() {
            self.placed.push(*name);
        }
    }
}

impl Bk {
    fn new(rows: u32) -> Bk {
        Bk { book: Book::new(rows), placed: Vec::new() }
    }

    /// An image of name `n` is in the atlas (nobody drew it yet): its record word.
    fn add(&mut self, n: u64, w: u32, h: u32) -> u32 {
        pack_word(self.book.place(nm(n), w, h).unwrap().id, 0, 0)
    }

    /// A prim drew name `n` in the current field (`repl_resolve`'s two steps).
    fn draw(&mut self, st: &mut ReplState, n: u64) {
        let field = st.drawn();
        let id = self.book.id_of(&nm(n)).unwrap();
        self.book.touch(id, field);
    }

    fn has(&self, n: u64) -> bool {
        self.book.id_of(&nm(n)).is_some()
    }
}

fn end(st: &mut ReplState, a: &mut Bk) {
    st.field_end(Some(a));
}

#[test]
fn an_eviction_forgets_every_entry_of_the_evicted_name_and_gives_refused_ones_another_try() {
    let mut st = ReplState::default();
    st.cache.insert((1, 0, 0, 0), entry(Some(1), 5));
    st.cache.insert((2, 0, 0, 0), entry(Some(1), 6)); // a second texture of the same name
    st.cache.insert((3, 0, 0, 0), entry(Some(2), 7));
    st.cache.insert((4, 0, 0, 0), entry(Some(3), 0)); // refused for room: word 0
    st.cache.insert((6, 0, 0, 0), entry(None, 0)); // no name (a render target): stays done
    st.cur = Some(((5, 0, 0, 0), entry(Some(2), 8))); // the entry in use counts too
    st.forget(|e| e.word != 0 && e.named.as_ref().is_some_and(|(n, _)| *n == nm(1)));
    let words: Vec<u32> = (1..=4).map(|k| st.cache[&(k, 0, 0, 0)].word).collect();
    assert_eq!(words, vec![0, 0, 7, 0], "both entries of the evicted name lost their atlas word, the others kept theirs");
    assert!(st.cache[&(1, 0, 0, 0)].named.is_some(), "the name stays: the entry asks the pack again");
    assert!(!st.cache[&(1, 0, 0, 0)].done && !st.cache[&(2, 0, 0, 0)].done, "the evicted entries ask again");
    assert!(!st.cache[&(4, 0, 0, 0)].done, "an entry refused for room tries again");
    assert!(st.cache[&(3, 0, 0, 0)].done && st.cur.as_ref().unwrap().1.done, "an entry that kept its image (or was refused for another reason) is left alone");
    assert!(st.cache[&(6, 0, 0, 0)].done, "an unnamed entry is not retried");
    assert_eq!(st.cur.as_ref().unwrap().1.word, 8);
}

#[test]
fn table_eviction_drops_the_least_recently_used_tables_until_the_refused_bytes_fit() {
    let mut st = ReplState::default();
    // Three tables of 100 bytes filling the budget exactly; table 1 was drawn in field 5, table 2 in field 2, table 3 never (use by serial).
    for (k, serial) in [(1u64, 1u64), (2, 2), (3, 3)] {
        st.tabs.insert((nm(k), 5, 5), table(serial));
    }
    st.tab_use = HashMap::from([(1, 5), (2, 2)]);
    st.tab_bytes = cpu::tab_budget();
    for (key, serial) in [(10u64, 1u64), (11, 2)] {
        let mut e = entry(Some(serial), 0);
        e.cpu = Some(CpuSel { tab: st.tabs[&(nm(serial), 5, 5)].clone(), x: 0, y: 0 });
        st.cache.insert((key, 0, 0, 0), e);
    }
    assert_eq!(st.evict_tables(250, 5), None, "the refused bytes do not fit without table 1 either: nothing is dropped");
    assert_eq!(st.tabs.len(), 3);
    let gone = st.evict_tables(150, 6).unwrap();
    assert_eq!(gone, HashSet::from([3, 2]), "the unused table first, then the one drawn longest ago; two are enough for 150 bytes");
    assert_eq!((st.tabs.len(), st.tab_bytes), (1, cpu::tab_budget() - 200));
    assert!(st.tabs.contains_key(&(nm(1), 5, 5)));
    st.forget(|e| e.cpu.as_ref().is_some_and(|c| gone.contains(&c.tab.serial)));
    assert!(st.cache[&(11, 0, 0, 0)].cpu.is_none(), "the entry that held the evicted table asks again");
    assert!(st.cache[&(10, 0, 0, 0)].cpu.is_some());
    // Room that was not refused evicts nothing.
    assert_eq!(st.evict_tables(1, 6), Some(HashSet::new()));
}

#[test]
fn a_texture_rebuilt_under_another_name_in_the_same_field_keeps_the_use_of_the_old_name() {
    // N1: names 1 and 2 were both drawn in field 1 from the same texture slot (one cache key: the texture was uploaded again and rebuilt as
    // name 2 between the two draws). The entry now holds name 2 only; name 1 must still count as drawn in field 1.
    let (mut st, mut a) = (ReplState::default(), Bk::new(600));
    a.add(1, 8000, 290);
    let w2 = a.add(2, 8000, 290); // two shelves of 291 rows, 18 rows left
    st.cache.insert((1, 0, 0, 0), entry(Some(2), w2));
    let w1 = pack_word(a.book.id_of(&nm(1)).unwrap(), 0, 0);
    st.cache.insert((2, 0, 0, 0), entry(Some(1), w1)); // another texture that holds name 1 (not drawn this field)
    a.draw(&mut st, 1);
    a.draw(&mut st, 2);
    st.refuse_img(nm(3), need(8000, 290));
    st.cache.insert((3, 0, 0, 0), entry(Some(3), 0));
    end(&mut st, &mut a);
    assert!(a.has(1) && a.has(2), "both were drawn in the field that just ended: nothing is freed for name 3");
    assert!(a.placed.is_empty());
    // Field 2 draws name 2 only and name 4 is refused: name 1 (last drawn in field 1) is the one to go, name 2 stays.
    a.draw(&mut st, 2);
    st.refuse_img(nm(3), need(8000, 290)); // given up in field 1: no new need while it waits
    st.refuse_img(nm(4), need(8000, 290));
    assert_eq!(st.need_imgs.len(), 1);
    end(&mut st, &mut a);
    assert!(!a.has(1) && a.has(2));
    assert_eq!(a.placed, vec![nm(4)], "name 4 is placed in the freed shelf at once");
    assert_eq!(st.cache[&(2, 0, 0, 0)].word, 0, "the entry that held the evicted name lost its word");
    assert!(!st.cache[&(2, 0, 0, 0)].done, "... and asks again");
    assert_eq!(st.cache[&(1, 0, 0, 0)].word, w2, "the drawn name keeps its word");
}

#[test]
fn fields_without_replacements_do_not_move_the_protection() {
    // N6: field 1 drew names 1 and 2; fields 2..=6 draw none (lag fields, a load screen); the back-off retry lands on field 7.
    let (mut st, mut a) = (ReplState::default(), Bk::new(600));
    a.add(1, 8000, 290);
    a.add(2, 8000, 290);
    a.draw(&mut st, 1);
    a.draw(&mut st, 2);
    end(&mut st, &mut a);
    for _ in 0..5 {
        end(&mut st, &mut a);
    }
    st.refuse_img(nm(3), need(8000, 290));
    end(&mut st, &mut a);
    assert!(a.has(1) && a.has(2), "what the last drawing field drew is still protected (a protection of one field would free both)");
    assert!(a.placed.is_empty());
    assert_eq!(st.drew, 1);
}

#[test]
fn two_textures_of_one_image_are_one_need_and_a_given_up_image_waits_without_a_need() {
    // N2: three shelves of 291 rows in 900 rows (27 left). Name 9 is refused by two textures: one shelf is enough.
    let (mut st, mut a) = (ReplState::default(), Bk::new(900));
    for n in 1..=3 {
        a.add(n, 8000, 290);
    }
    st.refuse_img(nm(9), need(8000, 290));
    st.refuse_img(nm(9), need(8000, 290));
    assert_eq!(st.need_imgs.len(), 1, "one need per image, not per texture entry");
    end(&mut st, &mut a);
    assert_eq!((a.book.len(), a.placed.clone()), (3, vec![nm(9)]), "one old shelf freed, not two");
    // Name 10 is 700 rows tall: with name 9 drawn (protected) and 2 old shelves + 27 end rows it cannot fit. It is given up for 64 fields.
    a.draw(&mut st, 9);
    st.cache.insert((5, 0, 0, 0), entry(Some(10), 0));
    st.refuse_img(nm(10), need(8000, 700));
    end(&mut st, &mut a);
    assert_eq!(a.book.len(), 3, "an image that cannot fit evicts nothing");
    assert!(st.need_imgs.is_empty() && st.need_tabs.is_empty(), "the need is settled, not carried on");
    let reopen = st.reopen;
    assert_eq!(reopen, st.frame + RETRY_FIELDS, "looked at again after the back-off");
    // The entry asks again at once after the eviction attempt, is refused again and records no need.
    let e = st.cache.get_mut(&(5, 0, 0, 0)).unwrap();
    assert!(!e.done, "an entry refused for room asks again after every attempt");
    (e.done, e.room) = (true, true);
    st.refuse_img(nm(10), need(8000, 700));
    assert!(st.need_imgs.is_empty(), "no new need while the image is given up");
    while st.frame + 1 < reopen {
        end(&mut st, &mut a);
    }
    assert!(st.cache[&(5, 0, 0, 0)].done, "still waiting one field before the time is over");
    end(&mut st, &mut a);
    assert!(!st.cache[&(5, 0, 0, 0)].done && st.retry.is_empty() && st.reopen == 0, "the time is over: the entry asks again");
    st.refuse_img(nm(10), need(8000, 700));
    assert_eq!(st.need_imgs.len(), 1, "a need is recorded again");
}

#[test]
fn one_image_that_cannot_fit_does_not_hold_up_the_others() {
    // N5: 700 rows, three shelves of 231 (7 left); name 3 is on screen. Name 7 (601 rows) cannot fit, name 8 (201 rows) can.
    let (mut st, mut a) = (ReplState::default(), Bk::new(700));
    for n in 1..=3 {
        a.add(n, 8000, 230);
    }
    a.draw(&mut st, 3);
    st.refuse_img(nm(7), need(8000, 600));
    st.refuse_img(nm(8), need(8000, 200));
    end(&mut st, &mut a);
    assert_eq!(a.placed, vec![nm(8)], "the image that fits is placed; the big one is given up");
    assert!(!a.has(1) && a.has(2) && a.has(3), "the least recently used shelf went (ties: the upper one)");
    assert!(st.retry.contains_key(&(nm(7), 0, 0)));
    // The next field: another refusal is planned at once, not after 64 fields. Name 7 is asked again meanwhile and records nothing.
    a.draw(&mut st, 3);
    a.draw(&mut st, 8);
    st.refuse_img(nm(7), need(8000, 600));
    st.refuse_img(nm(9), need(8000, 200));
    assert_eq!(st.need_imgs.len(), 1);
    end(&mut st, &mut a);
    assert_eq!(a.placed, vec![nm(8), nm(9)]);
    assert!(!a.has(2), "name 2 went for name 9");
}

#[test]
fn refused_images_are_placed_in_the_order_of_the_plan_not_the_draw_order() {
    // N4: a free 100-row shelf and 31 end rows (name 2's shelf, drawn, sits between them). Names 5 (30 rows) and 6 (90 rows) are refused.
    let layout = || {
        let mut a = Bk::new(162);
        a.add(1, 8000, 99);
        a.add(2, 8000, 30);
        a
    };
    // Drawn in the game's order (the small one first) the small image cuts the free shelf and the tall one has no place any more.
    let mut b = layout();
    let (rows, _) = b.book.plan_keys(&[((), (8000, 30)), ((), (8000, 90))], 1);
    b.book.free_rows(&rows);
    assert!(b.book.place(nm(5), 8000, 30).is_ok());
    assert!(b.book.place(nm(6), 8000, 90).is_err(), "the order the plan did not use fails");
    // The field end places them tallest first, as the dry run did.
    let (mut st, mut a) = (ReplState::default(), layout());
    a.book.touch(a.book.id_of(&nm(2)).unwrap(), 1);
    st.drew = 1;
    st.refuse_img(nm(5), need(8000, 30));
    st.refuse_img(nm(6), need(8000, 90));
    end(&mut st, &mut a);
    assert_eq!(a.placed, vec![nm(6), nm(5)]);
    assert!(a.has(2) && !a.has(1));
}

#[test]
fn the_gpu_takeover_drops_the_cpu_tables_with_their_refusals() {
    // N7: the CPU capture refused a table for room; the GPU capture takes over.
    let (mut st, mut a) = (ReplState::default(), Bk::new(100));
    st.tabs.insert((nm(1), 5, 5), table(1));
    st.tab_bytes = 100;
    st.refuse_tab((nm(2), 5, 5), 100);
    st.cache.insert((1, 0, 0, 0), entry(Some(2), 0));
    let mut held = entry(Some(1), 0);
    held.done = false;
    held.cpu = Some(CpuSel { tab: st.tabs[&(nm(1), 5, 5)].clone(), x: 0, y: 0 });
    st.cache.insert((2, 0, 0, 0), held);
    st.retry.insert((nm(2), 5, 5), 1000);
    end(&mut st, &mut a);
    assert!(st.tabs.is_empty() && st.tab_bytes == 0 && st.need_tabs.is_empty() && st.retry.is_empty());
    assert!(!st.cache[&(1, 0, 0, 0)].done, "the entry refused by the CPU tables asks the atlas now");
    assert!(st.cache[&(2, 0, 0, 0)].cpu.is_none());
}

#[test]
fn a_table_too_big_for_the_budget_is_given_up_and_does_not_stop_the_smaller_ones() {
    // N2 (CPU): table 1 was drawn in field 3 (protected), table 2 never. One refused table is as big as the budget (cannot fit next to
    // table 1), the other fits once table 2 is gone.
    let mut st = ReplState::default();
    st.tabs.insert((nm(1), 5, 5), table(1));
    st.tabs.insert((nm(2), 5, 5), table(2));
    st.tab_use = HashMap::from([(1, 3)]);
    st.tab_bytes = 200;
    (st.frame, st.drew) = (3, 3);
    let budget = cpu::tab_budget() as u64;
    st.refuse_tab((nm(10), 5, 5), budget);
    st.refuse_tab((nm(11), 5, 5), budget - 150);
    st.refuse_tab((nm(11), 5, 5), budget - 150);
    assert_eq!(st.need_tabs.len(), 2, "one need per table");
    st.field_end(None);
    assert!(st.tabs.contains_key(&(nm(1), 5, 5)) && !st.tabs.contains_key(&(nm(2), 5, 5)), "the unused table went, the drawn one stays");
    assert_eq!(st.tab_bytes, 100);
    assert!(st.need_tabs.is_empty(), "the need is settled");
    assert!(st.retry.contains_key(&(nm(10), 5, 5)) && !st.retry.contains_key(&(nm(11), 5, 5)));
    st.refuse_tab((nm(10), 5, 5), budget);
    assert!(st.need_tabs.is_empty(), "the table that cannot fit records nothing while it waits, so a later small table is not blocked by it");
    st.refuse_tab((nm(12), 5, 5), 100);
    assert_eq!(st.need_tabs.len(), 1);
}

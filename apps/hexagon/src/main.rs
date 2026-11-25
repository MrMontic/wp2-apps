#![cfg_attr(any(target_os = "none", wp2_sim), no_main, cfg_attr(target_os = "none", no_std))]

#[cfg_attr(not(any(target_os = "none", wp2_sim)), allow(dead_code))]
mod game;
#[cfg_attr(not(any(target_os = "none", wp2_sim)), allow(dead_code))]
mod view;

#[cfg(not(any(target_os = "none", wp2_sim)))]
fn main() {}

#[cfg(any(target_os = "none", wp2_sim))]
mod device {
    use crate::game::{FRAME_MS, Game, Mode, SAVE_LEN, State, Step, decode_save, encode_save};
    use crate::view::{RenderScratch, clear_screen, render};
    use cmf_wp2_app::{Api, BUTTON, CROWN, PAUSE, RESUME, TOUCH_DOWN, TOUCH_UP};
    use cmf_wp2_ui::haptic::{DONE, DOUBLE, Motor, TICK};
    use core::mem::MaybeUninit;
    use core::ptr::addr_of_mut;

    cmf_wp2_app::entry!(main);
    cmf_wp2_app::app_icon!();

    /// Gentle Title Screen rotation cadence (~28 FPS).
    const TITLE_FRAME_MS: u32 = 36;
    /// Cap physics catch-up so a slow tick never teleports walls onto the player.
    const CATCH_UP: u32 = 2;
    /// Guard against accidental instant tap after Game Over appears.
    const RETRY_GUARD_MS: u32 = 450;

    /// Keep Game and RenderScratch off the 8 KiB stack!
    static mut GAME_MEM: MaybeUninit<Game> = MaybeUninit::uninit();
    static mut SCRATCH: RenderScratch = RenderScratch::new();

    #[inline]
    fn due(now: u32, at: u32) -> bool {
        (now.wrapping_sub(at) as i32) >= 0
    }

    fn wait_while_paused(api: &Api) -> bool {
        loop {
            match api.event(u32::MAX).map(|e| e.kind) {
                Some(RESUME) => return true,
                Some(BUTTON) => return false,
                _ => {}
            }
        }
    }

    fn seed(api: &Api) -> u32 {
        let t = api
            .time()
            .map_or(0, |t| (t.minute as u32 * 60 + t.second as u32) * 1000 + t.millis as u32);
        api.now().wrapping_mul(2_654_435_761) ^ t ^ 0x517C_C1B7
    }

    fn save_if_dirty(api: &Api, best_cs: [u32; Mode::COUNT], saved_best: &mut [u32; Mode::COUNT]) {
        if best_cs != *saved_best {
            api.save(&encode_save(best_cs));
            *saved_best = best_cs;
        }
    }

    fn main(api: &'static Api) -> i32 {
        let mut screen = api;
        let mut motor = Motor::new(api);

        let mut blob = [0u8; SAVE_LEN];
        let n = api.load(&mut blob);
        let loaded = if n > 0 {
            decode_save(&blob[..(n as usize).min(SAVE_LEN)])
        } else {
            [0; Mode::COUNT]
        };
        let mut saved_best = loaded;

        // Initialize statics safely via addr_of_mut! (single-threaded bare-metal entry).
        let (game, scratch): (&mut Game, &mut RenderScratch) = unsafe {
            let g_ptr = addr_of_mut!(GAME_MEM);
            (*g_ptr).write(Game::new(seed(api), loaded));
            ((*g_ptr).assume_init_mut(), &mut *addr_of_mut!(SCRATCH))
        };

        clear_screen(&mut screen);
        render(&mut screen, game, scratch);
        api.show();

        let mut next = api.now().wrapping_add(TITLE_FRAME_MS);
        let mut over_at = 0u32;

        loop {
            motor.update();
            let now = api.now();

            let left = match game.state {
                State::Over => u32::MAX,
                State::Title | State::Playing | State::Dying => {
                    (next.wrapping_sub(now) as i32).max(0) as u32
                }
            };
            let wait = motor.wait().map_or(left, |m| m.min(left));

            // Drain ALL queued events in one batch so rapid crown spins never starve frames!
            let mut crown_acc: i32 = 0;
            let mut tap: Option<(i32, i32)> = None;
            let mut resumed = false;

            if let Some(first) = api.event(wait) {
                let mut ev = Some(first);
                while let Some(e) = ev {
                    match e.kind {
                        BUTTON => {
                            motor.off();
                            save_if_dirty(api, game.best_cs, &mut saved_best);
                            api.keep_screen_on(false);
                            return 0;
                        }
                        PAUSE => {
                            motor.off();
                            game.touch_up();
                            save_if_dirty(api, game.best_cs, &mut saved_best);
                            api.keep_screen_on(false);
                            if !wait_while_paused(api) {
                                return 0;
                            }
                            resumed = true;
                            crown_acc = 0;
                            tap = None;
                            break;
                        }
                        CROWN => {
                            crown_acc += e.delta as i32;
                        }
                        TOUCH_DOWN => {
                            let (tx, ty) = (e.x as i32, e.y as i32);
                            tap = Some((tx, ty));
                            if game.state == State::Playing {
                                game.touch_down(tx);
                            }
                        }
                        TOUCH_UP => {
                            if game.state == State::Playing {
                                game.touch_up();
                            }
                        }
                        _ => {}
                    }
                    ev = api.event(0);
                }
            }

            if resumed {
                clear_screen(&mut screen);
                render(&mut screen, game, scratch);
                api.show();
                let active = game.state == State::Playing || game.state == State::Dying;
                api.keep_screen_on(active);
                let dt = if game.state == State::Title { TITLE_FRAME_MS } else { FRAME_MS };
                next = api.now().wrapping_add(dt);
                continue;
            }

            let mut need_draw = false;

            match game.state {
                State::Title => {
                    if crown_acc > 0 {
                        game.next_mode();
                        motor.play(TICK);
                        need_draw = true;
                    } else if crown_acc < 0 {
                        game.prev_mode();
                        motor.play(TICK);
                        need_draw = true;
                    }

                    if let Some((tx, ty)) = tap {
                        if ty >= 164 && ty <= 296 && tx < 148 {
                            game.prev_mode();
                            motor.play(TICK);
                            need_draw = true;
                        } else if ty >= 164 && ty <= 296 && tx > 318 {
                            game.next_mode();
                            motor.play(TICK);
                            need_draw = true;
                        } else {
                            game.start();
                            motor.play(TICK);
                            api.keep_screen_on(true);
                            render(&mut screen, game, scratch);
                            api.show();
                            next = api.now().wrapping_add(FRAME_MS);
                            continue;
                        }
                    }

                    let tnow = api.now();
                    if due(tnow, next) {
                        game.step_title();
                        next = tnow.wrapping_add(TITLE_FRAME_MS);
                        need_draw = true;
                    }
                }
                State::Playing | State::Dying => {
                    if crown_acc != 0 && game.state == State::Playing {
                        game.apply_crown(crown_acc);
                        need_draw = true;
                    }

                    let tnow = api.now();
                    if due(tnow, next) {
                        let mut steps = 0u32;
                        while steps < CATCH_UP && due(tnow, next) {
                            next = next.wrapping_add(FRAME_MS);
                            steps += 1;
                            match game.step() {
                                Step::Nothing => {}
                                Step::RankUp => motor.play(DOUBLE),
                                Step::Morph => motor.play(TICK),
                                Step::Crashed => {
                                    motor.play(if game.new_best { DOUBLE } else { DONE });
                                    save_if_dirty(api, game.best_cs, &mut saved_best);
                                    break;
                                }
                                Step::OverReady => {
                                    over_at = tnow;
                                    api.keep_screen_on(false);
                                    break;
                                }
                            }
                        }
                        if due(tnow, next) {
                            next = tnow.wrapping_add(FRAME_MS);
                        }
                        need_draw = true;
                    }
                }
                State::Over => {
                    let tnow = api.now();
                    if due(tnow, over_at.wrapping_add(RETRY_GUARD_MS)) {
                        if crown_acc != 0 {
                            game.to_title();
                            motor.play(TICK);
                            render(&mut screen, game, scratch);
                            api.show();
                            next = tnow.wrapping_add(TITLE_FRAME_MS);
                            continue;
                        }
                        if let Some((_tx, ty)) = tap {
                            if ty >= 318 {
                                game.to_title();
                                motor.play(TICK);
                                render(&mut screen, game, scratch);
                                api.show();
                                next = tnow.wrapping_add(TITLE_FRAME_MS);
                            } else {
                                game.start();
                                motor.play(TICK);
                                api.keep_screen_on(true);
                                render(&mut screen, game, scratch);
                                api.show();
                                next = tnow.wrapping_add(FRAME_MS);
                            }
                            continue;
                        }
                    }
                }
            }

            if need_draw {
                render(&mut screen, game, scratch);
                api.show();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::game::{
        Game, Mode, PLAYER_HIT_IN, PLAYER_HIT_OUT, Q8, Rank, SAVE_LEN, State, Step, Wall,
        decode_save, encode_save, fmt_time, sanitize_wave_mask,
    };
    use crate::view::{RenderScratch, clear_screen, render};
    use cmf_wp2_ui::{Fb, SIZE};

    const N_PX: usize = (SIZE * SIZE) as usize;

    #[test]
    fn pattern_generator_is_always_passable() {
        // Direct invariant check on sanitize_wave_mask for 4, 5, 6 sides
        for sides in [4u8, 5, 6] {
            let full = (1u8 << sides) - 1;
            for gap in 0..sides {
                let m = sanitize_wave_mask(0xFF, sides, gap);
                assert_ne!(m, full, "sanitize must never allow all {sides} sectors closed");
                assert_eq!(m & !full, 0, "no out-of-range bits");
                assert_eq!((m >> gap) & 1, 0, "fallback gap must remain open");
            }
        }

        // Stress-test 300 waves per mode across all 5 modes and 4/5/6 sides!
        for mode in [
            Mode::Easy,
            Mode::Medium,
            Mode::Hexagon,
            Mode::Hexagoner,
            Mode::Hexagonest,
        ] {
            let mut g = Game::new(0xDEAD_BEEF ^ (mode.idx() as u32 + 1), [0; Mode::COUNT]);
            g.set_mode(mode);
            g.start();

            for sides in [6u8, 5, 4] {
                if mode != Mode::Hexagonest && sides != 6 {
                    continue;
                }
                g.sides = sides;
                let full = (1u8 << sides) - 1;
                for wave_i in 0..300 {
                    g.walls = [Wall::EMPTY; 32];
                    let speed = g.wall_speed_q8();
                    g.spawn_next_wave(speed);

                    let mut mask = 0u8;
                    let mut count = 0u8;
                    for w in &g.walls {
                        if w.active {
                            assert!(w.sector < sides, "sector {} >= sides {sides}", w.sector);
                            mask |= 1u8 << w.sector;
                            count += 1;
                        }
                    }
                    assert!(count > 0, "wave {wave_i} spawned zero walls");
                    assert_ne!(
                        mask, full,
                        "IMPOSSIBLE WAVE in {mode:?} (sides={sides}, wave={wave_i}): mask={mask:#b}"
                    );
                }
            }
        }
    }

    #[test]
    fn collision_side_slide_vs_front_crash() {
        let mut g = Game::new(42, [0; Mode::COUNT]);
        g.start();
        g.walls = [Wall::EMPTY; 32];
        g.sides = 6;
        g.player_angle = g.sector_center(0);

        // Place a wall occupying sector 1 right across the player's radial band
        g.walls[0] = Wall {
            active: true,
            sector: 1,
            r_in_q8: (PLAYER_HIT_IN - 4) * Q8,
            thick_q8: 20 * Q8,
        };

        // Steering CW into the side of sector 1's wall MUST slide cleanly without dying!
        for _ in 0..12 {
            g.move_player_angular(1800);
        }
        assert_eq!(g.state, State::Playing, "side contact should slide, not crash");
        assert_eq!(
            g.sector_of(g.player_angle),
            0,
            "player should stay clamped inside open sector 0"
        );

        // Fast crown burst into the side wall should also slide without tunneling
        g.apply_crown(6);
        assert_eq!(g.state, State::Playing);
        assert_eq!(g.sector_of(g.player_angle), 0);

        // Now place a closing wall directly ahead of the player in sector 0
        g.walls[1] = Wall {
            active: true,
            sector: 0,
            r_in_q8: (PLAYER_HIT_OUT + 1) * Q8,
            thick_q8: 20 * Q8,
        };

        // Advancing one tick moves the wall's front face into the player -> CRASH!
        let step = g.step();
        assert_eq!(step, Step::Crashed, "front face impact must crash");
        assert_eq!(g.state, State::Dying);

        // Stepping through crash flash/shake transitions cleanly to Game Over
        let mut reached_over = false;
        for _ in 0..20 {
            if g.step() == Step::OverReady {
                reached_over = true;
                break;
            }
        }
        assert!(reached_over, "Dying must transition to Over");
        assert_eq!(g.state, State::Over);
    }

    #[test]
    fn save_load_roundtrip_and_corruption() {
        for sample in [
            [0u32, 0, 0, 0, 0],
            [4520, 2310, 1248, 3512, 6299],
            [999_999, 888, 1, 42, 7777],
        ] {
            let encoded = encode_save(sample);
            assert_eq!(encoded.len(), SAVE_LEN);
            assert_eq!(&encoded[0..4], b"HEX2");
            assert_eq!(decode_save(&encoded), sample);
        }

        let valid = encode_save([3200, 1800, 1500, 2500, 4500]);

        // Bad magic
        let mut bad_magic = valid;
        bad_magic[0] = b'X';
        assert_eq!(decode_save(&bad_magic), [0; Mode::COUNT]);

        // Corrupted payload byte
        let mut bad_data = valid;
        bad_data[6] ^= 0x01;
        assert_eq!(decode_save(&bad_data), [0; Mode::COUNT]);

        // Corrupted checksum byte
        let mut bad_sum = valid;
        bad_sum[SAVE_LEN - 1] ^= 0x80;
        assert_eq!(decode_save(&bad_sum), [0; Mode::COUNT]);

        // Truncated & all-zero blobs
        assert_eq!(decode_save(&valid[..SAVE_LEN - 1]), [0; Mode::COUNT]);
        assert_eq!(decode_save(&[0u8; SAVE_LEN]), [0; Mode::COUNT]);
    }

    #[test]
    fn ranks_and_time_formatting() {
        assert_eq!(Rank::from_cs(0), Rank::Point);
        assert_eq!(Rank::from_cs(999), Rank::Point);
        assert_eq!(Rank::from_cs(1000), Rank::Line);
        assert_eq!(Rank::from_cs(2000), Rank::Triangle);
        assert_eq!(Rank::from_cs(3000), Rank::Square);
        assert_eq!(Rank::from_cs(4500), Rank::Pentagon);
        assert_eq!(Rank::from_cs(6000), Rank::Hexagon);
        assert_eq!(Rank::from_cs(99999), Rank::Hexagon);

        let mut buf = [0u8; 12];
        assert_eq!(fmt_time(0, &mut buf), "00.00");
        assert_eq!(fmt_time(5, &mut buf), "00.05");
        assert_eq!(fmt_time(950, &mut buf), "09.50");
        assert_eq!(fmt_time(1248, &mut buf), "12.48");
        assert_eq!(fmt_time(12345, &mut buf), "123.45");
    }

    #[test]
    fn render_all_screens_within_rect_budget() {
        let mut px = vec![0u16; N_PX];
        let mut scratch = Box::new(RenderScratch::new());
        let mut g = Game::new(777, [5120, 3410, 1425, 2890, 6110]);

        // 1. Title screen across all 5 modes
        clear_screen(&mut Fb::new(&mut px));
        for mode in [
            Mode::Easy,
            Mode::Medium,
            Mode::Hexagon,
            Mode::Hexagoner,
            Mode::Hexagonest,
        ] {
            g.set_mode(mode);
            g.step_title();
            let mut fb = Fb::new(&mut px);
            render(&mut fb, &g, &mut scratch);
            assert!(fb.rects > 100 && fb.rects <= 1050, "Title {mode:?} rects={}", fb.rects);
        }

        // 2. Gameplay across ticks with active walls, rotating world, and 6/5/4 sides!
        let mut min_rects = u32::MAX;
        let mut max_rects = 0u32;
        let mut sum_rects = 0u64;
        let mut frames = 0u64;

        for (mode, sides) in [
            (Mode::Easy, 6u8),
            (Mode::Medium, 6u8),
            (Mode::Hexagon, 6u8),
            (Mode::Hexagoner, 6u8),
            (Mode::Hexagonest, 5u8),
            (Mode::Hexagonest, 4u8),
        ] {
            g.set_mode(mode);
            g.start();
            g.sides = sides;
            // Pre-populate multiple dense waves on screen to stress worst-case rect count
            g.walls = [Wall::EMPTY; 32];
            for r in [82, 138, 194] {
                let speed = g.wall_speed_q8();
                g.spawn_next_wave(speed);
                for w in &mut g.walls {
                    if w.active && w.r_in_q8 > 230 * Q8 {
                        w.r_in_q8 = r * Q8;
                    }
                }
            }

            for t in 0..15 {
                if t % 3 == 0 {
                    g.apply_crown(1);
                }
                let _ = g.step();
                if g.state != State::Playing {
                    g.start();
                    g.sides = sides;
                }
                let mut fb = Fb::new(&mut px);
                render(&mut fb, &g, &mut scratch);
                min_rects = min_rects.min(fb.rects);
                max_rects = max_rects.max(fb.rects);
                sum_rects += fb.rects as u64;
                frames += 1;
                assert!(
                    fb.rects <= 1050,
                    "Gameplay rect budget exceeded ({mode:?}, sides={sides}, tick={t}): {} > 1050",
                    fb.rects
                );
            }
        }

        let avg_rects = sum_rects / frames.max(1);
        std::println!(
            "Gameplay frame rect counts across {frames} frames: min={min_rects}, avg={avg_rects}, max={max_rects}"
        );

        // Verify 100% single-pass circular screen coverage (zero gaps/cracks inside r <= 224)
        // and tiny static RAM footprint (< 4 KiB total).
        let ram_bytes = core::mem::size_of::<Game>() + core::mem::size_of::<RenderScratch>();
        assert!(ram_bytes < 4096, "static RAM too large: {ram_bytes} bytes");
        px.fill(0x1234);
        g.set_mode(Mode::Hexagon);
        g.start();
        let _ = g.step();
        let mut fb = Fb::new(&mut px);
        render(&mut fb, &g, &mut scratch);
        let mut uncovered = 0usize;
        for y in 12..SIZE - 12 {
            let dy = y - SIZE / 2;
            for x in 12..SIZE - 12 {
                let dx = x - SIZE / 2;
                if dx * dx + dy * dy <= 222 * 222 && px[(y * SIZE + x) as usize] == 0x1234 {
                    uncovered += 1;
                }
            }
        }
        assert_eq!(uncovered, 0, "found {uncovered} uncovered pixels inside circular arena");

        // 3. Dying flash/shake frame & Game Over overlay
        g.state = State::Dying;
        g.dying_ticks = 9;
        g.shake_x = 5;
        g.shake_y = -3;
        let mut fb = Fb::new(&mut px);
        render(&mut fb, &g, &mut scratch);
        assert!(fb.rects <= 1050, "Dying rects={}", fb.rects);

        g.state = State::Over;
        g.elapsed_ms = 24_830;
        g.rank = Rank::from_cs(g.time_cs());
        g.new_best = true;
        let mut fb = Fb::new(&mut px);
        render(&mut fb, &g, &mut scratch);
        assert!(fb.rects <= 1050, "Game Over rects={}", fb.rects);
    }
}

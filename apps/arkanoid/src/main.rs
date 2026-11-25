#![cfg_attr(any(target_os = "none", wp2_sim), no_main, cfg_attr(target_os = "none", no_std))]

#[cfg_attr(not(any(target_os = "none", wp2_sim)), allow(dead_code))]
mod game;
#[cfg_attr(not(any(target_os = "none", wp2_sim)), allow(dead_code))]
mod levels;
#[cfg_attr(not(any(target_os = "none", wp2_sim)), allow(dead_code))]
mod view;

#[cfg(not(any(target_os = "none", wp2_sim)))]
fn main() {}

#[cfg(any(target_os = "none", wp2_sim))]
mod device {
    use crate::game::{EventFx, Game, SAVE_LEN, SaveData, State, decode_save, encode_save};
    use crate::view::{ViewState, render_frame};
    use cmf_wp2_app::{Api, BUTTON, CROWN, PAUSE, RESUME, TOUCH_DOWN, TOUCH_UP};
    use cmf_wp2_ui::haptic::{DONE, DOUBLE, Motor, Pattern, TICK};
    use core::ptr::addr_of_mut;

    cmf_wp2_app::entry!(main);
    cmf_wp2_app::app_icon!();

    /// ~55 FPS smooth arcade frame cadence.
    const FRAME_MS: u32 = 18;
    const CATCH_UP: u32 = 2;
    const SCREEN_LOCK_MS: u32 = 500;

    // Distinct arcade haptic feedback patterns (all compile-time validated)
    const WALL_FX: Pattern = Pattern::new(&[(180, 12), (0, 0)]);
    const BRICK_FX: Pattern = Pattern::new(&[(320, 16), (0, 0)]);
    const LASER_FX: Pattern = Pattern::new(&[(200, 10), (0, 0)]);
    const SHIELD_FX: Pattern = Pattern::new(&[(480, 26), (0, 0)]);
    const POWER_FX: Pattern = Pattern::new(&[(360, 22), (0, 45), (560, 30), (0, 0)]);
    const TNT_FX: Pattern = Pattern::new(&[(780, 40), (0, 45), (900, 55), (0, 0)]);
    const LOSE_FX: Pattern = Pattern::new(&[(520, 90), (0, 0)]);
    const CLEAR_FX: Pattern =
        Pattern::new(&[(350, 25), (0, 50), (520, 30), (0, 50), (750, 45), (0, 0)]);

    // Kept in static BSS to respect the watch's strict 8 KiB stack limit!
    static mut GAME: Game = Game::new_const();
    static mut VIEW: ViewState = ViewState::new_const();

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
        api.now().wrapping_mul(2_654_435_761) ^ t ^ 0x4152_4B31
    }

    fn persist_if_changed(api: &Api, current: SaveData, saved: &mut SaveData) {
        if current != *saved {
            api.save(&encode_save(current));
            *saved = current;
        }
    }

    fn play_fx(motor: &mut Motor, fx: EventFx) {
        match fx {
            EventFx::None => {}
            EventFx::Wall => motor.play(WALL_FX),
            EventFx::Paddle => motor.play(TICK),
            EventFx::Laser => motor.play(LASER_FX),
            EventFx::Brick => motor.play(BRICK_FX),
            EventFx::Shield => motor.play(SHIELD_FX),
            EventFx::Powerup => motor.play(POWER_FX),
            EventFx::Explosion => motor.play(TNT_FX),
            EventFx::LoseLife => motor.play(LOSE_FX),
            EventFx::StageClear => motor.play(CLEAR_FX),
            EventFx::GameOver => motor.play(DONE),
        }
    }

    fn main(api: &'static Api) -> i32 {
        let mut screen = api;
        let mut motor = Motor::new(api);

        let mut blob = [0u8; SAVE_LEN];
        let n = api.load(&mut blob);
        let loaded = if n >= SAVE_LEN as i32 {
            decode_save(&blob).unwrap_or(SaveData::DEFAULT)
        } else {
            SaveData::DEFAULT
        };
        let mut saved = loaded;

        // Safe single-threaded access to static Game & ViewState
        let game = unsafe { &mut *addr_of_mut!(GAME) };
        let view = unsafe { &mut *addr_of_mut!(VIEW) };

        game.init(seed(api), loaded);
        view.invalidate();
        render_frame(&mut screen, game, view);
        api.show();

        let mut next = api.now().wrapping_add(FRAME_MS);
        let mut lock_until = 0u32;

        loop {
            motor.update();
            let now = api.now();
            let animating = game.state == State::Playing;
            let left = if !animating {
                u32::MAX
            } else {
                (next.wrapping_sub(now) as i32).max(0) as u32
            };
            let wait = motor.wait().map_or(left, |m| m.min(left));

            // Drain ALL queued events so rapid crown turns never starve frames!
            let mut crown_acc: i32 = 0;
            let mut tap_pos: Option<(i32, i32)> = None;
            let mut got_touch_up = false;
            let mut resumed = false;

            if let Some(first) = api.event(wait) {
                let mut ev = Some(first);
                while let Some(e) = ev {
                    match e.kind {
                        BUTTON => {
                            motor.off();
                            persist_if_changed(api, game.save, &mut saved);
                            api.keep_screen_on(false);
                            return 0;
                        }
                        PAUSE => {
                            motor.off();
                            persist_if_changed(api, game.save, &mut saved);
                            api.keep_screen_on(false);
                            game.touch_up();
                            if !wait_while_paused(api) {
                                return 0;
                            }
                            resumed = true;
                            crown_acc = 0;
                            tap_pos = None;
                            got_touch_up = false;
                            break;
                        }
                        CROWN => {
                            crown_acc += e.delta as i32;
                        }
                        TOUCH_DOWN => {
                            tap_pos = Some((e.x as i32, e.y as i32));
                            got_touch_up = false;
                        }
                        TOUCH_UP => {
                            got_touch_up = true;
                        }
                        _ => {}
                    }
                    ev = api.event(0);
                }
            }

            if resumed {
                view.invalidate();
                render_frame(&mut screen, game, view);
                api.show();
                api.keep_screen_on(game.state == State::Playing);
                next = api.now().wrapping_add(FRAME_MS);
                continue;
            }

            if crown_acc != 0 {
                let before = game.selected_level;
                game.apply_crown(crown_acc);
                if game.state == State::Title && game.selected_level != before {
                    motor.play(TICK);
                    render_frame(&mut screen, game, view);
                    api.show();
                }
            }

            if let Some((tx, ty)) = tap_pos {
                let tnow = api.now();
                match game.state {
                    State::Title => {
                        // Left / Right level selector arrows on Title screen
                        if game.save.max_level > 1 && ty >= 220 && ty <= 284 && tx < 145 {
                            if game.select_level_delta(-1) {
                                motor.play(TICK);
                                render_frame(&mut screen, game, view);
                                api.show();
                            }
                        } else if game.save.max_level > 1 && ty >= 220 && ty <= 284 && tx > 321 {
                            if game.select_level_delta(1) {
                                motor.play(TICK);
                                render_frame(&mut screen, game, view);
                                api.show();
                            }
                        } else {
                            game.start_new_run();
                            motor.play(DOUBLE);
                            render_frame(&mut screen, game, view);
                            api.show();
                            api.keep_screen_on(true);
                            next = tnow.wrapping_add(FRAME_MS);
                        }
                        continue;
                    }
                    State::StageClear => {
                        if due(tnow, lock_until) {
                            game.next_stage();
                            motor.play(TICK);
                            render_frame(&mut screen, game, view);
                            api.show();
                            api.keep_screen_on(true);
                            next = tnow.wrapping_add(FRAME_MS);
                        }
                        continue;
                    }
                    State::GameOver => {
                        if due(tnow, lock_until) {
                            game.start_new_run();
                            motor.play(TICK);
                            render_frame(&mut screen, game, view);
                            api.show();
                            api.keep_screen_on(true);
                            next = tnow.wrapping_add(FRAME_MS);
                        }
                        continue;
                    }
                    State::Playing => {
                        let fx = game.touch_down(tx);
                        play_fx(&mut motor, fx);
                    }
                }
            }

            if got_touch_up {
                game.touch_up();
            }

            if game.state != State::Playing {
                continue;
            }

            let now = api.now();
            if due(now, next) {
                let mut steps = 0u32;
                let mut best_fx = EventFx::None;
                while steps < CATCH_UP && due(now, next) {
                    next = next.wrapping_add(FRAME_MS);
                    steps += 1;
                    let fx = game.step();
                    if (fx as u8) > (best_fx as u8) {
                        best_fx = fx;
                    }
                    if game.state != State::Playing {
                        lock_until = now.wrapping_add(SCREEN_LOCK_MS);
                        persist_if_changed(api, game.save, &mut saved);
                        if game.state == State::GameOver {
                            api.keep_screen_on(false);
                        }
                        break;
                    }
                }
                if due(now, next) {
                    next = now.wrapping_add(FRAME_MS);
                }
                play_fx(&mut motor, best_fx);
                render_frame(&mut screen, game, view);
                api.show();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::game::{
        BALL_R, C, EventFx, Game, MAX_BALLS, PADDLE_Y, PLAY_B, PLAY_L, PLAY_R, PLAY_T, PowerKind,
        Q, SAVE_LEN, SaveData, SHIELD_Y, SIZE, State, brick_rect, decode_save, encode_save,
    };
    use crate::levels::{Brick, BrickKind, COLS, GRID_LEN, LEVELS, NUM_LEVELS, ROWS, load_level};
    use crate::view::{ViewState, draw_full, render_frame};
    use cmf_wp2_ui::Fb;

    #[test]
    fn all_12_levels_have_breakable_bricks_and_fit_round_screen() {
        assert_eq!(NUM_LEVELS, 12);
        let r_sq = C * C;
        for lvl in 0..NUM_LEVELS {
            let mut bricks = [Brick::EMPTY; GRID_LEN];
            let breakable = load_level(lvl, 0, &mut bricks);
            assert!(
                breakable >= 12,
                "level {} ({}) has too few breakable bricks: {breakable}",
                lvl + 1,
                LEVELS[lvl].name
            );
            for r in 0..ROWS {
                for c in 0..COLS {
                    if bricks[r * COLS + c].alive() {
                        let (x, y, w, h) = brick_rect(r, c);
                        assert!(x >= PLAY_L && x + w <= PLAY_R, "brick out of horizontal playfield");
                        assert!(y >= PLAY_T && y + h <= PLAY_B, "brick out of vertical playfield");
                        // Every corner of every brick lies strictly inside the 466px circular glass!
                        for &(cx, cy) in &[(x, y), (x + w, y), (x, y + h), (x + w, y + h)] {
                            let dx = cx - C;
                            let dy = cy - C;
                            assert!(
                                dx * dx + dy * dy <= r_sq,
                                "level {} brick ({r},{c}) corner ({cx},{cy}) outside circle!",
                                lvl + 1
                            );
                        }
                    }
                }
            }
            // Wave 2 escalation still produces breakable bricks
            let breakable_w2 = load_level(lvl, 1, &mut bricks);
            assert_eq!(breakable_w2, breakable);
        }
    }

    #[test]
    fn ball_reflection_wall_paddle_and_anti_stuck() {
        let mut g = Game::new_const();
        g.init(42, SaveData::DEFAULT);
        g.start_new_run();
        assert!(g.launch_ball());

        // Test left wall reflection
        g.balls[0].x_q = (PLAY_L + BALL_R + 1) * Q;
        g.balls[0].y_q = 300 * Q;
        g.balls[0].vx_q = -1100;
        g.balls[0].vy_q = -400;
        let fx = g.step();
        assert_eq!(fx, EventFx::Wall);
        assert!(g.balls[0].vx_q > 0, "ball must bounce right off left wall");

        // Test paddle angled reflection (left edge vs right edge)
        g.paddle_x = C;
        g.balls[0].x_q = (C - 26) * Q;
        g.balls[0].y_q = (PADDLE_Y - BALL_R - 1) * Q;
        g.balls[0].vx_q = 0;
        g.balls[0].vy_q = 1100;
        let fx = g.step();
        assert_eq!(fx, EventFx::Paddle);
        assert!(g.balls[0].vy_q < 0, "ball must bounce upward off paddle");
        assert!(g.balls[0].vx_q < 0, "left side of paddle must angle ball left");

        // Test anti-stuck horizontal bounce guard
        g.balls[0].x_q = C * Q;
        g.balls[0].y_q = 300 * Q;
        g.balls[0].vx_q = 1040;
        g.balls[0].vy_q = 5; // nearly flat!
        let _ = g.step();
        assert!(
            g.balls[0].vy_q.abs() >= g.target_speed_q() / 4,
            "anti-stuck must keep |vy| >= spd/4"
        );
    }

    #[test]
    fn tnt_chain_reaction_destroys_neighbors_and_chains() {
        let mut g = Game::new_const();
        g.init(99, SaveData::DEFAULT);
        g.start_new_run();
        g.bricks = [Brick::EMPTY; GRID_LEN];
        // Build a 3x4 cluster with two adjacent TNT bricks and a 3-HP Titanium neighbor + Gold Bumper
        g.bricks[2 * COLS + 3] = Brick::new(BrickKind::Tnt);
        g.bricks[2 * COLS + 4] = Brick::new(BrickKind::Tnt);
        g.bricks[1 * COLS + 3] = Brick::new(BrickKind::Titanium); // 3 HP -> obliterated by blast!
        g.bricks[2 * COLS + 5] = Brick::new(BrickKind::Silver); // adjacent to 2nd TNT -> chained!
        g.bricks[3 * COLS + 3] = Brick::new(BrickKind::Bumper); // indestructible -> survives!
        g.bricks_left = 4;

        let fx = g.damage_brick(2, 3, 1, false);
        assert_eq!(fx, EventFx::Explosion);
        assert!(!g.bricks[2 * COLS + 3].alive());
        assert!(!g.bricks[2 * COLS + 4].alive(), "second TNT must chain-detonate");
        assert!(!g.bricks[1 * COLS + 3].alive(), "Titanium neighbor must be destroyed by TNT");
        assert!(!g.bricks[2 * COLS + 5].alive(), "Silver neighbor of chained TNT must be destroyed");
        assert!(g.bricks[3 * COLS + 3].alive(), "Gold Bumper must survive explosion");
        assert_eq!(g.bricks_left, 0);
    }

    #[test]
    fn fireball_passes_through_bricks_without_bouncing() {
        let mut g = Game::new_const();
        g.init(11, SaveData::DEFAULT);
        g.start_new_run();
        g.bricks = [Brick::EMPTY; GRID_LEN];
        g.bricks[4 * COLS + 4] = Brick::new(BrickKind::Titanium); // 3 HP
        g.bricks[0 * COLS + 0] = Brick::new(BrickKind::Crimson); // keep stage alive
        g.bricks_left = 2;

        g.apply_powerup(PowerKind::Fire);
        let (rx, ry, rw, rh) = brick_rect(4, 4);
        g.balls[0].glued = false;
        g.balls[0].x_q = (rx + rw / 2) * Q;
        g.balls[0].y_q = (ry + rh + BALL_R + 1) * Q;
        g.balls[0].vx_q = 0;
        g.balls[0].vy_q = -1100;

        let _ = g.step();
        assert!(!g.bricks[4 * COLS + 4].alive(), "Fireball must obliterate 3-HP Titanium in one pass");
        assert!(g.balls[0].vy_q < 0, "Fireball must continue upward without reflecting!");
    }

    #[test]
    fn multiball_splits_up_to_8_balls() {
        let mut g = Game::new_const();
        g.init(77, SaveData::DEFAULT);
        g.start_new_run();
        assert_eq!(g.active_ball_count(), 1);

        g.apply_powerup(PowerKind::Multi);
        assert_eq!(g.active_ball_count(), 3, "1 ball splits to 3");

        g.apply_powerup(PowerKind::Multi);
        assert_eq!(g.active_ball_count(), MAX_BALLS, "second split caps cleanly at 8 balls");
    }

    #[test]
    fn laser_damages_and_destroys_bricks() {
        let mut g = Game::new_const();
        g.init(55, SaveData::DEFAULT);
        g.start_new_run();
        g.bricks = [Brick::EMPTY; GRID_LEN];
        g.bricks[7 * COLS + 4] = Brick::new(BrickKind::Silver); // 2 HP
        g.bricks[0 * COLS + 0] = Brick::new(BrickKind::Crimson);
        g.bricks_left = 2;

        let (rx, ry, rw, rh) = brick_rect(7, 4);
        g.apply_powerup(PowerKind::Laser);
        // Place two laser bolts right below the Silver brick
        g.lasers[0] = crate::game::Laser {
            active: true,
            x: (rx + rw / 2) as i16,
            y: (ry + rh + 4) as i16,
        };
        g.lasers[1] = crate::game::Laser {
            active: true,
            x: (rx + rw / 2) as i16,
            y: (ry + rh + 14) as i16,
        };
        g.balls[0].glued = true; // keep ball parked

        let _ = g.step();
        assert_eq!(g.bricks[7 * COLS + 4].hp, 1, "first laser chips Silver to 1 HP");
        let _ = g.step();
        assert!(!g.bricks[7 * COLS + 4].alive(), "second laser destroys Silver brick");
    }

    #[test]
    fn shield_bounces_missed_ball_and_consumes_hits() {
        let mut g = Game::new_const();
        g.init(33, SaveData::DEFAULT);
        g.start_new_run();
        g.apply_powerup(PowerKind::Shield);
        assert_eq!(g.shield_hits, 2);

        g.paddle_x = PLAY_L + 40; // move paddle away from center
        g.balls[0].glued = false;
        g.balls[0].x_q = C * Q;
        g.balls[0].y_q = (SHIELD_Y - BALL_R - 2) * Q;
        g.balls[0].vx_q = 200;
        g.balls[0].vy_q = 1100;

        let fx = g.step();
        assert_eq!(fx, EventFx::Shield);
        assert!(g.balls[0].vy_q < 0, "shield must bounce ball upward");
        assert_eq!(g.shield_hits, 1, "shield absorbs 1 hit and keeps 1 left");
    }

    #[test]
    fn save_load_roundtrip_and_corruption_rejection() {
        let original = SaveData {
            best_score: 48_920,
            max_level: 9,
            best_wave: 2,
        };
        let encoded = encode_save(original);
        assert_eq!(encoded.len(), SAVE_LEN);
        assert_eq!(&encoded[0..4], b"ARK1");
        assert_eq!(decode_save(&encoded), Some(original));

        // Corrupt checksum
        let mut bad_csum = encoded;
        bad_csum[15] ^= 0x01;
        assert_eq!(decode_save(&bad_csum), None);

        // Corrupt magic
        let mut bad_magic = encoded;
        bad_magic[0] = b'X';
        assert_eq!(decode_save(&bad_magic), None);

        // Truncated slice
        assert_eq!(decode_save(&encoded[..10]), None);

        // Out-of-range level with forged checksum
        let mut bad_lvl = encoded;
        bad_lvl[8] = 99;
        assert_eq!(decode_save(&bad_lvl), None);
    }

    #[test]
    fn rendering_all_states_diff_accuracy_and_rect_budget() {
        const N: usize = (SIZE * SIZE) as usize;
        let mut px = vec![0u16; N];
        let mut ref_px = vec![0u16; N];

        let mut g = Game::new_const();
        let mut view = ViewState::new_const();
        g.init(2026, SaveData { best_score: 12340, max_level: 6, best_wave: 1 });

        // 1. Title screen + crown level selector differential update
        {
            let mut fb = Fb::new(&mut px);
            render_frame(&mut fb, &g, &mut view);
            assert!(fb.rects > 0);
        }
        g.apply_crown(1);
        {
            let mut fb = Fb::new(&mut px);
            render_frame(&mut fb, &g, &mut view);
            assert!(fb.rects < 80, "title level switch should be tiny: {} rects", fb.rects);
        }

        // 2. Gameplay chaos: Multi-ball (8 balls) + Lasers + Fireball + Shield + Capsules + Particles across 30 ticks!
        g.selected_level = 1;
        g.start_new_run();
        g.apply_powerup(PowerKind::Wide);
        g.apply_powerup(PowerKind::Shield);
        g.apply_powerup(PowerKind::Laser);
        g.apply_powerup(PowerKind::Multi);
        g.apply_powerup(PowerKind::Multi); // 8 balls!
        g.apply_powerup(PowerKind::Fire);
        g.fire_lasers();
        // Also drop 3 falling capsules simultaneously
        g.capsules[0] = crate::game::Capsule { active: true, kind: PowerKind::Slow, x: 140, y_q: 260 * Q };
        g.capsules[1] = crate::game::Capsule { active: true, kind: PowerKind::Life, x: 230, y_q: 280 * Q };
        g.capsules[2] = crate::game::Capsule { active: true, kind: PowerKind::Multi, x: 320, y_q: 240 * Q };

        // Initial full gameplay paint
        {
            let mut fb = Fb::new(&mut px);
            render_frame(&mut fb, &g, &mut view);
        }

        let mut max_rects = 0u32;
        let mut sum_rects = 0u64;
        let mut frames = 0u64;

        for t in 0..30 {
            if t % 4 == 0 {
                g.apply_crown(1);
            } else if t % 4 == 2 {
                g.apply_crown(-1);
            }
            if t == 10 {
                g.fire_lasers();
            }
            let _ = g.step();
            let mut fb = Fb::new(&mut px);
            render_frame(&mut fb, &g, &mut view);

            if g.state == State::Playing {
                max_rects = max_rects.max(fb.rects);
                sum_rects += fb.rects as u64;
                frames += 1;
                assert!(
                    fb.rects <= 750,
                    "tick {t} exceeded rect budget: {} rects",
                    fb.rects
                );
                // Verify differential framebuffer matches full redraw pixel-for-pixel!
                ref_px.fill(0);
                let mut ref_fb = Fb::new(&mut ref_px);
                draw_full(&mut ref_fb, &g);
                let mut mismatches = 0usize;
                let mut first_xy = (0i32, 0i32);
                for (idx, (&a, &b)) in px.iter().zip(ref_px.iter()).enumerate() {
                    if a != b {
                        if mismatches == 0 {
                            first_xy = ((idx as i32) % SIZE, (idx as i32) / SIZE);
                        }
                        mismatches += 1;
                    }
                }
                assert_eq!(
                    mismatches, 0,
                    "differential mismatch at tick {t} (first at {:?})",
                    first_xy
                );
            }
        }
        assert!(frames > 0);
        let avg = sum_rects / frames;
        std::println!(
            "Heavy 8-ball+laser+fire+shield+capsule gameplay ({frames} ticks): avg={avg} rects/frame, max={max_rects} rects/frame"
        );

        // 3. Stage Clear screen
        g.state = State::StageClear;
        g.stage_bonus = 1150;
        {
            let mut fb = Fb::new(&mut px);
            render_frame(&mut fb, &g, &mut view);
            assert!(fb.rects > 0);
        }

        // 4. Game Over screen
        g.state = State::GameOver;
        g.beat_best = true;
        {
            let mut fb = Fb::new(&mut px);
            render_frame(&mut fb, &g, &mut view);
            assert!(fb.rects > 0);
        }
    }
}

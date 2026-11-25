#![cfg_attr(any(target_os = "none", wp2_sim), no_main, cfg_attr(target_os = "none", no_std))]

#[cfg_attr(not(any(target_os = "none", wp2_sim)), allow(dead_code))]
mod game;
#[cfg_attr(not(any(target_os = "none", wp2_sim)), allow(dead_code))]
mod knives;
#[cfg_attr(not(any(target_os = "none", wp2_sim)), allow(dead_code))]
mod view;

#[cfg(not(any(target_os = "none", wp2_sim)))]
fn main() {}

#[cfg(any(target_os = "none", wp2_sim))]
mod device {
    use crate::game::{Game, SAVE_LEN, SaveData, State, StepEvent, decode_save, encode_save};
    use crate::view::{
        BTN_KNIVES_OVER, BTN_KNIVES_TITLE, BTN_SHOP_ACT, BTN_SHOP_BACK, BTN_SHOP_NEXT,
        BTN_SHOP_PREV, ViewState, frame, hit_rect,
    };
    use cmf_wp2_app::{Api, BUTTON, CROWN, PAUSE, RESUME, TOUCH_DOWN};
    use cmf_wp2_ui::haptic::{Motor, Pattern, TICK};
    use core::ptr::addr_of_mut;

    cmf_wp2_app::entry!(main);
    cmf_wp2_app::app_icon!();

    /// ~55 FPS smooth gameplay cadence on the 60 Hz AMOLED panel.
    const FRAME_MS: u32 = 18;
    const CATCH_UP: u32 = 2;
    const RETRY_MS: u32 = 450;

    // Juicy custom Knife Hit haptic patterns (compile-time verified under 2s motor cap)
    const H_THUD: Pattern = Pattern::new(&[(420, 22), (0, 0)]);
    const H_SLICE: Pattern = Pattern::new(&[(360, 18), (0, 36), (520, 22), (0, 0)]);
    const H_CLANG: Pattern = Pattern::new(&[(880, 55), (0, 45), (560, 85), (0, 0)]);
    const H_SHATTER: Pattern = Pattern::new(&[(550, 30), (0, 35), (820, 45), (0, 0)]);
    const H_BOSS: Pattern =
        Pattern::new(&[(600, 28), (0, 38), (820, 35), (0, 38), (1000, 65), (0, 0)]);

    // Keep Game & ViewState off the 8 KiB stack!
    static mut GAME: Game = Game::new_const();
    static mut VIEW: ViewState = ViewState::new();

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

    fn flush_save(api: &Api, g: &mut Game) {
        if g.save_dirty {
            api.save(&encode_save(&g.save));
            g.save_dirty = false;
        }
    }

    #[inline]
    fn is_active_play(state: State) -> bool {
        matches!(state, State::Playing | State::Shatter | State::Bounce)
    }

    fn main(api: &'static Api) -> i32 {
        let mut screen = api;
        let mut motor = Motor::new(api);

        let mut blob = [0u8; 32];
        let n = api.load(&mut blob);
        let loaded = if n as usize >= SAVE_LEN {
            decode_save(&blob[..n as usize]).unwrap_or_else(SaveData::default_new)
        } else {
            SaveData::default_new()
        };

        // SAFETY: single-threaded entry point; exclusive references live for duration of `main`.
        let g: &mut Game = unsafe { &mut *addr_of_mut!(GAME) };
        let v: &mut ViewState = unsafe { &mut *addr_of_mut!(VIEW) };

        g.init(seed(api), loaded);
        v.invalidate();
        frame(&mut screen, g, v);
        api.show();

        let mut next = api.now().wrapping_add(FRAME_MS);
        let mut over_at = 0u32;

        loop {
            motor.update();
            let now = api.now();
            let animated = matches!(
                g.state,
                State::Title | State::Playing | State::Shatter | State::Bounce
            );
            let left = if !animated {
                u32::MAX
            } else {
                (next.wrapping_sub(now) as i32).max(0) as u32
            };
            let wait = motor.wait().map_or(left, |m| m.min(left));

            // Drain ALL queued events in one batch so fast crown spins never starve rendering!
            let mut crown_acc: i32 = 0;
            let mut tap: Option<(i32, i32)> = None;
            let mut resumed = false;

            if let Some(first) = api.event(wait) {
                let mut ev = Some(first);
                while let Some(e) = ev {
                    match e.kind {
                        BUTTON => {
                            motor.off();
                            flush_save(api, g);
                            api.keep_screen_on(false);
                            return 0;
                        }
                        PAUSE => {
                            motor.off();
                            flush_save(api, g);
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
                            tap = Some((e.x as i32, e.y as i32));
                        }
                        _ => {}
                    }
                    ev = api.event(0);
                }
            }

            if resumed {
                v.invalidate();
                frame(&mut screen, g, v);
                api.show();
                api.keep_screen_on(is_active_play(g.state));
                next = api.now().wrapping_add(FRAME_MS);
                continue;
            }

            let mut need_draw = false;

            // Handle crown input
            if crown_acc != 0 {
                match g.state {
                    State::Shop => {
                        g.shop_step(crown_acc);
                        motor.play(TICK);
                        need_draw = true;
                    }
                    State::Playing => {
                        g.request_throw();
                    }
                    _ => {}
                }
            }

            // Handle tap input
            if let Some((tx, ty)) = tap {
                let tnow = api.now();
                match g.state {
                    State::Playing => {
                        g.request_throw();
                    }
                    State::Title => {
                        if hit_rect(BTN_KNIVES_TITLE, tx, ty) || ty >= BTN_KNIVES_TITLE.1 {
                            g.open_shop(State::Title);
                            motor.play(TICK);
                        } else {
                            g.start_run();
                            api.keep_screen_on(true);
                            motor.play(TICK);
                            next = tnow.wrapping_add(FRAME_MS);
                        }
                        need_draw = true;
                    }
                    State::Shop => {
                        if hit_rect(BTN_SHOP_BACK, tx, ty) || ty >= BTN_SHOP_BACK.1 {
                            g.close_shop();
                            motor.play(TICK);
                        } else if hit_rect(BTN_SHOP_PREV, tx, ty) || (tx < 120 && ty < 300) {
                            g.shop_step(-1);
                            motor.play(TICK);
                        } else if hit_rect(BTN_SHOP_NEXT, tx, ty) || (tx > 346 && ty < 300) {
                            g.shop_step(1);
                            motor.play(TICK);
                        } else if hit_rect(BTN_SHOP_ACT, tx, ty) || (ty >= 72 && ty < 362) {
                            match g.shop_activate() {
                                2 => {
                                    motor.play(H_BOSS);
                                    flush_save(api, g);
                                }
                                1 => {
                                    motor.play(TICK);
                                    flush_save(api, g);
                                }
                                _ => {
                                    motor.play(H_CLANG);
                                }
                            }
                        }
                        need_draw = true;
                    }
                    State::Over => {
                        if due(tnow, over_at.wrapping_add(RETRY_MS)) {
                            if hit_rect(BTN_KNIVES_OVER, tx, ty) || ty >= BTN_KNIVES_OVER.1 {
                                g.open_shop(State::Over);
                                motor.play(TICK);
                            } else {
                                g.start_run();
                                api.keep_screen_on(true);
                                motor.play(TICK);
                                next = tnow.wrapping_add(FRAME_MS);
                            }
                            need_draw = true;
                        }
                    }
                    State::Shatter | State::Bounce => {}
                }
            }

            // Advance animation/physics ticks
            let now = api.now();
            let tick_due = animated && due(now, next);
            if tick_due {
                let mut steps = 0u32;
                while steps < CATCH_UP && due(now, next) {
                    next = next.wrapping_add(FRAME_MS);
                    steps += 1;
                    match g.step() {
                        StepEvent::None => {}
                        StepEvent::KnifeHit => motor.play(H_THUD),
                        StepEvent::AppleSliced => {
                            motor.play(H_SLICE);
                            flush_save(api, g);
                        }
                        StepEvent::StageCleared => {
                            motor.play(H_SHATTER);
                            flush_save(api, g);
                        }
                        StepEvent::BossCleared => {
                            motor.play(H_BOSS);
                            flush_save(api, g);
                        }
                        StepEvent::KnifeBounced => {
                            motor.play(H_CLANG);
                            flush_save(api, g);
                        }
                        StepEvent::GameOver => {
                            over_at = now;
                            flush_save(api, g);
                            api.keep_screen_on(false);
                            break;
                        }
                    }
                }
                if due(now, next) {
                    next = now.wrapping_add(FRAME_MS);
                }
                need_draw = true;
            }

            if need_draw {
                frame(&mut screen, g, v);
                api.show();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::game::{
        APPLE_HIT_ARC, BOTTOM_ANGLE, BossKind, Game, KNIFE_HIT_ARC, MIN_APPLE_KNIFE_GAP,
        MIN_OBSTACLE_GAP, SAVE_LEN, SaveData, State, StepEvent, angle_diff, decode_save,
        encode_save,
    };
    use crate::knives::{KNIVES, NUM_KNIVES};
    use crate::view::{ViewState, frame};
    use cmf_wp2_ui::{Fb, SIZE};

    const N: usize = (SIZE * SIZE) as usize;

    #[test]
    fn procedural_stage_generation_stages_1_to_50() {
        let mut g = Game::new_const();
        g.init(1337, SaveData::default_new());

        let mut seen_bosses = [false; 8];
        for stage in 1..=50u16 {
            g.generate_stage(stage);
            assert_eq!(g.stage, stage);
            assert!(
                (6..=12).contains(&g.knives_total),
                "stage {stage}: invalid knives_total {}",
                g.knives_total
            );
            assert_eq!(g.knives_left, g.knives_total);

            if stage % 5 == 0 {
                assert_ne!(g.boss, BossKind::None, "stage {stage} must be a boss");
                let idx = ((stage / 5 - 1) % 8) as usize;
                seen_bosses[idx] = true;
            } else {
                assert_eq!(g.boss, BossKind::None, "stage {stage} must not be a boss");
            }

            let obs: Vec<u16> = g.stuck.iter().filter(|k| k.active).map(|k| k.rel_angle).collect();
            let apps: Vec<u16> = g.apples.iter().filter(|a| a.active).map(|a| a.rel_angle).collect();

            if stage == 1 {
                assert_eq!(obs.len(), 0, "stage 1 should have 0 obstacle knives");
                assert_eq!(apps.len(), 1, "stage 1 should have 1 apple");
            } else {
                assert!(obs.len() <= 4, "stage {stage}: too many obstacles {}", obs.len());
            }

            // Guaranteed safe pairwise gaps between pre-stuck obstacle knives
            for i in 0..obs.len() {
                for j in (i + 1)..obs.len() {
                    let d = angle_diff(obs[i], obs[j]);
                    assert!(
                        d >= MIN_OBSTACLE_GAP,
                        "stage {stage}: obstacle gap too small ({d} < {MIN_OBSTACLE_GAP})"
                    );
                }
            }

            // Apples placed safely away from obstacle knives
            for &a in &apps {
                for &k in &obs {
                    let d = angle_diff(a, k);
                    assert!(
                        d >= MIN_APPLE_KNIFE_GAP,
                        "stage {stage}: apple too close to obstacle ({d} < {MIN_APPLE_KNIFE_GAP})"
                    );
                }
            }
        }
        assert!(seen_bosses.iter().all(|&b| b), "all 8 boss kinds should appear in stages 1..=50");
    }

    #[test]
    fn throw_collision_stick_bounce_and_apple_slice() {
        let mut g = Game::new_const();
        g.init(42, SaveData::default_new());
        g.start_run();

        // Force stage 1 stationary target with 1 obstacle at rel=0 and 1 apple at BOTTOM_ANGLE
        g.base_speed = 0;
        g.target_angle = 0;
        g.stuck[0] = crate::game::StuckKnife { active: true, rel_angle: 0, skin: 0 };
        g.apples[0] = crate::game::RimApple { active: true, rel_angle: BOTTOM_ANGLE };
        g.knives_left = 2;

        // 1. Throw at BOTTOM_ANGLE -> slices apple AND embeds in wood!
        g.request_throw();
        let mut got_apple = false;
        let mut landed = false;
        for _ in 0..20 {
            match g.step() {
                StepEvent::AppleSliced => got_apple = true,
                StepEvent::KnifeHit => {
                    landed = true;
                    break;
                }
                _ => {}
            }
            if !g.flying {
                landed = true;
                break;
            }
        }
        assert!(got_apple, "apple should have been sliced");
        assert!(landed, "knife should have embedded");
        assert_eq!(g.save.apples, 1);
        assert_eq!(g.score, 1);
        assert_eq!(g.knives_left, 1);
        assert!(!g.apples[0].active);
        assert!(g.recoil_y > 0);

        // 2. Throw again at the EXACT same angle -> must collide & bounce to Game Over!
        g.request_throw();
        let mut bounced = false;
        let mut game_over = false;
        for _ in 0..50 {
            match g.step() {
                StepEvent::KnifeBounced => bounced = true,
                StepEvent::GameOver => {
                    game_over = true;
                    break;
                }
                _ => {}
            }
        }
        assert!(bounced, "throwing at an occupied angle must bounce");
        assert!(game_over, "bounce must transition to Game Over");
        assert_eq!(g.state, State::Over);
    }

    #[test]
    fn boss_defeat_awards_bonus_and_unlocks_boss_knives() {
        let mut g = Game::new_const();
        g.init(99, SaveData::default_new());
        g.start_run();

        // Jump to Boss #1 (Stage 5) with 1 knife left and clear path
        g.generate_stage(5);
        assert_eq!(g.boss, BossKind::Cheese);
        g.base_speed = 0;
        g.target_angle = 0;
        g.stuck = [crate::game::StuckKnife::EMPTY; crate::game::MAX_STUCK];
        g.apples = [crate::game::RimApple::EMPTY; crate::game::MAX_APPLES];
        g.knives_left = 1;

        g.request_throw();
        let mut boss_cleared = false;
        for _ in 0..15 {
            if g.step() == StepEvent::BossCleared {
                boss_cleared = true;
                break;
            }
        }
        assert!(boss_cleared);
        assert_eq!(g.state, State::Shatter);
        assert_eq!(g.save.apples, 5, "boss clear awards +5 apples");
        assert_eq!(g.save.bosses_beaten, 1);
        assert!(g.save.is_unlocked(14), "Boss #1 unlocks SCYTHE (id 14)");
        assert_eq!(g.unlock_banner_knife, Some(14));

        // Let shatter finish -> advances to Stage 6
        for _ in 0..45 {
            g.step();
        }
        assert_eq!(g.state, State::Playing);
        assert_eq!(g.stage, 6);

        // Beat Boss #2 (Stage 10) -> unlocks ROYAL (id 15)
        g.generate_stage(10);
        assert_eq!(g.boss, BossKind::Tomato);
        g.base_speed = 0;
        g.stuck = [crate::game::StuckKnife::EMPTY; crate::game::MAX_STUCK];
        g.apples = [crate::game::RimApple::EMPTY; crate::game::MAX_APPLES];
        g.knives_left = 1;
        g.request_throw();
        for _ in 0..15 {
            if g.step() == StepEvent::BossCleared {
                break;
            }
        }
        assert_eq!(g.save.bosses_beaten, 2);
        assert_eq!(g.save.apples, 10);
        assert!(g.save.is_unlocked(15), "Boss #2 unlocks ROYAL (id 15)");
    }

    #[test]
    fn knife_unlock_and_equip_all_16_knives() {
        let mut save = SaveData::default_new();
        assert_eq!(NUM_KNIVES, 16);
        assert!(save.is_unlocked(0));
        assert_eq!(save.unlocked_count(), 1);
        assert!(!save.is_unlocked(1));
        assert!(!save.buy_knife(1), "cannot buy without apples");
        assert!(!save.equip_knife(1), "cannot equip locked knife");

        // Give exact sum of all costs and buy + equip every single knife 1..16
        let total_cost: u32 = KNIVES.iter().map(|k| k.cost as u32).sum();
        save.apples = total_cost;
        for id in 1..NUM_KNIVES as u8 {
            assert!(save.can_afford(id), "should afford knife {id}");
            assert!(save.buy_knife(id), "buy knife {id} failed");
            assert!(save.is_unlocked(id));
            assert_eq!(save.selected_knife, id);
        }
        assert_eq!(save.apples, 0);
        assert_eq!(save.unlocked_count(), 16);

        // Equip back to CLASSIC (0) and verify
        assert!(save.equip_knife(0));
        assert_eq!(save.selected_knife, 0);
    }

    #[test]
    fn save_load_roundtrip_and_corruption_handling() {
        let orig = SaveData {
            best_stage: 27,
            best_score: 214,
            apples: 95,
            unlocked_mask: 0b1010_0101_0000_0011,
            selected_knife: 15,
            bosses_beaten: 5,
        };
        let blob = encode_save(&orig);
        assert_eq!(blob.len(), SAVE_LEN);
        assert_eq!(decode_save(&blob), Some(orig));

        // Corrupt magic
        let mut bad_magic = blob;
        bad_magic[0] = b'X';
        assert_eq!(decode_save(&bad_magic), None);

        // Corrupt payload byte (checksum must catch it!)
        let mut bad_data = blob;
        bad_data[10] ^= 0x01;
        assert_eq!(decode_save(&bad_data), None);

        // Truncated slice
        assert_eq!(decode_save(&blob[..10]), None);

        // Selected knife pointing to locked slot sanitizes to 0
        let weird = SaveData { selected_knife: 5, unlocked_mask: 1, ..orig };
        let b2 = encode_save(&weird);
        let decoded = decode_save(&b2).unwrap();
        assert_eq!(decoded.selected_knife, 0);
    }

    #[test]
    fn render_all_screens_and_bosses_within_rect_budget() {
        let mut px = vec![0u16; N];
        let mut g = Game::new_const();
        let mut v = ViewState::new();
        g.init(777, SaveData { apples: 200, unlocked_mask: 0xFFFF, selected_knife: 5, ..SaveData::default_new() });

        // 1. Title screen + 10 animated spinner frames
        let mut fb = Fb::new(&mut px);
        frame(&mut fb, &g, &mut v);
        assert!(fb.rects > 0);
        for _ in 0..10 {
            g.step();
            let mut fb = Fb::new(&mut px);
            frame(&mut fb, &g, &mut v);
            assert!(fb.rects <= 850, "Title diff frame too heavy: {} rects", fb.rects);
        }

        // 2. Shop / Arsenal screen browsing ALL 16 knives
        g.open_shop(State::Title);
        for id in 0..NUM_KNIVES as u8 {
            g.browse_knife = id;
            let mut fb = Fb::new(&mut px);
            frame(&mut fb, &g, &mut v);
            assert!(fb.rects > 0);
        }

        // 3. Gameplay: Normal stage + ALL 8 Boss stages across 20 ticks each!
        let test_stages = [1u16, 5, 10, 15, 20, 25, 30, 35, 40];
        let mut max_play_rects = 0u32;
        for (idx, &stage) in test_stages.iter().enumerate() {
            g.save.selected_knife = (idx * 2 % NUM_KNIVES) as u8;
            g.start_run();
            g.generate_stage(stage);
            // Initial full paint of the stage
            {
                let mut fb = Fb::new(&mut px);
                frame(&mut fb, &g, &mut v);
            }
            // Run 20 differential ticks including throws!
            for t in 0..20 {
                if t == 2 || t == 10 {
                    // Ensure safe throw angle so we test stick + recoil + splinters
                    let cur_rel = BOTTOM_ANGLE.wrapping_sub(g.target_angle);
                    let safe = g
                        .stuck
                        .iter()
                        .all(|k| !k.active || angle_diff(cur_rel, k.rel_angle) >= KNIFE_HIT_ARC + 400);
                    if safe {
                        g.request_throw();
                    }
                }
                g.step();
                if g.state != State::Playing {
                    break;
                }
                let mut fb = Fb::new(&mut px);
                frame(&mut fb, &g, &mut v);
                max_play_rects = max_play_rects.max(fb.rects);
                assert!(
                    fb.rects <= 1050,
                    "Stage {stage} tick {t} exceeded 1050 rect cap: {} rects",
                    fb.rects
                );
            }
        }
        // Also test worst-case crowded boss wheel (14 stuck knives + 3 apples + flying knife + recoil)
        g.start_run();
        g.generate_stage(35); // Saw boss (has outer teeth!)
        for i in 0..14 {
            g.stuck[i] = crate::game::StuckKnife {
                active: true,
                rel_angle: (i as u16).wrapping_mul(4400),
                skin: (i % NUM_KNIVES) as u8,
            };
        }
        g.apples[0] = crate::game::RimApple { active: true, rel_angle: 2200 };
        g.apples[1] = crate::game::RimApple { active: true, rel_angle: 24200 };
        g.apples[2] = crate::game::RimApple { active: true, rel_angle: 46200 };
        {
            let mut fb = Fb::new(&mut px);
            frame(&mut fb, &g, &mut v);
        }
        let mut worst_boss_rects = 0u32;
        for _ in 0..10 {
            g.step();
            let mut fb = Fb::new(&mut px);
            frame(&mut fb, &g, &mut v);
            worst_boss_rects = worst_boss_rects.max(fb.rects);
        }
        assert!(
            worst_boss_rects <= 850,
            "Crowded 14-knife Saw Boss frame exceeded 850 target: {worst_boss_rects} rects"
        );
        std::println!(
            "Peak gameplay differential rects: normal+bosses={max_play_rects}, 14-knife Saw Boss={worst_boss_rects}"
        );

        // 4. Shatter animation (20 ticks)
        g.start_run();
        g.generate_stage(5);
        g.base_speed = 0;
        g.stuck = [crate::game::StuckKnife::EMPTY; crate::game::MAX_STUCK];
        g.knives_left = 1;
        {
            let mut fb = Fb::new(&mut px);
            frame(&mut fb, &g, &mut v);
        }
        g.request_throw();
        for _ in 0..25 {
            g.step();
            let mut fb = Fb::new(&mut px);
            frame(&mut fb, &g, &mut v);
            assert!(fb.rects <= 1050, "Shatter frame too heavy: {} rects", fb.rects);
        }

        // 5. Bounce + Game Over screen
        g.start_run();
        g.base_speed = 0;
        g.target_angle = 0;
        g.stuck[0] = crate::game::StuckKnife { active: true, rel_angle: BOTTOM_ANGLE, skin: 0 };
        {
            let mut fb = Fb::new(&mut px);
            frame(&mut fb, &g, &mut v);
        }
        g.request_throw();
        for _ in 0..40 {
            g.step();
            let mut fb = Fb::new(&mut px);
            frame(&mut fb, &g, &mut v);
            if g.state == State::Over {
                break;
            }
            assert!(fb.rects <= 1050, "Bounce frame too heavy: {} rects", fb.rects);
        }
        assert_eq!(g.state, State::Over);
    }

    #[test]
    fn angle_diff_handles_wraparound() {
        assert_eq!(angle_diff(0, 0), 0);
        assert_eq!(angle_diff(100, 65500), 136);
        assert_eq!(angle_diff(65500, 100), 136);
        assert_eq!(angle_diff(0, 32768), 32768);
        assert!(APPLE_HIT_ARC > KNIFE_HIT_ARC);
    }
}

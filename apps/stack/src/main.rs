#![cfg_attr(any(target_os = "none", wp2_sim), no_main, cfg_attr(target_os = "none", no_std))]

#[cfg_attr(not(any(target_os = "none", wp2_sim)), allow(dead_code))]
mod game;
#[cfg_attr(not(any(target_os = "none", wp2_sim)), allow(dead_code))]
mod view;

#[cfg(not(any(target_os = "none", wp2_sim)))]
fn main() {}

#[cfg(any(target_os = "none", wp2_sim))]
mod device {
    use crate::game::{DropResult, Game, State, decode_best, encode_best};
    use crate::view::frame;
    use cmf_wp2_app::{Api, BUTTON, PAUSE, RESUME, TOUCH_DOWN};
    use cmf_wp2_ui::haptic::{DONE, DOUBLE, Motor, Pattern, TICK};

    cmf_wp2_app::entry!(main);
    cmf_wp2_app::app_icon!();

    /// ~55 FPS smooth frame cadence.
    const FRAME_MS: u32 = 18;
    const CATCH_UP: u32 = 2;
    const RETRY_MS: u32 = 550;

    /// Crisp tactile snap for a PERFECT drop.
    const PERFECT_SNAP: Pattern = Pattern::new(&[(420, 22), (0, 0)]);

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

    fn main(api: &'static Api) -> i32 {
        let mut screen = api;
        let mut motor = Motor::new(api);

        let mut blob = [0u8; 16];
        let n = api.load(&mut blob);
        let mut best = if n > 0 { decode_best(&blob[..n as usize]) } else { 0 };
        let mut saved_best = best;

        let mut game = Game::new();
        let mut seen = Some(frame(&mut screen, &game, best, None));
        api.show();

        let mut next = api.now().wrapping_add(FRAME_MS);
        let mut over_at = 0u32;

        loop {
            motor.update();
            let now = api.now();
            let active = game.state == State::Playing || game.debris.is_some();
            let left = if !active {
                u32::MAX
            } else {
                (next.wrapping_sub(now) as i32).max(0) as u32
            };
            let wait = motor.wait().map_or(left, |m| m.min(left));

            let mut tapped = false;
            let mut resumed = false;

            if let Some(first) = api.event(wait) {
                let mut ev = Some(first);
                while let Some(e) = ev {
                    match e.kind {
                        BUTTON => {
                            motor.off();
                            if best > saved_best {
                                api.save(&encode_best(best));
                            }
                            api.keep_screen_on(false);
                            return 0;
                        }
                        PAUSE => {
                            motor.off();
                            if best > saved_best {
                                api.save(&encode_best(best));
                                saved_best = best;
                            }
                            api.keep_screen_on(false);
                            if !wait_while_paused(api) {
                                return 0;
                            }
                            resumed = true;
                            tapped = false;
                            break;
                        }
                        TOUCH_DOWN => {
                            tapped = true;
                        }
                        _ => {}
                    }
                    ev = api.event(0);
                }
            }

            if resumed {
                seen = Some(frame(&mut screen, &game, best, None));
                api.show();
                if game.state == State::Playing {
                    api.keep_screen_on(true);
                }
                next = api.now().wrapping_add(FRAME_MS);
                continue;
            }

            if tapped {
                let tnow = api.now();
                match game.state {
                    State::Ready => {
                        game.start(best);
                        motor.play(TICK);
                        seen = Some(frame(&mut screen, &game, best, seen));
                        api.show();
                        api.keep_screen_on(true);
                        next = tnow.wrapping_add(FRAME_MS);
                        continue;
                    }
                    State::Over => {
                        if due(tnow, over_at.wrapping_add(RETRY_MS)) {
                            game.start(best);
                            motor.play(TICK);
                            seen = Some(frame(&mut screen, &game, best, seen));
                            api.show();
                            api.keep_screen_on(true);
                            next = tnow.wrapping_add(FRAME_MS);
                        }
                        continue;
                    }
                    State::Playing => {
                        match game.drop_slab() {
                            DropResult::Placed => motor.play(TICK),
                            DropResult::NewBest => {
                                best = game.score;
                                motor.play(DOUBLE);
                            }
                            DropResult::Perfect { streak, new_best, .. } => {
                                if new_best || game.score > best {
                                    best = game.score;
                                }
                                if new_best || streak >= 3 {
                                    motor.play(DOUBLE);
                                } else {
                                    motor.play(PERFECT_SNAP);
                                }
                            }
                            DropResult::Missed => {
                                motor.play(DONE);
                                over_at = tnow;
                                if game.score > best {
                                    best = game.score;
                                }
                                if best > saved_best {
                                    api.save(&encode_best(best));
                                    saved_best = best;
                                }
                                api.keep_screen_on(false);
                            }
                        }
                        seen = Some(frame(&mut screen, &game, best, seen));
                        api.show();
                        next = tnow.wrapping_add(FRAME_MS);
                        continue;
                    }
                }
            }

            if game.state != State::Playing && game.debris.is_none() {
                continue;
            }

            let now = api.now();
            if due(now, next) {
                let mut steps = 0u32;
                while steps < CATCH_UP && due(now, next) {
                    next = next.wrapping_add(FRAME_MS);
                    steps += 1;
                    game.step();
                }
                if due(now, next) {
                    next = now.wrapping_add(FRAME_MS);
                }
                seen = Some(frame(&mut screen, &game, best, seen));
                api.show();
            }
        }
    }
}

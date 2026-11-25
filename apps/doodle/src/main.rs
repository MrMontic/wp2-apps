#![cfg_attr(any(target_os = "none", wp2_sim), no_main, cfg_attr(target_os = "none", no_std))]

#[cfg_attr(not(any(target_os = "none", wp2_sim)), allow(dead_code))]
mod game;
#[cfg_attr(not(any(target_os = "none", wp2_sim)), allow(dead_code))]
mod view;

#[cfg(not(any(target_os = "none", wp2_sim)))]
fn main() {}

#[cfg(any(target_os = "none", wp2_sim))]
mod device {
    use crate::game::{EventFx, Game, State, decode_best, encode_best};
    use crate::view::frame;
    use cmf_wp2_app::{Api, BUTTON, CROWN, PAUSE, RESUME, TOUCH_DOWN};
    use cmf_wp2_ui::haptic::{DONE, DOUBLE, Motor, Pattern, TICK};

    cmf_wp2_app::entry!(main);
    cmf_wp2_app::app_icon!();

    /// ~55 FPS smooth frame cadence.
    const FRAME_MS: u32 = 18;
    const CATCH_UP: u32 = 2;
    const RETRY_MS: u32 = 550;

    const SPRING_FX: Pattern = Pattern::new(&[(520, 32), (0, 0)]);
    const BREAK_FX: Pattern = Pattern::new(&[(220, 14), (0, 0)]);
    const POWER_FX: Pattern = Pattern::new(&[(420, 24), (0, 50), (620, 36), (0, 0)]);

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
        api.now().wrapping_mul(2_654_435_761) ^ t
    }

    fn main(api: &'static Api) -> i32 {
        let mut screen = api;
        let mut motor = Motor::new(api);

        let mut blob = [0u8; 16];
        let n = api.load(&mut blob);
        let mut best = if n > 0 { decode_best(&blob[..n as usize]) } else { 0 };
        let mut saved_best = best;

        let mut game = Game::new(seed(api));
        let mut seen = Some(frame(&mut screen, &game, best, None));
        api.show();

        let mut next = api.now().wrapping_add(FRAME_MS);
        let mut over_at = 0u32;

        loop {
            motor.update();
            let now = api.now();
            let animating = game.state != State::Over;
            let left = if !animating {
                u32::MAX
            } else {
                (next.wrapping_sub(now) as i32).max(0) as u32
            };
            let wait = motor.wait().map_or(left, |m| m.min(left));

            // Drain ALL queued events in one batch so fast crown steering NEVER starves frames!
            let mut crown_acc: i32 = 0;
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
                            crown_acc = 0;
                            tapped = false;
                            break;
                        }
                        CROWN => {
                            crown_acc += e.delta as i32;
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

            if crown_acc != 0 {
                game.apply_crown(crown_acc);
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
                        if game.shoot() {
                            motor.play(BREAK_FX);
                        }
                    }
                }
            }

            if game.state == State::Over {
                continue;
            }

            let now = api.now();
            if due(now, next) {
                let mut steps = 0u32;
                while steps < CATCH_UP && due(now, next) {
                    next = next.wrapping_add(FRAME_MS);
                    steps += 1;
                    let (fx, _) = game.step();
                    match fx {
                        EventFx::None => {}
                        EventFx::Jump => motor.play(TICK),
                        EventFx::Spring => motor.play(SPRING_FX),
                        EventFx::Break => motor.play(BREAK_FX),
                        EventFx::Powerup => motor.play(POWER_FX),
                        EventFx::Stomp | EventFx::Shot => motor.play(DOUBLE),
                        EventFx::NewBest => {
                            best = game.score;
                            motor.play(DOUBLE);
                        }
                        EventFx::GameOver => {
                            motor.play(DONE);
                            over_at = now;
                            if game.score > best {
                                best = game.score;
                            }
                            if best > saved_best {
                                api.save(&encode_best(best));
                                saved_best = best;
                            }
                            api.keep_screen_on(false);
                            break;
                        }
                    }
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

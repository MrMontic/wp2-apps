#![cfg_attr(any(target_os = "none", wp2_sim), no_main, cfg_attr(target_os = "none", no_std))]

#[cfg_attr(not(any(target_os = "none", wp2_sim)), allow(dead_code))]
mod game;
#[cfg_attr(not(any(target_os = "none", wp2_sim)), allow(dead_code))]
mod view;

#[cfg(not(any(target_os = "none", wp2_sim)))]
fn main() {}

#[cfg(any(target_os = "none", wp2_sim))]
mod device {
    use crate::game::{Game, State, Step, decode_best, encode_best};
    use crate::view::frame;
    use cmf_wp2_app::{Api, BUTTON, CROWN, PAUSE, RESUME, TOUCH_DOWN};
    use cmf_wp2_ui::haptic::{DONE, DOUBLE, Motor, TICK};

    cmf_wp2_app::entry!(main);
    cmf_wp2_app::app_icon!();

    /// ~55 FPS smooth frame cadence on the 60 Hz AMOLED panel.
    const FRAME_MS: u32 = 18;
    /// Cap physics catch-up so a slow tick never teleports the ball.
    const CATCH_UP: u32 = 2;
    /// Guard against accidental instant restart right after missing.
    const RETRY_MS: u32 = 550;

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
            let still = game.state != State::Playing;
            let left = if still {
                u32::MAX
            } else {
                (next.wrapping_sub(now) as i32).max(0) as u32
            };
            let wait = motor.wait().map_or(left, |m| m.min(left));

            // Drain ALL queued events in one batch so fast crown spins NEVER starve rendering!
            let mut crown_acc: i32 = 0;
            let mut tap: Option<(i32, i32)> = None;
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
                seen = Some(frame(&mut screen, &game, best, None));
                api.show();
                if game.state == State::Playing {
                    api.keep_screen_on(true);
                }
                next = api.now().wrapping_add(FRAME_MS);
                continue;
            }

            // Apply accumulated crown movement immediately.
            if crown_acc != 0 {
                game.apply_crown(crown_acc);
            }

            // Handle tap input.
            if let Some((tx, ty)) = tap {
                let tnow = api.now();
                match game.state {
                    State::Playing => game.steer(tx, ty),
                    State::Over if !due(tnow, over_at.wrapping_add(RETRY_MS)) => {}
                    state => {
                        if state == State::Over {
                            game.restart();
                        }
                        game.launch(best);
                        seen = Some(frame(&mut screen, &game, best, seen));
                        api.show();
                        api.keep_screen_on(true);
                        next = tnow.wrapping_add(FRAME_MS);
                        continue;
                    }
                }
            }

            // On Ready/Over screens, only redraw when the crown moved the paddle.
            if game.state != State::Playing {
                if crown_acc != 0 {
                    seen = Some(frame(&mut screen, &game, best, seen));
                    api.show();
                }
                continue;
            }

            // Playing: advance physics when due, or render immediately if crown moved.
            let now = api.now();
            let tick_due = due(now, next);
            if tick_due {
                let mut steps = 0u32;
                while steps < CATCH_UP && due(now, next) {
                    next = next.wrapping_add(FRAME_MS);
                    steps += 1;
                    match game.step() {
                        Step::Nothing => {}
                        Step::Scored => motor.play(TICK),
                        Step::NewBest => {
                            best = game.score;
                            motor.play(DOUBLE);
                        }
                        Step::Crashed => {
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
            }

            if tick_due || crown_acc != 0 {
                seen = Some(frame(&mut screen, &game, best, seen));
                api.show();
            }
        }
    }
}

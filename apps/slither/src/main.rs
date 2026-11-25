#![cfg_attr(any(target_os = "none", wp2_sim), no_main, cfg_attr(target_os = "none", no_std))]

#[cfg_attr(not(any(target_os = "none", wp2_sim)), allow(dead_code))]
mod game;
#[cfg_attr(not(any(target_os = "none", wp2_sim)), allow(dead_code))]
mod palette;
#[cfg_attr(not(any(target_os = "none", wp2_sim)), allow(dead_code))]
mod render;

#[cfg(not(any(target_os = "none", wp2_sim)))]
fn main() {}

#[cfg(any(target_os = "none", wp2_sim))]
mod device {
    use crate::game::{Game, SAVE_LEN, SaveData, Screen, StepEvent, decode_save, encode_save};
    use crate::palette::UI_BG;
    use crate::render::{
        FrameBuf, OVER_PLAY_RECT, OVER_SKIN_RECT, TITLE_NEXT_RECT, TITLE_PLAY_RECT,
        TITLE_PREV_RECT, hit, render_frame,
    };
    use cmf_wp2_app::{Api, BUTTON, CROWN, PAUSE, RESUME, TOUCH_DOWN, TOUCH_UP};
    use cmf_wp2_ui::haptic::{DONE, DOUBLE, Motor, TICK};
    use cmf_wp2_ui::SIZE;
    use core::ptr::addr_of_mut;

    cmf_wp2_app::entry!(main);
    cmf_wp2_app::app_icon!();

    /// ~45 FPS smooth gameplay cadence on 60 Hz AMOLED.
    const FRAME_MS: u32 = 22;
    /// Gentle ~20 FPS animation on Title Screen for the live preview worm.
    const TITLE_MS: u32 = 50;
    /// Cap physics catch-up so a slow frame never teleports snakes.
    const CATCH_UP: u32 = 2;
    /// Guard against accidental instant tap right after dying while holding boost.
    const RETRY_MS: u32 = 550;

    /// Large structs stay strictly in static RAM (never on the 8 KiB stack!).
    static mut GAME: Game = Game::zeroed();
    static mut FB: FrameBuf = FrameBuf::new();

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
        let t = api.time().map_or(0, |t| {
            (t.minute as u32 * 60 + t.second as u32) * 1000 + t.millis as u32
        });
        api.now().wrapping_mul(2_654_435_761) ^ t
    }

    fn persist_if_dirty(api: &Api, cur: SaveData, saved: &mut SaveData) {
        if cur.best_len != saved.best_len
            || cur.best_kills != saved.best_kills
            || cur.skin != saved.skin
        {
            api.save(&encode_save(cur));
            *saved = cur;
        }
    }

    fn main(api: &'static Api) -> i32 {
        let mut screen = api;
        let mut motor = Motor::new(api);

        let mut blob = [0u8; SAVE_LEN];
        let n = api.load(&mut blob);
        let loaded = if n >= SAVE_LEN as i32 {
            decode_save(&blob[..n as usize]).unwrap_or(SaveData::default_new())
        } else {
            SaveData::default_new()
        };
        let mut saved = loaded;

        // Safe single-threaded device entry: obtain exclusive refs to static buffers.
        let game = unsafe { &mut *addr_of_mut!(GAME) };
        let fb = unsafe { &mut *addr_of_mut!(FB) };

        game.init(seed(api), loaded);
        api.rect(0, 0, SIZE as u16, SIZE as u16, UI_BG);
        render_frame(&mut screen, fb, game, true);
        api.show();

        let mut next = api.now().wrapping_add(TITLE_MS);
        let mut over_at = 0u32;

        loop {
            motor.update();
            let now = api.now();
            let frame_left = match game.screen {
                Screen::Over => u32::MAX,
                Screen::Title | Screen::Playing => (next.wrapping_sub(now) as i32).max(0) as u32,
            };
            let wait = motor.wait().map_or(frame_left, |m| m.min(frame_left));

            // Drain ALL queued events in one batch so fast crown spins NEVER starve rendering!
            let mut crown_acc: i32 = 0;
            let mut touch_down: Option<(i32, i32)> = None;
            let mut saw_touch_up = false;
            let mut resumed = false;

            if let Some(first) = api.event(wait) {
                let mut ev = Some(first);
                while let Some(e) = ev {
                    match e.kind {
                        BUTTON => {
                            motor.off();
                            persist_if_dirty(api, game.save, &mut saved);
                            api.keep_screen_on(false);
                            return 0;
                        }
                        PAUSE => {
                            motor.off();
                            game.boost_held = false;
                            persist_if_dirty(api, game.save, &mut saved);
                            api.keep_screen_on(false);
                            if !wait_while_paused(api) {
                                return 0;
                            }
                            resumed = true;
                            crown_acc = 0;
                            touch_down = None;
                            saw_touch_up = false;
                            break;
                        }
                        CROWN => {
                            crown_acc += e.delta as i32;
                        }
                        TOUCH_DOWN => {
                            touch_down = Some((e.x as i32, e.y as i32));
                            if game.screen == Screen::Playing {
                                game.boost_held = true;
                            }
                        }
                        TOUCH_UP => {
                            saw_touch_up = true;
                            game.boost_held = false;
                        }
                        _ => {}
                    }
                    ev = api.event(0);
                }
            }

            if resumed {
                game.boost_held = false;
                api.rect(0, 0, SIZE as u16, SIZE as u16, UI_BG);
                render_frame(&mut screen, fb, game, true);
                api.show();
                api.keep_screen_on(game.screen == Screen::Playing);
                let dt = if game.screen == Screen::Playing { FRAME_MS } else { TITLE_MS };
                next = api.now().wrapping_add(dt);
                continue;
            }

            if crown_acc != 0 {
                let prev_skin = game.selected_skin;
                game.apply_crown(crown_acc);
                if game.screen == Screen::Title && game.selected_skin != prev_skin {
                    motor.play(TICK);
                    render_frame(&mut screen, fb, game, false);
                    api.show();
                }
            }

            if let Some((tx, ty)) = touch_down {
                let tnow = api.now();
                match game.screen {
                    Screen::Title => {
                        if hit(TITLE_PREV_RECT, tx, ty) {
                            game.cycle_skin(-1);
                            motor.play(TICK);
                            render_frame(&mut screen, fb, game, false);
                            api.show();
                        } else if hit(TITLE_NEXT_RECT, tx, ty) {
                            game.cycle_skin(1);
                            motor.play(TICK);
                            render_frame(&mut screen, fb, game, false);
                            api.show();
                        } else if hit(TITLE_PLAY_RECT, tx, ty) || ty > 300 {
                            persist_if_dirty(api, game.save, &mut saved);
                            game.start_match();
                            motor.play(TICK);
                            render_frame(&mut screen, fb, game, true);
                            api.show();
                            api.keep_screen_on(true);
                            next = tnow.wrapping_add(FRAME_MS);
                            continue;
                        }
                    }
                    Screen::Playing => {
                        // Already handled: `boost_held = !saw_touch_up`
                        if saw_touch_up {
                            game.boost_held = false;
                        }
                    }
                    Screen::Over => {
                        if due(tnow, over_at.wrapping_add(RETRY_MS)) {
                            if hit(OVER_SKIN_RECT, tx, ty) {
                                game.screen = Screen::Title;
                                motor.play(TICK);
                                api.rect(0, 0, SIZE as u16, SIZE as u16, UI_BG);
                                render_frame(&mut screen, fb, game, true);
                                api.show();
                                next = tnow.wrapping_add(TITLE_MS);
                                continue;
                            } else if hit(OVER_PLAY_RECT, tx, ty) || ty < 362 {
                                game.start_match();
                                motor.play(TICK);
                                render_frame(&mut screen, fb, game, true);
                                api.show();
                                api.keep_screen_on(true);
                                next = tnow.wrapping_add(FRAME_MS);
                                continue;
                            }
                        }
                    }
                }
            }

            let now = api.now();
            match game.screen {
                Screen::Title => {
                    if due(now, next) {
                        game.tick = game.tick.wrapping_add(1);
                        render_frame(&mut screen, fb, game, false);
                        api.show();
                        next = now.wrapping_add(TITLE_MS);
                    }
                }
                Screen::Playing => {
                    let tick_due = due(now, next);
                    if tick_due {
                        let mut steps = 0u32;
                        let mut died = false;
                        while steps < CATCH_UP && due(now, next) {
                            next = next.wrapping_add(FRAME_MS);
                            steps += 1;
                            match game.step() {
                                StepEvent::None => {}
                                StepEvent::AteBigOrb => motor.play(TICK),
                                StepEvent::KilledBot => motor.play(DOUBLE),
                                StepEvent::NewBest => motor.play(DOUBLE),
                                StepEvent::PlayerDied => {
                                    died = true;
                                    over_at = now;
                                    motor.play(DONE);
                                    persist_if_dirty(api, game.save, &mut saved);
                                    api.keep_screen_on(false);
                                    break;
                                }
                            }
                        }
                        if due(now, next) {
                            next = now.wrapping_add(FRAME_MS);
                        }
                        render_frame(&mut screen, fb, game, died);
                        api.show();
                    }
                }
                Screen::Over => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use crate::game::{
        ARENA_R, BOOST_BURN_TICKS, BotStyle, DeathCause, Game, MAX_ORBS, MAX_SNAKES,
        MIN_BOOST_MASS, Q_SHIFT, SaveData, Screen, StepEvent, decode_save, encode_save,
    };
    use crate::render::{FrameBuf, render_frame};
    use cmf_wp2_ui::{Fb, SIZE};
    use std::boxed::Box;
    use std::vec;

    fn new_game(seed: u32) -> Box<Game> {
        let mut g = Box::new(Game::zeroed());
        g.init(seed, SaveData::default_new());
        g
    }

    #[test]
    fn boost_drains_mass_and_drops_tail_orb() {
        let mut g = new_game(42);
        g.start_match();
        // Put player in open space with plenty of mass and no bots nearby
        for i in 1..MAX_SNAKES {
            g.snakes[i].alive = false;
            g.snakes[i].respawn_ticks = 9999;
        }
        g.snakes[0].spawn(true, 2, BotStyle::Farmer, 0, 0, 0, 40, 0);
        // Clear orbs so we can cleanly detect dropped boost orbs
        for o in &mut g.orbs {
            o.val = 0;
        }
        g.active_orbs = 0;

        let start_x = g.snakes[0].head_wx();
        g.boost_held = true;
        for _ in 0..(BOOST_BURN_TICKS as usize + 2) {
            g.step();
        }
        assert!(g.snakes[0].mass < 40, "boost must drain mass (now {})", g.snakes[0].mass);
        assert!(
            g.snakes[0].head_wx() - start_x > 50,
            "boost must move significantly faster than normal"
        );
        let boost_orbs = g.orbs.iter().filter(|o| o.val == 2).count();
        assert!(boost_orbs >= 1, "boosting must drop glowing orb behind tail");

        // Verify boost stops draining at MIN_BOOST_MASS
        g.snakes[0].mass = MIN_BOOST_MASS;
        g.step();
        assert!(!g.snakes[0].boosting, "cannot boost at minimum mass");
        assert_eq!(g.snakes[0].mass, MIN_BOOST_MASS);
    }

    #[test]
    fn self_body_never_kills_player() {
        let mut g = new_game(123);
        g.start_match();
        for i in 1..MAX_SNAKES {
            g.snakes[i].alive = false;
            g.snakes[i].respawn_ticks = 9999;
        }
        g.snakes[0].spawn(true, 0, BotStyle::Farmer, 0, 0, 0, 120, 0);

        // Tightly spin in a full circle crossing our own long body repeatedly!
        for _ in 0..90 {
            g.apply_crown(2);
            let ev = g.step();
            assert_ne!(ev, StepEvent::PlayerDied, "self-overlap must NEVER kill in Slither.io!");
            assert!(g.snakes[0].alive);
        }
    }

    #[test]
    fn enemy_head_to_player_body_kills_bot_and_bursts_loot() {
        let mut g = new_game(77);
        g.start_match();
        for i in 2..MAX_SNAKES {
            g.snakes[i].alive = false;
            g.snakes[i].respawn_ticks = 9999;
        }
        for o in &mut g.orbs {
            o.val = 0;
        }
        g.active_orbs = 0;

        // Player moving horizontally along y = 0 (body trails behind to the left: x = 0..-160)
        g.snakes[0].spawn(true, 1, BotStyle::Farmer, 40, 0, 0, 100, 0);
        // Bot #1 heading straight down (angle = 16384 = +90 deg) right into player's body at x = -20!
        g.snakes[1].spawn(false, 3, BotStyle::Hunter, -20, -24, 16384, 180, 0);
        g.snakes[1].target_ang = 16384;

        let mut killed = false;
        for _ in 0..12 {
            // Keep bot pointed into player body for deterministic collision test
            g.snakes[1].ang = 16384;
            g.snakes[1].target_ang = 16384;
            if g.step() == StepEvent::KilledBot {
                killed = true;
                break;
            }
        }
        assert!(killed, "bot colliding with player body must die and credit kill");
        assert!(!g.snakes[1].alive);
        assert_eq!(g.kills, 1);
        assert!(g.kill_flash_ticks > 0);
        let loot_orbs = g.orbs.iter().filter(|o| o.val >= 3).count();
        assert!(loot_orbs >= 6, "dead giant bot must burst into rich glowing loot trail (got {loot_orbs})");
    }

    #[test]
    fn wall_collision_kills_player_cleanly() {
        let mut g = new_game(99);
        g.start_match();
        for i in 1..MAX_SNAKES {
            g.snakes[i].alive = false;
            g.snakes[i].respawn_ticks = 9999;
        }
        g.snakes[0].spawn(true, 0, BotStyle::Farmer, ARENA_R - 20, 0, 0, 30, 0);
        let mut died = false;
        for _ in 0..20 {
            if g.step() == StepEvent::PlayerDied {
                died = true;
                break;
            }
        }
        assert!(died, "hitting red circular wall must kill player");
        assert_eq!(g.screen, Screen::Over);
        assert_eq!(g.death_cause, DeathCause::Wall);
    }

    #[test]
    fn smart_ai_bots_avoid_walls_forage_and_survive_300_ticks() {
        let mut g = new_game(2026);
        g.start_match();
        // Park player out of the way in a tight safe orbit near center so bots run freely
        g.snakes[0].spawn(true, 0, BotStyle::Farmer, 0, 0, 0, 20, 0);

        let initial_total_bot_mass: u32 = g.snakes[1..MAX_SNAKES].iter().map(|s| s.mass).sum();
        let mut max_bot_dist = 0i32;
        let mut bot_wall_deaths = 0usize;

        for _ in 0..300 {
            // Keep player orbiting gently near center so test doesn't end on player wall hit
            g.snakes[0].target_ang = g.snakes[0].ang.wrapping_add(1600);
            if !g.snakes[0].alive {
                g.snakes[0].spawn(true, 0, BotStyle::Farmer, 0, 0, 0, 20, 0);
                g.screen = Screen::Playing;
            }

            let prev_alive: [bool; MAX_SNAKES] = core::array::from_fn(|i| g.snakes[i].alive);
            let prev_pos: [(i32, i32); MAX_SNAKES] =
                core::array::from_fn(|i| (g.snakes[i].head_wx(), g.snakes[i].head_wy()));

            g.step();

            for i in 1..MAX_SNAKES {
                if g.snakes[i].alive {
                    let x = g.snakes[i].head_wx();
                    let y = g.snakes[i].head_wy();
                    let d = cmf_wp2_ui::motion::isqrt(x * x + y * y);
                    if d > max_bot_dist {
                        max_bot_dist = d;
                    }
                    assert!(d < ARENA_R, "alive bot {i} must stay inside arena (d={d})");
                } else if prev_alive[i] {
                    // Check if it died near wall (which smart wall-avoidance should prevent!)
                    let (px, py) = prev_pos[i];
                    let pd = cmf_wp2_ui::motion::isqrt(px * px + py * py);
                    if pd > ARENA_R - 45 {
                        bot_wall_deaths += 1;
                    }
                }
            }
        }

        let alive_bots = g.snakes[1..MAX_SNAKES].iter().filter(|s| s.alive).count();
        let final_total_bot_mass: u32 = g.snakes[1..MAX_SNAKES].iter().filter(|s| s.alive).map(|s| s.mass).sum();

        assert_eq!(bot_wall_deaths, 0, "smart bots must NOT suicide into arena walls!");
        assert!(alive_bots >= 4, "majority of bots must survive and thrive (alive={alive_bots}/7)");
        assert!(
            final_total_bot_mass > initial_total_bot_mass / 2,
            "bots should forage & maintain strong mass (start={initial_total_bot_mass}, end={final_total_bot_mass}, max_dist={max_bot_dist})"
        );
    }

    #[test]
    fn save_roundtrip_and_corruption_rejection() {
        let orig = SaveData { best_len: 482, best_kills: 9, skin: 6 };
        let encoded = encode_save(orig);
        let decoded = decode_save(&encoded).expect("valid SLT1 save must decode");
        assert_eq!(decoded.best_len, 482);
        assert_eq!(decoded.best_kills, 9);
        assert_eq!(decoded.skin, 6);

        // Wrong magic
        let mut bad_magic = encoded;
        bad_magic[0] = b'X';
        assert!(decode_save(&bad_magic).is_none());

        // Flipped bit in payload (checksum failure)
        let mut flipped = encoded;
        flipped[5] ^= 0x10;
        assert!(decode_save(&flipped).is_none());

        // Short buffer
        assert!(decode_save(&encoded[..10]).is_none());

        // Invalid skin out of range
        let mut bad_skin = encoded;
        bad_skin[10] = 99;
        assert!(decode_save(&bad_skin).is_none());
    }

    #[test]
    fn render_title_gameplay_and_gameover_within_rect_budget() {
        let mut g = new_game(31415);
        let mut fbuf = Box::new(FrameBuf::new());
        let mut px = vec![0u16; (SIZE * SIZE) as usize];

        // 1. Title screen across all 8 skins
        for skin in 0..8 {
            g.selected_skin = skin;
            g.tick += 5;
            let mut fb = Fb::new(&mut px);
            render_frame(&mut fb, &mut fbuf, &g, true);
            assert!(fb.rects > 50, "title screen must render content");
        }

        // 2. Gameplay: 20 frames including boosting, orbs, nearby bots, minimap, HUD
        g.start_match();
        g.snakes[0].mass = 85;
        g.boost_held = true;
        // Bring a couple bots into viewport so worst-case rendering is exercised!
        g.snakes[1].spawn(false, 2, BotStyle::Hunter, g.snakes[0].head_wx() + 70, g.snakes[0].head_wy() - 40, 20000, 140, 1);
        g.snakes[2].spawn(false, 6, BotStyle::Scavenger, g.snakes[0].head_wx() - 80, g.snakes[0].head_wy() + 50, 5000, 90, 2);
        g.cam_x = g.snakes[0].hx;
        g.cam_y = g.snakes[0].hy;

        let mut max_rects = 0u32;
        let mut total_rects = 0u64;
        for frame_i in 0..20 {
            if frame_i % 5 == 0 {
                g.apply_crown(1);
            }
            if g.screen != Screen::Playing {
                g.start_match();
            }
            g.step();
            let mut fb = Fb::new(&mut px);
            render_frame(&mut fb, &mut fbuf, &g, frame_i == 0);
            if fb.rects > max_rects {
                max_rects = fb.rects;
            }
            total_rects += fb.rects as u64;
            assert!(
                fb.rects <= 1050,
                "frame {frame_i} exceeded rect budget: {} > 1050",
                fb.rects
            );
        }
        let avg_rects = total_rects / 20;
        std::println!("Slither gameplay rect counts (with 3 snakes + orbs + boost + minimap): avg={avg_rects}, max={max_rects}");

        // 3. Also test near-border rendering (red circular arena wall in view!)
        g.start_match();
        g.snakes[0].spawn(true, 4, BotStyle::Farmer, ARENA_R - 120, 0, 32768, 60, 0);
        g.cam_x = g.snakes[0].hx;
        g.cam_y = g.snakes[0].hy;
        {
            let mut fb = Fb::new(&mut px);
            render_frame(&mut fb, &mut fbuf, &g, false);
            assert!(fb.rects <= 1050, "border frame exceeded rect budget: {}", fb.rects);
            std::println!("Slither border frame rect count: {}", fb.rects);
        }

        // 4. Game Over screen
        g.screen = Screen::Over;
        g.final_len = 256;
        g.kills = 4;
        g.best_rank = 1;
        g.new_best_achieved = true;
        g.death_cause = DeathCause::Snake;
        {
            let mut fb = Fb::new(&mut px);
            render_frame(&mut fb, &mut fbuf, &g, true);
            assert!(fb.rects > 10 && fb.rects < 600);
        }
        // Sanity: MAX_ORBS and Q_SHIFT used cleanly
        assert!(MAX_ORBS >= 200 && Q_SHIFT == 8);
    }
}

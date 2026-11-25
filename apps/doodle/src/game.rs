//! Full Doodle Jump physics, procedural generator, powerups, monsters, and bullets.
//! Pure `no_std` fixed-point (Q = 256) at ~55 FPS (18 ms ticks), zero heap allocations.

pub const SIZE: i32 = 466;
pub const C: i32 = SIZE / 2;
pub const Q: i32 = 256;

/// Visible horizontal corridor framed cleanly inside the 466x466 round watch bezel.
pub const PLAY_L: i32 = 72;
pub const PLAY_R: i32 = 394;
pub const PLAY_W: i32 = PLAY_R - PLAY_L;
pub const HUD_H: i32 = 64;
/// Camera scroll line: when Doodler rises above this Y, the world scrolls down smoothly.
pub const SCROLL_Y: i32 = 205;

/// Platform & character geometry (px).
pub const PLAT_W: i32 = 60;
pub const PLAT_H: i32 = 14;
pub const DOOD_W: i32 = 24;
pub const DOOD_H: i32 = 26;
pub const MON_W: i32 = 28;
pub const MON_H: i32 = 22;

/// Vertical jump physics (Q px per 18 ms tick).
/// Max jump height = (10.6)^2 / (2 * 0.44) ~= 127 px.
pub const JUMP_VY: i32 = -(10 * Q + 154); // -10.6 px/tick
pub const SPRING_VY: i32 = -(18 * Q + 64); // -18.25 px/tick (huge spring launch!)
pub const STOMP_VY: i32 = -(12 * Q + 64); // -12.25 px/tick (monster bounce)
pub const GRAVITY: i32 = 112; // ~0.438 px/tick²
pub const MAX_FALL_VY: i32 = 11 * Q;

/// Horizontal crown steering physics (snappy with smooth glide & wrap-around).
pub const CROWN_IMPULSE: i32 = 3 * Q + 180; // ~3.7 px/tick per detent
pub const MAX_VX: i32 = 9 * Q + 128;

/// Powerup flight durations (18 ms ticks) and upward speeds.
pub const PROP_TICKS: u8 = 88; // ~1.6 s
pub const PROP_VY: i32 = -(9 * Q + 64);
pub const JET_TICKS: u8 = 105; // ~1.9 s
pub const JET_VY: i32 = -(14 * Q + 128);

/// Maximum simultaneous platforms, bullets, and monsters.
pub const MAX_PLATS: usize = 16;
pub const MAX_BULLETS: usize = 3;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum State {
    Ready,
    Playing,
    Over,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EventFx {
    None,
    Jump,
    Spring,
    Break,
    Powerup,
    Stomp,
    Shot,
    NewBest,
    GameOver,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PlatKind {
    /// Classic green static platform.
    Green,
    /// Classic blue moving platform (glides left <-> right).
    Blue,
    /// Brown cracked platform: snaps in half when stepped on (no bounce).
    Brown,
    /// White one-shot platform: bounces once then vanishes.
    White,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Pickup {
    None,
    Spring,
    SpringUsed,
    Propeller,
    Jetpack,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Platform {
    pub active: bool,
    pub x_q: i32,
    pub y_q: i32,
    pub vx_q: i32,
    pub kind: PlatKind,
    pub pickup: Pickup,
    /// For broken brown platforms falling away (`0` = intact, `>0` = falling ticks).
    pub broken: u8,
}

impl Platform {
    pub const EMPTY: Self = Platform {
        active: false,
        x_q: 0,
        y_q: 0,
        vx_q: 0,
        kind: PlatKind::Green,
        pickup: Pickup::None,
        broken: 0,
    };

    #[inline]
    pub fn x(&self) -> i32 {
        (self.x_q + Q / 2) >> 8
    }

    #[inline]
    pub fn y(&self) -> i32 {
        (self.y_q + Q / 2) >> 8
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PowerMode {
    None,
    Propeller(u8),
    Jetpack(u8),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Monster {
    pub active: bool,
    pub x_q: i32,
    pub y_q: i32,
    pub vx_q: i32,
    pub variant: u8,
}

impl Monster {
    pub const EMPTY: Self = Monster {
        active: false,
        x_q: 0,
        y_q: 0,
        vx_q: 0,
        variant: 0,
    };

    #[inline]
    pub fn x(&self) -> i32 {
        (self.x_q + Q / 2) >> 8
    }

    #[inline]
    pub fn y(&self) -> i32 {
        (self.y_q + Q / 2) >> 8
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Bullet {
    pub active: bool,
    pub x: i32,
    pub y: i32,
}

impl Bullet {
    pub const EMPTY: Self = Bullet {
        active: false,
        x: 0,
        y: 0,
    };
}

#[derive(Clone)]
pub struct Game {
    pub state: State,
    /// Doodler center X and bottom-feet Y in Q px.
    pub x_q: i32,
    pub y_q: i32,
    pub vx_q: i32,
    pub vy_q: i32,
    pub facing_right: bool,
    /// Countdown for upward shooting snout pose.
    pub shoot_pose: u8,
    pub power: PowerMode,
    pub tick: u32,

    pub plats: [Platform; MAX_PLATS],
    pub monster: Monster,
    pub bullets: [Bullet; MAX_BULLETS],

    /// Total camera scroll distance (in Q px), determines height score.
    scroll_q: u32,
    pub bonus: u32,
    pub score: u32,
    pub beat_best: bool,
    run_best_start: u32,

    /// Highest spawned solid platform Y (screen coords, Q) so generator always keeps a reachable chain.
    highest_solid_y_q: i32,
    rng: u32,
}

fn xorshift(rng: &mut u32) -> u32 {
    let mut x = *rng;
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    *rng = x;
    x
}

impl Game {
    pub fn new(seed: u32) -> Self {
        let mut g = Game {
            state: State::Ready,
            x_q: C * Q,
            y_q: 330 * Q,
            vx_q: 0,
            vy_q: JUMP_VY,
            facing_right: true,
            shoot_pose: 0,
            power: PowerMode::None,
            tick: 0,
            plats: [Platform::EMPTY; MAX_PLATS],
            monster: Monster::EMPTY,
            bullets: [Bullet::EMPTY; MAX_BULLETS],
            scroll_q: 0,
            bonus: 0,
            score: 0,
            beat_best: false,
            run_best_start: 0,
            highest_solid_y_q: 350 * Q,
            rng: seed | 1,
        };
        g.reset();
        g
    }

    pub fn reset(&mut self) {
        self.state = State::Ready;
        self.x_q = C * Q;
        self.y_q = 340 * Q;
        self.vx_q = 0;
        self.vy_q = JUMP_VY;
        self.facing_right = true;
        self.shoot_pose = 0;
        self.power = PowerMode::None;
        self.tick = 0;
        self.plats = [Platform::EMPTY; MAX_PLATS];
        self.monster = Monster::EMPTY;
        self.bullets = [Bullet::EMPTY; MAX_BULLETS];
        self.scroll_q = 0;
        self.bonus = 0;
        self.score = 0;
        self.beat_best = false;

        // Safe starting floor platform right under Doodler
        self.plats[0] = Platform {
            active: true,
            x_q: (C - PLAT_W / 2) * Q,
            y_q: 352 * Q,
            vx_q: 0,
            kind: PlatKind::Green,
            pickup: Pickup::None,
            broken: 0,
        };
        self.highest_solid_y_q = 352 * Q;
        self.fill_platforms_upward();
    }

    pub fn start(&mut self, current_best: u32) {
        if self.state == State::Over {
            self.reset();
        }
        self.run_best_start = current_best;
        self.beat_best = false;
        self.state = State::Playing;
        self.vy_q = JUMP_VY;
    }

    #[inline]
    pub fn dood_x(&self) -> i32 {
        (self.x_q + Q / 2) >> 8
    }

    #[inline]
    pub fn dood_y(&self) -> i32 {
        (self.y_q + Q / 2) >> 8
    }

    #[inline]
    pub fn is_flying(&self) -> bool {
        !matches!(self.power, PowerMode::None)
    }

    /// Apply accumulated crown detents from one event-drain batch.
    pub fn apply_crown(&mut self, delta: i32) {
        if delta == 0 || self.state == State::Over {
            return;
        }
        self.facing_right = delta > 0;
        let boost = if delta.abs() >= 3 { 5 } else { 4 };
        let add = delta * CROWN_IMPULSE * boost / 4;
        self.vx_q = (self.vx_q / 2 + add).clamp(-MAX_VX, MAX_VX);
    }

    /// Tap during gameplay fires a doodle pellet straight up!
    pub fn shoot(&mut self) -> bool {
        if self.state != State::Playing || self.is_flying() {
            return false;
        }
        let bx = self.dood_x();
        let by = self.dood_y() - DOOD_H - 2;
        let Some(slot) = self.bullets.iter_mut().find(|b| !b.active) else {
            return false;
        };
        *slot = Bullet {
            active: true,
            x: bx,
            y: by,
        };
        self.shoot_pose = 12;
        true
    }

    /// Spawns reachable platforms upward until the top of the screen is populated.
    fn fill_platforms_upward(&mut self) {
        while self.highest_solid_y_q > (HUD_H - 12) * Q {
            let Some(idx) = self.plats.iter().position(|p| !p.active) else {
                break;
            };
            // Vertical spacing ramps smoothly with score, always strictly <= 104 px (< 127 px max jump!).
            let diff_lvl = (self.score / 120).min(10) as i32;
            let min_gap = 38 + diff_lvl * 3; // 38..68 px
            let span_gap = 28 + diff_lvl * 1; // +28..38 px -> max 106 px
            let gap = min_gap + (xorshift(&mut self.rng) % span_gap as u32) as i32;
            let ny_q = self.highest_solid_y_q - gap * Q;

            let range_x = (PLAY_W - PLAT_W).max(1) as u32;
            let nx = PLAY_L + (xorshift(&mut self.rng) % range_x) as i32;

            let r = xorshift(&mut self.rng) % 100;
            let kind = if self.score < 220 {
                PlatKind::Green
            } else if self.score < 850 {
                if r < 24 { PlatKind::Blue } else { PlatKind::Green }
            } else {
                match r {
                    0..=27 => PlatKind::Blue,
                    28..=45 => PlatKind::White,
                    _ => PlatKind::Green,
                }
            };

            let vx_q = if kind == PlatKind::Blue {
                let spd = (Q + 140 + diff_lvl * 26).min(3 * Q);
                if xorshift(&mut self.rng) & 1 == 0 { spd } else { -spd }
            } else {
                0
            };

            // Pickups spawn cleanly on static green platforms
            let pickup = if kind == PlatKind::Green && self.score >= 30 {
                let pr = xorshift(&mut self.rng) % 100;
                if pr < 11 {
                    Pickup::Spring
                } else if pr < 14 && self.score >= 180 {
                    Pickup::Propeller
                } else if pr < 16 && self.score >= 450 {
                    Pickup::Jetpack
                } else {
                    Pickup::None
                }
            } else {
                Pickup::None
            };

            self.plats[idx] = Platform {
                active: true,
                x_q: nx * Q,
                y_q: ny_q,
                vx_q,
                kind,
                pickup,
                broken: 0,
            };
            self.highest_solid_y_q = ny_q;

            // Occasionally add a breakable Brown decoy platform midway in the gap (never blocks reachability!)
            if self.score >= 120 && gap >= 54 && (xorshift(&mut self.rng) % 100) < 28 {
                if let Some(bidx) = self.plats.iter().position(|p| !p.active) {
                    let bx = PLAY_L + (xorshift(&mut self.rng) % range_x) as i32;
                    let by_q = ny_q + (gap / 2) * Q;
                    self.plats[bidx] = Platform {
                        active: true,
                        x_q: bx * Q,
                        y_q: by_q,
                        vx_q: 0,
                        kind: PlatKind::Brown,
                        pickup: Pickup::None,
                        broken: 0,
                    };
                }
            }

            // Occasionally spawn a hovering monster above the screen from score 450+
            if !self.monster.active && self.score >= 450 && (xorshift(&mut self.rng) % 100) < 12 {
                let mx = PLAY_L + 20 + (xorshift(&mut self.rng) % (PLAY_W - 40 - MON_W) as u32) as i32;
                let mvx = if xorshift(&mut self.rng) & 1 == 0 { Q + 60 } else { -(Q + 60) };
                self.monster = Monster {
                    active: true,
                    x_q: mx * Q,
                    y_q: ny_q - 24 * Q,
                    vx_q: mvx,
                    variant: (xorshift(&mut self.rng) & 1) as u8,
                };
            }
        }
    }

    /// Advance simulation by one 18 ms tick. Returns `(fx, scrolled_px)`.
    pub fn step(&mut self) -> (EventFx, i32) {
        self.tick = self.tick.wrapping_add(1);
        if self.shoot_pose > 0 {
            self.shoot_pose -= 1;
        }

        // On Ready screen: cute idle bounce on the home platform!
        if self.state == State::Ready {
            self.y_q += self.vy_q;
            self.vy_q += GRAVITY;
            if self.y_q >= 352 * Q {
                self.y_q = 352 * Q;
                self.vy_q = JUMP_VY;
            }
            return (EventFx::None, 0);
        }

        if self.state != State::Playing {
            return (EventFx::None, 0);
        }

        let mut fx = EventFx::None;

        // 1. Horizontal movement with smooth damping + classic screen wrap-around
        self.x_q += self.vx_q;
        self.vx_q = self.vx_q * 7 / 8;
        if self.vx_q.abs() < 24 {
            self.vx_q = 0;
        }
        if self.x_q < PLAY_L * Q {
            self.x_q += PLAY_W * Q;
        } else if self.x_q >= PLAY_R * Q {
            self.x_q -= PLAY_W * Q;
        }

        // 2. Vertical movement (Propeller / Jetpack vs normal gravity)
        let prev_y_q = self.y_q;
        match self.power {
            PowerMode::Propeller(t) => {
                self.vy_q = PROP_VY;
                self.y_q += self.vy_q;
                self.power = if t <= 1 { PowerMode::None } else { PowerMode::Propeller(t - 1) };
            }
            PowerMode::Jetpack(t) => {
                self.vy_q = JET_VY;
                self.y_q += self.vy_q;
                self.power = if t <= 1 { PowerMode::None } else { PowerMode::Jetpack(t - 1) };
            }
            PowerMode::None => {
                self.y_q += self.vy_q;
                self.vy_q = (self.vy_q + GRAVITY).min(MAX_FALL_VY);
            }
        }

        // 3. Move blue platforms & broken brown fragments
        for p in &mut self.plats {
            if !p.active {
                continue;
            }
            if p.broken > 0 {
                p.broken = p.broken.saturating_add(1);
                p.y_q += (3 + p.broken as i32 / 2) * Q;
                if p.y() > SIZE + 10 {
                    p.active = false;
                }
            } else if p.kind == PlatKind::Blue {
                p.x_q += p.vx_q;
                let min_x = PLAY_L * Q;
                let max_x = (PLAY_R - PLAT_W) * Q;
                if p.x_q <= min_x {
                    p.x_q = min_x;
                    p.vx_q = p.vx_q.abs();
                } else if p.x_q >= max_x {
                    p.x_q = max_x;
                    p.vx_q = -p.vx_q.abs();
                }
            }
        }

        // 4. Move monster & bullets
        if self.monster.active {
            self.monster.x_q += self.monster.vx_q;
            let min_x = (PLAY_L + 6) * Q;
            let max_x = (PLAY_R - MON_W - 6) * Q;
            if self.monster.x_q <= min_x {
                self.monster.x_q = min_x;
                self.monster.vx_q = self.monster.vx_q.abs();
            } else if self.monster.x_q >= max_x {
                self.monster.x_q = max_x;
                self.monster.vx_q = -self.monster.vx_q.abs();
            }
        }

        for b in &mut self.bullets {
            if !b.active {
                continue;
            }
            b.y -= 14;
            if b.y < HUD_H {
                b.active = false;
                continue;
            }
            if self.monster.active {
                let mx = self.monster.x();
                let my = self.monster.y();
                if b.x + 4 >= mx && b.x - 4 <= mx + MON_W && b.y + 4 >= my && b.y - 4 <= my + MON_H {
                    self.monster.active = false;
                    b.active = false;
                    self.bonus += 100;
                    fx = EventFx::Stomp;
                }
            }
        }

        // 5. Pickup collisions (Propeller & Jetpack can be grabbed even while rising!)
        let dx = self.dood_x();
        let dy = self.dood_y();
        if !self.is_flying() {
            for p in &mut self.plats {
                if !p.active || p.broken > 0 {
                    continue;
                }
                match p.pickup {
                    Pickup::Propeller | Pickup::Jetpack => {
                        let px = p.x() + PLAT_W / 2;
                        let py = p.y() - 10;
                        if (dx - px).abs() <= 20 && (dy - DOOD_H / 2 - py).abs() <= 20 {
                            self.power = if p.pickup == Pickup::Propeller {
                                PowerMode::Propeller(PROP_TICKS)
                            } else {
                                PowerMode::Jetpack(JET_TICKS)
                            };
                            p.pickup = Pickup::None;
                            fx = EventFx::Powerup;
                            break;
                        }
                    }
                    _ => {}
                }
            }
        }

        // 6. Monster collision (stomp from above OR crash if flying == false)
        if self.monster.active {
            let mx = self.monster.x();
            let my = self.monster.y();
            let hit_x = dx + 10 >= mx && dx - 10 <= mx + MON_W;
            let hit_y = dy >= my && dy - DOOD_H <= my + MON_H;
            if hit_x && hit_y {
                if self.is_flying() {
                    // Invincible during Propeller/Jetpack: smash right through!
                    self.monster.active = false;
                    self.bonus += 100;
                    fx = EventFx::Stomp;
                } else if self.vy_q > 0 && ((prev_y_q >> 8) <= my + 12) {
                    // Stomp on monster's head!
                    self.monster.active = false;
                    self.vy_q = STOMP_VY;
                    self.bonus += 100;
                    fx = EventFx::Stomp;
                } else {
                    self.state = State::Over;
                    return (EventFx::GameOver, 0);
                }
            }
        }

        // 7. Landing on platforms (ONLY when falling downward and not flying!)
        if self.vy_q > 0 && !self.is_flying() {
            let feet_l = dx - 9;
            let feet_r = dx + 9;
            for p in &mut self.plats {
                if !p.active || p.broken > 0 {
                    continue;
                }
                let px = p.x();
                let py_q = p.y_q;
                // Continuous sweep check so fast falling NEVER tunnels through a platform!
                if feet_r >= px && feet_l <= px + PLAT_W && prev_y_q <= py_q + 6 * Q && self.y_q >= py_q {
                    if p.kind == PlatKind::Brown {
                        p.broken = 1;
                        fx = EventFx::Break;
                        // Brown platform snaps — Doodler keeps falling!
                        continue;
                    }
                    self.y_q = py_q;
                    if p.pickup == Pickup::Spring {
                        p.pickup = Pickup::SpringUsed;
                        self.vy_q = SPRING_VY;
                        fx = EventFx::Spring;
                    } else {
                        self.vy_q = JUMP_VY;
                        if fx == EventFx::None {
                            fx = EventFx::Jump;
                        }
                    }
                    if p.kind == PlatKind::White {
                        p.active = false;
                    }
                    break;
                }
            }
        }

        // 8. Smooth upward camera scroll when Doodler rises above SCROLL_Y
        let mut scrolled_px = 0;
        if self.y_q < SCROLL_Y * Q {
            let delta_q = SCROLL_Y * Q - self.y_q;
            self.y_q = SCROLL_Y * Q;
            self.scroll_q = self.scroll_q.saturating_add(delta_q as u32);
            self.highest_solid_y_q += delta_q;
            scrolled_px = (delta_q + Q / 2) >> 8;

            for p in &mut self.plats {
                if !p.active {
                    continue;
                }
                p.y_q += delta_q;
                if p.y() > SIZE + 6 {
                    p.active = false;
                }
            }
            if self.monster.active {
                self.monster.y_q += delta_q;
                if self.monster.y() > SIZE + 6 {
                    self.monster.active = false;
                }
            }
            self.fill_platforms_upward();
        }

        // 9. Update height + bonus score
        let new_score = (self.scroll_q / (4 * Q as u32)) + self.bonus;
        if new_score > self.score {
            self.score = new_score;
            if self.run_best_start > 0 && !self.beat_best && self.score > self.run_best_start {
                self.beat_best = true;
                fx = EventFx::NewBest;
            }
        }

        // 10. Check fall off bottom of watch screen
        if self.dood_y() - DOOD_H > SIZE + 4 {
            self.state = State::Over;
            return (EventFx::GameOver, scrolled_px);
        }

        (fx, scrolled_px)
    }
}

/// Persistent high score blob: `DOOD` + u32 LE.
const MAGIC: &[u8; 4] = b"DOOD";

pub fn encode_best(best: u32) -> [u8; 8] {
    let mut out = [0u8; 8];
    out[..4].copy_from_slice(MAGIC);
    out[4..8].copy_from_slice(&best.to_le_bytes());
    out
}

pub fn decode_best(blob: &[u8]) -> u32 {
    if blob.len() >= 8 && &blob[..4] == MAGIC {
        u32::from_le_bytes([blob[4], blob[5], blob[6], blob[7]])
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn best_round_trip() {
        assert_eq!(decode_best(&encode_best(2450)), 2450);
        assert_eq!(decode_best(b"NOPE0000"), 0);
    }

    #[test]
    fn landing_on_green_and_spring_bounces() {
        let mut g = Game::new(42);
        g.start(0);
        g.x_q = C * Q;
        g.y_q = 348 * Q;
        g.vy_q = 6 * Q;
        g.plats[0].pickup = Pickup::Spring;
        let (fx, _) = g.step();
        assert_eq!(fx, EventFx::Spring);
        assert_eq!(g.vy_q, SPRING_VY);
        assert_eq!(g.plats[0].pickup, Pickup::SpringUsed);
    }

    #[test]
    fn brown_platform_breaks_and_does_not_bounce() {
        let mut g = Game::new(42);
        g.start(0);
        g.x_q = C * Q;
        g.y_q = 348 * Q;
        g.vy_q = 6 * Q;
        g.plats[0].kind = PlatKind::Brown;
        let (fx, _) = g.step();
        assert_eq!(fx, EventFx::Break);
        assert!(g.vy_q > 0, "brown platform must not bounce doodler");
        assert!(g.plats[0].broken > 0);
    }

    #[test]
    fn bullet_kills_monster_and_awards_bonus() {
        let mut g = Game::new(42);
        g.start(0);
        g.monster = Monster {
            active: true,
            x_q: (C - MON_W / 2) * Q,
            y_q: 260 * Q,
            vx_q: 0,
            variant: 0,
        };
        assert!(g.shoot());
        let mut hit = false;
        for _ in 0..15 {
            let (fx, _) = g.step();
            if fx == EventFx::Stomp {
                hit = true;
                break;
            }
        }
        assert!(hit);
        assert!(!g.monster.active);
        assert!(g.bonus >= 100);
    }
}

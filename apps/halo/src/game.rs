//! Pure fixed-point physics and state for HALO (circular pong for CMF Watch Pro 2).
//! Runs at ~55 FPS (18 ms ticks) with zero floating-point math and zero heap allocations.

use cmf_wp2_ui::motion::{angle, cos, isqrt, sin};

pub const SIZE: i32 = 466;
pub const C: i32 = SIZE / 2;

/// Fixed-point 1.0 px (8 fractional bits).
pub const Q: i32 = 256;

/// Ball & rail geometry (px).
pub const BALL_R: i32 = 9;
pub const RAIL_R0: i32 = 200;
pub const RAIL_R1: i32 = 214;
pub const OUT_R: i32 = C + BALL_R + 4;

/// Ball speed per 18 ms tick (in Q units).
pub const SPEED0: i32 = 2 * Q + 96; // ~2.38 px/tick (~132 px/s)
pub const SPEED_STEP: i32 = 26;
pub const SPEED_MAX: i32 = 6 * Q + 64; // ~6.25 px/tick (~347 px/s)

/// Paddle arc length in u16 turns (65536 = 360 deg).
pub const SPAN0: i32 = 10240; // ~56 deg at score 0
pub const SPAN_STEP: i32 = 100;
pub const SPAN_MIN: i32 = 5461; // 30 deg minimum

/// Crown sensitivity: ~8.8 deg per detent, with gentle 1.25x boost on fast multi-detent flicks.
pub const CROWN_STEP: i32 = 1600;
/// Optional touch steering speed per 18 ms tick (~2.9 deg/tick).
pub const STEER_STEP: i32 = 520;
pub const STEER_DEAD: i32 = 56;

/// Bounce deflection ("english") from hitting off-center on the paddle, plus subtle crown spin carry.
pub const ENGLISH: i32 = 5461; // up to 30 deg
pub const MAX_TILT: i32 = 10500; // ~58 deg max off radial so ball never skims forever

/// Angular half-width of the ball at the inner rail + tiny fair edge grace (~0.7 deg).
const BALL_ANGLE: i32 = (BALL_R * 65536) / (RAIL_R0 * 314 / 50) + 128;

/// Starting paddle center (6 o'clock).
pub const HOME: u16 = 16384;

/// Number of frames the paddle subtly brightens on impact (~110 ms at 55 FPS).
pub const FLASH_FRAMES: u8 = 6;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum State {
    Ready,
    Playing,
    Over,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Step {
    Nothing,
    Scored,
    NewBest,
    Crashed,
}

#[derive(Clone)]
pub struct Game {
    pub state: State,
    /// Ball position relative to screen center, in Q (1/256 px).
    pub x: i32,
    pub y: i32,
    /// Ball velocity per tick, in Q.
    pub vx: i32,
    pub vy: i32,
    pub speed: i32,
    /// Paddle center angle (u16 turns, 0 = 3 o'clock, clockwise).
    pub paddle: u16,
    /// Last frame's paddle angular delta (for subtle slice spin).
    pub paddle_vel: i32,
    /// Optional tap target angle.
    pub target: Option<u16>,
    pub score: u32,
    /// True once the player surpasses `run_best_start` during the current run.
    pub beat_best: bool,
    run_best_start: u32,
    /// Impact highlight countdown (frames).
    pub flash: u8,
    /// Two subtle ghost trail positions (screen px), newest first.
    pub trail: [(i32, i32); 2],
    pub trail_len: u8,
    tick: u32,
    missed: bool,
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

/// Signed angle difference `a - b` in `-32768..=32767`.
#[inline]
pub fn diff(a: u16, b: u16) -> i32 {
    a.wrapping_sub(b) as i16 as i32
}

impl Game {
    pub fn new(seed: u32) -> Self {
        let mut g = Game {
            state: State::Ready,
            x: 0,
            y: 0,
            vx: 0,
            vy: 0,
            speed: SPEED0,
            paddle: HOME,
            paddle_vel: 0,
            target: None,
            score: 0,
            beat_best: false,
            run_best_start: 0,
            flash: 0,
            trail: [(C, C); 2],
            trail_len: 0,
            tick: 0,
            missed: false,
            rng: seed | 1,
        };
        g.restart();
        g
    }

    pub fn restart(&mut self) {
        self.state = State::Ready;
        self.x = 0;
        self.y = 0;
        self.vx = 0;
        self.vy = 0;
        self.speed = SPEED0;
        self.paddle = HOME;
        self.paddle_vel = 0;
        self.target = None;
        self.score = 0;
        self.beat_best = false;
        self.flash = 0;
        self.trail = [(C, C); 2];
        self.trail_len = 0;
        self.tick = 0;
        self.missed = false;
    }

    pub fn launch(&mut self, current_best: u32) {
        if self.state != State::Ready {
            return;
        }
        self.state = State::Playing;
        self.run_best_start = current_best;
        self.beat_best = false;
        self.trail_len = 0;
        self.tick = 0;
        // Serve away from the very edge of the paddle so the first shot feels fair and engaging.
        let r = xorshift(&mut self.rng);
        let offset = ((r & 0x7FFF) as i32) - 16384; // -90..+90 deg around opposite side of paddle
        let serve_angle = self.paddle.wrapping_add(32768).wrapping_add((offset / 2) as u16);
        self.aim(serve_angle);
    }

    fn aim(&mut self, a: u16) {
        self.vx = (self.speed * cos(a)) >> 14;
        self.vy = (self.speed * sin(a)) >> 14;
    }

    #[inline]
    pub fn span(&self) -> i32 {
        (SPAN0 - SPAN_STEP * self.score.min(1000) as i32).max(SPAN_MIN)
    }

    #[inline]
    pub fn ball(&self) -> (i32, i32) {
        (C + ((self.x + Q / 2) >> 8), C + ((self.y + Q / 2) >> 8))
    }

    /// Apply accumulated crown detents from one event-drain batch.
    pub fn apply_crown(&mut self, total_delta: i32) {
        if total_delta == 0 {
            return;
        }
        self.target = None;
        let mag = total_delta.abs();
        // Smooth adaptive boost when spinning fast (3+ detents in one 18ms frame)
        let step = if mag >= 3 {
            total_delta * CROWN_STEP * 5 / 4
        } else {
            total_delta * CROWN_STEP
        };
        self.paddle = self.paddle.wrapping_add(step as u16);
        self.paddle_vel = step.clamp(-8192, 8192);
    }

    /// Optional tap steering toward the touched angle on the rim.
    pub fn steer(&mut self, px: i32, py: i32) {
        let (dx, dy) = (px - C, py - C);
        if dx * dx + dy * dy >= STEER_DEAD * STEER_DEAD {
            self.target = Some(angle(dx, dy));
        }
    }

    /// Advance simulation by one 18 ms tick.
    pub fn step(&mut self) -> Step {
        if self.state != State::Playing {
            return Step::Nothing;
        }

        if self.flash > 0 {
            self.flash -= 1;
        }

        // Smooth touch steering if active
        if let Some(t) = self.target {
            let d = diff(t, self.paddle);
            if d.abs() <= STEER_STEP {
                self.paddle = t;
                self.target = None;
                self.paddle_vel = d;
            } else {
                let mv = if d > 0 { STEER_STEP } else { -STEER_STEP };
                self.paddle = self.paddle.wrapping_add(mv as u16);
                self.paddle_vel = mv;
            }
        } else {
            // Decay crown spin carry gently
            self.paddle_vel = self.paddle_vel * 2 / 3;
        }

        // Record subtle motion trail every 2nd tick before moving
        self.tick = self.tick.wrapping_add(1);
        if self.tick & 1 == 1 {
            let cur = self.ball();
            self.trail[1] = self.trail[0];
            self.trail[0] = cur;
            if self.trail_len < 2 {
                self.trail_len += 1;
            }
        }

        // Move ball
        self.x += self.vx;
        self.y += self.vy;

        // Radius in integer px (Q -> px)
        let rx = self.x >> 8;
        let ry = self.y >> 8;
        let dist_sq = rx * rx + ry * ry;

        if self.missed {
            if dist_sq >= OUT_R * OUT_R {
                self.state = State::Over;
                return Step::Crashed;
            }
            return Step::Nothing;
        }

        let hit_r = RAIL_R0 - BALL_R;
        if dist_sq < hit_r * hit_r {
            return Step::Nothing;
        }

        // Only collide when moving outward
        let outward = self.x as i64 * self.vx as i64 + self.y as i64 * self.vy as i64;
        if outward <= 0 {
            return Step::Nothing;
        }

        let ball_a = angle(rx, ry);
        let d = diff(ball_a, self.paddle);
        let half = self.span() / 2;

        if d.abs() <= half + BALL_ANGLE {
            // Clean bounce!
            self.score += 1;
            self.speed = (self.speed + SPEED_STEP).min(SPEED_MAX);
            self.flash = FLASH_FRAMES;

            // Clamp position cleanly inside the rail so it never clips or double-bounces
            let dist = isqrt(dist_sq).max(1);
            let safe_r = (hit_r - 1) * Q;
            self.x = rx * safe_r / dist;
            self.y = ry * safe_r / dist;

            // Compute outgoing angle: reflect incoming vector around inward normal + english + tiny spin
            let inward = ball_a.wrapping_add(32768);
            let incoming = angle(self.vx, self.vy);
            // Mirror incoming across the radial line (ball_a -> inward)
            let mirrored = inward.wrapping_sub(diff(incoming, ball_a) as u16);
            // Blend mirrored bounce with position-based english & subtle paddle spin
            let english = (d.clamp(-half, half) * ENGLISH) / half.max(1);
            let spin = (self.paddle_vel / 8).clamp(-1800, 1800);
            let raw_tilt = diff(mirrored, inward) * 3 / 5 + english + spin;
            let tilt = raw_tilt.clamp(-MAX_TILT, MAX_TILT);
            self.aim(inward.wrapping_add(tilt as u16));

            if self.run_best_start > 0 && !self.beat_best && self.score > self.run_best_start {
                self.beat_best = true;
                return Step::NewBest;
            }
            return Step::Scored;
        }

        // Ball passed outside the paddle's arc
        self.missed = true;
        if dist_sq >= OUT_R * OUT_R {
            self.state = State::Over;
            return Step::Crashed;
        }
        Step::Nothing
    }
}

/// Persistent high score format: `HALO` + u32 LE (8 bytes).
const MAGIC: &[u8; 4] = b"HALO";

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
        let raw = encode_best(42);
        assert_eq!(decode_best(&raw), 42);
        assert_eq!(decode_best(b"NOPE1234"), 0);
        assert_eq!(decode_best(&[]), 0);
    }

    #[test]
    fn bounce_scores_and_speeds_up() {
        let mut g = Game::new(12345);
        g.launch(5);
        // Aim straight at paddle (HOME = 16384 -> +y down)
        g.x = 0;
        g.y = (RAIL_R0 - BALL_R - 1) * Q;
        g.vx = 0;
        g.vy = g.speed;
        assert_eq!(g.step(), Step::Scored);
        assert_eq!(g.score, 1);
        assert!(g.vy < 0, "ball must bounce inward");
        assert_eq!(g.flash, FLASH_FRAMES);
    }

    #[test]
    fn miss_ends_game() {
        let mut g = Game::new(99);
        g.launch(0);
        // Aim straight opposite the paddle (12 o'clock, -y)
        g.x = 0;
        g.y = -(RAIL_R0 - BALL_R) * Q;
        g.vx = 0;
        g.vy = -g.speed;
        let mut crashed = false;
        for _ in 0..40 {
            if g.step() == Step::Crashed {
                crashed = true;
                break;
            }
        }
        assert!(crashed);
        assert_eq!(g.state, State::Over);
    }
}

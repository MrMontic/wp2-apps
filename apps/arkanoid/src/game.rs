//! Pure no_std, zero-float fixed-point Arkanoid / Brick Breaker engine.

use crate::levels::{Brick, BrickKind, COLS, GRID_LEN, LEVELS, NUM_LEVELS, ROWS, load_level};
use cmf_wp2_ui::motion::{Q14, cos, isqrt, sin};

pub const SIZE: i32 = cmf_wp2_ui::SIZE;
pub const C: i32 = cmf_wp2_ui::C;

/// Playfield framed cleanly inside the 466×466 round watch bezel.
pub const PLAY_L: i32 = 56;
pub const PLAY_R: i32 = 410;
pub const PLAY_T: i32 = 74;
pub const PLAY_B: i32 = 398;

/// Brick wall layout: 10 cols × 8 rows, 100% visible inside circular glass.
pub const BRICK_W: i32 = 33;
pub const BRICK_H: i32 = 16;
pub const BRICK_GAP: i32 = 2;
pub const GRID_X0: i32 = PLAY_L + 3; // 59 .. 407 (348px)
pub const GRID_Y0: i32 = PLAY_T + 14; // 88 .. 230

/// Paddle & Safety Shield geometry.
pub const PADDLE_Y: i32 = 374;
pub const PADDLE_H: i32 = 12;
pub const PADDLE_W_BASE: i32 = 72;
pub const PADDLE_W_WIDE: i32 = 112;
pub const SHIELD_Y: i32 = 394;

/// Ball geometry & fixed-point scale (1 px = 256 Q-units).
pub const BALL_R: i32 = 5;
pub const Q: i32 = 256;
const SUBSTEPS: i32 = 2;

/// Entity caps (compact, zero heap, 8 KiB stack safe).
pub const MAX_BALLS: usize = 8;
pub const MAX_CAPSULES: usize = 8;
pub const MAX_LASERS: usize = 16;
pub const MAX_PARTICLES: usize = 28;

pub const START_LIVES: u8 = 3;
pub const MAX_LIVES: u8 = 5;

/// Power-up durations in ticks (~55 FPS, 18ms/tick).
pub const WIDE_DURATION: u16 = 770; // ~14s
pub const FIRE_DURATION: u16 = 555; // ~10s
pub const LASER_DURATION: u16 = 665; // ~12s
pub const SHIELD_DURATION: u16 = 830; // ~15s
pub const SLOW_DURATION: u16 = 610; // ~11s
pub const LASER_AUTO_INTERVAL: u8 = 25; // ~0.45s
pub const LASER_TAP_COOLDOWN: u8 = 8;

/// Capsule dimensions.
pub const CAP_W: i32 = 22;
pub const CAP_H: i32 = 13;

/// Laser bolt dimensions.
pub const LASER_W: i32 = 4;
pub const LASER_H: i32 = 12;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum State {
    Title,
    Playing,
    StageClear,
    GameOver,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum PowerKind {
    Multi = 0,  // 'M' ×3 Multi-ball (up to 8)
    Wide = 1,   // 'W' Wide paddle
    Fire = 2,   // 'F' Fireball pass-through
    Laser = 3,  // 'L' Twin laser cannons
    Shield = 4, // 'S' Safety energy floor
    Slow = 5,   // 'C' Slow-mo precision
    Life = 6,   // '+' Extra life
}

impl PowerKind {
    pub const fn letter(self) -> u8 {
        match self {
            PowerKind::Multi => b'M',
            PowerKind::Wide => b'W',
            PowerKind::Fire => b'F',
            PowerKind::Laser => b'L',
            PowerKind::Shield => b'S',
            PowerKind::Slow => b'C',
            PowerKind::Life => b'+',
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            PowerKind::Multi => "MULTI x3",
            PowerKind::Wide => "WIDE PAD",
            PowerKind::Fire => "FIREBALL",
            PowerKind::Laser => "LASERS",
            PowerKind::Shield => "SHIELD",
            PowerKind::Slow => "SLOW-MO",
            PowerKind::Life => "+1 LIFE",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Ball {
    pub active: bool,
    pub glued: bool,
    pub x_q: i32,
    pub y_q: i32,
    pub vx_q: i32,
    pub vy_q: i32,
    pub glue_off: i16,
    pub stuck_bounces: u8,
}

impl Ball {
    pub const EMPTY: Self = Self {
        active: false,
        glued: false,
        x_q: 0,
        y_q: 0,
        vx_q: 0,
        vy_q: 0,
        glue_off: 0,
        stuck_bounces: 0,
    };

    #[inline]
    pub const fn px(self) -> i32 {
        self.x_q >> 8
    }

    #[inline]
    pub const fn py(self) -> i32 {
        self.y_q >> 8
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Capsule {
    pub active: bool,
    pub kind: PowerKind,
    pub x: i16,
    pub y_q: i32,
}

impl Capsule {
    pub const EMPTY: Self = Self {
        active: false,
        kind: PowerKind::Multi,
        x: 0,
        y_q: 0,
    };

    #[inline]
    pub const fn py(self) -> i32 {
        self.y_q >> 8
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Laser {
    pub active: bool,
    pub x: i16,
    pub y: i16,
}

impl Laser {
    pub const EMPTY: Self = Self {
        active: false,
        x: 0,
        y: 0,
    };
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Particle {
    pub active: bool,
    pub x_q: i32,
    pub y_q: i32,
    pub vx_q: i16,
    pub vy_q: i16,
    pub life: u8,
    pub color: u16,
}

impl Particle {
    pub const EMPTY: Self = Self {
        active: false,
        x_q: 0,
        y_q: 0,
        vx_q: 0,
        vy_q: 0,
        life: 0,
        color: 0,
    };

    #[inline]
    pub const fn px(self) -> i32 {
        self.x_q >> 8
    }

    #[inline]
    pub const fn py(self) -> i32 {
        self.y_q >> 8
    }
}

/// Priority-ordered haptic/event feedback returned per simulation tick.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EventFx {
    None = 0,
    Wall = 1,
    Paddle = 2,
    Laser = 3,
    Brick = 4,
    Shield = 5,
    Powerup = 6,
    Explosion = 7,
    LoseLife = 8,
    StageClear = 9,
    GameOver = 10,
}

impl EventFx {
    #[inline]
    fn upgrade(self, next: EventFx) -> EventFx {
        if (next as u8) > (self as u8) { next } else { self }
    }
}

/// Persistent progress saved via `api.save` / `api.load`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SaveData {
    pub best_score: u32,
    pub max_level: u8, // 1..=12
    pub best_wave: u8, // 1..=255
}

impl SaveData {
    pub const DEFAULT: Self = Self {
        best_score: 0,
        max_level: 1,
        best_wave: 1,
    };
}

const SAVE_MAGIC: [u8; 4] = *b"ARK1";
pub const SAVE_LEN: usize = 16;

fn save_checksum(bytes: &[u8; 12]) -> u32 {
    let mut h: u32 = 0x811C_9DC5;
    let mut i = 0;
    while i < 12 {
        h ^= bytes[i] as u32;
        h = h.wrapping_mul(0x0100_0193);
        i += 1;
    }
    h ^ 0xA55A_5AA5
}

pub fn encode_save(data: SaveData) -> [u8; SAVE_LEN] {
    let mut out = [0u8; SAVE_LEN];
    let mut head = [0u8; 12];
    head[0..4].copy_from_slice(&SAVE_MAGIC);
    head[4..8].copy_from_slice(&data.best_score.to_le_bytes());
    head[8] = data.max_level.clamp(1, NUM_LEVELS as u8);
    head[9] = data.best_wave.max(1);
    head[10] = 0;
    head[11] = 0;
    let csum = save_checksum(&head);
    out[0..12].copy_from_slice(&head);
    out[12..16].copy_from_slice(&csum.to_le_bytes());
    out
}

pub fn decode_save(buf: &[u8]) -> Option<SaveData> {
    if buf.len() < SAVE_LEN || buf[0..4] != SAVE_MAGIC {
        return None;
    }
    let mut head = [0u8; 12];
    head.copy_from_slice(&buf[0..12]);
    let stored = u32::from_le_bytes([buf[12], buf[13], buf[14], buf[15]]);
    if stored != save_checksum(&head) {
        return None;
    }
    let max_level = head[8];
    let best_wave = head[9];
    if max_level == 0 || max_level > NUM_LEVELS as u8 || best_wave == 0 {
        return None;
    }
    let best_score = u32::from_le_bytes([head[4], head[5], head[6], head[7]]);
    Some(SaveData {
        best_score,
        max_level,
        best_wave,
    })
}

#[inline]
pub const fn brick_rect(r: usize, c: usize) -> (i32, i32, i32, i32) {
    (
        GRID_X0 + c as i32 * (BRICK_W + BRICK_GAP),
        GRID_Y0 + r as i32 * (BRICK_H + BRICK_GAP),
        BRICK_W,
        BRICK_H,
    )
}

#[inline]
pub const fn brick_spark_color(kind: BrickKind) -> u16 {
    use cmf_wp2_ui::color::rgb565;
    match kind {
        BrickKind::Crimson => rgb565(255, 68, 102),
        BrickKind::Orange => rgb565(255, 92, 32),
        BrickKind::Gold => rgb565(255, 214, 52),
        BrickKind::Emerald => rgb565(46, 232, 140),
        BrickKind::Cyan => rgb565(48, 224, 255),
        BrickKind::Violet => rgb565(192, 102, 255),
        BrickKind::Silver => rgb565(212, 224, 238),
        BrickKind::Titanium => rgb565(126, 170, 216),
        BrickKind::Tnt => rgb565(255, 168, 36),
        BrickKind::Bumper => rgb565(255, 228, 128),
        BrickKind::Empty => rgb565(255, 255, 255),
    }
}

pub struct Game {
    pub state: State,
    pub tick: u32,
    pub rng: u32,

    pub save: SaveData,
    pub selected_level: u8, // 1..=12 on Title screen

    pub level: u8, // 0..11 active index
    pub wave: u8,  // 0 = Wave 1, 1 = Wave 2, ...
    pub score: u32,
    pub stage_bonus: u32,
    pub beat_best: bool,
    pub lives: u8,
    pub combo: u8,
    pub bricks_left: u16,

    // Paddle
    pub paddle_x: i32, // center X
    pub paddle_w: i32,
    pub touch_dir: i8, // -1 left, 0 none, +1 right while touch held

    // Power-up timers (in ticks)
    pub wide_ticks: u16,
    pub fire_ticks: u16,
    pub laser_ticks: u16,
    pub shield_ticks: u16,
    pub shield_hits: u8,
    pub slow_ticks: u16,
    pub laser_auto_cd: u8,
    pub laser_tap_cd: u8,
    pub banner_kind: Option<PowerKind>,
    pub banner_ticks: u8,

    // Entities
    pub bricks: [Brick; GRID_LEN],
    pub balls: [Ball; MAX_BALLS],
    pub capsules: [Capsule; MAX_CAPSULES],
    pub lasers: [Laser; MAX_LASERS],
    pub particles: [Particle; MAX_PARTICLES],
}

impl Game {
    pub const fn new_const() -> Self {
        Self {
            state: State::Title,
            tick: 0,
            rng: 0x1234_5678,
            save: SaveData::DEFAULT,
            selected_level: 1,
            level: 0,
            wave: 0,
            score: 0,
            stage_bonus: 0,
            beat_best: false,
            lives: START_LIVES,
            combo: 0,
            bricks_left: 0,
            paddle_x: C,
            paddle_w: PADDLE_W_BASE,
            touch_dir: 0,
            wide_ticks: 0,
            fire_ticks: 0,
            laser_ticks: 0,
            shield_ticks: 0,
            shield_hits: 0,
            slow_ticks: 0,
            laser_auto_cd: 0,
            laser_tap_cd: 0,
            banner_kind: None,
            banner_ticks: 0,
            bricks: [Brick::EMPTY; GRID_LEN],
            balls: [Ball::EMPTY; MAX_BALLS],
            capsules: [Capsule::EMPTY; MAX_CAPSULES],
            lasers: [Laser::EMPTY; MAX_LASERS],
            particles: [Particle::EMPTY; MAX_PARTICLES],
        }
    }

    pub fn init(&mut self, seed: u32, save: SaveData) {
        *self = Self::new_const();
        self.rng = if seed == 0 { 0xA341_316C } else { seed };
        self.save = SaveData {
            best_score: save.best_score,
            max_level: save.max_level.clamp(1, NUM_LEVELS as u8),
            best_wave: save.best_wave.max(1),
        };
        self.selected_level = self.save.max_level.clamp(1, NUM_LEVELS as u8);
        self.state = State::Title;
    }

    #[inline]
    pub fn next_rand(&mut self) -> u32 {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        x
    }

    #[inline]
    pub fn level_name(&self) -> &'static str {
        LEVELS[(self.level as usize) % NUM_LEVELS].name
    }

    #[inline]
    pub fn selected_level_name(&self) -> &'static str {
        let idx = self.selected_level.saturating_sub(1) as usize % NUM_LEVELS;
        LEVELS[idx].name
    }

    /// Current ball target speed in Q-units per tick.
    pub fn target_speed_q(&self) -> i32 {
        let base = (1040 + self.level as i32 * 30 + self.wave as i32 * 135).min(1660);
        if self.slow_ticks > 0 {
            (base * 5 / 8).max(700)
        } else {
            base
        }
    }

    /// Title screen level selection (only cycles through unlocked levels 1..=max_level).
    pub fn select_level_delta(&mut self, delta: i32) -> bool {
        let max_l = self.save.max_level.clamp(1, NUM_LEVELS as u8) as i32;
        if max_l <= 1 {
            return false;
        }
        let cur = (self.selected_level as i32 - 1).rem_euclid(max_l);
        let next = (cur + delta).rem_euclid(max_l) + 1;
        if next as u8 != self.selected_level {
            self.selected_level = next as u8;
            true
        } else {
            false
        }
    }

    /// Starts a new run from `selected_level`.
    pub fn start_new_run(&mut self) {
        self.score = 0;
        self.stage_bonus = 0;
        self.beat_best = false;
        self.lives = START_LIVES;
        self.wave = 0;
        self.level = self.selected_level.saturating_sub(1) % NUM_LEVELS as u8;
        self.setup_stage();
        self.state = State::Playing;
    }

    /// Advances from StageClear to the next level (wrapping into Wave 2+ after Level 12).
    pub fn next_stage(&mut self) {
        if self.level + 1 >= NUM_LEVELS as u8 {
            self.level = 0;
            self.wave = self.wave.saturating_add(1);
            let w_num = self.wave.saturating_add(1);
            if w_num > self.save.best_wave {
                self.save.best_wave = w_num;
            }
        } else {
            self.level += 1;
        }
        self.setup_stage();
        self.state = State::Playing;
    }

    pub fn setup_stage(&mut self) {
        self.bricks_left = load_level(self.level as usize, self.wave, &mut self.bricks);
        self.combo = 0;
        self.touch_dir = 0;
        self.paddle_x = C;
        self.paddle_w = PADDLE_W_BASE;
        self.clear_powerups();
        self.capsules = [Capsule::EMPTY; MAX_CAPSULES];
        self.lasers = [Laser::EMPTY; MAX_LASERS];
        self.particles = [Particle::EMPTY; MAX_PARTICLES];
        self.spawn_glued_ball();
    }

    fn clear_powerups(&mut self) {
        self.wide_ticks = 0;
        self.fire_ticks = 0;
        self.laser_ticks = 0;
        self.shield_ticks = 0;
        self.shield_hits = 0;
        self.slow_ticks = 0;
        self.laser_auto_cd = 0;
        self.laser_tap_cd = 0;
        self.banner_kind = None;
        self.banner_ticks = 0;
        self.paddle_w = PADDLE_W_BASE;
        self.clamp_paddle();
    }

    pub fn spawn_glued_ball(&mut self) {
        self.balls = [Ball::EMPTY; MAX_BALLS];
        self.balls[0] = Ball {
            active: true,
            glued: true,
            x_q: self.paddle_x * Q,
            y_q: (PADDLE_Y - BALL_R - 1) * Q,
            vx_q: 0,
            vy_q: 0,
            glue_off: 0,
            stuck_bounces: 0,
        };
    }

    #[inline]
    pub fn has_glued_ball(&self) -> bool {
        self.balls.iter().any(|b| b.active && b.glued)
    }

    #[inline]
    pub fn active_ball_count(&self) -> usize {
        self.balls.iter().filter(|b| b.active).count()
    }

    /// Launches any glued balls upward with a slight arcade angle.
    pub fn launch_ball(&mut self) -> bool {
        let spd = self.target_speed_q();
        let mut launched = false;
        let tilt: i32 = if (self.next_rand() & 1) == 0 { 3200 } else { -3200 }; // ~±17.5°
        for i in 0..MAX_BALLS {
            if self.balls[i].active && self.balls[i].glued {
                let ang = (49152i32 + tilt) as u16;
                self.balls[i].glued = false;
                self.balls[i].vx_q = (cos(ang) * spd) / Q14;
                self.balls[i].vy_q = (sin(ang) * spd) / Q14;
                self.balls[i].stuck_bounces = 0;
                launched = true;
            }
        }
        launched
    }

    #[inline]
    fn clamp_paddle(&mut self) {
        let half = self.paddle_w / 2;
        let min_x = PLAY_L + 2 + half;
        let max_x = PLAY_R - 2 - half;
        self.paddle_x = self.paddle_x.clamp(min_x, max_x);
    }

    /// Applies accumulated crown detents in one crisp step.
    pub fn apply_crown(&mut self, delta: i32) {
        if self.state == State::Title {
            if delta != 0 {
                self.select_level_delta(delta.signum());
            }
            return;
        }
        if self.state != State::Playing || delta == 0 {
            return;
        }
        self.paddle_x += delta * 12;
        self.clamp_paddle();
        self.sync_glued_balls();
    }

    /// Handles touch down during play: launches glued ball, fires lasers, and starts smooth touch steering.
    pub fn touch_down(&mut self, x: i32) -> EventFx {
        if self.state != State::Playing {
            return EventFx::None;
        }
        if self.has_glued_ball() {
            if self.launch_ball() {
                return EventFx::Paddle;
            }
        }
        let mut fx = EventFx::None;
        if self.laser_ticks > 0 && self.laser_tap_cd == 0 {
            if self.fire_lasers() {
                self.laser_tap_cd = LASER_TAP_COOLDOWN;
                fx = fx.upgrade(EventFx::Laser);
            }
        }
        // Smooth touch steering: left or right side of watch nudges & holds glide direction.
        if x < C - 22 {
            self.touch_dir = -1;
            self.paddle_x -= 10;
            self.clamp_paddle();
        } else if x > C + 22 {
            self.touch_dir = 1;
            self.paddle_x += 10;
            self.clamp_paddle();
        } else {
            self.touch_dir = 0;
        }
        fx
    }

    pub fn touch_up(&mut self) {
        self.touch_dir = 0;
    }

    fn sync_glued_balls(&mut self) {
        for b in &mut self.balls {
            if b.active && b.glued {
                b.x_q = (self.paddle_x + b.glue_off as i32) * Q;
                b.y_q = (PADDLE_Y - BALL_R - 1) * Q;
            }
        }
    }

    /// Fires twin laser bolts upward from the left and right edges of the paddle.
    pub fn fire_lasers(&mut self) -> bool {
        let half = self.paddle_w / 2;
        let lx = (self.paddle_x - half + 5) as i16;
        let rx = (self.paddle_x + half - 5) as i16;
        let ly = (PADDLE_Y - 10) as i16;
        let mut spawned = false;
        for &sx in &[lx, rx] {
            for slot in &mut self.lasers {
                if !slot.active {
                    *slot = Laser {
                        active: true,
                        x: sx,
                        y: ly,
                    };
                    spawned = true;
                    break;
                }
            }
        }
        spawned
    }

    /// Applies a collected power-up capsule.
    pub fn apply_powerup(&mut self, kind: PowerKind) {
        self.banner_kind = Some(kind);
        self.banner_ticks = 95;
        self.add_score(100);
        match kind {
            PowerKind::Multi => {
                // Launch any glued ball first so Multi-ball immediately erupts!
                if self.has_glued_ball() {
                    self.launch_ball();
                }
                self.trigger_multiball();
            }
            PowerKind::Wide => {
                self.wide_ticks = WIDE_DURATION;
                self.paddle_w = PADDLE_W_WIDE;
                self.clamp_paddle();
            }
            PowerKind::Fire => {
                self.fire_ticks = FIRE_DURATION;
            }
            PowerKind::Laser => {
                self.laser_ticks = LASER_DURATION;
                self.laser_auto_cd = 6;
            }
            PowerKind::Shield => {
                self.shield_ticks = SHIELD_DURATION;
                self.shield_hits = 2;
            }
            PowerKind::Slow => {
                self.slow_ticks = SLOW_DURATION;
                self.rescale_all_balls();
            }
            PowerKind::Life => {
                if self.lives < MAX_LIVES {
                    self.lives += 1;
                } else {
                    self.add_score(500);
                }
            }
        }
    }

    /// Splits existing active balls into 3 diverging angles, up to `MAX_BALLS = 8`.
    pub fn trigger_multiball(&mut self) {
        let spd = self.target_speed_q();
        let mut snapshot = [Ball::EMPTY; MAX_BALLS];
        let mut src_count = 0usize;
        for &b in &self.balls {
            if b.active && !b.glued {
                snapshot[src_count] = b;
                src_count += 1;
            }
        }
        if src_count == 0 {
            return;
        }
        // Try to add 2 angled clones per existing ball until MAX_BALLS is reached.
        let offsets: [i32; 2] = [-4551, 4551]; // ±25 degrees
        for s_idx in 0..src_count {
            let src = snapshot[s_idx];
            let base_ang = cmf_wp2_ui::motion::angle(src.vx_q, src.vy_q) as i32;
            for &da in &offsets {
                let Some(slot) = self.balls.iter_mut().find(|b| !b.active) else {
                    return;
                };
                let ang = (base_ang + da) as u16;
                let mut vx = (cos(ang) * spd) / Q14;
                let mut vy = (sin(ang) * spd) / Q14;
                // Keep newly spawned balls moving noticeably vertically
                let min_vy = spd / 3;
                if vy.abs() < min_vy {
                    vy = if vy <= 0 { -min_vy } else { min_vy };
                    let rem = (spd * spd - vy * vy).max(1);
                    let mag = isqrt(rem);
                    vx = if vx < 0 { -mag } else { mag };
                }
                *slot = Ball {
                    active: true,
                    glued: false,
                    x_q: src.x_q,
                    y_q: src.y_q,
                    vx_q: vx,
                    vy_q: vy,
                    glue_off: 0,
                    stuck_bounces: 0,
                };
            }
        }
    }

    fn rescale_all_balls(&mut self) {
        let spd = self.target_speed_q();
        for b in &mut self.balls {
            if b.active && !b.glued {
                normalize_velocity(&mut b.vx_q, &mut b.vy_q, spd);
            }
        }
    }

    fn add_score(&mut self, pts: u32) {
        self.score = self.score.saturating_add(pts);
        if self.score > self.save.best_score {
            self.save.best_score = self.score;
            self.beat_best = true;
        }
    }

    /// Spawns a burst of crisp sparks centered at `(cx, cy)`.
    fn spawn_sparks(&mut self, cx: i32, cy: i32, color: u16, count: usize) {
        let mut spawned = 0;
        for i in 0..MAX_PARTICLES {
            if !self.particles[i].active {
                let r1 = self.next_rand();
                let r2 = self.next_rand();
                let vx = ((r1 & 0x3FF) as i32 - 512) as i16; // -2..+2 px/tick
                let vy = ((r2 & 0x3FF) as i32 - 640) as i16; // bias upward
                let life = 10 + (r1.wrapping_shr(12) & 7) as u8;
                self.particles[i] = Particle {
                    active: true,
                    x_q: cx * Q,
                    y_q: cy * Q,
                    vx_q: vx,
                    vy_q: vy,
                    life,
                    color,
                };
                spawned += 1;
                if spawned >= count {
                    break;
                }
            }
        }
    }

    fn maybe_spawn_capsule(&mut self, cx: i32, cy: i32, boosted: bool) {
        let roll = self.next_rand() % 100;
        let threshold = if boosted { 36 } else { 24 };
        if roll >= threshold {
            return;
        }
        let Some(slot_idx) = self.capsules.iter().position(|c| !c.active) else {
            return;
        };
        // Weighted fun distribution across all 7 power-ups!
        let pick = self.next_rand() % 100;
        let kind = if pick < 22 {
            PowerKind::Multi
        } else if pick < 38 {
            PowerKind::Wide
        } else if pick < 54 {
            PowerKind::Fire
        } else if pick < 70 {
            PowerKind::Laser
        } else if pick < 83 {
            PowerKind::Shield
        } else if pick < 93 {
            PowerKind::Slow
        } else {
            PowerKind::Life
        };
        let clamped_x = cx.clamp(PLAY_L + CAP_W / 2 + 2, PLAY_R - CAP_W / 2 - 2);
        self.capsules[slot_idx] = Capsule {
            active: true,
            kind,
            x: clamped_x as i16,
            y_q: cy * Q,
        };
    }

    /// Damages brick at `(r, c)`. Handles TNT chain-reaction explosions via a compact 80-byte BFS queue!
    pub fn damage_brick(&mut self, r: usize, c: usize, dmg: u8, allow_drops: bool) -> EventFx {
        let idx = r * COLS + c;
        let b = self.bricks[idx];
        if !b.alive() {
            return EventFx::None;
        }
        if b.kind == BrickKind::Bumper {
            self.bricks[idx].flash = 6;
            let (bx, by, bw, bh) = brick_rect(r, c);
            self.spawn_sparks(bx + bw / 2, by + bh / 2, brick_spark_color(BrickKind::Bumper), 3);
            return EventFx::Wall;
        }

        if b.hp > dmg {
            self.bricks[idx].hp -= dmg;
            self.bricks[idx].flash = 5;
            let (bx, by, bw, bh) = brick_rect(r, c);
            self.spawn_sparks(bx + bw / 2, by + bh / 2, brick_spark_color(b.kind), 3);
            return EventFx::Brick;
        }

        // Brick is destroyed! Check if TNT triggers a chain reaction.
        if b.kind == BrickKind::Tnt {
            self.detonate_tnt(r, c, allow_drops);
            return EventFx::Explosion;
        }

        self.destroy_single_brick(r, c, allow_drops);
        EventFx::Brick
    }

    fn destroy_single_brick(&mut self, r: usize, c: usize, allow_drops: bool) {
        let idx = r * COLS + c;
        let kind = self.bricks[idx].kind;
        if !self.bricks[idx].alive() || !kind.is_breakable() {
            return;
        }
        self.bricks[idx] = Brick::EMPTY;
        self.bricks_left = self.bricks_left.saturating_sub(1);
        self.combo = self.combo.saturating_add(1).min(8);
        let mult = self.combo.max(1) as u32;
        self.add_score(kind.base_points() * mult);

        let (bx, by, bw, bh) = brick_rect(r, c);
        let cx = bx + bw / 2;
        let cy = by + bh / 2;
        self.spawn_sparks(cx, cy, brick_spark_color(kind), 5);
        if allow_drops {
            let boosted = matches!(kind, BrickKind::Silver | BrickKind::Titanium | BrickKind::Tnt);
            self.maybe_spawn_capsule(cx, cy, boosted);
        }
    }

    /// Explosive TNT chain reaction: destroys all 8 surrounding bricks and cascades into adjacent TNTs!
    fn detonate_tnt(&mut self, start_r: usize, start_c: usize, allow_drops: bool) {
        let mut queue = [0u8; GRID_LEN];
        let mut head = 0usize;
        let mut tail = 0usize;

        let start_idx = start_r * COLS + start_c;
        if !self.bricks[start_idx].alive() {
            return;
        }
        self.destroy_single_brick(start_r, start_c, allow_drops);
        queue[tail] = start_idx as u8;
        tail += 1;

        while head < tail {
            let cur = queue[head] as usize;
            head += 1;
            let cr = cur / COLS;
            let cc = cur % COLS;
            let (bx, by, bw, bh) = brick_rect(cr, cc);
            self.spawn_sparks(bx + bw / 2, by + bh / 2, brick_spark_color(BrickKind::Tnt), 7);

            let r0 = cr.saturating_sub(1);
            let r1 = (cr + 1).min(ROWS - 1);
            let c0 = cc.saturating_sub(1);
            let c1 = (cc + 1).min(COLS - 1);

            for nr in r0..=r1 {
                for nc in c0..=c1 {
                    if nr == cr && nc == cc {
                        continue;
                    }
                    let nidx = nr * COLS + nc;
                    let nb = self.bricks[nidx];
                    if !nb.alive() {
                        continue;
                    }
                    if nb.kind == BrickKind::Bumper {
                        self.bricks[nidx].flash = 8;
                    } else if nb.kind == BrickKind::Tnt {
                        self.destroy_single_brick(nr, nc, allow_drops);
                        if tail < GRID_LEN {
                            queue[tail] = nidx as u8;
                            tail += 1;
                        }
                    } else {
                        // Blast obliterates even Titanium 3-HP armor!
                        self.destroy_single_brick(nr, nc, allow_drops);
                    }
                }
            }
        }
    }

    /// Advances simulation by one 18ms tick (~55 FPS).
    pub fn step(&mut self) -> EventFx {
        self.tick = self.tick.wrapping_add(1);
        if self.state != State::Playing {
            return EventFx::None;
        }

        let mut fx = EventFx::None;

        // 1. Touch-hold smooth paddle glide
        if self.touch_dir != 0 {
            self.paddle_x += self.touch_dir as i32 * 6;
            self.clamp_paddle();
            self.sync_glued_balls();
        }

        // 2. Decay brick hit flashes
        for b in &mut self.bricks {
            if b.flash > 0 {
                b.flash -= 1;
            }
        }

        // 3. Update power-up timers
        if self.wide_ticks > 0 {
            self.wide_ticks -= 1;
            if self.wide_ticks == 0 {
                self.paddle_w = PADDLE_W_BASE;
                self.clamp_paddle();
            }
        }
        if self.fire_ticks > 0 {
            self.fire_ticks -= 1;
        }
        if self.laser_ticks > 0 {
            self.laser_ticks -= 1;
            if self.laser_tap_cd > 0 {
                self.laser_tap_cd -= 1;
            }
            if !self.has_glued_ball() {
                if self.laser_auto_cd == 0 {
                    if self.fire_lasers() {
                        fx = fx.upgrade(EventFx::Laser);
                    }
                    self.laser_auto_cd = LASER_AUTO_INTERVAL;
                } else {
                    self.laser_auto_cd -= 1;
                }
            }
        }
        if self.shield_ticks > 0 {
            self.shield_ticks -= 1;
            if self.shield_ticks == 0 {
                self.shield_hits = 0;
            }
        }
        if self.slow_ticks > 0 {
            self.slow_ticks -= 1;
            if self.slow_ticks == 0 {
                self.rescale_all_balls();
            }
        }
        if self.banner_ticks > 0 {
            self.banner_ticks -= 1;
            if self.banner_ticks == 0 {
                self.banner_kind = None;
            }
        }

        // 4. Update laser bolts
        fx = fx.upgrade(self.step_lasers());

        // 5. Update balls (sub-stepped for zero tunneling!)
        fx = fx.upgrade(self.step_balls());

        // 6. Update falling power-up capsules
        fx = fx.upgrade(self.step_capsules());

        // 7. Update shatter particles
        self.step_particles();

        // 8. Check Stage Clear!
        if self.bricks_left == 0 {
            let bonus = 500 + (self.level as u32 + 1) * 100 + self.lives as u32 * 150;
            self.stage_bonus = bonus;
            self.add_score(bonus);
            let unlocked = (self.level + 2).min(NUM_LEVELS as u8);
            if unlocked > self.save.max_level {
                self.save.max_level = unlocked;
            }
            self.state = State::StageClear;
            return EventFx::StageClear;
        }

        // 9. Check if all balls were lost
        if self.active_ball_count() == 0 {
            if self.lives > 1 {
                self.lives -= 1;
                self.combo = 0;
                self.clear_powerups();
                self.lasers = [Laser::EMPTY; MAX_LASERS];
                self.spawn_glued_ball();
                fx = fx.upgrade(EventFx::LoseLife);
            } else {
                self.lives = 0;
                self.state = State::GameOver;
                return EventFx::GameOver;
            }
        }

        fx
    }

    fn step_lasers(&mut self) -> EventFx {
        let mut fx = EventFx::None;
        for i in 0..MAX_LASERS {
            if !self.lasers[i].active {
                continue;
            }
            let ny = self.lasers[i].y as i32 - 9;
            if ny <= PLAY_T + 2 {
                self.lasers[i].active = false;
                continue;
            }
            self.lasers[i].y = ny as i16;
            let lx = self.lasers[i].x as i32;

            // Quick grid lookup
            if ny <= GRID_Y0 + ROWS as i32 * (BRICK_H + BRICK_GAP) && ny + LASER_H >= GRID_Y0 {
                let mut hit = false;
                for r in (0..ROWS).rev() {
                    let ry = GRID_Y0 + r as i32 * (BRICK_H + BRICK_GAP);
                    if ny > ry + BRICK_H || ny + LASER_H < ry {
                        continue;
                    }
                    for c in 0..COLS {
                        let rx = GRID_X0 + c as i32 * (BRICK_W + BRICK_GAP);
                        if lx + LASER_W / 2 >= rx && lx - LASER_W / 2 <= rx + BRICK_W {
                            if self.bricks[r * COLS + c].alive() {
                                fx = fx.upgrade(self.damage_brick(r, c, 1, true));
                                self.lasers[i].active = false;
                                hit = true;
                                break;
                            }
                        }
                    }
                    if hit {
                        break;
                    }
                }
            }
        }
        fx
    }

    fn step_balls(&mut self) -> EventFx {
        let mut fx = EventFx::None;
        let spd = self.target_speed_q();
        let min_x_q = (PLAY_L + BALL_R) * Q;
        let max_x_q = (PLAY_R - BALL_R) * Q;
        let min_y_q = (PLAY_T + BALL_R) * Q;
        let shield_y_q = (SHIELD_Y - BALL_R) * Q;
        let kill_y_q = (PLAY_B + BALL_R + 6) * Q;

        for bi in 0..MAX_BALLS {
            if !self.balls[bi].active {
                continue;
            }
            if self.balls[bi].glued {
                self.balls[bi].x_q = (self.paddle_x + self.balls[bi].glue_off as i32) * Q;
                self.balls[bi].y_q = (PADDLE_Y - BALL_R - 1) * Q;
                continue;
            }

            for _ in 0..SUBSTEPS {
                if !self.balls[bi].active {
                    break;
                }
                let mut b = self.balls[bi];
                b.x_q += b.vx_q / SUBSTEPS;
                b.y_q += b.vy_q / SUBSTEPS;

                // Left / Right walls
                if b.x_q < min_x_q {
                    b.x_q = min_x_q;
                    b.vx_q = b.vx_q.abs();
                    b.stuck_bounces = b.stuck_bounces.saturating_add(1);
                    fx = fx.upgrade(EventFx::Wall);
                } else if b.x_q > max_x_q {
                    b.x_q = max_x_q;
                    b.vx_q = -b.vx_q.abs();
                    b.stuck_bounces = b.stuck_bounces.saturating_add(1);
                    fx = fx.upgrade(EventFx::Wall);
                }

                // Ceiling wall
                if b.y_q < min_y_q {
                    b.y_q = min_y_q;
                    b.vy_q = b.vy_q.abs();
                    b.stuck_bounces = b.stuck_bounces.saturating_add(1);
                    fx = fx.upgrade(EventFx::Wall);
                }

                // Paddle collision (only when descending)
                let bx = b.px();
                let by = b.py();
                let half_p = self.paddle_w / 2;
                let pl = self.paddle_x - half_p;
                let pr = self.paddle_x + half_p;
                if b.vy_q > 0
                    && by + BALL_R >= PADDLE_Y
                    && by - BALL_R <= PADDLE_Y + PADDLE_H
                    && bx + BALL_R >= pl
                    && bx - BALL_R <= pr
                {
                    b.y_q = (PADDLE_Y - BALL_R) * Q;
                    let off = (bx - self.paddle_x).clamp(-half_p, half_p);
                    // -65° to +65° around straight up (49152)
                    let ang = (49152i32 + (off * 11833) / half_p.max(1)) as u16;
                    b.vx_q = (cos(ang) * spd) / Q14;
                    b.vy_q = (sin(ang) * spd) / Q14;
                    if b.vy_q > -spd / 3 {
                        b.vy_q = -spd / 3;
                    }
                    normalize_velocity(&mut b.vx_q, &mut b.vy_q, spd);
                    b.stuck_bounces = 0;
                    self.combo = 0;
                    fx = fx.upgrade(EventFx::Paddle);
                }

                // Safety Shield collision
                if self.shield_ticks > 0 && self.shield_hits > 0 && b.vy_q > 0 && b.y_q >= shield_y_q {
                    b.y_q = shield_y_q;
                    b.vy_q = -b.vy_q.abs();
                    enforce_min_vy(&mut b.vx_q, &mut b.vy_q, spd);
                    b.stuck_bounces = 0;
                    self.shield_hits -= 1;
                    if self.shield_hits == 0 {
                        self.shield_ticks = 0;
                    }
                    let sx = b.px();
                    self.balls[bi] = b;
                    self.spawn_sparks(sx, SHIELD_Y, cmf_wp2_ui::color::rgb565(64, 240, 255), 8);
                    fx = fx.upgrade(EventFx::Shield);
                    continue;
                }

                // Bottom pit miss
                if b.y_q > kill_y_q {
                    self.balls[bi] = Ball::EMPTY;
                    break;
                }

                // Brick collisions
                self.balls[bi] = b;
                let brick_fx = self.collide_ball_with_bricks(bi, spd);
                fx = fx.upgrade(brick_fx);

                // Anti-stuck horizontal / bumper loop guard
                if self.balls[bi].active {
                    let b = &mut self.balls[bi];
                    if b.stuck_bounces >= 10 {
                        b.stuck_bounces = 0;
                        // Nudge downward slightly so ball never traps indefinitely
                        b.vy_q += spd / 5;
                        normalize_velocity(&mut b.vx_q, &mut b.vy_q, spd);
                    }
                    enforce_min_vy(&mut b.vx_q, &mut b.vy_q, spd);
                }
            }
        }
        fx
    }

    fn collide_ball_with_bricks(&mut self, bi: usize, spd: i32) -> EventFx {
        let b = self.balls[bi];
        let bx = b.px();
        let by = b.py();
        let grid_bottom = GRID_Y0 + ROWS as i32 * (BRICK_H + BRICK_GAP);
        if by - BALL_R > grid_bottom || by + BALL_R < GRID_Y0 {
            return EventFx::None;
        }

        let r0 = ((by - BALL_R - GRID_Y0) / (BRICK_H + BRICK_GAP)).clamp(0, ROWS as i32 - 1) as usize;
        let r1 = ((by + BALL_R - GRID_Y0) / (BRICK_H + BRICK_GAP)).clamp(0, ROWS as i32 - 1) as usize;
        let c0 = ((bx - BALL_R - GRID_X0) / (BRICK_W + BRICK_GAP)).clamp(0, COLS as i32 - 1) as usize;
        let c1 = ((bx + BALL_R - GRID_X0) / (BRICK_W + BRICK_GAP)).clamp(0, COLS as i32 - 1) as usize;

        let fireball = self.fire_ticks > 0;
        let mut fx = EventFx::None;

        for r in r0..=r1 {
            for c in c0..=c1 {
                let br = self.bricks[r * COLS + c];
                if !br.alive() {
                    continue;
                }
                let (rx, ry, rw, rh) = brick_rect(r, c);
                let cx = bx.clamp(rx, rx + rw);
                let cy = by.clamp(ry, ry + rh);
                let dx = bx - cx;
                let dy = by - cy;
                if dx * dx + dy * dy > BALL_R * BALL_R {
                    continue;
                }

                // Fireball smashes straight through breakable bricks without bouncing!
                if fireball && br.kind.is_breakable() {
                    let dfx = self.damage_brick(r, c, 3, true);
                    fx = fx.upgrade(dfx);
                    self.balls[bi].stuck_bounces = 0;
                    continue;
                }

                // Standard reflection: choose axis with smaller overlap / greater center delta
                let overlap_l = (bx + BALL_R) - rx;
                let overlap_r = (rx + rw) - (bx - BALL_R);
                let overlap_t = (by + BALL_R) - ry;
                let overlap_b = (ry + rh) - (by - BALL_R);
                let min_x = overlap_l.min(overlap_r);
                let min_y = overlap_t.min(overlap_b);

                if min_x < min_y {
                    if overlap_l < overlap_r {
                        self.balls[bi].x_q = (rx - BALL_R) * Q;
                        self.balls[bi].vx_q = -self.balls[bi].vx_q.abs();
                    } else {
                        self.balls[bi].x_q = (rx + rw + BALL_R) * Q;
                        self.balls[bi].vx_q = self.balls[bi].vx_q.abs();
                    }
                } else if overlap_t < overlap_b {
                    self.balls[bi].y_q = (ry - BALL_R) * Q;
                    self.balls[bi].vy_q = -self.balls[bi].vy_q.abs();
                } else {
                    self.balls[bi].y_q = (ry + rh + BALL_R) * Q;
                    self.balls[bi].vy_q = self.balls[bi].vy_q.abs();
                }

                enforce_min_vy(&mut self.balls[bi].vx_q, &mut self.balls[bi].vy_q, spd);

                if br.kind.is_breakable() {
                    self.balls[bi].stuck_bounces = 0;
                } else {
                    self.balls[bi].stuck_bounces = self.balls[bi].stuck_bounces.saturating_add(1);
                }

                let dfx = self.damage_brick(r, c, 1, true);
                fx = fx.upgrade(dfx);
                return fx; // One reflection per substep
            }
        }
        fx
    }

    fn step_capsules(&mut self) -> EventFx {
        let mut fx = EventFx::None;
        let half_p = self.paddle_w / 2;
        let pl = self.paddle_x - half_p;
        let pr = self.paddle_x + half_p;

        for i in 0..MAX_CAPSULES {
            if !self.capsules[i].active {
                continue;
            }
            self.capsules[i].y_q += 560; // ~2.18 px/tick
            let cx = self.capsules[i].x as i32;
            let cy = self.capsules[i].py();
            if cy - CAP_H / 2 > PLAY_B {
                self.capsules[i].active = false;
                continue;
            }
            // Paddle catch check
            if cy + CAP_H / 2 >= PADDLE_Y - 2
                && cy - CAP_H / 2 <= PADDLE_Y + PADDLE_H + 2
                && cx + CAP_W / 2 >= pl
                && cx - CAP_W / 2 <= pr
            {
                let kind = self.capsules[i].kind;
                self.capsules[i].active = false;
                self.apply_powerup(kind);
                self.spawn_sparks(cx, PADDLE_Y, cmf_wp2_ui::color::rgb565(255, 255, 160), 6);
                fx = fx.upgrade(EventFx::Powerup);
            }
        }
        fx
    }

    fn step_particles(&mut self) {
        for p in &mut self.particles {
            if !p.active {
                continue;
            }
            if p.life <= 1 {
                p.active = false;
                continue;
            }
            p.life -= 1;
            p.x_q += p.vx_q as i32;
            p.y_q += p.vy_q as i32;
            p.vy_q = p.vy_q.saturating_add(24); // subtle arcade spark gravity
            let px = p.px();
            let py = p.py();
            if px <= PLAY_L + 2 || px >= PLAY_R - 4 || py <= PLAY_T + 2 || py >= PLAY_B - 4 {
                p.active = false;
            }
        }
    }

    /// Returns the most urgent active power-up (for the HUD timer bar): `(kind, remaining, max)`.
    pub fn primary_power_timer(&self) -> Option<(PowerKind, u16, u16)> {
        if self.fire_ticks > 0 {
            Some((PowerKind::Fire, self.fire_ticks, FIRE_DURATION))
        } else if self.laser_ticks > 0 {
            Some((PowerKind::Laser, self.laser_ticks, LASER_DURATION))
        } else if self.wide_ticks > 0 {
            Some((PowerKind::Wide, self.wide_ticks, WIDE_DURATION))
        } else if self.shield_ticks > 0 {
            Some((PowerKind::Shield, self.shield_ticks, SHIELD_DURATION))
        } else if self.slow_ticks > 0 {
            Some((PowerKind::Slow, self.slow_ticks, SLOW_DURATION))
        } else {
            None
        }
    }
}

/// Rescales `(vx_q, vy_q)` to magnitude `spd` using integer `isqrt`.
pub fn normalize_velocity(vx_q: &mut i32, vy_q: &mut i32, spd: i32) {
    let mag_sq = (*vx_q as i64) * (*vx_q as i64) + (*vy_q as i64) * (*vy_q as i64);
    let mag = isqrt(mag_sq.min(i32::MAX as i64) as i32).max(1);
    *vx_q = (*vx_q * spd) / mag;
    *vy_q = (*vy_q * spd) / mag;
}

/// Anti-stuck guard: guarantees vertical speed is at least `spd / 4` (~14.5°) so balls never bounce flat forever.
pub fn enforce_min_vy(vx_q: &mut i32, vy_q: &mut i32, spd: i32) {
    let min_vy = spd / 4;
    if vy_q.abs() < min_vy {
        *vy_q = if *vy_q < 0 { -min_vy } else { min_vy };
        let rem = (spd * spd - *vy_q * *vy_q).max(1);
        let nx = isqrt(rem);
        *vx_q = if *vx_q < 0 { -nx } else { nx };
    }
}

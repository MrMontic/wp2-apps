//! Core Super Hexagon simulation: 3 modes, iconic guaranteed-passable wall patterns,
//! elastic world rotation, BPM beat pulse, forgiving side-sliding collision, and NAND save codec.
//!
//! Pure `no_std`, zero allocations, zero floating-point (fixed-point integer math only).

use cmf_wp2_ui::C;

/// Frame step in milliseconds (~45.5 FPS).
pub const FRAME_MS: u32 = 22;

/// Fixed-point radial scale (subpixels per pixel).
pub const Q8: i32 = 256;

/// Maximum simultaneous active wall trapezoids.
pub const MAX_WALLS: usize = 32;

/// Center polygon base radius (px).
pub const CENTER_R: i32 = 40;
/// Center polygon outline thickness (px).
pub const CENTER_RIM: i32 = 5;
/// Player orbit radius (px).
pub const PLAYER_R: i32 = 54;
/// Player radial hitbox (slightly inset for fair, responsive feel).
pub const PLAYER_HIT_IN: i32 = 51;
pub const PLAYER_HIT_OUT: i32 = 57;

/// Wall spawn radius (just outside the visible 231px circular rim).
pub const SPAWN_R_Q8: i32 = 242 * Q8;

/// Player touch angular speed (turns per 22ms tick; 65536 = 360 deg).
/// 1640 turns/tick ~ 409 deg/sec (1 sector = 60 deg in ~146ms).
const TOUCH_SPEED: i32 = 1640;

/// Angular rotation per crown detent (~16 deg = ~1/3.75 of a hexagon sector).
const CROWN_STEP: i32 = 2912;

/// Maximum angular sub-step for sweep/slide collision (prevents wall tunneling on fast crown spins).
const MAX_SUBSTEP: i32 = 1400;

/// Angular pad kept when sliding along a wall's side edge (~2.8 deg).
const SLIDE_PAD: u16 = 520;

/// Forgiving half-width of player's collision arc (~2.2 deg).
const PLAYER_HIT_HALF_ANG: u16 = 400;

/// Duration of crash flash/shake animation before showing Game Over overlay (ticks).
const DYING_TICKS: u8 = 11;

/// Save file magic headers (`HEX2` = 5 modes; `HEX1` = legacy 3-mode save migrated on load).
const SAVE_MAGIC_V1: [u8; 4] = *b"HEX1";
const SAVE_LEN_V1: usize = 20;
const SAVE_MAGIC: [u8; 4] = *b"HEX2";
/// Encoded save payload size (4 magic + 5*4 bests + 4 checksum).
pub const SAVE_LEN: usize = 28;

/// Difficulty modes: accessible Easy & Medium + classic Super Hexagon Hard / Harder / Hardest.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum Mode {
    Easy = 0,
    Medium = 1,
    Hexagon = 2,
    Hexagoner = 3,
    Hexagonest = 4,
}

impl Mode {
    pub const COUNT: usize = 5;

    #[inline]
    pub const fn idx(self) -> usize {
        self as usize
    }

    #[inline]
    pub const fn from_idx(i: usize) -> Self {
        match i % Self::COUNT {
            0 => Mode::Easy,
            1 => Mode::Medium,
            2 => Mode::Hexagon,
            3 => Mode::Hexagoner,
            _ => Mode::Hexagonest,
        }
    }

    #[inline]
    pub const fn next(self) -> Self {
        Self::from_idx(self.idx() + 1)
    }

    #[inline]
    pub const fn prev(self) -> Self {
        Self::from_idx(self.idx() + Self::COUNT - 1)
    }

    pub const fn name(self) -> &'static str {
        match self {
            Mode::Easy => "EASY",
            Mode::Medium => "MEDIUM",
            Mode::Hexagon => "HEXAGON",
            Mode::Hexagoner => "HEXAGONER",
            Mode::Hexagonest => "HEXAGONEST",
        }
    }

    pub const fn subtitle(self) -> &'static str {
        match self {
            Mode::Easy => "CHILL",
            Mode::Medium => "NORMAL",
            Mode::Hexagon => "HARD",
            Mode::Hexagoner => "HARDER",
            Mode::Hexagonest => "HARDEST",
        }
    }

    /// Initial wall speed in Q8 subpixels per 22ms tick.
    const fn base_wall_speed_q8(self) -> i32 {
        match self {
            Mode::Easy => 390,        // ~69 px/s
            Mode::Medium => 520,      // ~92 px/s
            Mode::Hexagon => 660,     // ~117 px/s
            Mode::Hexagoner => 845,   // ~150 px/s
            Mode::Hexagonest => 1020, // ~181 px/s
        }
    }

    /// Maximum wall speed in Q8 subpixels per 22ms tick.
    const fn max_wall_speed_q8(self) -> i32 {
        match self {
            Mode::Easy => 560,        // ~99 px/s
            Mode::Medium => 760,      // ~135 px/s
            Mode::Hexagon => 960,     // ~170 px/s
            Mode::Hexagoner => 1160,  // ~206 px/s
            Mode::Hexagonest => 1360, // ~241 px/s
        }
    }

    /// Base world angular velocity (turns per 22ms tick).
    const fn base_rot_vel(self) -> i32 {
        match self {
            Mode::Easy => 95,        // ~24 deg/s
            Mode::Medium => 150,     // ~37 deg/s
            Mode::Hexagon => 210,    // ~52 deg/s
            Mode::Hexagoner => 320,  // ~80 deg/s
            Mode::Hexagonest => 430, // ~107 deg/s
        }
    }

    /// Maximum world angular velocity (turns per 22ms tick).
    const fn max_rot_vel(self) -> i32 {
        match self {
            Mode::Easy => 185,       // ~46 deg/s
            Mode::Medium => 295,     // ~74 deg/s
            Mode::Hexagon => 410,    // ~102 deg/s
            Mode::Hexagoner => 540,  // ~135 deg/s
            Mode::Hexagonest => 660, // ~165 deg/s
        }
    }

    /// Beat period in milliseconds (115 / 128 / 140 / 150 / 160 BPM).
    const fn beat_ms(self) -> u32 {
        match self {
            Mode::Easy => 522,
            Mode::Medium => 468,
            Mode::Hexagon => 428,
            Mode::Hexagoner => 400,
            Mode::Hexagonest => 375,
        }
    }

    /// Extra reaction margin in ticks for spacing between consecutive waves.
    const fn reaction_ticks(self) -> i32 {
        match self {
            Mode::Easy => 19,
            Mode::Medium => 15,
            Mode::Hexagon => 11,
            Mode::Hexagoner => 8,
            Mode::Hexagonest => 6,
        }
    }
}

/// Super Hexagon survival rank tiers.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
#[repr(u8)]
pub enum Rank {
    Point = 0,
    Line = 1,
    Triangle = 2,
    Square = 3,
    Pentagon = 4,
    Hexagon = 5,
}

impl Rank {
    /// Rank from survival time in centiseconds (10s / 20s / 30s / 45s / 60s).
    #[inline]
    pub const fn from_cs(cs: u32) -> Self {
        if cs >= 6000 {
            Rank::Hexagon
        } else if cs >= 4500 {
            Rank::Pentagon
        } else if cs >= 3000 {
            Rank::Square
        } else if cs >= 2000 {
            Rank::Triangle
        } else if cs >= 1000 {
            Rank::Line
        } else {
            Rank::Point
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Rank::Point => "POINT",
            Rank::Line => "LINE",
            Rank::Triangle => "TRIANGLE",
            Rank::Square => "SQUARE",
            Rank::Pentagon => "PENTAGON",
            Rank::Hexagon => "HEXAGON",
        }
    }
}

/// High-level screen/game state.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum State {
    Title,
    Playing,
    Dying,
    Over,
}

/// Event emitted by `Game::step()` to drive haptics and persistence.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Step {
    Nothing,
    RankUp,
    Morph,
    Crashed,
    OverReady,
}

/// A single closing trapezoidal wall segment occupying one sector.
#[derive(Clone, Copy, Debug)]
pub struct Wall {
    pub active: bool,
    pub sector: u8,
    pub r_in_q8: i32,
    pub thick_q8: i32,
}

impl Wall {
    pub const EMPTY: Self = Self {
        active: false,
        sector: 0,
        r_in_q8: 0,
        thick_q8: 0,
    };
}

/// Iconic multi-wave pattern kinds.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Pattern {
    CWall,
    DoubleAlt,
    Spiral,
    DoubleC,
    Barrage,
    Pinch,
}

/// Complete game simulation state (~540 bytes; keep in `static mut` on watch!).
pub struct Game {
    pub state: State,
    pub mode: Mode,
    /// Number of polygon sides (6 in Hexagon/Hexagoner; morphs 6/5/4 in Hexagonest).
    pub sides: u8,
    /// Pending morph target (`0` = none; waits until arena clears near center).
    pending_sides: u8,
    /// Ticks until next morph attempt in Hexagonest.
    morph_cooldown: u16,

    /// Best survival time per mode in centiseconds (1/100s).
    pub best_cs: [u32; Mode::COUNT],
    /// Current run elapsed time in milliseconds.
    pub elapsed_ms: u32,
    /// Whether this run beat the previous mode best.
    pub new_best: bool,
    /// Highest rank reached during this run.
    pub rank: Rank,

    /// World rotation angle (`u16` turns) and angular velocity (turns/tick).
    pub world_angle: u16,
    rot_vel: i32,
    rot_target: i32,
    rot_flip_ticks: u16,

    /// Player angle in WORLD coordinates (`u16` turns).
    pub player_angle: u16,
    /// Active touch turn direction: `-1` CCW, `0` none, `+1` CW.
    pub touch_dir: i8,

    /// Beat pulse (0..=256) and beat accumulator in ms.
    pub pulse: i32,
    beat_accum_ms: u32,

    /// Palette cycling state.
    pub palette_idx: u8,
    pub palette_next: u8,
    /// Cross-fade progress `0..=256`.
    pub palette_blend: u16,
    palette_timer: u16,

    /// Crash flash/shake animation state.
    pub dying_ticks: u8,
    pub shake_x: i8,
    pub shake_y: i8,

    /// Active walls pool.
    pub walls: [Wall; MAX_WALLS],

    /// Pattern sequencer state.
    pat: Pattern,
    pat_waves_left: u8,
    pat_gap: u8,
    pat_dir: i8,
    pat_alt: u8,
    /// Remaining radial distance (Q8) before spawning next wave.
    spawn_dist_q8: i32,

    /// PRNG state (xorshift32).
    rng: u32,
    /// Monotonic tick counter.
    pub tick: u32,
}

impl Game {
    pub fn new(seed: u32, best_cs: [u32; Mode::COUNT]) -> Self {
        let mut g = Self {
            state: State::Title,
            mode: Mode::Easy,
            sides: 6,
            pending_sides: 0,
            morph_cooldown: 450,
            best_cs,
            elapsed_ms: 0,
            new_best: false,
            rank: Rank::Point,
            world_angle: 5461, // 30 deg pleasant starting orientation
            rot_vel: 110,
            rot_target: 110,
            rot_flip_ticks: 180,
            player_angle: 49152, // top
            touch_dir: 0,
            pulse: 0,
            beat_accum_ms: 0,
            palette_idx: 0,
            palette_next: 1,
            palette_blend: 0,
            palette_timer: 180,
            dying_ticks: 0,
            shake_x: 0,
            shake_y: 0,
            walls: [Wall::EMPTY; MAX_WALLS],
            pat: Pattern::CWall,
            pat_waves_left: 0,
            pat_gap: 0,
            pat_dir: 1,
            pat_alt: 0,
            spawn_dist_q8: 0,
            rng: if seed == 0 { 0x9E37_79B9 } else { seed },
            tick: 0,
        };
        g.seed_title_decor();
        g
    }

    /// Current survival time in centiseconds (`1248` -> `12.48s`).
    #[inline]
    pub const fn time_cs(&self) -> u32 {
        self.elapsed_ms / 10
    }

    /// Best time for the active mode in centiseconds.
    #[inline]
    pub const fn mode_best_cs(&self) -> u32 {
        self.best_cs[self.mode.idx()]
    }

    /// Current center polygon radius including beat pulse.
    #[inline]
    pub const fn center_r(&self) -> i32 {
        CENTER_R + ((self.pulse * 5) >> 8)
    }

    /// Xorshift32 PRNG.
    #[inline]
    fn rand(&mut self) -> u32 {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        x
    }

    #[inline]
    fn rand_range(&mut self, max: u32) -> u32 {
        if max <= 1 {
            0
        } else {
            self.rand() % max
        }
    }

    /// Select a difficulty mode on the Title Screen.
    pub fn set_mode(&mut self, mode: Mode) {
        self.mode = mode;
        self.sides = 6;
        self.palette_idx = 0;
        self.palette_next = 1;
        self.palette_blend = 0;
        self.palette_timer = 180;
    }

    pub fn next_mode(&mut self) {
        self.set_mode(self.mode.next());
    }

    pub fn prev_mode(&mut self) {
        self.set_mode(self.mode.prev());
    }

    /// Return to Title Screen from Game Over.
    pub fn to_title(&mut self) {
        self.state = State::Title;
        self.sides = 6;
        self.pending_sides = 0;
        self.touch_dir = 0;
        self.shake_x = 0;
        self.shake_y = 0;
        self.dying_ticks = 0;
        self.palette_idx = 0;
        self.palette_next = 1;
        self.palette_blend = 0;
        self.palette_timer = 180;
        self.seed_title_decor();
    }

    /// Place a couple of gentle decorative ambient walls on the Title Screen.
    fn seed_title_decor(&mut self) {
        self.walls = [Wall::EMPTY; MAX_WALLS];
        let mask1 = 0b010101u8;
        let mask2 = 0b101010u8;
        self.spawn_wave_at(mask1, 130 * Q8, 16 * Q8);
        self.spawn_wave_at(mask2, 200 * Q8, 16 * Q8);
    }

    /// Start a fresh run in the currently selected mode.
    pub fn start(&mut self) {
        self.state = State::Playing;
        self.sides = 6;
        self.pending_sides = 0;
        self.morph_cooldown = 420; // ~9.2s before first morph in Hexagonest
        self.elapsed_ms = 0;
        self.new_best = false;
        self.rank = Rank::Point;
        self.touch_dir = 0;
        self.dying_ticks = 0;
        self.shake_x = 0;
        self.shake_y = 0;
        self.walls = [Wall::EMPTY; MAX_WALLS];

        // Clean initial world spin & beat
        let dir = if self.rand() & 1 == 0 { 1 } else { -1 };
        self.rot_target = self.mode.base_rot_vel() * dir;
        self.rot_vel = self.rot_target / 2;
        self.rot_flip_ticks = self.next_flip_ticks();

        self.player_angle = self.sector_center(0);
        self.pulse = 220;
        self.beat_accum_ms = 0;

        self.palette_idx = 0;
        self.palette_next = 1;
        self.palette_blend = 0;
        self.palette_timer = 170;

        // First wave spawns slightly inside the rim so action begins swiftly (~0.4s)
        self.pat_waves_left = 0;
        self.pat_gap = 0;
        self.pat_dir = 1;
        self.pat_alt = 0;
        self.spawn_dist_q8 = 18 * Q8;
    }

    /// Set touch steering based on touch x coordinate (`x < C` -> CCW, `x >= C` -> CW).
    #[inline]
    pub fn touch_down(&mut self, x: i32) {
        self.touch_dir = if x < C { -1 } else { 1 };
    }

    /// Release touch steering.
    #[inline]
    pub fn touch_up(&mut self) {
        self.touch_dir = 0;
    }

    /// Apply accumulated crown detents immediately during gameplay.
    pub fn apply_crown(&mut self, detents: i32) {
        if self.state != State::Playing || detents == 0 {
            return;
        }
        // Clamp extreme burst to prevent >180 deg single-frame jumps
        let delta = (detents.clamp(-12, 12)) * CROWN_STEP;
        self.move_player_angular(delta);
    }

    /// Current wall approach speed in Q8 subpixels per tick, ramping smoothly over 60s.
    pub fn wall_speed_q8(&self) -> i32 {
        let base = self.mode.base_wall_speed_q8();
        let max = self.mode.max_wall_speed_q8();
        let prog = (self.elapsed_ms / 60).min(1000) as i32; // 0..1000 over 60s
        base + ((max - base) * prog) / 1000
    }

    /// Current target rotation speed magnitude (turns/tick), ramping smoothly over 60s.
    fn target_rot_mag(&self) -> i32 {
        let base = self.mode.base_rot_vel();
        let max = self.mode.max_rot_vel();
        let prog = (self.elapsed_ms / 60).min(1000) as i32;
        base + ((max - base) * prog) / 1000
    }

    fn next_flip_ticks(&mut self) -> u16 {
        let (min_t, span) = match self.mode {
            Mode::Easy => (190, 140),      // 4.2s .. 7.3s
            Mode::Medium => (150, 125),    // 3.3s .. 6.0s
            Mode::Hexagon => (115, 110),   // 2.5s .. 5.0s
            Mode::Hexagoner => (82, 90),   // 1.8s .. 3.8s
            Mode::Hexagonest => (60, 75),  // 1.3s .. 3.0s
        };
        (min_t + self.rand_range(span)) as u16
    }

    /// Sector index (`0..sides`) containing world-space angle `a`.
    #[inline]
    pub fn sector_of(&self, a: u16) -> u8 {
        (((a as u32) * (self.sides as u32)) >> 16) as u8
    }

    /// Start angle of sector `s` (`0..sides`).
    #[inline]
    pub fn sector_start(&self, s: u8) -> u16 {
        let n = self.sides as u32;
        (((s as u32 % n) * 65536 + n / 2) / n) as u16
    }

    /// End angle of sector `s` (`0..sides`).
    #[inline]
    pub fn sector_end(&self, s: u8) -> u16 {
        self.sector_start((s + 1) % self.sides)
    }

    /// Center angle of sector `s`.
    #[inline]
    pub fn sector_center(&self, s: u8) -> u16 {
        let n = self.sides as u32;
        ((((s as u32 % n) * 2 + 1) * 65536) / (2 * n)) as u16
    }

    /// Whether any active wall currently occupies `sector` at the player's radial collision band.
    fn sector_blocked_at_player(&self, sector: u8) -> bool {
        let hit_in = PLAYER_HIT_IN * Q8;
        let hit_out = PLAYER_HIT_OUT * Q8;
        for w in &self.walls {
            if w.active && w.sector == sector {
                let w_out = w.r_in_q8 + w.thick_q8;
                if w.r_in_q8 <= hit_out && w_out >= hit_in {
                    return true;
                }
            }
        }
        false
    }

    /// Moves the player by `delta` turns with authentic Super Hexagon **side-sliding**:
    /// entering a wall from its side slides/clamps the player neatly against the wall edge
    /// instead of killing them!
    pub fn move_player_angular(&mut self, mut delta: i32) {
        while delta != 0 {
            let step = delta.clamp(-MAX_SUBSTEP, MAX_SUBSTEP);
            delta -= step;

            let old_ang = self.player_angle;
            let new_ang = old_ang.wrapping_add(step as u16);

            let old_sec = self.sector_of(old_ang);
            let new_sec = self.sector_of(new_ang);

            // Check if the leading edge of the player's small arc hits an adjacent blocked sector
            let probe_ang = if step > 0 {
                new_ang.wrapping_add(PLAYER_HIT_HALF_ANG)
            } else {
                new_ang.wrapping_sub(PLAYER_HIT_HALF_ANG)
            };
            let probe_sec = self.sector_of(probe_ang);

            if probe_sec != old_sec && self.sector_blocked_at_player(probe_sec) {
                // Slide along the boundary of the blocked sector!
                if step > 0 {
                    let edge = self.sector_start(probe_sec);
                    self.player_angle = edge.wrapping_sub(SLIDE_PAD);
                } else {
                    let edge = self.sector_end(probe_sec);
                    self.player_angle = edge.wrapping_add(SLIDE_PAD);
                }
                return;
            }

            if new_sec != old_sec && self.sector_blocked_at_player(new_sec) {
                if step > 0 {
                    let edge = self.sector_start(new_sec);
                    self.player_angle = edge.wrapping_sub(SLIDE_PAD);
                } else {
                    let edge = self.sector_end(new_sec);
                    self.player_angle = edge.wrapping_add(SLIDE_PAD);
                }
                return;
            }

            self.player_angle = new_ang;
        }
    }

    /// Check if any wall's front face hits the player (called AFTER advancing walls inward).
    fn check_front_crash(&self) -> bool {
        let hit_in = PLAYER_HIT_IN * Q8;
        let hit_out = PLAYER_HIT_OUT * Q8;
        let sec_c = self.sector_of(self.player_angle);
        // Check center point of player cursor (forgiving & authentic!)
        for w in &self.walls {
            if w.active && w.sector == sec_c {
                let w_out = w.r_in_q8 + w.thick_q8;
                if w.r_in_q8 <= hit_out && w_out >= hit_in {
                    return true;
                }
            }
        }
        false
    }

    /// Advance the Title Screen ambient animation by one tick.
    pub fn step_title(&mut self) {
        self.tick = self.tick.wrapping_add(1);
        self.world_angle = self.world_angle.wrapping_add(95);
        self.update_beat();
        self.update_palette();
    }

    /// Advance gameplay or crash animation by one `FRAME_MS` tick.
    pub fn step(&mut self) -> Step {
        self.tick = self.tick.wrapping_add(1);

        match self.state {
            State::Title => {
                self.step_title();
                return Step::Nothing;
            }
            State::Over => return Step::Nothing,
            State::Dying => {
                if self.dying_ticks > 0 {
                    self.dying_ticks -= 1;
                    // Dampening impact shake
                    let amp = (self.dying_ticks as i32).min(6);
                    let sx = if self.dying_ticks & 1 == 0 { amp } else { -amp };
                    let sy = if self.dying_ticks & 2 == 0 { amp / 2 } else { -(amp / 2) };
                    self.shake_x = sx as i8;
                    self.shake_y = sy as i8;
                    self.world_angle = self.world_angle.wrapping_add((self.rot_vel / 4) as u16);
                    if self.dying_ticks == 0 {
                        self.shake_x = 0;
                        self.shake_y = 0;
                        self.state = State::Over;
                        return Step::OverReady;
                    }
                }
                return Step::Nothing;
            }
            State::Playing => {}
        }

        // 1. Advance survival timer & rank
        self.elapsed_ms = self.elapsed_ms.saturating_add(FRAME_MS);
        let cs = self.time_cs();
        let mut event = Step::Nothing;

        if cs > self.best_cs[self.mode.idx()] {
            self.best_cs[self.mode.idx()] = cs;
            self.new_best = true;
        }

        let new_rank = Rank::from_cs(cs);
        if new_rank > self.rank {
            self.rank = new_rank;
            self.pulse = 256;
            event = Step::RankUp;
        }

        // 2. Beat pulse & palette cycling
        self.update_beat();
        self.update_palette();

        // 3. Elastic world rotation with periodic reversals
        if self.rot_flip_ticks > 0 {
            self.rot_flip_ticks -= 1;
        } else {
            let sign = if self.rot_target >= 0 { -1 } else { 1 };
            self.rot_target = self.target_rot_mag() * sign;
            self.rot_flip_ticks = self.next_flip_ticks();
        }
        // Keep target magnitude up to date as speed ramps
        let mag = self.target_rot_mag();
        self.rot_target = if self.rot_target >= 0 { mag } else { -mag };
        let accel = match self.mode {
            Mode::Easy => 10,
            Mode::Medium => 14,
            Mode::Hexagon => 18,
            Mode::Hexagoner => 26,
            Mode::Hexagonest => 36,
        };
        if self.rot_vel < self.rot_target {
            self.rot_vel = (self.rot_vel + accel).min(self.rot_target);
        } else if self.rot_vel > self.rot_target {
            self.rot_vel = (self.rot_vel - accel).max(self.rot_target);
        }
        self.world_angle = self.world_angle.wrapping_add(self.rot_vel as u16);

        // 4. Touch steering (crown events are applied immediately in main loop)
        if self.touch_dir != 0 {
            self.move_player_angular((self.touch_dir as i32) * TOUCH_SPEED);
        }

        // 5. Advance walls inward & absorb into center polygon
        let speed = self.wall_speed_q8();
        let absorb_r = (self.center_r() - 2) * Q8;
        let mut any_wall_active = false;
        for w in &mut self.walls {
            if w.active {
                w.r_in_q8 -= speed;
                if w.r_in_q8 + w.thick_q8 <= absorb_r {
                    w.active = false;
                } else {
                    any_wall_active = true;
                }
            }
        }

        // 6. Front-face collision check
        if self.check_front_crash() {
            self.state = State::Dying;
            self.dying_ticks = DYING_TICKS;
            self.touch_dir = 0;
            self.shake_x = 6;
            self.shake_y = -4;
            return Step::Crashed;
        }

        // 7. Hexagonest polygon morphing (6 -> 5 -> 4 -> 6) during clean gap
        if self.mode == Mode::Hexagonest {
            if self.pending_sides != 0 {
                // Wait until all existing walls are absorbed so morphing is 100% fair!
                if !any_wall_active {
                    self.sides = self.pending_sides;
                    self.pending_sides = 0;
                    self.pat_waves_left = 0;
                    self.pat_gap %= self.sides;
                    self.spawn_dist_q8 = 12 * Q8;
                    self.pulse = 256;
                    self.morph_cooldown = 400 + self.rand_range(120) as u16;
                    if event == Step::Nothing {
                        event = Step::Morph;
                    }
                }
                return event;
            } else if self.morph_cooldown > 0 {
                self.morph_cooldown -= 1;
            } else {
                // Schedule a clean morph: 6 -> 5 -> 4 -> 6 (or 6 <-> 5)
                self.pending_sides = match self.sides {
                    6 => 5,
                    5 => if self.rand() & 1 == 0 { 4 } else { 6 },
                    _ => 6,
                };
                return event;
            }
        }

        // 8. Pattern sequencer & wave spawning
        self.spawn_dist_q8 -= speed;
        if self.spawn_dist_q8 <= 0 {
            self.spawn_next_wave(speed);
        }

        event
    }

    fn update_beat(&mut self) {
        self.beat_accum_ms += FRAME_MS;
        let period = self.mode.beat_ms();
        if self.beat_accum_ms >= period {
            self.beat_accum_ms -= period;
            self.pulse = 256;
        } else {
            self.pulse = (self.pulse - 28).max(0);
        }
    }

    fn update_palette(&mut self) {
        if self.palette_timer > 0 {
            self.palette_timer -= 1;
        } else {
            self.palette_blend = self.palette_blend.saturating_add(6);
            if self.palette_blend >= 256 {
                self.palette_idx = self.palette_next;
                self.palette_next = (self.palette_idx + 1) % 5;
                self.palette_blend = 0;
                self.palette_timer = match self.mode {
                    Mode::Easy => 240,
                    Mode::Medium => 215,
                    Mode::Hexagon => 190,
                    Mode::Hexagoner => 150,
                    Mode::Hexagonest => 115,
                };
            }
        }
    }

    /// Pick a new iconic Super Hexagon pattern when the current sequence completes.
    fn start_new_pattern(&mut self) {
        let roll = self.rand_range(6);
        let next = match roll {
            0 => Pattern::CWall,
            1 => Pattern::DoubleAlt,
            2 => Pattern::Spiral,
            3 => Pattern::DoubleC,
            4 => Pattern::Barrage,
            _ => Pattern::Pinch,
        };
        self.pat = next;
        self.pat_dir = if self.rand() & 1 == 0 { 1 } else { -1 };
        self.pat_alt = 0;
        self.pat_gap = self.rand_range(self.sides as u32) as u8;
        self.pat_waves_left = match next {
            Pattern::CWall => (1 + self.rand_range(2)) as u8,
            Pattern::DoubleAlt => (3 + self.rand_range(2)) as u8,
            Pattern::Spiral => (4 + self.rand_range(3)) as u8,
            Pattern::DoubleC => (2 + self.rand_range(2)) as u8,
            Pattern::Barrage => (4 + self.rand_range(2)) as u8,
            Pattern::Pinch => 3,
        };
    }

    /// Compute guaranteed-passable radial pitch (Q8) for a gap shift of `delta_sectors`.
    fn safe_pitch_q8(&self, speed_q8: i32, thick_q8: i32, delta_sectors: i32) -> i32 {
        let sector_turns = 65536 / (self.sides as i32);
        // Ticks needed by touch steering to cross `delta_sectors` (ceiling division)
        let travel_ticks = (delta_sectors * sector_turns + TOUCH_SPEED - 1) / TOUCH_SPEED;
        let margin_ticks = self.mode.reaction_ticks();
        thick_q8 + speed_q8 * (travel_ticks + margin_ticks)
    }

    /// Spawns the next wave in the active pattern and schedules exact fair spacing.
    pub fn spawn_next_wave(&mut self, speed_q8: i32) {
        if self.pat_waves_left == 0 {
            self.start_new_pattern();
        }
        let n = self.sides;
        let full_mask: u8 = (1u8 << n) - 1;
        let thick_q8: i32;
        let delta_sectors: i32;
        let mask: u8;

        match self.pat {
            Pattern::CWall => {
                // Classic C-wall (Easy leaves 2 adjacent sectors open for a friendlier entry!)
                let gap = self.pat_gap % n;
                if self.mode == Mode::Easy {
                    let gap2 = (gap + 1) % n;
                    mask = full_mask & !((1u8 << gap) | (1u8 << gap2));
                } else {
                    mask = full_mask & !(1u8 << gap);
                }
                thick_q8 = 22 * Q8;
                let shift = (2 + self.rand_range(2)) as u8; // shift 2 or 3 sectors
                self.pat_gap = (gap + shift) % n;
                delta_sectors = (shift as i32).min((n as i32) - (shift as i32)).max(1);
            }
            Pattern::DoubleAlt => {
                // Alternating sectors closed / open, flipped each wave
                let mut m = 0u8;
                let mut s = 0u8;
                while s < n {
                    if (s + self.pat_alt) & 1 == 0 {
                        m |= 1u8 << s;
                    }
                    s += 1;
                }
                // For odd N=5, make sure sector 0 and N-1 don't accidentally both close 3 adjacent
                if n == 5 {
                    let g0 = (self.pat_gap + self.pat_alt) % 5;
                    let g1 = (g0 + 2) % 5;
                    m = full_mask & !((1u8 << g0) | (1u8 << g1));
                }
                mask = m;
                self.pat_alt = self.pat_alt.wrapping_add(1);
                thick_q8 = 18 * Q8;
                delta_sectors = 1;
            }
            Pattern::Spiral => {
                // Iconic spiral staircase: gap steps +/-1 sector each wave!
                let gap = self.pat_gap % n;
                // Easy & Medium (and early Hexagon) leave a wider 2-sector spiral gap
                let wide_gap = self.mode == Mode::Easy
                    || (self.mode == Mode::Medium && self.elapsed_ms < 25_000)
                    || (self.mode == Mode::Hexagon && self.elapsed_ms < 12_000);
                if wide_gap {
                    let gap2 = (gap + 1) % n;
                    mask = full_mask & !((1u8 << gap) | (1u8 << gap2));
                } else {
                    mask = full_mask & !(1u8 << gap);
                }
                self.pat_gap = if self.pat_dir > 0 {
                    (gap + 1) % n
                } else {
                    (gap + n - 1) % n
                };
                thick_q8 = 18 * Q8;
                delta_sectors = 1;
            }
            Pattern::DoubleC => {
                // Opposite double-gap walls
                let g0 = self.pat_gap % n;
                let g1 = (g0 + n / 2) % n;
                mask = full_mask & !((1u8 << g0) | (1u8 << g1));
                self.pat_gap = (g0 + 1 + (self.pat_alt & 1)) % n;
                self.pat_alt = self.pat_alt.wrapping_add(1);
                thick_q8 = 21 * Q8;
                delta_sectors = 1;
            }
            Pattern::Barrage => {
                // Zig-zag corridor weaving +1 / -1
                let gap = self.pat_gap % n;
                mask = full_mask & !(1u8 << gap);
                let step_dir = if (self.pat_alt & 1) == 0 { self.pat_dir } else { -self.pat_dir };
                self.pat_gap = if step_dir > 0 {
                    (gap + 1) % n
                } else {
                    (gap + n - 1) % n
                };
                self.pat_alt = self.pat_alt.wrapping_add(1);
                thick_q8 = 24 * Q8;
                delta_sectors = 1;
            }
            Pattern::Pinch => {
                // Funnel: 2 adjacent open -> tighten to 1 -> shift 2
                let gap = self.pat_gap % n;
                match self.pat_alt {
                    0 => {
                        let gap2 = (gap + 1) % n;
                        mask = full_mask & !((1u8 << gap) | (1u8 << gap2));
                        delta_sectors = 1;
                    }
                    1 => {
                        let keep = if self.pat_dir > 0 { gap } else { (gap + 1) % n };
                        mask = full_mask & !(1u8 << keep);
                        delta_sectors = 2;
                    }
                    _ => {
                        let g_far = (gap + 2) % n;
                        mask = full_mask & !(1u8 << g_far);
                        delta_sectors = 2;
                    }
                }
                self.pat_alt = self.pat_alt.wrapping_add(1);
                thick_q8 = 21 * Q8;
            }
        }

        let safe_mask = sanitize_wave_mask(mask, n, self.pat_gap);
        self.spawn_wave_at(safe_mask, SPAWN_R_Q8, thick_q8);

        self.pat_waves_left -= 1;
        // Extra breathing space between distinct patterns
        let extra = if self.pat_waves_left == 0 {
            speed_q8 * 8
        } else {
            0
        };
        self.spawn_dist_q8 = self.safe_pitch_q8(speed_q8, thick_q8, delta_sectors) + extra;
    }

    fn spawn_wave_at(&mut self, mask: u8, r_in_q8: i32, thick_q8: i32) {
        let mut s = 0u8;
        while s < self.sides {
            if (mask >> s) & 1 != 0 {
                for w in &mut self.walls {
                    if !w.active {
                        *w = Wall {
                            active: true,
                            sector: s,
                            r_in_q8,
                            thick_q8,
                        };
                        break;
                    }
                }
            }
            s += 1;
        }
    }
}

/// Guarantees every spawned wall wave has at least 1 open sector (NEVER impossible!).
#[inline]
pub fn sanitize_wave_mask(mask: u8, sides: u8, fallback_gap: u8) -> u8 {
    let n = sides.clamp(4, 6);
    let full = (1u8 << n) - 1;
    let m = mask & full;
    if m == full {
        full & !(1u8 << (fallback_gap % n))
    } else {
        m
    }
}

/// Compute FNV-1a 32-bit checksum over an arbitrary payload slice (salted so all-zeros fails).
fn checksum(data: &[u8]) -> u32 {
    let mut h: u32 = 0x811C_9DC5 ^ 0x5A3C_96A5;
    let mut i = 0;
    while i < data.len() {
        h ^= data[i] as u32;
        h = h.wrapping_mul(0x0100_0193);
        i += 1;
    }
    h
}

/// Encode best times `[u32; 5]` (centiseconds) into a 28-byte NAND payload (`HEX2`).
pub fn encode_save(best_cs: [u32; Mode::COUNT]) -> [u8; SAVE_LEN] {
    let mut body = [0u8; 24];
    body[0..4].copy_from_slice(&SAVE_MAGIC);
    let mut i = 0usize;
    while i < Mode::COUNT {
        let off = 4 + i * 4;
        body[off..off + 4].copy_from_slice(&best_cs[i].to_le_bytes());
        i += 1;
    }
    let sum = checksum(&body);
    let mut out = [0u8; SAVE_LEN];
    out[0..24].copy_from_slice(&body);
    out[24..28].copy_from_slice(&sum.to_le_bytes());
    out
}

/// Decode best times from a NAND payload (`HEX2`, or automatically migrates legacy `HEX1`).
pub fn decode_save(blob: &[u8]) -> [u32; Mode::COUNT] {
    let max_cs = 999_999u32; // 9999.99s sanity cap
    if blob.len() >= SAVE_LEN && blob[0..4] == SAVE_MAGIC {
        let expected = checksum(&blob[0..24]);
        let got = u32::from_le_bytes([blob[24], blob[25], blob[26], blob[27]]);
        if got != expected {
            return [0; Mode::COUNT];
        }
        let mut out = [0u32; Mode::COUNT];
        let mut i = 0usize;
        while i < Mode::COUNT {
            let off = 4 + i * 4;
            out[i] = u32::from_le_bytes([blob[off], blob[off + 1], blob[off + 2], blob[off + 3]])
                .min(max_cs);
            i += 1;
        }
        return out;
    }
    // Seamless migration from v1 (`HEX1` 3-mode save -> indices 2, 3, 4)
    if blob.len() >= SAVE_LEN_V1 && blob[0..4] == SAVE_MAGIC_V1 {
        let expected = checksum(&blob[0..16]);
        let got = u32::from_le_bytes([blob[16], blob[17], blob[18], blob[19]]);
        if got != expected {
            return [0; Mode::COUNT];
        }
        return [
            0,
            0,
            u32::from_le_bytes([blob[4], blob[5], blob[6], blob[7]]).min(max_cs),
            u32::from_le_bytes([blob[8], blob[9], blob[10], blob[11]]).min(max_cs),
            u32::from_le_bytes([blob[12], blob[13], blob[14], blob[15]]).min(max_cs),
        ];
    }
    [0; Mode::COUNT]
}

/// Format centiseconds as `"SS.CC"` (e.g. `1248` -> `"12.48"`, `5` -> `"00.05"`).
pub fn fmt_time<'a>(cs: u32, buf: &'a mut [u8; 12]) -> &'a str {
    let cs = cs.min(999_999);
    let secs = cs / 100;
    let frac = cs % 100;
    let mut i = buf.len();

    i -= 1;
    buf[i] = b'0' + (frac % 10) as u8;
    i -= 1;
    buf[i] = b'0' + (frac / 10) as u8;
    i -= 1;
    buf[i] = b'.';

    if secs < 10 {
        i -= 1;
        buf[i] = b'0' + secs as u8;
        i -= 1;
        buf[i] = b'0';
    } else {
        let mut s = secs;
        while s > 0 {
            i -= 1;
            buf[i] = b'0' + (s % 10) as u8;
            s /= 10;
        }
    }

    core::str::from_utf8(&buf[i..]).unwrap_or("00.00")
}

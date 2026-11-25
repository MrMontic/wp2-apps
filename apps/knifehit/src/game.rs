//! Core Knife Hit gameplay, procedural infinite level generator, 8 Bosses, physics,
//! Arsenal unlock/equip logic, and `KNF1` save persistence.
//!
//! Pure `no_std`, zero heap, zero floating-point math.

use crate::knives::{KNIVES, NUM_KNIVES};
use cmf_wp2_ui::motion::{Q14, cos, sin};

pub const SIZE: i32 = cmf_wp2_ui::SIZE;
pub const CX: i32 = cmf_wp2_ui::C; // 233
pub const TARGET_CY: i32 = 192;
pub const TARGET_R: i32 = 70;
pub const READY_TIP_Y: i32 = 342;
pub const KNIFE_SPEED: i32 = 36; // px per 18ms tick -> reaches target in ~3-4 frames (super snappy!)

/// World angle pointing straight DOWN (+Y) toward the player's waiting knife.
pub const BOTTOM_ANGLE: u16 = 16384;

/// Angular collision threshold between two knives (~12.1° in u16 turns).
pub const KNIFE_HIT_ARC: u16 = 2200;
/// Angular hit arc for slicing an apple on the target rim (~16.5°).
pub const APPLE_HIT_ARC: u16 = 3000;
/// Minimum safe angular separation between pre-embedded obstacle knives (~36°).
pub const MIN_OBSTACLE_GAP: u16 = 6550;
/// Minimum safe angular separation between an apple and any pre-embedded knife (~28.5°).
pub const MIN_APPLE_KNIFE_GAP: u16 = 5200;

pub const MAX_STUCK: usize = 18;
pub const MAX_APPLES: usize = 4;
pub const MAX_PARTICLES: usize = 16;
pub const MAX_CHUNKS: usize = 10;
pub const MAX_FLY_KNIVES: usize = 14;

pub const SAVE_LEN: usize = 23;
const SAVE_MAGIC: [u8; 4] = *b"KNF1";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum State {
    Title,
    Shop,
    Playing,
    Shatter,
    Bounce,
    Over,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BossKind {
    None,
    Cheese,
    Tomato,
    Tire,
    Watermelon,
    Pizza,
    Shield,
    Saw,
    CmfCore,
}

impl BossKind {
    pub fn from_boss_num(n: u16) -> Self {
        match (n.saturating_sub(1)) % 8 {
            0 => BossKind::Cheese,
            1 => BossKind::Tomato,
            2 => BossKind::Tire,
            3 => BossKind::Watermelon,
            4 => BossKind::Pizza,
            5 => BossKind::Shield,
            6 => BossKind::Saw,
            _ => BossKind::CmfCore,
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            BossKind::None => "",
            BossKind::Cheese => "THE CHEESE",
            BossKind::Tomato => "THE TOMATO",
            BossKind::Tire => "THE TIRE",
            BossKind::Watermelon => "THE MELON",
            BossKind::Pizza => "THE PIZZA",
            BossKind::Shield => "THE SHIELD",
            BossKind::Saw => "THE SAW",
            BossKind::CmfCore => "CMF CORE",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SpinProfile {
    Steady,
    SineWave,
    StopReverse,
    Ratchet,
    BurstBrake,
    BossFakeout,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct StuckKnife {
    pub active: bool,
    pub rel_angle: u16,
    /// Visual knife skin (obstacle knives use 0 = CLASSIC; player knives use selected_knife).
    pub skin: u8,
}

impl StuckKnife {
    pub const EMPTY: Self = Self { active: false, rel_angle: 0, skin: 0 };
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct RimApple {
    pub active: bool,
    pub rel_angle: u16,
}

impl RimApple {
    pub const EMPTY: Self = Self { active: false, rel_angle: 0 };
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Particle {
    pub active: bool,
    pub x: i16,
    pub y: i16,
    pub vx: i16,
    pub vy: i16,
    pub life: u8,
    pub size: u8,
    pub color: u16,
    /// 0 = normal splinter/spark, 1 = left apple half, 2 = right apple half
    pub kind: u8,
}

impl Particle {
    pub const EMPTY: Self = Self {
        active: false,
        x: 0,
        y: 0,
        vx: 0,
        vy: 0,
        life: 0,
        size: 0,
        color: 0,
        kind: 0,
    };
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ShatterChunk {
    pub active: bool,
    pub x: i16,
    pub y: i16,
    pub vx: i16,
    pub vy: i16,
    pub w: u8,
    pub h: u8,
    pub col_a: u16,
    pub col_b: u16,
}

impl ShatterChunk {
    pub const EMPTY: Self =
        Self { active: false, x: 0, y: 0, vx: 0, vy: 0, w: 0, h: 0, col_a: 0, col_b: 0 };
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct LooseKnife {
    pub active: bool,
    pub x: i16,
    pub y: i16,
    pub vx: i16,
    pub vy: i16,
    pub ang: u16,
    pub vang: i16,
    pub skin: u8,
}

impl LooseKnife {
    pub const EMPTY: Self =
        Self { active: false, x: 0, y: 0, vx: 0, vy: 0, ang: 0, vang: 0, skin: 0 };
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StepEvent {
    None,
    KnifeHit,
    AppleSliced,
    StageCleared,
    BossCleared,
    KnifeBounced,
    GameOver,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SaveData {
    pub best_stage: u16,
    pub best_score: u32,
    pub apples: u32,
    pub unlocked_mask: u32,
    pub selected_knife: u8,
    pub bosses_beaten: u16,
}

impl SaveData {
    pub const fn default_new() -> Self {
        Self {
            best_stage: 0,
            best_score: 0,
            apples: 0,
            unlocked_mask: 1, // bit 0 (CLASSIC) always unlocked
            selected_knife: 0,
            bosses_beaten: 0,
        }
    }

    pub fn is_unlocked(&self, id: u8) -> bool {
        let idx = id as usize;
        idx < NUM_KNIVES && ((self.unlocked_mask >> idx) & 1) != 0
    }

    pub fn unlocked_count(&self) -> u8 {
        (self.unlocked_mask & ((1u32 << NUM_KNIVES) - 1)).count_ones() as u8
    }

    pub fn can_afford(&self, id: u8) -> bool {
        let idx = id as usize;
        idx < NUM_KNIVES && !self.is_unlocked(id) && self.apples >= KNIVES[idx].cost as u32
    }

    /// Attempts to buy & equip knife `id` with apples. Returns true if newly unlocked.
    pub fn buy_knife(&mut self, id: u8) -> bool {
        if !self.can_afford(id) {
            return false;
        }
        self.apples -= KNIVES[id as usize].cost as u32;
        self.unlocked_mask |= 1u32 << id;
        self.selected_knife = id;
        true
    }

    /// Equips knife `id` if unlocked.
    pub fn equip_knife(&mut self, id: u8) -> bool {
        if self.is_unlocked(id) {
            self.selected_knife = id;
            true
        } else {
            false
        }
    }

    /// Unlocks knife `id` for free (e.g. boss defeat reward). Returns true if newly unlocked.
    pub fn grant_knife(&mut self, id: u8) -> bool {
        let idx = id as usize;
        if idx >= NUM_KNIVES || self.is_unlocked(id) {
            return false;
        }
        self.unlocked_mask |= 1u32 << idx;
        true
    }
}

fn save_checksum(bytes: &[u8]) -> u16 {
    let mut h: u16 = 0x5A3C;
    for &b in bytes {
        h = h.rotate_left(5) ^ (b as u16).wrapping_mul(0x0135);
        h = h.wrapping_add(0x9E37);
    }
    h
}

pub fn encode_save(d: &SaveData) -> [u8; SAVE_LEN] {
    let mut out = [0u8; SAVE_LEN];
    out[0..4].copy_from_slice(&SAVE_MAGIC);
    out[4..6].copy_from_slice(&d.best_stage.to_le_bytes());
    out[6..10].copy_from_slice(&d.best_score.to_le_bytes());
    out[10..14].copy_from_slice(&d.apples.to_le_bytes());
    let mask = d.unlocked_mask | 1;
    out[14..18].copy_from_slice(&mask.to_le_bytes());
    out[18] = if (d.selected_knife as usize) < NUM_KNIVES { d.selected_knife } else { 0 };
    out[19..21].copy_from_slice(&d.bosses_beaten.to_le_bytes());
    let csum = save_checksum(&out[0..21]);
    out[21..23].copy_from_slice(&csum.to_le_bytes());
    out
}

pub fn decode_save(blob: &[u8]) -> Option<SaveData> {
    if blob.len() < SAVE_LEN || blob[0..4] != SAVE_MAGIC {
        return None;
    }
    let want = u16::from_le_bytes([blob[21], blob[22]]);
    if save_checksum(&blob[0..21]) != want {
        return None;
    }
    let best_stage = u16::from_le_bytes([blob[4], blob[5]]);
    let best_score = u32::from_le_bytes([blob[6], blob[7], blob[8], blob[9]]);
    let apples = u32::from_le_bytes([blob[10], blob[11], blob[12], blob[13]]);
    let unlocked_mask = (u32::from_le_bytes([blob[14], blob[15], blob[16], blob[17]]) | 1)
        & ((1u32 << NUM_KNIVES) - 1);
    let mut selected_knife = blob[18];
    if (selected_knife as usize) >= NUM_KNIVES || ((unlocked_mask >> selected_knife) & 1) == 0 {
        selected_knife = 0;
    }
    let bosses_beaten = u16::from_le_bytes([blob[19], blob[20]]);
    Some(SaveData {
        best_stage,
        best_score,
        apples,
        unlocked_mask,
        selected_knife,
        bosses_beaten,
    })
}

/// Shortest angular distance between two `u16` angles (0..=32768).
#[inline]
pub fn angle_diff(a: u16, b: u16) -> u16 {
    (a.wrapping_sub(b) as i16).unsigned_abs()
}

pub struct Game {
    pub state: State,
    pub save: SaveData,
    pub save_dirty: bool,

    // Run progress
    pub stage: u16,
    pub score: u32,
    pub run_apples: u16,
    pub beat_best: bool,
    pub boss: BossKind,
    pub unlock_banner_knife: Option<u8>,

    // Stage target state
    pub target_angle: u16,
    pub spin_profile: SpinProfile,
    pub base_speed: i32,
    pub spin_dir: i32, // +1 or -1
    pub stage_tick: u32,
    pub recoil_y: i16,
    pub hit_flash: u8,

    // Knives
    pub knives_total: u8,
    pub knives_left: u8,
    pub flying: bool,
    pub fly_y: i32,
    pub throw_buf: u8,
    pub stuck: [StuckKnife; MAX_STUCK],
    pub apples: [RimApple; MAX_APPLES],

    // Juice / FX
    pub particles: [Particle; MAX_PARTICLES],
    pub chunks: [ShatterChunk; MAX_CHUNKS],
    pub loose_knives: [LooseKnife; MAX_FLY_KNIVES],
    pub bounce_knife: LooseKnife,
    pub phase_timer: u8,

    // Shop / UI state
    pub browse_knife: u8,
    pub shop_return: State,
    pub tick: u32,

    // PRNG
    rng: u32,
}

impl Game {
    pub const fn new_const() -> Self {
        Self {
            state: State::Title,
            save: SaveData::default_new(),
            save_dirty: false,
            stage: 1,
            score: 0,
            run_apples: 0,
            beat_best: false,
            boss: BossKind::None,
            unlock_banner_knife: None,
            target_angle: 0,
            spin_profile: SpinProfile::Steady,
            base_speed: 380,
            spin_dir: 1,
            stage_tick: 0,
            recoil_y: 0,
            hit_flash: 0,
            knives_total: 7,
            knives_left: 7,
            flying: false,
            fly_y: READY_TIP_Y,
            throw_buf: 0,
            stuck: [StuckKnife::EMPTY; MAX_STUCK],
            apples: [RimApple::EMPTY; MAX_APPLES],
            particles: [Particle::EMPTY; MAX_PARTICLES],
            chunks: [ShatterChunk::EMPTY; MAX_CHUNKS],
            loose_knives: [LooseKnife::EMPTY; MAX_FLY_KNIVES],
            bounce_knife: LooseKnife::EMPTY,
            phase_timer: 0,
            browse_knife: 0,
            shop_return: State::Title,
            tick: 0,
            rng: 0x9E37_79B9,
        }
    }

    pub fn init(&mut self, seed: u32, save: SaveData) {
        *self = Self::new_const();
        self.rng = if seed == 0 { 0xCAFE_BABE } else { seed };
        self.save = save;
        self.browse_knife = save.selected_knife;
        // Prepare a decorative Stage 1 target preview for Title screen
        self.setup_title_preview();
    }

    fn setup_title_preview(&mut self) {
        self.state = State::Title;
        self.boss = BossKind::None;
        self.target_angle = 0;
        self.recoil_y = 0;
        self.hit_flash = 0;
        self.stuck = [StuckKnife::EMPTY; MAX_STUCK];
        self.apples = [RimApple::EMPTY; MAX_APPLES];
        // 3 decorative stuck knives + 1 apple spinning gently on Title screen
        let skin = self.save.selected_knife;
        self.stuck[0] = StuckKnife { active: true, rel_angle: 5000, skin };
        self.stuck[1] = StuckKnife { active: true, rel_angle: 26000, skin };
        self.stuck[2] = StuckKnife { active: true, rel_angle: 48000, skin };
        self.apples[0] = RimApple { active: true, rel_angle: 37000 };
    }

    #[inline]
    fn next_u32(&mut self) -> u32 {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        x
    }

    #[inline]
    fn rand_range(&mut self, lo: u32, hi_excl: u32) -> u32 {
        if hi_excl <= lo {
            return lo;
        }
        lo + (self.next_u32() % (hi_excl - lo))
    }

    /// Starts a brand-new run from Stage 1.
    pub fn start_run(&mut self) {
        self.stage = 1;
        self.score = 0;
        self.run_apples = 0;
        self.beat_best = false;
        self.unlock_banner_knife = None;
        self.particles = [Particle::EMPTY; MAX_PARTICLES];
        self.chunks = [ShatterChunk::EMPTY; MAX_CHUNKS];
        self.loose_knives = [LooseKnife::EMPTY; MAX_FLY_KNIVES];
        self.bounce_knife = LooseKnife::EMPTY;
        self.generate_stage(1);
        self.state = State::Playing;
    }

    /// Opens the Knife Arsenal / Shop screen.
    pub fn open_shop(&mut self, from: State) {
        self.shop_return = from;
        self.browse_knife = self.save.selected_knife;
        self.state = State::Shop;
    }

    pub fn close_shop(&mut self) {
        if self.shop_return == State::Title {
            self.setup_title_preview();
        } else {
            self.state = self.shop_return;
        }
    }

    pub fn shop_step(&mut self, delta: i32) {
        if delta == 0 {
            return;
        }
        let n = NUM_KNIVES as i32;
        let cur = self.browse_knife as i32;
        let next = ((cur + delta) % n + n) % n;
        self.browse_knife = next as u8;
    }

    /// Attempts to equip (if unlocked) or purchase+equip (if affordable) the browsed knife.
    /// Returns: `0` = locked/insufficient apples, `1` = equipped, `2` = newly purchased!
    pub fn shop_activate(&mut self) -> u8 {
        let id = self.browse_knife;
        if self.save.is_unlocked(id) {
            if self.save.selected_knife != id {
                self.save.equip_knife(id);
                self.save_dirty = true;
            }
            1
        } else if self.save.buy_knife(id) {
            self.save_dirty = true;
            2
        } else {
            0
        }
    }

    /// Procedurally generates any stage `1..=999+` with guaranteed fair angular gaps,
    /// progressive difficulty, 6 rotation curve profiles, and 8 Bosses every 5th stage!
    pub fn generate_stage(&mut self, stage: u16) {
        let stage = stage.max(1);
        self.stage = stage;
        self.stage_tick = 0;
        self.recoil_y = 0;
        self.hit_flash = 0;
        self.flying = false;
        self.fly_y = READY_TIP_Y;
        self.throw_buf = 0;
        self.unlock_banner_knife = None;
        self.stuck = [StuckKnife::EMPTY; MAX_STUCK];
        self.apples = [RimApple::EMPTY; MAX_APPLES];
        self.chunks = [ShatterChunk::EMPTY; MAX_CHUNKS];
        self.loose_knives = [LooseKnife::EMPTY; MAX_FLY_KNIVES];
        self.bounce_knife = LooseKnife::EMPTY;
        self.target_angle = (self.next_u32() & 0xFFFF) as u16;

        let is_boss = stage % 5 == 0;
        self.boss = if is_boss { BossKind::from_boss_num(stage / 5) } else { BossKind::None };

        // 1. Knives to throw (6..=12, scaling gently with stage)
        let knives_to_throw: u8 = if stage == 1 {
            7
        } else if is_boss {
            (8 + (stage / 10).min(4)) as u8
        } else {
            let base = 7 + ((stage - 1) / 3).min(4) as u32;
            let jitter = self.rand_range(0, 3) as i32 - 1; // -1, 0, +1
            (base as i32 + jitter).clamp(6, 12) as u8
        };
        self.knives_total = knives_to_throw;
        self.knives_left = knives_to_throw;

        // 2. Pre-embedded obstacle knives (0 on Stage 1, 1..=4 thereafter, always fair!)
        let max_obs: usize = if stage == 1 {
            0
        } else if stage == 2 {
            1
        } else if stage <= 4 {
            self.rand_range(1, 3) as usize
        } else if is_boss {
            (2 + ((stage / 10) as usize).min(2)).min(4)
        } else {
            let cap = (2 + (stage / 6) as usize).min(4);
            self.rand_range(1, (cap + 1) as u32) as usize
        };

        // Ensure total knives (obstacles + to_throw) <= MAX_STUCK and plenty of angular room
        let obs_count = max_obs.min(MAX_STUCK.saturating_sub(knives_to_throw as usize));

        // Place obstacle knives with guaranteed safe separation >= MIN_OBSTACLE_GAP
        let mut placed = 0usize;
        if is_boss && obs_count >= 2 && (self.next_u32() & 1) == 0 {
            // Iconic symmetric boss obstacle pattern (evenly spaced around the wheel)
            let step = (65536u32 / obs_count as u32) as u16;
            let start = (self.next_u32() & 0xFFFF) as u16;
            for i in 0..obs_count {
                let ang = start.wrapping_add(step.wrapping_mul(i as u16));
                self.stuck[placed] = StuckKnife { active: true, rel_angle: ang, skin: 0 };
                placed += 1;
            }
        } else {
            let mut tries = 0u32;
            while placed < obs_count && tries < 64 {
                tries += 1;
                let cand = (self.next_u32() & 0xFFFF) as u16;
                let ok = self.stuck[..placed]
                    .iter()
                    .all(|k| angle_diff(cand, k.rel_angle) >= MIN_OBSTACLE_GAP);
                if ok {
                    self.stuck[placed] = StuckKnife { active: true, rel_angle: cand, skin: 0 };
                    placed += 1;
                }
            }
        }

        // 3. Pre-placed Apples (1 guaranteed on Stage 1, 0..=2 normal, 2..=3 on Bosses)
        let apple_count: usize = if stage == 1 {
            1
        } else if is_boss {
            self.rand_range(2, 4) as usize
        } else {
            let roll = self.rand_range(0, 100);
            if roll < 22 {
                0
            } else if roll < 76 {
                1
            } else {
                2
            }
        };

        let mut apples_placed = 0usize;
        let mut tries = 0u32;
        while apples_placed < apple_count && tries < 80 {
            tries += 1;
            let cand = (self.next_u32() & 0xFFFF) as u16;
            let far_knives = self.stuck[..placed]
                .iter()
                .all(|k| angle_diff(cand, k.rel_angle) >= MIN_APPLE_KNIFE_GAP);
            let far_apples = self.apples[..apples_placed]
                .iter()
                .all(|a| angle_diff(cand, a.rel_angle) >= MIN_OBSTACLE_GAP);
            if far_knives && far_apples {
                self.apples[apples_placed] = RimApple { active: true, rel_angle: cand };
                apples_placed += 1;
            }
        }

        // 4. Rotation curve profile & speed scaling
        self.spin_dir = if (self.next_u32() & 1) == 0 { 1 } else { -1 };
        let speed_boost = ((stage as i32 - 1) * 18).min(420);
        self.base_speed = if is_boss { 430 + speed_boost } else { 360 + speed_boost };

        self.spin_profile = match stage {
            1 => SpinProfile::Steady,
            2 => SpinProfile::SineWave,
            3 => SpinProfile::StopReverse,
            4 => SpinProfile::Ratchet,
            s if s % 5 == 0 => {
                // Bosses cycle dramatic spin patterns
                match (s / 5) % 3 {
                    1 => SpinProfile::BossFakeout,
                    2 => SpinProfile::StopReverse,
                    _ => SpinProfile::BurstBrake,
                }
            }
            _ => match self.rand_range(0, 6) {
                0 => SpinProfile::Steady,
                1 => SpinProfile::SineWave,
                2 => SpinProfile::StopReverse,
                3 => SpinProfile::Ratchet,
                4 => SpinProfile::BurstBrake,
                _ => SpinProfile::BossFakeout,
            },
        };
    }

    /// Computes angular delta (u16 turns) for the current tick based on `spin_profile`.
    pub fn current_spin_delta(&self) -> i32 {
        let t = self.stage_tick;
        let base = self.base_speed;
        let d = self.spin_dir;
        match self.spin_profile {
            SpinProfile::Steady => d * base,
            SpinProfile::SineWave => {
                // Period ~160 ticks (~2.9s): smoothly breathes between 0.35x and 1.65x
                let phase = ((t * 410) & 0xFFFF) as u16;
                let s = sin(phase); // -16384..+16384
                let factor = 1024 + (s * 660) / Q14;
                d * (base * factor / 1024)
            }
            SpinProfile::StopReverse => {
                // 240-tick cycle (~4.3s): forward 95 -> brake 25 -> reverse 95 -> brake 25
                let m = (t % 240) as i32;
                let mult = if m < 80 {
                    1024
                } else if m < 105 {
                    (105 - m) * 1024 / 25
                } else if m < 120 {
                    0
                } else if m < 135 {
                    -(m - 120) * 1024 / 15
                } else if m < 215 {
                    -1024
                } else if m < 230 {
                    -(230 - m) * 1024 / 15
                } else {
                    0
                };
                d * (base * mult / 1024)
            }
            SpinProfile::Ratchet => {
                // 44-tick ratchet (~0.8s): crisp fast surge then short freeze!
                let m = t % 44;
                let mult = if m < 18 {
                    1650
                } else if m < 26 {
                    600
                } else {
                    120
                };
                d * (base * mult / 1024)
            }
            SpinProfile::BurstBrake => {
                // 150-tick cycle (~2.7s): calm cruise -> sudden fast whip -> brake
                let m = t % 150;
                let mult = if m < 75 {
                    700
                } else if m < 115 {
                    1850
                } else if m < 132 {
                    220
                } else {
                    700
                };
                d * (base * mult / 1024)
            }
            SpinProfile::BossFakeout => {
                // 220-tick dramatic boss rhythm: wave + sharp reverse fakeout!
                let m = (t % 220) as i32;
                let mult = if m < 70 {
                    let ph = ((m * 900) & 0xFFFF) as u16;
                    950 + (sin(ph) * 550) / Q14
                } else if m < 88 {
                    (88 - m) * 900 / 18
                } else if m < 145 {
                    -1480
                } else if m < 162 {
                    -(162 - m) * 1400 / 17
                } else if m < 176 {
                    0
                } else {
                    1320
                };
                d * (base * mult / 1024)
            }
        }
    }

    /// Triggers a knife throw (with a 3-tick input buffer if a previous knife is mid-flight).
    pub fn request_throw(&mut self) {
        if self.state != State::Playing {
            return;
        }
        if !self.flying && self.knives_left > 0 {
            self.flying = true;
            self.fly_y = READY_TIP_Y;
            self.throw_buf = 0;
        } else if self.knives_left > 1 {
            self.throw_buf = 4;
        }
    }

    #[inline]
    pub fn target_cy(&self) -> i32 {
        TARGET_CY - self.recoil_y as i32
    }

    /// Primary wood/boss shatter color pair for splinters and chunks.
    pub fn target_chunk_colors(&self) -> (u16, u16) {
        use cmf_wp2_ui::color::rgb565;
        match self.boss {
            BossKind::None => (rgb565(214, 166, 108), rgb565(126, 76, 40)),
            BossKind::Cheese => (rgb565(250, 206, 54), rgb565(204, 146, 22)),
            BossKind::Tomato => (rgb565(234, 48, 42), rgb565(164, 24, 24)),
            BossKind::Tire => (rgb565(52, 56, 66), rgb565(174, 184, 198)),
            BossKind::Watermelon => (rgb565(232, 54, 68), rgb565(42, 152, 62)),
            BossKind::Pizza => (rgb565(246, 198, 68), rgb565(198, 128, 48)),
            BossKind::Shield => (rgb565(176, 190, 208), rgb565(46, 86, 156)),
            BossKind::Saw => (rgb565(194, 204, 218), rgb565(224, 48, 42)),
            BossKind::CmfCore => (rgb565(42, 46, 54), rgb565(255, 104, 24)),
        }
    }

    fn spawn_particle(
        &mut self,
        x: i32,
        y: i32,
        vx: i32,
        vy: i32,
        life: u8,
        size: u8,
        color: u16,
        kind: u8,
    ) {
        for p in &mut self.particles {
            if !p.active {
                *p = Particle {
                    active: true,
                    x: x as i16,
                    y: y as i16,
                    vx: vx as i16,
                    vy: vy as i16,
                    life,
                    size,
                    color,
                    kind,
                };
                return;
            }
        }
    }

    fn spawn_splinters(&mut self, cx: i32, cy: i32, count: usize) {
        let (c1, c2) = self.target_chunk_colors();
        for i in 0..count {
            let vx = (self.rand_range(0, 15) as i32 - 7) * 2;
            let vy = 2 + self.rand_range(0, 8) as i32;
            let col = if i & 1 == 0 { c1 } else { c2 };
            let sz = 3 + (i & 1) as u8 * 2;
            self.spawn_particle(cx + vx, cy, vx, vy, 16 + (i as u8 * 2), sz, col, 0);
        }
    }

    fn spawn_sparks(&mut self, cx: i32, cy: i32) {
        use cmf_wp2_ui::color::rgb565;
        let bright = rgb565(255, 244, 152);
        let orange = rgb565(255, 132, 36);
        for i in 0..8 {
            let vx = (self.rand_range(0, 19) as i32 - 9) * 2;
            let vy = self.rand_range(0, 13) as i32 - 4;
            let col = if i & 1 == 0 { bright } else { orange };
            self.spawn_particle(cx, cy, vx, vy, 14 + (i as u8), 4, col, 0);
        }
    }

    fn spawn_apple_slice(&mut self, ax: i32, ay: i32) {
        use cmf_wp2_ui::color::rgb565;
        // Two big juicy flying apple halves!
        self.spawn_particle(ax - 5, ay, -5, -4, 24, 10, rgb565(232, 38, 48), 1);
        self.spawn_particle(ax + 5, ay, 5, -4, 24, 10, rgb565(232, 38, 48), 2);
        // Juice droplets
        let juice = rgb565(255, 118, 122);
        let pulp = rgb565(252, 242, 196);
        for i in 0..4 {
            let vx = (self.rand_range(0, 13) as i32 - 6) * 2;
            let vy = self.rand_range(0, 8) as i32 - 3;
            let col = if i & 1 == 0 { juice } else { pulp };
            self.spawn_particle(ax, ay, vx, vy, 14, 4, col, 0);
        }
    }

    fn trigger_shatter(&mut self) {
        let (ca, cb) = self.target_chunk_colors();
        let cy = self.target_cy();
        for i in 0..MAX_CHUNKS {
            let ang = ((i as u32 * 65536 / MAX_CHUNKS as u32) as u16)
                .wrapping_add((self.next_u32() & 2047) as u16);
            let r = 20 + (self.rand_range(0, 34) as i32);
            let x = CX + (r * cos(ang)) / Q14;
            let y = cy + (r * sin(ang)) / Q14;
            let spd = 6 + self.rand_range(0, 7) as i32;
            let vx = (spd * cos(ang)) / Q14;
            let vy = (spd * sin(ang)) / Q14 - 4;
            let w = 14 + self.rand_range(0, 12) as u8;
            let h = 12 + self.rand_range(0, 12) as u8;
            self.chunks[i] = ShatterChunk {
                active: true,
                x: x as i16,
                y: y as i16,
                vx: vx as i16,
                vy: vy as i16,
                w,
                h,
                col_a: ca,
                col_b: cb,
            };
        }
        // Convert stuck knives into outward-flying tumbling loose knives!
        self.loose_knives = [LooseKnife::EMPTY; MAX_FLY_KNIVES];
        let mut li = 0usize;
        for k in &self.stuck {
            if k.active && li < MAX_FLY_KNIVES {
                let wa = self.target_angle.wrapping_add(k.rel_angle);
                let kx = CX + (TARGET_R * cos(wa)) / Q14;
                let ky = cy + (TARGET_R * sin(wa)) / Q14;
                let vx = (9 * cos(wa)) / Q14;
                let vy = (9 * sin(wa)) / Q14 - 3;
                let vang = if (li & 1) == 0 { 1600 } else { -1600 };
                self.loose_knives[li] = LooseKnife {
                    active: true,
                    x: kx as i16,
                    y: ky as i16,
                    vx: vx as i16,
                    vy: vy as i16,
                    ang: wa,
                    vang,
                    skin: k.skin,
                };
                li += 1;
            }
        }
        self.stuck = [StuckKnife::EMPTY; MAX_STUCK];
        self.apples = [RimApple::EMPTY; MAX_APPLES];
        self.phase_timer = 38;
        self.state = State::Shatter;
    }

    fn update_particles(&mut self) {
        for p in &mut self.particles {
            if !p.active {
                continue;
            }
            p.x = p.x.saturating_add(p.vx);
            p.y = p.y.saturating_add(p.vy);
            p.vy = p.vy.saturating_add(1); // gravity
            if p.life <= 1 || p.y > SIZE as i16 + 20 {
                p.active = false;
            } else {
                p.life -= 1;
            }
        }
    }

    fn check_apple_slice(&mut self) -> bool {
        let impact_rel = BOTTOM_ANGLE.wrapping_sub(self.target_angle);
        let mut sliced = false;
        let cy = self.target_cy();
        for i in 0..MAX_APPLES {
            if self.apples[i].active && angle_diff(impact_rel, self.apples[i].rel_angle) < APPLE_HIT_ARC {
                let wa = self.target_angle.wrapping_add(self.apples[i].rel_angle);
                let ax = CX + ((TARGET_R + 12) * cos(wa)) / Q14;
                let ay = cy + ((TARGET_R + 12) * sin(wa)) / Q14;
                self.apples[i].active = false;
                self.save.apples = self.save.apples.saturating_add(1);
                self.run_apples = self.run_apples.saturating_add(1);
                self.save_dirty = true;
                self.spawn_apple_slice(ax, ay);
                sliced = true;
            }
        }
        sliced
    }

    /// Advances simulation by 1 tick (~18ms).
    pub fn step(&mut self) -> StepEvent {
        self.tick = self.tick.wrapping_add(1);

        match self.state {
            State::Title => {
                // Gentle decorative rotation on Title screen
                self.target_angle = self.target_angle.wrapping_add(240);
                StepEvent::None
            }
            State::Shop | State::Over => StepEvent::None,
            State::Shatter => {
                self.update_particles();
                for ch in &mut self.chunks {
                    if ch.active {
                        ch.x = ch.x.saturating_add(ch.vx);
                        ch.y = ch.y.saturating_add(ch.vy);
                        ch.vy = ch.vy.saturating_add(1);
                        if ch.y > SIZE as i16 + 32 || ch.x < -32 || ch.x > SIZE as i16 + 32 {
                            ch.active = false;
                        }
                    }
                }
                for lk in &mut self.loose_knives {
                    if lk.active {
                        lk.x = lk.x.saturating_add(lk.vx);
                        lk.y = lk.y.saturating_add(lk.vy);
                        lk.vy = lk.vy.saturating_add(1);
                        lk.ang = lk.ang.wrapping_add(lk.vang as u16);
                        if lk.y > SIZE as i16 + 70 || lk.x < -70 || lk.x > SIZE as i16 + 70 {
                            lk.active = false;
                        }
                    }
                }
                if self.phase_timer > 1 {
                    self.phase_timer -= 1;
                } else {
                    let next_stage = self.stage.saturating_add(1);
                    self.generate_stage(next_stage);
                    self.state = State::Playing;
                }
                StepEvent::None
            }
            State::Bounce => {
                self.stage_tick = self.stage_tick.wrapping_add(1);
                let delta = self.current_spin_delta();
                self.target_angle = self.target_angle.wrapping_add(delta as u16);
                self.update_particles();
                if self.bounce_knife.active {
                    self.bounce_knife.x = self.bounce_knife.x.saturating_add(self.bounce_knife.vx);
                    self.bounce_knife.y = self.bounce_knife.y.saturating_add(self.bounce_knife.vy);
                    self.bounce_knife.vy = self.bounce_knife.vy.saturating_add(2);
                    self.bounce_knife.ang =
                        self.bounce_knife.ang.wrapping_add(self.bounce_knife.vang as u16);
                }
                if self.phase_timer > 1 {
                    self.phase_timer -= 1;
                    StepEvent::None
                } else {
                    self.state = State::Over;
                    StepEvent::GameOver
                }
            }
            State::Playing => {
                self.stage_tick = self.stage_tick.wrapping_add(1);
                let delta = self.current_spin_delta();
                self.target_angle = self.target_angle.wrapping_add(delta as u16);

                // Smooth spring recoil recovery & hit flash decay
                if self.recoil_y > 0 {
                    self.recoil_y = (self.recoil_y - 2).max(0);
                }
                if self.hit_flash > 0 {
                    self.hit_flash -= 1;
                }
                self.update_particles();

                // Drain buffered throw if ready
                if !self.flying && self.throw_buf > 0 && self.knives_left > 0 {
                    self.flying = true;
                    self.fly_y = READY_TIP_Y;
                    self.throw_buf = 0;
                } else if self.throw_buf > 0 {
                    self.throw_buf -= 1;
                }

                let mut ev = StepEvent::None;
                if self.flying {
                    self.fly_y -= KNIFE_SPEED;
                    let rim_y = self.target_cy() + TARGET_R;

                    // Check apple slice as knife enters apple zone
                    if self.fly_y <= rim_y + 24 {
                        if self.check_apple_slice() {
                            ev = StepEvent::AppleSliced;
                        }
                    }

                    // Check target rim contact!
                    if self.fly_y <= rim_y {
                        self.flying = false;
                        let impact_rel = BOTTOM_ANGLE.wrapping_sub(self.target_angle);

                        // Check collision with any already-stuck knife
                        let hit_knife = self
                            .stuck
                            .iter()
                            .any(|k| k.active && angle_diff(impact_rel, k.rel_angle) < KNIFE_HIT_ARC);

                        if hit_knife {
                            // Metallic ricochet!
                            let side = if (self.next_u32() & 1) == 0 { -6 } else { 6 };
                            self.bounce_knife = LooseKnife {
                                active: true,
                                x: CX as i16,
                                y: (rim_y + 8) as i16,
                                vx: side,
                                vy: 4,
                                ang: BOTTOM_ANGLE,
                                vang: if side < 0 { -2600 } else { 2600 },
                                skin: self.save.selected_knife,
                            };
                            self.spawn_sparks(CX, rim_y + 4);
                            self.phase_timer = 28;
                            self.state = State::Bounce;

                            // Update bests on run end
                            if self.stage > self.save.best_stage {
                                self.save.best_stage = self.stage;
                                self.beat_best = true;
                                self.save_dirty = true;
                            }
                            if self.score > self.save.best_score {
                                self.save.best_score = self.score;
                                self.beat_best = true;
                                self.save_dirty = true;
                            }
                            return StepEvent::KnifeBounced;
                        }

                        // Clean embed into wood/boss!
                        for slot in &mut self.stuck {
                            if !slot.active {
                                *slot = StuckKnife {
                                    active: true,
                                    rel_angle: impact_rel,
                                    skin: self.save.selected_knife,
                                };
                                break;
                            }
                        }

                        self.knives_left = self.knives_left.saturating_sub(1);
                        self.score = self.score.saturating_add(1);
                        self.recoil_y = 6;
                        self.hit_flash = 3;
                        self.spawn_splinters(CX, rim_y, 6);

                        if self.score > self.save.best_score {
                            self.save.best_score = self.score;
                            self.beat_best = true;
                            self.save_dirty = true;
                        }
                        if self.stage > self.save.best_stage {
                            self.save.best_stage = self.stage;
                            self.beat_best = true;
                            self.save_dirty = true;
                        }

                        if self.knives_left == 0 {
                            // Stage Cleared!
                            let was_boss = self.boss != BossKind::None;
                            if was_boss {
                                self.save.apples = self.save.apples.saturating_add(5);
                                self.run_apples = self.run_apples.saturating_add(5);
                                self.save.bosses_beaten = self.save.bosses_beaten.saturating_add(1);
                                self.save_dirty = true;
                                // Boss #1 -> unlock SCYTHE (14), Boss #2 -> unlock ROYAL (15)!
                                if self.save.bosses_beaten >= 1 && self.save.grant_knife(14) {
                                    self.unlock_banner_knife = Some(14);
                                } else if self.save.bosses_beaten >= 2 && self.save.grant_knife(15) {
                                    self.unlock_banner_knife = Some(15);
                                }
                            }
                            self.trigger_shatter();
                            return if was_boss {
                                StepEvent::BossCleared
                            } else {
                                StepEvent::StageCleared
                            };
                        }

                        // Next knife ready at bottom
                        self.fly_y = READY_TIP_Y;
                        if ev == StepEvent::None {
                            ev = StepEvent::KnifeHit;
                        }
                    }
                }
                ev
            }
        }
    }
}

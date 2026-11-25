//! Core Slither.io fixed-point simulation, smart AI bots, collision/loot mechanics, and persistence.
//! Pure `no_std`, zero heap allocations, zero floating-point math.

use crate::palette::SKIN_COUNT;
use cmf_wp2_ui::motion::{angle, cos, isqrt, sin};

/// Fixed-point shift for head sub-pixel movement (256 units = 1 world unit).
pub const Q_SHIFT: i32 = 8;

/// Circular arena radius in world units (diameter 2800 vs ~464 viewport = ~36x area!).
pub const ARENA_R: i32 = 1400;

/// Maximum simultaneous snakes: index 0 = player, indices 1..MAX_SNAKES = 7 smart AI bots.
pub const MAX_SNAKES: usize = 8;
/// Ring buffer capacity of body segments per snake (uniformly spaced by `SEG_STEP`).
pub const MAX_SEGS: usize = 144;
/// World units between consecutive body segments.
pub const SEG_STEP: i32 = 8;

/// Maximum glowing orbs in the arena (ambient pellets + boost trails + death loot).
pub const MAX_ORBS: usize = 220;
/// Desired ambient orb population kept replenished across the arena.
pub const AMBIENT_TARGET: usize = 148;

pub const START_MASS: u32 = 15;
pub const MIN_BOOST_MASS: u32 = 18;

/// Normal speed: 3.25 world units/tick in Q8 (~146 units/s at 45 FPS).
pub const SPEED_NORMAL: i32 = 832;
/// Boost speed: 6.0 world units/tick in Q8 (~270 units/s at 45 FPS).
pub const SPEED_BOOST: i32 = 1536;
/// Ticks of boosting before burning 1 mass & dropping a glowing orb behind the tail.
pub const BOOST_BURN_TICKS: u8 = 10;

/// Crown turning responsiveness (u16 turns per detent; 65536 = 360 deg).
pub const CROWN_TURN_STEP: i32 = 2700;

/// Persistence format: 16 bytes (`SLT1` magic + best_len + best_kills + skin + checksum).
pub const SAVE_LEN: usize = 16;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Screen {
    Title,
    Playing,
    Over,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DeathCause {
    None,
    Wall,
    Snake,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StepEvent {
    None,
    AteBigOrb,
    KilledBot,
    NewBest,
    PlayerDied,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BotStyle {
    Hunter,
    Scavenger,
    Farmer,
}

#[derive(Clone, Copy)]
pub struct SaveData {
    pub best_len: u32,
    pub best_kills: u16,
    pub skin: u8,
}

impl SaveData {
    pub const fn default_new() -> Self {
        Self { best_len: 0, best_kills: 0, skin: 0 }
    }
}

fn save_checksum(bytes: &[u8; 12]) -> u32 {
    let mut h: u32 = 0x811C_9DC5;
    let mut i = 0;
    while i < 12 {
        h ^= bytes[i] as u32;
        h = h.wrapping_mul(0x0100_0193);
        i += 1;
    }
    h
}

pub fn encode_save(s: SaveData) -> [u8; SAVE_LEN] {
    let mut out = [0u8; SAVE_LEN];
    out[0..4].copy_from_slice(b"SLT1");
    out[4..8].copy_from_slice(&s.best_len.to_le_bytes());
    out[8..10].copy_from_slice(&s.best_kills.to_le_bytes());
    out[10] = s.skin % SKIN_COUNT as u8;
    out[11] = 0;
    let mut hdr = [0u8; 12];
    hdr.copy_from_slice(&out[0..12]);
    let csum = save_checksum(&hdr);
    out[12..16].copy_from_slice(&csum.to_le_bytes());
    out
}

pub fn decode_save(buf: &[u8]) -> Option<SaveData> {
    if buf.len() < SAVE_LEN || &buf[0..4] != b"SLT1" {
        return None;
    }
    let mut hdr = [0u8; 12];
    hdr.copy_from_slice(&buf[0..12]);
    let want = save_checksum(&hdr);
    let got = u32::from_le_bytes([buf[12], buf[13], buf[14], buf[15]]);
    if want != got {
        return None;
    }
    let skin = buf[10];
    if (skin as usize) >= SKIN_COUNT {
        return None;
    }
    let best_len = u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]);
    let best_kills = u16::from_le_bytes([buf[8], buf[9]]);
    Some(SaveData { best_len, best_kills, skin })
}

#[derive(Clone, Copy)]
pub struct Orb {
    pub x: i16,
    pub y: i16,
    /// `0` = inactive; `1..=2` = ambient/boost pellet; `3..=12` = glowing death loot.
    pub val: u8,
    /// Color index `0..8`.
    pub color: u8,
    /// Pulse animation phase.
    pub phase: u8,
    pub _pad: u8,
}

impl Orb {
    pub const fn empty() -> Self {
        Self { x: 0, y: 0, val: 0, color: 0, phase: 0, _pad: 0 }
    }
}

#[derive(Clone, Copy)]
pub struct Snake {
    pub alive: bool,
    pub is_player: bool,
    pub skin: u8,
    pub style: BotStyle,

    /// Head position in Q8 fixed point.
    pub hx: i32,
    pub hy: i32,
    /// Current heading and desired steering angle (u16 turns: 65536 = 360 deg).
    pub ang: u16,
    pub target_ang: u16,

    /// Length score (`LEN`), grows as orbs are eaten.
    pub mass: u32,
    pub boosting: bool,
    pub boost_acc: u8,

    /// Respawn timer for dead bots (ticks remaining).
    pub respawn_ticks: u16,
    /// AI boost burst timer and hunt cooldown.
    pub ai_boost_ticks: u8,
    pub ai_hunt_cd: u8,
    pub ai_wander_phase: u16,

    /// Cached world-space AABB for fast collision & AI pruning.
    pub min_x: i16,
    pub max_x: i16,
    pub min_y: i16,
    pub max_y: i16,

    /// Uniformly spaced body trail ring buffer (index `head_idx` is newest sample).
    pub head_idx: u16,
    pub trail_len: u16,
    pub trail_x: [i16; MAX_SEGS],
    pub trail_y: [i16; MAX_SEGS],
}

impl Snake {
    pub const fn zeroed() -> Self {
        Self {
            alive: false,
            is_player: false,
            skin: 0,
            style: BotStyle::Farmer,
            hx: 0,
            hy: 0,
            ang: 0,
            target_ang: 0,
            mass: START_MASS,
            boosting: false,
            boost_acc: 0,
            respawn_ticks: 0,
            ai_boost_ticks: 0,
            ai_hunt_cd: 0,
            ai_wander_phase: 0,
            min_x: 0,
            max_x: 0,
            min_y: 0,
            max_y: 0,
            head_idx: 0,
            trail_len: 0,
            trail_x: [0; MAX_SEGS],
            trail_y: [0; MAX_SEGS],
        }
    }

    #[inline]
    pub fn head_wx(&self) -> i32 {
        self.hx >> Q_SHIFT
    }

    #[inline]
    pub fn head_wy(&self) -> i32 {
        self.hy >> Q_SHIFT
    }

    /// Number of visible/collidable body segments for current mass.
    #[inline]
    pub fn seg_count(&self) -> usize {
        let target = mass_to_segs(self.mass);
        target.min(self.trail_len as usize).max(1)
    }

    /// Body radius in world units (grows smoothly with mass!).
    #[inline]
    pub fn body_radius(&self) -> i32 {
        mass_to_radius(self.mass)
    }

    /// World position of segment `k` (`0` = head, `1..seg_count` = trailing body).
    #[inline]
    pub fn seg_pos(&self, k: usize) -> (i32, i32) {
        if k == 0 {
            return (self.head_wx(), self.head_wy());
        }
        let max_k = (self.trail_len as usize).saturating_sub(1);
        let kk = k.min(max_k);
        let idx = (self.head_idx as usize + MAX_SEGS - kk) % MAX_SEGS;
        (self.trail_x[idx] as i32, self.trail_y[idx] as i32)
    }

    /// Tail world position (used for dropping boost orbs).
    #[inline]
    pub fn tail_pos(&self) -> (i32, i32) {
        let last = self.seg_count().saturating_sub(1);
        self.seg_pos(last)
    }

    /// Initialize snake in-place with a pre-laid smooth trail behind its head.
    pub fn spawn(
        &mut self,
        is_player: bool,
        skin: u8,
        style: BotStyle,
        wx: i32,
        wy: i32,
        ang: u16,
        mass: u32,
        wander_seed: u16,
    ) {
        self.alive = true;
        self.is_player = is_player;
        self.skin = skin % SKIN_COUNT as u8;
        self.style = style;
        self.hx = wx << Q_SHIFT;
        self.hy = wy << Q_SHIFT;
        self.ang = ang;
        self.target_ang = ang;
        self.mass = mass;
        self.boosting = false;
        self.boost_acc = 0;
        self.respawn_ticks = 0;
        self.ai_boost_ticks = 0;
        self.ai_hunt_cd = 25;
        self.ai_wander_phase = wander_seed;

        let segs = mass_to_segs(mass).min(MAX_SEGS);
        self.head_idx = 0;
        self.trail_len = segs as u16;

        // Lay trail directly behind heading so spawned snake immediately has full length.
        let back = ang.wrapping_add(32768);
        let bx = cos(back);
        let by = sin(back);
        let mut i = 0usize;
        while i < segs {
            let dist = (i as i32) * SEG_STEP;
            let sx = (wx + ((bx * dist) >> 14)).clamp(-ARENA_R + 32, ARENA_R - 32);
            let sy = (wy + ((by * dist) >> 14)).clamp(-ARENA_R + 32, ARENA_R - 32);
            let idx = (MAX_SEGS - i) % MAX_SEGS;
            self.trail_x[idx] = sx as i16;
            self.trail_y[idx] = sy as i16;
            i += 1;
        }
        self.recompute_aabb();
    }

    pub fn recompute_aabb(&mut self) {
        let (hx, hy) = (self.head_wx(), self.head_wy());
        let mut min_x = hx;
        let mut max_x = hx;
        let mut min_y = hy;
        let mut max_y = hy;
        let n = self.seg_count();
        let mut k = 1usize;
        while k < n {
            let (sx, sy) = self.seg_pos(k);
            if sx < min_x { min_x = sx; }
            if sx > max_x { max_x = sx; }
            if sy < min_y { min_y = sy; }
            if sy > max_y { max_y = sy; }
            k += 2;
        }
        if n > 1 {
            let (tx, ty) = self.seg_pos(n - 1);
            if tx < min_x { min_x = tx; }
            if tx > max_x { max_x = tx; }
            if ty < min_y { min_y = ty; }
            if ty > max_y { max_y = ty; }
        }
        let pad = self.body_radius() + 6;
        self.min_x = (min_x - pad).clamp(i16::MIN as i32, i16::MAX as i32) as i16;
        self.max_x = (max_x + pad).clamp(i16::MIN as i32, i16::MAX as i32) as i16;
        self.min_y = (min_y - pad).clamp(i16::MIN as i32, i16::MAX as i32) as i16;
        self.max_y = (max_y + pad).clamp(i16::MIN as i32, i16::MAX as i32) as i16;
    }

    /// Turn `ang` smoothly toward `target_ang`, capped by size-appropriate turn rate.
    #[inline]
    pub fn turn_toward_target(&mut self) {
        let diff = self.target_ang.wrapping_sub(self.ang) as i16 as i32;
        // Slightly tighter turn for nimble small snakes, heavier arc for giant Leviathans.
        let r = self.body_radius();
        let max_turn = if self.is_player {
            (2700 - (r - 10) * 60).max(1850)
        } else {
            (2450 - (r - 10) * 55).max(1700)
        };
        let step = diff.clamp(-max_turn, max_turn);
        self.ang = self.ang.wrapping_add(step as u16);
    }

    /// Advance head along heading and push uniformly spaced trail points.
    pub fn advance_body(&mut self) -> bool {
        let can_boost = self.boosting && self.mass > MIN_BOOST_MASS;
        self.boosting = can_boost;
        let speed = if can_boost { SPEED_BOOST } else { SPEED_NORMAL };

        let vx = (cos(self.ang) * speed) >> 14;
        let vy = (sin(self.ang) * speed) >> 14;
        self.hx += vx;
        self.hy += vy;

        let wx = self.head_wx();
        let wy = self.head_wy();

        let mut lx = self.trail_x[self.head_idx as usize] as i32;
        let mut ly = self.trail_y[self.head_idx as usize] as i32;
        let mut dx = wx - lx;
        let mut dy = wy - ly;
        let mut guard = 0;
        while dx * dx + dy * dy >= SEG_STEP * SEG_STEP && guard < 4 {
            let d = isqrt(dx * dx + dy * dy).max(1);
            let nx = lx + (dx * SEG_STEP) / d;
            let ny = ly + (dy * SEG_STEP) / d;
            self.head_idx = ((self.head_idx as usize + 1) % MAX_SEGS) as u16;
            self.trail_x[self.head_idx as usize] = nx as i16;
            self.trail_y[self.head_idx as usize] = ny as i16;
            if (self.trail_len as usize) < MAX_SEGS {
                self.trail_len += 1;
            }
            lx = nx;
            ly = ny;
            dx = wx - lx;
            dy = wy - ly;
            guard += 1;
        }

        self.recompute_aabb();

        // Boost mass drain & orb drop behind tail
        if can_boost {
            self.boost_acc += 1;
            if self.boost_acc >= BOOST_BURN_TICKS {
                self.boost_acc = 0;
                if self.mass > MIN_BOOST_MASS {
                    self.mass -= 1;
                    return true; // Caller drops a glowing orb at tail!
                }
            }
        } else {
            self.boost_acc = 0;
        }
        false
    }
}

#[inline]
pub fn mass_to_segs(mass: u32) -> usize {
    let m = (mass as i32).min(5000).max(0);
    (8 + isqrt(m * 9) as usize).clamp(10, MAX_SEGS)
}

#[inline]
pub fn mass_to_radius(mass: u32) -> i32 {
    let m = (mass as i32).min(2000).max(0);
    (10 + isqrt(m) / 4).clamp(10, 20)
}

/// Signed angular delta `a - b` in `[-32768, 32767]`.
#[inline]
pub fn ang_diff(a: u16, b: u16) -> i32 {
    a.wrapping_sub(b) as i16 as i32
}

/// Complete game state (lives in `static mut` on watch — ~7.2 KB RAM, zero stack bloat).
pub struct Game {
    pub screen: Screen,
    pub tick: u32,
    pub rng: u32,

    pub save: SaveData,
    pub selected_skin: u8,

    /// Camera center in Q8 fixed-point (smoothly tracks player head).
    pub cam_x: i32,
    pub cam_y: i32,

    /// Player touch boost state.
    pub boost_held: bool,

    /// Match stats for HUD and Game Over summary.
    pub kills: u16,
    pub final_len: u32,
    pub best_rank: u8,
    pub new_best_achieved: bool,
    pub death_cause: DeathCause,
    pub kill_flash_ticks: u8,

    /// Active ambient orb count (for fast replenishment).
    pub active_orbs: u16,

    pub snakes: [Snake; MAX_SNAKES],
    pub orbs: [Orb; MAX_ORBS],
}

impl Game {
    pub const fn zeroed() -> Self {
        Self {
            screen: Screen::Title,
            tick: 0,
            rng: 0x51A7_E210,
            save: SaveData::default_new(),
            selected_skin: 0,
            cam_x: 0,
            cam_y: 0,
            boost_held: false,
            kills: 0,
            final_len: START_MASS,
            best_rank: MAX_SNAKES as u8,
            new_best_achieved: false,
            death_cause: DeathCause::None,
            kill_flash_ticks: 0,
            active_orbs: 0,
            snakes: [Snake::zeroed(); MAX_SNAKES],
            orbs: [Orb::empty(); MAX_ORBS],
        }
    }

    pub fn init(&mut self, seed: u32, save: SaveData) {
        self.screen = Screen::Title;
        self.tick = 0;
        self.rng = if seed == 0 { 0xA51F_9137 } else { seed };
        self.save = save;
        self.selected_skin = save.skin % SKIN_COUNT as u8;
        self.cam_x = 0;
        self.cam_y = 0;
        self.boost_held = false;
        self.kills = 0;
        self.final_len = START_MASS;
        self.best_rank = MAX_SNAKES as u8;
        self.new_best_achieved = false;
        self.death_cause = DeathCause::None;
        self.kill_flash_ticks = 0;
        self.clear_world();
    }

    #[inline]
    pub fn next_rand(&mut self) -> u32 {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        if x == 0 {
            x = 0x6D2B_79F5;
        }
        self.rng = x;
        x
    }

    #[inline]
    pub fn rand_range(&mut self, lo: i32, hi: i32) -> i32 {
        if hi <= lo {
            return lo;
        }
        let span = (hi - lo) as u32;
        lo + (self.next_rand() % span) as i32
    }

    pub fn cycle_skin(&mut self, delta: i32) {
        let cur = self.selected_skin as i32;
        let next = (cur + delta).rem_euclid(SKIN_COUNT as i32) as u8;
        self.selected_skin = next;
        self.save.skin = next;
    }

    fn clear_world(&mut self) {
        let mut i = 0;
        while i < MAX_SNAKES {
            self.snakes[i].alive = false;
            self.snakes[i].respawn_ticks = 0;
            i += 1;
        }
        let mut o = 0;
        while o < MAX_ORBS {
            self.orbs[o].val = 0;
            o += 1;
        }
        self.active_orbs = 0;
    }

    /// Start a brand new Slither.io match!
    pub fn start_match(&mut self) {
        self.clear_world();
        self.screen = Screen::Playing;
        self.tick = 0;
        self.boost_held = false;
        self.kills = 0;
        self.final_len = START_MASS;
        self.best_rank = MAX_SNAKES as u8;
        self.new_best_achieved = false;
        self.death_cause = DeathCause::None;
        self.kill_flash_ticks = 0;

        // Spawn player near center with random heading
        let p_ang = (self.next_rand() & 0xFFFF) as u16;
        let px = self.rand_range(-120, 120);
        let py = self.rand_range(-120, 120);
        self.snakes[0].spawn(
            true,
            self.selected_skin,
            BotStyle::Farmer,
            px,
            py,
            p_ang,
            START_MASS,
            0,
        );
        self.cam_x = self.snakes[0].hx;
        self.cam_y = self.snakes[0].hy;

        // Spawn 7 diverse smart AI bots (nimble hunters, scavengers, and 2 big boss worms!)
        let styles = [
            BotStyle::Hunter,
            BotStyle::Scavenger,
            BotStyle::Farmer,
            BotStyle::Hunter,
            BotStyle::Scavenger,
            BotStyle::Farmer,
            BotStyle::Hunter,
        ];
        let start_masses: [u32; 7] = [24, 38, 65, 95, 130, 195, 280];

        let mut i = 1usize;
        while i < MAX_SNAKES {
            let style = styles[(i - 1) % styles.len()];
            let mass = start_masses[(i - 1) % start_masses.len()];
            let skin = ((self.selected_skin as usize + i) % SKIN_COUNT) as u8;
            self.spawn_bot_safely(i, skin, style, mass);
            i += 1;
        }

        // Populate rich field of ambient orbs across the arena
        let mut n = 0;
        while n < AMBIENT_TARGET {
            self.spawn_ambient_orb();
            n += 1;
        }
    }

    fn spawn_bot_safely(&mut self, slot: usize, skin: u8, style: BotStyle, mass: u32) {
        let px = self.snakes[0].head_wx();
        let py = self.snakes[0].head_wy();
        let mut best_x = 500;
        let mut best_y = 0;

        let mut attempt = 0;
        while attempt < 20 {
            let a = (self.next_rand() & 0xFFFF) as u16;
            let r = self.rand_range(260, ARENA_R - 300);
            let cx = (cos(a) * r) >> 14;
            let cy = (sin(a) * r) >> 14;

            // Keep safe distance from player head so bots never pop in right on top of you
            let dp_x = cx - px;
            let dp_y = cy - py;
            if self.snakes[0].alive && dp_x * dp_x + dp_y * dp_y < 340 * 340 {
                attempt += 1;
                continue;
            }

            // Avoid spawning directly on another bot's head
            let mut clear = true;
            let mut j = 1;
            while j < MAX_SNAKES {
                if j != slot && self.snakes[j].alive {
                    let dx = cx - self.snakes[j].head_wx();
                    let dy = cy - self.snakes[j].head_wy();
                    if dx * dx + dy * dy < 160 * 160 {
                        clear = false;
                        break;
                    }
                }
                j += 1;
            }
            best_x = cx;
            best_y = cy;
            if clear {
                break;
            }
            attempt += 1;
        }

        // Point generally inward so newly spawned bots glide nicely into the arena
        let inward = angle(-best_x, -best_y);
        let jitter = self.rand_range(-10000, 10000) as i16 as u16;
        let heading = inward.wrapping_add(jitter);
        let wseed = (self.next_rand() & 0xFFFF) as u16;
        self.snakes[slot].spawn(false, skin, style, best_x, best_y, heading, mass, wseed);
    }

    fn spawn_ambient_orb(&mut self) {
        // Find free slot
        let mut slot = None;
        let mut i = 0;
        while i < MAX_ORBS {
            if self.orbs[i].val == 0 {
                slot = Some(i);
                break;
            }
            i += 1;
        }
        let Some(idx) = slot else { return };

        // Uniform area distribution inside radius (ARENA_R - 90)
        let a = (self.next_rand() & 0xFFFF) as u16;
        let u = (self.next_rand() % 10000) as i32;
        let max_r = ARENA_R - 90;
        let r = (isqrt(u) * max_r) / 100;
        let x = ((cos(a) * r) >> 14) as i16;
        let y = ((sin(a) * r) >> 14) as i16;
        let val = if (self.next_rand() & 7) == 0 { 2 } else { 1 };
        let color = (self.next_rand() & 7) as u8;
        let phase = (self.next_rand() & 255) as u8;

        self.orbs[idx] = Orb { x, y, val, color, phase, _pad: 0 };
        self.active_orbs += 1;
    }

    /// Spawn a boost-drop or death-loot orb, evicting a distant small ambient orb if pool is full.
    pub fn spawn_loot_orb(&mut self, wx: i32, wy: i32, val: u8, color: u8) {
        if val == 0 {
            return;
        }
        // Clamp strictly inside arena
        let d2 = (wx as i64) * (wx as i64) + (wy as i64) * (wy as i64);
        let max_r = ARENA_R - 40;
        let (fx, fy) = if d2 > (max_r as i64) * (max_r as i64) {
            let d = isqrt(d2.min(i32::MAX as i64) as i32).max(1);
            ((wx * max_r) / d, (wy * max_r) / d)
        } else {
            (wx, wy)
        };

        let mut target_slot: Option<usize> = None;
        let mut i = 0;
        while i < MAX_ORBS {
            if self.orbs[i].val == 0 {
                target_slot = Some(i);
                break;
            }
            i += 1;
        }

        let mut added = false;
        if target_slot.is_some() {
            added = true;
        } else if val >= 2 {
            // Evict the smallest ambient orb farthest from the player so fresh death loot always appears!
            let px = self.snakes[0].head_wx();
            let py = self.snakes[0].head_wy();
            let mut best_evict: Option<usize> = None;
            let mut best_score: i32 = -1;
            let mut k = 0;
            while k < MAX_ORBS {
                if self.orbs[k].val <= 1 {
                    let md = (self.orbs[k].x as i32 - px).abs() + (self.orbs[k].y as i32 - py).abs();
                    if md > best_score {
                        best_score = md;
                        best_evict = Some(k);
                    }
                }
                k += 1;
            }
            target_slot = best_evict;
        }

        if let Some(idx) = target_slot {
            let phase = (self.next_rand() & 255) as u8;
            self.orbs[idx] = Orb {
                x: fx as i16,
                y: fy as i16,
                val,
                color: color & 7,
                phase,
                _pad: 0,
            };
            if added {
                self.active_orbs += 1;
            }
        }
    }

    /// Apply crown rotation directly to player's desired heading.
    pub fn apply_crown(&mut self, delta: i32) {
        if delta == 0 {
            return;
        }
        if self.screen == Screen::Title {
            self.cycle_skin(delta.signum());
            return;
        }
        if self.screen != Screen::Playing || !self.snakes[0].alive {
            return;
        }
        let cur_diff = ang_diff(self.snakes[0].target_ang, self.snakes[0].ang);
        let new_diff = (cur_diff + delta * CROWN_TURN_STEP).clamp(-22000, 22000);
        self.snakes[0].target_ang = self.snakes[0].ang.wrapping_add(new_diff as u16);
    }

    /// Current player rank (`1..=alive_count`) and total alive snakes.
    pub fn player_rank(&self) -> (u8, u8) {
        let p_mass = self.snakes[0].mass;
        let mut rank = 1u8;
        let mut total = 0u8;
        let mut i = 0;
        while i < MAX_SNAKES {
            if self.snakes[i].alive {
                total += 1;
                if i != 0 && self.snakes[i].mass > p_mass {
                    rank += 1;
                }
            }
            i += 1;
        }
        (rank, total.max(1))
    }

    /// Advance one fixed simulation tick (~45 Hz).
    pub fn step(&mut self) -> StepEvent {
        self.tick = self.tick.wrapping_add(1);
        if self.screen != Screen::Playing {
            return StepEvent::None;
        }
        if self.kill_flash_ticks > 0 {
            self.kill_flash_ticks -= 1;
        }

        let mut evt = StepEvent::None;

        // 1. Update player boost intent
        self.snakes[0].boosting = self.boost_held && self.snakes[0].mass > MIN_BOOST_MASS;

        // 2. Smart AI Bot decisions (staggered every 2 ticks for CPU efficiency + continuous wall guard)
        let mut i = 1usize;
        while i < MAX_SNAKES {
            if self.snakes[i].alive {
                if ((self.tick as usize + i) & 1) == 0 {
                    self.think_bot(i);
                } else {
                    self.bot_wall_guard(i);
                }
            } else if self.snakes[i].respawn_ticks > 0 {
                self.snakes[i].respawn_ticks -= 1;
                if self.snakes[i].respawn_ticks == 0 {
                    let style = match self.next_rand() % 3 {
                        0 => BotStyle::Hunter,
                        1 => BotStyle::Scavenger,
                        _ => BotStyle::Farmer,
                    };
                    // Mix of fresh small bots and occasional heavyweight challenger
                    let mass = match self.next_rand() % 5 {
                        0 => self.rand_range(140, 240) as u32,
                        1 | 2 => self.rand_range(45, 110) as u32,
                        _ => self.rand_range(20, 48) as u32,
                    };
                    let skin = (self.next_rand() as usize % SKIN_COUNT) as u8;
                    self.spawn_bot_safely(i, skin, style, mass);
                }
            }
            i += 1;
        }

        // 3. Turn & advance all alive snakes, dropping boost orbs behind boosting tails
        let mut i = 0usize;
        while i < MAX_SNAKES {
            if self.snakes[i].alive {
                self.snakes[i].turn_toward_target();
                let dropped = self.snakes[i].advance_body();
                if dropped {
                    let (tx, ty) = self.snakes[i].tail_pos();
                    let col = self.snakes[i].skin;
                    self.spawn_loot_orb(tx, ty, 2, col);
                }
            }
            i += 1;
        }

        // 4. Orb magnet pull & eating
        let ate_big = self.update_orbs_and_eating();
        if ate_big && evt == StepEvent::None {
            evt = StepEvent::AteBigOrb;
        }

        // 5. Wall & Snake-vs-Snake collisions (authentic Slither.io: self body never kills!)
        let mut dead = [false; MAX_SNAKES];
        let mut killer = [255u8; MAX_SNAKES];
        let mut p_cause = DeathCause::None;

        let mut a = 0usize;
        while a < MAX_SNAKES {
            if !self.snakes[a].alive {
                a += 1;
                continue;
            }
            let hx = self.snakes[a].head_wx();
            let hy = self.snakes[a].head_wy();
            let ra = self.snakes[a].body_radius();

            // 5a. Circular arena wall collision
            let limit = (ARENA_R - ra) as i64;
            let d2 = (hx as i64) * (hx as i64) + (hy as i64) * (hy as i64);
            if d2 >= limit * limit {
                dead[a] = true;
                if a == 0 {
                    p_cause = DeathCause::Wall;
                }
                a += 1;
                continue;
            }

            // 5b. Head-vs-Other-Snake body collision
            let hit_ra = (ra * 3) / 4;
            let mut b = 0usize;
            while b < MAX_SNAKES {
                if a == b || !self.snakes[b].alive {
                    b += 1;
                    continue;
                }
                // Fast AABB rejection
                if hx < self.snakes[b].min_x as i32 - hit_ra
                    || hx > self.snakes[b].max_x as i32 + hit_ra
                    || hy < self.snakes[b].min_y as i32 - hit_ra
                    || hy > self.snakes[b].max_y as i32 + hit_ra
                {
                    b += 1;
                    continue;
                }
                let rb = self.snakes[b].body_radius();
                let thresh = hit_ra + rb - 2;
                let thresh2 = thresh * thresh;
                let segs_b = self.snakes[b].seg_count();
                let mut collided = false;

                // Head-to-head check (seg 0): smaller snake loses; if equal both collide
                let (bx0, by0) = self.snakes[b].seg_pos(0);
                let dx0 = hx - bx0;
                let dy0 = hy - by0;
                if dx0 * dx0 + dy0 * dy0 < thresh2 {
                    if self.snakes[a].mass <= self.snakes[b].mass {
                        collided = true;
                    }
                }

                // Body segments (1..segs_b)
                if !collided {
                    let mut k = 1usize;
                    while k < segs_b {
                        let (sx, sy) = self.snakes[b].seg_pos(k);
                        let dx = hx - sx;
                        let dy = hy - sy;
                        if dx * dx + dy * dy < thresh2 {
                            collided = true;
                            break;
                        }
                        k += 1;
                    }
                }

                if collided {
                    dead[a] = true;
                    killer[a] = b as u8;
                    if a == 0 {
                        p_cause = DeathCause::Snake;
                    }
                    break;
                }
                b += 1;
            }
            a += 1;
        }

        // 6. Process deaths -> explosive glowing loot trails!
        let mut s = 0usize;
        while s < MAX_SNAKES {
            if dead[s] {
                self.burst_snake_into_loot(s);
                self.snakes[s].alive = false;
                if s == 0 {
                    self.death_cause = p_cause;
                    self.final_len = self.snakes[0].mass;
                    if self.final_len > self.save.best_len {
                        self.save.best_len = self.final_len;
                        self.new_best_achieved = true;
                    }
                    if self.kills > self.save.best_kills {
                        self.save.best_kills = self.kills;
                    }
                    self.screen = Screen::Over;
                    self.boost_held = false;
                    return StepEvent::PlayerDied;
                } else {
                    self.snakes[s].respawn_ticks = self.rand_range(45, 95) as u16;
                    if killer[s] == 0 {
                        self.kills = self.kills.saturating_add(1);
                        self.kill_flash_ticks = 64;
                        evt = StepEvent::KilledBot;
                    }
                }
            }
            s += 1;
        }

        // 7. Smooth camera tracking of player head
        if self.snakes[0].alive {
            // Track with slight smooth follow (factor 1/2 per tick -> ultra-responsive zero lag)
            self.cam_x += (self.snakes[0].hx - self.cam_x) >> 1;
            self.cam_y += (self.snakes[0].hy - self.cam_y) >> 1;

            let len = self.snakes[0].mass;
            if len > self.save.best_len {
                let was_new = self.new_best_achieved;
                self.save.best_len = len;
                self.new_best_achieved = true;
                if !was_new && evt != StepEvent::KilledBot {
                    evt = StepEvent::NewBest;
                }
            }
            let (rank, _) = self.player_rank();
            if rank < self.best_rank {
                self.best_rank = rank;
            }
        }

        // 8. Replenish ambient pellets smoothly
        if (self.active_orbs as usize) < AMBIENT_TARGET && (self.tick & 1) == 0 {
            self.spawn_ambient_orb();
        }

        evt
    }

    /// Burst a defeated snake into a rich glowing trail of high-value loot orbs along its body!
    fn burst_snake_into_loot(&mut self, idx: usize) {
        let segs = self.snakes[idx].seg_count();
        let mass = self.snakes[idx].mass;
        let skin = self.snakes[idx].skin;

        // Drop up to ~28 big glowing orbs distributed cleanly along the whole body
        let stride = (segs / 26).max(2);
        let orb_count = (segs / stride).max(4) as u32;
        let loot_val = ((mass * 3) / (orb_count * 4)).clamp(3, 12) as u8;

        let mut k = 0usize;
        while k < segs {
            let (sx, sy) = self.snakes[idx].seg_pos(k);
            let jx = self.rand_range(-9, 10);
            let jy = self.rand_range(-9, 10);
            let col = if skin == 6 {
                ((k / 2) & 7) as u8
            } else {
                skin
            };
            self.spawn_loot_orb(sx + jx, sy + jy, loot_val, col);
            k += stride;
        }
    }

    fn update_orbs_and_eating(&mut self) -> bool {
        let mut player_ate_big = false;
        let mut oi = 0usize;
        while oi < MAX_ORBS {
            let val = self.orbs[oi].val;
            if val == 0 {
                oi += 1;
                continue;
            }
            let mut ox = self.orbs[oi].x as i32;
            let mut oy = self.orbs[oi].y as i32;

            let mut eaten = false;
            let mut si = 0usize;
            while si < MAX_SNAKES {
                if !self.snakes[si].alive {
                    si += 1;
                    continue;
                }
                let hx = self.snakes[si].head_wx();
                let hy = self.snakes[si].head_wy();
                let pull_r = self.snakes[si].body_radius() + 24;
                let dx = hx - ox;
                let dy = hy - oy;
                // Quick Manhattan reject before squaring
                if dx.abs() <= pull_r && dy.abs() <= pull_r {
                    let d2 = dx * dx + dy * dy;
                    let eat_r = self.snakes[si].body_radius() + 10;
                    if d2 <= eat_r * eat_r {
                        self.snakes[si].mass = self.snakes[si].mass.saturating_add(val as u32);
                        self.orbs[oi].val = 0;
                        self.active_orbs = self.active_orbs.saturating_sub(1);
                        if si == 0 && val >= 3 {
                            player_ate_big = true;
                        }
                        eaten = true;
                        break;
                    } else if d2 <= pull_r * pull_r {
                        // Authentic Slither.io orb magnet pull into snake mouth!
                        ox += dx / 3;
                        oy += dy / 3;
                        self.orbs[oi].x = ox as i16;
                        self.orbs[oi].y = oy as i16;
                    }
                }
                si += 1;
            }

            if !eaten {
                self.orbs[oi].phase = self.orbs[oi].phase.wrapping_add(5);
            }
            oi += 1;
        }
        player_ate_big
    }

    /// Lightweight continuous wall safety guard run on off-ticks for bots.
    fn bot_wall_guard(&mut self, i: usize) {
        let hx = self.snakes[i].head_wx();
        let hy = self.snakes[i].head_wy();
        let d2 = (hx as i64) * (hx as i64) + (hy as i64) * (hy as i64);
        let hard = (ARENA_R - 190) as i64;
        if d2 > hard * hard {
            self.snakes[i].target_ang = angle(-hx, -hy);
            self.snakes[i].boosting = false;
            self.snakes[i].ai_boost_ticks = 0;
        }
    }

    /// Multi-priority smart AI brain for bot `i`:
    /// 1. Strong circular wall avoidance (zero stupid wall suicides!)
    /// 2. Multi-zone forward cone danger avoidance + evasive boost
    /// 3. Opportunistic parallel cut-off hunting
    /// 4. High-value death-loot scavenging & pellet foraging
    fn think_bot(&mut self, i: usize) {
        let hx = self.snakes[i].head_wx();
        let hy = self.snakes[i].head_wy();
        let ang = self.snakes[i].ang;
        let mass = self.snakes[i].mass;
        let my_r = self.snakes[i].body_radius();
        let style = self.snakes[i].style;

        if self.snakes[i].ai_hunt_cd > 0 {
            self.snakes[i].ai_hunt_cd -= 1;
        }
        if self.snakes[i].ai_boost_ticks > 0 {
            self.snakes[i].ai_boost_ticks -= 1;
        }
        self.snakes[i].ai_wander_phase = self.snakes[i].ai_wander_phase.wrapping_add(190);

        let dist = isqrt(((hx as i64) * (hx as i64) + (hy as i64) * (hy as i64)).min(i32::MAX as i64) as i32);
        let inward_ang = angle(-hx, -hy);

        // Priority 1: Hard Arena Wall Avoidance
        if dist > ARENA_R - 200 {
            self.snakes[i].target_ang = inward_ang;
            self.snakes[i].boosting = false;
            self.snakes[i].ai_boost_ticks = 0;
            return;
        }

        // Priority 2: Cone / Ray Obstacle Avoidance against ALL other snakes
        let look = if self.snakes[i].boosting { 195 } else { 148 } + my_r * 3;
        let look2 = look * look;
        let mut danger_left: i32 = 0;
        let mut danger_center: i32 = 0;
        let mut danger_right: i32 = 0;
        let mut min_threat_d: i32 = look + 1;

        let mut j = 0usize;
        while j < MAX_SNAKES {
            if j == i || !self.snakes[j].alive {
                j += 1;
                continue;
            }
            // Fast AABB cull
            if hx < self.snakes[j].min_x as i32 - look
                || hx > self.snakes[j].max_x as i32 + look
                || hy < self.snakes[j].min_y as i32 - look
                || hy > self.snakes[j].max_y as i32 + look
            {
                j += 1;
                continue;
            }
            let segs = self.snakes[j].seg_count();
            let mut k = 0usize;
            while k < segs {
                let (sx, sy) = self.snakes[j].seg_pos(k);
                let dx = sx - hx;
                let dy = sy - hy;
                let d2 = dx * dx + dy * dy;
                if d2 < look2 {
                    let d = isqrt(d2).max(1);
                    let rel = ang_diff(angle(dx, dy), ang);
                    let abs_rel = rel.abs();
                    if abs_rel < 20500 || d < my_r + 22 {
                        let closeness = (look - d).max(1);
                        let w = (closeness * closeness) >> 4;
                        // Extra threat weight on enemy heads (could turn into us!)
                        let w = if k == 0 { w + (w >> 1) } else { w };
                        if abs_rel < 6200 {
                            danger_center += w;
                        }
                        if rel < -1800 && rel > -20500 {
                            danger_left += w;
                        } else if rel > 1800 && rel < 20500 {
                            danger_right += w;
                        }
                        if abs_rel < 12500 && d < min_threat_d {
                            min_threat_d = d;
                        }
                    }
                }
                k += 2;
            }
            j += 1;
        }

        if danger_center > 20 || danger_left > 140 || danger_right > 140 {
            // Turn decisively toward the clearer side!
            let turn_dir: i32 = if danger_left > danger_right + 25 {
                15500 // danger on left -> veer right
            } else if danger_right > danger_left + 25 {
                -15500 // danger on right -> veer left
            } else {
                // Tie-break toward arena center
                if ang_diff(inward_ang, ang) >= 0 { 15500 } else { -15500 }
            };
            let mut evade_ang = ang.wrapping_add(turn_dir as u16);

            // If near outer boundary, bias escape inward so dodging never pushes into wall
            if dist > ARENA_R - 300 && ang_diff(evade_ang, inward_ang).abs() > 19000 {
                evade_ang = inward_ang;
            }
            self.snakes[i].target_ang = evade_ang;

            // Evasive boost when tightly boxed in
            let want_evade_boost = min_threat_d < 72
                && mass > MIN_BOOST_MASS + 6
                && dist < ARENA_R - 260;
            self.snakes[i].boosting = want_evade_boost;
            if want_evade_boost {
                self.snakes[i].ai_boost_ticks = 8;
            }
            return;
        }

        // Soft boundary curve (smooth arc back toward center well before hard margin)
        if dist > ARENA_R - 290 {
            let diff_in = ang_diff(inward_ang, ang);
            if diff_in.abs() > 9000 {
                self.snakes[i].target_ang = inward_ang;
                self.snakes[i].boosting = false;
                return;
            }
        }

        // Priority 3: Opportunistic Cut-Off Hunting (Hunters & big confident snakes)
        let aggressive = style == BotStyle::Hunter || (style == BotStyle::Scavenger && mass > 90);
        if aggressive && mass > 30 && self.snakes[i].ai_hunt_cd == 0 && dist < ARENA_R - 340 {
            let mut best_target: Option<(i32, i32, i32)> = None;
            let mut best_d2 = 225 * 225;
            let mut t = 0usize;
            while t < MAX_SNAKES {
                if t != i && self.snakes[t].alive {
                    let tx = self.snakes[t].head_wx();
                    let ty = self.snakes[t].head_wy();
                    let dx = tx - hx;
                    let dy = ty - hy;
                    let d2 = dx * dx + dy * dy;
                    if d2 > 55 * 55 && d2 < best_d2 {
                        let rel = ang_diff(angle(dx, dy), ang).abs();
                        // Only cut off targets generally ahead/beside us
                        if rel < 15000 {
                            best_d2 = d2;
                            let tang = self.snakes[t].ang;
                            // Aim 72 units ahead of victim's nose to force a cut-off!
                            let ix = tx + ((cos(tang) * 72) >> 14);
                            let iy = ty + ((sin(tang) * 72) >> 14);
                            best_target = Some((ix, iy, d2));
                        }
                    }
                }
                t += 1;
            }
            if let Some((ix, iy, _)) = best_target {
                let id2 = (ix as i64) * (ix as i64) + (iy as i64) * (iy as i64);
                let safe_r = (ARENA_R - 240) as i64;
                if id2 < safe_r * safe_r {
                    self.snakes[i].target_ang = angle(ix - hx, iy - hy);
                    self.snakes[i].boosting = true;
                    self.snakes[i].ai_boost_ticks = 16;
                    self.snakes[i].ai_hunt_cd = 75;
                    return;
                }
            }
        }

        // Priority 4: Smart Scavenging & Orb Foraging
        let mut best_score: i32 = 0;
        let mut best_orb_ang: u16 = ang;
        let mut best_orb_val: u8 = 0;
        let mut best_orb_dist: i32 = 999;
        let safe_orb_r2 = ((ARENA_R - 135) as i64) * ((ARENA_R - 135) as i64);

        let mut oi = 0usize;
        while oi < MAX_ORBS {
            let val = self.orbs[oi].val;
            if val == 0 {
                oi += 1;
                continue;
            }
            let ox = self.orbs[oi].x as i32;
            let oy = self.orbs[oi].y as i32;
            let dx = ox - hx;
            let dy = oy - hy;
            let md = dx.abs() + dy.abs();
            if md < 420 && md > 6 {
                let od2 = (ox as i64) * (ox as i64) + (oy as i64) * (oy as i64);
                if od2 < safe_orb_r2 {
                    let v = val as i32;
                    // Quadratic preference for death loot (val >= 3), extra for Scavenger!
                    let mut weight = v * v * 420;
                    if val >= 3 && style == BotStyle::Scavenger {
                        weight *= 2;
                    }
                    let o_ang = angle(dx, dy);
                    let rel = ang_diff(o_ang, ang).abs();
                    // Prefer orbs ahead of us so we sweep smoothly instead of spinning in place
                    if rel < 12000 {
                        weight = (weight * 5) / 4;
                    } else if rel > 22000 {
                        weight /= 2;
                    }
                    let score = weight / (md + 22);
                    if score > best_score {
                        best_score = score;
                        best_orb_ang = o_ang;
                        best_orb_val = val;
                        best_orb_dist = md;
                    }
                }
            }
            oi += 1;
        }

        if best_score > 0 {
            self.snakes[i].target_ang = best_orb_ang;
            // Rush high-value death-loot trails with boost just like real Slither.io players!
            let rush_loot = best_orb_val >= 3
                && best_orb_dist > 45
                && best_orb_dist < 260
                && mass > MIN_BOOST_MASS + 5
                && dist < ARENA_R - 280;
            self.snakes[i].boosting = rush_loot || self.snakes[i].ai_boost_ticks > 0;
        } else {
            // Smooth natural roaming biased gently toward the central arena
            let sway = (sin(self.snakes[i].ai_wander_phase) * 3200) >> 14;
            let base = if dist > 750 { inward_ang } else { ang };
            self.snakes[i].target_ang = base.wrapping_add(sway as u16);
            self.snakes[i].boosting = self.snakes[i].ai_boost_ticks > 0 && mass > MIN_BOOST_MASS + 6;
        }
    }
}

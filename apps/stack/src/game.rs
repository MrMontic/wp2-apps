//! Pure fixed-point physics and state for STACK (monochrome tower builder for CMF Watch Pro 2).
//! Runs at ~55 FPS (18 ms ticks) with zero floating-point math and zero heap allocations.

pub const SIZE: i32 = 466;
pub const C: i32 = SIZE / 2;

/// Fixed-point 1.0 px (8 fractional bits).
pub const Q: i32 = 256;

/// Geometry (px) carefully framed inside the 466x466 round watch bezel.
pub const START_W: i32 = 196;
pub const MIN_W: i32 = 8;
pub const BLOCK_H: i32 = 22;
/// Top edge of the base pedestal (level 0 rests directly on top of BASE_Y).
pub const BASE_Y: i32 = 380;
/// Active sliding block stops climbing above this Y; beyond this, the camera scrolls smoothly down.
pub const TARGET_TOP_Y: i32 = 182;
/// Horizontal travel arena for the sliding slab.
pub const ARENA_LEFT: i32 = 42;
pub const ARENA_RIGHT: i32 = SIZE - 42;

/// Tolerance (px) for a PERFECT snap flush with the block below.
pub const PERFECT_TOL: i32 = 4;
/// Streak needed before PERFECT hits start expanding the slab slightly.
pub const GROW_STREAK: u32 = 3;
pub const GROW_PX: i32 = 8;

/// Sliding speed per 18 ms tick (in Q units).
pub const SPEED0: i32 = 2 * Q + 140; // ~2.55 px/tick (~142 px/s)
pub const SPEED_STEP: i32 = 22; // gentle ramp per level
pub const SPEED_MAX: i32 = 6 * Q + 128; // ~6.5 px/tick (~360 px/s)

/// Ring buffer of visible stacked blocks (we only ever display up to ~11 layers on screen).
pub const HIST_CAP: usize = 16;

/// Smooth camera scroll speed (px per 18 ms tick).
pub const CAM_STEP: i32 = 5;

/// Impact flash duration on PERFECT (frames).
pub const FLASH_FRAMES: u8 = 7;
/// Banner ("PERFECT" / "x3") display duration (frames).
pub const BANNER_FRAMES: u8 = 28;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum State {
    Ready,
    Playing,
    Over,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DropResult {
    Placed,
    Perfect { streak: u32, grew: bool, new_best: bool },
    NewBest,
    Missed,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Block {
    pub x: i32,
    pub w: i32,
    /// Absolute stack level (0 = first placed block on base, used for monochrome shading wave).
    pub level: u32,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Debris {
    pub x: i32,
    pub y_q: i32,
    pub vy_q: i32,
    pub w: i32,
    pub level: u32,
    pub life: u8,
}

impl Debris {
    #[inline]
    pub fn y(&self) -> i32 {
        (self.y_q + Q / 2) >> 8
    }
}

#[derive(Clone)]
pub struct Game {
    pub state: State,
    /// Circular history of stacked blocks (index 0 is the foundation block at level 0).
    history: [Block; HIST_CAP],
    /// Total blocks in stack including foundation (starts at 1: foundation).
    pub count: u32,
    /// Current score (number of player-placed blocks = count - 1).
    pub score: u32,
    pub streak: u32,
    pub beat_best: bool,
    run_best_start: u32,

    /// Sliding active slab position in Q (left edge).
    pub cur_x_q: i32,
    pub cur_w: i32,
    pub cur_vx_q: i32,

    /// Smooth vertical camera offset in px (0 while tower is short, increases as tower grows).
    pub cam_y: i32,
    pub cam_target_y: i32,

    /// Last sliced overhanging scrap falling off-screen.
    pub debris: Option<Debris>,

    /// Visual feedback timers.
    pub flash: u8,
    pub banner_timer: u8,
    pub banner_streak: u32,
    pub banner_grew: bool,
}

impl Game {
    pub fn new() -> Self {
        let mut g = Game {
            state: State::Ready,
            history: [Block::default(); HIST_CAP],
            count: 1,
            score: 0,
            streak: 0,
            beat_best: false,
            run_best_start: 0,
            cur_x_q: (C - START_W / 2) * Q,
            cur_w: START_W,
            cur_vx_q: SPEED0,
            cam_y: 0,
            cam_target_y: 0,
            debris: None,
            flash: 0,
            banner_timer: 0,
            banner_streak: 0,
            banner_grew: false,
        };
        g.reset();
        g
    }

    pub fn reset(&mut self) {
        self.state = State::Ready;
        self.history = [Block::default(); HIST_CAP];
        self.history[0] = Block {
            x: C - START_W / 2,
            w: START_W,
            level: 0,
        };
        self.count = 1;
        self.score = 0;
        self.streak = 0;
        self.beat_best = false;
        self.cam_y = 0;
        self.cam_target_y = 0;
        self.debris = None;
        self.flash = 0;
        self.banner_timer = 0;
        self.banner_streak = 0;
        self.banner_grew = false;
        self.spawn_next();
    }

    pub fn start(&mut self, current_best: u32) {
        if self.state == State::Over {
            self.reset();
        }
        self.run_best_start = current_best;
        self.beat_best = false;
        self.state = State::Playing;
    }

    #[inline]
    pub fn top_block(&self) -> Block {
        self.history[((self.count - 1) as usize) % HIST_CAP]
    }

    /// Returns stored block at absolute index `idx` (if still in the ring buffer).
    #[inline]
    pub fn block_at(&self, idx: u32) -> Option<Block> {
        if idx >= self.count || self.count - idx > HIST_CAP as u32 {
            return None;
        }
        Some(self.history[(idx as usize) % HIST_CAP])
    }

    /// Screen Y (top edge) for block at absolute index `idx` (0 = foundation resting at BASE_Y - BLOCK_H).
    #[inline]
    pub fn block_y(&self, idx: u32) -> i32 {
        BASE_Y - ((idx as i32 + 1) * BLOCK_H) + self.cam_y
    }

    /// Screen Y (top edge) of the currently gliding slab.
    #[inline]
    pub fn active_y(&self) -> i32 {
        self.block_y(self.count)
    }

    /// Integer screen X (left edge) of the currently gliding slab.
    #[inline]
    pub fn active_x(&self) -> i32 {
        (self.cur_x_q + Q / 2) >> 8
    }

    #[inline]
    fn speed_for_score(score: u32) -> i32 {
        (SPEED0 + SPEED_STEP * score.min(200) as i32).min(SPEED_MAX)
    }

    fn spawn_next(&mut self) {
        let top = self.top_block();
        self.cur_w = top.w;
        let speed = Self::speed_for_score(self.score);
        // Alternate entry side left/right on each level so rhythm feels balanced and crisp.
        let from_left = self.count & 1 == 1;
        let left_bound = ARENA_LEFT.min(top.x - 16).max(20);
        let right_bound = (ARENA_RIGHT - self.cur_w).max(top.x + 16).min(SIZE - 20 - self.cur_w);
        if from_left {
            self.cur_x_q = left_bound * Q;
            self.cur_vx_q = speed;
        } else {
            self.cur_x_q = right_bound * Q;
            self.cur_vx_q = -speed;
        }
        // Compute camera target so active slab never climbs above TARGET_TOP_Y.
        let natural_y = BASE_Y - ((self.count as i32 + 1) * BLOCK_H);
        self.cam_target_y = (TARGET_TOP_Y - natural_y).max(0);
    }

    /// Advance animation by one 18 ms tick (sliding slab, smooth camera, falling scrap, timers).
    pub fn step(&mut self) {
        if self.flash > 0 {
            self.flash -= 1;
        }
        if self.banner_timer > 0 {
            self.banner_timer -= 1;
        }

        // Smooth camera scroll toward target
        if self.cam_y < self.cam_target_y {
            self.cam_y = (self.cam_y + CAM_STEP).min(self.cam_target_y);
        }

        // Update falling sliced scrap
        if let Some(mut d) = self.debris {
            d.vy_q += 110; // smooth gravity
            d.y_q += d.vy_q;
            if d.life <= 1 || d.y() > SIZE + 4 {
                self.debris = None;
            } else {
                d.life -= 1;
                self.debris = Some(d);
            }
        }

        if self.state != State::Playing {
            return;
        }

        // Move sliding slab cleanly between arena walls
        let top = self.top_block();
        let min_x = ARENA_LEFT.min(top.x - 16).max(20) * Q;
        let max_x = (ARENA_RIGHT - self.cur_w).max(top.x + 16).min(SIZE - 20 - self.cur_w) * Q;

        self.cur_x_q += self.cur_vx_q;
        if self.cur_x_q <= min_x {
            self.cur_x_q = min_x;
            self.cur_vx_q = self.cur_vx_q.abs();
        } else if self.cur_x_q >= max_x {
            self.cur_x_q = max_x;
            self.cur_vx_q = -self.cur_vx_q.abs();
        }
    }

    /// Player tapped to drop/lock the active slab onto the tower.
    pub fn drop_slab(&mut self) -> DropResult {
        if self.state != State::Playing {
            return DropResult::Missed;
        }
        let top = self.top_block();
        let x = self.active_x();
        let w = self.cur_w;
        let level = self.count;
        let active_y = self.active_y();

        let dx = x - top.x;

        // 1. Check PERFECT snap
        if dx.abs() <= PERFECT_TOL {
            self.streak += 1;
            let mut new_x = top.x;
            let mut new_w = top.w;
            let mut grew = false;
            if self.streak >= GROW_STREAK && new_w < START_W {
                let target_w = (new_w + GROW_PX).min(START_W);
                let delta = target_w - new_w;
                new_x = (new_x - delta / 2).clamp(ARENA_LEFT, ARENA_RIGHT - target_w);
                new_w = target_w;
                grew = true;
            }

            self.push_block(Block { x: new_x, w: new_w, level });
            self.flash = FLASH_FRAMES;
            self.banner_timer = BANNER_FRAMES;
            self.banner_streak = self.streak;
            self.banner_grew = grew;
            let new_best = self.check_new_best();
            self.spawn_next();
            return DropResult::Perfect {
                streak: self.streak,
                grew,
                new_best,
            };
        }

        // 2. Compute overlap with top block
        let left = x.max(top.x);
        let right = (x + w).min(top.x + top.w);
        let overlap = right - left;

        if overlap < MIN_W {
            // Complete miss (or sliver too thin to stand): whole slab falls off!
            self.debris = Some(Debris {
                x,
                y_q: active_y * Q,
                vy_q: 2 * Q,
                w,
                level,
                life: 24,
            });
            self.streak = 0;
            self.state = State::Over;
            return DropResult::Missed;
        }

        // 3. Slice overhanging piece into falling debris
        let (cut_x, cut_w) = if x < top.x {
            (x, top.x - x)
        } else {
            (top.x + top.w, (x + w) - (top.x + top.w))
        };
        if cut_w > 0 {
            self.debris = Some(Debris {
                x: cut_x,
                y_q: active_y * Q,
                vy_q: Q,
                w: cut_w,
                level,
                life: 22,
            });
        }

        self.streak = 0;
        self.banner_timer = 0;
        self.push_block(Block {
            x: left,
            w: overlap,
            level,
        });
        let new_best = self.check_new_best();
        self.spawn_next();
        if new_best {
            DropResult::NewBest
        } else {
            DropResult::Placed
        }
    }

    fn push_block(&mut self, b: Block) {
        self.history[(self.count as usize) % HIST_CAP] = b;
        self.count += 1;
        self.score += 1;
    }

    fn check_new_best(&mut self) -> bool {
        if self.run_best_start > 0 && !self.beat_best && self.score > self.run_best_start {
            self.beat_best = true;
            true
        } else {
            false
        }
    }
}

/// Persistent high score blob: `STAK` + u32 LE.
const MAGIC: &[u8; 4] = b"STAK";

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
        assert_eq!(decode_best(&encode_best(37)), 37);
        assert_eq!(decode_best(b"HALO1234"), 0);
        assert_eq!(decode_best(&[]), 0);
    }

    #[test]
    fn perfect_streak_grows_after_trim() {
        let mut g = Game::new();
        g.start(0);
        // Slightly off drop trims the block
        g.cur_x_q = (g.top_block().x + 20) * Q;
        assert_eq!(g.drop_slab(), DropResult::Placed);
        let trimmed_w = g.top_block().w;
        assert_eq!(trimmed_w, START_W - 20);
        assert!(g.debris.is_some());

        // 3 perfect drops in a row should grow it back by GROW_PX
        for i in 1..=3 {
            g.cur_x_q = (g.top_block().x + 2) * Q; // within PERFECT_TOL
            let res = g.drop_slab();
            assert_eq!(
                res,
                DropResult::Perfect {
                    streak: i,
                    grew: i == 3,
                    new_best: false
                }
            );
        }
        assert_eq!(g.top_block().w, trimmed_w + GROW_PX);
    }

    #[test]
    fn complete_miss_ends_game() {
        let mut g = Game::new();
        g.start(5);
        g.cur_x_q = 10 * Q;
        g.cur_w = 20;
        assert_eq!(g.drop_slab(), DropResult::Missed);
        assert_eq!(g.state, State::Over);
    }
}

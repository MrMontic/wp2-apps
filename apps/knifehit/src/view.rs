//! Zero-flicker differential renderer for Knife Hit on CMF Watch Pro 2 (466×466 round AMOLED).
//!
//! Renders:
//!   - Warm oak Wooden Log target (concentric growth rings, bark rim, rotating grain, impact flash)
//!   - All 8 distinct visual Bosses (Cheese, Tomato, Tire, Watermelon, Pizza, Shield, Saw, CMF Core)
//!   - All 16 custom-rendered unlockable knives (upright, rotated, mini queue icons, 2× Shop showcase)
//!   - Glossy apples on rim + sliced flying halves, wood splinters, metallic sparks, shatter chunks
//!   - Crisp differential updates keeping peak gameplay frame rects well below 850 rects/frame!

use crate::game::{
    BossKind, CX, Game, MAX_APPLES, MAX_CHUNKS, MAX_FLY_KNIVES, MAX_PARTICLES, MAX_STUCK,
    READY_TIP_Y, SIZE, State, TARGET_R,
};
use crate::knives::{self, KNIVES, NUM_KNIVES, polar};
use cmf_wp2_ui::Surface;
use cmf_wp2_ui::color::rgb565;
use cmf_wp2_ui::motion::isqrt;
use cmf_wp2_ui::text::{big, big_width, centre, int};

// Core aesthetic palette (deep midnight-slate with crisp CMF orange & warm wood accents)
pub const BG: u16 = rgb565(14, 20, 30);
const HDR_BG: u16 = rgb565(19, 27, 40);
const HDR_LINE: u16 = rgb565(34, 48, 68);
const CARD_BG: u16 = rgb565(22, 31, 46);
const CARD_EDGE: u16 = rgb565(44, 62, 88);
const WHITE: u16 = rgb565(248, 250, 254);
const MUTED: u16 = rgb565(132, 148, 172);
const DIM: u16 = rgb565(52, 66, 86);
const CMF_ORANGE: u16 = rgb565(255, 104, 24);
const CMF_ORANGE_DK: u16 = rgb565(186, 66, 12);
const GOLD: u16 = rgb565(250, 204, 52);
const GOLD_DK: u16 = rgb565(172, 124, 18);
const GREEN: u16 = rgb565(56, 204, 104);
const GREEN_DK: u16 = rgb565(28, 124, 62);
const RED: u16 = rgb565(234, 48, 54);
const RED_DK: u16 = rgb565(148, 22, 30);
const APPLE_LEAF: u16 = rgb565(68, 206, 74);

const PLAY_TOP: i32 = 50;
const BAND_H: i32 = 3;

/// Button hit regions shared between renderer and input handler.
pub const BTN_PLAY: (i32, i32, i32, i32) = (123, 310, 220, 48);
pub const BTN_KNIVES_TITLE: (i32, i32, i32, i32) = (133, 370, 200, 42);
pub const BTN_RETRY: (i32, i32, i32, i32) = (123, 304, 220, 48);
pub const BTN_KNIVES_OVER: (i32, i32, i32, i32) = (133, 364, 200, 42);
pub const BTN_SHOP_PREV: (i32, i32, i32, i32) = (32, 140, 78, 96);
pub const BTN_SHOP_NEXT: (i32, i32, i32, i32) = (356, 140, 78, 96);
pub const BTN_SHOP_ACT: (i32, i32, i32, i32) = (103, 310, 260, 48);
pub const BTN_SHOP_BACK: (i32, i32, i32, i32) = (163, 372, 140, 42);

#[inline]
pub fn hit_rect(r: (i32, i32, i32, i32), x: i32, y: i32) -> bool {
    x >= r.0 && x < r.0 + r.2 && y >= r.1 && y < r.1 + r.3
}

/// Compact previous-frame snapshot for differential rendering (stored in `static mut` on device).
pub struct ViewState {
    pub valid: bool,
    pub state: State,
    pub stage: u16,
    pub score: u32,
    pub apples_total: u32,
    pub knives_left: u8,
    pub knives_total: u8,
    pub selected_knife: u8,
    pub browse_knife: u8,
    pub unlocked_mask: u32,
    pub target_cy: i16,
    pub target_r: u8,
    pub target_angle: u16,
    pub boss: BossKind,

    pub stuck_boxes: [(i16, i16, u8, u8); MAX_STUCK * 2],
    pub stuck_box_count: u8,
    pub apple_pos: [(i16, i16); MAX_APPLES],
    pub apple_count: u8,
    pub rim_extras: [(i16, i16); 12], // tire treads / saw teeth
    pub rim_extra_count: u8,
    pub had_flying: bool,
    pub old_fly_y: i16,
    pub had_ready: bool,
    pub part_boxes: [(i16, i16, u8, u8); MAX_PARTICLES],
    pub part_count: u8,
    pub chunk_boxes: [(i16, i16, u8, u8); MAX_CHUNKS],
    pub chunk_count: u8,
    pub loose_boxes: [(i16, i16, u8, u8); MAX_FLY_KNIVES * 2],
    pub loose_box_count: u8,
    pub bounce_boxes: [(i16, i16, u8, u8); 2],
    pub had_bounce: bool,
}

impl ViewState {
    pub const fn new() -> Self {
        Self {
            valid: false,
            state: State::Title,
            stage: 0,
            score: 0,
            apples_total: 0,
            knives_left: 0,
            knives_total: 0,
            selected_knife: 0,
            browse_knife: 0,
            unlocked_mask: 0,
            target_cy: 0,
            target_r: 0,
            target_angle: 0,
            boss: BossKind::None,
            stuck_boxes: [(0, 0, 0, 0); MAX_STUCK * 2],
            stuck_box_count: 0,
            apple_pos: [(0, 0); MAX_APPLES],
            apple_count: 0,
            rim_extras: [(0, 0); 12],
            rim_extra_count: 0,
            had_flying: false,
            old_fly_y: 0,
            had_ready: false,
            part_boxes: [(0, 0, 0, 0); MAX_PARTICLES],
            part_count: 0,
            chunk_boxes: [(0, 0, 0, 0); MAX_CHUNKS],
            chunk_count: 0,
            loose_boxes: [(0, 0, 0, 0); MAX_FLY_KNIVES * 2],
            loose_box_count: 0,
            bounce_boxes: [(0, 0, 0, 0); 2],
            had_bounce: false,
        }
    }

    pub fn invalidate(&mut self) {
        self.valid = false;
    }
}

#[inline]
fn pack_box(b: (i32, i32, i32, i32)) -> (i16, i16, u8, u8) {
    (
        b.0.clamp(-200, 600) as i16,
        b.1.clamp(-200, 600) as i16,
        b.2.clamp(0, 255) as u8,
        b.3.clamp(0, 255) as u8,
    )
}

#[inline]
fn erase_play(s: &mut impl Surface, x: i32, y: i32, w: i32, h: i32, min_y: i32) {
    let x0 = x.max(0);
    let y0 = y.max(min_y);
    let x1 = (x + w).min(SIZE);
    let y1 = (y + h).min(SIZE);
    if x0 < x1 && y0 < y1 {
        s.rect(x0, y0, x1 - x0, y1 - y0, BG);
    }
}

/// Fast scanline-band filled circle (`BAND_H`-px bands). Smooth & low rect count!
fn fill_disc(s: &mut impl Surface, cx: i32, cy: i32, r: i32, col: u16, band_h: i32, min_y: i32) {
    let rr = r * r;
    let mut dy = -r;
    while dy < r {
        let h = band_h.min(r - dy);
        let mid = dy + h / 2;
        let m = rr - mid * mid;
        if m > 0 {
            let dx = isqrt(m);
            let y0 = (cy + dy).max(min_y);
            let y1 = (cy + dy + h).min(SIZE);
            if dx > 0 && y0 < y1 {
                s.rect(cx - dx, y0, dx * 2, y1 - y0, col);
            }
        }
        dy += h;
    }
}

/// Crisp rounded pill button with subtle border and centered label.
fn draw_pill(
    s: &mut impl Surface,
    r: (i32, i32, i32, i32),
    fill: u16,
    border: u16,
    label: &str,
    text_col: u16,
) {
    let (x, y, w, h) = r;
    s.rect(x + 8, y, w - 16, h, border);
    s.rect(x + 3, y + 3, w - 6, h - 6, border);
    s.rect(x, y + 8, w, h - 16, border);

    s.rect(x + 9, y + 2, w - 18, h - 4, fill);
    s.rect(x + 4, y + 5, w - 8, h - 10, fill);
    s.rect(x + 2, y + 9, w - 4, h - 18, fill);
    // Subtle top glossy highlight
    s.rect(x + 12, y + 3, w - 24, 2, cmf_wp2_ui::color::blend565(fill, WHITE, 240));
    centre(s, x + w / 2, y + (h - 16) / 2, label, text_col);
}

/// Glossy mini apple icon centered at `(cx, cy)` (8 rects).
pub fn draw_apple(s: &mut impl Surface, cx: i32, cy: i32, min_y: i32) {
    let clip = |s: &mut dyn Surface, x: i32, y: i32, w: i32, h: i32, c: u16| {
        let y0 = y.max(min_y);
        let y1 = (y + h).min(SIZE);
        if w > 0 && y0 < y1 {
            s.rect(x, y0, w, y1 - y0, c);
        }
    };
    // Stem & green leaf
    clip(s, cx - 1, cy - 10, 2, 4, rgb565(120, 74, 38));
    clip(s, cx + 1, cy - 10, 5, 3, APPLE_LEAF);
    // Plump red apple body
    clip(s, cx - 5, cy - 6, 10, 2, RED);
    clip(s, cx - 7, cy - 4, 14, 8, RED);
    clip(s, cx - 5, cy + 4, 10, 3, RED_DK);
    // Glossy specular highlight + subtle shadow
    clip(s, cx - 4, cy - 3, 3, 3, WHITE);
    clip(s, cx + 3, cy + 1, 3, 3, RED_DK);
}

/// Draws the rotating target disc (Warm Oak Log OR one of the 8 distinct Bosses!).
/// Records any outer protrusions (tire treads / saw teeth) into `rim_extras` for clean erasure.
fn draw_target(
    s: &mut impl Surface,
    cx: i32,
    cy: i32,
    r: i32,
    ang: u16,
    boss: BossKind,
    hit_flash: u8,
    rim_extras: &mut [(i16, i16); 12],
    rim_extra_count: &mut u8,
    min_y: i32,
) {
    *rim_extra_count = 0;
    let flash_hi = hit_flash >= 2;
    let dot = |s: &mut dyn Surface, px: i32, py: i32, rad: i32, c: u16| {
        let sz = rad * 2 + 1;
        let y0 = (py - rad).max(min_y);
        let y1 = (py - rad + sz).min(SIZE);
        if sz > 0 && y0 < y1 {
            s.rect(px - rad, y0, sz, y1 - y0, c);
        }
    };

    match boss {
        BossKind::None => {
            // Classic Knife Hit Warm Oak Wooden Log!
            let bark = if flash_hi { rgb565(184, 140, 102) } else { rgb565(92, 52, 26) };
            let wood = if flash_hi { rgb565(250, 224, 184) } else { rgb565(214, 164, 102) };
            let ring_col = if flash_hi { rgb565(234, 198, 148) } else { rgb565(176, 122, 66) };
            let inner = if flash_hi { rgb565(248, 222, 182) } else { rgb565(224, 178, 118) };
            let heart = rgb565(146, 94, 48);

            fill_disc(s, cx, cy, r, bark, BAND_H, min_y);
            fill_disc(s, cx, cy, r - 8, wood, BAND_H, min_y);
            if r >= 50 {
                fill_disc(s, cx, cy, r - 24, ring_col, 4, min_y);
                fill_disc(s, cx, cy, r - 29, inner, 4, min_y);
                fill_disc(s, cx, cy, r / 4, ring_col, 4, min_y);
            }
            fill_disc(s, cx, cy, 7, heart, 3, min_y);

            // Rotating wood grain cracks & bark notches so spin is super crisp & readable!
            let crack_col = rgb565(142, 92, 46);
            for i in 0..4 {
                let a = ang.wrapping_add((i as u16).wrapping_mul(16384));
                let (x1, y1) = polar(cx, cy, r * 38 / 100, a);
                let (x2, y2) = polar(cx, cy, r * 66 / 100, a);
                dot(s, x1, y1, 2, crack_col);
                dot(s, (x1 + x2) / 2, (y1 + y2) / 2, 2, crack_col);
                dot(s, x2, y2, 2, crack_col);
            }
            // Subtle bark knots
            let knot_col = rgb565(64, 34, 16);
            for i in 0..6 {
                let a = ang.wrapping_add((i as u16).wrapping_mul(10922));
                let (kx, ky) = polar(cx, cy, r - 4, a);
                dot(s, kx, ky, 2, knot_col);
            }
        }
        BossKind::Cheese => {
            // BOSS 1: THE CHEESE — Golden Swiss wheel with holes!
            let rind = if flash_hi { WHITE } else { rgb565(212, 146, 18) };
            let body = if flash_hi { rgb565(255, 242, 168) } else { rgb565(252, 208, 48) };
            let hole_rim = rgb565(198, 134, 16);
            let hole_in = rgb565(148, 92, 12);

            fill_disc(s, cx, cy, r, rind, BAND_H, min_y);
            fill_disc(s, cx, cy, r - 7, body, BAND_H, min_y);
            // 7 rotating Swiss cheese holes of varied radii
            const HOLES: [(u16, i32, i32); 7] = [
                (0, 38, 9),
                (9800, 26, 6),
                (19200, 44, 7),
                (29000, 22, 8),
                (39800, 42, 9),
                (50500, 32, 6),
                (58500, 12, 5),
            ];
            for &(off, dist, hr) in &HOLES {
                let (hx, hy) = polar(cx, cy, dist, ang.wrapping_add(off));
                dot(s, hx, hy, hr, hole_rim);
                dot(s, hx + 1, hy + 1, hr - 2, hole_in);
            }
        }
        BossKind::Tomato => {
            // BOSS 2: THE TOMATO — Glossy crimson + pale seeds + 5-petal green calyx!
            let skin = if flash_hi { rgb565(255, 168, 168) } else { rgb565(178, 22, 28) };
            let flesh = if flash_hi { rgb565(255, 122, 122) } else { rgb565(234, 44, 42) };
            let core = rgb565(248, 92, 84);
            let seed = rgb565(252, 226, 142);
            let leaf = rgb565(48, 178, 56);
            let leaf_dk = rgb565(26, 118, 36);

            fill_disc(s, cx, cy, r, skin, BAND_H, min_y);
            fill_disc(s, cx, cy, r - 7, flesh, BAND_H, min_y);
            fill_disc(s, cx, cy, r - 28, core, 4, min_y);
            // 6 rotating seed pockets
            for i in 0..6 {
                let a = ang.wrapping_add((i as u16).wrapping_mul(10922));
                let (px, py) = polar(cx, cy, 44, a);
                dot(s, px, py, 6, skin);
                dot(s, px, py, 3, seed);
            }
            // 5-petal green star stem in center
            for i in 0..5 {
                let a = ang.wrapping_add((i as u16).wrapping_mul(13107));
                let (lx, ly) = polar(cx, cy, 16, a);
                let (mx, my) = polar(cx, cy, 9, a);
                dot(s, mx, my, 4, leaf);
                dot(s, lx, ly, 3, leaf_dk);
            }
            dot(s, cx, cy, 5, leaf_dk);
        }
        BossKind::Tire => {
            // BOSS 3: THE TIRE — Rubber treads + silver alloy rim + 5 spokes!
            let rubber = if flash_hi { rgb565(118, 126, 138) } else { rgb565(34, 38, 46) };
            let tread = rgb565(56, 62, 74);
            let alloy = if flash_hi { WHITE } else { rgb565(196, 206, 220) };
            let brake = rgb565(22, 26, 34);

            // 12 outer rotating rubber tread blocks (recorded for exact differential erase!)
            for i in 0..12 {
                let a = ang.wrapping_add((i as u16).wrapping_mul(5461));
                let (tx, ty) = polar(cx, cy, r + 1, a);
                rim_extras[i] = (tx as i16, ty as i16);
                dot(s, tx, ty, 5, tread);
            }
            *rim_extra_count = 12;

            fill_disc(s, cx, cy, r, rubber, BAND_H, min_y);
            fill_disc(s, cx, cy, 45, alloy, 3, min_y);
            fill_disc(s, cx, cy, 37, brake, 4, min_y);
            // 5 rotating metallic alloy spokes
            for i in 0..5 {
                let a = ang.wrapping_add((i as u16).wrapping_mul(13107));
                for step in 1..=4 {
                    let (sx, sy) = polar(cx, cy, step * 9, a);
                    dot(s, sx, sy, 3, alloy);
                }
            }
            fill_disc(s, cx, cy, 12, alloy, 4, min_y);
            dot(s, cx, cy, 4, rubber);
        }
        BossKind::Watermelon => {
            // BOSS 4: THE WATERMELON — Green striped rind + crimson flesh + black seeds!
            let rind = if flash_hi { rgb565(126, 224, 136) } else { rgb565(34, 148, 56) };
            let stripe = rgb565(16, 86, 32);
            let pith = rgb565(214, 240, 198);
            let flesh = if flash_hi { rgb565(255, 136, 146) } else { rgb565(232, 48, 66) };
            let seed = rgb565(22, 22, 28);

            fill_disc(s, cx, cy, r, rind, BAND_H, min_y);
            for i in 0..8 {
                let a = ang.wrapping_add((i as u16).wrapping_mul(8192));
                let (sx, sy) = polar(cx, cy, r - 4, a);
                dot(s, sx, sy, 4, stripe);
            }
            fill_disc(s, cx, cy, r - 9, pith, BAND_H, min_y);
            fill_disc(s, cx, cy, r - 14, flesh, BAND_H, min_y);
            // 8 rotating black seeds
            for i in 0..8 {
                let a = ang.wrapping_add((i as u16).wrapping_mul(8192) + 4096);
                let (sx, sy) = polar(cx, cy, 35, a);
                dot(s, sx, sy, 3, seed);
            }
        }
        BossKind::Pizza => {
            // BOSS 5: THE PIZZA — Golden crust + red sauce + cheese + pepperoni + basil!
            let crust = if flash_hi { rgb565(255, 226, 152) } else { rgb565(210, 142, 54) };
            let sauce = rgb565(204, 42, 32);
            let cheese = if flash_hi { rgb565(255, 246, 178) } else { rgb565(252, 212, 74) };
            let pep = rgb565(188, 34, 32);
            let pep_hi = rgb565(224, 72, 62);
            let basil = rgb565(42, 166, 64);

            fill_disc(s, cx, cy, r, crust, BAND_H, min_y);
            fill_disc(s, cx, cy, r - 11, sauce, BAND_H, min_y);
            fill_disc(s, cx, cy, r - 16, cheese, BAND_H, min_y);
            // 6 rotating pepperoni slices
            for i in 0..6 {
                let a = ang.wrapping_add((i as u16).wrapping_mul(10922));
                let (px, py) = polar(cx, cy, 34, a);
                dot(s, px, py, 8, pep);
                dot(s, px - 2, py - 2, 2, pep_hi);
            }
            // Center pepperoni + 4 basil flecks
            dot(s, cx, cy, 7, pep);
            for i in 0..4 {
                let a = ang.wrapping_add((i as u16).wrapping_mul(16384) + 8192);
                let (bx, by) = polar(cx, cy, 20, a);
                dot(s, bx, by, 3, basil);
            }
        }
        BossKind::Shield => {
            // BOSS 6: THE SHIELD — Medieval forged steel rim + heraldic cross + brass rivets!
            let steel = if flash_hi { WHITE } else { rgb565(184, 196, 214) };
            let steel_dk = rgb565(104, 118, 138);
            let crest = if flash_hi { rgb565(96, 146, 224) } else { rgb565(34, 68, 138) };
            let brass = GOLD;

            fill_disc(s, cx, cy, r, steel, BAND_H, min_y);
            fill_disc(s, cx, cy, r - 10, steel_dk, BAND_H, min_y);
            fill_disc(s, cx, cy, r - 14, crest, BAND_H, min_y);
            // Rotating forged steel heraldic cross
            for arm in 0..4 {
                let a = ang.wrapping_add((arm as u16).wrapping_mul(16384));
                for step in 1..=5 {
                    let (px, py) = polar(cx, cy, step * 10, a);
                    dot(s, px, py, 5, steel);
                }
            }
            // 8 brass rivets on steel rim
            for i in 0..8 {
                let a = ang.wrapping_add((i as u16).wrapping_mul(8192));
                let (rx, ry) = polar(cx, cy, r - 5, a);
                dot(s, rx, ry, 2, brass);
            }
            fill_disc(s, cx, cy, 14, steel, 4, min_y);
            dot(s, cx, cy, 6, brass);
        }
        BossKind::Saw => {
            // BOSS 7: THE SAW — Metallic buzzsaw teeth + hazard red core!
            let blade = if flash_hi { WHITE } else { rgb565(198, 208, 222) };
            let blade_dk = rgb565(118, 130, 148);
            let danger = if flash_hi { rgb565(255, 138, 132) } else { RED };

            // 12 rotating sharp sawtooth spikes (recorded for exact differential erase!)
            for i in 0..12 {
                let a = ang.wrapping_add((i as u16).wrapping_mul(5461));
                let (tx, ty) = polar(cx, cy, r + 1, a);
                rim_extras[i] = (tx as i16, ty as i16);
                dot(s, tx, ty, 5, blade);
                let (tx2, ty2) = polar(cx, cy, r - 2, a.wrapping_add(1600));
                dot(s, tx2, ty2, 3, blade_dk);
            }
            *rim_extra_count = 12;

            fill_disc(s, cx, cy, r - 2, blade, BAND_H, min_y);
            fill_disc(s, cx, cy, 42, blade_dk, 4, min_y);
            fill_disc(s, cx, cy, 38, blade, 4, min_y);
            fill_disc(s, cx, cy, 24, danger, 4, min_y);
            // 4 rotating cooling slots
            for i in 0..4 {
                let a = ang.wrapping_add((i as u16).wrapping_mul(16384));
                let (sx, sy) = polar(cx, cy, 48, a);
                dot(s, sx, sy, 4, BG);
            }
            fill_disc(s, cx, cy, 9, rgb565(56, 64, 78), 3, min_y);
        }
        BossKind::CmfCore => {
            // BOSS 8: CMF CORE — Matte black Nothing/CMF disc + glowing orange dot-matrix ring!
            let shell = if flash_hi { rgb565(94, 100, 112) } else { rgb565(36, 40, 48) };
            let bezel = rgb565(66, 72, 84);
            let face = rgb565(20, 23, 28);
            let dot_on = if flash_hi { WHITE } else { CMF_ORANGE };
            let dot_dim = rgb565(96, 42, 16);

            fill_disc(s, cx, cy, r, shell, BAND_H, min_y);
            fill_disc(s, cx, cy, r - 6, bezel, BAND_H, min_y);
            fill_disc(s, cx, cy, r - 9, face, BAND_H, min_y);
            // 16 rotating dot-matrix LEDs around the disc
            for i in 0..16 {
                let a = ang.wrapping_add((i as u16).wrapping_mul(4096));
                let (dx, dy) = polar(cx, cy, 48, a);
                let col = if i % 4 == 0 { WHITE } else if i & 1 == 0 { dot_on } else { dot_dim };
                dot(s, dx, dy, 3, col);
            }
            // Iconic CMF Orange center tactile dial
            fill_disc(s, cx, cy, 22, CMF_ORANGE, 4, min_y);
            let (ix, iy) = polar(cx, cy, 13, ang);
            dot(s, ix, iy, 3, face);
            dot(s, cx, cy, 5, face);
        }
    }
}

/// Draws the top header bar: Stage/Boss title + 5 stage progress dots.
fn draw_top_header(s: &mut impl Surface, g: &Game) {
    s.rect(0, 0, SIZE, PLAY_TOP - 2, HDR_BG);
    s.rect(0, PLAY_TOP - 2, SIZE, 2, HDR_LINE);

    if g.boss != BossKind::None {
        centre(s, CX, 14, g.boss.name(), GOLD);
    } else {
        let mut buf = [0u8; 18];
        buf[..6].copy_from_slice(b"STAGE ");
        let mut d = [0u8; 11];
        let ds = int(g.stage as i32, &mut d).as_bytes();
        buf[6..6 + ds.len()].copy_from_slice(ds);
        if let Ok(lbl) = core::str::from_utf8(&buf[..6 + ds.len()]) {
            centre(s, CX, 14, lbl, WHITE);
        }
    }

    // 5 Stage progress indicators (4 regular dots + 1 Boss diamond!)
    let pos = ((g.stage.saturating_sub(1)) % 5) as usize;
    let start_x = CX - 44;
    let y = 36;
    for i in 0..5 {
        let x = start_x + (i as i32) * 22;
        if i == 4 {
            // Boss icon (diamond/crown)
            let col = if pos == 4 { GOLD } else { RED_DK };
            s.rect(x - 1, y - 3, 4, 10, col);
            s.rect(x - 4, y, 10, 4, col);
            if pos == 4 {
                s.rect(x, y + 1, 2, 2, WHITE);
            }
        } else {
            let col = if i < pos {
                CMF_ORANGE
            } else if i == pos {
                WHITE
            } else {
                DIM
            };
            s.rect(x - 3, y - 1, 6, 6, col);
        }
    }
}

/// Draws top-left Score counter (safely inside circular screen and outside knife orbit).
fn draw_score_hud(s: &mut impl Surface, score: u32) {
    // Erase score area cleanly
    s.rect(58, 84, 54, 42, BG);
    centre(s, 85, 86, "HIT", MUTED);
    let mut d = [0u8; 11];
    let ds = int(score.min(999_999) as i32, &mut d);
    centre(s, 85, 106, ds, WHITE);
}

/// Draws top-right Apple icon + total apple count.
fn draw_apple_hud(s: &mut impl Surface, apples: u32) {
    s.rect(354, 80, 56, 48, BG);
    draw_apple(s, 380, 92, PLAY_TOP);
    let mut d = [0u8; 11];
    let ds = int(apples.min(999_999) as i32, &mut d);
    centre(s, 380, 106, ds, GOLD);
}

/// Draws the remaining knives stack neatly curving along the left round bezel!
fn draw_knives_queue(s: &mut impl Surface, total: u8, left: u8, skin: u8) {
    // Clear queue strip (x = 40..90, y = 172..342)
    s.rect(40, 172, 50, 170, BG);
    let n = (total as usize).min(12);
    let base_y = 326;
    for i in 0..n {
        let y = base_y - (i as i32) * 13;
        // Gentle bezel-hugging x curve!
        let dy = (y - 233).abs();
        let curve_x = 44 + (dy * dy) / 520;
        let active = i < left as usize;
        knives::draw_mini_icon(s, curve_x, y, active, skin);
    }
}

fn draw_particles(
    s: &mut impl Surface,
    g: &Game,
    part_boxes: &mut [(i16, i16, u8, u8); MAX_PARTICLES],
    part_count: &mut u8,
) {
    *part_count = 0;
    for p in &g.particles {
        if !p.active {
            continue;
        }
        let x = p.x as i32;
        let y = p.y as i32;
        let sz = p.size as i32;
        if p.kind == 1 || p.kind == 2 {
            // Juicy sliced apple half!
            let b = (x - 6, y - 6, 13, 13);
            if p.kind == 1 {
                s.rect(x - 5, y - 5, 5, 10, RED);
                s.rect(x, y - 4, 4, 8, rgb565(252, 244, 206));
                s.rect(x + 1, y - 1, 2, 2, rgb565(56, 32, 18));
            } else {
                s.rect(x - 4, y - 4, 4, 8, rgb565(252, 244, 206));
                s.rect(x, y - 5, 5, 10, RED);
                s.rect(x - 3, y - 1, 2, 2, rgb565(56, 32, 18));
            }
            if (*part_count as usize) < MAX_PARTICLES {
                part_boxes[*part_count as usize] = pack_box(b);
                *part_count += 1;
            }
        } else {
            let rx = x - sz / 2;
            let ry = y - sz / 2;
            if ry >= PLAY_TOP && ry < SIZE {
                s.rect(rx, ry, sz, sz, p.color);
                if (*part_count as usize) < MAX_PARTICLES {
                    part_boxes[*part_count as usize] = pack_box((rx, ry, sz, sz));
                    *part_count += 1;
                }
            }
        }
    }
}

/// Draws Title screen (full paint).
fn draw_title_full(s: &mut impl Surface, g: &Game, v: &mut ViewState) {
    s.rect(0, 0, SIZE, SIZE, BG);

    // Bold "KNIFE HIT" logo at top
    let k_w = big_width("KNIFE", 3);
    let h_w = big_width("HIT", 3);
    let total_w = k_w + 14 + h_w;
    let x0 = CX - total_w / 2;
    big(s, x0, 44, "KNIFE", 3, WHITE);
    big(s, x0 + k_w + 14, 44, "HIT", 3, CMF_ORANGE);

    // Live spinning mini wood log preview at (CX, 166), r = 42
    draw_title_spinner(s, g, v);

    // Stats strip
    let mut buf = [0u8; 24];
    let mut d = [0u8; 11];
    if g.save.best_stage > 0 {
        buf[..5].copy_from_slice(b"BEST ");
        let ds = int(g.save.best_stage as i32, &mut d).as_bytes();
        buf[5..5 + ds.len()].copy_from_slice(ds);
        if let Ok(lbl) = core::str::from_utf8(&buf[..5 + ds.len()]) {
            centre(s, CX - 48, 272, lbl, MUTED);
        }
    } else {
        centre(s, CX - 48, 272, "STAGE 1", MUTED);
    }
    draw_apple(s, CX + 34, 280, 0);
    let app_s = int(g.save.apples.min(999_999) as i32, &mut d);
    s.text(CX + 46, 272, app_s, GOLD);

    // Primary PLAY button
    draw_pill(s, BTN_PLAY, CMF_ORANGE, CMF_ORANGE_DK, "PLAY", WHITE);

    // Secondary KNIVES (N/16) button
    let mut kb = [0u8; 16];
    kb[..7].copy_from_slice(b"KNIVES ");
    let mut nbuf = [0u8; 11];
    let ns = int(g.save.unlocked_count() as i32, &mut nbuf).as_bytes();
    let mut idx = 7;
    kb[idx..idx + ns.len()].copy_from_slice(ns);
    idx += ns.len();
    kb[idx..idx + 3].copy_from_slice(b"/16");
    idx += 3;
    let klbl = core::str::from_utf8(&kb[..idx]).unwrap_or("KNIVES");
    draw_pill(s, BTN_KNIVES_TITLE, CARD_BG, CMF_ORANGE, klbl, WHITE);
}

fn draw_title_spinner(s: &mut impl Surface, g: &Game, v: &mut ViewState) {
    let cy = 166;
    let r = 42;
    // Erase previous title spinner stuck knives & apple
    if v.valid && v.state == State::Title {
        for i in 0..v.stuck_box_count as usize {
            let b = v.stuck_boxes[i];
            erase_play(s, b.0 as i32, b.1 as i32, b.2 as i32, b.3 as i32, 74);
        }
        for i in 0..v.apple_count as usize {
            let (ax, ay) = v.apple_pos[i];
            erase_play(s, ax as i32 - 10, ay as i32 - 12, 20, 22, 74);
        }
    }

    // Draw 3 decorative stuck knives (shortened protrusion for mini target)
    v.stuck_box_count = 0;
    for k in &g.stuck {
        if !k.active {
            continue;
        }
        let wa = g.target_angle.wrapping_add(k.rel_angle);
        knives::draw_rotated(s, CX, cy, r - 12, wa, k.skin, 74);
        let boxes = knives::rotated_bboxes(CX, cy, r - 12, wa);
        for &b in &boxes {
            if (v.stuck_box_count as usize) < v.stuck_boxes.len() {
                v.stuck_boxes[v.stuck_box_count as usize] = pack_box(b);
                v.stuck_box_count += 1;
            }
        }
    }

    // Draw mini wooden log
    draw_target(
        s,
        CX,
        cy,
        r,
        g.target_angle,
        BossKind::None,
        0,
        &mut v.rim_extras,
        &mut v.rim_extra_count,
        74,
    );

    // Draw apple on rim
    v.apple_count = 0;
    for a in &g.apples {
        if !a.active {
            continue;
        }
        let wa = g.target_angle.wrapping_add(a.rel_angle);
        let (ax, ay) = polar(CX, cy, r + 10, wa);
        draw_apple(s, ax, ay, 74);
        if (v.apple_count as usize) < MAX_APPLES {
            v.apple_pos[v.apple_count as usize] = (ax as i16, ay as i16);
            v.apple_count += 1;
        }
    }
    v.target_cy = cy as i16;
    v.target_r = r as u8;
    v.target_angle = g.target_angle;
}

/// Draws the Knife Arsenal / Shop screen showcasing all 16 knives at big 2× scale!
fn draw_shop_full(s: &mut impl Surface, g: &Game) {
    s.rect(0, 0, SIZE, SIZE, BG);
    s.rect(0, 0, SIZE, 56, HDR_BG);
    s.rect(0, 56, SIZE, 2, HDR_LINE);

    // Header: ARSENAL + Apples
    centre(s, CX, 16, "ARSENAL", WHITE);
    let mut ibuf = [0u8; 16];
    let mut d = [0u8; 11];
    let cur = int(g.browse_knife as i32 + 1, &mut d).as_bytes();
    let mut len = cur.len();
    ibuf[..len].copy_from_slice(cur);
    ibuf[len..len + 3].copy_from_slice(b"/16");
    len += 3;
    if let Ok(lbl) = core::str::from_utf8(&ibuf[..len]) {
        centre(s, CX - 42, 36, lbl, MUTED);
    }
    draw_apple(s, CX + 22, 43, 0);
    let app_s = int(g.save.apples.min(999_999) as i32, &mut d);
    s.text(CX + 34, 36, app_s, GOLD);

    let id = g.browse_knife;
    let spec = &KNIVES[(id as usize).min(NUM_KNIVES - 1)];
    let unlocked = g.save.is_unlocked(id);
    let equipped = g.save.selected_knife == id;
    let affordable = g.save.can_afford(id);

    // Center showcase card
    let card_edge = if equipped {
        GREEN
    } else if unlocked {
        CMF_ORANGE
    } else if affordable {
        GOLD
    } else {
        CARD_EDGE
    };
    s.rect(128, 72, 210, 194, card_edge);
    s.rect(131, 75, 204, 188, CARD_BG);

    // Big 2× live upright preview of the knife!
    knives::draw_upright(s, CX, 92, id, 2, 75);

    // Left '<' and Right '>' navigation arrows
    draw_pill(s, BTN_SHOP_PREV, CARD_BG, CARD_EDGE, "<", WHITE);
    draw_pill(s, BTN_SHOP_NEXT, CARD_BG, CARD_EDGE, ">", WHITE);

    // Knife name
    centre(s, CX, 276, spec.name, if unlocked { WHITE } else { MUTED });

    // Status / Unlock / Equip pill button
    if equipped {
        draw_pill(s, BTN_SHOP_ACT, GREEN, GREEN_DK, "EQUIPPED", WHITE);
    } else if unlocked {
        draw_pill(s, BTN_SHOP_ACT, CMF_ORANGE, CMF_ORANGE_DK, "TAP TO EQUIP", WHITE);
    } else {
        let mut abuf = [0u8; 20];
        abuf[..7].copy_from_slice(b"UNLOCK ");
        let cs = int(spec.cost as i32, &mut d).as_bytes();
        let mut p = 7;
        abuf[p..p + cs.len()].copy_from_slice(cs);
        p += cs.len();
        abuf[p..p + 2].copy_from_slice(b" A");
        // Actually write "UNLOCK 15" cleanly!
        let lbl = core::str::from_utf8(&abuf[..7 + cs.len()]).unwrap_or("UNLOCK");
        if affordable {
            draw_pill(s, BTN_SHOP_ACT, GOLD, GOLD_DK, lbl, rgb565(20, 16, 8));
        } else {
            draw_pill(s, BTN_SHOP_ACT, CARD_BG, DIM, lbl, MUTED);
        }
        // Draw mini apple next to cost inside pill
        draw_apple(s, CX + 88, BTN_SHOP_ACT.1 + 24, 0);

        if spec.boss_unlock > 0 {
            let hint = if spec.boss_unlock == 1 { "OR BEAT BOSS 1" } else { "OR BEAT BOSS 2" };
            centre(s, CX, 294, hint, GOLD);
        }
    }

    // Bottom BACK button
    draw_pill(s, BTN_SHOP_BACK, CARD_BG, CARD_EDGE, "BACK", WHITE);
}

/// Draws Game Over screen.
fn draw_over_full(s: &mut impl Surface, g: &Game) {
    s.rect(0, 0, SIZE, SIZE, BG);
    s.rect(0, 0, SIZE, 58, HDR_BG);
    s.rect(0, 58, SIZE, 2, HDR_LINE);

    if g.beat_best {
        centre(s, CX, 22, "NEW BEST!", GOLD);
    } else {
        centre(s, CX, 22, "GAME OVER", RED);
    }

    // Score big display
    let mut d = [0u8; 11];
    let sc = int(g.score.min(999_999) as i32, &mut d);
    big(s, CX - big_width(sc, 6) / 2, 82, sc, 6, WHITE);
    centre(s, CX, 142, "KNIVES HIT", MUTED);

    // Stage & Best card
    s.rect(78, 174, 310, 108, CARD_EDGE);
    s.rect(80, 176, 306, 104, CARD_BG);

    let mut sb = [0u8; 20];
    sb[..6].copy_from_slice(b"STAGE ");
    let st = int(g.stage as i32, &mut d).as_bytes();
    sb[6..6 + st.len()].copy_from_slice(st);
    if let Ok(lbl) = core::str::from_utf8(&sb[..6 + st.len()]) {
        centre(s, CX, 190, lbl, CMF_ORANGE);
    }

    let mut bb = [0u8; 20];
    bb[..11].copy_from_slice(b"BEST STAGE ");
    let bs = int(g.save.best_stage as i32, &mut d).as_bytes();
    bb[11..11 + bs.len()].copy_from_slice(bs);
    if let Ok(lbl) = core::str::from_utf8(&bb[..11 + bs.len()]) {
        centre(s, CX, 218, lbl, WHITE);
    }

    // Apples summary
    draw_apple(s, CX - 26, 254, 0);
    let ap = int(g.save.apples.min(999_999) as i32, &mut d);
    s.text(CX - 12, 246, ap, GOLD);

    draw_pill(s, BTN_RETRY, CMF_ORANGE, CMF_ORANGE_DK, "RETRY", WHITE);
    draw_pill(s, BTN_KNIVES_OVER, CARD_BG, CMF_ORANGE, "KNIVES", WHITE);
}

fn draw_shatter_banner(s: &mut impl Surface, g: &Game) {
    if let Some(kid) = g.unlock_banner_knife {
        centre(s, CX, 168, "BOSS KNIFE UNLOCKED!", GOLD);
        centre(s, CX, 192, KNIVES[kid as usize].name, WHITE);
    } else if g.boss != BossKind::None {
        centre(s, CX, 176, "BOSS DEFEATED!", GOLD);
        centre(s, CX, 200, "+5 APPLES", CMF_ORANGE);
    } else {
        centre(s, CX, 186, "STAGE CLEAR!", WHITE);
    }
}

/// Draws active gameplay scene elements (stuck knives, target, apples, flying/ready knife, particles)
/// and records their exact bounding boxes into `v` for next frame's zero-flicker differential erase!
fn draw_play_entities(s: &mut impl Surface, g: &Game, v: &mut ViewState) {
    let cy = g.target_cy();

    // 1. Stuck knives protruding behind target rim (so blades embed seamlessly into wood/boss!)
    v.stuck_box_count = 0;
    for k in &g.stuck {
        if !k.active {
            continue;
        }
        let wa = g.target_angle.wrapping_add(k.rel_angle);
        knives::draw_rotated(s, CX, cy, TARGET_R - 4, wa, k.skin, PLAY_TOP);
        let boxes = knives::rotated_bboxes(CX, cy, TARGET_R - 4, wa);
        for &b in &boxes {
            if (v.stuck_box_count as usize) < v.stuck_boxes.len() {
                v.stuck_boxes[v.stuck_box_count as usize] = pack_box(b);
                v.stuck_box_count += 1;
            }
        }
    }

    // 2. Target disc (Wood Log or Boss!)
    draw_target(
        s,
        CX,
        cy,
        TARGET_R,
        g.target_angle,
        g.boss,
        g.hit_flash,
        &mut v.rim_extras,
        &mut v.rim_extra_count,
        PLAY_TOP,
    );

    // 3. Apples on rim
    v.apple_count = 0;
    for a in &g.apples {
        if !a.active {
            continue;
        }
        let wa = g.target_angle.wrapping_add(a.rel_angle);
        let (ax, ay) = polar(CX, cy, TARGET_R + 11, wa);
        draw_apple(s, ax, ay, PLAY_TOP);
        if (v.apple_count as usize) < MAX_APPLES {
            v.apple_pos[v.apple_count as usize] = (ax as i16, ay as i16);
            v.apple_count += 1;
        }
    }

    // 4. Flying knife OR ready waiting knife at bottom
    if g.state == State::Playing {
        if g.flying {
            knives::draw_upright(s, CX, g.fly_y, g.save.selected_knife, 1, PLAY_TOP);
            v.had_flying = true;
            v.old_fly_y = g.fly_y as i16;
            v.had_ready = false;
        } else if g.knives_left > 0 {
            knives::draw_upright(s, CX, READY_TIP_Y, g.save.selected_knife, 1, PLAY_TOP);
            v.had_flying = false;
            v.had_ready = true;
        } else {
            v.had_flying = false;
            v.had_ready = false;
        }
    } else {
        v.had_flying = false;
        v.had_ready = false;
    }

    // 5. Bouncing colliding knife (State::Bounce)
    if g.state == State::Bounce && g.bounce_knife.active {
        let bk = g.bounce_knife;
        knives::draw_rotated(s, bk.x as i32, bk.y as i32, 0, bk.ang, bk.skin, PLAY_TOP);
        let boxes = knives::rotated_bboxes(bk.x as i32, bk.y as i32, 0, bk.ang);
        v.bounce_boxes[0] = pack_box(boxes[0]);
        v.bounce_boxes[1] = pack_box(boxes[1]);
        v.had_bounce = true;
    } else {
        v.had_bounce = false;
    }

    // 6. Particles & sliced apple halves
    draw_particles(s, g, &mut v.part_boxes, &mut v.part_count);

    v.target_cy = cy as i16;
    v.target_r = TARGET_R as u8;
    v.target_angle = g.target_angle;
    v.boss = g.boss;
}

/// Full redraw (screen entry, state transition, RESUME).
pub fn full(s: &mut impl Surface, g: &Game, v: &mut ViewState) {
    *v = ViewState::new();
    match g.state {
        State::Title => draw_title_full(s, g, v),
        State::Shop => draw_shop_full(s, g),
        State::Over => draw_over_full(s, g),
        State::Playing | State::Bounce => {
            s.rect(0, 0, SIZE, SIZE, BG);
            draw_top_header(s, g);
            draw_score_hud(s, g.score);
            draw_apple_hud(s, g.save.apples);
            draw_knives_queue(s, g.knives_total, g.knives_left, g.save.selected_knife);
            draw_play_entities(s, g, v);
        }
        State::Shatter => {
            s.rect(0, 0, SIZE, SIZE, BG);
            draw_top_header(s, g);
            draw_score_hud(s, g.score);
            draw_apple_hud(s, g.save.apples);
            draw_knives_queue(s, g.knives_total, g.knives_left, g.save.selected_knife);
            draw_shatter_entities(s, g, v);
            draw_shatter_banner(s, g);
        }
    }
    v.valid = true;
    v.state = g.state;
    v.stage = g.stage;
    v.score = g.score;
    v.apples_total = g.save.apples;
    v.knives_left = g.knives_left;
    v.knives_total = g.knives_total;
    v.selected_knife = g.save.selected_knife;
    v.browse_knife = g.browse_knife;
    v.unlocked_mask = g.save.unlocked_mask;
}

fn draw_shatter_entities(s: &mut impl Surface, g: &Game, v: &mut ViewState) {
    v.chunk_count = 0;
    for ch in &g.chunks {
        if !ch.active {
            continue;
        }
        let x = ch.x as i32 - (ch.w as i32) / 2;
        let y = ch.y as i32 - (ch.h as i32) / 2;
        let w = ch.w as i32;
        let h = ch.h as i32;
        if y + h > PLAY_TOP && y < SIZE {
            let y0 = y.max(PLAY_TOP);
            let y1 = (y + h).min(SIZE);
            s.rect(x, y0, w, y1 - y0, ch.col_a);
            if y1 - y0 >= 4 {
                s.rect(x, y0, w, 3, ch.col_b);
            }
            if (v.chunk_count as usize) < MAX_CHUNKS {
                v.chunk_boxes[v.chunk_count as usize] = pack_box((x, y, w, h));
                v.chunk_count += 1;
            }
        }
    }

    v.loose_box_count = 0;
    for lk in &g.loose_knives {
        if !lk.active {
            continue;
        }
        knives::draw_rotated(s, lk.x as i32, lk.y as i32, 0, lk.ang, lk.skin, PLAY_TOP);
        let boxes = knives::rotated_bboxes(lk.x as i32, lk.y as i32, 0, lk.ang);
        for &b in &boxes {
            if (v.loose_box_count as usize) < v.loose_boxes.len() {
                v.loose_boxes[v.loose_box_count as usize] = pack_box(b);
                v.loose_box_count += 1;
            }
        }
    }

    draw_particles(s, g, &mut v.part_boxes, &mut v.part_count);
}

/// Differential frame update: erases ONLY moved objects and repaints them — zero flicker & < 850 rects!
pub fn frame(s: &mut impl Surface, g: &Game, v: &mut ViewState) {
    if !v.valid {
        full(s, g, v);
        return;
    }

    // Smooth non-flashing transition Playing -> Shatter (keeps HUD intact!)
    if v.state == State::Playing && g.state == State::Shatter {
        // Erase target area + old waiting knife cleanly
        erase_play(s, 94, PLAY_TOP, 278, 286, PLAY_TOP);
        erase_play(s, CX - 16, READY_TIP_Y - 4, 32, 86, PLAY_TOP);
        for i in 0..v.part_count as usize {
            let b = v.part_boxes[i];
            erase_play(s, b.0 as i32, b.1 as i32, b.2 as i32, b.3 as i32, PLAY_TOP);
        }
        if v.score != g.score {
            draw_score_hud(s, g.score);
        }
        if v.apples_total != g.save.apples {
            draw_apple_hud(s, g.save.apples);
        }
        draw_knives_queue(s, g.knives_total, g.knives_left, g.save.selected_knife);
        draw_shatter_entities(s, g, v);
        draw_shatter_banner(s, g);
        v.state = State::Shatter;
        v.score = g.score;
        v.apples_total = g.save.apples;
        v.knives_left = g.knives_left;
        return;
    }

    // Smooth non-flashing transition Playing -> Bounce
    let play_or_bounce_cont =
        (v.state == State::Playing || v.state == State::Bounce)
            && (g.state == State::Playing || g.state == State::Bounce);

    if !play_or_bounce_cont && v.state != g.state {
        full(s, g, v);
        return;
    }

    match g.state {
        State::Title => {
            draw_title_spinner(s, g, v);
            return;
        }
        State::Shop => {
            if v.browse_knife != g.browse_knife
                || v.selected_knife != g.save.selected_knife
                || v.apples_total != g.save.apples
                || v.unlocked_mask != g.save.unlocked_mask
            {
                full(s, g, v);
            }
            return;
        }
        State::Over => return,
        State::Shatter => {
            // Erase old chunks, loose knives, particles
            for i in 0..v.chunk_count as usize {
                let b = v.chunk_boxes[i];
                erase_play(s, b.0 as i32, b.1 as i32, b.2 as i32, b.3 as i32, PLAY_TOP);
            }
            for i in 0..v.loose_box_count as usize {
                let b = v.loose_boxes[i];
                erase_play(s, b.0 as i32, b.1 as i32, b.2 as i32, b.3 as i32, PLAY_TOP);
            }
            for i in 0..v.part_count as usize {
                let b = v.part_boxes[i];
                erase_play(s, b.0 as i32, b.1 as i32, b.2 as i32, b.3 as i32, PLAY_TOP);
            }
            draw_shatter_entities(s, g, v);
            draw_shatter_banner(s, g);
            return;
        }
        State::Playing | State::Bounce => {}
    }

    // Stage number changed (e.g. direct jump): full redraw
    if v.stage != g.stage {
        full(s, g, v);
        return;
    }

    // 1. Erase previous stuck knives
    for i in 0..v.stuck_box_count as usize {
        let b = v.stuck_boxes[i];
        erase_play(s, b.0 as i32, b.1 as i32, b.2 as i32, b.3 as i32, PLAY_TOP);
    }

    // 2. Erase previous rim apples
    for i in 0..v.apple_count as usize {
        let (ax, ay) = v.apple_pos[i];
        erase_play(s, ax as i32 - 10, ay as i32 - 12, 20, 22, PLAY_TOP);
    }

    // 3. Erase previous tire treads / saw teeth protrusions
    for i in 0..v.rim_extra_count as usize {
        let (rx, ry) = v.rim_extras[i];
        erase_play(s, rx as i32 - 7, ry as i32 - 7, 15, 15, PLAY_TOP);
    }

    // 4. Erase previous flying/ready knife
    if v.had_flying {
        erase_play(s, CX - 15, v.old_fly_y as i32 - 2, 30, 82, PLAY_TOP);
    } else if v.had_ready && (g.flying || g.knives_left == 0 || g.state == State::Bounce) {
        erase_play(s, CX - 15, READY_TIP_Y - 2, 30, 82, PLAY_TOP);
    }

    // 5. Erase previous bounce knife
    if v.had_bounce {
        for b in &v.bounce_boxes {
            erase_play(s, b.0 as i32, b.1 as i32, b.2 as i32, b.3 as i32, PLAY_TOP);
        }
    }

    // 6. Erase previous particles
    for i in 0..v.part_count as usize {
        let b = v.part_boxes[i];
        erase_play(s, b.0 as i32, b.1 as i32, b.2 as i32, b.3 as i32, PLAY_TOP);
    }

    // 7. If target recoiled vertically (`old_cy != new_cy`), erase the old disc footprint cleanly!
    let new_cy = g.target_cy();
    let old_cy = v.target_cy as i32;
    if old_cy != new_cy {
        fill_disc(s, CX, old_cy, TARGET_R + 2, BG, 6, PLAY_TOP);
    }

    // 8. Draw all new play entities
    draw_play_entities(s, g, v);

    // 9. Update HUD elements ONLY when their values actually changed!
    if v.score != g.score {
        draw_score_hud(s, g.score);
        v.score = g.score;
    }
    if v.apples_total != g.save.apples {
        draw_apple_hud(s, g.save.apples);
        v.apples_total = g.save.apples;
    }
    if v.knives_left != g.knives_left || v.knives_total != g.knives_total {
        draw_knives_queue(s, g.knives_total, g.knives_left, g.save.selected_knife);
        v.knives_left = g.knives_left;
        v.knives_total = g.knives_total;
    }
    v.state = g.state;
}

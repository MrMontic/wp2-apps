//! Ultra-fast, zero-flicker differential renderer for CMF Watch Pro 2 (466×466 round AMOLED).
//!
//! - Full arena + rails + brick wall painted ONCE on state entry/resume.
//! - Per gameplay frame: updates ONLY changed bricks, HUD sub-regions, and dirty entity bounds.
//! - Balls are drawn at their new positions BEFORE erasing `(old_bbox \ new_bbox)` so balls
//!   never blink on the single-buffered AMOLED panel.
//! - `ViewState` is mutated in-place (designed to live in `static mut`) to respect the 8 KiB stack limit.

use crate::game::{
    BALL_R, BRICK_H, BRICK_W, C, CAP_H, CAP_W, Game, LASER_H, LASER_W, MAX_BALLS, MAX_CAPSULES,
    MAX_LASERS, MAX_PARTICLES, PADDLE_H, PADDLE_Y, PLAY_B, PLAY_L, PLAY_R, PLAY_T, PowerKind,
    SHIELD_Y, SIZE, State, brick_rect,
};
use crate::levels::{Brick, BrickKind, COLS, GRID_LEN, LEVELS, NUM_LEVELS, ROWS};
use cmf_wp2_ui::Surface;
use cmf_wp2_ui::color::rgb565;
use cmf_wp2_ui::text::{big, big_width, centre, int};

// Sleek OLED Arcade Palette
const OLED_BG: u16 = rgb565(3, 5, 10);
const ARENA_BG: u16 = rgb565(6, 9, 16);
const RAIL_DIM: u16 = rgb565(22, 62, 86);
const RAIL_GLOW: u16 = rgb565(42, 214, 248);
const CMF_ORANGE: u16 = rgb565(255, 85, 26);
const CMF_ORANGE_HI: u16 = rgb565(255, 152, 92);
const CMF_ORANGE_DK: u16 = rgb565(146, 40, 8);

const WHITE: u16 = rgb565(250, 252, 255);
const INK_DIM: u16 = rgb565(132, 148, 170);
const GOLD_HI: u16 = rgb565(255, 234, 118);
const GOLD_MID: u16 = rgb565(236, 182, 34);
const GOLD_DK: u16 = rgb565(132, 88, 12);

const HEART_RED: u16 = rgb565(248, 52, 84);
const HEART_HI: u16 = rgb565(255, 138, 156);

const PADDLE_BODY: u16 = rgb565(42, 186, 224);
const PADDLE_HI: u16 = rgb565(168, 242, 255);
const PADDLE_DK: u16 = rgb565(18, 86, 118);
const LASER_COL: u16 = rgb565(255, 76, 56);
const LASER_CORE: u16 = rgb565(255, 236, 152);

const FIRE_OUT: u16 = rgb565(255, 78, 22);
const FIRE_MID: u16 = rgb565(255, 186, 44);
const FIRE_CORE: u16 = rgb565(255, 252, 200);

const BALL_HALO: u16 = rgb565(72, 226, 255);
const SHIELD_GLOW: u16 = rgb565(32, 212, 246);
const SHIELD_DIM: u16 = rgb565(20, 124, 154);

const CRACK_COL: u16 = rgb565(18, 24, 36);

#[inline]
const fn brick_palette(kind: BrickKind) -> (u16, u16, u16) {
    // (fill, highlight, shadow)
    match kind {
        BrickKind::Crimson => (rgb565(228, 38, 72), rgb565(255, 116, 138), rgb565(126, 16, 36)),
        BrickKind::Orange => (CMF_ORANGE, CMF_ORANGE_HI, CMF_ORANGE_DK),
        BrickKind::Gold => (rgb565(238, 190, 28), rgb565(255, 234, 118), rgb565(138, 102, 10)),
        BrickKind::Emerald => (rgb565(26, 204, 116), rgb565(112, 250, 180), rgb565(10, 106, 58)),
        BrickKind::Cyan => (rgb565(24, 196, 236), rgb565(126, 242, 255), rgb565(10, 96, 126)),
        BrickKind::Violet => (rgb565(166, 74, 244), rgb565(216, 156, 255), rgb565(88, 32, 140)),
        BrickKind::Silver => (rgb565(166, 178, 196), rgb565(232, 240, 250), rgb565(86, 96, 112)),
        BrickKind::Titanium => (rgb565(74, 112, 156), rgb565(142, 184, 228), rgb565(34, 56, 84)),
        BrickKind::Tnt => (rgb565(224, 42, 34), rgb565(255, 214, 46), rgb565(118, 18, 14)),
        BrickKind::Bumper => (GOLD_MID, GOLD_HI, GOLD_DK),
        BrickKind::Empty => (ARENA_BG, ARENA_BG, ARENA_BG),
    }
}

#[inline]
const fn power_colors(kind: PowerKind) -> (u16, u16) {
    // (body, highlight)
    match kind {
        PowerKind::Multi => (rgb565(36, 198, 255), rgb565(146, 242, 255)),
        PowerKind::Wide => (rgb565(34, 206, 118), rgb565(132, 252, 188)),
        PowerKind::Fire => (CMF_ORANGE, rgb565(255, 196, 92)),
        PowerKind::Laser => (rgb565(236, 44, 72), rgb565(255, 136, 148)),
        PowerKind::Shield => (rgb565(56, 134, 255), rgb565(152, 204, 255)),
        PowerKind::Slow => (rgb565(172, 82, 248), rgb565(224, 168, 255)),
        PowerKind::Life => (rgb565(242, 62, 128), rgb565(255, 168, 202)),
    }
}

#[inline]
fn erase_arena(s: &mut impl Surface, x: i32, y: i32, w: i32, h: i32) {
    let x0 = x.max(PLAY_L);
    let y0 = y.max(PLAY_T);
    let x1 = (x + w).min(PLAY_R);
    let y1 = (y + h).min(PLAY_B);
    if x0 < x1 && y0 < y1 {
        s.rect(x0, y0, x1 - x0, y1 - y0, ARENA_BG);
    }
}

#[inline]
fn rects_overlap(a: (i32, i32, i32, i32), b: (i32, i32, i32, i32)) -> bool {
    a.0 < b.0 + b.2 && a.0 + a.2 > b.0 && a.1 < b.1 + b.3 && a.1 + a.3 > b.1
}

/// Draws a single glossy 3D beveled brick (4..9 rects).
fn draw_brick(s: &mut impl Surface, r: usize, c: usize, b: Brick) {
    let (x, y, w, h) = brick_rect(r, c);
    if !b.alive() {
        erase_arena(s, x, y, w, h);
        return;
    }

    let (fill, hi, dk) = if b.flash > 0 {
        (WHITE, WHITE, GOLD_HI)
    } else {
        brick_palette(b.kind)
    };

    // Glossy 3D bevel body (5 rects)
    s.rect(x, y, w, h, fill);
    s.rect(x, y, w, 2, hi);
    s.rect(x, y + 2, 2, h - 2, hi);
    s.rect(x, y + h - 2, w, 2, dk);
    s.rect(x + w - 2, y + 2, 2, h - 4, dk);

    match b.kind {
        BrickKind::Tnt => {
            // Bold yellow hazard stripes + blazing inner core!
            s.rect(x + 4, y + 3, 5, h - 6, GOLD_HI);
            s.rect(x + w - 9, y + 3, 5, h - 6, GOLD_HI);
            s.rect(x + 12, y + 5, 9, h - 10, WHITE);
        }
        BrickKind::Bumper => {
            // Metallic gold bumper center bar
            s.rect(x + 6, y + 5, w - 12, h - 10, GOLD_HI);
        }
        BrickKind::Silver => {
            // Brushed metallic glint; visible crack when damaged to 1 HP!
            if b.hp <= 1 {
                s.rect(x + 11, y + 2, 2, 5, CRACK_COL);
                s.rect(x + 13, y + 6, 4, 2, CRACK_COL);
                s.rect(x + 16, y + 8, 2, 6, CRACK_COL);
            } else {
                s.rect(x + 5, y + 4, 8, 2, WHITE);
            }
        }
        BrickKind::Titanium => {
            // Corner rivets + progressive cracks at 2 HP and 1 HP!
            s.rect(x + 4, y + 4, 2, 2, hi);
            s.rect(x + w - 6, y + 4, 2, 2, hi);
            if b.hp <= 2 {
                s.rect(x + 10, y + 2, 2, 6, CRACK_COL);
                s.rect(x + 12, y + 7, 5, 2, CRACK_COL);
            }
            if b.hp <= 1 {
                s.rect(x + 21, y + 3, 2, 5, CRACK_COL);
                s.rect(x + 17, y + 8, 5, 2, CRACK_COL);
                s.rect(x + 15, y + 10, 2, 4, CRACK_COL);
            }
        }
        _ => {
            // Specular glossy highlight bar
            s.rect(x + 4, y + 4, 7, 2, hi);
        }
    }
}

/// Draws a crisp pixel heart (`9×8` px, 4 rects).
fn draw_heart(s: &mut impl Surface, x: i32, y: i32, full: bool) {
    let col = if full { HEART_RED } else { rgb565(36, 42, 56) };
    s.rect(x + 1, y, 3, 2, col);
    s.rect(x + 5, y, 3, 2, col);
    s.rect(x, y + 2, 9, 3, col);
    s.rect(x + 2, y + 5, 5, 2, col);
    s.rect(x + 4, y + 7, 1, 1, col);
    if full {
        s.rect(x + 2, y + 1, 1, 1, HEART_HI);
    }
}

/// Draws a crisp 3×5 mini glyph inside a power-up pill.
fn draw_mini_glyph(s: &mut impl Surface, x: i32, y: i32, ch: u8, col: u16) {
    match ch {
        b'M' => {
            s.rect(x, y, 1, 5, col);
            s.rect(x + 4, y, 1, 5, col);
            s.rect(x + 1, y + 1, 1, 1, col);
            s.rect(x + 2, y + 2, 1, 1, col);
            s.rect(x + 3, y + 1, 1, 1, col);
        }
        b'W' => {
            s.rect(x, y, 1, 5, col);
            s.rect(x + 4, y, 1, 5, col);
            s.rect(x + 1, y + 3, 1, 1, col);
            s.rect(x + 2, y + 2, 1, 1, col);
            s.rect(x + 3, y + 3, 1, 1, col);
        }
        b'F' => {
            s.rect(x + 1, y, 1, 5, col);
            s.rect(x + 2, y, 2, 1, col);
            s.rect(x + 2, y + 2, 2, 1, col);
        }
        b'L' => {
            s.rect(x + 1, y, 1, 5, col);
            s.rect(x + 2, y + 4, 2, 1, col);
        }
        b'S' => {
            s.rect(x + 1, y, 3, 1, col);
            s.rect(x + 1, y + 1, 1, 1, col);
            s.rect(x + 1, y + 2, 3, 1, col);
            s.rect(x + 3, y + 3, 1, 1, col);
            s.rect(x + 1, y + 4, 3, 1, col);
        }
        b'C' => {
            s.rect(x + 1, y, 3, 1, col);
            s.rect(x + 1, y + 1, 1, 3, col);
            s.rect(x + 1, y + 4, 3, 1, col);
        }
        _ => {
            // '+'
            s.rect(x + 2, y, 1, 5, col);
            s.rect(x, y + 2, 5, 1, col);
        }
    }
}

fn draw_capsule(s: &mut impl Surface, cx: i32, cy: i32, kind: PowerKind) {
    let x = cx - CAP_W / 2;
    let y = cy - CAP_H / 2;
    if y + CAP_H <= PLAY_T || y >= PLAY_B {
        return;
    }
    let (body, hi) = power_colors(kind);
    let clip = |s: &mut dyn Surface, rx: i32, ry: i32, rw: i32, rh: i32, c: u16| {
        let x0 = rx.max(PLAY_L);
        let y0 = ry.max(PLAY_T);
        let x1 = (rx + rw).min(PLAY_R);
        let y1 = (ry + rh).min(PLAY_B);
        if x0 < x1 && y0 < y1 {
            s.rect(x0, y0, x1 - x0, y1 - y0, c);
        }
    };
    // Rounded glowing capsule pill
    clip(s, x + 3, y, CAP_W - 6, 2, hi);
    clip(s, x, y + 2, CAP_W, CAP_H - 4, body);
    clip(s, x + 3, y + CAP_H - 2, CAP_W - 6, 2, body);
    clip(s, x + 2, y + 3, 2, CAP_H - 6, hi);
    // Crisp white letter icon centered
    if y + 4 >= PLAY_T && y + 9 <= PLAY_B {
        draw_mini_glyph(s, cx - 2, cy - 2, kind.letter(), WHITE);
    }
}

#[inline]
fn capsule_bbox(cx: i32, cy: i32) -> (i32, i32, i32, i32) {
    (cx - CAP_W / 2, cy - CAP_H / 2, CAP_W, CAP_H)
}

fn draw_paddle(s: &mut impl Surface, cx: i32, w: i32, laser: bool) {
    let x = cx - w / 2;
    let y = PADDLE_Y;
    // Sleek glowing arcade paddle with CMF-orange end bumpers
    s.rect(x + 3, y, w - 6, 2, PADDLE_HI);
    s.rect(x + 1, y + 2, w - 2, PADDLE_H - 4, PADDLE_BODY);
    s.rect(x + 3, y + PADDLE_H - 2, w - 6, 2, PADDLE_DK);
    // Glowing CMF-orange left & right end-caps
    s.rect(x, y + 2, 8, PADDLE_H - 4, CMF_ORANGE);
    s.rect(x + w - 8, y + 2, 8, PADDLE_H - 4, CMF_ORANGE);
    s.rect(x + 1, y + 2, 6, 2, CMF_ORANGE_HI);
    s.rect(x + w - 7, y + 2, 6, 2, CMF_ORANGE_HI);
    // Center sight pip
    s.rect(cx - 5, y + 4, 10, 3, WHITE);

    // Twin laser cannons mounted on shoulders!
    if laser {
        s.rect(x + 3, y - 5, 4, 5, LASER_COL);
        s.rect(x + 4, y - 5, 2, 2, LASER_CORE);
        s.rect(x + w - 7, y - 5, 4, 5, LASER_COL);
        s.rect(x + w - 6, y - 5, 2, 2, LASER_CORE);
    }
}

#[inline]
fn paddle_bbox(cx: i32, w: i32) -> (i32, i32, i32, i32) {
    (cx - w / 2, PADDLE_Y - 5, w, PADDLE_H + 5)
}

fn draw_ball(s: &mut impl Surface, cx: i32, cy: i32, fireball: bool) {
    if cy - BALL_R >= PLAY_B || cy + BALL_R <= PLAY_T {
        return;
    }
    let clip = |s: &mut dyn Surface, rx: i32, ry: i32, rw: i32, rh: i32, c: u16| {
        let x0 = rx.max(PLAY_L);
        let y0 = ry.max(PLAY_T);
        let x1 = (rx + rw).min(PLAY_R);
        let y1 = (ry + rh).min(PLAY_B);
        if x0 < x1 && y0 < y1 {
            s.rect(x0, y0, x1 - x0, y1 - y0, c);
        }
    };
    let (outer, mid, core) = if fireball {
        (FIRE_OUT, FIRE_MID, FIRE_CORE)
    } else {
        (BALL_HALO, WHITE, WHITE)
    };
    // 10×10 crisp rounded glowing orb (4 rects)
    clip(s, cx - 3, cy - 5, 6, 10, outer);
    clip(s, cx - 5, cy - 3, 10, 6, outer);
    clip(s, cx - 3, cy - 3, 6, 6, mid);
    clip(s, cx - 2, cy - 2, 3, 3, core);
}

#[inline]
fn ball_bbox(cx: i32, cy: i32) -> (i32, i32, i32, i32) {
    (cx - BALL_R, cy - BALL_R, BALL_R * 2, BALL_R * 2)
}

/// Erases `old \ new` using non-overlapping strips + any rounded 2×2 corner cutouts of `new`
/// that were previously covered by `old` (zero flicker AND pixel-exact!).
fn erase_ball_residual(s: &mut impl Surface, old: (i32, i32, i32, i32), new: (i32, i32, i32, i32)) {
    let (ox0, oy0, ow, oh) = old;
    let ox1 = ox0 + ow;
    let oy1 = oy0 + oh;
    let (nx0, ny0, nw, nh) = new;
    let nx1 = nx0 + nw;
    let ny1 = ny0 + nh;

    // Horizontal residual outside [ny0, ny1)
    if oy0 < ny0 {
        let h = (ny0.min(oy1)) - oy0;
        erase_arena(s, ox0, oy0, ow, h);
    }
    if oy1 > ny1 {
        let y0 = ny1.max(oy0);
        erase_arena(s, ox0, y0, ow, oy1 - y0);
    }
    // Vertical residual inside [oy0..oy1] ∩ [ny0..ny1]
    let iy0 = oy0.max(ny0);
    let iy1 = oy1.min(ny1);
    if iy0 < iy1 {
        if ox0 < nx0 {
            erase_arena(s, ox0, iy0, nx0.min(ox1) - ox0, iy1 - iy0);
        }
        if ox1 > nx1 {
            let x0 = nx1.max(ox0);
            erase_arena(s, x0, iy0, ox1 - x0, iy1 - iy0);
        }
    }
    // Clean any of the 4 rounded 2×2 corner cutouts of `new` that overlap `old`
    let corners = [
        (nx0, ny0, 2, 2),
        (nx1 - 2, ny0, 2, 2),
        (nx0, ny1 - 2, 2, 2),
        (nx1 - 2, ny1 - 2, 2, 2),
    ];
    for c in corners {
        if rects_overlap(c, old) {
            erase_arena(s, c.0, c.1, c.2, c.3);
        }
    }
}

fn draw_laser(s: &mut impl Surface, x: i32, y: i32) {
    let x0 = (x - LASER_W / 2).max(PLAY_L);
    let y0 = y.max(PLAY_T);
    let x1 = (x + LASER_W / 2).min(PLAY_R);
    let y1 = (y + LASER_H).min(PLAY_B);
    if x0 < x1 && y0 < y1 {
        s.rect(x0, y0, x1 - x0, y1 - y0, LASER_COL);
        if x1 - x0 >= 2 && y1 - y0 >= 4 {
            s.rect(x0 + 1, y0 + 1, (x1 - x0 - 2).max(1), y1 - y0 - 2, LASER_CORE);
        }
    }
}

fn draw_shield(s: &mut impl Surface, on: bool, hits: u8, phase: u8) {
    let x0 = PLAY_L + 4;
    let w = (PLAY_R - PLAY_L) - 8;
    if !on {
        erase_arena(s, x0, SHIELD_Y - 1, w, 5);
        return;
    }
    let col = if hits >= 2 || (phase & 1) == 0 {
        SHIELD_GLOW
    } else {
        SHIELD_DIM
    };
    s.rect(x0, SHIELD_Y, w, 3, col);
    s.rect(x0 + 12, SHIELD_Y + 1, w - 24, 1, WHITE);
}

/// Draws the sleek outer playfield frame & glowing cyan/orange rails ONCE.
fn draw_arena_frame(s: &mut impl Surface) {
    s.rect(0, 0, SIZE, SIZE, OLED_BG);
    // Playfield interior
    s.rect(PLAY_L, PLAY_T, PLAY_R - PLAY_L, PLAY_B - PLAY_T, ARENA_BG);

    // Glowing cyan side & top rails
    s.rect(PLAY_L - 3, PLAY_T - 3, 3, PLAY_B - PLAY_T + 3, RAIL_DIM);
    s.rect(PLAY_L - 1, PLAY_T - 1, 1, PLAY_B - PLAY_T + 1, RAIL_GLOW);

    s.rect(PLAY_R, PLAY_T - 3, 3, PLAY_B - PLAY_T + 3, RAIL_DIM);
    s.rect(PLAY_R, PLAY_T - 1, 1, PLAY_B - PLAY_T + 1, RAIL_GLOW);

    s.rect(PLAY_L - 3, PLAY_T - 3, PLAY_R - PLAY_L + 6, 3, RAIL_DIM);
    s.rect(PLAY_L - 1, PLAY_T - 1, PLAY_R - PLAY_L + 2, 1, RAIL_GLOW);

    // CMF Orange shoulder accents on top rail corners
    s.rect(PLAY_L - 3, PLAY_T - 3, 28, 3, CMF_ORANGE);
    s.rect(PLAY_R - 25, PLAY_T - 3, 28, 3, CMF_ORANGE);

    // Subtle bottom gutter marker line
    s.rect(PLAY_L + 20, PLAY_B, PLAY_R - PLAY_L - 40, 2, rgb565(22, 30, 46));
}

fn draw_hud_score(s: &mut impl Surface, score: u32, beat_best: bool) {
    s.rect(132, 18, 202, 20, OLED_BG);
    let mut buf = [0u8; 11];
    let text = int(score.min(9_999_999) as i32, &mut buf);
    let col = if beat_best { GOLD_HI } else { WHITE };
    centre(s, C, 20, text, col);
}

fn draw_hud_level_combo(s: &mut impl Surface, level: u8, wave: u8, combo: u8) {
    s.rect(96, 42, 96, 20, OLED_BG);
    let mut b = [0u8; 8];
    let lvl_num = level + 1;
    let label: &str = if wave == 0 {
        b[0] = b'L';
        b[1] = b'0' + (lvl_num / 10);
        b[2] = b'0' + (lvl_num % 10);
        core::str::from_utf8(&b[..3]).unwrap_or("L01")
    } else {
        let w = (wave + 1).min(9);
        b[0] = b'W';
        b[1] = b'0' + w;
        b[2] = b'-';
        b[3] = b'0' + (lvl_num / 10);
        b[4] = b'0' + (lvl_num % 10);
        core::str::from_utf8(&b[..5]).unwrap_or("W2")
    };
    s.text(98, 44, label, RAIL_GLOW);

    if combo >= 2 {
        let mut cb = [b'x', b'0' + combo.min(9)];
        let cstr = core::str::from_utf8(&cb).unwrap_or("x2");
        let cx = if wave == 0 { 154 } else { 180 };
        let col = if combo >= 5 { CMF_ORANGE } else { GOLD_HI };
        s.text(cx, 44, cstr, col);
        let _ = &mut cb;
    }
}

fn draw_hud_lives(s: &mut impl Surface, lives: u8) {
    s.rect(298, 44, 68, 14, OLED_BG);
    for i in 0..5u8 {
        let hx = 300 + i as i32 * 12;
        if i < lives || i < 3 {
            draw_heart(s, hx, 47, i < lives);
        }
    }
}

fn draw_hud_power_bar(s: &mut impl Surface, kind: Option<PowerKind>, fill_px: u8) {
    let bx = 196;
    let by = 46;
    let bw = 92;
    s.rect(bx, by, bw, 12, OLED_BG);
    let Some(k) = kind else {
        return;
    };
    let (body, hi) = power_colors(k);
    // Tiny letter pill on left + remaining time bar on right
    s.rect(bx, by, 13, 11, body);
    draw_mini_glyph(s, bx + 4, by + 3, k.letter(), WHITE);
    s.rect(bx + 16, by + 2, 74, 7, rgb565(20, 28, 42));
    let px = (fill_px as i32).clamp(0, 72);
    if px > 0 {
        s.rect(bx + 17, by + 3, px, 5, body);
        s.rect(bx + 17, by + 3, px, 2, hi);
    }
}

/// Footer status bar below `PLAY_B = 398` (shows launch prompt, power-up callout, or level name).
#[derive(Clone, Copy, PartialEq, Eq)]
enum FooterTag {
    None,
    Launch,
    Power(PowerKind),
    LevelName(u8),
}

fn current_footer(g: &Game) -> FooterTag {
    if g.has_glued_ball() {
        FooterTag::Launch
    } else if let Some(k) = g.banner_kind {
        FooterTag::Power(k)
    } else {
        FooterTag::LevelName(g.level)
    }
}

fn draw_footer(s: &mut impl Surface, tag: FooterTag) {
    s.rect(92, 408, 282, 22, OLED_BG);
    match tag {
        FooterTag::None => {}
        FooterTag::Launch => {
            centre(s, C, 410, "TAP TO LAUNCH", CMF_ORANGE_HI);
        }
        FooterTag::Power(k) => {
            let (_, hi) = power_colors(k);
            centre(s, C, 410, k.label(), hi);
        }
        FooterTag::LevelName(idx) => {
            let name = LEVELS[(idx as usize) % NUM_LEVELS].name;
            centre(s, C, 410, name, INK_DIM);
        }
    }
}

/// Draws the interactive level-selector strip on the Title screen.
fn draw_title_level_strip(s: &mut impl Surface, g: &Game) {
    s.rect(72, 228, 322, 54, OLED_BG);

    let max_l = g.save.max_level.clamp(1, NUM_LEVELS as u8);
    let sel = g.selected_level.clamp(1, max_l);

    // Left / Right arrows when multiple levels unlocked
    if max_l > 1 {
        s.rect(86, 232, 34, 28, rgb565(18, 32, 48));
        centre(s, 103, 238, "<", RAIL_GLOW);
        s.rect(346, 232, 34, 28, rgb565(18, 32, 48));
        centre(s, 363, 238, ">", RAIL_GLOW);
    }

    let mut buf = [0u8; 10];
    buf[..4].copy_from_slice(b"LVL ");
    buf[4] = b'0' + (sel / 10);
    buf[5] = b'0' + (sel % 10);
    let lbl = core::str::from_utf8(&buf[..6]).unwrap_or("LVL 01");
    centre(s, C, 236, lbl, WHITE);
    centre(s, C, 260, g.selected_level_name(), CMF_ORANGE_HI);
}

fn draw_title_full(s: &mut impl Surface, g: &Game) {
    s.rect(0, 0, SIZE, SIZE, OLED_BG);

    // Decorative arcade neon brick ribbon at top
    let demo_kinds = [
        BrickKind::Crimson,
        BrickKind::Orange,
        BrickKind::Gold,
        BrickKind::Emerald,
        BrickKind::Cyan,
        BrickKind::Violet,
    ];
    for (i, &k) in demo_kinds.iter().enumerate() {
        let bx = 113 + i as i32 * 40;
        let (fill, hi, dk) = brick_palette(k);
        s.rect(bx, 74, 36, 14, fill);
        s.rect(bx, 74, 36, 2, hi);
        s.rect(bx, 86, 36, 2, dk);
    }

    // Crisp big title
    let title = "ARKANOID";
    let tw = big_width(title, 4);
    big(s, C - tw / 2, 114, title, 4, CMF_ORANGE);
    big(s, C - tw / 2, 112, title, 4, WHITE);

    centre(s, C, 162, "CROWN OR TAP", RAIL_GLOW);

    // Best score line
    if g.save.best_score > 0 {
        let mut b = [0u8; 20];
        b[..5].copy_from_slice(b"BEST ");
        let mut db = [0u8; 11];
        let ds = int(g.save.best_score.min(9_999_999) as i32, &mut db).as_bytes();
        b[5..5 + ds.len()].copy_from_slice(ds);
        if let Ok(lbl) = core::str::from_utf8(&b[..5 + ds.len()]) {
            centre(s, C, 192, lbl, GOLD_HI);
        }
    } else {
        centre(s, C, 192, "12 ARCADE STAGES", INK_DIM);
    }

    draw_title_level_strip(s, g);

    // Glowing PLAY button
    s.rect(128, 306, 210, 42, CMF_ORANGE);
    s.rect(132, 308, 202, 3, CMF_ORANGE_HI);
    s.rect(132, 344, 202, 4, CMF_ORANGE_DK);
    centre(s, C, 319, "TAP TO PLAY", WHITE);

    // Mini decorative paddle & glowing ball at bottom
    draw_paddle(s, C, PADDLE_W_BASE, false);
    draw_ball(s, C + 18, PADDLE_Y - 12, false);
}

fn draw_stage_clear_full(s: &mut impl Surface, g: &Game) {
    s.rect(0, 0, SIZE, SIZE, OLED_BG);

    let t = "CLEAR!";
    big(s, C - big_width(t, 5) / 2, 92, t, 5, rgb565(42, 240, 146));
    centre(s, C, 152, g.level_name(), GOLD_HI);

    // Stage bonus line
    let mut b = [0u8; 20];
    b[..6].copy_from_slice(b"BONUS ");
    let mut db = [0u8; 11];
    let ds = int(g.stage_bonus as i32, &mut db).as_bytes();
    b[6..6 + ds.len()].copy_from_slice(ds);
    if let Ok(lbl) = core::str::from_utf8(&b[..6 + ds.len()]) {
        centre(s, C, 188, lbl, CMF_ORANGE_HI);
    }

    // Score
    let mut sb = [0u8; 11];
    let stxt = int(g.score.min(9_999_999) as i32, &mut sb);
    big(s, C - big_width(stxt, 4) / 2, 224, stxt, 4, WHITE);

    // Next stage preview
    if g.level + 1 >= NUM_LEVELS as u8 {
        let mut wb = [0u8; 14];
        wb[..10].copy_from_slice(b"NEXT WAVE ");
        wb[10] = b'0' + (g.wave + 2).min(9);
        if let Ok(lbl) = core::str::from_utf8(&wb[..11]) {
            centre(s, C, 280, lbl, RAIL_GLOW);
        }
    } else {
        let next_n = g.level + 2;
        let mut nb = [0u8; 12];
        nb[..9].copy_from_slice(b"NEXT LVL ");
        nb[9] = b'0' + (next_n / 10);
        nb[10] = b'0' + (next_n % 10);
        if let Ok(lbl) = core::str::from_utf8(&nb[..11]) {
            centre(s, C, 280, lbl, RAIL_GLOW);
        }
    }

    s.rect(122, 326, 222, 40, CMF_ORANGE);
    s.rect(126, 328, 214, 3, CMF_ORANGE_HI);
    centre(s, C, 338, "TAP NEXT", WHITE);
}

fn draw_game_over_full(s: &mut impl Surface, g: &Game) {
    s.rect(0, 0, SIZE, SIZE, OLED_BG);

    if g.beat_best {
        centre(s, C, 96, "NEW BEST!", GOLD_HI);
    } else {
        centre(s, C, 96, "GAME OVER", rgb565(255, 76, 98));
    }

    let mut sb = [0u8; 11];
    let stxt = int(g.score.min(9_999_999) as i32, &mut sb);
    let scale = if stxt.len() <= 5 { 5 } else { 4 };
    big(s, C - big_width(stxt, scale) / 2, 134, stxt, scale, WHITE);

    // Best score
    let mut b = [0u8; 20];
    b[..5].copy_from_slice(b"BEST ");
    let mut db = [0u8; 11];
    let ds = int(g.save.best_score.min(9_999_999) as i32, &mut db).as_bytes();
    b[5..5 + ds.len()].copy_from_slice(ds);
    if let Ok(lbl) = core::str::from_utf8(&b[..5 + ds.len()]) {
        centre(s, C, 206, lbl, INK_DIM);
    }

    // Reached stage
    let lvl = g.level + 1;
    let mut lb = [0u8; 14];
    lb[..6].copy_from_slice(b"STAGE ");
    lb[6] = b'0' + (lvl / 10);
    lb[7] = b'0' + (lvl % 10);
    if let Ok(lbl) = core::str::from_utf8(&lb[..8]) {
        centre(s, C, 240, lbl, RAIL_GLOW);
    }

    s.rect(122, 304, 222, 42, CMF_ORANGE);
    s.rect(126, 306, 214, 3, CMF_ORANGE_HI);
    centre(s, C, 317, "TAP RETRY", WHITE);
}

const PADDLE_W_BASE: i32 = crate::game::PADDLE_W_BASE;

#[derive(Clone, Copy, PartialEq, Eq)]
struct BallSnap {
    active: bool,
    x: i16,
    y: i16,
    fire: bool,
}

impl BallSnap {
    const EMPTY: Self = Self {
        active: false,
        x: 0,
        y: 0,
        fire: false,
    };
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct CapSnap {
    active: bool,
    x: i16,
    y: i16,
    kind: PowerKind,
}

impl CapSnap {
    const EMPTY: Self = Self {
        active: false,
        x: 0,
        y: 0,
        kind: PowerKind::Multi,
    };
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct PosSnap {
    active: bool,
    x: i16,
    y: i16,
}

impl PosSnap {
    const EMPTY: Self = Self {
        active: false,
        x: 0,
        y: 0,
    };
}

/// Persistent differential view cache. Lives in `static mut VIEW` on watch (zero stack copy!).
pub struct ViewState {
    valid: bool,
    state: State,
    selected_level: u8,
    score: u32,
    beat_best: bool,
    level: u8,
    wave: u8,
    combo: u8,
    lives: u8,
    bar_kind: Option<PowerKind>,
    bar_px: u8,
    footer: FooterTag,
    paddle_x: i16,
    paddle_w: i16,
    paddle_laser: bool,
    shield_on: bool,
    shield_hits: u8,
    shield_phase: u8,
    bricks: [Brick; GRID_LEN],
    balls: [BallSnap; MAX_BALLS],
    capsules: [CapSnap; MAX_CAPSULES],
    lasers: [PosSnap; MAX_LASERS],
    particles: [PosSnap; MAX_PARTICLES],
}

impl ViewState {
    pub const fn new_const() -> Self {
        Self {
            valid: false,
            state: State::Title,
            selected_level: 1,
            score: 0,
            beat_best: false,
            level: 0,
            wave: 0,
            combo: 0,
            lives: 0,
            bar_kind: None,
            bar_px: 0,
            footer: FooterTag::None,
            paddle_x: C as i16,
            paddle_w: PADDLE_W_BASE as i16,
            paddle_laser: false,
            shield_on: false,
            shield_hits: 0,
            shield_phase: 0,
            bricks: [Brick::EMPTY; GRID_LEN],
            balls: [BallSnap::EMPTY; MAX_BALLS],
            capsules: [CapSnap::EMPTY; MAX_CAPSULES],
            lasers: [PosSnap::EMPTY; MAX_LASERS],
            particles: [PosSnap::EMPTY; MAX_PARTICLES],
        }
    }

    #[inline]
    pub fn invalidate(&mut self) {
        self.valid = false;
    }

    fn sync_from(&mut self, g: &Game) {
        self.valid = true;
        self.state = g.state;
        self.selected_level = g.selected_level;
        self.score = g.score;
        self.beat_best = g.beat_best;
        self.level = g.level;
        self.wave = g.wave;
        self.combo = g.combo;
        self.lives = g.lives;
        let (bk, bpx) = quantize_power_bar(g);
        self.bar_kind = bk;
        self.bar_px = bpx;
        self.footer = current_footer(g);
        self.paddle_x = g.paddle_x as i16;
        self.paddle_w = g.paddle_w as i16;
        self.paddle_laser = g.laser_ticks > 0;
        self.shield_on = g.shield_ticks > 0 && g.shield_hits > 0;
        self.shield_hits = g.shield_hits;
        self.shield_phase = ((g.tick >> 3) & 1) as u8;
        self.bricks = g.bricks;

        let fire = g.fire_ticks > 0;
        for i in 0..MAX_BALLS {
            let b = g.balls[i];
            self.balls[i] = if b.active {
                BallSnap {
                    active: true,
                    x: b.px() as i16,
                    y: b.py() as i16,
                    fire,
                }
            } else {
                BallSnap::EMPTY
            };
        }
        for i in 0..MAX_CAPSULES {
            let c = g.capsules[i];
            self.capsules[i] = if c.active {
                CapSnap {
                    active: true,
                    x: c.x,
                    y: c.py() as i16,
                    kind: c.kind,
                }
            } else {
                CapSnap::EMPTY
            };
        }
        for i in 0..MAX_LASERS {
            let l = g.lasers[i];
            self.lasers[i] = if l.active {
                PosSnap {
                    active: true,
                    x: l.x,
                    y: l.y,
                }
            } else {
                PosSnap::EMPTY
            };
        }
        for i in 0..MAX_PARTICLES {
            let p = g.particles[i];
            self.particles[i] = if p.active {
                PosSnap {
                    active: true,
                    x: p.px() as i16,
                    y: p.py() as i16,
                }
            } else {
                PosSnap::EMPTY
            };
        }
    }
}

fn quantize_power_bar(g: &Game) -> (Option<PowerKind>, u8) {
    match g.primary_power_timer() {
        Some((k, rem, max)) => {
            let raw = ((rem as u32 * 72) / (max.max(1) as u32)).min(72) as u8;
            // Quantize to 2px steps so HUD bar doesn't repaint every single tick
            (Some(k), raw & !1)
        }
        None => (None, 0),
    }
}

/// Full redraw (used on startup, state transitions, and RESUME).
pub fn draw_full(s: &mut impl Surface, g: &Game) {
    match g.state {
        State::Title => draw_title_full(s, g),
        State::StageClear => draw_stage_clear_full(s, g),
        State::GameOver => draw_game_over_full(s, g),
        State::Playing => {
            draw_arena_frame(s);
            draw_hud_score(s, g.score, g.beat_best);
            draw_hud_level_combo(s, g.level, g.wave, g.combo);
            draw_hud_lives(s, g.lives);
            let (bk, bpx) = quantize_power_bar(g);
            draw_hud_power_bar(s, bk, bpx);
            draw_footer(s, current_footer(g));

            for r in 0..ROWS {
                for c in 0..COLS {
                    let b = g.bricks[r * COLS + c];
                    if b.alive() {
                        draw_brick(s, r, c, b);
                    }
                }
            }
            for &cap in &g.capsules {
                if cap.active {
                    draw_capsule(s, cap.x as i32, cap.py(), cap.kind);
                }
            }
            for &l in &g.lasers {
                if l.active {
                    draw_laser(s, l.x as i32, l.y as i32);
                }
            }
            for &p in &g.particles {
                if p.active {
                    s.rect(p.px() - 1, p.py() - 1, 3, 3, p.color);
                }
            }
            if g.shield_ticks > 0 && g.shield_hits > 0 {
                draw_shield(s, true, g.shield_hits, ((g.tick >> 3) & 1) as u8);
            }
            draw_paddle(s, g.paddle_x, g.paddle_w, g.laser_ticks > 0);
            let fire = g.fire_ticks > 0;
            for &b in &g.balls {
                if b.active {
                    draw_ball(s, b.px(), b.py(), fire);
                }
            }
        }
    }
}

/// Marks any alive brick intersecting `rect` as dirty so it gets crisply repainted after erasures.
fn mark_bricks_in_rect(dirty: &mut [bool; GRID_LEN], g: &Game, rect: (i32, i32, i32, i32)) {
    let grid_bottom = crate::game::GRID_Y0 + ROWS as i32 * (BRICK_H + crate::game::BRICK_GAP);
    if rect.1 >= grid_bottom || rect.1 + rect.3 <= crate::game::GRID_Y0 {
        return;
    }
    let r0 = ((rect.1 - crate::game::GRID_Y0) / (BRICK_H + crate::game::BRICK_GAP))
        .clamp(0, ROWS as i32 - 1) as usize;
    let r1 = ((rect.1 + rect.3 - crate::game::GRID_Y0) / (BRICK_H + crate::game::BRICK_GAP))
        .clamp(0, ROWS as i32 - 1) as usize;
    let c0 = ((rect.0 - crate::game::GRID_X0) / (BRICK_W + crate::game::BRICK_GAP))
        .clamp(0, COLS as i32 - 1) as usize;
    let c1 = ((rect.0 + rect.2 - crate::game::GRID_X0) / (BRICK_W + crate::game::BRICK_GAP))
        .clamp(0, COLS as i32 - 1) as usize;
    for r in r0..=r1 {
        for c in c0..=c1 {
            let idx = r * COLS + c;
            if g.bricks[idx].alive() && rects_overlap(rect, brick_rect(r, c)) {
                dirty[idx] = true;
            }
        }
    }
}

/// Differential frame renderer: updates `view` in-place with zero flicker and minimal rects.
pub fn render_frame(s: &mut impl Surface, g: &Game, view: &mut ViewState) {
    if !view.valid || view.state != g.state {
        draw_full(s, g);
        view.sync_from(g);
        return;
    }

    match g.state {
        State::Title => {
            if view.selected_level != g.selected_level {
                draw_title_level_strip(s, g);
                view.selected_level = g.selected_level;
            }
            return;
        }
        State::StageClear | State::GameOver => {
            return;
        }
        State::Playing => {}
    }

    // If stage/wave switched directly while in Playing (e.g. unit test), full redraw.
    if view.level != g.level || view.wave != g.wave {
        draw_full(s, g);
        view.sync_from(g);
        return;
    }

    // 1. Differential HUD updates (only changed sub-regions!)
    if view.score != g.score || view.beat_best != g.beat_best {
        draw_hud_score(s, g.score, g.beat_best);
    }
    if view.combo != g.combo {
        draw_hud_level_combo(s, g.level, g.wave, g.combo);
    }
    if view.lives != g.lives {
        draw_hud_lives(s, g.lives);
    }
    let (new_bk, new_bpx) = quantize_power_bar(g);
    if view.bar_kind != new_bk || view.bar_px != new_bpx {
        draw_hud_power_bar(s, new_bk, new_bpx);
    }
    let new_footer = current_footer(g);
    if view.footer != new_footer {
        draw_footer(s, new_footer);
    }

    let mut dirty_bricks = [false; GRID_LEN];
    let mut paddle_dirty = false;
    let mut shield_dirty = false;

    // 2. Detect bricks whose kind, hp, or flash changed
    for idx in 0..GRID_LEN {
        if view.bricks[idx] != g.bricks[idx] {
            dirty_bricks[idx] = true;
        }
    }

    // 3. Erase old particles
    for i in 0..MAX_PARTICLES {
        let op = view.particles[i];
        if op.active {
            let pr = (op.x as i32 - 1, op.y as i32 - 1, 3, 3);
            erase_arena(s, pr.0, pr.1, pr.2, pr.3);
            mark_bricks_in_rect(&mut dirty_bricks, g, pr);
            if pr.1 + pr.3 >= PADDLE_Y - 6 {
                paddle_dirty = true;
            }
            if pr.1 + pr.3 >= SHIELD_Y - 2 {
                shield_dirty = true;
            }
        }
    }

    // 4. Erase moved/removed lasers
    for i in 0..MAX_LASERS {
        let ol = view.lasers[i];
        let nl = g.lasers[i];
        if ol.active && (!nl.active || ol.x != nl.x || ol.y != nl.y) {
            let lr = (ol.x as i32 - LASER_W / 2, ol.y as i32, LASER_W, LASER_H);
            erase_arena(s, lr.0, lr.1, lr.2, lr.3);
            mark_bricks_in_rect(&mut dirty_bricks, g, lr);
            if lr.1 + lr.3 >= PADDLE_Y - 6 {
                paddle_dirty = true;
            }
        }
    }

    // 5. Erase moved/removed capsules
    for i in 0..MAX_CAPSULES {
        let oc = view.capsules[i];
        let nc = g.capsules[i];
        if oc.active && (!nc.active || oc.x != nc.x || oc.y != nc.py() as i16 || oc.kind != nc.kind) {
            let cr = capsule_bbox(oc.x as i32, oc.y as i32);
            erase_arena(s, cr.0, cr.1, cr.2, cr.3);
            mark_bricks_in_rect(&mut dirty_bricks, g, cr);
            if cr.1 + cr.3 >= PADDLE_Y - 6 {
                paddle_dirty = true;
            }
            if cr.1 + cr.3 >= SHIELD_Y - 2 {
                shield_dirty = true;
            }
        }
    }

    // 6. Erase paddle delta (including laser turret strip & rounded corner cutouts)
    let new_laser = g.laser_ticks > 0;
    let paddle_changed = view.paddle_x as i32 != g.paddle_x
        || view.paddle_w as i32 != g.paddle_w
        || view.paddle_laser != new_laser;
    if paddle_changed {
        let ob = paddle_bbox(view.paddle_x as i32, view.paddle_w as i32);
        let nb = paddle_bbox(g.paddle_x, g.paddle_w);
        if view.paddle_w as i32 == g.paddle_w && view.paddle_laser == new_laser && rects_overlap(ob, nb) {
            // Erase turret band above old paddle if laser turrets were drawn
            if view.paddle_laser {
                erase_arena(s, ob.0, PADDLE_Y - 5, ob.2, 5);
            }
            // Erase uncovered side strip
            if ob.0 < nb.0 {
                erase_arena(s, ob.0, PADDLE_Y, nb.0 - ob.0, PADDLE_H);
            }
            let ob_r = ob.0 + ob.2;
            let nb_r = nb.0 + nb.2;
            if ob_r > nb_r {
                erase_arena(s, nb_r, PADDLE_Y, ob_r - nb_r, PADDLE_H);
            }
            // Clean the tiny 3×2 rounded corner notches of the new paddle
            let nx = g.paddle_x - g.paddle_w / 2;
            erase_arena(s, nx, PADDLE_Y, 3, 2);
            erase_arena(s, nx + g.paddle_w - 3, PADDLE_Y, 3, 2);
            erase_arena(s, nx, PADDLE_Y + PADDLE_H - 2, 3, 2);
            erase_arena(s, nx + g.paddle_w - 3, PADDLE_Y + PADDLE_H - 2, 3, 2);
        } else {
            erase_arena(s, ob.0, ob.1, ob.2, ob.3);
        }
        paddle_dirty = true;
    }

    // 7. Zero-flicker ball update: draw new ball BEFORE erasing `(old \ new)`!
    let fire = g.fire_ticks > 0;
    let new_p_box = paddle_bbox(g.paddle_x, g.paddle_w);
    for i in 0..MAX_BALLS {
        let ob = view.balls[i];
        let nb = g.balls[i];
        if ob.active {
            let obox = ball_bbox(ob.x as i32, ob.y as i32);
            if nb.active {
                let nx = nb.px();
                let ny = nb.py();
                if ob.x as i32 != nx || ob.y as i32 != ny || ob.fire != fire {
                    let nbox = ball_bbox(nx, ny);
                    if rects_overlap(obox, nbox) {
                        // Draw new ball first, then erase ONLY non-overlapping residual strips!
                        draw_ball(s, nx, ny, fire);
                        erase_ball_residual(s, obox, nbox);
                    } else {
                        erase_arena(s, obox.0, obox.1, obox.2, obox.3);
                    }
                    mark_bricks_in_rect(&mut dirty_bricks, g, obox);
                    if rects_overlap(obox, new_p_box) {
                        paddle_dirty = true;
                    }
                    if obox.1 + obox.3 >= SHIELD_Y - 2 {
                        shield_dirty = true;
                    }
                }
            } else {
                erase_arena(s, obox.0, obox.1, obox.2, obox.3);
                mark_bricks_in_rect(&mut dirty_bricks, g, obox);
                if rects_overlap(obox, new_p_box) {
                    paddle_dirty = true;
                }
                if obox.1 + obox.3 >= SHIELD_Y - 2 {
                    shield_dirty = true;
                }
            }
        }
    }

    // 8. Repaint dirty bricks
    for idx in 0..GRID_LEN {
        if dirty_bricks[idx] {
            draw_brick(s, idx / COLS, idx % COLS, g.bricks[idx]);
        }
    }

    // 9. Draw active capsules
    for &c in &g.capsules {
        if c.active {
            let cy = c.py();
            draw_capsule(s, c.x as i32, cy, c.kind);
            if cy + CAP_H / 2 >= PADDLE_Y - 5 {
                paddle_dirty = true;
            }
            if cy + CAP_H / 2 >= SHIELD_Y - 1 {
                shield_dirty = true;
            }
        }
    }

    // 10. Draw active lasers
    for &l in &g.lasers {
        if l.active {
            let ly = l.y as i32;
            draw_laser(s, l.x as i32, ly);
            if ly + LASER_H >= PADDLE_Y - 5 {
                paddle_dirty = true;
            }
        }
    }

    // 11. Draw active particles
    for &p in &g.particles {
        if p.active {
            let py = p.py();
            s.rect(p.px() - 1, py - 1, 3, 3, p.color);
            if py + 2 >= PADDLE_Y - 5 {
                paddle_dirty = true;
            }
            if py + 2 >= SHIELD_Y - 1 {
                shield_dirty = true;
            }
        }
    }

    // 12. Shield update
    let new_shield_on = g.shield_ticks > 0 && g.shield_hits > 0;
    let new_shield_phase = ((g.tick >> 3) & 1) as u8;
    if shield_dirty
        || view.shield_on != new_shield_on
        || (new_shield_on
            && (view.shield_hits != g.shield_hits || (g.shield_hits == 1 && view.shield_phase != new_shield_phase)))
    {
        draw_shield(s, new_shield_on, g.shield_hits, new_shield_phase);
    }

    // 13. Repaint paddle if changed or touched
    if paddle_dirty {
        draw_paddle(s, g.paddle_x, g.paddle_w, new_laser);
    }

    // 14. Ensure all active balls sit cleanly on top of bricks/capsules/lasers/particles!
    for &b in &g.balls {
        if b.active {
            draw_ball(s, b.px(), b.py(), fire);
        }
    }

    view.sync_from(g);
}

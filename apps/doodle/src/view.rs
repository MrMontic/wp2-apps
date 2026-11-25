//! Authentic Doodle Jump notebook-paper renderer with ultra-fast differential updates!
//!
//! Key optimization:
//!   - Classic warm cream graph-paper background (`#F3EEE0` with `#E2DAC5` 28px grid) is drawn ONCE!
//!   - `erase_paper(x, y, w, h)` restores the warm paper + exact 1px grid lines ONLY inside the tiny
//!     changed rectangles — ZERO full-screen clears during gameplay, even while scrolling upward!
//!   - Cute procedural pixel-art Doodler (snout flips left/right/up), 4 platform types, coiled springs,
//!     spinning propeller hat, jetpack with flickering flame, hovering monsters, and shot pellets!

use crate::game::{
    Bullet, C, DOOD_H, DOOD_W, Game, HUD_H, MAX_BULLETS, MAX_PLATS, MON_H, MON_W, Monster, Pickup,
    PLAT_H, PLAT_W, PlatKind, Platform, PowerMode, SIZE, State,
};
use cmf_wp2_ui::Surface;
use cmf_wp2_ui::color::rgb565;
use cmf_wp2_ui::text::{big, big_width, centre, int};

// Soft, eye-friendly classic Doodle Jump notebook palette (warm cream, never glaring white).
const PAPER: u16 = rgb565(240, 235, 220);
const GRID: u16 = rgb565(222, 215, 196);
const HUD_BG: u16 = rgb565(226, 219, 200);
const HUD_LINE: u16 = rgb565(184, 176, 156);
const INK: u16 = rgb565(42, 46, 40);
const INK_DIM: u16 = rgb565(112, 114, 106);

// Platform colors
const GREEN_FILL: u16 = rgb565(94, 182, 54);
const GREEN_HI: u16 = rgb565(138, 214, 96);
const GREEN_EDGE: u16 = rgb565(48, 108, 28);

const BLUE_FILL: u16 = rgb565(64, 156, 224);
const BLUE_HI: u16 = rgb565(124, 198, 246);
const BLUE_EDGE: u16 = rgb565(32, 94, 148);

const BROWN_FILL: u16 = rgb565(158, 108, 66);
const BROWN_HI: u16 = rgb565(192, 144, 100);
const BROWN_EDGE: u16 = rgb565(92, 58, 32);

const WHITE_FILL: u16 = rgb565(234, 238, 244);
const WHITE_HI: u16 = rgb565(252, 252, 255);
const WHITE_EDGE: u16 = rgb565(142, 152, 166);

// Doodler & powerup colors
const DOOD_BODY: u16 = rgb565(132, 204, 62);
const DOOD_SHIRT: u16 = rgb565(64, 128, 42);
const DOOD_OUT: u16 = rgb565(40, 74, 24);
const DOOD_EYE: u16 = rgb565(24, 28, 22);

const SPRING_METAL: u16 = rgb565(108, 114, 122);
const SPRING_TOP: u16 = rgb565(74, 80, 88);

const PROP_RED: u16 = rgb565(218, 68, 56);
const PROP_BLUE: u16 = rgb565(58, 134, 214);
const PROP_YELLOW: u16 = rgb565(238, 194, 52);

const JET_METAL: u16 = rgb565(156, 164, 174);
const JET_DARK: u16 = rgb565(84, 90, 98);
const FLAME_OUT: u16 = rgb565(242, 116, 36);
const FLAME_IN: u16 = rgb565(252, 224, 92);

const MON_A: u16 = rgb565(156, 78, 178);
const MON_A_DARK: u16 = rgb565(92, 42, 108);
const MON_B: u16 = rgb565(214, 84, 84);
const MON_B_DARK: u16 = rgb565(132, 42, 42);
const BULLET_COL: u16 = rgb565(222, 76, 44);
const BULLET_HI: u16 = rgb565(252, 206, 92);

const GRID_STEP: i32 = 28;

/// Fills `[x, y, w, h)` (clipped below HUD) with warm notebook paper + exact 28px grid lines!
fn erase_paper(s: &mut impl Surface, x: i32, y: i32, w: i32, h: i32) {
    let x0 = x.max(0);
    let y0 = y.max(HUD_H);
    let x1 = (x + w).min(SIZE);
    let y1 = (y + h).min(SIZE);
    if x0 >= x1 || y0 >= y1 {
        return;
    }
    s.rect(x0, y0, x1 - x0, y1 - y0, PAPER);

    // Restore horizontal grid lines intersecting [y0, y1)
    let first_gy = ((y0 + GRID_STEP - 1) / GRID_STEP) * GRID_STEP;
    let mut gy = first_gy;
    while gy < y1 {
        s.rect(x0, gy, x1 - x0, 1, GRID);
        gy += GRID_STEP;
    }

    // Restore vertical grid lines intersecting [x0, x1)
    let first_gx = ((x0 + GRID_STEP - 1) / GRID_STEP) * GRID_STEP;
    let mut gx = first_gx;
    while gx < x1 {
        s.rect(gx, y0, 1, y1 - y0, GRID);
        gx += GRID_STEP;
    }
}

/// Draws a classic rounded Doodle Jump platform (+ pickup sitting on top if any).
fn draw_platform(s: &mut impl Surface, p: Platform) {
    let x = p.x();
    let y = p.y();
    if y + PLAT_H <= HUD_H || y - 20 >= SIZE {
        return;
    }

    let (fill, hi, edge) = match p.kind {
        PlatKind::Green => (GREEN_FILL, GREEN_HI, GREEN_EDGE),
        PlatKind::Blue => (BLUE_FILL, BLUE_HI, BLUE_EDGE),
        PlatKind::Brown => (BROWN_FILL, BROWN_HI, BROWN_EDGE),
        PlatKind::White => (WHITE_FILL, WHITE_HI, WHITE_EDGE),
    };

    let clip_rect = |s: &mut dyn Surface, rx: i32, ry: i32, rw: i32, rh: i32, c: u16| {
        let y0 = ry.max(HUD_H);
        let y1 = (ry + rh).min(SIZE);
        if rw > 0 && y0 < y1 {
            s.rect(rx, y0, rw, y1 - y0, c);
        }
    };

    if p.kind == PlatKind::Brown && p.broken > 0 {
        // Snapped in half! Two small angled halves falling apart.
        let spread = (p.broken as i32 * 2).min(14);
        let hw = PLAT_W / 2 - 3;
        clip_rect(s, x - spread, y, hw, PLAT_H - 4, BROWN_FILL);
        clip_rect(s, x - spread, y, hw, 2, BROWN_EDGE);
        clip_rect(s, x + PLAT_W - hw + spread, y + 3, hw, PLAT_H - 4, BROWN_FILL);
        clip_rect(s, x + PLAT_W - hw + spread, y + 3, hw, 2, BROWN_EDGE);
        return;
    }

    // Crisp rounded pill silhouette (14px tall)
    clip_rect(s, x + 4, y, PLAT_W - 8, 2, edge);
    clip_rect(s, x + 2, y + 2, PLAT_W - 4, 2, hi);
    clip_rect(s, x, y + 4, PLAT_W, PLAT_H - 8, fill);
    clip_rect(s, x, y + 4, 2, PLAT_H - 8, edge);
    clip_rect(s, x + PLAT_W - 2, y + 4, 2, PLAT_H - 8, edge);
    clip_rect(s, x + 2, y + PLAT_H - 4, PLAT_W - 4, 2, fill);
    clip_rect(s, x + 4, y + PLAT_H - 2, PLAT_W - 8, 2, edge);

    // Classic dark crack on intact Brown platform
    if p.kind == PlatKind::Brown {
        let mx = x + PLAT_W / 2;
        clip_rect(s, mx - 2, y + 2, 2, 4, BROWN_EDGE);
        clip_rect(s, mx, y + 5, 2, 4, BROWN_EDGE);
        clip_rect(s, mx - 1, y + 8, 2, 4, BROWN_EDGE);
    }

    // Pickup sitting cleanly on top of the platform
    let cx = x + PLAT_W / 2;
    match p.pickup {
        Pickup::None => {}
        Pickup::Spring => {
            // Compact coiled spring (14x9)
            clip_rect(s, cx - 7, y - 9, 14, 3, SPRING_TOP);
            clip_rect(s, cx - 5, y - 6, 10, 2, SPRING_METAL);
            clip_rect(s, cx - 3, y - 4, 6, 2, SPRING_TOP);
            clip_rect(s, cx - 5, y - 2, 10, 2, SPRING_METAL);
        }
        Pickup::SpringUsed => {
            // Stretched spring (14x15)
            clip_rect(s, cx - 7, y - 15, 14, 3, SPRING_TOP);
            clip_rect(s, cx - 4, y - 12, 8, 2, SPRING_METAL);
            clip_rect(s, cx - 3, y - 9, 6, 2, SPRING_METAL);
            clip_rect(s, cx - 4, y - 6, 8, 2, SPRING_METAL);
            clip_rect(s, cx - 5, y - 3, 10, 3, SPRING_TOP);
        }
        Pickup::Propeller => {
            // Cute propeller hat pickup (18x12)
            clip_rect(s, cx - 9, y - 12, 18, 2, PROP_YELLOW);
            clip_rect(s, cx - 1, y - 10, 2, 3, SPRING_TOP);
            clip_rect(s, cx - 7, y - 7, 14, 7, PROP_BLUE);
            clip_rect(s, cx - 3, y - 7, 6, 7, PROP_RED);
        }
        Pickup::Jetpack => {
            // Iconic silver/orange jetpack pickup (16x18)
            clip_rect(s, cx - 8, y - 17, 7, 15, JET_METAL);
            clip_rect(s, cx + 1, y - 17, 7, 15, JET_METAL);
            clip_rect(s, cx - 8, y - 14, 16, 3, PROP_RED);
            clip_rect(s, cx - 7, y - 2, 5, 2, JET_DARK);
            clip_rect(s, cx + 2, y - 2, 5, 2, JET_DARK);
        }
    }
}

/// Bounding box `[x, y, w, h]` of a platform (including pickup or broken spread).
fn plat_bbox(p: Platform) -> (i32, i32, i32, i32) {
    let x = p.x();
    let y = p.y();
    if p.kind == PlatKind::Brown && p.broken > 0 {
        let sp = (p.broken as i32 * 2).min(14);
        (x - sp, y, PLAT_W + sp * 2, PLAT_H + 4)
    } else {
        let top_ext = match p.pickup {
            Pickup::None => 0,
            Pickup::Spring => 10,
            Pickup::SpringUsed => 16,
            Pickup::Propeller => 13,
            Pickup::Jetpack => 18,
        };
        (x, y - top_ext, PLAT_W, PLAT_H + top_ext)
    }
}

/// Draws our cute classic green Doodler (`cx` = center X, `by` = bottom feet Y).
fn draw_doodler(
    s: &mut impl Surface,
    cx: i32,
    by: i32,
    facing_right: bool,
    shooting: bool,
    power: PowerMode,
    tick: u32,
) {
    let x0 = cx - DOOD_W / 2;
    let y0 = by - DOOD_H;

    let clip = |s: &mut dyn Surface, rx: i32, ry: i32, rw: i32, rh: i32, c: u16| {
        let cy0 = ry.max(HUD_H);
        let cy1 = (ry + rh).min(SIZE);
        if rw > 0 && cy0 < cy1 {
            s.rect(rx, cy0, rw, cy1 - cy0, c);
        }
    };

    // Jetpack strapped behind Doodler + flickering flame!
    if matches!(power, PowerMode::Jetpack(_)) {
        let jx = if facing_right { x0 - 6 } else { x0 + DOOD_W - 2 };
        clip(s, jx, y0 + 6, 8, 15, JET_METAL);
        clip(s, jx, y0 + 10, 8, 3, PROP_RED);
        clip(s, jx + 1, y0 + 21, 6, 2, JET_DARK);
        let fl_h = if tick & 1 == 0 { 11 } else { 7 };
        clip(s, jx + 1, y0 + 23, 6, fl_h, FLAME_OUT);
        clip(s, jx + 2, y0 + 23, 4, fl_h - 3, FLAME_IN);
    }

    // Rounded green head & torso
    clip(s, x0 + 4, y0, DOOD_W - 8, 2, DOOD_OUT);
    clip(s, x0 + 2, y0 + 2, DOOD_W - 4, 12, DOOD_BODY);
    clip(s, x0 + 1, y0 + 4, 2, 16, DOOD_OUT);
    clip(s, x0 + DOOD_W - 3, y0 + 4, 2, 16, DOOD_OUT);
    // Striped dark-green shirt on lower torso
    clip(s, x0 + 2, y0 + 14, DOOD_W - 4, 6, DOOD_SHIRT);
    clip(s, x0 + 3, y0 + 16, DOOD_W - 6, 2, DOOD_BODY);
    clip(s, x0 + 2, y0 + 20, DOOD_W - 4, 2, DOOD_OUT);

    // 4 cute little legs (like classic Doodler!)
    clip(s, x0 + 4, y0 + 22, 2, 4, DOOD_OUT);
    clip(s, x0 + 9, y0 + 22, 2, 4, DOOD_OUT);
    clip(s, x0 + 13, y0 + 22, 2, 4, DOOD_OUT);
    clip(s, x0 + 18, y0 + 22, 2, 4, DOOD_OUT);

    // Snout & eyes (facing up when shooting, else left/right!)
    if shooting {
        clip(s, cx - 3, y0 - 7, 6, 7, DOOD_BODY);
        clip(s, cx - 4, y0 - 7, 8, 2, DOOD_OUT);
        clip(s, cx - 5, y0 + 5, 3, 3, DOOD_EYE);
        clip(s, cx + 2, y0 + 5, 3, 3, DOOD_EYE);
    } else if facing_right {
        // Snout sticking right
        clip(s, x0 + DOOD_W - 2, y0 + 6, 7, 5, DOOD_BODY);
        clip(s, x0 + DOOD_W + 4, y0 + 6, 2, 5, DOOD_OUT);
        clip(s, x0 + DOOD_W - 2, y0 + 5, 7, 1, DOOD_OUT);
        clip(s, x0 + DOOD_W - 2, y0 + 11, 7, 1, DOOD_OUT);
        // Eyes looking right
        clip(s, x0 + 11, y0 + 5, 3, 3, DOOD_EYE);
        clip(s, x0 + 16, y0 + 5, 3, 3, DOOD_EYE);
    } else {
        // Snout sticking left
        clip(s, x0 - 5, y0 + 6, 7, 5, DOOD_BODY);
        clip(s, x0 - 6, y0 + 6, 2, 5, DOOD_OUT);
        clip(s, x0 - 5, y0 + 5, 7, 1, DOOD_OUT);
        clip(s, x0 - 5, y0 + 11, 7, 1, DOOD_OUT);
        // Eyes looking left
        clip(s, x0 + 5, y0 + 5, 3, 3, DOOD_EYE);
        clip(s, x0 + 10, y0 + 5, 3, 3, DOOD_EYE);
    }

    // Spinning Propeller Hat on top of Doodler's head!
    if matches!(power, PowerMode::Propeller(_)) {
        clip(s, cx - 7, y0 - 6, 14, 6, PROP_BLUE);
        clip(s, cx - 3, y0 - 6, 6, 6, PROP_RED);
        clip(s, cx - 1, y0 - 9, 2, 3, SPRING_TOP);
        let blade_w = if (tick >> 1) & 1 == 0 { 22 } else { 10 };
        clip(s, cx - blade_w / 2, y0 - 11, blade_w, 2, PROP_YELLOW);
    }
}

/// Generous bounding box covering Doodler + snout + propeller + jetpack flame.
fn dood_bbox(cx: i32, by: i32) -> (i32, i32, i32, i32) {
    (cx - DOOD_W / 2 - 8, by - DOOD_H - 12, DOOD_W + 16, DOOD_H + 22)
}

fn draw_monster(s: &mut impl Surface, m: Monster, tick: u32) {
    let x = m.x();
    let y = m.y();
    if y + MON_H <= HUD_H || y >= SIZE {
        return;
    }
    let (body, dark) = if m.variant == 0 { (MON_A, MON_A_DARK) } else { (MON_B, MON_B_DARK) };
    let clip = |s: &mut dyn Surface, rx: i32, ry: i32, rw: i32, rh: i32, c: u16| {
        let cy0 = ry.max(HUD_H);
        let cy1 = (ry + rh).min(SIZE);
        if rw > 0 && cy0 < cy1 {
            s.rect(rx, cy0, rw, cy1 - cy0, c);
        }
    };
    let bob = if (tick >> 2) & 1 == 0 { 0 } else { 1 };
    // Little antennae
    clip(s, x + 6, y - 3 + bob, 2, 4, dark);
    clip(s, x + MON_W - 8, y - 3 + bob, 2, 4, dark);
    // Body
    clip(s, x + 3, y + bob, MON_W - 6, 2, dark);
    clip(s, x, y + 2 + bob, MON_W, MON_H - 6, body);
    clip(s, x + 2, y + MON_H - 4 + bob, MON_W - 4, 3, dark);
    // Cute wide white eyes + dark pupils
    clip(s, x + 5, y + 6 + bob, 6, 6, WHITE_HI);
    clip(s, x + MON_W - 11, y + 6 + bob, 6, 6, WHITE_HI);
    clip(s, x + 7, y + 8 + bob, 3, 3, DOOD_EYE);
    clip(s, x + MON_W - 9, y + 8 + bob, 3, 3, DOOD_EYE);
}

fn mon_bbox(m: Monster) -> (i32, i32, i32, i32) {
    (m.x(), m.y() - 4, MON_W, MON_H + 6)
}

fn draw_bullet(s: &mut impl Surface, b: Bullet) {
    if b.y - 4 < HUD_H || b.y + 4 >= SIZE {
        return;
    }
    s.rect(b.x - 3, b.y - 3, 7, 7, BULLET_COL);
    s.rect(b.x - 1, b.y - 1, 3, 3, BULLET_HI);
}

fn draw_hud(s: &mut impl Surface, score: u32, best: u32, beat_best: bool) {
    s.rect(0, 0, SIZE, HUD_H - 2, HUD_BG);
    s.rect(0, HUD_H - 2, SIZE, 2, HUD_LINE);
    let mut buf = [0u8; 11];
    let text = int(score as i32, &mut buf);
    let w = big_width(text, 3);
    big(s, C - w / 2, 24, text, 3, INK);
    if beat_best {
        centre(s, C, 48, "NEW BEST", GREEN_EDGE);
    } else if best > 0 {
        let mut b = [0u8; 18];
        b[..5].copy_from_slice(b"BEST ");
        let mut d = [0u8; 11];
        let ds = int(best.min(9_999_999) as i32, &mut d).as_bytes();
        b[5..5 + ds.len()].copy_from_slice(ds);
        if let Ok(lbl) = core::str::from_utf8(&b[..5 + ds.len()]) {
            centre(s, C, 48, lbl, INK_DIM);
        }
    }
}

fn best_line(s: &mut impl Surface, y: i32, best: u32, col: u16) {
    if best == 0 {
        return;
    }
    let mut buf = [0u8; 18];
    buf[..5].copy_from_slice(b"BEST ");
    let mut digits = [0u8; 11];
    let d = int(best.min(9_999_999) as i32, &mut digits).as_bytes();
    buf[5..5 + d.len()].copy_from_slice(d);
    if let Ok(label) = core::str::from_utf8(&buf[..5 + d.len()]) {
        centre(s, C, y, label, col);
    }
}

/// Full screen redraw (startup, state transitions, RESUME).
pub fn full(s: &mut impl Surface, g: &Game, best: u32) {
    // Full graph-paper notebook background
    s.rect(0, 0, SIZE, HUD_H, PAPER);
    let mut gy = 0;
    while gy < HUD_H {
        s.rect(0, gy, SIZE, 1, GRID);
        gy += GRID_STEP;
    }
    let mut gx = 0;
    while gx < SIZE {
        s.rect(gx, 0, 1, HUD_H, GRID);
        gx += GRID_STEP;
    }
    erase_paper(s, 0, HUD_H, SIZE, SIZE - HUD_H);

    match g.state {
        State::Ready => {
            let t1 = "DOODLE";
            let t2 = "JUMP";
            big(s, C - big_width(t1, 4) / 2, 74, t1, 4, GREEN_EDGE);
            big(s, C - big_width(t2, 4) / 2, 114, t2, 4, INK);
            centre(s, C, 168, "CROWN TO MOVE", INK_DIM);
            centre(s, C, 192, "TAP = PLAY / SHOOT", INK);
            best_line(s, 224, best, GREEN_EDGE);

            // Live green home platform + bouncing Doodler!
            draw_platform(s, g.plats[0]);
            draw_doodler(s, g.dood_x(), g.dood_y(), g.facing_right, false, PowerMode::None, g.tick);
        }
        State::Playing => {
            draw_hud(s, g.score, best, g.beat_best);
            for &p in &g.plats {
                if p.active {
                    draw_platform(s, p);
                }
            }
            if g.monster.active {
                draw_monster(s, g.monster, g.tick);
            }
            for &b in &g.bullets {
                if b.active {
                    draw_bullet(s, b);
                }
            }
            draw_doodler(
                s,
                g.dood_x(),
                g.dood_y(),
                g.facing_right,
                g.shoot_pose > 0,
                g.power,
                g.tick,
            );
        }
        State::Over => {
            if g.beat_best || (g.score > 0 && g.score == best) {
                centre(s, C, 116, "NEW BEST", GREEN_EDGE);
            } else {
                centre(s, C, 116, "GAME OVER", INK_DIM);
            }
            let mut buf = [0u8; 11];
            let text = int(g.score as i32, &mut buf);
            big(s, C - big_width(text, 6) / 2, 158, text, 6, INK);
            best_line(s, 240, best, INK_DIM);
            centre(s, C, 286, "TAP TO RETRY", GREEN_EDGE);
            // Decorative classic green platform at bottom
            draw_platform(
                s,
                Platform {
                    active: true,
                    x_q: (C - PLAT_W / 2) * crate::game::Q,
                    y_q: 356 * crate::game::Q,
                    vx_q: 0,
                    kind: PlatKind::Green,
                    pickup: Pickup::Spring,
                    broken: 0,
                },
            );
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Seen {
    state: State,
    dood_x: i32,
    dood_y: i32,
    facing_right: bool,
    shooting: bool,
    power: PowerMode,
    plats: [Platform; MAX_PLATS],
    monster: Monster,
    bullets: [Bullet; MAX_BULLETS],
    score: u32,
    beat_best: bool,
    anim_phase: u8,
}

fn snapshot(g: &Game) -> Seen {
    Seen {
        state: g.state,
        dood_x: g.dood_x(),
        dood_y: g.dood_y(),
        facing_right: g.facing_right,
        shooting: g.shoot_pose > 0,
        power: g.power,
        plats: g.plats,
        monster: g.monster,
        bullets: g.bullets,
        score: g.score,
        beat_best: g.beat_best,
        anim_phase: (g.tick & 3) as u8,
    }
}

#[inline]
fn rects_overlap(a: (i32, i32, i32, i32), b: (i32, i32, i32, i32)) -> bool {
    a.0 < b.0 + b.2 && a.0 + a.2 > b.0 && a.1 < b.1 + b.3 && a.1 + a.3 > b.1
}

/// Differential frame update: erases ONLY moved objects back to graph paper and repaints them!
/// Zero full-screen clears during gameplay -> smooth ~55 FPS!
pub fn frame(s: &mut impl Surface, g: &Game, best: u32, last: Option<Seen>) -> Seen {
    let now = snapshot(g);
    let Some(old) = last.filter(|l| l.state == now.state) else {
        full(s, g, best);
        return now;
    };

    if g.state == State::Over {
        return now;
    }

    if g.state == State::Ready {
        // Only the cute bouncing Doodler moves on Ready screen!
        let (ox, oy, ow, oh) = dood_bbox(old.dood_x, old.dood_y);
        erase_paper(s, ox, oy, ow, oh);
        draw_platform(s, g.plats[0]);
        draw_doodler(s, now.dood_x, now.dood_y, now.facing_right, false, PowerMode::None, g.tick);
        return now;
    }

    // 1. HUD updates only when score or beat_best changed.
    if old.score != now.score || old.beat_best != now.beat_best {
        draw_hud(s, now.score, best, now.beat_best);
    }

    // Track which old boxes were erased so any stationary overlapping platform gets cleanly redrawn.
    let mut dirty_plats = [false; MAX_PLATS];
    let old_d_box = dood_bbox(old.dood_x, old.dood_y);
    let new_d_box = dood_bbox(now.dood_x, now.dood_y);
    let dood_changed = old.dood_x != now.dood_x
        || old.dood_y != now.dood_y
        || old.facing_right != now.facing_right
        || old.shooting != now.shooting
        || old.power != now.power
        || (old.power != PowerMode::None && old.anim_phase != now.anim_phase);

    // 2. Erase moved/removed platforms
    for i in 0..MAX_PLATS {
        let op = old.plats[i];
        let np = now.plats[i];
        let changed = op.active != np.active
            || (op.active
                && (op.x() != np.x()
                    || op.y() != np.y()
                    || op.kind != np.kind
                    || op.pickup != np.pickup
                    || op.broken != np.broken));
        if changed {
            if op.active {
                let (bx, by, bw, bh) = plat_bbox(op);
                erase_paper(s, bx, by, bw, bh);
                // Mark any active platform overlapping this erased rect for repaint
                for j in 0..MAX_PLATS {
                    if now.plats[j].active && rects_overlap((bx, by, bw, bh), plat_bbox(now.plats[j])) {
                        dirty_plats[j] = true;
                    }
                }
            }
            if np.active {
                dirty_plats[i] = true;
            }
        }
    }

    // 3. Erase moved/removed monster
    let mon_changed = old.monster.active != now.monster.active
        || (old.monster.active
            && (old.monster.x() != now.monster.x()
                || old.monster.y() != now.monster.y()
                || (old.anim_phase >> 2) != (now.anim_phase >> 2)));
    if mon_changed && old.monster.active {
        let mb = mon_bbox(old.monster);
        erase_paper(s, mb.0, mb.1, mb.2, mb.3);
        for j in 0..MAX_PLATS {
            if now.plats[j].active && rects_overlap(mb, plat_bbox(now.plats[j])) {
                dirty_plats[j] = true;
            }
        }
    }

    // 4. Erase moved/removed bullets
    for i in 0..MAX_BULLETS {
        let ob = old.bullets[i];
        let nb = now.bullets[i];
        if ob.active && (!nb.active || ob.x != nb.x || ob.y != nb.y) {
            let bb = (ob.x - 3, ob.y - 3, 7, 7);
            erase_paper(s, bb.0, bb.1, bb.2, bb.3);
            for j in 0..MAX_PLATS {
                if now.plats[j].active && rects_overlap(bb, plat_bbox(now.plats[j])) {
                    dirty_plats[j] = true;
                }
            }
        }
    }

    // 5. Erase old Doodler box and mark any platforms under it for repaint
    if dood_changed {
        erase_paper(s, old_d_box.0, old_d_box.1, old_d_box.2, old_d_box.3);
        for j in 0..MAX_PLATS {
            if now.plats[j].active && rects_overlap(old_d_box, plat_bbox(now.plats[j])) {
                dirty_plats[j] = true;
            }
        }
    }

    // 6. Repaint dirty platforms
    for i in 0..MAX_PLATS {
        if dirty_plats[i] && now.plats[i].active {
            draw_platform(s, now.plats[i]);
        }
    }

    // 7. Repaint monster if active & changed (or overlapped by erased Doodler/platform)
    if now.monster.active {
        let mb = mon_bbox(now.monster);
        let touched = mon_changed || (dood_changed && rects_overlap(old_d_box, mb));
        if touched {
            draw_monster(s, now.monster, g.tick);
        }
    }

    // 8. Repaint active bullets
    for &b in &now.bullets {
        if b.active {
            draw_bullet(s, b);
        }
    }

    // 9. Draw Doodler on top!
    let dood_touched_by_plat = dirty_plats
        .iter()
        .enumerate()
        .any(|(i, &d)| d && now.plats[i].active && rects_overlap(new_d_box, plat_bbox(now.plats[i])));
    if dood_changed || dood_touched_by_plat || mon_changed {
        draw_doodler(
            s,
            now.dood_x,
            now.dood_y,
            now.facing_right,
            now.shooting,
            now.power,
            g.tick,
        );
    }

    now
}

#[cfg(test)]
mod tests {
    use super::*;
    use cmf_wp2_ui::{Count, Fb};

    const N: usize = (SIZE * SIZE) as usize;

    fn render_full(g: &Game, best: u32) -> Vec<u16> {
        let mut px = vec![0x1234u16; N];
        full(&mut Fb::new(&mut px), g, best);
        px
    }

    #[test]
    fn stationary_and_scrolling_diff_matches_full() {
        let mut g = Game::new(42);
        g.start(500);
        let mut px = vec![0x5678u16; N];
        let mut seen = Some(frame(&mut Fb::new(&mut px), &g, 500, None));

        for i in 0..70 {
            if i == 8 { g.apply_crown(2); }
            if i == 15 { g.shoot(); }
            if i == 25 { g.apply_crown(-3); }
            let _ = g.step();
            seen = Some(frame(&mut Fb::new(&mut px), &g, 500, seen));
            if g.state != State::Playing {
                break;
            }
            let expected = render_full(&g, 500);
            let mismatches = px.iter().zip(expected.iter()).filter(|(a, b)| a != b).count();
            assert_eq!(mismatches, 0, "pixel mismatch at step {i}");
        }
    }

    #[test]
    fn stationary_frame_is_super_fast() {
        let mut g = Game::new(7);
        g.start(0);
        let mut dummy = Count::default();
        let seen = Some(frame(&mut dummy, &g, 0, None));
        let _ = g.step();
        let mut cost = Count::default();
        frame(&mut cost, &g, 0, seen);
        assert!(cost.px < 3500, "stationary frame too heavy: {} px ({} rects)", cost.px, cost.rects);
        std::println!("1-tick stationary doodle frame cost: {} rects, {} px", cost.rects, cost.px);
    }

    #[test]
    #[ignore]
    fn dump_previews() {
        let out_dir = std::path::Path::new(r"C:\Users\rusya\.gemini\antigravity\brain\fb8755fe-2686-4950-a4ae-0546bbe7b103\scratch");
        let _ = std::fs::create_dir_all(out_dir);
        let save = |name: &str, px: &[u16]| {
            let mut bytes = Vec::with_capacity(px.len() * 2);
            for &c in px {
                bytes.extend_from_slice(&c.to_le_bytes());
            }
            std::fs::write(out_dir.join(name), bytes).unwrap();
        };

        // 1. Ready screen
        let mut g = Game::new(42);
        save("doodle_ready.rgb565", &render_full(&g, 1420));

        // 2. Action-packed Playing screen (Green, Blue, Brown, White, Spring, Propeller, Monster, Jetpack Doodler!)
        g.start(1420);
        g.score = 680;
        g.plats = [Platform::EMPTY; MAX_PLATS];
        g.plats[0] = Platform { active: true, x_q: 110 * crate::game::Q, y_q: 375 * crate::game::Q, vx_q: 0, kind: PlatKind::Green, pickup: Pickup::None, broken: 0 };
        g.plats[1] = Platform { active: true, x_q: 260 * crate::game::Q, y_q: 328 * crate::game::Q, vx_q: 0, kind: PlatKind::Green, pickup: Pickup::Spring, broken: 0 };
        g.plats[2] = Platform { active: true, x_q: 145 * crate::game::Q, y_q: 274 * crate::game::Q, vx_q: 0, kind: PlatKind::Blue, pickup: Pickup::None, broken: 0 };
        g.plats[3] = Platform { active: true, x_q: 280 * crate::game::Q, y_q: 226 * crate::game::Q, vx_q: 0, kind: PlatKind::Brown, pickup: Pickup::None, broken: 0 };
        g.plats[4] = Platform { active: true, x_q: 120 * crate::game::Q, y_q: 170 * crate::game::Q, vx_q: 0, kind: PlatKind::Green, pickup: Pickup::Propeller, broken: 0 };
        g.plats[5] = Platform { active: true, x_q: 248 * crate::game::Q, y_q: 118 * crate::game::Q, vx_q: 0, kind: PlatKind::White, pickup: Pickup::None, broken: 0 };
        g.monster = Monster { active: true, x_q: 295 * crate::game::Q, y_q: 145 * crate::game::Q, vx_q: 0, variant: 0 };
        g.x_q = 205 * crate::game::Q;
        g.y_q = 242 * crate::game::Q;
        g.power = PowerMode::Jetpack(50);
        save("doodle_playing.rgb565", &render_full(&g, 1420));

        // 3. Game over screen
        g.state = State::Over;
        g.score = 1680;
        g.beat_best = true;
        save("doodle_gameover.rgb565", &render_full(&g, 1680));
    }
}

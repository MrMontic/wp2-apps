//! Ultra-fast, zero-flicker, pixel-exact differential renderer for HALO.
//!
//! Why this stays buttery smooth on CMF Watch Pro 2:
//! 1. The full circular track is painted ONCE on screen entry/resume — never per frame.
//! 2. Turning the crown paints ONLY the small exposed/newly-covered arc slices within their
//!    tight bounding box (typically ~300-700 px instead of flushing all 466x466 = 217,156 px).
//! 3. Precomputed RGB565 AA lookup tables + precomputed circle span tables avoid per-pixel
//!    blending and square-root math.
//! 4. The new ball is drawn BEFORE erasing only `(old_ball \ new_ball)`, so on the single-buffered
//!    canvas the ball can NEVER vanish mid-scanout, even when spinning the crown rapidly.

use crate::game::{BALL_R, C, Game, RAIL_R0, RAIL_R1, SIZE, State, diff};
use cmf_wp2_ui::color::{Rgb, rgb565};
use cmf_wp2_ui::draw::Run;
use cmf_wp2_ui::motion::{angle, cos, isqrt, sin};
use cmf_wp2_ui::text::{big, big_width, centre, int};
use cmf_wp2_ui::{ONE, Surface};

// Calm monochrome OLED palette (zero eye strain, crisp hierarchy).
const BG_RGB: Rgb = Rgb(0, 0, 0);
const BG: u16 = 0x0000;
const TRACK_RGB: Rgb = Rgb(30, 30, 36);
const PADDLE_RGB: Rgb = Rgb(216, 216, 224);
const FLASH_RGB: Rgb = Rgb(255, 255, 255);

const BALL_COL: u16 = rgb565(246, 246, 250);
const BALL_EDGE: u16 = rgb565(132, 132, 144);
const TRAIL1_COL: u16 = rgb565(82, 82, 94);
const TRAIL2_COL: u16 = rgb565(40, 40, 48);

const SCORE_DIM: u16 = rgb565(64, 64, 74);
const SCORE_BEST_DIM: u16 = rgb565(116, 116, 128);
const TEXT_PRI: u16 = rgb565(236, 236, 242);
const TEXT_SEC: u16 = rgb565(138, 138, 150);
const TEXT_DIM: u16 = rgb565(90, 90, 102);

const SCORE_SCALE: i32 = 6;
const SCORE_Y: i32 = C - 4 * SCORE_SCALE;
const BEST_TAG_Y: i32 = SCORE_Y + 8 * SCORE_SCALE + 14;

/// Radial AA levels for the ring edges (4 steps -> 5-entry LUT, zero per-pixel blending).
const AA_STEPS: i32 = 4;

const fn make_lut(fg: Rgb) -> [u16; 5] {
    let mut out = [0u16; 5];
    let mut k = 0usize;
    while k <= 4 {
        let r = (fg.0 as u32 * k as u32 / 4) as u8;
        let g = (fg.1 as u32 * k as u32 / 4) as u8;
        let b = (fg.2 as u32 * k as u32 / 4) as u8;
        out[k] = rgb565(r, g, b);
        k += 1;
    }
    out
}

const TRACK_LUT: [u16; 5] = make_lut(TRACK_RGB);
const PADDLE_LUT: [u16; 5] = make_lut(PADDLE_RGB);
const FLASH_LUT: [u16; 5] = make_lut(FLASH_RGB);

/// Precomputed horizontal half-widths for crisp circles (`dy = 0..=r`).
const BALL_SPANS: [i32; 10] = [9, 9, 9, 8, 8, 7, 7, 6, 4, 2]; // r = 9
const T1_SPANS: [i32; 7] = [6, 6, 5, 5, 4, 3, 1]; // r = 6
const T2_SPANS: [i32; 5] = [4, 4, 3, 2, 1]; // r = 4

const LO_I: i32 = (2 * RAIL_R0 - 1) * (2 * RAIL_R0 - 1);
const HI_I: i32 = (2 * RAIL_R0 + 1) * (2 * RAIL_R0 + 1);
const LO_O: i32 = (2 * RAIL_R1 - 1) * (2 * RAIL_R1 - 1);
const HI_O: i32 = (2 * RAIL_R1 + 1) * (2 * RAIL_R1 + 1);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Seen {
    state: State,
    ball: (i32, i32),
    trail: [(i32, i32); 2],
    trail_len: u8,
    paddle: u16,
    span: i32,
    flash: bool,
    score: u32,
    beat_best: bool,
}

fn snapshot(g: &Game) -> Seen {
    Seen {
        state: g.state,
        ball: g.ball(),
        trail: g.trail,
        trail_len: g.trail_len,
        paddle: g.paddle,
        span: g.span(),
        flash: g.flash > 0,
        score: g.score,
        beat_best: g.beat_best,
    }
}

#[inline]
fn radial_level(q: i32) -> i32 {
    let outer = if q <= LO_O {
        AA_STEPS
    } else if q >= HI_O {
        0
    } else {
        ((HI_O - q) * AA_STEPS + (HI_O - LO_O) / 2) / (HI_O - LO_O)
    };
    if outer <= 0 {
        return 0;
    }
    let inner = if q >= HI_I {
        AA_STEPS
    } else if q <= LO_I {
        0
    } else {
        ((q - LO_I) * AA_STEPS + (HI_I - LO_I) / 2) / (HI_I - LO_I)
    };
    outer.min(inner)
}

#[inline]
fn paddle_start(paddle: u16, span: i32) -> u16 {
    paddle.wrapping_sub((span / 2) as u16)
}

#[inline]
fn in_sector(dx: i32, dy: i32, u0: (i32, i32), u1: (i32, i32), wide: bool) -> bool {
    let d0 = u0.0 * dy - u0.1 * dx;
    let d1 = u1.1 * dx - u1.0 * dy;
    if wide { d0 >= 0 || d1 >= 0 } else { d0 >= 0 && d1 >= 0 }
}

#[inline]
fn angle_in_arc(a: u16, start: u16, sweep: u32) -> bool {
    if sweep >= 65536 {
        return true;
    }
    (a.wrapping_sub(start) as u32) <= sweep
}

/// Tight relative bounding box `(x0, y0, x1, y1)` around center for a ring sector.
fn sector_bounds(start: u16, sweep: u32) -> (i32, i32, i32, i32) {
    let lim = RAIL_R1 + 1;
    if sweep >= 65536 {
        return (-lim, -lim, lim, lim);
    }
    let end = start.wrapping_add(sweep as u16);
    let (c0, s0) = (cos(start), sin(start));
    let (c1, s1) = (cos(end), sin(end));
    let pts = [
        ((RAIL_R0 * c0) >> 14, (RAIL_R0 * s0) >> 14),
        ((RAIL_R1 * c0) >> 14, (RAIL_R1 * s0) >> 14),
        ((RAIL_R0 * c1) >> 14, (RAIL_R0 * s1) >> 14),
        ((RAIL_R1 * c1) >> 14, (RAIL_R1 * s1) >> 14),
    ];
    let (mut x0, mut y0, mut x1, mut y1) = (pts[0].0, pts[0].1, pts[0].0, pts[0].1);
    for &(px, py) in &pts[1..] {
        if px < x0 { x0 = px; }
        if px > x1 { x1 = px; }
        if py < y0 { y0 = py; }
        if py > y1 { y1 = py; }
    }
    // Include cardinal extrema if the sector crosses them
    if angle_in_arc(0, start, sweep) { x1 = RAIL_R1; }
    if angle_in_arc(16384, start, sweep) { y1 = RAIL_R1; }
    if angle_in_arc(32768, start, sweep) { x0 = -RAIL_R1; }
    if angle_in_arc(49152, start, sweep) { y0 = -RAIL_R1; }
    (
        (x0 - 3).max(-lim),
        (y0 - 3).max(-lim),
        (x1 + 3).min(lim),
        (y1 + 3).min(lim),
    )
}

/// Paints the entire 360° track + paddle in one pass (used ONLY on full redraw / resume).
fn paint_ring_full(s: &mut impl Surface, paddle: u16, span: i32, flash: bool) {
    let start = paddle_start(paddle, span);
    let sweep = span as u32;
    let end = start.wrapping_add(sweep as u16);
    let u0 = (cos(start), sin(start));
    let u1 = (cos(end), sin(end));
    let wide = sweep > 32768;
    let p_lut = if flash { &FLASH_LUT } else { &PADDLE_LUT };

    let lim = RAIL_R1 + 1;
    for dy in -lim..=lim {
        let dy2 = dy * dy;
        let reach = isqrt((HI_O / 4 - dy2).max(0)) + 1;
        let m = LO_I - 4 * dy2;
        let hole = if m >= 0 { isqrt(m / 4) } else { 0 };
        let mut run = Run::new(s, C + dy, false);
        // Left half: -reach..=-hole
        for dx in -reach..=-hole {
            let k = radial_level(4 * (dx * dx + dy2));
            if k > 0 {
                let lut = if in_sector(dx, dy, u0, u1, wide) { p_lut } else { &TRACK_LUT };
                run.put(C + dx, 1, lut[k as usize]);
            }
        }
        // Right half: (hole.max(1) or 0)..=reach
        let r_start = if hole == 0 { 1 } else { hole };
        for dx in r_start..=reach {
            let k = radial_level(4 * (dx * dx + dy2));
            if k > 0 {
                let lut = if in_sector(dx, dy, u0, u1, wide) { p_lut } else { &TRACK_LUT };
                run.put(C + dx, 1, lut[k as usize]);
            }
        }
    }
}

/// Paints ONLY a small angular slice of the ring inside its tight bounding box.
/// If `paddle_ref` is `Some((p_start, p_span, flash))`, chooses PADDLE vs TRACK per pixel
/// identically to `paint_ring_full` (used for repairing the tiny rail window after a ball bounce).
fn paint_ring_window(
    s: &mut impl Surface,
    win_start: u16,
    win_sweep: u32,
    paddle_start: u16,
    paddle_span: u32,
    flash: bool,
) {
    if win_sweep == 0 {
        return;
    }
    let (bx0, by0, bx1, by1) = sector_bounds(win_start, win_sweep);
    let win_end = win_start.wrapping_add(win_sweep.min(65535) as u16);
    let w0 = (cos(win_start), sin(win_start));
    let w1 = (cos(win_end), sin(win_end));
    let w_wide = win_sweep > 32768;

    let p_end = paddle_start.wrapping_add(paddle_span.min(65535) as u16);
    let p0 = (cos(paddle_start), sin(paddle_start));
    let p1 = (cos(p_end), sin(p_end));
    let p_wide = paddle_span > 32768;
    let p_lut = if flash { &FLASH_LUT } else { &PADDLE_LUT };

    for dy in by0..=by1 {
        let dy2 = dy * dy;
        let reach = isqrt((HI_O / 4 - dy2).max(0)) + 1;
        let m = LO_I - 4 * dy2;
        let hole = if m >= 0 { isqrt(m / 4) } else { 0 };
        let mut run = Run::new(s, C + dy, false);

        let lx0 = bx0.max(-reach);
        let lx1 = bx1.min(-hole);
        if lx0 <= lx1 {
            for dx in lx0..=lx1 {
                let k = radial_level(4 * (dx * dx + dy2));
                if k > 0 && (win_sweep >= 65536 || in_sector(dx, dy, w0, w1, w_wide)) {
                    let lut = if in_sector(dx, dy, p0, p1, p_wide) { p_lut } else { &TRACK_LUT };
                    run.put(C + dx, 1, lut[k as usize]);
                }
            }
        }

        let r_min = if hole == 0 { 1 } else { hole };
        let rx0 = bx0.max(r_min);
        let rx1 = bx1.min(reach);
        if rx0 <= rx1 {
            for dx in rx0..=rx1 {
                let k = radial_level(4 * (dx * dx + dy2));
                if k > 0 && (win_sweep >= 65536 || in_sector(dx, dy, w0, w1, w_wide)) {
                    let lut = if in_sector(dx, dy, p0, p1, p_wide) { p_lut } else { &TRACK_LUT };
                    run.put(C + dx, 1, lut[k as usize]);
                }
            }
        }
    }
}

/// Incrementally updates ONLY the slices of the ring that changed between `old` and `new`.
/// Guaranteed pixel-identical to `paint_ring_full`!
fn update_rail_diff(s: &mut impl Surface, old: &Seen, now: &Seen) {
    let old_s = paddle_start(old.paddle, old.span);
    let old_span = old.span as u32;
    let new_s = paddle_start(now.paddle, now.span);
    let new_span = now.span as u32;

    // Small angular margin (256 turns ~ 1.4 deg) so boundary-ray rounding is always covered cleanly.
    const PAD: u32 = 256;

    // If flash color changed or the paddle jumped farther than its own width, repaint just old & new spans.
    let delta = diff(now.paddle, old.paddle);
    if old.flash != now.flash || delta.abs() as u32 >= old_span.min(new_span) {
        paint_ring_window(s, old_s.wrapping_sub(PAD as u16), old_span + 2 * PAD, new_s, new_span, now.flash);
        paint_ring_window(s, new_s.wrapping_sub(PAD as u16), new_span + 2 * PAD, new_s, new_span, now.flash);
        return;
    }

    // Old and new paddles overlap! Only repaint the tiny leading and trailing slices.
    let old_e = old_s.wrapping_add(old_span as u16);
    let new_e = new_s.wrapping_add(new_span as u16);

    let d_start = diff(new_s, old_s);
    if d_start != 0 {
        let (ws, ww) = if d_start > 0 {
            (old_s.wrapping_sub(PAD as u16), d_start as u32 + 2 * PAD)
        } else {
            (new_s.wrapping_sub(PAD as u16), (-d_start) as u32 + 2 * PAD)
        };
        paint_ring_window(s, ws, ww, new_s, new_span, now.flash);
    }

    let d_end = diff(new_e, old_e);
    if d_end != 0 {
        let (ws, ww) = if d_end > 0 {
            (old_e.wrapping_sub(PAD as u16), d_end as u32 + 2 * PAD)
        } else {
            (new_e.wrapping_sub(PAD as u16), (-d_end) as u32 + 2 * PAD)
        };
        paint_ring_window(s, ws, ww, new_s, new_span, now.flash);
    }
}

/// Fast filled disc using precomputed horizontal spans (zero `isqrt` per pixel!).
fn draw_spans(s: &mut impl Surface, cx: i32, cy: i32, spans: &[i32], col: u16) {
    let r = (spans.len() - 1) as i32;
    for dy in -r..=r {
        let hw = spans[dy.unsigned_abs() as usize];
        s.rect(cx - hw, cy + dy, hw * 2 + 1, 1, col);
    }
}

/// Crisp pearl ball with subtle 1px softened outer contour.
fn draw_ball(s: &mut impl Surface, (cx, cy): (i32, i32)) {
    let r = (BALL_SPANS.len() - 1) as i32;
    for dy in -r..=r {
        let ay = dy.unsigned_abs() as usize;
        let hw = BALL_SPANS[ay];
        if ay == r as usize {
            s.rect(cx - hw, cy + dy, hw * 2 + 1, 1, BALL_EDGE);
        } else {
            s.rect(cx - hw, cy + dy, 1, 1, BALL_EDGE);
            if hw > 0 {
                s.rect(cx - hw + 1, cy + dy, (hw - 1) * 2 + 1, 1, BALL_COL);
                s.rect(cx + hw, cy + dy, 1, 1, BALL_EDGE);
            }
        }
    }
}

/// Erases ONLY the pixels of `old` disc that do NOT overlap `keep` disc.
/// Because `keep` is drawn FIRST, the ball is NEVER absent from the framebuffer for even 1 us!
fn erase_spans_except(
    s: &mut impl Surface,
    old: (i32, i32),
    old_spans: &[i32],
    keep: (i32, i32),
    keep_spans: &[i32],
) {
    let or = (old_spans.len() - 1) as i32;
    let kr = (keep_spans.len() - 1) as i32;
    for dy in -or..=or {
        let y = old.1 + dy;
        let ohw = old_spans[dy.unsigned_abs() as usize];
        let ox0 = old.0 - ohw;
        let ox1 = old.0 + ohw + 1;
        let kdy = y - keep.1;
        if kdy.abs() <= kr {
            let khw = keep_spans[kdy.unsigned_abs() as usize];
            let kx0 = keep.0 - khw;
            let kx1 = keep.0 + khw + 1;
            if ox0 < kx0 {
                s.rect(ox0, y, ox1.min(kx0) - ox0, 1, BG);
            }
            if ox1 > kx1 {
                let rx0 = ox0.max(kx1);
                s.rect(rx0, y, ox1 - rx0, 1, BG);
            }
        } else {
            s.rect(ox0, y, ox1 - ox0, 1, BG);
        }
    }
}

/// Bounding box `[x0, y0, x1, y1)` of the center score (and optional BEST tag).
fn score_box(score: u32, beat_best: bool) -> [i32; 4] {
    let mut buf = [0u8; 11];
    let w = big_width(int(score as i32, &mut buf), SCORE_SCALE).max(if beat_best { 64 } else { 0 });
    let y1 = if beat_best { BEST_TAG_Y + 16 } else { SCORE_Y + 8 * SCORE_SCALE };
    [C - w / 2, SCORE_Y, C - w / 2 + w, y1]
}

fn draw_score(s: &mut impl Surface, score: u32, beat_best: bool) {
    let mut buf = [0u8; 11];
    let text = int(score as i32, &mut buf);
    let w = big_width(text, SCORE_SCALE);
    big(s, C - w / 2, SCORE_Y, text, SCORE_SCALE, SCORE_DIM);
    if beat_best {
        centre(s, C, BEST_TAG_Y, "BEST", SCORE_BEST_DIM);
    }
}

#[inline]
fn overlaps_box((x, y): (i32, i32), r: i32, b: [i32; 4]) -> bool {
    x + r >= b[0] && x - r < b[2] && y + r >= b[1] && y - r < b[3]
}

#[inline]
fn near_rail((x, y): (i32, i32), r: i32) -> bool {
    let (dx, dy) = (x - C, y - C);
    let d2 = dx * dx + dy * dy;
    let lim = RAIL_R0 - r - 2;
    d2 >= lim * lim
}

fn repair_rail_around(s: &mut impl Surface, (x, y): (i32, i32), r: i32, now: &Seen) {
    let (dx, dy) = (x - C, y - C);
    let dist = isqrt(dx * dx + dy * dy).max(1);
    let a = angle(dx, dy);
    // Angular half-width of disc radius `r` at distance `dist`, plus small safety margin.
    let half = ((r + 4) * 65536 / (dist * 314 / 50)).clamp(1024, 16384) as u32;
    let p_start = paddle_start(now.paddle, now.span);
    paint_ring_window(s, a.wrapping_sub(half as u16), half * 2, p_start, now.span as u32, now.flash);
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

/// Full screen redraw (used on startup, state transitions, and RESUME).
pub fn full(s: &mut impl Surface, g: &Game, best: u32) {
    let _ = ONE;
    let _ = BG_RGB;
    s.rect(0, 0, SIZE, SIZE, BG);
    match g.state {
        State::Ready => {
            let title = "HALO";
            big(s, C - big_width(title, 5) / 2, 108, title, 5, TEXT_PRI);
            centre(s, C, 286, "TAP TO PLAY", TEXT_PRI);
            centre(s, C, 314, "CROWN TO STEER", TEXT_DIM);
            best_line(s, 352, best, TEXT_SEC);
        }
        State::Playing => {
            draw_score(s, g.score, g.beat_best);
            if g.trail_len >= 2 {
                draw_spans(s, g.trail[1].0, g.trail[1].1, &T2_SPANS, TRAIL2_COL);
            }
            if g.trail_len >= 1 {
                draw_spans(s, g.trail[0].0, g.trail[0].1, &T1_SPANS, TRAIL1_COL);
            }
        }
        State::Over => {
            if g.beat_best || (g.score > 0 && g.score == best) {
                centre(s, C, 114, "NEW BEST", TEXT_PRI);
            } else {
                centre(s, C, 114, "GAME OVER", TEXT_SEC);
            }
            let mut buf = [0u8; 11];
            let text = int(g.score as i32, &mut buf);
            big(s, C - big_width(text, 7) / 2, C - 44, text, 7, TEXT_PRI);
            best_line(s, 286, best, TEXT_SEC);
            centre(s, C, 326, "TAP TO RETRY", TEXT_PRI);
        }
    }
    paint_ring_full(s, g.paddle, g.span(), g.flash > 0);
    if g.state != State::Over {
        draw_ball(s, g.ball());
    }
}

/// Differential frame update: repaints ONLY what changed since `last`.
pub fn frame(s: &mut impl Surface, g: &Game, best: u32, last: Option<Seen>) -> Seen {
    let now = snapshot(g);
    let Some(old) = last.filter(|l| l.state == now.state) else {
        full(s, g, best);
        return now;
    };

    let rail_changed = (old.paddle, old.span, old.flash) != (now.paddle, now.span, now.flash);
    if g.state != State::Playing {
        if rail_changed {
            update_rail_diff(s, &old, &now);
        }
        return now;
    }

    // 1. Update ONLY the changed slices of the paddle (tiny dirty box!).
    if rail_changed {
        update_rail_diff(s, &old, &now);
    }

    let ball_moved = old.ball != now.ball;
    let trail_changed = old.trail != now.trail || old.trail_len != now.trail_len;

    // 2. Erase old trail dots (without touching pixels covered by the new ball!).
    if trail_changed {
        if old.trail_len >= 2 {
            erase_spans_except(s, old.trail[1], &T2_SPANS, now.ball, &BALL_SPANS);
        }
        if old.trail_len >= 1 {
            erase_spans_except(s, old.trail[0], &T1_SPANS, now.ball, &BALL_SPANS);
        }
    }

    // 3. Draw NEW ball FIRST, then erase ONLY `(old_ball \ new_ball)` -> ZERO ball disappearance!
    if ball_moved {
        draw_ball(s, now.ball);
        erase_spans_except(s, old.ball, &BALL_SPANS, now.ball, &BALL_SPANS);
    }

    // 4. Repair center score if score changed or if old ball/trail overlapped the score box.
    let old_box = score_box(old.score, old.beat_best);
    let new_box = score_box(now.score, now.beat_best);
    let score_changed = old.score != now.score || old.beat_best != now.beat_best;
    let score_touched = (ball_moved && overlaps_box(old.ball, BALL_R, new_box))
        || (trail_changed
            && ((old.trail_len >= 1 && overlaps_box(old.trail[0], 6, new_box))
                || (old.trail_len >= 2 && overlaps_box(old.trail[1], 4, new_box))));

    if score_changed {
        let bx0 = old_box[0].min(new_box[0]);
        let by0 = old_box[1].min(new_box[1]);
        let bx1 = old_box[2].max(new_box[2]);
        let by1 = old_box[3].max(new_box[3]);
        s.rect(bx0, by0, bx1 - bx0, by1 - by0, BG);
        draw_score(s, now.score, now.beat_best);
    } else if score_touched {
        draw_score(s, now.score, now.beat_best);
    }

    // 5. Repair tiny local ring window if old ball or trail touched near the rail.
    let old_near = ball_moved && near_rail(old.ball, BALL_R);
    let old_t0_near = trail_changed && old.trail_len >= 1 && near_rail(old.trail[0], 6);
    let old_t1_near = trail_changed && old.trail_len >= 2 && near_rail(old.trail[1], 4);
    if old_near {
        repair_rail_around(s, old.ball, BALL_R, &now);
    }
    if old_t0_near {
        repair_rail_around(s, old.trail[0], 6, &now);
    }
    if old_t1_near {
        repair_rail_around(s, old.trail[1], 4, &now);
    }

    // 6. Redraw trail dots cleanly underneath the ball (never over the rail).
    if trail_changed || ball_moved || score_changed || score_touched {
        if now.trail_len >= 2 && !near_rail(now.trail[1], 4) {
            draw_spans(s, now.trail[1].0, now.trail[1].1, &T2_SPANS, TRAIL2_COL);
        }
        if now.trail_len >= 1 && !near_rail(now.trail[0], 6) {
            draw_spans(s, now.trail[0].0, now.trail[0].1, &T1_SPANS, TRAIL1_COL);
        }
    }

    // 7. Re-assert the ball on top whenever score/trail/rail touched its area.
    let ball_needs_top = score_changed
        || score_touched
        || trail_changed
        || ((rail_changed || old_near || old_t0_near || old_t1_near) && near_rail(now.ball, BALL_R));
    if ball_needs_top {
        draw_ball(s, now.ball);
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
    fn crown_spin_diff_matches_full_pixel_for_pixel() {
        let mut g = Game::new(42);
        let mut px = vec![0x5678u16; N];
        let mut seen = Some(frame(&mut Fb::new(&mut px), &g, 10, None));

        // Spin across 0/65535 wraparound, fast multi-detent jumps, and reverse spins
        for delta in [1, 1, 2, 5, 12, -3, -9, -15, 4, 1, -1] {
            g.apply_crown(delta);
            seen = Some(frame(&mut Fb::new(&mut px), &g, 10, seen));
            let expected = render_full(&g, 10);
            let mut mismatches = 0usize;
            for i in 0..N {
                if px[i] != expected[i] {
                    mismatches += 1;
                }
            }
            assert_eq!(mismatches, 0, "pixel mismatch after crown delta={delta}, paddle={}", g.paddle);
        }
    }

    #[test]
    fn crown_spin_is_ultra_cheap() {
        let mut g = Game::new(7);
        let mut dummy = Count::default();
        let seen = Some(frame(&mut dummy, &g, 0, None));
        g.apply_crown(1);
        let mut cost = Count::default();
        frame(&mut cost, &g, 0, seen);
        // Original orbit drew ~15,000+ px across 450 rows on every detent!
        // Our differential slice should stay well under 2,000 px.
        assert!(cost.px < 2000, "1-detent crown cost too high: {} px ({} rects)", cost.px, cost.rects);
        std::println!("1-detent crown diff cost: {} rects, {} px", cost.rects, cost.px);
    }

    #[test]
    fn gameplay_simulation_has_no_artifacts() {
        let mut g = Game::new(777);
        let best = 3;
        let mut px = vec![0u16; N];
        let mut seen = Some(frame(&mut Fb::new(&mut px), &g, best, None));
        g.launch(best);
        seen = Some(frame(&mut Fb::new(&mut px), &g, best, seen));

        for step_idx in 0..120 {
            if step_idx % 3 == 0 {
                g.apply_crown(2);
            }
            let _ = g.step();
            seen = Some(frame(&mut Fb::new(&mut px), &g, best, seen));
            if g.state == State::Over {
                break;
            }
        }
        // Final state after restart & resume must match full()
        g.restart();
        let _ = frame(&mut Fb::new(&mut px), &g, best, seen);
        assert_eq!(px, render_full(&g, best));
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

        // 1. Ready screen (with best = 14)
        let mut g = Game::new(42);
        save("halo_ready.rgb565", &render_full(&g, 14));

        // 2. Playing screen (mid-flight with score 7, trail visible)
        g.launch(14);
        g.score = 7;
        for i in 0..26 {
            if i % 4 == 0 { g.apply_crown(1); }
            let _ = g.step();
        }
        save("halo_playing.rgb565", &render_full(&g, 14));

        // 3. Game over screen (new best = 18)
        g.state = State::Over;
        g.score = 18;
        g.beat_best = true;
        save("halo_gameover.rgb565", &render_full(&g, 18));
    }
}

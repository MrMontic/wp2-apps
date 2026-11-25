//! Ultra-fast, zero-flicker monochrome renderer for STACK.
//!
//! During normal gliding (95% of frames), ONLY the 2 tiny side slivers of the moving slab
//! (and falling scrap if present) are repainted — typically ~6-14 rect calls (~300-800 px)!
//! The stacked tower is only redrawn when a block locks, flashes, or scrolls smoothly.

use crate::game::{BLOCK_H, BASE_Y, Block, C, Debris, Game, SIZE, START_W, State};
use cmf_wp2_ui::Surface;
use cmf_wp2_ui::color::rgb565;
use cmf_wp2_ui::text::{big, big_width, centre, int};

const BG: u16 = 0x0000;
const TEXT_PRI: u16 = rgb565(238, 238, 244);
const TEXT_SEC: u16 = rgb565(136, 136, 148);
const TEXT_DIM: u16 = rgb565(84, 84, 96);
const PERFECT_COL: u16 = rgb565(246, 246, 252);

const SCORE_Y: i32 = 44;
const SCORE_SCALE: i32 = 5;
const SUB_Y: i32 = 96;

/// Visible tower viewport vertically clipped nicely inside the circular watch face.
const TOWER_TOP_CLIP: i32 = 122;
const TOWER_BOT_CLIP: i32 = 412;

#[derive(Clone, Copy)]
struct SlabPalette {
    body: u16,
    top: u16,
    bot: u16,
    side: u16,
}

/// Calm oscillating monochrome palette (graphite -> silver -> pearl -> silver -> graphite)
/// with subtle cool slate tint and architectural top/bottom bevel highlights.
fn slab_palette(level: u32, flash: bool) -> SlabPalette {
    if flash {
        return SlabPalette {
            body: rgb565(250, 250, 255),
            top: rgb565(255, 255, 255),
            bot: rgb565(212, 212, 224),
            side: rgb565(255, 255, 255),
        };
    }
    // Triangle wave 0..16..0 across 32 levels: lum 68..212 (never harsh, never invisible).
    let phase = (level % 32) as i32;
    let step = if phase <= 16 { phase } else { 32 - phase };
    let lum = 68 + step * 9; // 68..212
    let clamp = |v: i32| v.clamp(0, 255) as u8;
    let make = |l: i32| rgb565(clamp(l), clamp(l), clamp(l + 7));
    SlabPalette {
        body: make(lum),
        top: make(lum + 30),
        bot: make(lum - 24),
        side: make(lum + 14),
    }
}

/// Draws a refined architectural monochrome slab clipped to `[TOWER_TOP_CLIP, TOWER_BOT_CLIP)`.
fn draw_slab(s: &mut impl Surface, x: i32, y: i32, w: i32, h: i32, p: SlabPalette) {
    if w <= 0 || h <= 0 {
        return;
    }
    let y0 = y.max(TOWER_TOP_CLIP);
    let y1 = (y + h).min(TOWER_BOT_CLIP);
    if y0 >= y1 {
        return;
    }
    // Top 2px highlight bevel
    let top_y1 = (y + 2).min(y1);
    if y0 < top_y1 {
        s.rect(x, y0, w, top_y1 - y0, p.top);
    }
    // Bottom 2px shadow bevel
    let bot_y0 = (y + h - 2).max(y0);
    if bot_y0 < y1 {
        s.rect(x, bot_y0, w, y1 - bot_y0, p.bot);
    }
    // Main matte body
    let mid_y0 = top_y1.max(y0);
    let mid_y1 = bot_y0.min(y1);
    if mid_y0 < mid_y1 {
        if w > 2 {
            s.rect(x, mid_y0, 1, mid_y1 - mid_y0, p.side);
            s.rect(x + 1, mid_y0, w - 2, mid_y1 - mid_y0, p.body);
            s.rect(x + w - 1, mid_y0, 1, mid_y1 - mid_y0, p.side);
        } else {
            s.rect(x, mid_y0, w, mid_y1 - mid_y0, p.body);
        }
    }
}

/// Draws the subtle dark pedestal beneath level 0 so the tower looks grounded in OLED black.
fn draw_pedestal(s: &mut impl Surface, cam_y: i32) {
    let y = BASE_Y + cam_y;
    if y >= TOWER_BOT_CLIP {
        return;
    }
    let x = C - START_W / 2;
    let shades = [
        (0, 6, rgb565(34, 34, 40)),
        (6, 12, rgb565(24, 24, 29)),
        (18, 16, rgb565(14, 14, 18)),
    ];
    for (off, h, col) in shades {
        let ry0 = (y + off).max(TOWER_TOP_CLIP);
        let ry1 = (y + off + h).min(TOWER_BOT_CLIP);
        if ry0 < ry1 {
            s.rect(x, ry0, START_W, ry1 - ry0, col);
        }
    }
}

fn draw_debris(s: &mut impl Surface, d: Debris) {
    let y = d.y();
    let y0 = y.max(TOWER_TOP_CLIP);
    let y1 = (y + BLOCK_H).min(TOWER_BOT_CLIP);
    if y0 >= y1 || d.w <= 0 {
        return;
    }
    // Fade smoothly to dark as life counts down
    let k = (d.life as i32).clamp(0, 22);
    let base = 24 + k * 7; // fades down into OLED black
    let col = rgb565(base as u8, base as u8, (base + 6).min(255) as u8);
    let top_col = rgb565((base + 20).min(255) as u8, (base + 20).min(255) as u8, (base + 26).min(255) as u8);
    let top_y1 = (y + 2).min(y1);
    if y0 < top_y1 {
        s.rect(d.x, y0, d.w, top_y1 - y0, top_col);
    }
    if top_y1 < y1 {
        s.rect(d.x, top_y1, d.w, y1 - top_y1, col);
    }
}

fn erase_rect_clipped(s: &mut impl Surface, x: i32, y: i32, w: i32, h: i32) {
    if w <= 0 || h <= 0 {
        return;
    }
    let y0 = y.max(TOWER_TOP_CLIP);
    let y1 = (y + h).min(TOWER_BOT_CLIP);
    if y0 < y1 {
        s.rect(x, y0, w, y1 - y0, BG);
    }
}

/// Draws ONLY the changed horizontal slivers when the active slab slides at the same Y.
/// Zero flicker: newly covered side is painted FIRST, then uncovered trailing edge is erased!
fn update_active_slide(
    s: &mut impl Surface,
    old_x: i32,
    new_x: i32,
    y: i32,
    w: i32,
    level: u32,
) {
    if old_x == new_x {
        return;
    }
    let p = slab_palette(level, false);
    let y0 = y.max(TOWER_TOP_CLIP);
    let y1 = (y + BLOCK_H).min(TOWER_BOT_CLIP);
    if y0 >= y1 {
        return;
    }

    let dx = new_x - old_x;
    if dx.abs() >= w {
        draw_slab(s, new_x, y, w, BLOCK_H, p);
        erase_rect_clipped(s, old_x, y, w, BLOCK_H);
        return;
    }

    let top_y1 = (y + 2).min(y1);
    let bot_y0 = (y + BLOCK_H - 2).max(y0);
    let mid_y0 = top_y1.max(y0);
    let mid_y1 = bot_y0.min(y1);

    if dx > 0 {
        // Moving right: fill newly exposed right strip [old_x + w - 1 .. new_x + w)
        // (-1 converts previous right-side 1px edge back into body!)
        let lead_x = (old_x + w - 1).max(new_x);
        let lead_w = (new_x + w) - lead_x;
        if y0 < top_y1 { s.rect(lead_x, y0, lead_w, top_y1 - y0, p.top); }
        if bot_y0 < y1 { s.rect(lead_x, bot_y0, lead_w, y1 - bot_y0, p.bot); }
        if mid_y0 < mid_y1 {
            if lead_w > 1 {
                s.rect(lead_x, mid_y0, lead_w - 1, mid_y1 - mid_y0, p.body);
            }
            s.rect(new_x + w - 1, mid_y0, 1, mid_y1 - mid_y0, p.side);
            // Restore crisp 1px left edge at new_x
            s.rect(new_x, mid_y0, 1, mid_y1 - mid_y0, p.side);
        }
        // Erase trailing left strip [old_x .. new_x)
        s.rect(old_x, y0, dx, y1 - y0, BG);
    } else {
        // Moving left: fill newly exposed left strip [new_x .. old_x + 1)
        let lead_end = (old_x + 1).min(new_x + w);
        let lead_w = lead_end - new_x;
        if y0 < top_y1 { s.rect(new_x, y0, lead_w, top_y1 - y0, p.top); }
        if bot_y0 < y1 { s.rect(new_x, bot_y0, lead_w, y1 - bot_y0, p.bot); }
        if mid_y0 < mid_y1 {
            s.rect(new_x, mid_y0, 1, mid_y1 - mid_y0, p.side);
            if lead_w > 1 {
                s.rect(new_x + 1, mid_y0, lead_w - 1, mid_y1 - mid_y0, p.body);
            }
            // Restore crisp 1px right edge at new_x + w - 1
            s.rect(new_x + w - 1, mid_y0, 1, mid_y1 - mid_y0, p.side);
        }
        // Erase trailing right strip [new_x + w .. old_x + w)
        s.rect(new_x + w, y0, -dx, y1 - y0, BG);
    }
}

fn draw_tower_area(s: &mut impl Surface, g: &Game) {
    s.rect(0, TOWER_TOP_CLIP, SIZE, TOWER_BOT_CLIP - TOWER_TOP_CLIP, BG);
    draw_pedestal(s, g.cam_y);
    let start_idx = g.count.saturating_sub(14);
    for idx in start_idx..g.count {
        if let Some(b) = g.block_at(idx) {
            let y = g.block_y(idx);
            if y < TOWER_BOT_CLIP && y + BLOCK_H > TOWER_TOP_CLIP {
                let is_flash = g.flash > 0 && idx + 1 == g.count;
                draw_slab(s, b.x, y, b.w, BLOCK_H, slab_palette(b.level, is_flash));
            }
        }
    }
    if g.state == State::Playing {
        draw_slab(s, g.active_x(), g.active_y(), g.cur_w, BLOCK_H, slab_palette(g.count, false));
    }
    if let Some(d) = g.debris {
        draw_debris(s, d);
    }
}

fn draw_hud(s: &mut impl Surface, g: &Game, best: u32) {
    s.rect(0, 32, SIZE, TOWER_TOP_CLIP - 32, BG);
    let mut buf = [0u8; 11];
    let score_str = int(g.score as i32, &mut buf);
    big(s, C - big_width(score_str, SCORE_SCALE) / 2, SCORE_Y, score_str, SCORE_SCALE, TEXT_PRI);

    if g.banner_timer > 0 && g.banner_streak > 0 {
        if g.banner_grew {
            centre(s, C, SUB_Y, "PERFECT +", PERFECT_COL);
        } else if g.banner_streak >= 2 {
            let mut b = [0u8; 16];
            b[..9].copy_from_slice(b"PERFECT x");
            let mut d = [0u8; 11];
            let ds = int(g.banner_streak.min(999) as i32, &mut d).as_bytes();
            b[9..9 + ds.len()].copy_from_slice(ds);
            if let Ok(lbl) = core::str::from_utf8(&b[..9 + ds.len()]) {
                centre(s, C, SUB_Y, lbl, PERFECT_COL);
            }
        } else {
            centre(s, C, SUB_Y, "PERFECT", PERFECT_COL);
        }
    } else if g.beat_best {
        centre(s, C, SUB_Y, "NEW BEST", TEXT_PRI);
    } else if best > 0 {
        best_line(s, SUB_Y, best, TEXT_DIM);
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

/// Full screen render for Ready, Playing, and Over states.
pub fn full(s: &mut impl Surface, g: &Game, best: u32) {
    s.rect(0, 0, SIZE, SIZE, BG);
    match g.state {
        State::Ready => {
            let title = "STACK";
            big(s, C - big_width(title, 5) / 2, 92, title, 5, TEXT_PRI);
            centre(s, C, 166, "TAP TO DROP", TEXT_PRI);
            best_line(s, 198, best, TEXT_SEC);

            // Preview miniature calm monochrome tower at the bottom of the Ready screen
            draw_pedestal(s, 0);
            let demo_w = [START_W, START_W - 18, START_W - 36, START_W - 54];
            for (i, &w) in demo_w.iter().enumerate() {
                let y = BASE_Y - (i as i32 + 1) * BLOCK_H;
                draw_slab(s, C - w / 2, y, w, BLOCK_H, slab_palette(i as u32 * 3, false));
            }
        }
        State::Playing => {
            draw_hud(s, g, best);
            draw_tower_area(s, g);
        }
        State::Over => {
            // Show the top of the built tower softly below the Game Over card
            let start_idx = g.count.saturating_sub(5);
            for idx in start_idx..g.count {
                if let Some(b) = g.block_at(idx) {
                    let row = (g.count - 1 - idx) as i32;
                    let y = 326 + row * BLOCK_H;
                    if y < TOWER_BOT_CLIP {
                        // Dimmed version of the tower as subtle backdrop
                        let p = SlabPalette {
                            body: rgb565(36, 36, 42),
                            top: rgb565(50, 50, 58),
                            bot: rgb565(22, 22, 26),
                            side: rgb565(44, 44, 52),
                        };
                        draw_slab(s, b.x, y, b.w, BLOCK_H, p);
                    }
                }
            }
            if g.beat_best || (g.score > 0 && g.score == best) {
                centre(s, C, 92, "NEW BEST", TEXT_PRI);
            } else {
                centre(s, C, 92, "GAME OVER", TEXT_SEC);
            }
            let mut buf = [0u8; 11];
            let text = int(g.score as i32, &mut buf);
            big(s, C - big_width(text, 7) / 2, 144, text, 7, TEXT_PRI);
            best_line(s, 232, best, TEXT_SEC);
            centre(s, C, 272, "TAP TO RETRY", TEXT_PRI);
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Seen {
    state: State,
    count: u32,
    score: u32,
    top: Block,
    active_x: i32,
    active_y: i32,
    active_w: i32,
    cam_y: i32,
    flash: bool,
    banner_on: bool,
    banner_streak: u32,
    banner_grew: bool,
    beat_best: bool,
    debris: Option<(i32, i32, i32, u8)>,
}

fn snapshot(g: &Game) -> Seen {
    Seen {
        state: g.state,
        count: g.count,
        score: g.score,
        top: g.top_block(),
        active_x: g.active_x(),
        active_y: g.active_y(),
        active_w: g.cur_w,
        cam_y: g.cam_y,
        flash: g.flash > 0,
        banner_on: g.banner_timer > 0,
        banner_streak: g.banner_streak,
        banner_grew: g.banner_grew,
        beat_best: g.beat_best,
        debris: g.debris.map(|d| (d.x, d.y(), d.w, d.life)),
    }
}

/// Differential frame update: repaints ONLY what changed since `last`.
pub fn frame(s: &mut impl Surface, g: &Game, best: u32, last: Option<Seen>) -> Seen {
    let now = snapshot(g);
    let Some(old) = last.filter(|l| l.state == now.state) else {
        full(s, g, best);
        return now;
    };

    if g.state != State::Playing {
        return now;
    }

    // 1. HUD updates only when score/banner/record changes (rare: once per tap!).
    let hud_dirty = old.score != now.score
        || old.banner_on != now.banner_on
        || old.banner_streak != now.banner_streak
        || old.banner_grew != now.banner_grew
        || old.beat_best != now.beat_best;
    if hud_dirty {
        draw_hud(s, g, best);
    }

    // 2. If camera scrolled or stack count changed, repaint the tower region cleanly.
    if old.cam_y != now.cam_y || old.count != now.count {
        draw_tower_area(s, g);
        return now;
    }

    // 3. If top block's PERFECT flash ended, repaint just that 1 block.
    if old.flash != now.flash {
        let top_y = g.block_y(g.count - 1);
        draw_slab(s, now.top.x, top_y, now.top.w, BLOCK_H, slab_palette(now.top.level, now.flash));
    }

    // 4. Update falling debris piece (if any) — tiny rect diff!
    if old.debris != now.debris {
        if let Some((ox, oy, ow, _)) = old.debris {
            erase_rect_clipped(s, ox, oy, ow, BLOCK_H);
            // Repair any stacked block rows that debris overlapped
            let start_idx = g.count.saturating_sub(14);
            for idx in start_idx..g.count {
                if let Some(b) = g.block_at(idx) {
                    let by = g.block_y(idx);
                    if by < oy + BLOCK_H && by + BLOCK_H > oy && b.x < ox + ow && b.x + b.w > ox {
                        let is_flash = g.flash > 0 && idx + 1 == g.count;
                        draw_slab(s, b.x, by, b.w, BLOCK_H, slab_palette(b.level, is_flash));
                    }
                }
            }
        }
        if let Some(d) = g.debris {
            draw_debris(s, d);
        }
    }

    // 5. Ultra-fast horizontal sliver update for the gliding active slab!
    if old.active_x != now.active_x {
        update_active_slide(s, old.active_x, now.active_x, now.active_y, now.active_w, now.count);
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
    fn slide_diff_matches_full_pixel_for_pixel() {
        let mut g = Game::new();
        g.start(10);
        let mut px = vec![0x5678u16; N];
        let mut seen = Some(frame(&mut Fb::new(&mut px), &g, 10, None));

        // Glide back and forth across both walls and verify EVERY SINGLE PIXEL!
        for _ in 0..80 {
            g.step();
            seen = Some(frame(&mut Fb::new(&mut px), &g, 10, seen));
            let expected = render_full(&g, 10);
            assert_eq!(px, expected, "pixel mismatch at x={}", g.active_x());
        }
    }

    #[test]
    fn slide_frame_is_insanely_cheap() {
        let mut g = Game::new();
        g.start(0);
        let mut dummy = Count::default();
        let seen = Some(frame(&mut dummy, &g, 0, None));
        g.step();
        let mut cost = Count::default();
        frame(&mut cost, &g, 0, seen);
        assert!(cost.rects <= 10, "too many rects for slide: {}", cost.rects);
        assert!(cost.px < 400, "too many pixels for slide: {}", cost.px);
        std::println!("1-tick slide diff cost: {} rects, {} px", cost.rects, cost.px);
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
        let mut g = Game::new();
        save("stack_ready.rgb565", &render_full(&g, 21));

        // 2. Playing screen (tower of 7 blocks, gliding slab + falling debris + PERFECT streak!)
        g.start(21);
        for off in [0, 8, -6, 0, 10, -4, 0] {
            g.cur_x_q = (g.top_block().x + off) * crate::game::Q;
            let _ = g.drop_slab();
            for _ in 0..4 { g.step(); }
        }
        // Trigger a crisp sliced debris visible mid-fall
        g.cur_x_q = (g.top_block().x + 22) * crate::game::Q;
        let _ = g.drop_slab();
        for _ in 0..10 { g.step(); }
        save("stack_playing.rgb565", &render_full(&g, 21));

        // 3. Game over screen (new best!)
        g.state = State::Over;
        g.score = 24;
        g.beat_best = true;
        save("stack_gameover.rgb565", &render_full(&g, 24));
    }
}

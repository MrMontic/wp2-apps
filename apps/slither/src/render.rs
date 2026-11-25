//! Zero-flicker single-pass RLE renderer (`116x116` 8-bit indexed framebuffer scaled 4x4 to 464x464)
//! + live Title screen skin preview + crisp native HUD + Game Over summary.

use crate::game::{ARENA_R, DeathCause, Game, MAX_ORBS, MAX_SNAKES, MIN_BOOST_MASS, Q_SHIFT, Screen};
use crate::palette::{
    PAL_BG, PAL_BORDER, PAL_BORDER_GLOW, PAL_BOOST_AURA, PAL_EYE_PUPIL, PAL_EYE_WHITE, PAL_HEX,
    PAL_HEX_HI, PAL_MAP_BG, PAL_MAP_BOT, PAL_MAP_BOSS, PAL_MAP_LOOT, PAL_MAP_RING, PAL_MAP_YOU,
    PAL_OUTSIDE, PALETTE, SKIN_NAMES, UI_BG, UI_BORDER, UI_CYAN, UI_GOLD, UI_LIME, UI_ORANGE,
    UI_PANEL, UI_RED, UI_SILVER, UI_WHITE, orb_palette, skin_segment_color, skin_ui_color,
};
use cmf_wp2_ui::motion::{cos, isqrt, sin};
use cmf_wp2_ui::text::{big, big_width, centre, int};
use cmf_wp2_ui::{C, SIZE, Surface};

/// Compact indexed framebuffer: `116 x 116` = 13,456 bytes (~13.1 KB RAM).
pub const FB_W: usize = 116;
pub const FB_H: usize = 116;
pub const FB_SCALE: i32 = 4;
pub const FB_OFFSET: i32 = 1; // 1 + 116*4 = 465 centered inside 466x466!

/// Shared UI button hitboxes (all comfortably inside the 466px circular watch bezel).
pub const TITLE_PREV_RECT: [i32; 4] = [56, 250, 58, 48];
pub const TITLE_NEXT_RECT: [i32; 4] = [SIZE - 114, 250, 58, 48];
pub const TITLE_PLAY_RECT: [i32; 4] = [C - 92, 322, 184, 52];

pub const OVER_PLAY_RECT: [i32; 4] = [C - 104, 304, 208, 50];
pub const OVER_SKIN_RECT: [i32; 4] = [C - 76, 366, 152, 42];

/// HUD pill region in framebuffer cell coordinates (composited directly into `FrameBuf`!).
const HUD_CX0: usize = 31;
const HUD_CX1: usize = 85;
const HUD_CY0: usize = 8;
const HUD_CY1: usize = 15;

/// Circular radar minimap placement in framebuffer cell coordinates (bottom-right inside bezel).
const MAP_CX: i32 = 88;
const MAP_CY: i32 = 86;
const MAP_R: i32 = 11;

#[inline]
pub fn hit(r: [i32; 4], x: i32, y: i32) -> bool {
    x >= r[0] && x < r[0] + r[2] && y >= r[1] && y < r[1] + r[3]
}

/// 13.1 KB indexed framebuffer (lives in `static mut` on watch, never on stack).
pub struct FrameBuf {
    pub px: [u8; FB_W * FB_H],
}

impl FrameBuf {
    pub const fn new() -> Self {
        Self { px: [0; FB_W * FB_H] }
    }

    #[inline]
    fn set(&mut self, x: i32, y: i32, col: u8) {
        if x >= 0 && y >= 0 && (x as usize) < FB_W && (y as usize) < FB_H {
            self.px[y as usize * FB_W + x as usize] = col;
        }
    }

    #[inline]
    fn hline(&mut self, x0: i32, x1: i32, y: i32, col: u8) {
        if y < 0 || (y as usize) >= FB_H {
            return;
        }
        let a = x0.max(0) as usize;
        let b = x1.min(FB_W as i32 - 1);
        if b < a as i32 {
            return;
        }
        let row = y as usize * FB_W;
        self.px[row + a..=row + b as usize].fill(col);
    }

    /// Filled integer circle into the indexed framebuffer.
    #[inline]
    fn disc(&mut self, cx: i32, cy: i32, r: i32, col: u8) {
        if r <= 0 {
            self.set(cx, cy, col);
            return;
        }
        if cx + r < 0 || cx - r >= FB_W as i32 || cy + r < 0 || cy - r >= FB_H as i32 {
            return;
        }
        let r2 = r * r + r; // slightly rounder pixel circles at small radii (2..5)
        let mut dy = -r;
        while dy <= r {
            let y = cy + dy;
            if y >= 0 && y < FB_H as i32 {
                let rem = r2 - dy * dy;
                if rem >= 0 {
                    let dx = isqrt(rem);
                    self.hline(cx - dx, cx + dx, y, col);
                }
            }
            dy += 1;
        }
    }

    /// Flush entire `116x116` buffer inside the round watch circle using horizontal RLE spans!
    /// Every visible pixel is written EXACTLY ONCE per frame -> 100% zero flicker!
    pub fn flush_round(&self, s: &mut impl Surface) {
        let half = (FB_W as i32) / 2; // 58
        let r2 = half * half + half;
        let mut y = 0usize;
        while y < FB_H {
            let dy = y as i32 - half;
            let rem = r2 - dy * dy;
            if rem <= 0 {
                y += 1;
                continue;
            }
            let span = isqrt(rem);
            let x_start = (half - span).max(0) as usize;
            let x_end = (half + span + 1).min(FB_W as i32) as usize;
            let row = y * FB_W;
            let py = FB_OFFSET + (y as i32) * FB_SCALE;

            let mut x = x_start;
            while x < x_end {
                let c = self.px[row + x];
                let run_start = x;
                x += 1;
                while x < x_end && self.px[row + x] == c {
                    x += 1;
                }
                let rgb = PALETTE[(c as usize).min(PALETTE.len() - 1)];
                s.rect(
                    FB_OFFSET + (run_start as i32) * FB_SCALE,
                    py,
                    ((x - run_start) as i32) * FB_SCALE,
                    FB_SCALE,
                    rgb,
                );
            }
            y += 1;
        }
    }
}

/// Top-level frame dispatcher. Returns `true` if a full screen redraw occurred.
pub fn render_frame(s: &mut impl Surface, fb: &mut FrameBuf, g: &Game, screen_changed: bool) {
    match g.screen {
        Screen::Title => render_title(s, fb, g),
        Screen::Playing => {
            if screen_changed {
                // One-time corner clear when entering gameplay (never per-frame!).
                s.rect(0, 0, SIZE, SIZE, UI_BG);
            }
            render_playing(s, fb, g);
        }
        Screen::Over => render_over(s, g),
    }
}

/// Renders one complete gameplay frame into `fb`, flushes via RLE, then draws crisp native HUD text.
pub fn render_playing(s: &mut impl Surface, fb: &mut FrameBuf, g: &Game) {
    let cam_wx = g.cam_x >> Q_SHIFT;
    let cam_wy = g.cam_y >> Q_SHIFT;
    // World coordinates of cell (0, 0) center:
    let wx0 = cam_wx - (FB_W as i32) * 2;
    let wy0 = cam_wy - (FB_H as i32) * 2;

    // 1. Deep navy Slither.io background
    fb.px.fill(PAL_BG);

    // 2. Smoothly scrolling Slither.io hexagonal floor tiles (RLE-friendly!)
    draw_hex_floor(fb, wx0, wy0);

    // 3. Glowing crimson circular arena border (skipped entirely when far from edge!)
    draw_arena_border(fb, cam_wx, cam_wy, wx0, wy0);

    // 4. Glowing orbs (ambient pellets + pulsing high-value death loot)
    draw_orbs(fb, g, wx0, wy0);

    // 5. All snakes (AI bots first, player on top; tail -> head layering!)
    let mut si = 1usize;
    while si < MAX_SNAKES {
        if g.snakes[si].alive {
            draw_snake(fb, g, si, wx0, wy0);
        }
        si += 1;
    }
    if g.snakes[0].alive {
        draw_snake(fb, g, 0, wx0, wy0);
    }

    // 6. Compact circular radar minimap composited right into `fb`
    draw_minimap(fb, g);

    // 7. Top HUD pill background composited right into `fb` (zero overdraw flicker!)
    draw_hud_pill_bg(fb, g.snakes[0].boosting);

    // 8. Single-pass circular RLE flush to display!
    fb.flush_round(s);

    // 9. Crisp native-resolution HUD text on top of pre-composited dark pill
    draw_hud_text(s, g);
}

/// Draws scrolling dark slate-blue beveled hexagonal tiles in `fb` (~130-180 extra RLE runs total).
fn draw_hex_floor(fb: &mut FrameBuf, wx0: i32, wy0: i32) {
    const HEX_DX: i32 = 96; // 24 cells horizontally
    const HEX_DY: i32 = 80; // 20 cells vertically

    let first_row = (wy0 - 40).div_euclid(HEX_DY);
    let last_row = (wy0 + FB_H as i32 * 4 + 40).div_euclid(HEX_DY);

    let mut row = first_row;
    while row <= last_row {
        let wy = row * HEX_DY;
        let cy = (wy - wy0) >> 2;
        if cy >= -2 && cy <= FB_H as i32 + 1 {
            let stagger = if (row & 1) != 0 { HEX_DX / 2 } else { 0 };
            let first_col = (wx0 - stagger - 40).div_euclid(HEX_DX);
            let last_col = (wx0 - stagger + FB_W as i32 * 4 + 40).div_euclid(HEX_DX);

            let mut col = first_col;
            while col <= last_col {
                let wx = col * HEX_DX + stagger;
                let cx = (wx - wx0) >> 2;
                if cx >= -4 && cx <= FB_W as i32 + 3 {
                    // 3-row crisp beveled hex tile (16px, 24px, 16px on screen)
                    fb.hline(cx - 2, cx + 1, cy - 1, PAL_HEX_HI);
                    fb.hline(cx - 3, cx + 2, cy, PAL_HEX);
                    fb.hline(cx - 2, cx + 1, cy + 1, PAL_HEX);
                }
                col += 1;
            }
        }
        row += 1;
    }
}

/// Draws the glowing red circular arena boundary and outside void when camera is near the edge.
fn draw_arena_border(fb: &mut FrameBuf, cam_wx: i32, cam_wy: i32, wx0: i32, wy0: i32) {
    // Quick rejection if entire viewport is well inside the circular arena
    let cam_d = isqrt(((cam_wx as i64) * (cam_wx as i64) + (cam_wy as i64) * (cam_wy as i64)).min(i32::MAX as i64) as i32);
    if cam_d + 350 < ARENA_R {
        return;
    }

    let r_glow2 = ((ARENA_R - 18) as i64) * ((ARENA_R - 18) as i64);
    let r_in2 = ((ARENA_R - 6) as i64) * ((ARENA_R - 6) as i64);
    let r_out2 = ((ARENA_R + 10) as i64) * ((ARENA_R + 10) as i64);

    let half = (FB_W as i32) / 2;
    let circ_r2 = half * half + half;

    let mut y = 0usize;
    while y < FB_H {
        let dy_c = y as i32 - half;
        let rem = circ_r2 - dy_c * dy_c;
        if rem <= 0 {
            y += 1;
            continue;
        }
        let span = isqrt(rem);
        let x0 = (half - span).max(0) as usize;
        let x1 = (half + span + 1).min(FB_W as i32) as usize;

        let wy = (wy0 + (y as i32) * 4) as i64;
        let wy2 = wy * wy;
        let row = y * FB_W;

        let mut x = x0;
        while x < x1 {
            let wx = (wx0 + (x as i32) * 4) as i64;
            let d2 = wx * wx + wy2;
            if d2 >= r_glow2 {
                fb.px[row + x] = if d2 > r_out2 {
                    PAL_OUTSIDE
                } else if d2 >= r_in2 {
                    PAL_BORDER
                } else {
                    PAL_BORDER_GLOW
                };
            }
            x += 1;
        }
        y += 1;
    }
}

fn draw_orbs(fb: &mut FrameBuf, g: &Game, wx0: i32, wy0: i32) {
    let tick_b = (g.tick as u8).wrapping_mul(3);
    let mut i = 0usize;
    while i < MAX_ORBS {
        let val = g.orbs[i].val;
        if val == 0 {
            i += 1;
            continue;
        }
        let cx = (g.orbs[i].x as i32 - wx0) >> 2;
        let cy = (g.orbs[i].y as i32 - wy0) >> 2;
        if cx < -3 || cx >= FB_W as i32 + 3 || cy < -3 || cy >= FB_H as i32 + 3 {
            i += 1;
            continue;
        }
        let color = g.orbs[i].color;
        let core = orb_palette(color, true);
        let halo = orb_palette(color, false);
        let pulse = (g.orbs[i].phase.wrapping_add(tick_b) & 64) != 0;

        if val >= 6 {
            // Giant glowing boss-death loot orb!
            fb.disc(cx, cy, if pulse { 3 } else { 2 }, halo);
            fb.disc(cx, cy, 1, core);
        } else if val >= 3 {
            // Bright death-loot orb!
            fb.disc(cx, cy, 2, halo);
            fb.set(cx, cy, core);
            if pulse {
                fb.set(cx + 1, cy, core);
                fb.set(cx, cy + 1, core);
            }
        } else if val == 2 {
            // Boost drop / medium pellet
            if pulse {
                fb.set(cx - 1, cy, halo);
                fb.set(cx + 1, cy, halo);
                fb.set(cx, cy - 1, halo);
                fb.set(cx, cy + 1, halo);
            }
            fb.set(cx, cy, core);
        } else {
            // Small ambient pellet (1 cell = 4x4 px, crisp & super low rect cost!)
            fb.set(cx, cy, if pulse { core } else { halo });
        }
        i += 1;
    }
}

fn draw_snake(fb: &mut FrameBuf, g: &Game, idx: usize, wx0: i32, wy0: i32) {
    let sn = &g.snakes[idx];
    let wx1 = wx0 + (FB_W as i32) * 4;
    let wy1 = wy0 + (FB_H as i32) * 4;

    // Fast viewport AABB rejection
    if (sn.max_x as i32) < wx0 - 28
        || (sn.min_x as i32) > wx1 + 28
        || (sn.max_y as i32) < wy0 - 28
        || (sn.min_y as i32) > wy1 + 28
    {
        return;
    }

    let segs = sn.seg_count();
    let r_cell = ((sn.body_radius() + 2) / 4).clamp(2, 5);
    let skin = sn.skin;
    let boosting = sn.boosting;

    // Optional outer boost glow aura (drawn behind segments so body pops cleanly!)
    if boosting {
        let mut k = segs;
        while k > 0 {
            k -= 1;
            if (k & 1) == 0 {
                let (sx, sy) = sn.seg_pos(k);
                let cx = (sx - wx0) >> 2;
                let cy = (sy - wy0) >> 2;
                if cx >= -6 && cx < FB_W as i32 + 6 && cy >= -6 && cy < FB_H as i32 + 6 {
                    fb.disc(cx, cy, r_cell + 1, PAL_BOOST_AURA);
                }
            }
        }
    }

    // Draw body segments from tail to head so each segment overlaps cleanly forward
    let mut k = segs;
    while k > 0 {
        k -= 1;
        let (sx, sy) = sn.seg_pos(k);
        let cx = (sx - wx0) >> 2;
        let cy = (sy - wy0) >> 2;
        if cx < -6 || cx >= FB_W as i32 + 6 || cy < -6 || cy >= FB_H as i32 + 6 {
            continue;
        }
        // Taper the final 2 tail segments slightly for a sleek worm silhouette!
        let seg_r = if k + 1 == segs && r_cell > 2 {
            r_cell - 1
        } else {
            r_cell
        };
        let col = skin_segment_color(skin, k, g.tick, boosting);
        fb.disc(cx, cy, seg_r, col);
    }

    // Cute expressive Slither.io eyes on the head looking in the heading direction!
    let (hx, hy) = sn.seg_pos(0);
    let hcx = (hx - wx0) >> 2;
    let hcy = (hy - wy0) >> 2;
    if hcx >= -6 && hcx < FB_W as i32 + 6 && hcy >= -6 && hcy < FB_H as i32 + 6 {
        draw_snake_eyes(fb, hcx, hcy, r_cell, sn.ang);
    }
}

#[inline]
fn draw_snake_eyes(fb: &mut FrameBuf, hcx: i32, hcy: i32, r_cell: i32, ang: u16) {
    let fx = cos(ang);
    let fy = sin(ang);
    // Perpendicular left/right vector
    let px = -fy;
    let py = fx;

    let fwd = (r_cell * 5) / 8;
    let sep = ((r_cell * 6) / 10).max(1);

    let base_x = hcx + ((fx * fwd + 8192) >> 14);
    let base_y = hcy + ((fy * fwd + 8192) >> 14);
    let off_x = (px * sep + 8192) >> 14;
    let off_y = (py * sep + 8192) >> 14;

    // Pupil nudge in look direction
    let pup_x = if fx > 5500 { 1 } else if fx < -5500 { -1 } else { 0 };
    let pup_y = if fy > 5500 { 1 } else if fy < -5500 { -1 } else { 0 };

    // Left eye & Right eye
    for sign in [-1i32, 1i32] {
        let ex = base_x + off_x * sign;
        let ey = base_y + off_y * sign;
        // Crisp 2x2 white eye sclera (8x8 px on watch!)
        fb.set(ex, ey, PAL_EYE_WHITE);
        fb.set(ex + 1, ey, PAL_EYE_WHITE);
        fb.set(ex, ey + 1, PAL_EYE_WHITE);
        fb.set(ex + 1, ey + 1, PAL_EYE_WHITE);
        // Dark pupil staring forward!
        let px_c = ex + if pup_x > 0 { 1 } else { 0 };
        let py_c = ey + if pup_y > 0 { 1 } else { 0 };
        fb.set(px_c, py_c, PAL_EYE_PUPIL);
    }
}

/// Compact circular radar minimap composited into bottom-right of `fb`.
fn draw_minimap(fb: &mut FrameBuf, g: &Game) {
    fb.disc(MAP_CX, MAP_CY, MAP_R + 1, PAL_MAP_RING);
    fb.disc(MAP_CX, MAP_CY, MAP_R, PAL_MAP_BG);

    let scale = MAP_R - 1; // 10 cells = ARENA_R

    // Highlight any huge death-loot orbs on radar so player can rush kills!
    let mut oi = 0usize;
    while oi < MAX_ORBS {
        if g.orbs[oi].val >= 5 {
            let mx = MAP_CX + (g.orbs[oi].x as i32 * scale) / ARENA_R;
            let my = MAP_CY + (g.orbs[oi].y as i32 * scale) / ARENA_R;
            fb.set(mx, my, PAL_MAP_LOOT);
        }
        oi += 1;
    }

    // Bots (giant bosses in bright orange, others in dim steel)
    let mut i = 1usize;
    while i < MAX_SNAKES {
        if g.snakes[i].alive {
            let mx = MAP_CX + (g.snakes[i].head_wx() * scale) / ARENA_R;
            let my = MAP_CY + (g.snakes[i].head_wy() * scale) / ARENA_R;
            if g.snakes[i].mass >= 150 {
                fb.set(mx, my, PAL_MAP_BOSS);
                fb.set(mx + 1, my, PAL_MAP_BOSS);
            } else {
                fb.set(mx, my, PAL_MAP_BOT);
            }
        }
        i += 1;
    }

    // Player bright white 2x2 marker
    if g.snakes[0].alive {
        let mx = MAP_CX + (g.snakes[0].head_wx() * scale) / ARENA_R;
        let my = MAP_CY + (g.snakes[0].head_wy() * scale) / ARENA_R;
        fb.set(mx, my, PAL_MAP_YOU);
        fb.set(mx + 1, my, PAL_MAP_YOU);
        fb.set(mx, my + 1, PAL_MAP_YOU);
        fb.set(mx + 1, my + 1, PAL_MAP_YOU);
    }
}

/// Pre-composite dark HUD pill directly into `fb` so text draws over it with zero flicker!
fn draw_hud_pill_bg(fb: &mut FrameBuf, boosting: bool) {
    let border = if boosting { PAL_BOOST_AURA } else { PAL_MAP_RING };
    let mut y = HUD_CY0;
    while y <= HUD_CY1 {
        let is_edge = y == HUD_CY0 || y == HUD_CY1;
        let x0 = if is_edge { HUD_CX0 + 1 } else { HUD_CX0 };
        let x1 = if is_edge { HUD_CX1 - 1 } else { HUD_CX1 };
        if is_edge {
            fb.hline(x0 as i32, x1 as i32, y as i32, border);
        } else {
            fb.set(x0 as i32, y as i32, border);
            fb.hline((x0 + 1) as i32, (x1 - 1) as i32, y as i32, PAL_MAP_BG);
            fb.set(x1 as i32, y as i32, border);
        }
        y += 1;
    }
}

/// Crisp native 16px HUD text inside the pre-composited top pill + kill/boost indicators.
fn draw_hud_text(s: &mut impl Surface, g: &Game) {
    let (rank, total) = g.player_rank();
    let len = g.snakes[0].mass as i32;

    // Format compact single-line HUD: `LEN 142 #3/8` (12 chars = 192 px inside 216 px pill!)
    let mut nb = [0u8; 11];
    let len_str = int(len, &mut nb);
    let mut line = [b' '; 16];
    let mut pos = 0usize;
    for &b in b"LEN " {
        line[pos] = b;
        pos += 1;
    }
    for &b in len_str.as_bytes() {
        if pos < 10 {
            line[pos] = b;
            pos += 1;
        }
    }
    line[pos] = b' ';
    pos += 1;
    line[pos] = b'#';
    pos += 1;
    line[pos] = b'0' + rank.min(9);
    pos += 1;
    line[pos] = b'/';
    pos += 1;
    line[pos] = b'0' + total.min(9);
    pos += 1;

    let hud_str = core::str::from_utf8(&line[..pos]).unwrap_or("LEN");
    let hud_y = FB_OFFSET + (HUD_CY0 as i32) * FB_SCALE + 8;
    let col = if rank == 1 { UI_GOLD } else { UI_WHITE };
    centre(s, C, hud_y, hud_str, col);

    // Kill celebration flash!
    if g.kill_flash_ticks > 0 {
        centre(s, C, hud_y + 28, "KILL! +1", UI_GOLD);
    }

    // Subtle bottom boost feedback (only when holding touch!)
    if g.boost_held {
        if g.snakes[0].mass > MIN_BOOST_MASS {
            centre(s, C, SIZE - 42, "BOOST!", UI_GOLD);
        } else {
            centre(s, C, SIZE - 42, "NEED LEN 19", UI_SILVER);
        }
    }
}

fn draw_pill(s: &mut impl Surface, r: [i32; 4], fill: u16, border: u16, label: &str, fg: u16) {
    let [x, y, w, h] = r;
    s.rect(x + 3, y, w - 6, h, border);
    s.rect(x, y + 3, w, h - 6, border);
    s.rect(x + 2, y + 2, w - 4, h - 4, fill);
    centre(s, x + w / 2, y + (h - 16) / 2, label, fg);
}

/// Renders the animated Slither.io Title Screen with live skin preview worm!
pub fn render_title(s: &mut impl Surface, fb: &mut FrameBuf, g: &Game) {
    // 1. Render subtle hex floor + curling animated preview snake in `fb`
    fb.px.fill(PAL_BG);
    let scroll = (g.tick as i32) * 2;
    draw_hex_floor(fb, scroll, scroll / 2);

    // Draw a live slithering preview snake horizontally across y = 48 (px y = 193)
    let skin = g.selected_skin;
    let preview_segs = 18usize;
    let mut k = preview_segs;
    while k > 0 {
        k -= 1;
        let phase = ((g.tick as i32 * 520) - (k as i32 * 3600)) as u16;
        let cx = 76 - (k as i32) * 2;
        let cy = 48 + ((sin(phase) * 5) >> 14);
        let col = skin_segment_color(skin, k, g.tick, false);
        fb.disc(cx, cy, if k + 1 == preview_segs { 2 } else { 3 }, col);
    }
    let head_phase = (g.tick as i32 * 520) as u16;
    let hcy = 48 + ((sin(head_phase) * 5) >> 14);
    draw_snake_eyes(fb, 76, hcy, 3, 0);

    // Subtle glowing orbs near the preview snake's mouth
    fb.disc(88, 46, 2, orb_palette(skin, false));
    fb.set(88, 46, orb_palette(skin, true));
    fb.set(96, 51, orb_palette((skin + 3) & 7, true));

    fb.flush_round(s);

    // 2. Crisp Title Header
    let accent = skin_ui_color(skin);
    let tw = big_width("SLITHER", 4);
    big(s, C - tw / 2 + 2, 74, "SLITHER", 4, UI_BG);
    big(s, C - tw / 2, 72, "SLITHER", 4, accent);
    centre(s, C, 114, "ARENA + SMART BOTS", UI_SILVER);

    // 3. Best record line (if played before)
    if g.save.best_len > 0 {
        let mut b1 = [0u8; 11];
        let mut b2 = [0u8; 11];
        let bl = int(g.save.best_len as i32, &mut b1);
        let bk = int(g.save.best_kills as i32, &mut b2);
        let mut buf = [b' '; 24];
        let mut p = 0;
        for &ch in b"BEST " { buf[p] = ch; p += 1; }
        for &ch in bl.as_bytes() { if p < 14 { buf[p] = ch; p += 1; } }
        for &ch in b"  K:" { buf[p] = ch; p += 1; }
        for &ch in bk.as_bytes() { if p < 24 { buf[p] = ch; p += 1; } }
        let rec = core::str::from_utf8(&buf[..p]).unwrap_or("BEST");
        centre(s, C, 140, rec, UI_GOLD);
    }

    // 4. Skin selector `< SKIN >`
    draw_pill(s, TITLE_PREV_RECT, UI_PANEL, UI_BORDER, "<", UI_WHITE);
    draw_pill(s, TITLE_NEXT_RECT, UI_PANEL, UI_BORDER, ">", UI_WHITE);
    let name = SKIN_NAMES[(skin as usize) % SKIN_NAMES.len()];
    centre(s, C, 266, name, accent);

    // 5. Tactile PLAY button + controls legend (cleanly inside round watch bezel!)
    draw_pill(s, TITLE_PLAY_RECT, UI_LIME, UI_WHITE, "PLAY", UI_BG);
    centre(s, C, 384, "CROWN: STEER", UI_SILVER);
    centre(s, C, 404, "HOLD: BOOST", UI_SILVER);
}

/// Renders the clean Game Over summary screen.
pub fn render_over(s: &mut impl Surface, g: &Game) {
    s.rect(0, 0, SIZE, SIZE, UI_BG);

    // Top banner
    if g.new_best_achieved {
        centre(s, C, 56, "NEW RECORD!", UI_GOLD);
    } else {
        centre(s, C, 56, "ELIMINATED", UI_RED);
    }

    let cause = match g.death_cause {
        DeathCause::Wall => "HIT ARENA WALL",
        DeathCause::Snake => "CRASHED INTO SNAKE",
        DeathCause::None => "GAME OVER",
    };
    centre(s, C, 84, cause, UI_SILVER);

    // Huge final length score
    let mut b_len = [0u8; 11];
    let ls = int(g.final_len as i32, &mut b_len);
    let lw = big_width(ls, 5);
    big(s, C - lw / 2, 122, ls, 5, UI_WHITE);
    centre(s, C, 174, "FINAL LENGTH", UI_CYAN);

    // Stats card
    s.rect(C - 134, 206, 268, 76, UI_BORDER);
    s.rect(C - 132, 208, 264, 72, UI_PANEL);

    let mut b_best = [0u8; 11];
    let mut b_kills = [0u8; 11];
    let best_s = int(g.save.best_len as i32, &mut b_best);
    let kill_s = int(g.kills as i32, &mut b_kills);

    // Row 1: BEST & KILLS
    let mut r1 = [b' '; 24];
    let mut p = 0;
    for &c in b"BEST " { r1[p] = c; p += 1; }
    for &c in best_s.as_bytes() { if p < 12 { r1[p] = c; p += 1; } }
    for &c in b"  KILLS " { r1[p] = c; p += 1; }
    for &c in kill_s.as_bytes() { if p < 24 { r1[p] = c; p += 1; } }
    centre(s, C, 220, core::str::from_utf8(&r1[..p]).unwrap_or("BEST"), UI_GOLD);

    // Row 2: PEAK RANK
    let mut r2 = [b' '; 16];
    let mut q = 0;
    for &c in b"TOP RANK #" { r2[q] = c; q += 1; }
    r2[q] = b'0' + g.best_rank.min(9);
    q += 1;
    centre(s, C, 250, core::str::from_utf8(&r2[..q]).unwrap_or("RANK"), UI_ORANGE);

    // Tactile replay / skin buttons
    draw_pill(s, OVER_PLAY_RECT, UI_LIME, UI_WHITE, "PLAY AGAIN", UI_BG);
    draw_pill(s, OVER_SKIN_RECT, UI_PANEL, UI_BORDER, "SKINS", UI_WHITE);
}

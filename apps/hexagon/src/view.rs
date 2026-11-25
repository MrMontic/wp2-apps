//! Single-pass analytical horizontal scanline-band renderer for Super Hexagon.
//!
//! Architecture (zero flicker, zero per-pixel trig, ~600–850 rects/frame on 466x466 AMOLED):
//! 1. Precomputed compile-time band table (`128` bands: 4px outer, 3px center high-res zone)
//!    clipped to the round 466px bezel (`r = 231`).
//! 2. Per frame: precompute `N` sector ray `(cos, sin)`, active wall 4-vertex trapezoids,
//!    pulsing center polygon (outer + inner), and orbiting 3-vertex player triangle into
//!    `RenderScratch` (kept in `static mut` on watch to stay well below the 8 KiB stack limit).
//! 3. For each band `y`:
//!    - Intersect the `N` background sector rays analytically in O(1) -> <= 4 background spans.
//!    - Scan-convert active wall trapezoids, center polygon, player triangle, and HUD/UI cards
//!      into a tiny sorted, adjacent-merged horizontal span list (`MAX_SPANS = 32`).
//!    - Emit one `s.rect(x0, y0, w, h, col)` per final merged span: every pixel on screen is
//!      written ONCE in painter's order (ZERO full-screen black clears, ZERO flicker!).

use crate::game::{CENTER_RIM, Game, MAX_WALLS, Mode, PLAYER_R, State, fmt_time};
use cmf_wp2_ui::color::{Rgb, rgb565};
use cmf_wp2_ui::motion::{cos, sin};
use cmf_wp2_ui::text::{big, big_width, centre};
use cmf_wp2_ui::{C, ONE, SIZE, Surface};

const SCREEN_R: i32 = 231;
const BAND_COUNT: usize = 86;
const MAX_SPANS: usize = 32;

const WHITE: u16 = 0xFFFF;
const BLACK: u16 = 0x0000;
const HUD_BG: u16 = rgb565(8, 10, 14);
const CARD_BG: u16 = rgb565(10, 12, 18);
const BTN_SEC_BG: u16 = rgb565(34, 38, 52);
const TEXT_SOFT: u16 = rgb565(198, 204, 218);
const TEXT_DIM: u16 = rgb565(132, 140, 158);
const GOLD: u16 = rgb565(255, 224, 68);

/// One horizontal scanline band clipped to the round display.
#[derive(Clone, Copy)]
struct Band {
    y0: i16,
    h: u8,
    y_mid: i16,
    x_min: i16,
    x_max: i16,
}

const fn isqrt_const(n: i32) -> i32 {
    if n <= 0 {
        return 0;
    }
    let mut x = n as u32;
    let mut r = 0u32;
    let mut bit = 1u32 << 30;
    while bit > x {
        bit >>= 2;
    }
    while bit != 0 {
        if x >= r + bit {
            x -= r + bit;
            r = (r >> 1) + bit;
        } else {
            r >>= 1;
        }
        bit >>= 2;
    }
    r as i32
}

const fn build_bands() -> [Band; BAND_COUNT] {
    let mut out = [Band {
        y0: 0,
        h: 4,
        y_mid: 0,
        x_min: 0,
        x_max: 0,
    }; BAND_COUNT];
    let mut i = 0usize;
    let mut y = 7i32;

    // 27 outer top bands (h = 6, y = 7..169)
    // 32 center high-res bands (h = 4, y = 169..297) -> covers entire center hex + player orbit, y_mid never equals C (233)
    // 27 outer bottom bands (h = 6, y = 297..459)
    while i < BAND_COUNT {
        let h = if i >= 27 && i < 59 { 4i32 } else { 6i32 };
        let y_mid = y + h / 2;
        let dy = y_mid - C;
        let hw = isqrt_const(SCREEN_R * SCREEN_R - dy * dy);
        let x_min = if C - hw < 0 { 0 } else { C - hw };
        let x_max = if C + hw > SIZE { SIZE } else { C + hw };
        out[i] = Band {
            y0: y as i16,
            h: h as u8,
            y_mid: y_mid as i16,
            x_min: x_min as i16,
            x_max: x_max as i16,
        };
        y += h;
        i += 1;
    }
    out
}

/// Precomputed band geometry stored in `.rodata` (0 bytes of RAM).
const BANDS: [Band; BAND_COUNT] = build_bands();

#[derive(Clone, Copy)]
struct RawPalette {
    bg_a: Rgb,
    bg_b: Rgb,
    wall: Rgb,
    core: Rgb,
}

const PAL_EASY: [RawPalette; 5] = [
    RawPalette { bg_a: Rgb(6, 40, 28), bg_b: Rgb(14, 66, 46), wall: Rgb(76, 255, 168), core: Rgb(4, 20, 14) },
    RawPalette { bg_a: Rgb(8, 34, 52), bg_b: Rgb(16, 56, 84), wall: Rgb(92, 224, 255), core: Rgb(4, 18, 28) },
    RawPalette { bg_a: Rgb(16, 44, 14), bg_b: Rgb(28, 72, 24), wall: Rgb(156, 255, 102), core: Rgb(8, 22, 8) },
    RawPalette { bg_a: Rgb(14, 28, 56), bg_b: Rgb(24, 48, 92), wall: Rgb(132, 196, 255), core: Rgb(8, 14, 30) },
    RawPalette { bg_a: Rgb(6, 44, 40), bg_b: Rgb(14, 72, 66), wall: Rgb(72, 255, 228), core: Rgb(4, 22, 20) },
];

const PAL_MEDIUM: [RawPalette; 5] = [
    RawPalette { bg_a: Rgb(44, 28, 6), bg_b: Rgb(72, 48, 12), wall: Rgb(255, 194, 62), core: Rgb(22, 14, 4) },
    RawPalette { bg_a: Rgb(8, 36, 54), bg_b: Rgb(18, 60, 88), wall: Rgb(68, 218, 255), core: Rgb(4, 18, 28) },
    RawPalette { bg_a: Rgb(46, 18, 8), bg_b: Rgb(74, 32, 16), wall: Rgb(255, 134, 64), core: Rgb(24, 10, 4) },
    RawPalette { bg_a: Rgb(24, 18, 56), bg_b: Rgb(40, 32, 90), wall: Rgb(164, 144, 255), core: Rgb(12, 10, 30) },
    RawPalette { bg_a: Rgb(12, 44, 32), bg_b: Rgb(22, 72, 52), wall: Rgb(96, 255, 184), core: Rgb(6, 22, 16) },
];

const PAL_HEXAGON: [RawPalette; 5] = [
    RawPalette { bg_a: Rgb(8, 34, 48), bg_b: Rgb(15, 56, 76), wall: Rgb(48, 240, 255), core: Rgb(5, 18, 28) },
    RawPalette { bg_a: Rgb(48, 10, 18), bg_b: Rgb(74, 18, 32), wall: Rgb(255, 72, 102), core: Rgb(26, 6, 12) },
    RawPalette { bg_a: Rgb(48, 34, 6), bg_b: Rgb(74, 54, 12), wall: Rgb(255, 214, 52), core: Rgb(26, 18, 4) },
    RawPalette { bg_a: Rgb(32, 12, 54), bg_b: Rgb(52, 22, 84), wall: Rgb(198, 108, 255), core: Rgb(18, 6, 30) },
    RawPalette { bg_a: Rgb(8, 44, 26), bg_b: Rgb(16, 68, 42), wall: Rgb(66, 255, 156), core: Rgb(5, 22, 14) },
];

const PAL_HEXAGONER: [RawPalette; 5] = [
    RawPalette { bg_a: Rgb(50, 8, 42), bg_b: Rgb(80, 16, 68), wall: Rgb(255, 68, 220), core: Rgb(24, 4, 20) },
    RawPalette { bg_a: Rgb(54, 22, 6), bg_b: Rgb(84, 38, 12), wall: Rgb(255, 140, 38), core: Rgb(28, 12, 4) },
    RawPalette { bg_a: Rgb(24, 48, 6), bg_b: Rgb(40, 76, 14), wall: Rgb(180, 255, 48), core: Rgb(12, 24, 4) },
    RawPalette { bg_a: Rgb(10, 22, 60), bg_b: Rgb(20, 40, 96), wall: Rgb(92, 174, 255), core: Rgb(6, 12, 32) },
    RawPalette { bg_a: Rgb(56, 14, 24), bg_b: Rgb(86, 26, 42), wall: Rgb(255, 108, 138), core: Rgb(28, 8, 12) },
];

const PAL_HEXAGONEST: [RawPalette; 5] = [
    RawPalette { bg_a: Rgb(12, 12, 16), bg_b: Rgb(30, 30, 40), wall: Rgb(248, 248, 255), core: Rgb(6, 6, 10) },
    RawPalette { bg_a: Rgb(42, 4, 8), bg_b: Rgb(72, 10, 16), wall: Rgb(255, 44, 60), core: Rgb(16, 2, 4) },
    RawPalette { bg_a: Rgb(40, 32, 4), bg_b: Rgb(66, 54, 10), wall: Rgb(255, 244, 58), core: Rgb(16, 12, 2) },
    RawPalette { bg_a: Rgb(6, 30, 48), bg_b: Rgb(16, 54, 84), wall: Rgb(124, 248, 255), core: Rgb(4, 12, 22) },
    RawPalette { bg_a: Rgb(32, 6, 52), bg_b: Rgb(58, 14, 92), wall: Rgb(238, 132, 255), core: Rgb(14, 4, 22) },
];

#[derive(Clone, Copy)]
struct FramePalette {
    bg_a: u16,
    bg_b: u16,
    bg_c: u16, // third subtle shade for odd 5-sided Pentagon mode
    wall: u16,
    rim: u16,
    core: u16,
    player: u16,
}

fn resolve_palette(g: &Game) -> FramePalette {
    let table = match g.mode {
        Mode::Easy => &PAL_EASY,
        Mode::Medium => &PAL_MEDIUM,
        Mode::Hexagon => &PAL_HEXAGON,
        Mode::Hexagoner => &PAL_HEXAGONER,
        Mode::Hexagonest => &PAL_HEXAGONEST,
    };
    let a = table[(g.palette_idx as usize) % 5];
    let b = table[(g.palette_next as usize) % 5];
    let t = (g.palette_blend as i32 * ONE) >> 8;

    let mut bg_a = a.bg_a.mix(b.bg_a, t);
    let mut bg_b = a.bg_b.mix(b.bg_b, t);
    let mut wall = a.wall.mix(b.wall, t);
    let mut core = a.core.mix(b.core, t);

    // Beat pulse subtle luminance lift
    let pulse_k = ONE + ((g.pulse * 140) >> 8);
    bg_b = bg_b.scale(pulse_k);
    let mut rim = wall.mix(Rgb(255, 255, 255), (g.pulse * 320) >> 8);
    let mut player = Rgb(255, 255, 255);

    match g.state {
        State::Dying => {
            if g.dying_ticks >= 7 {
                // Dramatic impact strobe flash
                let flash_t = ((g.dying_ticks as i32 - 6) * ONE) / 5;
                bg_a = bg_a.mix(Rgb(140, 24, 32), flash_t);
                bg_b = bg_b.mix(Rgb(200, 36, 48), flash_t);
                wall = wall.mix(Rgb(255, 255, 255), flash_t);
                rim = Rgb(255, 255, 255);
                player = Rgb(255, 240, 120);
            }
        }
        State::Over => {
            // Dim frozen arena slightly so Game Over card pops cleanly
            bg_a = bg_a.scale(620);
            bg_b = bg_b.scale(620);
            wall = wall.scale(700);
            core = core.scale(700);
            rim = rim.scale(740);
        }
        State::Title => {
            bg_a = bg_a.scale(860);
            bg_b = bg_b.scale(860);
            wall = wall.scale(880);
        }
        State::Playing => {}
    }

    let bg_c = bg_a.mix(bg_b, ONE / 2);

    FramePalette {
        bg_a: bg_a.rgb565(),
        bg_b: bg_b.rgb565(),
        bg_c: bg_c.rgb565(),
        wall: wall.rgb565(),
        rim: rim.rgb565(),
        core: core.rgb565(),
        player: player.rgb565(),
    }
}

/// Precomputed 4-vertex wall trapezoid for the current frame.
#[derive(Clone, Copy)]
struct WallPoly {
    vx: [i16; 4],
    vy: [i16; 4],
    y_min: i16,
    y_max: i16,
}

impl WallPoly {
    const EMPTY: Self = Self {
        vx: [0; 4],
        vy: [0; 4],
        y_min: 0,
        y_max: 0,
    };
}

/// Reusable per-frame geometry scratchpad (~820 bytes; store in `static mut` on watch!).
pub struct RenderScratch {
    ray_cos: [i32; 6],
    ray_sin: [i32; 6],
    walls: [WallPoly; MAX_WALLS],
    wall_count: usize,
    center_ox: [i16; 6],
    center_oy: [i16; 6],
    center_oy_min: i16,
    center_oy_max: i16,
    center_ix: [i16; 6],
    center_iy: [i16; 6],
    center_iy_min: i16,
    center_iy_max: i16,
    player_x: [i16; 3],
    player_y: [i16; 3],
    player_y_min: i16,
    player_y_max: i16,
}

impl RenderScratch {
    pub const fn new() -> Self {
        Self {
            ray_cos: [0; 6],
            ray_sin: [0; 6],
            walls: [WallPoly::EMPTY; MAX_WALLS],
            wall_count: 0,
            center_ox: [0; 6],
            center_oy: [0; 6],
            center_oy_min: 0,
            center_oy_max: 0,
            center_ix: [0; 6],
            center_iy: [0; 6],
            center_iy_min: 0,
            center_iy_max: 0,
            player_x: [0; 3],
            player_y: [0; 3],
            player_y_min: 0,
            player_y_max: 0,
        }
    }
}

#[derive(Clone, Copy)]
struct Span {
    x0: i16,
    x1: i16,
    col: u16,
}

impl Span {
    const EMPTY: Self = Self { x0: 0, x1: 0, col: 0 };
}

#[inline]
fn push_merged(out: &mut [Span; MAX_SPANS], len: &mut usize, x0: i16, x1: i16, col: u16) {
    if x0 >= x1 {
        return;
    }
    if *len > 0 && out[*len - 1].col == col && out[*len - 1].x1 == x0 {
        out[*len - 1].x1 = x1;
        return;
    }
    if *len < MAX_SPANS {
        out[*len] = Span { x0, x1, col };
        *len += 1;
    }
}

/// Overlay `[fx0, fx1)` with `fcol` onto sorted span list `spans[..*len]`, merging adjacent runs.
fn paint_span(
    spans: &mut [Span; MAX_SPANS],
    len: &mut usize,
    clip_min: i16,
    clip_max: i16,
    raw_x0: i16,
    raw_x1: i16,
    fcol: u16,
) {
    let fx0 = raw_x0.max(clip_min);
    let fx1 = raw_x1.min(clip_max);
    if fx0 >= fx1 {
        return;
    }
    let mut next = [Span::EMPTY; MAX_SPANS];
    let mut nlen = 0usize;
    let mut inserted = false;
    let mut i = 0usize;
    while i < *len {
        let sp = spans[i];
        if sp.x1 <= fx0 {
            push_merged(&mut next, &mut nlen, sp.x0, sp.x1, sp.col);
        } else if sp.x0 >= fx1 {
            if !inserted {
                push_merged(&mut next, &mut nlen, fx0, fx1, fcol);
                inserted = true;
            }
            push_merged(&mut next, &mut nlen, sp.x0, sp.x1, sp.col);
        } else {
            if sp.x0 < fx0 {
                push_merged(&mut next, &mut nlen, sp.x0, fx0, sp.col);
            }
            if !inserted {
                push_merged(&mut next, &mut nlen, fx0, fx1, fcol);
                inserted = true;
            }
            if sp.x1 > fx1 {
                push_merged(&mut next, &mut nlen, fx1, sp.x1, sp.col);
            }
        }
        i += 1;
    }
    if !inserted {
        push_merged(&mut next, &mut nlen, fx0, fx1, fcol);
    }
    spans[..nlen].copy_from_slice(&next[..nlen]);
    *len = nlen;
}

/// Exact horizontal scan-conversion of a convex `n`-gon at scanline `y`.
#[inline]
fn poly_span(vx: &[i16], vy: &[i16], y: i32) -> Option<(i16, i16)> {
    let n = vx.len();
    let mut x_lo = i32::MAX;
    let mut x_hi = i32::MIN;
    let mut j = n - 1;
    let mut i = 0;
    while i < n {
        let y0 = vy[j] as i32;
        let y1 = vy[i] as i32;
        if (y0 <= y && y1 > y) || (y1 <= y && y0 > y) {
            let x0 = vx[j] as i32;
            let x1 = vx[i] as i32;
            let x = x0 + (y - y0) * (x1 - x0) / (y1 - y0);
            if x < x_lo {
                x_lo = x;
            }
            if x > x_hi {
                x_hi = x;
            }
        }
        j = i;
        i += 1;
    }
    if x_lo <= x_hi {
        Some((x_lo as i16, (x_hi + 1) as i16))
    } else {
        None
    }
}

#[inline]
fn sector_color(sec: u8, n: u8, pal: &FramePalette) -> u16 {
    if n == 5 && sec == 4 {
        pal.bg_c
    } else if (sec & 1) == 0 {
        pal.bg_a
    } else {
        pal.bg_b
    }
}

/// Populate `spans` with exact O(1) ray-intersected background sector spans for row `y_mid`.
fn fill_bg_spans(
    spans: &mut [Span; MAX_SPANS],
    len: &mut usize,
    scratch: &RenderScratch,
    n: u8,
    cx: i32,
    cy: i32,
    y_mid: i32,
    x_min: i16,
    x_max: i16,
    pal: &FramePalette,
) {
    *len = 0;
    let raw_dy = y_mid - cy;
    let dy = if raw_dy == 0 { 1 } else { raw_dy };

    let mut hits = [(0i32, 0u8); 6];
    let mut nhits = 0usize;
    let mut k = 0u8;
    while k < n {
        let rs = scratch.ray_sin[k as usize];
        if (dy > 0 && rs > 0) || (dy < 0 && rs < 0) {
            let rc = scratch.ray_cos[k as usize];
            let x_hit = cx + (dy * rc) / rs;
            hits[nhits] = (x_hit, k);
            nhits += 1;
        }
        k += 1;
    }

    if nhits == 0 {
        push_merged(spans, len, x_min, x_max, pal.bg_a);
        return;
    }

    // Insertion sort <= 3 hits ascending by x_hit
    let mut i = 1usize;
    while i < nhits {
        let key = hits[i];
        let mut j = i;
        while j > 0 && hits[j - 1].0 > key.0 {
            hits[j] = hits[j - 1];
            j -= 1;
        }
        hits[j] = key;
        i += 1;
    }

    let x_min_i32 = x_min as i32;
    let x_max_i32 = x_max as i32;

    // First interval to the left of hits[0]
    let first_sec = if dy > 0 {
        hits[0].1
    } else {
        (hits[0].1 + n - 1) % n
    };
    let mut prev_x = x_min_i32;
    let mut seg_end = hits[0].0.clamp(x_min_i32, x_max_i32);
    if seg_end > prev_x {
        push_merged(
            spans,
            len,
            prev_x as i16,
            seg_end as i16,
            sector_color(first_sec, n, pal),
        );
        prev_x = seg_end;
    }

    // Intervals to the right of each hit
    let mut idx = 0usize;
    while idx < nhits {
        let sec = if dy > 0 {
            (hits[idx].1 + n - 1) % n
        } else {
            hits[idx].1
        };
        let next_raw = if idx + 1 < nhits {
            hits[idx + 1].0
        } else {
            x_max_i32
        };
        seg_end = next_raw.clamp(x_min_i32, x_max_i32);
        if seg_end > prev_x {
            push_merged(
                spans,
                len,
                prev_x as i16,
                seg_end as i16,
                sector_color(sec, n, pal),
            );
            prev_x = seg_end;
        }
        idx += 1;
    }

    // Safety guarantee: cover right tail if anything remains
    if prev_x < x_max_i32 {
        let last_sec = if dy > 0 {
            (hits[nhits - 1].1 + n - 1) % n
        } else {
            hits[nhits - 1].1
        };
        push_merged(
            spans,
            len,
            prev_x as i16,
            x_max,
            sector_color(last_sec, n, pal),
        );
    }
}

/// Precompute all polygon vertices for the current frame into `scratch`.
fn prepare_geometry(g: &Game, scratch: &mut RenderScratch) -> (i32, i32) {
    let cx = C + g.shake_x as i32;
    let cy = C + g.shake_y as i32;
    let n = g.sides.clamp(4, 6);
    let nu = n as u32;

    // 1. Precompute N spoke rays
    let mut k = 0u32;
    while k < nu {
        let a = g
            .world_angle
            .wrapping_add(((k * 65536 + nu / 2) / nu) as u16);
        scratch.ray_cos[k as usize] = cos(a);
        scratch.ray_sin[k as usize] = sin(a);
        k += 1;
    }

    // 2. Center polygon (outer + inner core)
    let r_out = g.center_r();
    let r_in = (r_out - CENTER_RIM).max(12);
    let (mut oy_min, mut oy_max) = (i16::MAX, i16::MIN);
    let (mut iy_min, mut iy_max) = (i16::MAX, i16::MIN);
    let mut i = 0usize;
    while i < n as usize {
        let rc = scratch.ray_cos[i];
        let rs = scratch.ray_sin[i];
        let ox = (cx + ((r_out * rc) >> 14)) as i16;
        let oy = (cy + ((r_out * rs) >> 14)) as i16;
        let ix = (cx + ((r_in * rc) >> 14)) as i16;
        let iy = (cy + ((r_in * rs) >> 14)) as i16;
        scratch.center_ox[i] = ox;
        scratch.center_oy[i] = oy;
        scratch.center_ix[i] = ix;
        scratch.center_iy[i] = iy;
        if oy < oy_min { oy_min = oy; }
        if oy > oy_max { oy_max = oy; }
        if iy < iy_min { iy_min = iy; }
        if iy > iy_max { iy_max = iy; }
        i += 1;
    }
    scratch.center_oy_min = oy_min;
    scratch.center_oy_max = oy_max;
    scratch.center_iy_min = iy_min;
    scratch.center_iy_max = iy_max;

    // 3. Active wall trapezoids
    let mut wcount = 0usize;
    let clip_r_in = r_out - 1;
    for w in &g.walls {
        if !w.active || wcount >= MAX_WALLS {
            continue;
        }
        let wr_in = (w.r_in_q8 >> 8).max(clip_r_in);
        let wr_out = ((w.r_in_q8 + w.thick_q8) >> 8).min(SCREEN_R + 6);
        if wr_out <= wr_in || wr_in > SCREEN_R + 4 {
            continue;
        }
        let s0 = (w.sector % n) as usize;
        let s1 = (s0 + 1) % (n as usize);
        let (c0, sn0) = (scratch.ray_cos[s0], scratch.ray_sin[s0]);
        let (c1, sn1) = (scratch.ray_cos[s1], scratch.ray_sin[s1]);

        let vx = [
            (cx + ((wr_in * c0) >> 14)) as i16,
            (cx + ((wr_out * c0) >> 14)) as i16,
            (cx + ((wr_out * c1) >> 14)) as i16,
            (cx + ((wr_in * c1) >> 14)) as i16,
        ];
        let vy = [
            (cy + ((wr_in * sn0) >> 14)) as i16,
            (cy + ((wr_out * sn0) >> 14)) as i16,
            (cy + ((wr_out * sn1) >> 14)) as i16,
            (cy + ((wr_in * sn1) >> 14)) as i16,
        ];
        let mut ymin = vy[0];
        let mut ymax = vy[0];
        let mut vi = 1;
        while vi < 4 {
            if vy[vi] < ymin { ymin = vy[vi]; }
            if vy[vi] > ymax { ymax = vy[vi]; }
            vi += 1;
        }
        scratch.walls[wcount] = WallPoly {
            vx,
            vy,
            y_min: ymin,
            y_max: ymax,
        };
        wcount += 1;
    }
    scratch.wall_count = wcount;

    // 4. Player cursor triangle (pointing outward, orbiting just outside center polygon)
    let pa = g.world_angle.wrapping_add(g.player_angle);
    let r_tip = PLAYER_R + 7;
    let r_base = PLAYER_R - 5;
    let a_l = pa.wrapping_sub(1950);
    let a_r = pa.wrapping_add(1950);
    let px = [
        (cx + ((r_tip * cos(pa)) >> 14)) as i16,
        (cx + ((r_base * cos(a_l)) >> 14)) as i16,
        (cx + ((r_base * cos(a_r)) >> 14)) as i16,
    ];
    let py = [
        (cy + ((r_tip * sin(pa)) >> 14)) as i16,
        (cy + ((r_base * sin(a_l)) >> 14)) as i16,
        (cy + ((r_base * sin(a_r)) >> 14)) as i16,
    ];
    scratch.player_x = px;
    scratch.player_y = py;
    scratch.player_y_min = py[0].min(py[1]).min(py[2]);
    scratch.player_y_max = py[0].max(py[1]).max(py[2]);

    (cx, cy)
}

/// Clear the outer corners outside the circular arena (call ONCE on startup / resume!).
pub fn clear_screen(s: &mut impl Surface) {
    s.rect(0, 0, SIZE, SIZE, BLACK);
}

/// Draw the top HUD text (time + rank) during Playing/Dying.
fn draw_hud_text(s: &mut impl Surface, g: &Game, accent: u16) {
    let mut tbuf = [0u8; 12];
    let tstr = fmt_time(g.time_cs(), &mut tbuf);
    centre(s, C, 25, tstr, WHITE);
    centre(s, C, 43, g.rank.name(), accent);
}

/// Draw Title Screen text overlays (drawn directly over carved dark cards).
fn draw_title_overlay(s: &mut impl Surface, g: &Game, accent: u16) {
    // Title header
    let t1 = "SUPER";
    big(s, C - big_width(t1, 3) / 2, 70, t1, 3, WHITE);
    let t2 = "HEXAGON";
    big(s, C - big_width(t2, 4) / 2, 102, t2, 4, accent);

    // Mode card arrows & mode info
    big(s, 84, 212, "<", 3, TEXT_SOFT);
    big(s, 358, 212, ">", 3, TEXT_SOFT);
    centre(s, C, 188, g.mode.name(), WHITE);
    centre(s, C, 210, g.mode.subtitle(), accent);

    // Best time + rank for selected mode
    let best_cs = g.mode_best_cs();
    let mut tbuf = [0u8; 12];
    let ts = fmt_time(best_cs, &mut tbuf).as_bytes();
    let mut line = [0u8; 18];
    line[0..5].copy_from_slice(b"BEST ");
    let nlen = ts.len().min(10);
    line[5..5 + nlen].copy_from_slice(&ts[..nlen]);
    line[5 + nlen] = b's';
    if let Ok(lbl) = core::str::from_utf8(&line[..6 + nlen]) {
        centre(s, C, 240, lbl, TEXT_SOFT);
    }
    let rank = crate::game::Rank::from_cs(best_cs);
    centre(s, C, 260, rank.name(), if best_cs > 0 { accent } else { TEXT_DIM });

    // PLAY button label (drawn on bright accent pill)
    centre(s, C, 331, "PLAY", BLACK);

    // Mode indicator dots (5 tiny squares for EASY / MEDIUM / HEXAGON / HEXAGONER / HEXAGONEST)
    for m in 0..Mode::COUNT as i32 {
        let dx = C - 44 + m * 20;
        let col = if m as usize == g.mode.idx() { WHITE } else { TEXT_DIM };
        s.rect(dx, 378, 8, 8, col);
    }
    centre(s, C, 398, "TAP OR CROWN", TEXT_SOFT);
}

/// Draw Game Over card text overlays.
fn draw_gameover_overlay(s: &mut impl Surface, g: &Game, accent: u16) {
    if g.new_best {
        centre(s, C, 110, "NEW BEST!", GOLD);
    } else {
        centre(s, C, 110, "GAME OVER", accent);
    }
    centre(s, C, 132, g.mode.name(), TEXT_SOFT);

    let mut tbuf = [0u8; 12];
    let tstr = fmt_time(g.time_cs(), &mut tbuf);
    let scale = if tstr.len() <= 5 { 4 } else { 3 };
    let ty = if scale == 4 { 158 } else { 164 };
    big(s, C - big_width(tstr, scale) / 2, ty, tstr, scale, WHITE);

    // Rank line: "RANK: TRIANGLE"
    let rname = g.rank.name().as_bytes();
    let mut rbuf = [0u8; 16];
    rbuf[0..6].copy_from_slice(b"RANK: ");
    let rlen = rname.len().min(10);
    rbuf[6..6 + rlen].copy_from_slice(&rname[..rlen]);
    if let Ok(lbl) = core::str::from_utf8(&rbuf[..6 + rlen]) {
        centre(s, C, 204, lbl, accent);
    }

    // Best line: "BEST: 12.48s"
    let mut bbuf = [0u8; 12];
    let bs = fmt_time(g.mode_best_cs(), &mut bbuf).as_bytes();
    let mut bline = [0u8; 18];
    bline[0..6].copy_from_slice(b"BEST: ");
    let blen = bs.len().min(10);
    bline[6..6 + blen].copy_from_slice(&bs[..blen]);
    bline[6 + blen] = b's';
    if let Ok(lbl) = core::str::from_utf8(&bline[..7 + blen]) {
        centre(s, C, 228, lbl, TEXT_SOFT);
    }

    // Action button labels
    centre(s, C, 281, "RETRY", BLACK);
    centre(s, C, 332, "MODES", TEXT_SOFT);
}

/// Render an entire frame in ONE single scanline-band pass with zero flicker.
pub fn render(s: &mut impl Surface, g: &Game, scratch: &mut RenderScratch) {
    let pal = resolve_palette(g);
    let (cx, cy) = prepare_geometry(g, scratch);
    let n = g.sides.clamp(4, 6);
    let nu = n as usize;
    let show_player = g.state == State::Playing || g.state == State::Dying;

    let mut spans = [Span::EMPTY; MAX_SPANS];
    let mut hud_drawn = false;

    for band in &BANDS {
        let y0 = band.y0 as i32;
        let h = band.h as i32;
        let y_mid = band.y_mid as i32;
        let x_min = band.x_min;
        let x_max = band.x_max;
        if x_min >= x_max {
            continue;
        }

        let mut len = 0usize;

        // 1. Exact O(1) rotating background sectors
        fill_bg_spans(
            &mut spans,
            &mut len,
            scratch,
            n,
            cx,
            cy,
            y_mid,
            x_min,
            x_max,
            &pal,
        );

        // 2. Active closing walls
        let mut wi = 0usize;
        while wi < scratch.wall_count {
            let wp = &scratch.walls[wi];
            if y_mid >= wp.y_min as i32 && y_mid <= wp.y_max as i32 {
                if let Some((lx, rx)) = poly_span(&wp.vx, &wp.vy, y_mid) {
                    paint_span(&mut spans, &mut len, x_min, x_max, lx, rx, pal.wall);
                }
            }
            wi += 1;
        }

        // 3. Pulsing center polygon (outer rim + dark core)
        if y_mid >= scratch.center_oy_min as i32 && y_mid <= scratch.center_oy_max as i32 {
            if let Some((lx, rx)) =
                poly_span(&scratch.center_ox[..nu], &scratch.center_oy[..nu], y_mid)
            {
                paint_span(&mut spans, &mut len, x_min, x_max, lx, rx, pal.rim);
            }
        }
        if y_mid >= scratch.center_iy_min as i32 && y_mid <= scratch.center_iy_max as i32 {
            if let Some((lx, rx)) =
                poly_span(&scratch.center_ix[..nu], &scratch.center_iy[..nu], y_mid)
            {
                paint_span(&mut spans, &mut len, x_min, x_max, lx, rx, pal.core);
            }
        }

        // 4. Player cursor triangle
        if show_player
            && y_mid >= scratch.player_y_min as i32
            && y_mid <= scratch.player_y_max as i32
        {
            if let Some((lx, rx)) = poly_span(&scratch.player_x, &scratch.player_y, y_mid) {
                paint_span(&mut spans, &mut len, x_min, x_max, lx, rx, pal.player);
            }
        }

        // 5. State-specific carved UI badges/cards (single-pass -> ZERO flicker!)
        match g.state {
            State::Playing | State::Dying => {
                // Sleek rounded-step top HUD badge inside y = 25..61
                if y0 >= 25 && y0 < 61 {
                    let hw = if y0 == 25 || y0 == 55 { 68i16 } else { 76i16 };
                    paint_span(
                        &mut spans,
                        &mut len,
                        x_min,
                        x_max,
                        C as i16 - hw,
                        C as i16 + hw,
                        HUD_BG,
                    );
                }
            }
            State::Title => {
                // Mode selector card: y = 173..289 (4px center bands -> exact 4px borders!)
                if y0 >= 173 && y0 < 289 {
                    let is_border = y0 == 173 || y0 == 285;
                    let col = if is_border { pal.wall } else { CARD_BG };
                    paint_span(&mut spans, &mut len, x_min, x_max, 68, 398, col);
                }
                // Bright PLAY button pill: y = 315..363
                if y0 >= 315 && y0 < 363 {
                    let pad = if y0 == 315 || y0 == 357 { 8i16 } else { 0i16 };
                    paint_span(
                        &mut spans,
                        &mut len,
                        x_min,
                        x_max,
                        158 + pad,
                        308 - pad,
                        pal.wall,
                    );
                }
            }
            State::Over => {
                // Game Over card: y = 97..369
                if y0 >= 97 && y0 < 369 {
                    let is_border = y0 == 97 || y0 == 363;
                    let col = if is_border { pal.wall } else { CARD_BG };
                    paint_span(&mut spans, &mut len, x_min, x_max, 74, 392, col);
                    // Bright RETRY button inside card: y = 265..309
                    if y0 >= 265 && y0 < 309 {
                        let pad = if y0 == 265 || y0 >= 303 { 6i16 } else { 0i16 };
                        paint_span(
                            &mut spans,
                            &mut len,
                            x_min,
                            x_max,
                            148 + pad,
                            318 - pad,
                            pal.wall,
                        );
                    }
                    // Secondary MODES button inside card: y = 321..357
                    if y0 >= 321 && y0 < 357 {
                        let pad = if y0 == 321 || y0 == 351 { 4i16 } else { 0i16 };
                        paint_span(
                            &mut spans,
                            &mut len,
                            x_min,
                            x_max,
                            166 + pad,
                            300 - pad,
                            BTN_SEC_BG,
                        );
                    }
                }
            }
        }

        // 6. Emit final merged horizontal rects for this band
        let mut si = 0usize;
        while si < len {
            let sp = spans[si];
            s.rect(sp.x0 as i32, y0, (sp.x1 - sp.x0) as i32, h, sp.col);
            si += 1;
        }

        // Draw HUD text right as the top badge finishes so text latency is microseconds
        if !hud_drawn && show_player && y0 + h >= 61 {
            draw_hud_text(s, g, pal.wall);
            hud_drawn = true;
        }
    }

    // Draw Title or Game Over text overlays once bands finish
    match g.state {
        State::Title => draw_title_overlay(s, g, pal.wall),
        State::Over => draw_gameover_overlay(s, g, pal.wall),
        State::Playing | State::Dying => {
            if !hud_drawn {
                draw_hud_text(s, g, pal.wall);
            }
        }
    }
}

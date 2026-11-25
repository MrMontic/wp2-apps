//! 16 distinct, custom-rendered unlockable knives for Knife Hit!
//!
//! Each knife has its own silhouette, palette, guard, and pommel rendered in pure fixed-point
//! integer math — both upright (ready, flying, big 2× Arsenal preview) and rotated at any `u16`
//! angle (stuck in target, bouncing off, or flying during shatter).

use cmf_wp2_ui::Surface;
use cmf_wp2_ui::color::rgb565;
use cmf_wp2_ui::motion::{Q14, cos, sin};

pub const NUM_KNIVES: usize = 16;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct KnifeSpec {
    pub name: &'static str,
    pub cost: u16,
    /// If non-zero, also automatically unlocks upon defeating this boss number (1 = Stage 5, 2 = Stage 10).
    pub boss_unlock: u8,
    pub blade: u16,
    pub blade_hi: u16,
    pub blade_dark: u16,
    pub guard: u16,
    pub handle: u16,
    pub accent: u16,
}

pub const KNIVES: [KnifeSpec; NUM_KNIVES] = [
    // 0: CLASSIC — Free starter
    KnifeSpec {
        name: "CLASSIC",
        cost: 0,
        boss_unlock: 0,
        blade: rgb565(200, 212, 226),
        blade_hi: rgb565(246, 250, 255),
        blade_dark: rgb565(118, 130, 146),
        guard: rgb565(92, 102, 118),
        handle: rgb565(56, 62, 74),
        accent: rgb565(172, 184, 198),
    },
    // 1: ORANGE — CMF tactical orange
    KnifeSpec {
        name: "ORANGE",
        cost: 5,
        boss_unlock: 0,
        blade: rgb565(48, 54, 64),
        blade_hi: rgb565(255, 104, 24),
        blade_dark: rgb565(28, 32, 40),
        guard: rgb565(255, 104, 24),
        handle: rgb565(36, 40, 48),
        accent: rgb565(255, 148, 64),
    },
    // 2: KUNAI — Shinobi ring handle + diamond blade
    KnifeSpec {
        name: "KUNAI",
        cost: 8,
        boss_unlock: 0,
        blade: rgb565(152, 166, 184),
        blade_hi: rgb565(226, 236, 248),
        blade_dark: rgb565(80, 92, 108),
        guard: rgb565(72, 80, 94),
        handle: rgb565(198, 42, 48),
        accent: rgb565(184, 196, 210),
    },
    // 3: DAGGER — Twin-edged ruby pommel
    KnifeSpec {
        name: "DAGGER",
        cost: 10,
        boss_unlock: 0,
        blade: rgb565(214, 224, 236),
        blade_hi: rgb565(255, 255, 255),
        blade_dark: rgb565(126, 138, 156),
        guard: rgb565(228, 182, 56),
        handle: rgb565(84, 48, 36),
        accent: rgb565(232, 34, 62),
    },
    // 4: CLEAVER — Wide butcher blade with cutout hole
    KnifeSpec {
        name: "CLEAVER",
        cost: 12,
        boss_unlock: 0,
        blade: rgb565(190, 202, 214),
        blade_hi: rgb565(242, 248, 255),
        blade_dark: rgb565(112, 124, 138),
        guard: rgb565(142, 152, 164),
        handle: rgb565(146, 86, 44),
        accent: rgb565(218, 186, 118),
    },
    // 5: KATANA — Long sleek blade + gold tsuba
    KnifeSpec {
        name: "KATANA",
        cost: 15,
        boss_unlock: 0,
        blade: rgb565(208, 220, 236),
        blade_hi: rgb565(252, 254, 255),
        blade_dark: rgb565(116, 132, 152),
        guard: rgb565(236, 190, 52),
        handle: rgb565(146, 28, 42),
        accent: rgb565(34, 34, 44),
    },
    // 6: NEON — Cyan glowing cyber blade
    KnifeSpec {
        name: "NEON",
        cost: 18,
        boss_unlock: 0,
        blade: rgb565(28, 228, 255),
        blade_hi: rgb565(216, 252, 255),
        blade_dark: rgb565(12, 124, 168),
        guard: rgb565(46, 56, 78),
        handle: rgb565(28, 34, 48),
        accent: rgb565(42, 244, 255),
    },
    // 7: GOLD — 24K gleaming gold ceremonial blade
    KnifeSpec {
        name: "GOLD",
        cost: 20,
        boss_unlock: 0,
        blade: rgb565(244, 196, 46),
        blade_hi: rgb565(255, 242, 156),
        blade_dark: rgb565(174, 122, 18),
        guard: rgb565(252, 212, 74),
        handle: rgb565(114, 36, 148),
        accent: rgb565(255, 232, 118),
    },
    // 8: FROST — Ice-blue crystal jagged blade
    KnifeSpec {
        name: "FROST",
        cost: 25,
        boss_unlock: 0,
        blade: rgb565(106, 204, 255),
        blade_hi: rgb565(234, 248, 255),
        blade_dark: rgb565(46, 120, 198),
        guard: rgb565(64, 156, 236),
        handle: rgb565(38, 76, 132),
        accent: rgb565(200, 242, 255),
    },
    // 9: FLAME — Fiery orange-red serrated blade
    KnifeSpec {
        name: "FLAME",
        cost: 30,
        boss_unlock: 0,
        blade: rgb565(255, 92, 24),
        blade_hi: rgb565(255, 228, 86),
        blade_dark: rgb565(184, 30, 18),
        guard: rgb565(64, 44, 42),
        handle: rgb565(46, 30, 32),
        accent: rgb565(255, 168, 42),
    },
    // 10: BONE — Ivory bone dagger
    KnifeSpec {
        name: "BONE",
        cost: 35,
        boss_unlock: 0,
        blade: rgb565(234, 224, 198),
        blade_hi: rgb565(254, 250, 238),
        blade_dark: rgb565(168, 154, 128),
        guard: rgb565(132, 82, 46),
        handle: rgb565(112, 68, 38),
        accent: rgb565(242, 234, 212),
    },
    // 11: SAI — 3-pronged martial arts trident
    KnifeSpec {
        name: "SAI",
        cost: 40,
        boss_unlock: 0,
        blade: rgb565(188, 200, 216),
        blade_hi: rgb565(242, 248, 255),
        blade_dark: rgb565(98, 110, 128),
        guard: rgb565(172, 186, 204),
        handle: rgb565(186, 32, 42),
        accent: rgb565(228, 186, 62),
    },
    // 12: PIXEL — Retro blocky emerald sword
    KnifeSpec {
        name: "PIXEL",
        cost: 45,
        boss_unlock: 0,
        blade: rgb565(48, 222, 112),
        blade_hi: rgb565(156, 255, 184),
        blade_dark: rgb565(20, 146, 66),
        guard: rgb565(32, 134, 72),
        handle: rgb565(138, 84, 42),
        accent: rgb565(92, 244, 148),
    },
    // 13: LASER — Plasma beam emitter + magenta core
    KnifeSpec {
        name: "LASER",
        cost: 50,
        boss_unlock: 0,
        blade: rgb565(255, 42, 188),
        blade_hi: rgb565(255, 236, 252),
        blade_dark: rgb565(168, 18, 122),
        guard: rgb565(146, 156, 172),
        handle: rgb565(74, 82, 96),
        accent: rgb565(255, 92, 212),
    },
    // 14: SCYTHE — Curved dark-crimson reaper hook (Boss #1 or 60 apples)
    KnifeSpec {
        name: "SCYTHE",
        cost: 60,
        boss_unlock: 1,
        blade: rgb565(198, 30, 52),
        blade_hi: rgb565(255, 108, 116),
        blade_dark: rgb565(64, 24, 34),
        guard: rgb565(72, 76, 88),
        handle: rgb565(42, 44, 54),
        accent: rgb565(238, 52, 72),
    },
    // 15: ROYAL — Diamond-studded crown blade (Boss #2 or 80 apples)
    KnifeSpec {
        name: "ROYAL",
        cost: 80,
        boss_unlock: 2,
        blade: rgb565(238, 244, 252),
        blade_hi: rgb565(112, 242, 255),
        blade_dark: rgb565(138, 154, 178),
        guard: rgb565(248, 198, 46),
        handle: rgb565(164, 24, 56),
        accent: rgb565(104, 240, 255),
    },
];

#[inline]
pub fn spec(id: u8) -> &'static KnifeSpec {
    &KNIVES[(id as usize).min(NUM_KNIVES - 1)]
}

#[inline]
fn clip_rect(s: &mut impl Surface, x: i32, y: i32, w: i32, h: i32, c: u16, min_y: i32) {
    let y0 = y.max(min_y);
    let y1 = (y + h).min(cmf_wp2_ui::SIZE);
    if w > 0 && y0 < y1 {
        s.rect(x, y0, w, y1 - y0, c);
    }
}

/// Draws a full-detail upright knife pointing straight UP (`tip_y` = topmost tip pixel).
/// Total height is `76 * scale` px. Used for the ready/flying knife (`scale = 1`) and the
/// Arsenal showcase (`scale = 2`).
pub fn draw_upright(s: &mut impl Surface, cx: i32, tip_y: i32, id: u8, scale: i32, min_y: i32) {
    let k = spec(id);
    let sc = scale.max(1);
    let r = |s: &mut dyn Surface, dx: i32, dy: i32, w: i32, h: i32, c: u16| {
        let rx = cx + dx * sc;
        let ry = tip_y + dy * sc;
        let rw = w * sc;
        let rh = h * sc;
        let y0 = ry.max(min_y);
        let y1 = (ry + rh).min(cmf_wp2_ui::SIZE);
        if rw > 0 && y0 < y1 {
            s.rect(rx, y0, rw, y1 - y0, c);
        }
    };

    match id {
        // 2: KUNAI — diamond blade + hollow ring pommel
        2 => {
            r(s, -1, 0, 2, 6, k.blade_hi);
            r(s, -4, 6, 4, 8, k.blade_hi);
            r(s, 0, 6, 4, 8, k.blade_dark);
            r(s, -7, 14, 7, 14, k.blade);
            r(s, 0, 14, 7, 14, k.blade_dark);
            r(s, -5, 28, 10, 8, k.blade);
            r(s, -2, 36, 4, 6, k.guard);
            // Wrapped shaft
            r(s, -3, 42, 6, 20, k.handle);
            r(s, -3, 46, 6, 2, k.guard);
            r(s, -3, 52, 6, 2, k.guard);
            r(s, -3, 58, 6, 2, k.guard);
            // Iconic hollow metal ring pommel
            r(s, -6, 62, 12, 3, k.accent);
            r(s, -6, 71, 12, 3, k.accent);
            r(s, -6, 65, 3, 6, k.accent);
            r(s, 3, 65, 3, 6, k.accent);
        }
        // 4: CLEAVER — wide butcher blade with hanging hole
        4 => {
            r(s, -9, 2, 18, 42, k.blade);
            r(s, -10, 4, 3, 38, k.blade_hi);
            r(s, 7, 2, 3, 42, k.blade_dark);
            // Cutout hole near top-right corner
            r(s, 2, 7, 4, 4, rgb565(14, 20, 30));
            // Bolster & wooden riveted handle
            r(s, -6, 44, 12, 4, k.guard);
            r(s, -4, 48, 9, 26, k.handle);
            r(s, -1, 53, 3, 3, k.accent);
            r(s, -1, 61, 3, 3, k.accent);
            r(s, -1, 69, 3, 3, k.accent);
        }
        // 5: KATANA — slim long blade + circular gold tsuba + diamond wrap
        5 => {
            r(s, -2, 0, 3, 6, k.blade_hi);
            r(s, -3, 6, 3, 40, k.blade_hi);
            r(s, 0, 6, 3, 40, k.blade_dark);
            // Round gold tsuba guard
            r(s, -9, 46, 18, 4, k.guard);
            // Wrapped tsuka hilt
            r(s, -3, 50, 6, 22, k.handle);
            r(s, -3, 53, 6, 2, k.accent);
            r(s, -3, 58, 6, 2, k.accent);
            r(s, -3, 63, 6, 2, k.accent);
            r(s, -3, 68, 6, 2, k.accent);
            r(s, -4, 72, 8, 4, k.guard);
        }
        // 8: FROST — jagged crystalline ice blade
        8 => {
            r(s, -1, 0, 2, 6, k.blade_hi);
            r(s, -4, 6, 8, 8, k.blade);
            r(s, -7, 14, 12, 8, k.blade_hi);
            r(s, -4, 22, 11, 8, k.blade);
            r(s, -7, 30, 12, 8, k.blade_dark);
            r(s, -5, 38, 10, 6, k.blade);
            r(s, -1, 6, 2, 34, k.blade_hi);
            // Frozen crossguard & hilt
            r(s, -10, 44, 20, 5, k.guard);
            r(s, -4, 49, 8, 21, k.handle);
            r(s, -5, 70, 10, 6, k.accent);
        }
        // 9: FLAME — wavy serrated fiery blade
        9 => {
            r(s, -1, 0, 2, 6, k.blade_hi);
            r(s, -4, 6, 8, 8, k.blade);
            r(s, -6, 14, 10, 8, k.blade_dark);
            r(s, -4, 22, 10, 8, k.blade);
            r(s, -6, 30, 10, 8, k.blade_dark);
            r(s, -5, 38, 10, 6, k.blade);
            r(s, -2, 8, 4, 34, k.blade_hi);
            // Obsidian guard & ember hilt
            r(s, -10, 44, 20, 5, k.guard);
            r(s, -4, 49, 8, 21, k.handle);
            r(s, -2, 53, 4, 12, k.accent);
            r(s, -5, 70, 10, 6, k.guard);
        }
        // 11: SAI — iconic 3-pronged martial arts trident
        11 => {
            // Slender center tine
            r(s, -1, 0, 2, 8, k.blade_hi);
            r(s, -2, 8, 4, 38, k.blade);
            r(s, -2, 8, 2, 36, k.blade_hi);
            // Twin curved outer prongs
            r(s, -11, 24, 3, 18, k.guard);
            r(s, 8, 24, 3, 18, k.guard);
            r(s, -11, 42, 22, 5, k.guard);
            // Crimson grip + gold pommel
            r(s, -3, 47, 6, 22, k.handle);
            r(s, -5, 69, 10, 6, k.accent);
        }
        // 12: PIXEL — retro blocky emerald sword
        12 => {
            r(s, -2, 0, 4, 4, k.blade_hi);
            r(s, -4, 4, 8, 4, k.blade);
            r(s, -6, 8, 12, 34, k.blade);
            r(s, -6, 8, 4, 34, k.blade_hi);
            r(s, 2, 8, 4, 34, k.blade_dark);
            // Stepped pixel crossguard
            r(s, -12, 42, 24, 6, k.guard);
            r(s, -8, 48, 16, 4, k.blade_dark);
            // Pixel brown handle + emerald pommel
            r(s, -4, 52, 8, 16, k.handle);
            r(s, -6, 68, 12, 6, k.accent);
        }
        // 13: LASER — plasma emitter + glowing beam
        13 => {
            // Outer magenta glow
            r(s, -5, 2, 10, 44, k.blade_dark);
            r(s, -4, 0, 8, 46, k.blade);
            // White-hot plasma core
            r(s, -2, 2, 4, 44, k.blade_hi);
            // Sci-fi chrome emitter & ribbed hilt
            r(s, -8, 46, 16, 5, k.guard);
            r(s, -5, 51, 10, 22, k.handle);
            r(s, -5, 54, 10, 2, k.guard);
            r(s, -5, 60, 10, 2, k.guard);
            r(s, -5, 66, 10, 2, k.guard);
            r(s, -2, 56, 4, 4, k.accent);
            r(s, -6, 73, 12, 3, k.guard);
        }
        // 14: SCYTHE — curved dark-crimson reaper hook
        14 => {
            // Curved upper reaper hook jutting right
            r(s, -2, 0, 8, 5, k.blade_hi);
            r(s, 4, 2, 9, 6, k.blade);
            r(s, 8, 8, 6, 8, k.blade_hi);
            r(s, -4, 5, 8, 39, k.blade_dark);
            r(s, -4, 5, 3, 39, k.blade);
            // Dark reaper collar & long shaft
            r(s, -7, 44, 14, 5, k.guard);
            r(s, -3, 49, 6, 22, k.handle);
            r(s, -5, 71, 10, 5, k.accent);
        }
        // 15: ROYAL — diamond-studded crown blade
        15 => {
            r(s, -1, 0, 2, 6, k.blade);
            r(s, -4, 6, 8, 8, k.blade);
            r(s, -6, 14, 6, 30, k.blade);
            r(s, 0, 14, 6, 30, k.blade_dark);
            // Gleaming diamond studs down the center
            r(s, -2, 16, 4, 4, k.blade_hi);
            r(s, -2, 26, 4, 4, k.blade_hi);
            r(s, -2, 36, 4, 4, k.blade_hi);
            // 3-point gold crown crossguard
            r(s, -11, 44, 22, 5, k.guard);
            r(s, -11, 40, 4, 4, k.guard);
            r(s, 7, 40, 4, 4, k.guard);
            // Velvet crimson hilt + diamond pommel
            r(s, -4, 49, 8, 20, k.handle);
            r(s, -5, 69, 10, 6, k.guard);
            r(s, -2, 70, 4, 4, k.accent);
        }
        // 0: CLASSIC, 1: ORANGE, 3: DAGGER, 6: NEON, 7: GOLD, 10: BONE
        _ => {
            let wide = if id == 7 || id == 10 { 6 } else { 5 };
            r(s, -1, 0, 2, 6, k.blade_hi);
            r(s, -3, 6, 6, 8, k.blade);
            r(s, -wide, 14, wide, 30, k.blade);
            r(s, 0, 14, wide, 30, k.blade_dark);
            // Edge highlight
            r(s, -wide, 14, 2, 28, k.blade_hi);
            // Bone notch detail
            if id == 10 {
                r(s, 2, 20, 4, 3, rgb565(14, 20, 30));
                r(s, 2, 30, 4, 3, rgb565(14, 20, 30));
            }
            // Crossguard
            let gw = if id == 3 || id == 7 { 10 } else { 8 };
            r(s, -gw, 44, gw * 2, 5, k.guard);
            // Handle
            r(s, -4, 49, 8, 21, k.handle);
            if id == 1 || id == 6 {
                // Tactical / cyber grip stripes
                r(s, -4, 53, 8, 2, k.accent);
                r(s, -4, 60, 8, 2, k.accent);
            }
            // Pommel
            r(s, -5, 70, 10, 6, k.accent);
            if id == 3 {
                // Ruby center gem
                r(s, -2, 71, 4, 4, rgb565(255, 118, 138));
            }
        }
    }
}

/// Draws a rotated knife protruding radially outward from `(cx, cy)` at angle `ang` (`u16` turns).
/// `r_base` is where the blade exits the target rim (typically `TARGET_R - 4 = 66`);
/// the handle extends outward to `r_base + 56` (`~122`).
/// Uses only ~14–18 rects per knife so even 14 stuck knives stay super fast!
pub fn draw_rotated(
    s: &mut impl Surface,
    cx: i32,
    cy: i32,
    r_base: i32,
    ang: u16,
    id: u8,
    min_y: i32,
) {
    let k = spec(id);
    let ca = cos(ang);
    let sa = sin(ang);
    // Perpendicular unit vector (-sa, ca)
    let px = -sa;
    let py = ca;

    let stamp = |s: &mut dyn Surface, along: i32, side: i32, rad: i32, col: u16| {
        let x = cx + ((along * ca + side * px) >> 14) - rad;
        let y = cy + ((along * sa + side * py) >> 14) - rad;
        let sz = rad * 2 + 1;
        let y0 = y.max(min_y);
        let y1 = (y + sz).min(cmf_wp2_ui::SIZE);
        if sz > 0 && y0 < y1 {
            s.rect(x, y0, sz, y1 - y0, col);
        }
    };

    // Blade protruding from r_base .. r_base + 24
    match id {
        // CLEAVER: wide rectangular offset blade with hole
        4 => {
            for step in 0..4 {
                let al = r_base + 3 + step * 6;
                stamp(s, al, -3, 4, k.blade_hi);
                stamp(s, al, 3, 4, k.blade);
            }
            stamp(s, r_base + 8, 3, 1, rgb565(14, 20, 30));
        }
        // KUNAI: wide diamond mid-blade
        2 => {
            stamp(s, r_base + 4, 0, 5, k.blade);
            stamp(s, r_base + 11, 0, 6, k.blade_hi);
            stamp(s, r_base + 18, 0, 4, k.blade_dark);
            stamp(s, r_base + 24, 0, 2, k.guard);
        }
        // SAI: slender central tine + two side prongs
        11 => {
            for step in 0..4 {
                stamp(s, r_base + 3 + step * 6, 0, 2, k.blade_hi);
            }
            stamp(s, r_base + 14, -8, 2, k.guard);
            stamp(s, r_base + 21, -8, 2, k.guard);
            stamp(s, r_base + 14, 8, 2, k.guard);
            stamp(s, r_base + 21, 8, 2, k.guard);
        }
        // SCYTHE: hooked reaper blade jutting sideways near the target rim
        14 => {
            stamp(s, r_base + 6, 7, 4, k.blade_hi);
            stamp(s, r_base + 10, 11, 3, k.blade);
            for step in 0..4 {
                stamp(s, r_base + 3 + step * 6, 0, 3, k.blade_dark);
            }
            stamp(s, r_base + 12, -2, 2, k.blade);
        }
        // LASER: glowing magenta outer beam + bright white core
        13 => {
            for step in 0..4 {
                let al = r_base + 3 + step * 6;
                stamp(s, al, 0, 4, k.blade);
                stamp(s, al, 0, 2, k.blade_hi);
            }
        }
        // FROST / FLAME: jagged alternating blade
        8 | 9 => {
            for step in 0..4 {
                let al = r_base + 3 + step * 6;
                let off = if step & 1 == 0 { -2 } else { 2 };
                stamp(s, al, off, 4, k.blade);
                stamp(s, al, 0, 2, k.blade_hi);
            }
        }
        // ROYAL: blade with gleaming cyan diamond studs
        15 => {
            for step in 0..4 {
                let al = r_base + 3 + step * 6;
                stamp(s, al, 0, 4, k.blade);
            }
            stamp(s, r_base + 8, 0, 2, k.blade_hi);
            stamp(s, r_base + 18, 0, 2, k.blade_hi);
        }
        // Standard sleek blades (Classic, Orange, Dagger, Katana, Neon, Gold, Bone, Pixel)
        _ => {
            let rad = if id == 5 { 2 } else if id == 12 { 4 } else { 3 };
            for step in 0..4 {
                let al = r_base + 3 + step * 6;
                stamp(s, al, -1, rad, k.blade);
                if step == 1 || step == 2 {
                    stamp(s, al, -rad + 1, 1, k.blade_hi);
                }
            }
        }
    }

    // Crossguard at r_base + 27
    let g_span = match id {
        11 | 12 | 15 => 8,
        3 | 5 | 7 => 7,
        2 => 4,
        _ => 6,
    };
    stamp(s, r_base + 27, -g_span, 2, k.guard);
    stamp(s, r_base + 27, 0, 3, k.guard);
    stamp(s, r_base + 27, g_span, 2, k.guard);

    // Handle from r_base + 33 .. r_base + 48
    stamp(s, r_base + 34, 0, 3, k.handle);
    stamp(s, r_base + 40, 0, 3, if id == 1 || id == 6 { k.accent } else { k.handle });
    stamp(s, r_base + 46, 0, 3, k.handle);

    // Pommel at r_base + 52
    if id == 2 {
        // Kunai hollow ring
        stamp(s, r_base + 53, 0, 5, k.accent);
        stamp(s, r_base + 53, 0, 2, rgb565(14, 20, 30));
    } else {
        stamp(s, r_base + 52, 0, 4, k.accent);
    }
}

/// Returns two tight axis-aligned bounding boxes covering a rotated knife at `(cx, cy, r_base, ang)`
/// for ultra-fast differential erasure without overlapping distant HUD elements.
pub fn rotated_bboxes(cx: i32, cy: i32, r_base: i32, ang: u16) -> [(i32, i32, i32, i32); 2] {
    let ca = cos(ang);
    let sa = sin(ang);
    let p0x = cx + ((r_base * ca) >> 14);
    let p0y = cy + ((r_base * sa) >> 14);
    let p1x = cx + (((r_base + 28) * ca) >> 14);
    let p1y = cy + (((r_base + 28) * sa) >> 14);
    let p2x = cx + (((r_base + 55) * ca) >> 14);
    let p2y = cy + (((r_base + 55) * sa) >> 14);

    let pad1 = 15; // accommodates cleaver, sai prongs, scythe hook, wide guards
    let b1 = (
        p0x.min(p1x) - pad1,
        p0y.min(p1y) - pad1,
        (p0x - p1x).abs() + pad1 * 2 + 1,
        (p0y - p1y).abs() + pad1 * 2 + 1,
    );
    let pad2 = 12; // handle + pommel/ring
    let b2 = (
        p1x.min(p2x) - pad2,
        p1y.min(p2y) - pad2,
        (p1x - p2x).abs() + pad2 * 2 + 1,
        (p1y - p2y).abs() + pad2 * 2 + 1,
    );
    [b1, b2]
}

/// Draws a sleek mini knife icon in the left-side remaining knives queue!
pub fn draw_mini_icon(s: &mut impl Surface, x: i32, y: i32, active: bool, id: u8) {
    let k = spec(id);
    if active {
        // Horizontal little knife pointing right
        clip_rect(s, x, y + 2, 7, 4, k.handle, 0);
        clip_rect(s, x + 7, y, 2, 8, k.guard, 0);
        clip_rect(s, x + 9, y + 1, 13, 6, k.blade, 0);
        clip_rect(s, x + 9, y + 1, 11, 2, k.blade_hi, 0);
        clip_rect(s, x + 22, y + 2, 3, 4, k.blade_hi, 0);
    } else {
        let dim = rgb565(34, 44, 60);
        clip_rect(s, x, y + 2, 7, 4, dim, 0);
        clip_rect(s, x + 7, y + 1, 16, 6, dim, 0);
    }
}

/// Fixed-point angle-to-offset helper (`(r * cos(a) / Q14, r * sin(a) / Q14)`).
#[inline]
pub fn polar(cx: i32, cy: i32, r: i32, ang: u16) -> (i32, i32) {
    (cx + (r * cos(ang)) / Q14, cy + (r * sin(ang)) / Q14)
}

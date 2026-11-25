//! Color palette and 8 vibrant Slither.io snake skins (100% const in flash, zero RAM).

use cmf_wp2_ui::color::rgb565;

pub const SKIN_COUNT: usize = 8;

/// Palette indices into [`PALETTE`].
pub const PAL_BG: u8 = 0;
pub const PAL_HEX: u8 = 1;
pub const PAL_HEX_HI: u8 = 2;
pub const PAL_OUTSIDE: u8 = 3;
pub const PAL_BORDER: u8 = 4;
pub const PAL_BORDER_GLOW: u8 = 5;
pub const PAL_EYE_WHITE: u8 = 6;
pub const PAL_EYE_PUPIL: u8 = 7;
pub const PAL_BOOST_AURA: u8 = 8;
pub const PAL_MAP_BG: u8 = 9;
pub const PAL_MAP_RING: u8 = 10;
pub const PAL_MAP_YOU: u8 = 11;
pub const PAL_MAP_BOSS: u8 = 12;
pub const PAL_MAP_BOT: u8 = 13;
pub const PAL_MAP_LOOT: u8 = 14;

/// Base index for skin shades: each of the 8 skins uses 4 consecutive entries:
/// `[dark_band, mid_body, bright_stripe, boost_highlight]`.
pub const PAL_SKIN_BASE: u8 = 16; // 16..48

/// 8 rainbow segment colors for skin #6 (`RAINBOW`).
pub const PAL_RAINBOW_BASE: u8 = 48; // 48..56

/// 8 glowing orb colors: each uses 2 entries `[halo, core]`.
pub const PAL_ORB_BASE: u8 = 56; // 56..72

pub const PALETTE_LEN: usize = 72;

pub const SKIN_NAMES: [&str; SKIN_COUNT] = [
    "NEON",
    "LIME",
    "CMF",
    "CRIMSON",
    "GOLD",
    "VIOLET",
    "RAINBOW",
    "GHOST",
];

// Native RGB565 colors for HUD and menus
pub const UI_BG: u16 = rgb565(10, 14, 22);
pub const UI_PANEL: u16 = rgb565(18, 24, 36);
pub const UI_BORDER: u16 = rgb565(56, 72, 96);
pub const UI_WHITE: u16 = rgb565(244, 248, 255);
pub const UI_SILVER: u16 = rgb565(156, 170, 192);
pub const UI_GOLD: u16 = rgb565(255, 214, 56);
pub const UI_RED: u16 = rgb565(255, 68, 72);
pub const UI_LIME: u16 = rgb565(88, 242, 118);
pub const UI_CYAN: u16 = rgb565(48, 228, 255);
pub const UI_ORANGE: u16 = rgb565(255, 102, 24);

/// Master RGB565 palette LUT stored in flash (144 bytes).
pub const PALETTE: [u16; PALETTE_LEN] = [
    // 0..16: Arena, eyes, minimap, system
    rgb565(11, 15, 23),   // 0: PAL_BG (classic Slither deep navy-charcoal)
    rgb565(19, 26, 39),   // 1: PAL_HEX (hex floor tile fill)
    rgb565(27, 37, 54),   // 2: PAL_HEX_HI (hex floor highlight)
    rgb565(26, 8, 12),    // 3: PAL_OUTSIDE (void outside red border)
    rgb565(255, 42, 52),  // 4: PAL_BORDER (glowing crimson arena wall)
    rgb565(128, 18, 28),  // 5: PAL_BORDER_GLOW (soft red wall glow)
    rgb565(250, 252, 255),// 6: PAL_EYE_WHITE
    rgb565(10, 12, 20),   // 7: PAL_EYE_PUPIL
    rgb565(255, 242, 168),// 8: PAL_BOOST_AURA
    rgb565(8, 12, 18),    // 9: PAL_MAP_BG
    rgb565(64, 82, 108),  // 10: PAL_MAP_RING
    rgb565(255, 255, 255),// 11: PAL_MAP_YOU
    rgb565(255, 134, 34), // 12: PAL_MAP_BOSS
    rgb565(108, 134, 164),// 13: PAL_MAP_BOT
    rgb565(255, 232, 74), // 14: PAL_MAP_LOOT
    rgb565(0, 0, 0),      // 15: PAL_BLACK

    // 16..20: Skin 0 - NEON (Cyan glow)
    rgb565(14, 122, 168),
    rgb565(26, 202, 244),
    rgb565(108, 242, 255),
    rgb565(216, 254, 255),

    // 20..24: Skin 1 - LIME (Classic Slither green)
    rgb565(28, 136, 46),
    rgb565(64, 214, 78),
    rgb565(136, 250, 114),
    rgb565(224, 255, 204),

    // 24..28: Skin 2 - CMF (Orange + matte white stripes!)
    rgb565(184, 56, 10),
    rgb565(255, 96, 18),
    rgb565(236, 240, 248),
    rgb565(255, 198, 96),

    // 28..32: Skin 3 - CRIMSON (Ruby red)
    rgb565(152, 18, 34),
    rgb565(234, 42, 58),
    rgb565(255, 114, 122),
    rgb565(255, 212, 210),

    // 32..36: Skin 4 - GOLD (Solar gold + dark bronze rings)
    rgb565(112, 74, 14),
    rgb565(242, 182, 24),
    rgb565(255, 234, 96),
    rgb565(255, 252, 204),

    // 36..40: Skin 5 - VIOLET (Electric purple)
    rgb565(96, 32, 174),
    rgb565(164, 68, 248),
    rgb565(222, 134, 255),
    rgb565(246, 216, 255),

    // 40..44: Skin 6 - RAINBOW fallback accent shades
    rgb565(255, 64, 88),
    rgb565(255, 196, 40),
    rgb565(56, 234, 144),
    rgb565(255, 255, 255),

    // 44..48: Skin 7 - GHOST (Ice white + cyan)
    rgb565(82, 128, 164),
    rgb565(172, 212, 236),
    rgb565(238, 248, 255),
    rgb565(146, 246, 255),

    // 48..56: 8 Rainbow cycle colors
    rgb565(255, 58, 76),   // 48: Red
    rgb565(255, 136, 30),  // 49: Orange
    rgb565(255, 224, 48),  // 50: Yellow
    rgb565(72, 236, 92),   // 51: Green
    rgb565(40, 224, 244),  // 52: Cyan
    rgb565(74, 136, 255),  // 53: Blue
    rgb565(178, 76, 255),  // 54: Violet
    rgb565(255, 86, 198),  // 55: Magenta

    // 56..72: 8 Glowing orb pairs [halo, core]
    rgb565(18, 126, 166), rgb565(116, 246, 255), // 0: Cyan
    rgb565(28, 146, 52),  rgb565(138, 255, 132), // 1: Lime
    rgb565(176, 74, 14),  rgb565(255, 178, 78),  // 2: Orange
    rgb565(162, 34, 128), rgb565(255, 132, 234), // 3: Magenta
    rgb565(168, 132, 16), rgb565(255, 242, 122), // 4: Gold
    rgb565(164, 28, 38),  rgb565(255, 124, 132), // 5: Coral Red
    rgb565(32, 84, 186),  rgb565(132, 196, 255), // 6: Electric Blue
    rgb565(72, 152, 142), rgb565(224, 255, 248), // 7: Mint White
];

/// Returns the palette index for a given snake skin, segment index, and boost state.
#[inline]
pub fn skin_segment_color(skin: u8, seg_idx: usize, tick: u32, boosting: bool) -> u8 {
    let s = (skin as usize) % SKIN_COUNT;
    if s == 6 {
        // RAINBOW: smooth cycling wave along the snake body
        if boosting && ((seg_idx + (tick as usize >> 1)) & 3) == 0 {
            return PAL_BOOST_AURA;
        }
        let phase = (seg_idx / 2 + (tick as usize >> 3)) & 7;
        return PAL_RAINBOW_BASE + phase as u8;
    }

    let base = PAL_SKIN_BASE + (s as u8) * 4;
    if boosting {
        // Bright pulsing energy wave traveling down the snake when boosting!
        let wave = ((seg_idx >> 1).wrapping_sub(tick as usize >> 2)) & 3;
        return match wave {
            0 => base + 3,
            1 | 2 => base + 2,
            _ => base + 1,
        };
    }

    // Classic Slither.io smooth glossy alternating bands (2-segment wide bands = cleaner look + fewer RLE runs!)
    let band = (seg_idx >> 1) & 3;
    if s == 2 {
        // CMF signature: crisp orange with matte-white stripes!
        match band {
            0 => base + 2, // matte white stripe
            3 => base,     // burnt orange shade
            _ => base + 1, // CMF orange
        }
    } else {
        match band {
            0 => base + 2, // bright highlight band
            2 => base,     // darker ring
            _ => base + 1, // primary body tone
        }
    }
}

/// Primary accent RGB565 color for UI labels of the given skin.
#[inline]
pub fn skin_ui_color(skin: u8) -> u16 {
    let s = (skin as usize) % SKIN_COUNT;
    if s == 6 {
        return UI_GOLD;
    }
    PALETTE[(PAL_SKIN_BASE as usize) + s * 4 + 1]
}

/// Orb palette index (`halo` or `core`) for orb color group `0..8`.
#[inline]
pub fn orb_palette(color: u8, core: bool) -> u8 {
    PAL_ORB_BASE + ((color & 7) * 2) + if core { 1 } else { 0 }
}

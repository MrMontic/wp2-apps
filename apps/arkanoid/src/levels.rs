//! 12 handcrafted iconic Arkanoid levels (10 cols × 8 rows) + endless Wave 2+ escalation.

pub const COLS: usize = 10;
pub const ROWS: usize = 8;
pub const GRID_LEN: usize = COLS * ROWS;
pub const NUM_LEVELS: usize = 12;

/// Visual & behavioral kind of brick cell.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum BrickKind {
    Empty = 0,
    Crimson = 1,
    Orange = 2,
    Gold = 3,
    Emerald = 4,
    Cyan = 5,
    Violet = 6,
    Silver = 7,   // 2 HP armored
    Titanium = 8, // 3 HP heavy steel
    Tnt = 9,      // 1 HP explosive chain-reaction
    Bumper = 10,  // Indestructible gold bumper
}

impl BrickKind {
    pub const fn from_code(b: u8) -> Self {
        match b {
            b'R' => BrickKind::Crimson,
            b'O' => BrickKind::Orange,
            b'Y' => BrickKind::Gold,
            b'G' => BrickKind::Emerald,
            b'C' => BrickKind::Cyan,
            b'V' => BrickKind::Violet,
            b'S' => BrickKind::Silver,
            b'T' => BrickKind::Titanium,
            b'X' => BrickKind::Tnt,
            b'B' => BrickKind::Bumper,
            _ => BrickKind::Empty,
        }
    }

    pub const fn max_hp(self) -> u8 {
        match self {
            BrickKind::Empty => 0,
            BrickKind::Silver => 2,
            BrickKind::Titanium => 3,
            BrickKind::Bumper => 255,
            _ => 1,
        }
    }

    pub const fn is_breakable(self) -> bool {
        !matches!(self, BrickKind::Empty | BrickKind::Bumper)
    }

    pub const fn base_points(self) -> u32 {
        match self {
            BrickKind::Empty | BrickKind::Bumper => 0,
            BrickKind::Crimson | BrickKind::Orange | BrickKind::Gold => 60,
            BrickKind::Emerald | BrickKind::Cyan | BrickKind::Violet => 50,
            BrickKind::Silver => 120,
            BrickKind::Titanium => 200,
            BrickKind::Tnt => 150,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Brick {
    pub kind: BrickKind,
    pub hp: u8,
    pub flash: u8, // brief highlight ticks on armored/bumper hit
}

impl Brick {
    pub const EMPTY: Self = Self {
        kind: BrickKind::Empty,
        hp: 0,
        flash: 0,
    };

    pub const fn new(kind: BrickKind) -> Self {
        Self {
            kind,
            hp: kind.max_hp(),
            flash: 0,
        }
    }

    #[inline]
    pub const fn alive(self) -> bool {
        self.hp > 0 && !matches!(self.kind, BrickKind::Empty)
    }
}

pub struct LevelDef {
    pub name: &'static str,
    pub rows: [&'static [u8; COLS]; ROWS],
}

/// All 12 handcrafted arcade levels.
/// Legend:
///   '.' = Empty
///   'R' = Crimson, 'O' = CMF Orange, 'Y' = Gold, 'G' = Emerald, 'C' = Cyan, 'V' = Violet
///   'S' = Silver Armored (2 HP), 'T' = Titanium Steel (3 HP)
///   'X' = TNT Explosive, 'B' = Indestructible Gold Bumper
pub const LEVELS: [LevelDef; NUM_LEVELS] = [
    // 1. CLASSIC RAINBOW — Warm-up arcade rows with two juicy TNT triggers
    LevelDef {
        name: "RAINBOW",
        rows: [
            b"RRRRRRRRRR",
            b"OOOOOOOOOO",
            b"YYXYYYYXYY",
            b"GGGGGGGGGG",
            b"CCCCCCCCCC",
            b"VVVVVVVVVV",
            b"..........",
            b"..........",
        ],
    },
    // 2. PYRAMID — Stepped triangle with explosive TNT core & silver base corners
    LevelDef {
        name: "PYRAMID",
        rows: [
            b"....OO....",
            b"...YYYY...",
            b"..GGXXGG..",
            b".CCCSSCCC.",
            b"VVVVXXVVVV",
            b"S.RRRRRR.S",
            b"..........",
            b"..........",
        ],
    },
    // 3. SPACE INVADER — Classic pixel-art alien!
    LevelDef {
        name: "INVADER",
        rows: [
            b"..G....G..",
            b"...G..G...",
            b"..GGGGGG..",
            b".GGOCCOGG.",
            b"GGGGXXGGGG",
            b"G.GGGGGG.G",
            b"G.G....G.G",
            b"...CC..CC.",
        ],
    },
    // 4. DIAMOND VAULT — Silver armored shell guarding explosive TNT treasury
    LevelDef {
        name: "VAULT",
        rows: [
            b"....SS....",
            b"...SYYYS..",
            b"..SYXXYS..",
            b".SYXOOXYS.",
            b"..SYXXYS..",
            b"...SYYYS..",
            b"....SS....",
            b"..CC..CC..",
        ],
    },
    // 5. TWIN TOWERS — Two corridors with gold bumper caps for pinball multi-hits
    LevelDef {
        name: "TOWERS",
        rows: [
            b".RRR..CCC.",
            b".OOO..VVV.",
            b".YXS..SXC.",
            b".GGG..GGG.",
            b".CCC..OOO.",
            b".VVV..RRR.",
            b".BSB..BSB.",
            b"..........",
        ],
    },
    // 6. HEART — Vibrant crimson & CMF-orange pixel heart with silver center
    LevelDef {
        name: "HEART",
        rows: [
            b".RRR..RRR.",
            b"ROOORROOOR",
            b"ROYYSSYYOR",
            b"ROYYXXYYOR",
            b".ROOOOOOR.",
            b"..ROOOOR..",
            b"...ROOR...",
            b"....RR....",
        ],
    },
    // 7. FORTRESS — Gold-capped walls + sneaky side alleys + TNT barrels
    LevelDef {
        name: "FORTRESS",
        rows: [
            b".TTTTTTTT.",
            b".RXOOOOXR.",
            b".RYYYYYYR.",
            b".BSGGGGSB.",
            b".B.CCCC.B.",
            b".B.VXXV.B.",
            b"....SS....",
            b"..........",
        ],
    },
    // 8. CHECKERBOARD — Alternating armored + explosive checkerboard
    LevelDef {
        name: "CHECKERS",
        rows: [
            b"S.R.S.R.S.",
            b".O.X.O.X.O",
            b"Y.S.Y.S.Y.",
            b".G.C.X.G.C",
            b"S.V.S.V.S.",
            b".R.O.R.O.X",
            b"C.S.C.S.C.",
            b"..........",
        ],
    },
    // 9. SMILEY — Cheerful yellow arcade face with cyan eyes & orange grin
    LevelDef {
        name: "SMILEY",
        rows: [
            b"..YYYYYY..",
            b".YYYYYYYY.",
            b"YYCCYYCCYY",
            b"YYCCYYCCYY",
            b"YYYYXXYYYY",
            b"YROYYYYORY",
            b".YROOOORY.",
            b"..YYYYYY..",
        ],
    },
    // 10. HOURGLASS — Pinched waist with TNT mines!
    LevelDef {
        name: "HOURGLASS",
        rows: [
            b"SSSSSSSSSS",
            b".CCCCCCCC.",
            b"..GGGGGG..",
            b"...YXXY...",
            b"...OXXO...",
            b"..RRRRRR..",
            b".VVVVVVVV.",
            b"SS......SS",
        ],
    },
    // 11. SPIRAL MAZE — Winding armored corridor with TNT shortcuts
    LevelDef {
        name: "SPIRAL",
        rows: [
            b"TTTTTTTTTT",
            b"C........T",
            b"C.SSSSSS.T",
            b"C.S.XX.S.T",
            b"C.S.OO.S.T",
            b"C.S..Y.S.T",
            b"C.GGGGGG.T",
            b"VVVVVVVV.B",
        ],
    },
    // 12. BOSS CORE — 3-HP titanium fortress guarding CMF-orange TNT reactor
    LevelDef {
        name: "BOSS CORE",
        rows: [
            b"B.TTTTTT.B",
            b".TSSSSSST.",
            b"TSOOXXOOST",
            b"TSOXXXXOST",
            b"TSOOXXOOST",
            b".TSSSSSST.",
            b"B.T.YY.T.B",
            b"..CC..CC..",
        ],
    },
];

/// Populates `out` with the layout for `level_idx` (0..12) and `wave` (0 = Wave 1, 1 = Wave 2, ...).
/// Returns the number of breakable bricks placed.
pub fn load_level(level_idx: usize, wave: u8, out: &mut [Brick; GRID_LEN]) -> u16 {
    let def = &LEVELS[level_idx % NUM_LEVELS];
    let mut breakable = 0u16;
    for r in 0..ROWS {
        for c in 0..COLS {
            let mut kind = BrickKind::from_code(def.rows[r][c]);
            // Escalate armor on Wave 2+ for endless arcade replayability!
            if wave > 0 {
                let hash = (r * 7 + c * 3 + wave as usize) % 4;
                kind = match kind {
                    BrickKind::Crimson
                    | BrickKind::Orange
                    | BrickKind::Gold
                    | BrickKind::Emerald
                    | BrickKind::Cyan
                    | BrickKind::Violet
                        if hash == 0 =>
                    {
                        BrickKind::Silver
                    }
                    BrickKind::Silver if wave >= 2 || hash == 1 => BrickKind::Titanium,
                    other => other,
                };
            }
            let b = Brick::new(kind);
            out[r * COLS + c] = b;
            if kind.is_breakable() {
                breakable += 1;
            }
        }
    }
    breakable
}

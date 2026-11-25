//! Schatch data model: 6-day (Mon..Sat) x 8-lesson schedule, rooms, bell times,
//! accent theme settings, atomic NAND serialization, and live countdown calculator.

pub const DAYS: usize = 6; // 0 = Пн .. 5 = Сб
pub const LESSONS: usize = 8;
pub const ROOM_CAP: usize = 12;

pub const DAY_NAMES: [&str; DAYS] = [
    "Понедельник",
    "Вторник",
    "Среда",
    "Четверг",
    "Пятница",
    "Суббота",
];

pub const DAY_SHORT: [&str; DAYS] = ["Пн", "Вт", "Ср", "Чт", "Пт", "Сб"];

pub const CAT_NAMES: [&str; 4] = ["Точные", "Языки", "Науки", "Другие"];

pub struct SubjectInfo {
    pub name: &'static str,
    pub cat: u8,
}

/// Subject IDs are 1..=SUBJECTS.len(); 0 means empty slot ("Нет урока").
pub const SUBJECTS: &[SubjectInfo] = &[
    // 0: Точные (7)
    SubjectInfo { name: "Математика", cat: 0 },
    SubjectInfo { name: "Алгебра", cat: 0 },
    SubjectInfo { name: "Геометрия", cat: 0 },
    SubjectInfo { name: "Физика", cat: 0 },
    SubjectInfo { name: "Информатика", cat: 0 },
    SubjectInfo { name: "Вероятность", cat: 0 },
    SubjectInfo { name: "Экономика", cat: 0 },
    // 1: Языки (6)
    SubjectInfo { name: "Русский язык", cat: 1 },
    SubjectInfo { name: "Литература", cat: 1 },
    SubjectInfo { name: "Английский", cat: 1 },
    SubjectInfo { name: "Родной язык", cat: 1 },
    SubjectInfo { name: "Немецкий", cat: 1 },
    SubjectInfo { name: "Иностранный", cat: 1 },
    // 2: Науки (6)
    SubjectInfo { name: "Химия", cat: 2 },
    SubjectInfo { name: "Биология", cat: 2 },
    SubjectInfo { name: "География", cat: 2 },
    SubjectInfo { name: "Естествознание", cat: 2 },
    SubjectInfo { name: "Астрономия", cat: 2 },
    SubjectInfo { name: "Экология", cat: 2 },
    // 3: Другие (9)
    SubjectInfo { name: "История", cat: 3 },
    SubjectInfo { name: "Обществознание", cat: 3 },
    SubjectInfo { name: "Физкультура", cat: 3 },
    SubjectInfo { name: "Труд", cat: 3 },
    SubjectInfo { name: "ОБЖ", cat: 3 },
    SubjectInfo { name: "Классный час", cat: 3 },
    SubjectInfo { name: "Музыка", cat: 3 },
    SubjectInfo { name: "ИЗО", cat: 3 },
    SubjectInfo { name: "Право", cat: 3 },
];

pub fn subject_name(id: u8) -> &'static str {
    if id == 0 {
        return "Нет урока";
    }
    SUBJECTS.get((id - 1) as usize).map_or("Предмет", |s| s.name)
}

/// 12 curated dark-theme RGB565 accents (index 0 = signature Schatch lavender).
pub const ACCENTS: [i32; 12] = [
    0xD5BF, // 0: Лавандовый (#D4B4FF)
    0xFC74, // 1: Розовый (#FF8CA4)
    0xFA23, // 2: Красный CMF (#FF4519)
    0xFD24, // 3: Оранжевый (#FFA420)
    0xFFE6, // 4: Жёлтый (#FFFC30)
    0x9FA8, // 5: Лаймовый (#9CF440)
    0x5F72, // 6: Мятный (#5AEF90)
    0x4F7D, // 7: Бирюзовый (#48ECEA)
    0x5DDF, // 8: Голубой (#58BCFF)
    0x6B5F, // 9: Синий (#686BFF)
    0xB39F, // 10: Фиолетовый (#B072FF)
    0xFFFF, // 11: Белый (#FFFFFF)
];

/// Contrasting text color on top of an accent button/pill (dark text on bright pastel accents).
pub const fn accent_fg(idx: u8) -> i32 {
    match idx {
        2 | 9 => 0xFFFF, // CMF Red and Deep Blue take white text
        _ => 0x0841,     // Near-black on bright pastels
    }
}

pub const DEFAULT_BELLS: [(u16, u16); LESSONS] = [
    (8 * 60, 8 * 60 + 45),
    (8 * 60 + 55, 9 * 60 + 40),
    (9 * 60 + 55, 10 * 60 + 40),
    (10 * 60 + 55, 11 * 60 + 40),
    (11 * 60 + 55, 12 * 60 + 40),
    (12 * 60 + 50, 13 * 60 + 35),
    (13 * 60 + 45, 14 * 60 + 30),
    (14 * 60 + 40, 15 * 60 + 25),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Lesson {
    pub subject: u8,
    pub start_min: u16,
    pub end_min: u16,
    pub room_len: u8,
    pub room: [u8; ROOM_CAP],
}

impl Lesson {
    pub const fn empty(slot: usize) -> Self {
        let (s, e) = DEFAULT_BELLS[slot];
        Self { subject: 0, start_min: s, end_min: e, room_len: 0, room: [0; ROOM_CAP] }
    }

    pub fn room_str(&self) -> &str {
        let len = (self.room_len as usize).min(ROOM_CAP);
        core::str::from_utf8(&self.room[..len]).unwrap_or("")
    }

    pub fn set_room(&mut self, s: &str) {
        self.room = [0; ROOM_CAP];
        let mut n = 0;
        for ch in s.chars() {
            let clen = ch.len_utf8();
            if n + clen > ROOM_CAP {
                break;
            }
            ch.encode_utf8(&mut self.room[n..n + clen]);
            n += clen;
        }
        self.room_len = n as u8;
    }

    pub fn push_room_str(&mut self, add: &str) {
        let cur = self.room_len as usize;
        if cur + add.len() <= ROOM_CAP {
            self.room[cur..cur + add.len()].copy_from_slice(add.as_bytes());
            self.room_len = (cur + add.len()) as u8;
        }
    }

    pub fn pop_room_char(&mut self) {
        let mut n = (self.room_len as usize).min(ROOM_CAP);
        if n == 0 {
            return;
        }
        // Step back over UTF-8 continuation bytes (10xxxxxx)
        n -= 1;
        while n > 0 && (self.room[n] & 0xC0) == 0x80 {
            n -= 1;
        }
        for b in &mut self.room[n..] {
            *b = 0;
        }
        self.room_len = n as u8;
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Settings {
    pub accent_idx: u8,
    pub tint_text: bool,
    pub always_on: bool,
    pub vibrate: bool,
}

impl Settings {
    pub const fn default_new() -> Self {
        Self { accent_idx: 0, tint_text: false, always_on: false, vibrate: true }
    }

    pub fn accent(&self) -> i32 {
        ACCENTS[(self.accent_idx as usize).min(ACCENTS.len() - 1)]
    }

    pub fn text_primary(&self) -> i32 {
        if self.tint_text { self.accent() } else { 0xFFFF }
    }
}

pub struct Schedule {
    pub settings: Settings,
    pub days: [[Lesson; LESSONS]; DAYS],
}

const MAGIC: [u8; 4] = *b"SCH1";
pub const SAVE_BYTES: usize = 4 + 4 + DAYS * LESSONS * (1 + 2 + 2 + 1 + ROOM_CAP) + 4; // 876 bytes

impl Schedule {
    pub const fn empty() -> Self {
        let day = [
            Lesson::empty(0),
            Lesson::empty(1),
            Lesson::empty(2),
            Lesson::empty(3),
            Lesson::empty(4),
            Lesson::empty(5),
            Lesson::empty(6),
            Lesson::empty(7),
        ];
        Self { settings: Settings::default_new(), days: [day; DAYS] }
    }

    /// First-boot starter schedule so the app immediately looks alive and easy to customize.
    pub fn starter() -> Self {
        let mut s = Self::empty();
        // Sample Monday
        s.set_sample(0, 0, 25, "25"); // Классный час
        s.set_sample(0, 1, 5, "6");   // Информатика
        s.set_sample(0, 2, 1, "14");  // Математика
        s.set_sample(0, 3, 8, "18");  // Русский язык
        // Sample Tuesday..Saturday light presets
        s.set_sample(1, 0, 2, "14");  // Алгебра
        s.set_sample(1, 1, 4, "21");  // Физика
        s.set_sample(1, 2, 10, "9");  // Английский
        s.set_sample(2, 0, 3, "14");  // Геометрия
        s.set_sample(2, 1, 14, "32"); // Химия
        s.set_sample(2, 2, 20, "11"); // История
        s.set_sample(3, 0, 8, "18");  // Русский язык
        s.set_sample(3, 1, 15, "27"); // Биология
        s.set_sample(3, 2, 22, "СЗ"); // Физкультура
        s.set_sample(4, 0, 5, "6");   // Информатика
        s.set_sample(4, 1, 9, "18");  // Литература
        s.set_sample(4, 2, 16, "19"); // География
        s
    }

    fn set_sample(&mut self, day: usize, idx: usize, subject: u8, room: &str) {
        self.days[day][idx].subject = subject;
        self.days[day][idx].set_room(room);
    }

    pub fn clear_all_lessons(&mut self) {
        for d in 0..DAYS {
            for i in 0..LESSONS {
                self.days[d][i] = Lesson::empty(i);
            }
        }
    }

    pub fn apply_time_all_days(&mut self, lesson_idx: usize, start_min: u16, end_min: u16) {
        if lesson_idx >= LESSONS {
            return;
        }
        for d in 0..DAYS {
            self.days[d][lesson_idx].start_min = start_min;
            self.days[d][lesson_idx].end_min = end_min;
        }
    }

    pub fn sync_room_for_subject(&mut self, subject_id: u8, room: &str) {
        if subject_id == 0 {
            return;
        }
        for d in 0..DAYS {
            for i in 0..LESSONS {
                if self.days[d][i].subject == subject_id {
                    self.days[d][i].set_room(room);
                }
            }
        }
    }

    pub fn serialize(&self, out: &mut [u8; SAVE_BYTES]) {
        let mut p = 0;
        out[p..p + 4].copy_from_slice(&MAGIC);
        p += 4;
        out[p] = self.settings.accent_idx;
        out[p + 1] = self.settings.tint_text as u8;
        out[p + 2] = self.settings.always_on as u8;
        out[p + 3] = self.settings.vibrate as u8;
        p += 4;
        for d in 0..DAYS {
            for i in 0..LESSONS {
                let l = &self.days[d][i];
                out[p] = l.subject;
                out[p + 1..p + 3].copy_from_slice(&l.start_min.to_le_bytes());
                out[p + 3..p + 5].copy_from_slice(&l.end_min.to_le_bytes());
                out[p + 5] = l.room_len;
                out[p + 6..p + 6 + ROOM_CAP].copy_from_slice(&l.room);
                p += 6 + ROOM_CAP;
            }
        }
        let c = fnv32(&out[..p]);
        out[p..p + 4].copy_from_slice(&c.to_le_bytes());
    }

    pub fn deserialize(&mut self, data: &[u8]) -> bool {
        if data.len() != SAVE_BYTES || data[..4] != MAGIC {
            return false;
        }
        let expected = u32::from_le_bytes([data[SAVE_BYTES - 4], data[SAVE_BYTES - 3], data[SAVE_BYTES - 2], data[SAVE_BYTES - 1]]);
        if fnv32(&data[..SAVE_BYTES - 4]) != expected {
            return false;
        }
        let mut p = 4;
        self.settings.accent_idx = data[p].min((ACCENTS.len() - 1) as u8);
        self.settings.tint_text = data[p + 1] != 0;
        self.settings.always_on = data[p + 2] != 0;
        self.settings.vibrate = data[p + 3] != 0;
        p += 4;
        for d in 0..DAYS {
            for i in 0..LESSONS {
                let subj = data[p];
                let start = u16::from_le_bytes([data[p + 1], data[p + 2]]).min(23 * 60 + 59);
                let end = u16::from_le_bytes([data[p + 3], data[p + 4]]).min(23 * 60 + 59);
                let rlen = data[p + 5].min(ROOM_CAP as u8);
                let mut room = [0u8; ROOM_CAP];
                room.copy_from_slice(&data[p + 6..p + 6 + ROOM_CAP]);
                self.days[d][i] = Lesson {
                    subject: if subj as usize <= SUBJECTS.len() { subj } else { 0 },
                    start_min: start,
                    end_min: end,
                    room_len: rlen,
                    room,
                };
                p += 6 + ROOM_CAP;
            }
        }
        true
    }
}

fn fnv32(bytes: &[u8]) -> u32 {
    let mut h: u32 = 0x811C_9DC5;
    for &b in bytes {
        h ^= b as u32;
        h = h.wrapping_mul(0x0100_0193);
    }
    h
}

/// Maps RTC weekday (0 = Sunday, 1 = Monday .. 6 = Saturday) to `Some(0..6)` or `None` on Sunday.
pub fn rtc_to_day(weekday: u8) -> Option<usize> {
    match weekday {
        1..=6 => Some((weekday - 1) as usize),
        _ => None,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimerKind {
    Weekend,
    NoLessons,
    BeforeSchool { lesson_idx: usize },
    InLesson { lesson_idx: usize },
    Break { next_idx: usize },
    Done,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TimerInfo {
    pub kind: TimerKind,
    pub remain_sec: u32,
    /// 0..=100 for the circular LVGL Arc
    pub arc_pct: i32,
}

pub fn compute_timer(sched: &Schedule, weekday: u8, hour: u8, minute: u8, second: u8) -> TimerInfo {
    let Some(day) = rtc_to_day(weekday) else {
        return TimerInfo { kind: TimerKind::Weekend, remain_sec: 0, arc_pct: 0 };
    };
    let now_sec = (hour as u32) * 3600 + (minute as u32) * 60 + (second as u32);

    // Collect indices of filled lessons in chronological order (insertion sort, <= 8 items)
    let mut order = [0usize; LESSONS];
    let mut count = 0usize;
    for i in 0..LESSONS {
        if sched.days[day][i].subject != 0 {
            order[count] = i;
            count += 1;
        }
    }
    if count == 0 {
        return TimerInfo { kind: TimerKind::NoLessons, remain_sec: 0, arc_pct: 0 };
    }
    for i in 1..count {
        let mut j = i;
        while j > 0 && sched.days[day][order[j]].start_min < sched.days[day][order[j - 1]].start_min {
            order.swap(j, j - 1);
            j -= 1;
        }
    }

    let first = &sched.days[day][order[0]];
    let first_start = (first.start_min as u32) * 60;
    if now_sec < first_start {
        let rem = first_start - now_sec;
        return TimerInfo { kind: TimerKind::BeforeSchool { lesson_idx: order[0] }, remain_sec: rem, arc_pct: 100 };
    }

    for k in 0..count {
        let idx = order[k];
        let l = &sched.days[day][idx];
        let st = (l.start_min as u32) * 60;
        let en = (l.end_min as u32) * 60;
        if now_sec >= st && now_sec < en {
            let total = (en - st).max(1);
            let rem = en - now_sec;
            let pct = ((rem * 100) / total) as i32;
            return TimerInfo { kind: TimerKind::InLesson { lesson_idx: idx }, remain_sec: rem, arc_pct: pct.clamp(0, 100) };
        }
        if k + 1 < count {
            let next_idx = order[k + 1];
            let next_st = (sched.days[day][next_idx].start_min as u32) * 60;
            if now_sec >= en && now_sec < next_st {
                let total = (next_st - en).max(1);
                let rem = next_st - now_sec;
                let pct = ((rem * 100) / total) as i32;
                return TimerInfo { kind: TimerKind::Break { next_idx }, remain_sec: rem, arc_pct: pct.clamp(0, 100) };
            }
        }
    }

    TimerInfo { kind: TimerKind::Done, remain_sec: 0, arc_pct: 0 }
}

/// Zero-alloc UTF-8 scratch formatter (safe on 8 KiB stack!).
pub struct StrBuf<const N: usize> {
    buf: [u8; N],
    len: usize,
}

impl<const N: usize> StrBuf<N> {
    pub const fn new() -> Self {
        Self { buf: [0; N], len: 0 }
    }

    pub fn push_str(&mut self, s: &str) {
        let take = s.len().min(N - self.len);
        // Keep UTF-8 boundary
        let mut end = take;
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        self.buf[self.len..self.len + end].copy_from_slice(&s.as_bytes()[..end]);
        self.len += end;
    }

    pub fn push_u2(&mut self, v: u32) {
        let v = v % 100;
        self.push_byte(b'0' + (v / 10) as u8);
        self.push_byte(b'0' + (v % 10) as u8);
    }

    pub fn push_num(&mut self, mut v: u32) {
        if v == 0 {
            self.push_byte(b'0');
            return;
        }
        let mut tmp = [0u8; 10];
        let mut k = 0;
        while v > 0 {
            tmp[k] = b'0' + (v % 10) as u8;
            v /= 10;
            k += 1;
        }
        while k > 0 {
            k -= 1;
            self.push_byte(tmp[k]);
        }
    }

    pub fn push_byte(&mut self, b: u8) {
        if self.len < N {
            self.buf[self.len] = b;
            self.len += 1;
        }
    }

    pub fn as_str(&self) -> &str {
        core::str::from_utf8(&self.buf[..self.len]).unwrap_or("")
    }
}

pub fn fmt_time_range(start_min: u16, end_min: u16) -> StrBuf<16> {
    let mut b = StrBuf::new();
    b.push_u2((start_min / 60) as u32);
    b.push_byte(b':');
    b.push_u2((start_min % 60) as u32);
    b.push_byte(b'-');
    b.push_u2((end_min / 60) as u32);
    b.push_byte(b':');
    b.push_u2((end_min % 60) as u32);
    b
}

pub fn fmt_countdown(sec: u32) -> StrBuf<16> {
    let mut b = StrBuf::new();
    let mins = sec / 60;
    let rem_s = sec % 60;
    if mins >= 60 {
        b.push_u2(mins / 60);
        b.push_byte(b':');
        b.push_u2(mins % 60);
    } else {
        b.push_u2(mins);
        b.push_byte(b':');
        b.push_u2(rem_s);
    }
    b
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serialize_roundtrip_preserves_everything() {
        let mut s = Schedule::starter();
        s.settings.accent_idx = 3;
        s.settings.tint_text = true;
        s.settings.always_on = true;
        s.settings.vibrate = false;
        s.days[2][4].subject = 7;
        s.days[2][4].start_min = 12 * 60 + 15;
        s.days[2][4].end_min = 13 * 60;
        s.days[2][4].set_room("304Б");

        let mut buf = [0u8; SAVE_BYTES];
        s.serialize(&mut buf);

        let mut loaded = Schedule::empty();
        assert!(loaded.deserialize(&buf));
        assert_eq!(loaded.settings, s.settings);
        assert_eq!(loaded.days[2][4].subject, 7);
        assert_eq!(loaded.days[2][4].room_str(), "304Б");
        assert_eq!(loaded.days[0][1].room_str(), "6");

        // Corrupt 1 byte -> rejected
        buf[20] ^= 0x55;
        assert!(!loaded.deserialize(&buf));
    }

    #[test]
    fn room_utf8_push_and_pop() {
        let mut l = Lesson::empty(0);
        l.push_room_str("25");
        l.push_room_str("А");
        l.push_room_str("Б");
        assert_eq!(l.room_str(), "25АБ");
        l.pop_room_char();
        assert_eq!(l.room_str(), "25А");
        l.pop_room_char();
        assert_eq!(l.room_str(), "25");
    }

    #[test]
    fn timer_detects_before_lesson_break_done_and_weekend() {
        let s = Schedule::starter();
        // Sunday (0)
        assert_eq!(compute_timer(&s, 0, 10, 0, 0).kind, TimerKind::Weekend);
        // Monday 07:30 -> before lesson 0 (08:00)
        let before = compute_timer(&s, 1, 7, 30, 0);
        assert_eq!(before.kind, TimerKind::BeforeSchool { lesson_idx: 0 });
        assert_eq!(before.remain_sec, 30 * 60);
        // Monday 08:20 -> inside lesson 0 (08:00-08:45)
        let in_l = compute_timer(&s, 1, 8, 20, 0);
        assert_eq!(in_l.kind, TimerKind::InLesson { lesson_idx: 0 });
        assert_eq!(in_l.remain_sec, 25 * 60);
        // Monday 08:50 -> break before lesson 1 (08:55)
        let brk = compute_timer(&s, 1, 8, 50, 0);
        assert_eq!(brk.kind, TimerKind::Break { next_idx: 1 });
        assert_eq!(brk.remain_sec, 5 * 60);
        // Monday 16:00 -> done
        assert_eq!(compute_timer(&s, 1, 16, 0, 0).kind, TimerKind::Done);
    }

    #[test]
    fn apply_time_and_sync_room() {
        let mut s = Schedule::starter();
        s.apply_time_all_days(0, 8 * 60 + 30, 9 * 60 + 15);
        for d in 0..DAYS {
            assert_eq!(s.days[d][0].start_min, 8 * 60 + 30);
            assert_eq!(s.days[d][0].end_min, 9 * 60 + 15);
        }
        s.sync_room_for_subject(5, "404"); // Информатика (Mon slot 1 & Fri slot 0)
        assert_eq!(s.days[0][1].room_str(), "404");
        assert_eq!(s.days[4][0].room_str(), "404");
    }
}

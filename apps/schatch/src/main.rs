//! Schatch — школьное расписание уроков и таймер звонков для CMF Watch Pro 2.
//! Нативный LVGL 8.3 (ABI 1.3), энергонезависимая память NAND, виджет на главный экран,
//! 12 акцентных тем, перекрас текста, барабаны времени и нумпад кабинетов.
#![cfg_attr(target_os = "none", no_std, no_main)]

#[cfg_attr(not(target_os = "none"), allow(dead_code))]
mod data;
#[cfg_attr(not(target_os = "none"), allow(dead_code))]
mod ui;

#[cfg(not(target_os = "none"))]
fn main() {}

#[cfg(target_os = "none")]
mod device {
    use crate::data::*;
    use crate::ui::{self, Action, Screen, UiState};
    use cmf_wp2_app::lv::{Batch, Lv};
    use cmf_wp2_app::{Api, BUTTON, LV, PAUSE, RESUME, WIDGET};
    use cmf_wp2_ui::haptic::{ALERT, DONE, Motor, TICK};
    use core::ptr::addr_of_mut;

    cmf_wp2_app::entry!(main);
    cmf_wp2_app::app_icon!();
    cmf_wp2_app::app_features!(WIDGET);

    const CLICKED: i16 = 7;
    const VALUE_CHANGED: i16 = 28;

    struct AppState {
        sched: Schedule,
        ui: UiState,
        screen: Screen,
        view_day: usize,
        edit_idx: usize,
        active_cat: u8,
        // Time editor draft
        sh: u8,
        sm: u8,
        eh: u8,
        em: u8,
        apply_all_days: bool,
        // Room editor draft
        room_draft: Lesson,
        sync_room: bool,
        // Last timer kind for bell transition haptics
        last_timer: Option<TimerKind>,
        dirty: bool,
        save_buf: [u8; SAVE_BYTES],
    }

    static mut STATE: AppState = AppState {
        sched: Schedule::empty(),
        ui: UiState::new(),
        screen: Screen::Schedule,
        view_day: 0,
        edit_idx: 0,
        active_cat: 0,
        sh: 8,
        sm: 0,
        eh: 8,
        em: 45,
        apply_all_days: false,
        room_draft: Lesson::empty(0),
        sync_room: false,
        last_timer: None,
        dirty: false,
        save_buf: [0; SAVE_BYTES],
    };

    fn st() -> &'static mut AppState {
        unsafe { &mut *addr_of_mut!(STATE) }
    }

    fn wall_time(api: &Api) -> (u8, u8, u8, u8) {
        match api.time() {
            Some(t) => (t.weekday, t.hour, t.minute, t.second),
            None => (1, 8, 15, 0),
        }
    }

    fn load_or_init(api: &Api, s: &mut AppState) {
        let n = api.load(&mut s.save_buf);
        if n == SAVE_BYTES as i32 && s.sched.deserialize(&s.save_buf) {
            return;
        }
        s.sched = Schedule::starter();
        save_now(api, s);
    }

    fn save_now(api: &Api, s: &mut AppState) {
        s.dirty = false;
        s.sched.serialize(&mut s.save_buf);
        let _ = api.save(&s.save_buf);
        update_widget(api, s);
    }

    fn flush_save(api: &Api, s: &mut AppState) {
        if s.dirty {
            save_now(api, s);
        }
    }

    fn update_widget(api: &Api, s: &AppState) {
        let (wd, h, m, sec) = wall_time(api);
        let info = compute_timer(&s.sched, wd, h, m, sec);
        let mut val = StrBuf::<39>::new();
        match (info.kind, rtc_to_day(wd)) {
            (TimerKind::Weekend, _) => val.push_str("Выходной"),
            (TimerKind::NoLessons, _) => val.push_str("Нет уроков"),
            (TimerKind::Done, _) => val.push_str("Уроки окончены"),
            (TimerKind::InLesson { lesson_idx }, Some(d)) => {
                let l = &s.sched.days[d][lesson_idx];
                val.push_str(subject_name(l.subject));
                val.push_byte(b'\n');
                let tr = fmt_time_range(l.start_min, l.end_min);
                val.push_str(tr.as_str());
                if !l.room_str().is_empty() {
                    val.push_str(" к.");
                    val.push_str(l.room_str());
                }
            }
            (TimerKind::BeforeSchool { lesson_idx }, Some(d))
            | (TimerKind::Break { next_idx: lesson_idx }, Some(d)) => {
                let l = &s.sched.days[d][lesson_idx];
                val.push_str(subject_name(l.subject));
                val.push_byte(b'\n');
                let tr = fmt_time_range(l.start_min, l.end_min);
                val.push_str(tr.as_str());
            }
            _ => val.push_str("Расписание"),
        }
        let _ = api.widget("Schatch", val.as_str());
    }

    fn rebuild(api: &Api, b: &mut Batch, lv: &Lv, s: &mut AppState) {
        let (wd, h, m, sec) = wall_time(api);
        // Intercept back swipe on subscreens; let main Schedule exit cleanly
        api.own_back(s.screen != Screen::Schedule);
        let _ = match s.screen {
            Screen::Schedule => ui::build_schedule(b, lv, &mut s.ui, &s.sched, s.view_day, wd, h, m, sec),
            Screen::Timer => {
                let r = ui::build_timer(b, lv, &mut s.ui, &s.sched, wd, h, m, sec);
                s.last_timer = Some(compute_timer(&s.sched, wd, h, m, sec).kind);
                r
            }
            Screen::Days => ui::build_days(b, lv, &mut s.ui, &s.sched, s.view_day),
            Screen::LessonEdit => ui::build_lesson_edit(b, lv, &mut s.ui, &s.sched, s.view_day, s.edit_idx),
            Screen::SubjectPick => {
                let cur = s.sched.days[s.view_day][s.edit_idx].subject;
                ui::build_subject_pick(b, lv, &mut s.ui, &s.sched, s.active_cat, cur)
            }
            Screen::TimeEdit => ui::build_time_edit(
                b,
                lv,
                &mut s.ui,
                &s.sched,
                s.edit_idx,
                s.sh,
                s.sm,
                s.eh,
                s.em,
                s.apply_all_days,
            ),
            Screen::RoomEdit => {
                ui::build_room_edit(b, lv, &mut s.ui, &s.sched, s.room_draft.room_str(), s.sync_room)
            }
            Screen::Settings => ui::build_settings(b, lv, &mut s.ui, &s.sched),
            Screen::ConfirmReset => ui::build_confirm_reset(b, lv, &mut s.ui, &s.sched),
        };
    }

    fn tick_vibe(s: &AppState, motor: &mut Motor) {
        if s.sched.settings.vibrate {
            motor.play(TICK);
        }
    }

    fn main(api: &'static Api) -> i32 {
        let Some(lv) = api.lv() else { return -95 };
        let Some(mut b) = Batch::new() else { return -1 };
        let s = st();
        load_or_init(api, s);

        let (wd, _, _, _) = wall_time(api);
        s.view_day = rtc_to_day(wd).unwrap_or(0);
        s.screen = Screen::Schedule;

        if ui::init_fonts(&mut b, &lv, &mut s.ui).is_err() {
            return -2;
        }
        api.keep_screen_on(s.sched.settings.always_on);
        update_widget(api, s);
        rebuild(api, &mut b, &lv, s);

        let mut motor = Motor::new(api);
        let mut last_sec: u8 = 255;

        loop {
            motor.update();
            // On Timer screen wake every ~250 ms for crisp 1s ticking; on Schedule wake every 15s
            let base_wait = if s.screen == Screen::Timer { 250 } else { 15_000 };
            let wait = motor.wait().map_or(base_wait, |w| w.min(base_wait)).max(1);
            let ev = api.event(wait);

            // Live 1 Hz Timer refresh
            if s.screen == Screen::Timer {
                let (wd, h, m, sec) = wall_time(api);
                if sec != last_sec {
                    last_sec = sec;
                    if let Ok(kind) = ui::update_timer_live(&mut b, &lv, &s.ui, &s.sched, wd, h, m, sec) {
                        if let Some(prev) = s.last_timer {
                            if prev != kind && s.sched.settings.vibrate {
                                motor.play(ALERT);
                            }
                        }
                        s.last_timer = Some(kind);
                    }
                }
            }

            let Some(e) = ev else { continue };
            match e.kind {
                BUTTON => {
                    tick_vibe(s, &mut motor);
                    match s.screen {
                        Screen::Schedule => {
                            flush_save(api, s);
                            break;
                        }
                        Screen::Timer | Screen::Days | Screen::Settings => {
                            flush_save(api, s);
                            s.screen = Screen::Schedule;
                            rebuild(api, &mut b, &lv, s);
                        }
                        Screen::LessonEdit => {
                            flush_save(api, s);
                            s.screen = Screen::Schedule;
                            rebuild(api, &mut b, &lv, s);
                        }
                        Screen::SubjectPick | Screen::TimeEdit | Screen::RoomEdit => {
                            s.screen = Screen::LessonEdit;
                            rebuild(api, &mut b, &lv, s);
                        }
                        Screen::ConfirmReset => {
                            s.screen = Screen::Settings;
                            rebuild(api, &mut b, &lv, s);
                        }
                    }
                }
                PAUSE => {
                    motor.off();
                    flush_save(api, s);
                }
                RESUME => {
                    api.keep_screen_on(s.sched.settings.always_on);
                    last_sec = 255;
                }
                LV => {
                    let act = s.ui.lookup(e.x as u16);
                    if e.y == VALUE_CHANGED {
                        let v = e.delta.max(0) as u8;
                        match act {
                            Action::RollerStartH => s.sh = v.min(23),
                            Action::RollerStartM => s.sm = v.min(59),
                            Action::RollerEndH => s.eh = v.min(23),
                            Action::RollerEndM => s.em = v.min(59),
                            _ => {}
                        }
                        continue;
                    }
                    if e.y != CLICKED || act == Action::None {
                        continue;
                    }
                    tick_vibe(s, &mut motor);
                    match act {
                        Action::None => {}
                        Action::GoSchedule => {
                            flush_save(api, s);
                            s.screen = Screen::Schedule;
                            rebuild(api, &mut b, &lv, s);
                        }
                        Action::GoTimer => {
                            s.screen = Screen::Timer;
                            last_sec = 255;
                            rebuild(api, &mut b, &lv, s);
                        }
                        Action::GoDays => {
                            s.screen = Screen::Days;
                            rebuild(api, &mut b, &lv, s);
                        }
                        Action::GoSettings => {
                            s.screen = Screen::Settings;
                            rebuild(api, &mut b, &lv, s);
                        }
                        Action::GoToday => {
                            let (wd, _, _, _) = wall_time(api);
                            s.view_day = rtc_to_day(wd).unwrap_or(0);
                            s.screen = Screen::Schedule;
                            rebuild(api, &mut b, &lv, s);
                        }
                        Action::PickDay(d) => {
                            s.view_day = (d as usize).min(DAYS - 1);
                            s.screen = Screen::Schedule;
                            rebuild(api, &mut b, &lv, s);
                        }
                        Action::EditLesson(idx) => {
                            s.edit_idx = (idx as usize).min(LESSONS - 1);
                            s.screen = Screen::LessonEdit;
                            rebuild(api, &mut b, &lv, s);
                        }
                        Action::AddNextLesson => {
                            if let Some(slot) = (0..LESSONS).find(|&i| s.sched.days[s.view_day][i].subject == 0) {
                                s.edit_idx = slot;
                                s.active_cat = 0;
                                s.screen = Screen::SubjectPick;
                                rebuild(api, &mut b, &lv, s);
                            }
                        }
                        Action::GoSubjectPick => {
                            let cur = s.sched.days[s.view_day][s.edit_idx].subject;
                            if cur > 0 {
                                if let Some(info) = SUBJECTS.get((cur - 1) as usize) {
                                    s.active_cat = info.cat;
                                }
                            }
                            s.screen = Screen::SubjectPick;
                            rebuild(api, &mut b, &lv, s);
                        }
                        Action::GoTimeEdit => {
                            let l = s.sched.days[s.view_day][s.edit_idx];
                            s.sh = (l.start_min / 60) as u8;
                            s.sm = (l.start_min % 60) as u8;
                            s.eh = (l.end_min / 60) as u8;
                            s.em = (l.end_min % 60) as u8;
                            s.apply_all_days = false;
                            s.screen = Screen::TimeEdit;
                            rebuild(api, &mut b, &lv, s);
                        }
                        Action::GoRoomEdit => {
                            s.room_draft = s.sched.days[s.view_day][s.edit_idx];
                            s.sync_room = false;
                            s.screen = Screen::RoomEdit;
                            rebuild(api, &mut b, &lv, s);
                        }
                        Action::ClearLesson => {
                            s.sched.days[s.view_day][s.edit_idx].subject = 0;
                            s.sched.days[s.view_day][s.edit_idx].set_room("");
                            save_now(api, s);
                            s.screen = Screen::Schedule;
                            rebuild(api, &mut b, &lv, s);
                        }
                        Action::SelectCat(cat) => {
                            s.active_cat = cat.min(3);
                            rebuild(api, &mut b, &lv, s);
                        }
                        Action::SelectSubject(id) => {
                            s.sched.days[s.view_day][s.edit_idx].subject = id;
                            s.dirty = true;
                            s.screen = Screen::LessonEdit;
                            rebuild(api, &mut b, &lv, s);
                        }
                        Action::RollerStartH | Action::RollerStartM | Action::RollerEndH | Action::RollerEndM => {}
                        Action::ToggleApplyAllDays => {
                            s.apply_all_days = !s.apply_all_days;
                            let _ = ui::update_check_pill(
                                &mut b,
                                &lv,
                                &s.ui,
                                &s.sched,
                                s.apply_all_days,
                                "[+] Ко всем дням",
                                "[ ] Ко всем дням",
                            );
                        }
                        Action::SaveTime => {
                            let start = (s.sh as u16) * 60 + (s.sm as u16);
                            let mut end = (s.eh as u16) * 60 + (s.em as u16);
                            if end <= start {
                                end = (start + 45).min(23 * 60 + 59);
                            }
                            if s.apply_all_days {
                                s.sched.apply_time_all_days(s.edit_idx, start, end);
                            } else {
                                s.sched.days[s.view_day][s.edit_idx].start_min = start;
                                s.sched.days[s.view_day][s.edit_idx].end_min = end;
                            }
                            s.dirty = true;
                            if s.sched.settings.vibrate {
                                motor.play(DONE);
                            }
                            s.screen = Screen::LessonEdit;
                            rebuild(api, &mut b, &lv, s);
                        }
                        Action::RoomKey(k) => {
                            if let Some(&ch) = ui::ROOM_KEYS.get(k as usize) {
                                s.room_draft.push_room_str(ch);
                                let _ = ui::update_room_preview(&mut b, &lv, &s.ui, s.room_draft.room_str());
                            }
                        }
                        Action::RoomBackspace => {
                            s.room_draft.pop_room_char();
                            let _ = ui::update_room_preview(&mut b, &lv, &s.ui, s.room_draft.room_str());
                        }
                        Action::RoomClear => {
                            s.room_draft.set_room("");
                            let _ = ui::update_room_preview(&mut b, &lv, &s.ui, "");
                        }
                        Action::ToggleSyncRoom => {
                            s.sync_room = !s.sync_room;
                            let _ = ui::update_check_pill(
                                &mut b,
                                &lv,
                                &s.ui,
                                &s.sched,
                                s.sync_room,
                                "[+] Синхр. каб.",
                                "[ ] Синхр. каб.",
                            );
                        }
                        Action::SaveRoom => {
                            let subj = s.sched.days[s.view_day][s.edit_idx].subject;
                            let mut copy = [0u8; ROOM_CAP];
                            let len = s.room_draft.room_len as usize;
                            copy[..len].copy_from_slice(&s.room_draft.room[..len]);
                            let rstr = core::str::from_utf8(&copy[..len]).unwrap_or("");
                            s.sched.days[s.view_day][s.edit_idx].set_room(rstr);
                            if s.sync_room && subj != 0 {
                                s.sched.sync_room_for_subject(subj, rstr);
                            }
                            s.dirty = true;
                            if s.sched.settings.vibrate {
                                motor.play(DONE);
                            }
                            s.screen = Screen::LessonEdit;
                            rebuild(api, &mut b, &lv, s);
                        }
                        Action::PickAccent(idx) => {
                            s.sched.settings.accent_idx = idx.min((ACCENTS.len() - 1) as u8);
                            s.dirty = true;
                            rebuild(api, &mut b, &lv, s);
                        }
                        Action::ToggleTintText => {
                            s.sched.settings.tint_text = !s.sched.settings.tint_text;
                            s.dirty = true;
                            rebuild(api, &mut b, &lv, s);
                        }
                        Action::ToggleAlwaysOn => {
                            s.sched.settings.always_on = !s.sched.settings.always_on;
                            api.keep_screen_on(s.sched.settings.always_on);
                            s.dirty = true;
                            let _ = ui::update_switch_pill(
                                &mut b,
                                &lv,
                                &s.sched,
                                s.ui.sw_always_pill,
                                s.ui.sw_always_lbl,
                                s.sched.settings.always_on,
                            );
                        }
                        Action::ToggleVibrate => {
                            s.sched.settings.vibrate = !s.sched.settings.vibrate;
                            s.dirty = true;
                            let _ = ui::update_switch_pill(
                                &mut b,
                                &lv,
                                &s.sched,
                                s.ui.sw_vibe_pill,
                                s.ui.sw_vibe_lbl,
                                s.sched.settings.vibrate,
                            );
                        }
                        Action::AskReset => {
                            s.screen = Screen::ConfirmReset;
                            rebuild(api, &mut b, &lv, s);
                        }
                        Action::DoReset => {
                            s.sched.clear_all_lessons();
                            save_now(api, s);
                            if s.sched.settings.vibrate {
                                motor.play(DONE);
                            }
                            s.screen = Screen::Schedule;
                            rebuild(api, &mut b, &lv, s);
                        }
                    }
                }
                _ => {}
            }
        }
        flush_save(api, s);
        api.keep_screen_on(false);
        motor.off();
        0
    }
}

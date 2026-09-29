//! Icons, status glyphs, colours and spinner scheduling.
use alfredo_tui::theme::{
    ColorMode, IconSet, Record, RowStatus, SpinnerSchedule, Theme, Tone, ASCII_SPINNER, BRAILLE,
};
use ratatui::style::Color;
use std::time::{Duration, Instant};
use unicode_width::UnicodeWidthStr;

const STATUSES: [RowStatus; 11] = [
    RowStatus::Working,
    RowStatus::Queued,
    RowStatus::Review,
    RowStatus::Decision,
    RowStatus::Failed,
    RowStatus::Blocked,
    RowStatus::Complete,
    RowStatus::Idle,
    RowStatus::Pending,
    RowStatus::Cancelled,
    RowStatus::Unverified,
];

#[test]
fn unicode_and_ascii_glyphs_and_spinner_frames_are_one_cell() {
    for set in [IconSet::Unicode, IconSet::Ascii] {
        for record in [Record::Task, Record::Repair, Record::Agent] {
            assert_eq!(set.record(record).width(), 1, "{set:?} {record:?}");
        }
        for expanded in [true, false] {
            assert_eq!(set.fold(expanded).width(), 1, "{set:?}");
        }
        for status in STATUSES {
            for motion in [true, false] {
                let theme = Theme {
                    icons: set,
                    motion,
                    color: ColorMode::Ansi16,
                };
                let glyph = theme.status_glyph(status, Instant::now());
                assert_eq!(glyph.width(), 1, "{set:?} {status:?} {glyph}");
            }
        }
    }
    for frame in BRAILLE.iter().chain(ASCII_SPINNER.iter()) {
        assert_eq!(frame.width(), 1, "{frame}");
    }
    assert_eq!(BRAILLE.concat(), "⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏");
}

#[test]
fn record_icons_follow_the_spec_table() {
    assert_eq!(
        [Record::Task, Record::Repair, Record::Agent].map(|r| IconSet::Unicode.record(r)),
        ["▤", "⑂", "◈"]
    );
    assert_eq!(
        [Record::Task, Record::Repair, Record::Agent].map(|r| IconSet::Ascii.record(r)),
        ["#", "Y", "@"]
    );
    // Nerd Font private-use codepoints, documented in theme.rs.
    assert_eq!(
        [Record::Task, Record::Repair, Record::Agent].map(|r| IconSet::Nerd.record(r)),
        ["\u{f145}", "\u{e725}", "\u{f06a9}"]
    );
    assert_eq!(IconSet::parse("nerd"), Ok(IconSet::Nerd));
    assert_eq!(IconSet::parse("unicode"), Ok(IconSet::Unicode));
    assert_eq!(IconSet::parse("ascii"), Ok(IconSet::Ascii));
    assert!(IconSet::parse("emoji").is_err());
    assert_eq!(IconSet::default(), IconSet::Unicode);
}

#[test]
fn status_glyphs_follow_the_spec_table_and_motion_setting() {
    let theme = Theme::default();
    for (status, glyph) in [
        (RowStatus::Queued, "◌"),
        (RowStatus::Review, "◐"),
        (RowStatus::Decision, "●"),
        (RowStatus::Failed, "✗"),
        (RowStatus::Blocked, "‖"),
        (RowStatus::Complete, "✓"),
        (RowStatus::Idle, "○"),
    ] {
        assert_eq!(theme.status_glyph(status, Instant::now()), glyph);
    }
    let working = theme.status_glyph(RowStatus::Working, Instant::now());
    assert!(BRAILLE.contains(&working), "{working}");
    let still = Theme {
        motion: false,
        ..Theme::default()
    };
    assert_eq!(still.status_glyph(RowStatus::Working, Instant::now()), "▶");
    // Frames advance every 100 ms.
    let start = Instant::now();
    let frames: std::collections::BTreeSet<_> = (0..10)
        .map(|step| {
            theme.status_glyph(
                RowStatus::Working,
                start + Duration::from_millis(step * 100),
            )
        })
        .collect();
    assert_eq!(frames.len(), 10);
}

#[test]
fn environment_selects_icons_motion_and_colour_mode() {
    let env = |pairs: &'static [(&'static str, &'static str)]| {
        move |key: &str| {
            pairs
                .iter()
                .find(|(name, _)| *name == key)
                .map(|(_, value)| value.to_string())
        }
    };
    let theme = Theme::from_env(env(&[])).unwrap();
    assert_eq!(theme, Theme::default());
    assert_eq!(theme.color, ColorMode::Ansi16);
    let theme = Theme::from_env(env(&[
        ("ALFREDO_ICONS", "ascii"),
        ("ALFREDO_NO_MOTION", "1"),
        ("COLORTERM", "truecolor"),
    ]))
    .unwrap();
    assert_eq!(theme.icons, IconSet::Ascii);
    assert!(!theme.motion);
    assert_eq!(theme.color, ColorMode::Truecolor);
    assert_eq!(
        Theme::from_env(env(&[("COLORTERM", "24bit")]))
            .unwrap()
            .color,
        ColorMode::Truecolor
    );
    assert_eq!(
        Theme::from_env(env(&[("COLORTERM", "truecolor"), ("NO_COLOR", "1")]))
            .unwrap()
            .color,
        ColorMode::None
    );
    // An empty NO_COLOR does not disable colour.
    assert_eq!(
        Theme::from_env(env(&[("NO_COLOR", "")])).unwrap().color,
        ColorMode::Ansi16
    );
    assert!(Theme::from_env(env(&[("ALFREDO_ICONS", "emoji")])).is_err());
}

#[test]
fn palette_is_truecolor_or_nearest_named_colour() {
    assert_eq!(
        ColorMode::Truecolor.color(Tone::Green),
        Color::Rgb(0x75, 0xd9, 0x9b)
    );
    assert_eq!(
        ColorMode::Truecolor.color(Tone::Amber),
        Color::Rgb(0xe8, 0xb4, 0x58)
    );
    assert_eq!(
        ColorMode::Truecolor.color(Tone::Red),
        Color::Rgb(0xec, 0x76, 0x6f)
    );
    assert_eq!(
        ColorMode::Truecolor.color(Tone::Blue),
        Color::Rgb(0x7e, 0xb7, 0xff)
    );
    assert_eq!(
        ColorMode::Truecolor.color(Tone::Cyan),
        Color::Rgb(0x72, 0xd6, 0xcf)
    );
    assert_eq!(
        ColorMode::Truecolor.color(Tone::Dim),
        Color::Rgb(0x6e, 0x74, 0x6f)
    );
    assert_eq!(
        ColorMode::Truecolor.color(Tone::Lime),
        Color::Rgb(0xb6, 0xf3, 0x6b)
    );
    for (tone, named) in [
        (Tone::Green, Color::Green),
        (Tone::Amber, Color::Yellow),
        (Tone::Red, Color::Red),
        (Tone::Blue, Color::LightBlue),
        (Tone::Cyan, Color::Cyan),
        (Tone::Dim, Color::DarkGray),
        (Tone::Lime, Color::LightGreen),
        (Tone::Magenta, Color::Magenta),
    ] {
        assert_eq!(ColorMode::Ansi16.color(tone), named);
        assert_eq!(ColorMode::None.color(tone), Color::Reset);
    }
}

/// Simulated event loop: 33 ms polls for 2 s, counting spinner-driven redraws.
fn redraws(working: bool, motion: bool) -> usize {
    let mut schedule = SpinnerSchedule::default();
    let start = Instant::now();
    (0..60)
        .filter(|step| schedule.due(start + Duration::from_millis(step * 33), working, motion))
        .count()
}

#[test]
fn spinner_redraws_only_while_something_is_working() {
    let busy = redraws(true, true);
    assert!((19..=22).contains(&busy), "{busy}");
    assert_eq!(redraws(false, true), 0);
    assert_eq!(redraws(true, false), 0);
    // Once work stops, no further redraws are scheduled.
    let mut schedule = SpinnerSchedule::default();
    let start = Instant::now();
    assert!(schedule.due(start, true, true));
    for step in 1..100 {
        assert!(!schedule.due(start + Duration::from_millis(step * 50), false, true));
    }
}

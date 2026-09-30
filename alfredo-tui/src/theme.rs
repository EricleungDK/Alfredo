//! Presentation vocabulary: record icons, status glyphs, palette and spinner.
//! Pure values only; nothing here reads state or performs IO.
use ratatui::style::Color;
use std::{sync::OnceLock, time::Instant};

/// Braille spinner, one frame per 100 ms.
pub const BRAILLE: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
pub const ASCII_SPINNER: [&str; 4] = ["|", "/", "-", "\\"];
const FRAME_MS: u128 = 100;
/// Columns of the dotted track the capacity-wait cat runs along.
pub const CAT_TRACK: usize = 25;
/// Where the cat starts on frame 1, which is also the static `--no-motion` frame.
const CAT_START: u64 = 7;
const KAOMOJI: &str = "(=^･ω･^=)";
const ASCII_CAT: &str = "=^.^=";

/// One frame of the capacity-wait cat: a kaomoji (or `=^.^=` in ASCII mode) with
/// a blank margin either side, one cell further right each `tick` and wrapping
/// around a dotted track. Paws alternate left, right, both; frame 1 has both.
/// The track is `CAT_TRACK` columns, or `width` when narrower; the result is
/// empty when the cat itself cannot fit. Every glyph is one column wide, so
/// frames always occupy exactly the returned width.
pub fn cat_track(icons: IconSet, tick: u64, width: usize) -> String {
    let ascii = icons == IconSet::Ascii;
    let sprite: Vec<char> = if ascii {
        format!(" {ASCII_CAT} ").chars().collect()
    } else {
        let left = if tick % 3 == 2 { ' ' } else { 'ฅ' };
        let right = if tick % 3 == 1 { ' ' } else { 'ฅ' };
        format!(" {left}{KAOMOJI}{right} ").chars().collect()
    };
    let track = width.min(CAT_TRACK);
    if track < sprite.len() {
        return String::new();
    }
    let dot = if ascii { '.' } else { '·' };
    let mut cells: Vec<char> = (0..track)
        .map(|cell| if cell % 2 == 0 { dot } else { ' ' })
        .collect();
    let origin = ((CAT_START + tick) % track as u64) as usize;
    for (offset, glyph) in sprite.into_iter().enumerate() {
        cells[(origin + offset) % track] = glyph;
    }
    cells.into_iter().collect()
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum IconSet {
    Nerd,
    #[default]
    Unicode,
    Ascii,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Record {
    Task,
    Repair,
    Agent,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowStatus {
    Working,
    /// Waiting for model capacity.
    Queued,
    Review,
    Decision,
    Failed,
    Blocked,
    Complete,
    /// Idle chat or agent.
    Idle,
    /// Proposed or approved, not started.
    Pending,
    Cancelled,
    /// Recorded as running without an observed local worker.
    Unverified,
}

impl IconSet {
    pub fn parse(text: &str) -> Result<Self, String> {
        match text.trim() {
            "nerd" => Ok(Self::Nerd),
            "unicode" => Ok(Self::Unicode),
            "ascii" => Ok(Self::Ascii),
            other => Err(format!(
                "Icons must be nerd, unicode or ascii, not {other:?}"
            )),
        }
    }

    /// Nerd Font codepoints (Nerd Fonts v3):
    /// - Task: nf-fa-ticket U+F145
    /// - Repair / delegation: nf-dev-git_branch U+E725
    /// - Agent session: nf-md-robot U+F06A9
    pub fn record(self, record: Record) -> &'static str {
        match (self, record) {
            (Self::Nerd, Record::Task) => "\u{f145}",
            (Self::Nerd, Record::Repair) => "\u{e725}",
            (Self::Nerd, Record::Agent) => "\u{f06a9}",
            (Self::Unicode, Record::Task) => "▤",
            (Self::Unicode, Record::Repair) => "⑂",
            (Self::Unicode, Record::Agent) => "◈",
            (Self::Ascii, Record::Task) => "#",
            (Self::Ascii, Record::Repair) => "Y",
            (Self::Ascii, Record::Agent) => "@",
        }
    }

    pub fn fold(self, expanded: bool) -> &'static str {
        match (self, expanded) {
            (Self::Ascii, true) => "v",
            (Self::Ascii, false) => ">",
            (_, true) => "▾",
            (_, false) => "▸",
        }
    }

    fn status(self, status: RowStatus) -> &'static str {
        let ascii = self == Self::Ascii;
        match status {
            RowStatus::Working | RowStatus::Unverified => {
                if ascii {
                    ">"
                } else {
                    "▶"
                }
            }
            RowStatus::Queued => pick(ascii, "~", "◌"),
            RowStatus::Review => pick(ascii, "?", "◐"),
            RowStatus::Decision => pick(ascii, "!", "●"),
            RowStatus::Failed => pick(ascii, "x", "✗"),
            RowStatus::Blocked => pick(ascii, "=", "‖"),
            RowStatus::Complete => pick(ascii, "+", "✓"),
            RowStatus::Idle | RowStatus::Pending => pick(ascii, "o", "○"),
            RowStatus::Cancelled => pick(ascii, "-", "–"),
        }
    }
}

fn pick(ascii: bool, plain: &'static str, fancy: &'static str) -> &'static str {
    if ascii {
        plain
    } else {
        fancy
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ColorMode {
    #[default]
    Ansi16,
    Truecolor,
    None,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Green,
    Amber,
    Red,
    Blue,
    Cyan,
    Dim,
    Lime,
    Magenta,
}

impl ColorMode {
    pub fn detect(colorterm: Option<&str>, no_color: Option<&str>) -> Self {
        if no_color.is_some_and(|value| !value.is_empty()) {
            Self::None
        } else if matches!(colorterm, Some("truecolor" | "24bit")) {
            Self::Truecolor
        } else {
            Self::Ansi16
        }
    }

    pub fn color(self, tone: Tone) -> Color {
        match self {
            Self::None => Color::Reset,
            Self::Truecolor => match tone {
                Tone::Green => Color::Rgb(0x75, 0xd9, 0x9b),
                Tone::Amber => Color::Rgb(0xe8, 0xb4, 0x58),
                Tone::Red => Color::Rgb(0xec, 0x76, 0x6f),
                Tone::Blue => Color::Rgb(0x7e, 0xb7, 0xff),
                Tone::Cyan => Color::Rgb(0x72, 0xd6, 0xcf),
                Tone::Dim => Color::Rgb(0x6e, 0x74, 0x6f),
                Tone::Lime => Color::Rgb(0xb6, 0xf3, 0x6b),
                Tone::Magenta => Color::Magenta,
            },
            Self::Ansi16 => match tone {
                Tone::Green => Color::Green,
                Tone::Amber => Color::Yellow,
                Tone::Red => Color::Red,
                Tone::Blue => Color::LightBlue,
                Tone::Cyan => Color::Cyan,
                Tone::Dim => Color::DarkGray,
                Tone::Lime => Color::LightGreen,
                Tone::Magenta => Color::Magenta,
            },
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Theme {
    pub icons: IconSet,
    pub motion: bool,
    pub color: ColorMode,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            icons: IconSet::Unicode,
            motion: true,
            color: ColorMode::Ansi16,
        }
    }
}

impl Theme {
    /// `ALFREDO_ICONS`, `ALFREDO_NO_MOTION=1`, `COLORTERM` and `NO_COLOR`.
    pub fn from_env(get: impl Fn(&str) -> Option<String>) -> Result<Self, String> {
        Ok(Self {
            icons: get("ALFREDO_ICONS")
                .map(|value| IconSet::parse(&value))
                .transpose()?
                .unwrap_or_default(),
            motion: get("ALFREDO_NO_MOTION").is_none_or(|value| value.trim() != "1"),
            color: ColorMode::detect(get("COLORTERM").as_deref(), get("NO_COLOR").as_deref()),
        })
    }

    pub fn status_glyph(&self, status: RowStatus, now: Instant) -> &'static str {
        if status == RowStatus::Working && self.motion {
            let tick = spinner_tick(now) as usize;
            return if self.icons == IconSet::Ascii {
                ASCII_SPINNER[tick % ASCII_SPINNER.len()]
            } else {
                BRAILLE[tick % BRAILLE.len()]
            };
        }
        self.icons.status(status)
    }

    /// The capacity-wait cat on the spinner clock; frame 1 without motion.
    pub fn capacity_cat(&self, now: Instant, width: usize) -> String {
        let tick = if self.motion { spinner_tick(now) } else { 0 };
        cat_track(self.icons, tick, width)
    }

    pub fn tone(status: RowStatus) -> Tone {
        match status {
            RowStatus::Working => Tone::Lime,
            RowStatus::Queued | RowStatus::Idle | RowStatus::Pending | RowStatus::Cancelled => {
                Tone::Dim
            }
            RowStatus::Review => Tone::Cyan,
            RowStatus::Decision | RowStatus::Unverified => Tone::Amber,
            RowStatus::Failed => Tone::Red,
            RowStatus::Blocked => Tone::Magenta,
            RowStatus::Complete => Tone::Green,
        }
    }

    pub fn color(&self, tone: Tone) -> Color {
        self.color.color(tone)
    }
}

fn epoch() -> Instant {
    static EPOCH: OnceLock<Instant> = OnceLock::new();
    *EPOCH.get_or_init(Instant::now)
}

/// 100 ms ticks on the monotonic clock since the first call in this process.
pub fn spinner_tick(now: Instant) -> u64 {
    (now.saturating_duration_since(epoch()).as_millis() / FRAME_MS) as u64
}

/// Decides when the spinner alone needs a redraw: once per frame while
/// something is working and motion is enabled, never otherwise.
#[derive(Debug, Default)]
pub struct SpinnerSchedule {
    last: Option<u64>,
}

impl SpinnerSchedule {
    pub fn due(&mut self, now: Instant, working: bool, motion: bool) -> bool {
        if !working || !motion {
            self.last = None;
            return false;
        }
        let tick = spinner_tick(now);
        if self.last == Some(tick) {
            return false;
        }
        self.last = Some(tick);
        true
    }
}

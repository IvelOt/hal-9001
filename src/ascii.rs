
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogoSize {

    Main,

    Medium,

    Compact,
}

const MAIN: &[&str] = &[
    "              =====              ",
    "        ==####=====####==        ",
    "      ====             ====      ",
    "    ##=     xxxx+++++     =##    ",
    "  ####   ++++x     ++xxx   ####  ",
    " ===   x+++...........x+++   === ",
    " ===  xxx ..ooooooooo.. +++  === ",
    "#==   ++ ..ooooOOOoooo.. ++   ==#",
    "###   ++ ..oooOOOOOooo.. xx   ###",
    "#==   ++ ..ooooOOOoooo.. ++   ==#",
    " ===  xxx ..ooooooooo.. +++  === ",
    " ===   x+++...........x+++   === ",
    "  ####   ++++x     ++xxx   ####  ",
    "    ##=     xxxx+++++     =##    ",
    "      ====             ====      ",
    "        ==####=====####==        ",
    "              =====              ",
];

const MEDIUM: &[&str] = &[
    "        #=====###==        ",
    "     =###         ====     ",
    "   ===    x+++++x    ###   ",
    "  ==   +xxx.....xxx+   ==  ",
    " ##   ++...ooooo...++   == ",
    " ==  ++..ooooOoooo..++  == ",
    "===  xx..ooOOOOOoo..xx  ###",
    " ==  ++..ooooOoooo..++  == ",
    " ##   ++...ooooo...++   == ",
    "  ==   +xxx.....xxx+   ==  ",
    "   ===    x+++++x    ###   ",
    "     =###         ====     ",
    "        #=====###==        ",
];

const COMPACT: &[&str] = &[
    "     ====   ====    ",
    "   ##           ##  ",
    "  =     .....     = ",
    " ==   ..ooooo..   ==",
    " #    .ooOOOoo.    #",
    " ==   ..ooooo..   ==",
    "  =     .....     = ",
    "   ##           ##  ",
    "     ====   ====    ",
];

impl LogoSize {

    fn art(self) -> &'static [&'static str] {
        match self {
            LogoSize::Main => MAIN,
            LogoSize::Medium => MEDIUM,
            LogoSize::Compact => COMPACT,
        }
    }

    pub fn width(self) -> u16 {
        self.art()
            .iter()
            .map(|l| UnicodeWidthStr::width(*l))
            .max()
            .unwrap_or(0) as u16
    }

    pub fn height(self) -> u16 {
        self.art().len() as u16
    }
}

const RING_TEETH: Color = Color::Rgb(180, 140, 60);
const RING_GAP: Color = Color::DarkGray;
const HUB_TEETH: Color = Color::Rgb(210, 170, 90);
const HUB_GAP: Color = Color::Yellow;

fn eye_colors(phase: u8) -> (Color, Color, Color) {
    match phase % 4 {

        0 => (Color::LightRed, Color::Red, Color::Rgb(255, 50, 50)),
        1 => (
            Color::Rgb(255, 130, 110),
            Color::Rgb(255, 80, 70),
            Color::Rgb(255, 80, 80),
        ),
        2 => (
            Color::Rgb(255, 165, 150),
            Color::Rgb(255, 110, 95),
            Color::Rgb(255, 110, 110),
        ),
        _ => (
            Color::Rgb(255, 130, 110),
            Color::Rgb(255, 80, 70),
            Color::Rgb(255, 80, 80),
        ),
    }
}

fn glyph_color(c: char, eye: (Color, Color, Color)) -> Option<Color> {
    match c {
        '#' => Some(RING_TEETH),
        '=' => Some(RING_GAP),
        'x' => Some(HUB_TEETH),
        '+' => Some(HUB_GAP),
        '.' => Some(eye.0),
        'o' => Some(eye.1),
        'O' => Some(eye.2),
        _ => None,
    }
}

pub fn logo_lines(size: LogoSize) -> Vec<Line<'static>> {
    logo_lines_phase(size, 0)
}

pub fn logo_lines_phase(size: LogoSize, phase: u8) -> Vec<Line<'static>> {
    let eye = eye_colors(phase);
    size.art()
        .iter()
        .map(|&raw| {
            let mut spans: Vec<Span<'static>> = Vec::new();
            let mut cur: Option<Color> = None;
            let mut start = 0;

            let flush = |spans: &mut Vec<Span<'static>>, start: usize, end: usize, cur: Option<Color>| {
                if start == end {
                    return;
                }
                let style = match cur {
                    Some(c) => Style::default().fg(c),
                    None => Style::default(),
                };
                spans.push(Span::styled(&raw[start..end], style));
            };

            for (i, c) in raw.char_indices() {
                let color = glyph_color(c, eye);
                if color != cur {
                    if i > start {
                        flush(&mut spans, start, i, cur);
                    }
                    cur = color;
                    start = i;
                }
            }
            flush(&mut spans, start, raw.len(), cur);
            Line::from(spans)
        })
        .collect()
}

pub fn select(pref: &str, logo_budget: u16) -> Option<LogoSize> {
    match pref {
        "main" | "A" => Some(LogoSize::Main),
        "medium" | "C" => Some(LogoSize::Medium),
        "compact" | "B" => Some(LogoSize::Compact),
        "none" | "off" => None,
        _ => auto(logo_budget),
    }
}

fn auto(logo_budget: u16) -> Option<LogoSize> {
    [LogoSize::Main, LogoSize::Medium, LogoSize::Compact]
        .into_iter()
        .find(|size| size.width() <= logo_budget)
}

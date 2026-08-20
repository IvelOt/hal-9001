//! Logo do HAL-9001 — anéis concêntricos de engrenagens com o Olho do HAL-9000
//! ao centro. Ver `docs/04_ascii_art_besouro.md`.
//!
//! A arte é **somente ASCII** (largura previsível com `unicode-width`) e todas
//! as linhas de uma mesma arte têm a **mesma largura** (preenchidas com
//! espaços), mantendo o olho central alinhado e a colorização radial simétrica.
//!
//! Colorização multi-span por glifo (ver [`logo_lines`]):
//! - Anel externo: dentes `#` em bronze, vales `=` em cinza escuro;
//! - Anel interno: dentes `x` em âmbar, vales `+` em amarelo (ouro);
//! - Olho do HAL: halo `.` em vermelho claro, íris `o` em vermelho, núcleo
//!   incandescente `O` em vermelho vivo.

use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

/// Tamanho da logo. Escolhido pela largura disponível e **fixado** enquanto o
/// Overview estiver aberto — o modo detalhado (`.`) não encolhe a logo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogoSize {
    /// Versão principal (~33 col × 17 linhas).
    Main,
    /// Versão média (~27 col × 13 linhas).
    Medium,
    /// Versão compacta (~20 col × 9 linhas).
    Compact,
}

/// Logo principal — dois anéis de engrenagens + olho incandescente.
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

/// Logo média — dois anéis mais compactos.
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

/// Logo compacta — um anel + olho.
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
    /// Linhas cruas (ASCII) da arte.
    fn art(self) -> &'static [&'static str] {
        match self {
            LogoSize::Main => MAIN,
            LogoSize::Medium => MEDIUM,
            LogoSize::Compact => COMPACT,
        }
    }

    /// Largura fixa (colunas) da coluna da logo.
    pub fn width(self) -> u16 {
        self.art()
            .iter()
            .map(|l| UnicodeWidthStr::width(*l))
            .max()
            .unwrap_or(0) as u16
    }

    /// Altura fixa (linhas) da logo.
    pub fn height(self) -> u16 {
        self.art().len() as u16
    }
}

// --- Paleta da logo (fixa: o olho do HAL é sempre vermelho) --------------

const RING_TEETH: Color = Color::Rgb(180, 140, 60); // bronze  ('#')
const RING_GAP: Color = Color::DarkGray; //             cinza   ('=')
const HUB_TEETH: Color = Color::Rgb(210, 170, 90); //   âmbar   ('x')
const HUB_GAP: Color = Color::Yellow; //                ouro    ('+')
const EYE_HALO: Color = Color::LightRed; //             halo    ('.')
const EYE_IRIS: Color = Color::Red; //                  íris    ('o')
const EYE_CORE: Color = Color::Rgb(255, 50, 50); //     núcleo  ('O')

/// Cor de um glifo da logo, ou `None` para espaço (sem span).
fn glyph_color(c: char) -> Option<Color> {
    match c {
        '#' => Some(RING_TEETH),
        '=' => Some(RING_GAP),
        'x' => Some(HUB_TEETH),
        '+' => Some(HUB_GAP),
        '.' => Some(EYE_HALO),
        'o' => Some(EYE_IRIS),
        'O' => Some(EYE_CORE),
        _ => None,
    }
}

/// Constrói as linhas coloridas (multi-span) da logo, agrupando glifos
/// consecutivos de mesma cor num único span.
pub fn logo_lines(size: LogoSize) -> Vec<Line<'static>> {
    size.art()
        .iter()
        .map(|raw| {
            let mut spans: Vec<Span<'static>> = Vec::new();
            let mut buf = String::new();
            let mut cur: Option<Color> = None;

            let flush = |spans: &mut Vec<Span<'static>>, buf: &mut String, cur: Option<Color>| {
                if buf.is_empty() {
                    return;
                }
                let style = match cur {
                    Some(c) => Style::default().fg(c),
                    None => Style::default(),
                };
                spans.push(Span::styled(std::mem::take(buf), style));
            };

            for c in raw.chars() {
                let color = glyph_color(c);
                if color != cur {
                    flush(&mut spans, &mut buf, cur);
                    cur = color;
                }
                buf.push(c);
            }
            flush(&mut spans, &mut buf, cur);
            Line::from(spans)
        })
        .collect()
}

/// Escolhe o tamanho da logo conforme a **largura reservada para a coluna da
/// logo** (`logo_budget`), respeitando a preferência da config. Retorna `None`
/// quando não cabe nem a versão compacta — o Overview então recolhe a logo e
/// centraliza apenas o painel de informações.
///
/// A escolha depende só da largura reservada (independente do modo detalhado),
/// garantindo que a logo permaneça estável ao alternar `.`.
pub fn select(pref: &str, logo_budget: u16) -> Option<LogoSize> {
    match pref {
        "main" | "A" => Some(LogoSize::Main),
        "medium" | "C" => Some(LogoSize::Medium),
        "compact" | "B" => Some(LogoSize::Compact),
        "none" | "off" => None,
        _ => auto(logo_budget),
    }
}

/// Maior logo cuja largura caiba no orçamento reservado.
fn auto(logo_budget: u16) -> Option<LogoSize> {
    [LogoSize::Main, LogoSize::Medium, LogoSize::Compact]
        .into_iter()
        .find(|size| size.width() <= logo_budget)
}

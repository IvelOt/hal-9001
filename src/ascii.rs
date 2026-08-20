//! ASCII arts do Besouro para o Overview. Ver `docs/04_ascii_art_besouro.md`.
//!
//! Somente ASCII para largura previsível com `unicode-width`.

/// Art A — "Scarab" principal (~24 col).
pub const SCARAB_A: &str = r#"        . ~ ~ ~ .
      /  \ | /  \
     |    \|/    |
      \   (o)   /
   .--- \ /-\ / ---.
  /      \| |/      \
 |    _.-'\ /'-._    |
 |  .'     V     '.  |
  \/   .-'''-.   \/
   |   /  _  \   |
   |  |  (_)  |  |
    \  \     /  /
     '. '._.' .'
       '-...-'
       /     \
      '       '"#;

/// Art B — "Beetle Compacto" (~16 col).
pub const BEETLE_B: &str = r#"     , _ ,
    ( o o )
   /'` ' `'\
   |'''''''|
   |\     /|
   ( \___/ )
    '.___.'
    /     \"#;

/// Art C — "Scarab Detalhado" (~30 col).
pub const SCARAB_C: &str = r#"          __/\__
         `==/\==`
     ___/  ||  \___
    /   \  ||  /   \
   | /\  \ || / /\ |
   | ||   \||/   || |
    \ \   (**)   / /
     \ '--/  \--' /
      '.  |  |  .'
    ____'.|  |.'____
   /     /    \     \
  '     |      |     '
        '.    .'
          '..'"#;

/// Seleciona a art conforme a preferência da config e a largura disponível
/// para a coluna do besouro.
///
/// Retorna `None` quando não há largura suficiente nem para a art compacta —
/// o Overview então recolhe o besouro e centraliza só o painel de informações
/// (telas muito estreitas). No modo `auto`, escolhe a maior art que couber.
pub fn select(pref: &str, avail_width: u16) -> Option<&'static str> {
    match pref {
        "A" => Some(SCARAB_A),
        "B" => Some(BEETLE_B),
        "C" => Some(SCARAB_C),
        "none" | "off" => None,
        _ => auto(avail_width),
    }
}

/// Escolhe automaticamente a maior art cuja largura caiba em `avail_width`.
fn auto(avail_width: u16) -> Option<&'static str> {
    if avail_width >= art_width(SCARAB_C) as u16 {
        Some(SCARAB_C)
    } else if avail_width >= art_width(SCARAB_A) as u16 {
        Some(SCARAB_A)
    } else if avail_width >= art_width(BEETLE_B) as u16 {
        Some(BEETLE_B)
    } else {
        None
    }
}

/// Largura (colunas) da linha mais larga de uma art.
pub fn art_width(art: &str) -> usize {
    art.lines()
        .map(unicode_width::UnicodeWidthStr::width)
        .max()
        .unwrap_or(0)
}

/// Altura (linhas) de uma art.
pub fn art_height(art: &str) -> usize {
    art.lines().count()
}

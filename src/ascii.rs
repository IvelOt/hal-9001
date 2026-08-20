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

/// Seleciona a art conforme a preferência da config e a largura disponível.
pub fn select(pref: &str, width: u16) -> &'static str {
    match pref {
        "A" => SCARAB_A,
        "B" => BEETLE_B,
        "C" => SCARAB_C,
        _ => {
            // auto: escolhe pela largura da coluna esquerda.
            if width >= 32 {
                SCARAB_C
            } else if width >= 24 {
                SCARAB_A
            } else {
                BEETLE_B
            }
        }
    }
}

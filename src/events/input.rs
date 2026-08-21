//! Ponte assíncrona entre o `EventStream` do crossterm e as [`Action`]s.

use crossterm::event::{Event, EventStream, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use futures::StreamExt;

use crate::app::Tab;

use super::Action;

/// Stream de input que traduz eventos de terminal em [`Action`]s.
pub struct InputStream {
    inner: EventStream,
}

impl InputStream {
    pub fn new() -> Self {
        Self {
            inner: EventStream::new(),
        }
    }

    /// Próxima ação, ou `None` quando o stream de terminal encerra. `active`
    /// desambigua teclas que mudam de significado conforme a aba (ex.: `m`
    /// alterna mudo global, mas monta/desmonta na aba Storage).
    pub async fn next(&mut self, active: Tab) -> Option<Action> {
        loop {
            match self.inner.next().await {
                Some(Ok(Event::Key(key))) if key.kind == KeyEventKind::Press => {
                    if let Some(action) = map_key(key, active) {
                        return Some(action);
                    }
                    // Tecla ignorada; continua aguardando.
                }
                Some(Ok(Event::Resize(_, _))) => return Some(Action::Redraw),
                Some(Ok(_)) => continue,
                Some(Err(_)) => continue,
                None => return None,
            }
        }
    }
}

impl Default for InputStream {
    fn default() -> Self {
        Self::new()
    }
}

/// Keymap global. Teclas não reconhecidas viram [`Action::Raw`] para eventual
/// repasse ao PTY da aba de terminal.
fn map_key(key: KeyEvent, active: Tab) -> Option<Action> {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

    // Aba Storage: `m`/`e`/`r` têm significado próprio (montar/ejetar/refresh
    // da árvore de discos), sobrepondo os atalhos globais de mudo/refresh.
    if active == Tab::Storage {
        match key.code {
            KeyCode::Char('m') => return Some(Action::StorageMountToggleSelected),
            KeyCode::Char('e') => return Some(Action::StorageEjectSelected),
            KeyCode::Char('r') => return Some(Action::StorageRefresh),
            _ => {}
        }
    }

    match key.code {
        KeyCode::Char('c') if ctrl => Some(Action::Quit),
        KeyCode::Char('q') => Some(Action::Quit),
        KeyCode::Tab => Some(Action::NextTab),
        KeyCode::BackTab => Some(Action::PrevTab),
        KeyCode::Char(c @ '1'..='8') => {
            let idx = (c as u8 - b'1') as usize;
            Some(Action::SelectTab(idx))
        }
        KeyCode::Down | KeyCode::Char('j') => Some(Action::Down),
        KeyCode::Up | KeyCode::Char('k') => Some(Action::Up),
        KeyCode::Left | KeyCode::Char('h') => Some(Action::Left),
        KeyCode::Right | KeyCode::Char('l') => Some(Action::Right),
        KeyCode::Enter => Some(Action::Enter),
        KeyCode::Char('r') => Some(Action::Refresh),
        KeyCode::Char('.') => Some(Action::ToggleDetail),
        // Configurações interativas: `c`/`C` ou F2.
        KeyCode::Char('c') | KeyCode::Char('C') | KeyCode::F(2) => Some(Action::ToggleConfig),
        KeyCode::Char('s') | KeyCode::Char('S') => Some(Action::SaveConfig),
        // Brilho: minúscula/`-` diminui, maiúscula/`+`/`=` aumenta.
        KeyCode::Char('b') | KeyCode::Char('-') => Some(Action::BrightnessDown),
        KeyCode::Char('B') | KeyCode::Char('+') | KeyCode::Char('=') => Some(Action::BrightnessUp),
        // Volume: minúscula/`[` diminui, maiúscula/`]` aumenta, `m` alterna mudo.
        KeyCode::Char('v') | KeyCode::Char('[') => Some(Action::VolumeDown),
        KeyCode::Char('V') | KeyCode::Char(']') => Some(Action::VolumeUp),
        KeyCode::Char('m') => Some(Action::ToggleMute),
        // Perfil de energia: `p`/`P` cicla Economia→Equilibrado→Desempenho.
        KeyCode::Char('p') | KeyCode::Char('P') => Some(Action::CyclePowerProfile),
        KeyCode::Char('?') => Some(Action::ToggleHelp),
        KeyCode::Esc => Some(Action::ToggleConfig),
        _ => Some(Action::Raw(key)),
    }
}

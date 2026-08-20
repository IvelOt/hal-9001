//! Ponte assíncrona entre o `EventStream` do crossterm e as [`Action`]s.

use crossterm::event::{Event, EventStream, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use futures::StreamExt;

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

    /// Próxima ação, ou `None` quando o stream de terminal encerra.
    pub async fn next(&mut self) -> Option<Action> {
        loop {
            match self.inner.next().await {
                Some(Ok(Event::Key(key))) if key.kind == KeyEventKind::Press => {
                    if let Some(action) = map_key(key) {
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
fn map_key(key: KeyEvent) -> Option<Action> {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
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
        KeyCode::Enter => Some(Action::Enter),
        KeyCode::Char('r') => Some(Action::Refresh),
        KeyCode::Char('?') => Some(Action::ToggleHelp),
        _ => Some(Action::Raw(key)),
    }
}

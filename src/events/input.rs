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
    ///
    /// `storage_modal_open` e `text_mode` desviam o teclado para os modais
    /// interativos de Storage (formatação/ISO Flasher): com um modal aberto,
    /// atalhos globais (`m`, `r`, dígitos de aba, etc.) ficam suspensos; em
    /// `text_mode` (campo de caminho de ISO, rótulo ou confirmação digitada),
    /// todo caractere vira `Action::StorageModalChar`.
    ///
    /// `sudo_prompt_open` tem prioridade máxima sobre tudo isso: enquanto o
    /// modal nativo de senha de sudo estiver aberto, todo o teclado (mesmo
    /// fora da aba Storage) vira digitação mascarada nesse campo.
    pub async fn next(
        &mut self,
        active: Tab,
        storage_modal_open: bool,
        text_mode: bool,
        sudo_prompt_open: bool,
    ) -> Option<Action> {
        loop {
            match self.inner.next().await {
                Some(Ok(Event::Key(key))) if key.kind == KeyEventKind::Press => {
                    if let Some(action) =
                        map_key(key, active, storage_modal_open, text_mode, sudo_prompt_open)
                    {
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
fn map_key(
    key: KeyEvent,
    active: Tab,
    storage_modal_open: bool,
    text_mode: bool,
    sudo_prompt_open: bool,
) -> Option<Action> {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

    // Ctrl+C sempre encerra, mesmo dentro de um campo de texto de modal.
    if ctrl && key.code == KeyCode::Char('c') {
        return Some(Action::Quit);
    }

    // Modal nativo de senha de sudo: prioridade máxima, funciona em qualquer
    // aba — todo caractere digitado vira `Action::StorageModalChar` (exibido
    // mascarado pela UI), `Enter` confirma, `Esc` cancela a operação.
    if sudo_prompt_open {
        return match key.code {
            KeyCode::Char(c) => Some(Action::StorageModalChar(c)),
            KeyCode::Backspace => Some(Action::StorageModalBackspace),
            KeyCode::Enter => Some(Action::Enter),
            KeyCode::Esc => Some(Action::ToggleConfig),
            _ => None,
        };
    }

    // Modal de senha de Wi-Fi: digitação mascarada da PSK.
    if active == Tab::Network && text_mode {
        return match key.code {
            KeyCode::Char(c) => Some(Action::NetworkModalChar(c)),
            KeyCode::Backspace => Some(Action::NetworkModalBackspace),
            KeyCode::Enter => Some(Action::Enter),
            KeyCode::Esc => Some(Action::ToggleConfig),
            _ => None,
        };
    }

    // Tecla dedicada (F3) que abre o seletor de arquivos estilo Yazi a partir
    // de qualquer modal de storage.
    if active == Tab::Storage && storage_modal_open && key.code == KeyCode::F(3) {
        return Some(Action::StorageModalOpenPicker);
    }

    // Campo de texto ativo num modal de Storage:
    if active == Tab::Storage && text_mode {
        return match key.code {
            KeyCode::Char(c) => Some(Action::StorageModalChar(c)),
            KeyCode::Backspace => Some(Action::StorageModalBackspace),
            KeyCode::Enter => Some(Action::Enter),
            KeyCode::Esc => Some(Action::ToggleConfig),
            KeyCode::Up => Some(Action::Up),
            KeyCode::Down => Some(Action::Down),
            KeyCode::Tab => Some(Action::NextTab),
            KeyCode::BackTab => Some(Action::PrevTab),
            _ => None,
        };
    }

    // Aba Network (Wi-Fi):
    if active == Tab::Network {
        match key.code {
            KeyCode::Char('r') => return Some(Action::NetworkRescan),
            KeyCode::Char('t') => return Some(Action::NetworkToggleRadio),
            KeyCode::Char('d') => return Some(Action::NetworkDisconnect(crate::events::DeviceId(String::new()))),
            KeyCode::Char('f') => return Some(Action::NetworkForget(String::new())),
            _ => {}
        }
    }

    // Aba Storage:
    if active == Tab::Storage {
        match key.code {
            KeyCode::Char('f') if !storage_modal_open => return Some(Action::StorageFormatOpen),
            KeyCode::Char('g') | KeyCode::Char('b') if !storage_modal_open => {
                return Some(Action::StorageFlasherOpen)
            }
            KeyCode::Char('B') if !storage_modal_open => {
                return Some(Action::StorageMultibootPrepareOpen)
            }
            KeyCode::Char('G') if !storage_modal_open => {
                return Some(Action::StorageMultibootIsoManagerOpen)
            }
            KeyCode::Delete if storage_modal_open => return Some(Action::StorageModalDelete),
            KeyCode::Char(c) if storage_modal_open => return Some(Action::StorageModalChar(c)),
            KeyCode::Char('m') if !storage_modal_open => {
                return Some(Action::StorageMountToggleSelected)
            }
            KeyCode::Char('e') if !storage_modal_open => return Some(Action::StorageEjectSelected),
            KeyCode::Char('r') if !storage_modal_open => return Some(Action::StorageRefresh),
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

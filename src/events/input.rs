//! Ponte assíncrona entre o `EventStream` do crossterm e as [`Action`]s.

use crossterm::event::{Event, EventStream, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use futures::StreamExt;

use crate::app::Tab;

use super::{Action, PtyTarget};

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
    ///
    /// `pty_focused` tem a mesma prioridade máxima quando ativo: com o
    /// teclado capturado pela sessão PTY da aba Arquivos/Terminal, toda
    /// tecla vira `Action::PtyInput` (bytes VT100/xterm), exceto o leader de
    /// escape (`Ctrl-a`/`Esc`) e as teclas de função `F1..F8`, que devolvem o
    /// foco ao chrome da TUI.
    #[allow(clippy::too_many_arguments)]
    pub async fn next(
        &mut self,
        active: Tab,
        storage_modal_open: bool,
        text_mode: bool,
        sudo_prompt_open: bool,
        pty_focused: bool,
    ) -> Option<Action> {
        loop {
            match self.inner.next().await {
                Some(Ok(Event::Key(key))) if key.kind == KeyEventKind::Press => {
                    if let Some(action) = map_key(
                        key,
                        active,
                        storage_modal_open,
                        text_mode,
                        sudo_prompt_open,
                        pty_focused,
                    ) {
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
    pty_focused: bool,
) -> Option<Action> {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

    // Ctrl+C sempre encerra, mesmo dentro de um campo de texto de modal.
    if ctrl && key.code == KeyCode::Char('c') {
        return Some(Action::Quit);
    }

    // PTY em foco (abas Arquivos/Terminal, Módulos 7/8): prioridade máxima
    // (mesmo nível do modal de sudo). `Ctrl-a`/`Esc` e `F1..F8` devolvem o
    // foco ao chrome; qualquer outra tecla vira input bruto para o PTY.
    if pty_focused {
        return match key.code {
            KeyCode::F(n @ 1..=8) => Some(Action::SelectTab((n - 1) as usize)),
            KeyCode::Esc => Some(Action::PtyUnfocus),
            KeyCode::Char('a') if ctrl => Some(Action::PtyUnfocus),
            _ => Some(Action::PtyInput {
                target: pty_target_for(active),
                bytes: key_to_pty_bytes(key),
            }),
        };
    }

    // Tecla `Enter` nas abas Arquivos/Terminal (sem foco de PTY ainda): pede
    // para o `App` dar foco ao PTY, se a sessão já estiver rodando.
    if (active == Tab::Files || active == Tab::Terminal) && key.code == KeyCode::Enter {
        return Some(Action::PtyFocus);
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

    // Aba Bluetooth:
    if active == Tab::Bluetooth {
        match key.code {
            KeyCode::Char('r') => return Some(Action::BluetoothRescan),
            KeyCode::Char('t') => return Some(Action::BluetoothToggleRadio),
            KeyCode::Char('p') => return Some(Action::BluetoothPair(crate::events::DeviceId(String::new()))),
            KeyCode::Char('f') => return Some(Action::BluetoothForget(crate::events::DeviceId(String::new()))),
            KeyCode::Char('b') => return Some(Action::BluetoothToggleBlock(crate::events::DeviceId(String::new()))),
            _ => {}
        }
    }

    // Aba Audio (Mixer):
    if active == Tab::Audio {
        match key.code {
            KeyCode::Char('+') | KeyCode::Char('=') | KeyCode::Right | KeyCode::Char('l') => {
                return Some(Action::VolumeUp);
            }
            KeyCode::Char('-') | KeyCode::Left | KeyCode::Char('h') => {
                return Some(Action::VolumeDown);
            }
            KeyCode::Char('m') => return Some(Action::ToggleMute),
            KeyCode::Tab => return Some(Action::AudioSelectCategory(99)),
            KeyCode::Char('1') => return Some(Action::AudioSelectCategory(0)),
            KeyCode::Char('2') => return Some(Action::AudioSelectCategory(1)),
            KeyCode::Char('3') => return Some(Action::AudioSelectCategory(2)),
            _ => {}
        }
    }

    // Aba Displays (Telas & Monitores):
    if active == Tab::Displays {
        match key.code {
            KeyCode::Char('1') => {
                return Some(Action::DisplaySetLayout(crate::backend::display::DisplayLayoutMode::ExtendRight));
            }
            KeyCode::Char('2') => {
                return Some(Action::DisplaySetLayout(crate::backend::display::DisplayLayoutMode::ExtendLeft));
            }
            KeyCode::Char('3') => {
                return Some(Action::DisplaySetLayout(crate::backend::display::DisplayLayoutMode::Mirror));
            }
            KeyCode::Char('4') => {
                return Some(Action::DisplaySetLayout(crate::backend::display::DisplayLayoutMode::ExternalOnly));
            }
            KeyCode::Char('5') => {
                return Some(Action::DisplaySetLayout(crate::backend::display::DisplayLayoutMode::InternalOnly));
            }
            KeyCode::Char('p') => {
                return Some(Action::DisplaySetPrimary(String::new()));
            }
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

/// Sessão PTY endereçada pela aba ativa (só chamado quando `active` é
/// `Files` ou `Terminal`, os únicos casos em que `pty_focused` pode ser
/// `true`; qualquer outra aba cai no `_ => PtyTarget::Terminal` neutro, que
/// nunca é de fato despachado por não haver como focar o PTY fora dessas
/// duas abas).
fn pty_target_for(active: Tab) -> PtyTarget {
    match active {
        Tab::Files => PtyTarget::Files,
        _ => PtyTarget::Terminal,
    }
}

/// Codifica uma tecla em bytes VT100/xterm para escrever na sessão PTY em
/// foco — mesma convenção usada por emuladores de terminal reais (setas,
/// Home/End/PageUp/PageDown/Delete como sequências CSI; `Ctrl+letra` como o
/// byte de controle `0x01..=0x1A`; `Enter`/`Backspace`/`Tab` nos bytes
/// clássicos). `Esc` e o leader `Ctrl-a` nunca chegam aqui — já são
/// interceptados antes, em `map_key`.
pub fn key_to_pty_bytes(key: KeyEvent) -> Vec<u8> {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Char(c) => {
            if ctrl {
                let upper = c.to_ascii_uppercase();
                if upper.is_ascii_uppercase() {
                    return vec![(upper as u8) - b'A' + 1];
                }
            }
            let mut buf = [0u8; 4];
            c.encode_utf8(&mut buf).as_bytes().to_vec()
        }
        KeyCode::Enter => vec![b'\r'],
        KeyCode::Backspace => vec![0x7f],
        KeyCode::Tab => vec![b'\t'],
        KeyCode::BackTab => b"\x1b[Z".to_vec(),
        KeyCode::Up => b"\x1b[A".to_vec(),
        KeyCode::Down => b"\x1b[B".to_vec(),
        KeyCode::Right => b"\x1b[C".to_vec(),
        KeyCode::Left => b"\x1b[D".to_vec(),
        KeyCode::Home => b"\x1b[H".to_vec(),
        KeyCode::End => b"\x1b[F".to_vec(),
        KeyCode::PageUp => b"\x1b[5~".to_vec(),
        KeyCode::PageDown => b"\x1b[6~".to_vec(),
        KeyCode::Insert => b"\x1b[2~".to_vec(),
        KeyCode::Delete => b"\x1b[3~".to_vec(),
        _ => Vec::new(),
    }
}

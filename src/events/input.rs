use crossterm::event::{Event, EventStream, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use futures::StreamExt;

use crate::app::Tab;

use super::Action;

pub struct InputStream {
    inner: EventStream,
}

impl InputStream {
    pub fn new() -> Self {
        Self {
            inner: EventStream::new(),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn next(
        &mut self,
        active: Tab,
        storage_modal_open: bool,
        text_mode: bool,
        sudo_prompt_open: bool,
        storage_analyzer_open: bool,
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
                        storage_analyzer_open,
                    ) {
                        return Some(action);
                    }
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

fn map_key(
    key: KeyEvent,
    active: Tab,
    storage_modal_open: bool,
    text_mode: bool,
    sudo_prompt_open: bool,
    storage_analyzer_open: bool,
) -> Option<Action> {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

    if ctrl && key.code == KeyCode::Char('c') {
        return Some(Action::Quit);
    }

    if active == Tab::Storage && storage_analyzer_open {
        return match key.code {
            KeyCode::Up | KeyCode::Char('k') => Some(Action::Up),
            KeyCode::Down | KeyCode::Char('j') => Some(Action::Down),
            KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => {
                Some(Action::StorageAnalyzerDrillDown)
            }
            KeyCode::Backspace | KeyCode::Left | KeyCode::Char('h') => {
                Some(Action::StorageAnalyzerGoUp)
            }
            KeyCode::Char('r') => Some(Action::StorageAnalyzerRescan),
            KeyCode::Esc => Some(Action::StorageAnalyzerClose),
            _ => None,
        };
    }

    if sudo_prompt_open {
        return match key.code {
            KeyCode::Char(c) => Some(Action::StorageModalChar(c)),
            KeyCode::Backspace => Some(Action::StorageModalBackspace),
            KeyCode::Enter => Some(Action::Enter),
            KeyCode::Esc => Some(Action::ToggleConfig),
            _ => None,
        };
    }

    if active == Tab::Network && text_mode {
        return match key.code {
            KeyCode::Char(c) => Some(Action::NetworkModalChar(c)),
            KeyCode::Backspace => Some(Action::NetworkModalBackspace),
            KeyCode::Enter => Some(Action::Enter),
            KeyCode::Esc => Some(Action::ToggleConfig),
            _ => None,
        };
    }

    if active == Tab::Storage && storage_modal_open && key.code == KeyCode::F(3) {
        return Some(Action::StorageModalOpenPicker);
    }

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

    if active == Tab::Network {
        match key.code {
            KeyCode::Char('r') => return Some(Action::NetworkRescan),
            KeyCode::Char('t') => return Some(Action::NetworkToggleRadio),
            KeyCode::Char('d') => {
                return Some(Action::NetworkDisconnect(crate::events::DeviceId(
                    String::new(),
                )))
            }
            KeyCode::Char('f') => return Some(Action::NetworkForget(String::new())),
            _ => {}
        }
    }

    if active == Tab::Overview {
        if key.code == KeyCode::Char('k') || key.code == KeyCode::Char('K') {
            return Some(Action::KillTopProcess);
        }
        if key.code == KeyCode::Char('A') {
            return Some(Action::ToggleAirplaneMode);
        }
    }

    if active == Tab::Bluetooth {
        match key.code {
            KeyCode::Char('r') => return Some(Action::BluetoothRescan),
            KeyCode::Char('t') => return Some(Action::BluetoothToggleRadio),
            KeyCode::Char('p') => {
                return Some(Action::BluetoothPair(
                    crate::events::DeviceId(String::new()),
                ))
            }
            KeyCode::Char('f') => {
                return Some(Action::BluetoothForget(crate::events::DeviceId(
                    String::new(),
                )))
            }
            KeyCode::Char('b') => {
                return Some(Action::BluetoothToggleBlock(crate::events::DeviceId(
                    String::new(),
                )))
            }
            _ => {}
        }
    }

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
            KeyCode::BackTab => return Some(Action::AudioSelectCategory(98)),
            _ => {}
        }
    }

    if active == Tab::Displays {
        match key.code {
            KeyCode::Char('e') => {
                return Some(Action::DisplaySetLayout(
                    crate::backend::display::DisplayLayoutMode::ExtendRight,
                ));
            }
            KeyCode::Char('E') => {
                return Some(Action::DisplaySetLayout(
                    crate::backend::display::DisplayLayoutMode::ExtendLeft,
                ));
            }
            KeyCode::Char('m') => {
                return Some(Action::DisplaySetLayout(
                    crate::backend::display::DisplayLayoutMode::Mirror,
                ));
            }
            KeyCode::Char('x') => {
                return Some(Action::DisplaySetLayout(
                    crate::backend::display::DisplayLayoutMode::ExternalOnly,
                ));
            }
            KeyCode::Char('i') => {
                return Some(Action::DisplaySetLayout(
                    crate::backend::display::DisplayLayoutMode::InternalOnly,
                ));
            }
            KeyCode::Char('p') | KeyCode::Char('P') => {
                return Some(Action::DisplaySetPrimary(String::new()));
            }
            _ => {}
        }
    }

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
            KeyCode::Char('a') if !storage_modal_open => {
                return Some(Action::StorageOpenAnalyzer(None))
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
        KeyCode::Char(c @ '1'..='6') => {
            let idx = (c as u8 - b'1') as usize;
            Some(Action::SelectTab(idx))
        }
        KeyCode::F(n @ 1..=6) => Some(Action::SelectTab((n - 1) as usize)),
        KeyCode::Down | KeyCode::Char('j') => Some(Action::Down),
        KeyCode::Up | KeyCode::Char('k') => Some(Action::Up),
        KeyCode::Left | KeyCode::Char('h') => Some(Action::Left),
        KeyCode::Right | KeyCode::Char('l') => Some(Action::Right),
        KeyCode::Enter => Some(Action::Enter),
        KeyCode::Char('r') => Some(Action::Refresh),
        KeyCode::Char('U') => Some(Action::CheckUpdates),
        KeyCode::Char('.') => Some(Action::ToggleDetail),

        KeyCode::Char('c') | KeyCode::Char('C') => Some(Action::ToggleConfig),
        KeyCode::Char('s') | KeyCode::Char('S') => Some(Action::SaveConfig),

        KeyCode::Char('b') | KeyCode::Char('-') => Some(Action::BrightnessDown),
        KeyCode::Char('B') | KeyCode::Char('+') | KeyCode::Char('=') => Some(Action::BrightnessUp),

        KeyCode::Char('v') | KeyCode::Char('[') => Some(Action::VolumeDown),
        KeyCode::Char('V') | KeyCode::Char(']') => Some(Action::VolumeUp),
        KeyCode::Char('m') => Some(Action::ToggleMute),

        KeyCode::Char('p') | KeyCode::Char('P') => Some(Action::CyclePowerProfile),
        KeyCode::Char('?') => Some(Action::ToggleHelp),
        KeyCode::Esc => Some(Action::ToggleConfig),
        _ => None,
    }
}

use std::sync::{Arc, Mutex};

enum ClipboardState {
    Uninitialized,
    Available(arboard::Clipboard),
    Unavailable,
}

#[derive(Clone)]
pub struct ClipboardService {
    inner: Arc<Mutex<ClipboardState>>,
}

impl std::fmt::Debug for ClipboardService {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let status = self
            .inner
            .lock()
            .map_or("poisoned", |clipboard| match &*clipboard {
                ClipboardState::Uninitialized => "uninitialized",
                ClipboardState::Available(_) => "available",
                ClipboardState::Unavailable => "unavailable",
            });
        formatter
            .debug_struct("ClipboardService")
            .field("status", &status)
            .finish()
    }
}

impl ClipboardService {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(ClipboardState::Uninitialized)),
        }
    }

    #[cfg(test)]
    pub fn unavailable() -> Self {
        Self {
            inner: Arc::new(Mutex::new(ClipboardState::Unavailable)),
        }
    }

    pub fn copy_text(&self, text: String) -> Result<(), String> {
        let mut clipboard = self
            .inner
            .lock()
            .map_err(|_| "clipboard access is unavailable".to_string())?;
        Self::initialize(&mut clipboard)?
            .set_text(text)
            .map_err(|_| "the operating system rejected the clipboard write".to_string())
    }

    pub fn read_text(&self) -> Result<String, String> {
        let mut clipboard = self
            .inner
            .lock()
            .map_err(|_| "clipboard access is unavailable".to_string())?;
        Self::initialize(&mut clipboard)?
            .get_text()
            .map_err(|_| "the clipboard does not contain readable text".to_string())
    }

    fn initialize(state: &mut ClipboardState) -> Result<&mut arboard::Clipboard, String> {
        if matches!(state, ClipboardState::Uninitialized) {
            *state = arboard::Clipboard::new()
                .map_or(ClipboardState::Unavailable, ClipboardState::Available);
        }
        match state {
            ClipboardState::Available(clipboard) => Ok(clipboard),
            ClipboardState::Uninitialized | ClipboardState::Unavailable => {
                Err("clipboard access is unavailable in this environment".to_string())
            }
        }
    }
}

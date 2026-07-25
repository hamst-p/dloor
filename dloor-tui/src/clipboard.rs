use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub struct ClipboardService {
    inner: Arc<Mutex<Option<arboard::Clipboard>>>,
}

impl std::fmt::Debug for ClipboardService {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ClipboardService")
            .field("available", &self.is_available())
            .finish()
    }
}

impl ClipboardService {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(arboard::Clipboard::new().ok())),
        }
    }

    #[cfg(test)]
    pub fn unavailable() -> Self {
        Self {
            inner: Arc::new(Mutex::new(None)),
        }
    }

    pub fn is_available(&self) -> bool {
        self.inner.lock().is_ok_and(|clipboard| clipboard.is_some())
    }

    pub fn copy_text(&self, text: String) -> Result<(), String> {
        let mut clipboard = self
            .inner
            .lock()
            .map_err(|_| "clipboard access is unavailable".to_string())?;
        clipboard
            .as_mut()
            .ok_or_else(|| "clipboard access is unavailable in this environment".to_string())?
            .set_text(text)
            .map_err(|_| "the operating system rejected the clipboard write".to_string())
    }
}

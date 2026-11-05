//! Terminal setup and restore. The terminal is never left broken: the
//! [`Guard`] restores it on `Drop`, the panic hook restores it before the
//! panic message is printed, and [`restore`] may run any number of times.

use std::io::{self, Write};
use std::sync::Once;
use std::sync::atomic::{AtomicBool, Ordering};

use crossterm::cursor::Show;
use crossterm::event::{
    DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
    KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
    supports_keyboard_enhancement,
};

use crate::error::{Error, Result};

/// Something was set up that [`restore`] has to undo.
static ACTIVE: AtomicBool = AtomicBool::new(false);
static MOUSE: AtomicBool = AtomicBool::new(false);
static KEYBOARD: AtomicBool = AtomicBool::new(false);

/// Installs the hook once: restore first, so that the message of the
/// previous hook lands on the main screen.
pub fn install_panic_hook() {
    static HOOK: Once = Once::new();
    HOOK.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            restore();
            previous(info);
        }));
    });
}

/// The terminal in TUI mode; dropping it restores the terminal.
#[derive(Debug)]
pub struct Guard;

impl Guard {
    /// Raw mode, alternate screen, bracketed paste, mouse capture if `mouse`,
    /// and keyboard enhancement where the terminal has it (for `Shift-Enter`).
    /// A failure restores what was already set up.
    pub fn new(mouse: bool) -> Result<Guard> {
        // From here on the guard restores, also if a step below fails.
        ACTIVE.store(true, Ordering::SeqCst);
        let guard = Guard;
        let context = Error::io("setting up the terminal");
        enable_raw_mode().map_err(&context)?;
        let mut out = io::stdout();
        execute!(out, EnterAlternateScreen, EnableBracketedPaste).map_err(&context)?;
        if mouse {
            MOUSE.store(true, Ordering::SeqCst);
            execute!(out, EnableMouseCapture).map_err(&context)?;
        }
        if matches!(supports_keyboard_enhancement(), Ok(true)) {
            KEYBOARD.store(true, Ordering::SeqCst);
            execute!(
                out,
                PushKeyboardEnhancementFlags(
                    KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
                        | KeyboardEnhancementFlags::REPORT_ALTERNATE_KEYS
                )
            )
            .map_err(&context)?;
        }
        Ok(guard)
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        restore();
    }
}

/// Undoes the setup. Harmless when nothing is set up. Errors are ignored:
/// each step should still get its chance, and there is nowhere to report to.
pub fn restore() {
    if !ACTIVE.swap(false, Ordering::SeqCst) {
        return;
    }
    let mut out = io::stdout();
    if KEYBOARD.swap(false, Ordering::SeqCst) {
        let _ = execute!(out, PopKeyboardEnhancementFlags);
    }
    if MOUSE.swap(false, Ordering::SeqCst) {
        let _ = execute!(out, DisableMouseCapture);
    }
    let _ = execute!(out, DisableBracketedPaste, LeaveAlternateScreen, Show);
    let _ = disable_raw_mode();
    let _ = out.flush();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restore_twice_is_harmless() {
        restore();
        restore();
        assert!(!ACTIVE.load(Ordering::SeqCst));
        // A guard that was set up only on paper: restored once, by `Drop`.
        ACTIVE.store(true, Ordering::SeqCst);
        drop(Guard);
        assert!(!ACTIVE.load(Ordering::SeqCst));
        restore();
    }
}

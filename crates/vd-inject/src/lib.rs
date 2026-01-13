//! # vd-inject
//!
//! Text injection for Voice-Dict using enigo.
//!
//! This crate provides cross-platform text injection into the active window.
//! It uses enigo to simulate keyboard input, making the transcribed text
//! appear as if typed by the user.
//!
//! ## Key Features
//!
//! - Cross-platform support (Linux, macOS, Windows)
//! - Fast character-by-character injection
//! - Unicode character support
//! - Configurable typing delay
//!
//! ## Example
//!
//! ```ignore
//! use vd_inject::TextInjector;
//!
//! let mut injector = TextInjector::new()?;
//! injector.inject("Hello, World!")?;
//! ```

use cli_clipboard::{ClipboardContext, ClipboardProvider};
use enigo::{Direction, Enigo, Key, Keyboard, Settings};
use std::thread;
use std::time::Duration;
use tracing::{debug, info, trace, warn};
use vd_core::InjectionError;

/// Default delay between characters in milliseconds
const DEFAULT_CHAR_DELAY_MS: u64 = 0;

/// Configuration for text injection
#[derive(Debug, Clone)]
pub struct InjectorConfig {
    /// Delay between characters in milliseconds (0 for fastest)
    pub char_delay_ms: u64,
    /// Whether to clear any existing selection before injecting
    pub clear_selection: bool,
    /// Use clipboard-based injection (Ctrl+V) instead of direct typing
    /// This is more reliable on Windows with focus issues
    pub use_clipboard: bool,
}

impl Default for InjectorConfig {
    fn default() -> Self {
        Self {
            char_delay_ms: DEFAULT_CHAR_DELAY_MS,
            clear_selection: false,
            use_clipboard: true, // Default to clipboard on Windows for reliability
        }
    }
}

/// Text injector
///
/// Injects text into the active window by simulating keyboard input.
pub struct TextInjector {
    enigo: Enigo,
    config: InjectorConfig,
}

impl TextInjector {
    /// Create a new text injector with default configuration
    pub fn new() -> Result<Self, InjectionError> {
        Self::with_config(InjectorConfig::default())
    }

    /// Create a new text injector with custom configuration
    pub fn with_config(config: InjectorConfig) -> Result<Self, InjectionError> {
        let settings = Settings::default();
        let enigo = Enigo::new(&settings)
            .map_err(|e| InjectionError::InitError(e.to_string()))?;

        info!("Text injector initialized");

        Ok(Self { enigo, config })
    }

    /// Inject text into the active window
    ///
    /// The text will appear at the current cursor position in the focused
    /// application, as if typed by the user.
    pub fn inject(&mut self, text: &str) -> Result<(), InjectionError> {
        if text.is_empty() {
            debug!("Empty text, nothing to inject");
            return Ok(());
        }

        debug!("Injecting {} characters (clipboard={})", text.len(), self.config.use_clipboard);

        if self.config.use_clipboard {
            // Use clipboard-based injection for better Windows compatibility
            self.inject_via_clipboard(text)?;
        } else {
            // Use enigo's text method for direct text injection
            self.enigo
                .text(text)
                .map_err(|e| InjectionError::InjectionFailed(e.to_string()))?;

            // Add delay if configured
            if self.config.char_delay_ms > 0 {
                thread::sleep(Duration::from_millis(
                    self.config.char_delay_ms * text.len() as u64,
                ));
            }
        }

        trace!("Text injected successfully");
        Ok(())
    }

    /// Inject text via clipboard (copy + Ctrl+V)
    ///
    /// This method is more reliable on Windows as it doesn't depend on
    /// the enigo text method which can have issues with window focus.
    fn inject_via_clipboard(&mut self, text: &str) -> Result<(), InjectionError> {
        // Copy text to clipboard
        let mut ctx: ClipboardContext = ClipboardProvider::new()
            .map_err(|e| InjectionError::InjectionFailed(format!("Clipboard init failed: {}", e)))?;

        ctx.set_contents(text.to_owned())
            .map_err(|e| InjectionError::InjectionFailed(format!("Clipboard set failed: {}", e)))?;

        debug!("Text copied to clipboard, sending Ctrl+V");

        // Minimal delay to ensure clipboard is ready
        thread::sleep(Duration::from_millis(10));

        // Send Ctrl+V to paste
        self.enigo
            .key(Key::Control, Direction::Press)
            .map_err(|e| InjectionError::InjectionFailed(e.to_string()))?;

        self.enigo
            .key(Key::Unicode('v'), Direction::Click)
            .map_err(|e| InjectionError::InjectionFailed(e.to_string()))?;

        self.enigo
            .key(Key::Control, Direction::Release)
            .map_err(|e| InjectionError::InjectionFailed(e.to_string()))?;

        // Minimal delay after paste
        thread::sleep(Duration::from_millis(20));

        info!("Text pasted via Ctrl+V: {} chars", text.len());
        Ok(())
    }

    /// Inject text character by character with delay
    ///
    /// This is slower but more reliable for some applications that
    /// don't handle rapid input well.
    pub fn inject_slow(&mut self, text: &str, char_delay_ms: u64) -> Result<(), InjectionError> {
        if text.is_empty() {
            return Ok(());
        }

        debug!("Slow-injecting {} characters with {}ms delay", text.len(), char_delay_ms);

        for c in text.chars() {
            self.enigo
                .text(&c.to_string())
                .map_err(|e| InjectionError::InjectionFailed(e.to_string()))?;

            if char_delay_ms > 0 {
                thread::sleep(Duration::from_millis(char_delay_ms));
            }
        }

        Ok(())
    }

    /// Get the current configuration
    pub fn config(&self) -> &InjectorConfig {
        &self.config
    }

    /// Update the configuration
    pub fn set_config(&mut self, config: InjectorConfig) {
        self.config = config;
    }
}

/// Inject text using a default injector
///
/// Convenience function for one-off text injection.
pub fn inject_text(text: &str) -> Result<(), InjectionError> {
    let mut injector = TextInjector::new()?;
    injector.inject(text)
}

/// Check if text injection is available on this platform
pub fn is_available() -> bool {
    #[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
    {
        true
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        warn!("Text injection not available on this platform");
        false
    }
}

/// Platform-specific notes
pub fn platform_notes() -> &'static str {
    #[cfg(target_os = "linux")]
    {
        "Linux: Requires X11 or compatible Wayland compositor with XWayland. \
         May need to run with appropriate permissions."
    }
    #[cfg(target_os = "macos")]
    {
        "macOS: Requires Accessibility permissions in System Preferences > \
         Security & Privacy > Privacy > Accessibility."
    }
    #[cfg(target_os = "windows")]
    {
        "Windows: May require running with appropriate permissions for \
         some applications. UAC elevation may be needed for admin apps."
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        "This platform is not supported for text injection."
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_injector_config_default() {
        let config = InjectorConfig::default();
        assert_eq!(config.char_delay_ms, 0);
        assert!(!config.clear_selection);
        assert!(config.use_clipboard); // Default to clipboard on Windows
    }

    #[test]
    fn test_is_available() {
        // Should be true on supported platforms
        #[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
        {
            assert!(is_available());
        }
    }

    #[test]
    fn test_platform_notes() {
        let notes = platform_notes();
        assert!(!notes.is_empty());
    }

    // Note: Actual injection tests require a display environment
    // and would inject text into the active window, so they're
    // not included in automated tests.
}

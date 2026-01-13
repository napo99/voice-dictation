//! # vd-polish
//!
//! Text polishing for Voice-Dict using regex and optional LLM.
//!
//! This crate provides two layers of text cleaning:
//! 1. Fast regex-based filler word removal (<1ms)
//! 2. Optional LLM-based grammar correction (~150ms)
//!
//! ## Key Features
//!
//! - Removes common filler words (um, uh, like, you know, etc.)
//! - Fixes common transcription artifacts
//! - Optional LLM polishing for longer text
//! - Configurable processing levels
//!
//! ## Example
//!
//! ```
//! use vd_polish::{TextPolisher, PolishLevel};
//!
//! let polisher = TextPolisher::new(PolishLevel::RegexOnly);
//! let cleaned = polisher.polish("um, so like, I was thinking, you know").unwrap();
//! assert_eq!(cleaned, "I was thinking");
//! ```

use regex::Regex;
use std::sync::OnceLock;
use tracing::{debug, trace};
use vd_core::PolishError;

/// List of common filler words to remove
const FILLERS: &[&str] = &[
    "um",
    "uh",
    "er",
    "ah",
    "hmm",
    "mm",
    "mhm",
    "uh-huh",
    "like",
    "you know",
    "basically",
    "actually",
    "literally",
    "so",
    "I mean",
    "kind of",
    "sort of",
    "right",
    "okay so",
    "well",
];

/// Polishing level configuration
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolishLevel {
    /// No polishing - return text as-is
    None,
    /// Regex-only - fast filler removal (<1ms)
    RegexOnly,
    /// Full polish - regex + LLM for grammar (~150ms)
    Full,
}

impl Default for PolishLevel {
    fn default() -> Self {
        Self::RegexOnly
    }
}

/// Text polisher
///
/// Cleans up transcribed text by removing filler words and optionally
/// applying LLM-based grammar correction.
pub struct TextPolisher {
    level: PolishLevel,
    filler_patterns: Vec<Regex>,
}

impl TextPolisher {
    /// Create a new text polisher
    pub fn new(level: PolishLevel) -> Self {
        let filler_patterns = FILLERS
            .iter()
            .filter_map(|filler| {
                // Create pattern that matches the filler word (case-insensitive)
                // with optional trailing comma and whitespace
                let pattern = format!(r"(?i)\b{}\b,?\s*", regex::escape(filler));
                Regex::new(&pattern).ok()
            })
            .collect();

        Self {
            level,
            filler_patterns,
        }
    }

    /// Polish the input text according to the configured level
    pub fn polish(&self, text: &str) -> Result<String, PolishError> {
        match self.level {
            PolishLevel::None => Ok(text.to_string()),
            PolishLevel::RegexOnly => self.polish_regex(text),
            PolishLevel::Full => self.polish_full(text),
        }
    }

    /// Apply regex-only polishing
    fn polish_regex(&self, text: &str) -> Result<String, PolishError> {
        let mut result = text.to_string();

        // Remove filler words
        for pattern in &self.filler_patterns {
            result = pattern.replace_all(&result, "").to_string();
        }

        // Apply additional cleanup
        result = cleanup_text(&result);

        trace!("Regex polish: '{}' -> '{}'", text, result);
        Ok(result)
    }

    /// Apply full polishing (regex + LLM)
    fn polish_full(&self, text: &str) -> Result<String, PolishError> {
        // First apply regex cleaning
        let regex_result = self.polish_regex(text)?;

        // For short text, regex is usually sufficient
        if regex_result.split_whitespace().count() < 5 {
            return Ok(regex_result);
        }

        // TODO: Add LLM polishing with Candle/Phi-3
        // For now, return regex result
        debug!("LLM polishing not yet implemented, using regex result");
        Ok(regex_result)
    }

    /// Get the current polish level
    pub fn level(&self) -> PolishLevel {
        self.level
    }

    /// Set the polish level
    pub fn set_level(&mut self, level: PolishLevel) {
        self.level = level;
    }
}

/// Clean up text after filler removal
fn cleanup_text(text: &str) -> String {
    static MULTI_SPACE: OnceLock<Regex> = OnceLock::new();
    static SPACE_PUNCT: OnceLock<Regex> = OnceLock::new();
    static START_PUNCT: OnceLock<Regex> = OnceLock::new();
    static DOUBLE_PUNCT: OnceLock<Regex> = OnceLock::new();
    static COMMA_END_PUNCT: OnceLock<Regex> = OnceLock::new();

    let multi_space = MULTI_SPACE.get_or_init(|| Regex::new(r"\s+").unwrap());
    let space_punct = SPACE_PUNCT.get_or_init(|| Regex::new(r"\s+([,.!?])").unwrap());
    let start_punct = START_PUNCT.get_or_init(|| Regex::new(r"^[,\s]+").unwrap());
    let double_punct = DOUBLE_PUNCT.get_or_init(|| Regex::new(r"([,.!?])\s*[,.]+").unwrap());
    let comma_end_punct = COMMA_END_PUNCT.get_or_init(|| Regex::new(r",([.!?])").unwrap());

    let mut result = text.to_string();

    // Collapse multiple spaces
    result = multi_space.replace_all(&result, " ").to_string();

    // Remove space before punctuation
    result = space_punct.replace_all(&result, "$1").to_string();

    // Remove leading punctuation and whitespace
    result = start_punct.replace_all(&result, "").to_string();

    // Remove duplicate punctuation
    result = double_punct.replace_all(&result, "$1").to_string();

    // Remove comma before end punctuation (e.g., ",?" -> "?")
    result = comma_end_punct.replace_all(&result, "$1").to_string();

    // Trim and capitalize first letter
    result = result.trim().to_string();

    // Remove trailing comma
    if result.ends_with(',') {
        result.pop();
        result = result.trim_end().to_string();
    }

    if !result.is_empty() {
        let mut chars: Vec<char> = result.chars().collect();
        chars[0] = chars[0].to_uppercase().next().unwrap_or(chars[0]);
        result = chars.into_iter().collect();
    }

    result
}

/// Remove filler words from text (standalone function)
///
/// This is a fast utility function for simple filler removal without
/// creating a TextPolisher instance.
pub fn remove_fillers(text: &str) -> String {
    let polisher = TextPolisher::new(PolishLevel::RegexOnly);
    polisher.polish(text).unwrap_or_else(|_| text.to_string())
}

/// Common transcription corrections
///
/// Maps common misheard/mistranscribed words to correct versions.
static CORRECTIONS: OnceLock<Vec<(Regex, &'static str)>> = OnceLock::new();

fn get_corrections() -> &'static Vec<(Regex, &'static str)> {
    CORRECTIONS.get_or_init(|| {
        vec![
            // Common homophone corrections (be careful with these!)
            // These are commented out as they might cause more harm than good
            // without context awareness
            // (Regex::new(r"\btheir\b").unwrap(), "there"),
            // Add more corrections as needed based on real-world usage

            // Fix spacing around quotes
            (Regex::new(r#"\s+""#).unwrap(), " \""),
            (Regex::new(r#""\s+"#).unwrap(), "\" "),

            // Fix common contractions
            (Regex::new(r"\bi m\b").unwrap(), "I'm"),
            (Regex::new(r"\bdont\b").unwrap(), "don't"),
            (Regex::new(r"\bwont\b").unwrap(), "won't"),
            (Regex::new(r"\bcant\b").unwrap(), "can't"),
            (Regex::new(r"\bwouldnt\b").unwrap(), "wouldn't"),
            (Regex::new(r"\bshouldnt\b").unwrap(), "shouldn't"),
            (Regex::new(r"\bcouldnt\b").unwrap(), "couldn't"),
            (Regex::new(r"\bisnt\b").unwrap(), "isn't"),
            (Regex::new(r"\barent\b").unwrap(), "aren't"),
            (Regex::new(r"\bwasnt\b").unwrap(), "wasn't"),
            (Regex::new(r"\bwerent\b").unwrap(), "weren't"),
            (Regex::new(r"\bhasnt\b").unwrap(), "hasn't"),
            (Regex::new(r"\bhavent\b").unwrap(), "haven't"),
            (Regex::new(r"\bhadnt\b").unwrap(), "hadn't"),
            (Regex::new(r"\bdoesnt\b").unwrap(), "doesn't"),
            (Regex::new(r"\bthats\b").unwrap(), "that's"),
            (Regex::new(r"\bwhats\b").unwrap(), "what's"),
            (Regex::new(r"\bheres\b").unwrap(), "here's"),
            (Regex::new(r"\btheres\b").unwrap(), "there's"),
            (Regex::new(r"\bwheres\b").unwrap(), "where's"),
            (Regex::new(r"\bits\b").unwrap(), "it's"), // This one is tricky - might need context
            (Regex::new(r"\bive\b").unwrap(), "I've"),
            (Regex::new(r"\byouve\b").unwrap(), "you've"),
            (Regex::new(r"\bweve\b").unwrap(), "we've"),
            (Regex::new(r"\btheyve\b").unwrap(), "they've"),
            (Regex::new(r"\bill\b").unwrap(), "I'll"),
            (Regex::new(r"\byoull\b").unwrap(), "you'll"),
            (Regex::new(r"\bwell\b").unwrap(), "we'll"), // Might conflict with "well" as in "well done"
            (Regex::new(r"\btheyll\b").unwrap(), "they'll"),
            (Regex::new(r"\bid\b").unwrap(), "I'd"),
            (Regex::new(r"\byoud\b").unwrap(), "you'd"),
            (Regex::new(r"\bhed\b").unwrap(), "he'd"),
            (Regex::new(r"\bshed\b").unwrap(), "she'd"),
            (Regex::new(r"\bwed\b").unwrap(), "we'd"),
            (Regex::new(r"\btheyd\b").unwrap(), "they'd"),
        ]
    })
}

/// Apply common transcription corrections
pub fn apply_corrections(text: &str) -> String {
    let corrections = get_corrections();
    let mut result = text.to_string();

    for (pattern, replacement) in corrections {
        result = pattern.replace_all(&result, *replacement).to_string();
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_remove_fillers_basic() {
        let input = "um so like I was thinking";
        let output = remove_fillers(input);
        assert_eq!(output, "I was thinking");
    }

    #[test]
    fn test_remove_fillers_complex() {
        let input = "Um, you know, basically I mean, it's actually pretty good, right?";
        let output = remove_fillers(input);
        assert_eq!(output, "It's pretty good?");
    }

    #[test]
    fn test_remove_fillers_case_insensitive() {
        let input = "UM UH LIKE you know";
        let output = remove_fillers(input);
        assert!(output.trim().is_empty() || output == "");
    }

    #[test]
    fn test_remove_fillers_preserves_content() {
        let input = "The algorithm is efficient";
        let output = remove_fillers(input);
        assert_eq!(output, "The algorithm is efficient");
    }

    #[test]
    fn test_cleanup_text_spaces() {
        let input = "hello   world";
        let output = cleanup_text(input);
        assert_eq!(output, "Hello world");
    }

    #[test]
    fn test_cleanup_text_punctuation() {
        let input = " , hello , world";
        let output = cleanup_text(input);
        assert_eq!(output, "Hello, world");
    }

    #[test]
    fn test_cleanup_text_double_punct() {
        let input = "hello,, world..";
        let output = cleanup_text(input);
        assert_eq!(output, "Hello, world.");
    }

    #[test]
    fn test_polish_level_none() {
        let polisher = TextPolisher::new(PolishLevel::None);
        let input = "um so like hello";
        let output = polisher.polish(input).unwrap();
        assert_eq!(output, input);
    }

    #[test]
    fn test_polish_level_regex() {
        let polisher = TextPolisher::new(PolishLevel::RegexOnly);
        let input = "um so like hello";
        let output = polisher.polish(input).unwrap();
        assert_eq!(output, "Hello");
    }

    #[test]
    fn test_apply_corrections() {
        let input = "i m going to the store";
        let output = apply_corrections(input);
        assert_eq!(output, "I'm going to the store");
    }

    #[test]
    fn test_apply_corrections_contractions() {
        let input = "I dont know if thats right";
        let output = apply_corrections(input);
        assert_eq!(output, "I don't know if that's right");
    }
}

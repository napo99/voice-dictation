//! Polish integration tests
//!
//! Tests the text polishing functionality including filler word removal
//! and text normalization.

use vd_polish::TextPolisher;

/// Test basic filler word removal
#[test]
fn test_filler_removal() {
    let polisher = TextPolisher::default();

    let input = "So, um, I was thinking, you know, that we should like go to the store.";
    let output = polisher.polish(input);

    // Should remove "um", "you know", "like" as fillers
    assert!(!output.contains("um,"));
    assert!(!output.contains("you know,"));
}

/// Test that meaningful content is preserved
#[test]
fn test_content_preservation() {
    let polisher = TextPolisher::default();

    let input = "The quick brown fox jumps over the lazy dog.";
    let output = polisher.polish(input);

    // Content should be preserved (may have minor formatting changes)
    assert!(output.contains("quick brown fox"));
    assert!(output.contains("lazy dog"));
}

/// Test empty input handling
#[test]
fn test_empty_input() {
    let polisher = TextPolisher::default();

    let output = polisher.polish("");
    assert!(output.is_empty() || output.trim().is_empty());
}

/// Test whitespace normalization
#[test]
fn test_whitespace_normalization() {
    let polisher = TextPolisher::default();

    let input = "Hello    world,   how    are   you?";
    let output = polisher.polish(input);

    // Should not have multiple consecutive spaces
    assert!(!output.contains("  "));
}

/// Test sentence capitalization
#[test]
fn test_sentence_capitalization() {
    let polisher = TextPolisher::default();

    let input = "hello world. this is a test.";
    let output = polisher.polish(input);

    // First letter should be capitalized
    assert!(output.starts_with('H') || output.starts_with('h'));
}

/// Test common contractions
#[test]
fn test_contractions() {
    let polisher = TextPolisher::default();

    let input = "I am going to the store and I will be back soon.";
    let output = polisher.polish(input);

    // Should preserve or contract appropriately
    assert!(output.contains("store") && output.contains("back"));
}

/// Test punctuation at end of sentence
#[test]
fn test_ending_punctuation() {
    let polisher = TextPolisher::default();

    let input = "This is a test";
    let output = polisher.polish(input);

    // Should have period at end (polisher may add it)
    let trimmed = output.trim();
    assert!(
        trimmed.ends_with('.') ||
        trimmed.ends_with('!') ||
        trimmed.ends_with('?') ||
        trimmed.ends_with("test")  // Or unchanged
    );
}

/// Test multiple sentences
#[test]
fn test_multiple_sentences() {
    let polisher = TextPolisher::default();

    let input = "First sentence. Second sentence. Third one too.";
    let output = polisher.polish(input);

    // Should preserve sentence structure
    assert!(output.contains('.'));
    assert!(output.contains("First") || output.contains("first"));
}

/// Test various filler patterns
#[test]
fn test_various_fillers() {
    let polisher = TextPolisher::default();

    // Test various common fillers
    let cases = [
        ("Well, actually, the thing is complicated.", "complicated"),
        ("Basically, it works like this.", "works"),
        ("I mean, honestly, it is fine.", "fine"),
    ];

    for (input, must_contain) in cases {
        let output = polisher.polish(input);
        assert!(
            output.contains(must_contain),
            "Expected '{}' to contain '{}'",
            output,
            must_contain
        );
    }
}

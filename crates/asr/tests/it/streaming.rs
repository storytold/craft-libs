//! The streaming protocol through the public API: partial then final, silence, and status.

use craft_asr::{AudioBuffer, FakeRecognizer, Recognizer, State, TARGET_RATE};

fn speech(seconds: usize) -> AudioBuffer {
    AudioBuffer::new(vec![0.2; TARGET_RATE as usize * seconds], TARGET_RATE, 1)
}

#[test]
fn a_scripted_provider_reports_partial_then_final_and_starts_and_stops() {
    let mut provider = FakeRecognizer::scripted(&["hello", "world"]);
    assert_eq!(provider.status().state, State::Idle);
    provider.start().unwrap_or(());
    assert_eq!(provider.status().state, State::Running);

    provider.feed(&speech(1)).unwrap_or(());
    let first = provider.poll();
    assert_eq!(first.len(), 1);
    assert!(!first[0].r#final);
    assert_eq!(first[0].text, "hello");

    provider.feed(&speech(1)).unwrap_or(());
    let second = provider.poll();
    assert_eq!(second.len(), 1);
    assert!(second[0].r#final);
    assert_eq!(second[0].text, "hello");

    provider.feed(&speech(1)).unwrap_or(());
    assert_eq!(provider.poll()[0].text, "world");

    provider.finish().unwrap_or(());
    assert_eq!(provider.status().state, State::Stopped);
}

#[test]
fn empty_audio_gives_an_empty_result() {
    let mut provider = FakeRecognizer::scripted(&["never spoken"]);
    provider.start().unwrap_or(());
    provider.feed(&AudioBuffer::new(Vec::new(), TARGET_RATE, 1)).unwrap_or(());
    assert!(provider.poll().is_empty(), "silence produces no partial line");
    provider.finish().unwrap_or(());
    assert!(provider.poll().is_empty(), "and no final line either");
}

#[test]
fn a_refusing_provider_reports_the_failure_in_its_status() {
    let mut provider = FakeRecognizer::refusing("no microphone");
    assert!(provider.start().is_err());
    assert_eq!(provider.status().state, State::Failed);
    assert!(provider.status().summary().contains("no microphone"));
}

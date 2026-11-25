//! Hostile audio and transcript bounds through the public API.

use craft_asr::{AudioBuffer, AudioError, FakeRecognizer, Limits, MAX_SAMPLE_RATE, MAX_SAMPLES, Recognizer, TARGET_RATE, downmix, resample_linear};

fn speech(seconds: usize) -> AudioBuffer {
    AudioBuffer::new(vec![0.1; TARGET_RATE as usize * seconds], TARGET_RATE, 1)
}

#[test]
fn hostile_sample_rates_channels_and_lengths_are_rejected() {
    let limits = Limits::default();
    for buffer in [
        AudioBuffer::new(vec![], 0, 1),
        AudioBuffer::new(vec![], MAX_SAMPLE_RATE + 1, 1),
        AudioBuffer::new(vec![], TARGET_RATE, 0),
        AudioBuffer::new(vec![], TARGET_RATE, 33),
        AudioBuffer::new(vec![0.0; 3], TARGET_RATE, 2),
    ] {
        assert!(buffer.validate(&limits).is_err(), "{buffer:?}");
    }
    assert!(matches!(AudioBuffer::new(vec![], 0, 1).validate(&limits), Err(AudioError::ZeroSampleRate)));
    assert!(matches!(AudioBuffer::new(vec![], TARGET_RATE, 33).validate(&limits), Err(AudioError::TooManyChannels { .. })));
    let byte_limited = Limits { max_bytes: 8, ..Limits::default() };
    assert!(matches!(AudioBuffer::new(vec![0.0; 3], TARGET_RATE, 1).validate(&byte_limited), Err(AudioError::TooManyBytes { .. })));

    let mut provider = FakeRecognizer::scripted(&["hello"]).with_limits(Limits { max_audio_seconds: 1, ..Limits::default() });
    provider.start().unwrap_or(());
    let too_long = AudioBuffer::new(vec![0.0; TARGET_RATE as usize * 2], TARGET_RATE, 1);
    assert!(provider.feed(&too_long).is_err(), "a buffer over the configured span is refused");
    let huge_rate = AudioBuffer::new(vec![0.0; 2], MAX_SAMPLE_RATE + 1, 1);
    assert!(provider.feed(&huge_rate).is_err());
}

#[test]
fn conversions_are_bounded_and_clamped() {
    assert!(downmix(&[], 0).is_empty());
    assert_eq!(downmix(&[f32::NAN, f32::INFINITY], 1), vec![0.0, 0.0], "non-finite samples become silence");
    assert!(resample_linear(&[], 48_000, TARGET_RATE).is_empty());
    assert!(resample_linear(&[0.0], 0, TARGET_RATE).is_empty());
    let capped = resample_linear(&vec![0.0; MAX_SAMPLES], 1, MAX_SAMPLE_RATE);
    assert_eq!(capped.len(), MAX_SAMPLES, "a hostile ratio cannot grow the output past the cap");
}

#[test]
fn a_short_limit_stops_a_runaway_transcript() {
    let limits = Limits { max_segments: 1, ..Limits::default() };
    let mut provider = FakeRecognizer::scripted(&["one", "two"]).with_limits(limits);
    provider.start().unwrap_or(());
    provider.feed(&speech(1)).unwrap_or(());
    assert_eq!(provider.poll().len(), 1);
    assert!(provider.feed(&speech(1)).is_err(), "the second segment is refused");
}

#[test]
fn an_empty_buffer_is_valid_but_reports_nothing() {
    assert_eq!(AudioBuffer::new(vec![], TARGET_RATE, 1).validate(&Limits::default()), Ok(()));
    let mut provider = FakeRecognizer::scripted(&["hello"]);
    provider.start().unwrap_or(());
    provider.feed(&AudioBuffer::new(vec![], TARGET_RATE, 1)).unwrap_or(());
    assert!(provider.poll().is_empty());
}

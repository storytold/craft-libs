//! Saved-channel metadata, without native display defaults or color conversion.
use super::{Budget, Diagnostic, Error, Evidence, Failure, Issue, IssueKind, Limits, Property, Reader, Result, Scan, StopReason, word};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelColorIndicates {
    SelectedAreas,
    MaskedAreas,
}

#[derive(Debug, Default)]
pub struct ChannelMetadata {
    pub selected: Option<Evidence<()>>,
    /// Raw nonzero flag; native visibility/default policy belongs to consumers.
    pub visible: Option<Evidence<u32>>,
    /// Nonlinear source RGB, independent of the drawable precision/profile.
    pub byte_color: Option<Evidence<[u8; 3]>>,
    /// Valid float evidence supersedes byte compatibility evidence, regardless of order.
    pub float_color: Option<Evidence<[f32; 3]>>,
    pub byte_opacity: Option<Evidence<u32>>,
    /// Valid float evidence supersedes byte compatibility evidence, regardless of order.
    pub float_opacity: Option<Evidence<f32>>,
    pub indicates: Option<Evidence<ChannelColorIndicates>>,
    pub issues: Vec<Issue>,
    pub source_stopped: Option<Diagnostic>,
    pub stopped: Option<Diagnostic>,
}

impl ChannelMetadata {
    /// Prefix evidence cannot establish final duplicate precedence or absence.
    pub fn complete(&self) -> bool {
        self.source_stopped.is_none() && self.stopped.is_none()
    }
}

/// Retain last-valid evidence independently for each wire representation. No
/// absent value becomes a native default; callers must require `complete()`
/// before resolving float-over-byte precedence, selected targets or visibility.
pub fn channel(properties: &Scan<Property<'_>>, limits: Limits, budget: &mut Budget, keep_going: &dyn Fn() -> bool) -> Result<ChannelMetadata> {
    channel_from_properties(&properties.records, properties.stopped, limits, budget, keep_going)
}

/// Decode an already checked property slice, including an effect-mask header's properties,
/// without allocating a record vector or rescanning the source. Pass its source stop
/// diagnostic: partial source evidence cannot establish final duplicate precedence.
pub fn channel_from_properties(
    properties: &[Property<'_>],
    source_stopped: Option<Diagnostic>,
    limits: Limits,
    budget: &mut Budget,
    keep_going: &dyn Fn() -> bool,
) -> Result<ChannelMetadata> {
    budget.cancel_safe(|budget| channel_read(properties, source_stopped, limits, budget, keep_going))
}

fn channel_read(
    properties: &[Property<'_>],
    source_stopped: Option<Diagnostic>,
    limits: Limits,
    budget: &mut Budget,
    keep_going: &dyn Fn() -> bool,
) -> Result<ChannelMetadata> {
    let mut reader = Reader { limits, budget, keep_going };
    reader.check().map_err(|_| Error::Cancelled)?;
    let mut output = ChannelMetadata { source_stopped, ..Default::default() };
    for (ordinal, property) in properties.iter().enumerate() {
        reader.check().map_err(|_| Error::Cancelled)?;
        let reason = if ordinal >= limits.records.records {
            Some(StopReason::RecordLimit)
        } else if property.payload.len() > limits.records.payload_bytes {
            Some(StopReason::PayloadLimit)
        } else {
            None
        };
        if let Some(reason) = reason {
            output.stopped = Some(Diagnostic { offset: property.offset, ordinal, reason });
            break;
        }
        let result = (|| {
            reader.charge(8)?;
            if !matches!(property.kind, 3 | 6 | 8 | 14 | 16 | 33 | 38) {
                return Ok(());
            }
            reader.charge(property.payload.len())?;
            let data = property.payload;
            let mut valid = true;
            match property.kind {
                3 if data.is_empty() => output.selected = Some(Evidence { offset: property.offset, value: () }),
                8 => {
                    if let Some(value) = word(data) {
                        output.visible = Some(Evidence { offset: property.offset, value });
                    } else {
                        valid = false;
                    }
                }
                6 => {
                    if let Some(value) = word(data).filter(|value| *value <= 255) {
                        output.byte_opacity = Some(Evidence { offset: property.offset, value });
                    } else {
                        valid = false;
                    }
                }
                14 => {
                    if let Some(value) = word(data).filter(|value| *value <= 1) {
                        let value = if value == 0 { ChannelColorIndicates::SelectedAreas } else { ChannelColorIndicates::MaskedAreas };
                        output.indicates = Some(Evidence { offset: property.offset, value });
                    } else {
                        valid = false;
                    }
                }
                16 => {
                    if let Some(&value) = data.as_array::<3>() {
                        output.byte_color = Some(Evidence { offset: property.offset, value });
                    } else {
                        valid = false;
                    }
                }
                33 => {
                    if let Some(value) = data.as_array::<4>().map(|bytes| f32::from_be_bytes(*bytes)).filter(|value| (0.0..=1.0).contains(value)) {
                        output.float_opacity = Some(Evidence { offset: property.offset, value });
                    } else {
                        valid = false;
                    }
                }
                38 => {
                    if let Some(words) = data.as_chunks::<4>().0.as_array::<3>().filter(|_| data.len() == 12) {
                        let value = words.map(f32::from_be_bytes);
                        if value.iter().all(|value| (0.0..=1.0).contains(value)) {
                            output.float_color = Some(Evidence { offset: property.offset, value });
                        } else {
                            valid = false;
                        }
                    } else {
                        valid = false;
                    }
                }
                _ => valid = false,
            }
            if !valid {
                reader.issue(&mut output.issues, property, IssueKind::InvalidValue)?;
            }
            Ok(())
        })();
        match result {
            Ok(()) => {}
            Err(Failure::Cancelled) => return Err(Error::Cancelled),
            Err(Failure::Stop(reason)) => {
                output.stopped = Some(Diagnostic { offset: property.offset, ordinal, reason });
                break;
            }
        }
    }
    reader.check().map_err(|_| Error::Cancelled)?;
    Ok(output)
}

#[cfg(test)]
mod tests;

//! [`Procedural`]: one use of a graph package (`DataProcedural`): the resource, its parameter values
//! (tweaks), the outputs wired to channels and the inputs fed from other sources.

use super::channel::{ChannelMask, ProjectChannel};
use super::source::ProjectSource;

#[derive(Debug, Clone, PartialEq)]
pub struct Procedural {
    pub uid: Option<u64>,
    /// `urlToSbsRes`: `resource://<shelf path>?version=<hash>` and the package extension, or an embedded `/<name>?version=<id>`.
    pub url: String,
    pub tweaks: Vec<Tweak>,
    pub outputs: Vec<ProceduralOutput>,
    pub inputs: Vec<ProceduralInput>,
    /// `outputIdentifierAlpha`: the output that is the fill's coverage, empty for none.
    pub alpha_output: String,
    /// `monoChannelOutputIndex`: which of `outputs` a grey use (a mask) takes.
    pub mono_output: u64,
}

impl Procedural {
    /// The resource name without shelf path and version (`concrete_cast` for
    /// `resource://<shelf>/concrete_cast/concrete_cast?version=…` and the package extension).
    pub fn name(&self) -> &str {
        let path = self.url.split('?').next().unwrap_or("");
        path.rsplit('/').next().unwrap_or(path)
    }
}

/// One parameter value (`DataTweak*`), keyed by the graph's input identifier.
#[derive(Debug, Clone, PartialEq)]
pub struct Tweak {
    pub identifier: String,
    pub value: TweakValue,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TweakValue {
    Int(i32),
    /// `DataTweakInt2`, e.g. `$outputsize` as log2 sizes.
    Int2([i32; 2]),
    /// `DataTweakFloat` … `DataTweakFloat4`: 1 to 4 components.
    Floats(Vec<f32>),
    /// `DataTweakByteArray` (text inputs).
    Text(String),
    /// A tweak class the model does not know, by class name.
    Other(String),
}

/// An output of the graph and the channels it feeds (`DataProceduralOutput`).
#[derive(Debug, Clone, PartialEq)]
pub struct ProceduralOutput {
    pub identifier: String,
    pub channels: ChannelMask,
    /// `identifierAlpha`: the graph output that is this output's coverage (`output_Alpha`), empty for none.
    pub alpha: String,
}

/// An image input of the graph and what feeds it (`DataProceduralInputsSource`).
#[derive(Debug, Clone, PartialEq)]
pub struct ProceduralInput {
    /// Input identifiers (`DataProceduralInput.identifier`) with the channel each one reads.
    pub identifiers: Vec<(String, ProjectChannel)>,
    pub source: Option<Box<ProjectSource>>,
}

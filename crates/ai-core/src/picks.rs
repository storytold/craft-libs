//! Model presets: which model to suggest for a job, and why.
//!
//! A person who adds a connection should not have to know which of a few hundred models is good at
//! inpainting. For each job this module names up to six picks (see [`Pick`]): the ones that can be
//! worked out from what a provider reports (cheapest, newest) are computed, and the ones that are a
//! judgement (recommended, best result, fastest, most popular) come from a small curated
//! [`Catalog`] kept as data, so it can change without an app release and needs no telemetry.
//!
//! The rules that keep this honest:
//! - A pick is only ever one of the candidates given. A curated entry the connections do not offer
//!   is skipped, never invented, and a model that disappears simply stops earning its pick.
//! - Cheapest only compares models that report a price (or are free to run); with no price data
//!   there is no cheapest pick rather than a guess. Newest likewise needs a release date.
//! - Nothing here binds anything. The caller offers the picks and the person agrees.

use serde::{Deserialize, Serialize};

use crate::capability::CapabilityId;
use crate::model::{ModelInfo, Pricing};

/// Largest catalog file read, in bytes.
pub const MAX_CATALOG_BYTES: usize = 256 * 1024;

/// What a preset is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Pick {
    /// The one we would choose: a balance of quality, speed and price.
    Recommended,
    BestResult,
    LowestCost,
    Fastest,
    Newest,
    MostPopular,
}

impl Pick {
    /// In the order a picker lists them.
    pub const ALL: [Pick; 6] = [Pick::Recommended, Pick::BestResult, Pick::LowestCost, Pick::Fastest, Pick::Newest, Pick::MostPopular];

    /// Whether this pick is a judgement kept in the catalog, as opposed to worked out from data.
    pub fn is_curated(self) -> bool {
        !matches!(self, Pick::LowestCost | Pick::Newest)
    }

    pub fn key(self) -> &'static str {
        match self {
            Pick::Recommended => "recommended",
            Pick::BestResult => "best_result",
            Pick::LowestCost => "lowest_cost",
            Pick::Fastest => "fastest",
            Pick::Newest => "newest",
            Pick::MostPopular => "most_popular",
        }
    }
}

/// One curated pick: for this job, on this kind of connection, this model earns this tag.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogEntry {
    pub capability: String,
    /// The tag it earns, as [`Pick::key`].
    pub pick: String,
    /// The connection's provider key, such as `openrouter`.
    pub provider: String,
    pub model: String,
}

/// The curated picks. Earlier entries for the same job and tag win.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Catalog {
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub entries: Vec<CatalogEntry>,
}

/// Why a catalog could not be used.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CatalogError {
    TooLarge,
    Syntax(String),
}

impl std::fmt::Display for CatalogError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooLarge => write!(f, "the catalog is larger than {MAX_CATALOG_BYTES} bytes"),
            Self::Syntax(e) => write!(f, "the catalog could not be read: {e}"),
        }
    }
}

impl std::error::Error for CatalogError {}

impl Catalog {
    /// The catalog built into the apps. It starts empty: curated picks are added as they are
    /// checked, and the computed picks work without it.
    pub fn builtin() -> Self {
        Self::from_json(include_str!("catalog.json")).unwrap_or_default()
    }

    pub fn from_json(text: &str) -> Result<Self, CatalogError> {
        if text.len() > MAX_CATALOG_BYTES {
            return Err(CatalogError::TooLarge);
        }
        serde_json::from_str(text).map_err(|e| CatalogError::Syntax(e.to_string()))
    }

    /// This catalog on top of `base`: where both name a model for the same job and tag, this one
    /// wins, and anything only `base` has still applies. That is how a catalog file updated
    /// without an app release sits over the one built into the app.
    pub fn over(mut self, base: Catalog) -> Catalog {
        self.version = self.version.max(base.version);
        self.entries.extend(base.entries);
        self
    }

    fn lookup<'a>(&self, capability: &CapabilityId, pick: Pick, candidates: &'a [Candidate<'a>]) -> Option<&'a Candidate<'a>> {
        self.entries
            .iter()
            .filter(|e| e.capability == capability.as_str() && e.pick == pick.key())
            .find_map(|e| candidates.iter().find(|c| c.provider == e.provider && c.model.id.as_str() == e.model))
    }
}

/// A model that could fill the job, with what is needed to rank it.
#[derive(Clone, Copy, Debug)]
pub struct Candidate<'a> {
    /// The connection's provider key.
    pub provider: &'a str,
    pub model: &'a ModelInfo,
    /// Running it costs nothing (a local model, a free connection).
    pub free: bool,
}

/// What earned a pick, for the one line a picker shows next to it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PickedBy {
    /// Chosen by whoever keeps the catalog.
    Catalog,
    /// The lowest reported price, in millionths of a dollar for one image or one million tokens.
    Price(u64),
    /// Free to run.
    Free,
    /// The most recent release, as Unix seconds.
    Released(u64),
}

/// A pick that was earned, and by which candidate (an index into the candidates given).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Picked {
    pub pick: Pick,
    pub candidate: usize,
    pub by: PickedBy,
}

/// One number to compare prices on: a per-image price, or input plus output per million tokens.
fn price_rank(p: &Pricing, capability: &CapabilityId) -> Option<u64> {
    if capability.as_str().starts_with("image") || capability.as_str().starts_with("video") {
        return p.per_image_micros;
    }
    match (p.input_micros_per_mtok, p.output_micros_per_mtok) {
        (None, None) => None,
        (i, o) => Some(i.unwrap_or(0).saturating_add(o.unwrap_or(0))),
    }
}

/// Every pick the candidates earn for `capability`, in [`Pick::ALL`] order. Ties go to the model
/// id that sorts first, so the answer does not change from one run to the next.
pub fn choose(capability: &CapabilityId, candidates: &[Candidate<'_>], catalog: &Catalog) -> Vec<Picked> {
    let index_of = |c: &Candidate<'_>| candidates.iter().position(|x| std::ptr::eq(x.model, c.model) && x.provider == c.provider);
    let mut out = Vec::new();
    for pick in Pick::ALL {
        let found = match pick {
            Pick::LowestCost => lowest_cost(capability, candidates),
            Pick::Newest => newest(candidates),
            _ => catalog.lookup(capability, pick, candidates).and_then(|c| index_of(c).map(|i| (i, PickedBy::Catalog))),
        };
        if let Some((candidate, by)) = found {
            out.push(Picked { pick, candidate, by });
        }
    }
    out
}

fn lowest_cost(capability: &CapabilityId, candidates: &[Candidate<'_>]) -> Option<(usize, PickedBy)> {
    candidates
        .iter()
        .enumerate()
        .filter_map(|(i, c)| {
            if c.free {
                return Some((i, 0, PickedBy::Free));
            }
            let rank = price_rank(c.model.pricing.as_ref()?, capability)?;
            Some((i, rank, PickedBy::Price(rank)))
        })
        .min_by(|a, b| (a.1, candidates[a.0].model.id.as_str()).cmp(&(b.1, candidates[b.0].model.id.as_str())))
        .map(|(i, _, by)| (i, by))
}

fn newest(candidates: &[Candidate<'_>]) -> Option<(usize, PickedBy)> {
    candidates
        .iter()
        .enumerate()
        .filter_map(|(i, c)| Some((i, c.model.released?)))
        .max_by(|a, b| {
            a.1.cmp(&b.1)
                // On a tie the id that sorts first wins, so reverse the id comparison.
                .then_with(|| candidates[b.0].model.id.as_str().cmp(candidates[a.0].model.id.as_str()))
        })
        .map(|(i, at)| (i, PickedBy::Released(at)))
}

#[cfg(test)]
mod tests;

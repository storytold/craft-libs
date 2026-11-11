//! [`ScopeWords`]: every word of the scope panel, its menus and the vectorscope (`scopes.*`).

craft_ui::words! {
    /// The scope panel's words, English by default (Photoshop's and Camera Raw's). Channel and source names are the
    /// host's ([`ScopeChannel::label`](super::ScopeChannel::label), [`ScopeSource::label`](super::ScopeSource::label)).
    #[non_exhaustive]
    pub struct ScopeWords {
        /// The plot's accessible name.
        histogram: "scopes.histogram" = "Histogram";
        /// The plot's value text: `{channel}` and `{source}` are the host's labels.
        histogram_value: template "scopes.histogramValue" = "{channel} channel, {source}";
        /// The source named in the value text when the host passes none.
        entire_image: "scopes.entireImage" = "Entire Image";
        /// The Channel row's caption.
        channel: "scopes.channel" = "Channel:";
        /// The Channel list's accessible name.
        channel_name: "scopes.channelName" = "Channel";
        /// The Channel list's entry that overlays the colour channels.
        colors: "scopes.colors" = "Colors";
        /// The Source row's caption.
        source: "scopes.source" = "Source:";
        /// The Source list's accessible name.
        source_name: "scopes.sourceName" = "Source";
        /// The refresh button's name.
        uncached_refresh: "scopes.uncachedRefresh" = "Uncached Refresh";
        /// The cached-data mark's tooltip and name.
        refresh_hint: "scopes.refreshHint" = "Click to refresh histogram with uncached data";
        /// The scope menu's accessible name.
        scope_menu: "scopes.menu" = "Scope";
        /// The statistics grid's accessible name.
        statistics: "scopes.statistics" = "Statistics";
        mean: "scopes.mean" = "Mean:";
        standard_deviation: "scopes.standardDeviation" = "Std Dev:";
        median: "scopes.median" = "Median:";
        pixels: "scopes.pixels" = "Pixels:";
        level: "scopes.level" = "Level:";
        count: "scopes.count" = "Count:";
        percentile: "scopes.percentile" = "Percentile:";
        cache_level: "scopes.cacheLevel" = "Cache Level:";
        /// The clipping buttons' names and tooltips.
        shadow_warning: "scopes.shadowWarning" = "Shadow Clipping Warning (U)";
        highlight_warning: "scopes.highlightWarning" = "Highlight Clipping Warning (O)";
        /// The tone zones' group name.
        tone_zones: "scopes.toneZones" = "Tone zones";
        blacks: "scopes.blacks" = "Blacks";
        shadows: "scopes.shadows" = "Shadows";
        exposure: "scopes.exposure" = "Exposure";
        highlights: "scopes.highlights" = "Highlights";
        whites: "scopes.whites" = "Whites";
        /// A zone's tooltip: `{zone}` its name, `{value}` the host's value text.
        zone_tooltip: template "scopes.zoneTooltip" = "{zone}  {value}";
        /// The vectorscope's accessible name.
        vectorscope: "scopes.vectorscope" = "Vectorscope";
        /// The vectorscope's value text.
        vectorscope_value: template "scopes.vectorscopeValue" = "Peak saturation {saturation} %, mean hue {hue} degrees";
        /// The hover badge, RGB form: `{r}`, `{g}`, `{b}` in 0..=255, no decimals.
        readout_rgb: template "scopes.readoutRgb" = "R {r}  G {g}  B {b}";
        /// The hover badge, Lab form, one decimal.
        readout_lab: template "scopes.readoutLab" = "L {l}  a {a}  b {b}";
    }
}

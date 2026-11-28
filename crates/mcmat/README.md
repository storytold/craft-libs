# craft-mcmat

The MCMAT v3 portable material is a ZIP containing a version-only `material.json`, one
`source.sbs`, and the required source assets under `resources/`. The sole root-level
material graph in the SBS is the entry. Applications inspect its exposed controls and
cook its outputs through MaterialCraft. Texture maps and MaterialX documents are generated
by exporters. Earlier archives must be republished from their original projects.

`Material::open` checks archive entries and expansion budgets before returning source
files. `write` compresses the XML and version marker and preserves source asset bytes.

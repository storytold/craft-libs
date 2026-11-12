# craft-crs

Adobe Camera Raw settings: the `crs:` namespace (`http://ns.adobe.com/camera-raw-settings/1.0/`)
in an XMP packet, as Camera Raw's Save Settings writes an `.xmp` settings file and as Camera Raw,
Lightroom and Bridge write a raw file's sidecar.

- `Settings::parse` reads a file; `Settings::from_packet` reads a packet already parsed with
  `packet` (or any `craft_xml` tree with the `crs:` / `rdf:` / `x:` prefixes of `CANONICAL`).
  Settings come back as written, in file order: a name and a `Value` (text from an attribute or a
  plain element, the plain items of an `rdf:Seq`, or the element of a structure such as
  `crs:GradientBasedCorrections`, for the caller to read). The file's own prefixes do not matter.
  A description inside a structure belongs to the structure, not to the settings.
- `number`, `signed` and `points`: Camera Raw's numbers (`+0.50`) and tone curve points
  (`"32, 22"`).
- `Settings::crop`: the crop rectangle, only when `HasCrop` is `True`; straightened, inverted,
  empty or unreadable crops are reported as unusable with the keys present. `Crop::oriented` and
  `Crop::pixels` place it on an upright frame of a given size.
- `Settings::to_xmp`: a settings file Camera Raw's Load Settings reads, structures included,
  naming the toolkit that wrote it; `toolkit` reads that name back.

Which setting drives which control, and the ranges they clamp to, are the application's.

## Limits

| Limit | Value |
| --- | --- |
| Input | 4 MiB (`MAX_XMP_BYTES`) |
| Element nesting | 512 (`MAX_XML_DEPTH`) |
| Document type declarations | refused |

## XMP

The packet is parsed with `craft-xml` directly. A shared XMP reader can take over `packet` later;
`Settings::from_packet` is the seam.

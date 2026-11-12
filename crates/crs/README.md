# craft-crs

Adobe Camera Raw settings: the `crs:` namespace (`http://ns.adobe.com/camera-raw-settings/1.0/`)
in an XMP packet, as Camera Raw's Save Settings writes an `.xmp` settings file and as Camera Raw,
Lightroom and Bridge write a raw file's sidecar.

- `Settings::parse` reads a file; `Settings::from_xmp` takes a packet already read by
  `craft_metadata::parse_xmp`. Settings come back by name (`Exposure2012`), the last one winning
  where a name repeats: text from an attribute or a plain element, the plain items of an
  `rdf:Seq`, or a structure (`crs:GradientBasedCorrections` and other local corrections) as
  `craft_metadata`'s `XmpValue`, for the caller to read. The file's own prefixes do not matter.
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
| Nesting and size of the packet | `craft-metadata`'s XMP limits |
| Structure depth written | 64 (`MAX_WRITE_DEPTH`) |

## XMP

Packets are read by `craft-metadata`'s XMP reader, the one every crate shares. `toolkit` reads
only the `x:xmptk` attribute of the first `x:xmpmeta` start tag, with a bounded `craft-xml` scan,
because that reader does not return it.

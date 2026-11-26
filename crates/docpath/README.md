# craft-docpath

Decides whether a file path or file URL stored in a document may be opened, read or written, and
makes safe file names from user text. No dependencies beyond the standard library.

| Item | Responsibility |
| --- | --- |
| `classify`, `is_local`, `resolve`, `resolve_absolute` | A lexical check of a stored path: local files, and files on the share the document came from, are followed; other computers, device paths and non-file URLs are refused (`PathKind`, `Refused`). It does no I/O and gives the same answer on every platform apart from colons. |
| `links` | Linked files: `Link`, `LinkIdentity` and `LinkStatus`, `status` per link, `check_candidate` and `rank` for a file picked to replace a missing one, `derive_remap` and `Remap` to relink the others, `folder_matches`, and `LinkSearch` to search a folder tree as a job. File access goes only through `LinkFiles` (`SystemLinkFiles`; `FakeLinkFiles` in stories and tests). |
| `names` | File names made from user text: a document title as the suggested name of an export, a page range as a set of file names, an attachment's stored name as a Save suggestion; `natural_cmp` is the order file names are listed in. The rules hold on every platform, so a name made on Linux also works on Windows. |

## Names

- `safe_file_name(text, fallback)`: path separators, reserved characters (`/ \ : * ? " < > |`) and
  control characters become `_`; bidirectional controls are dropped; leading whitespace and
  trailing dots and whitespace are trimmed; Windows device names (`CON`, `NUL`, `COM0`–`COM9`,
  `LPT0`–`LPT9`, `CONIN$`, `CONOUT$`, superscript forms, with any extension) get a leading `_`.
  The result is cut to `MAX_NAME_BYTES` (200) at a character boundary; a short extension (under
  100 bytes) is kept and the stem is cut instead. `fallback` is returned, as given, when nothing
  is left.
- `safe_file_stem(text, extension)`: the same rules for a name whose extension the app adds; a
  trailing `.<extension>` in any case is removed first.
- `is_plain_file_name` and `is_reserved_device_name`: the checks the two functions use.
- `unique_file_name(name, CopyNumbering, taken)`: the first free `name 2.ext` (`Space`, Finder) or
  `name (2).ext` (`Parenthesized`, Explorer and GNOME Files; `CopyNumbering::platform()` picks the
  platform's), numbered before the last extension; `Parenthesized` counts on from a trailing `(n)`, `Space` keeps a trailing number as part of the name (`Chapter 1 2.docx`).

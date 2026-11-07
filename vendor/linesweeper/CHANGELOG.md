# Changelog

## 0.5.0 - 2026-10-05

### Changed

- Boolean operations now attempt to preserve quadratic segments: they will no
  longer be automatically converted into cubics (issue 2774a82).

### Fixed

- Fixed a panic with polylines that have degenerate segments (issue da2b71b).
- Fixed a panic with intersections exactly at the end of segments (issue f8732c7).
- Fixed a panic (in slow-asserts mode only) from an ordering violation (issue df03063).
- Fixed an infinite loop (issue a140a5f).

### Added

- Added some convenience methods to `Contours` (patch d4a09be).

## 0.4.0 - 2026-06-14

0.4.0 is the first release with a changelog entry; for previous releases, we
didn't have enough users to worry about it.

### Changed

- There is a new "approximate" output mode, which is now the default. Previous
  versions of linesweeper provided strong correctness guarantees, but those
  guarantees made some cases slow and caused more subdivisions than necessary.
  Also, I think that those stronger guarantees are probably not needed for most
  applications -- other boolean ops libraries don't provide them as far as I
  know. So linesweeper now provides weaker guarantees by default. For the old
  behaviour, see `Topology::contours_correct`.
- The winding direction conventions have changed; output curves have been
  reversed, so that they agree with `kurbo`'s conventions.
- The `BinaryWindingNumber` API has changed; it now uses the `OneOfTwo` enum to
  distinguish the two sets, instead of a `bool`.

### Added

- The new `Topology::has_normal_contours` and `Topology::is_disjoint_tags` allow
  for detecting some no-op (or approximate no-op) situations.

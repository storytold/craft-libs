# craft-ical

iCalendar (RFC 5545) building blocks, read and written. Times are `chrono` values; instants are
`i64` seconds since 1970-01-01T00:00:00Z.

- **`lines`**: `Lines` unfolds a text (CRLF, LF or CR; a byte order mark is skipped);
  `ContentLine::parse` splits `NAME;PARAM="quoted",x:value` into an upper-cased name,
  parameters (RFC 6868 caret escapes undone) and the value as written; `fold` writes a line in
  75-octet pieces without splitting a character; `param_value` quotes and escapes a parameter.
- **`values`**: `When` (`DATE`, floating or zoned `DATE-TIME`, UTC `DATE-TIME`, with the ISO 8601
  punctuation some writers add), `parse_duration`, `escape_text` / `unescape_text`, `split_list`,
  `format_date` / `format_local` / `format_utc`, and the wall-clock helpers they rest on.
- **`rrule`**: `Recurrence`, the subset of RRULE calendars show and edit (daily, weekly, monthly,
  yearly; interval; weekdays; the nth or last weekday; count; until), `parse_rrule` and
  `format_rrule`, and `format_for` / `format_zoned`, which spell out the start's weekday and
  month and give `UNTIL` the start's value type. A rule outside the subset is read as the nearest
  it holds.
- **`zones`**: `TimeZones::collect` reads a text's `VTIMEZONE`s (yearly rules by weekday or day
  of the month, `UNTIL`, `COUNT`, `RDATE`s); a `VTimezone` answers `offset_at(instant)`,
  `offset_for_wall(wall)` and `instant(wall)`, with RFC 5545's reading of skipped and repeated
  hours. `write_vtimezone` writes a zone from the caller's `ZoneRules` (offset at an instant,
  transitions): one `DAYLIGHT` and one `STANDARD` part with yearly rules where the zone keeps one
  rule, else change by change.

No time-zone database is here: matching a `TZID` to an IANA or Windows zone, and which zone an
event is shown in, are the caller's. So is what events and to-dos mean: an app reads them from
`ContentLine`s with these values and maps them onto its own model.

## Hostile input

Nothing panics on any text.

| Limit | Value |
|---|---|
| unfolded line (`lines::MAX_LINE`) | 1 MiB |
| parameters on a line, values of a parameter | 64 each |
| recurrence rule | 4096 bytes, 64 parts |
| durations | about 10,000 years |
| zones per text (`zones::MAX_ZONES`) | 64 |
| parts per zone, dates per part | 64 each |
| `TZID` kept (`zones::MAX_TZID`) | 128 bytes |
| offsets | at most 18 hours |
| work on zone definitions per text (`zones::WORK_BUDGET`) | 20,000,000 units, spent by the caller through `TimeZones::spend` |

Every date and time written is in the years 1 to 9999; a value outside them is moved to the
nearest end.

## Origin

MailCraft `crates/ical` (`lines.rs`, `values.rs`, `rrule.rs`, `zones.rs`) at
[a44a450](https://github.com/UCS-Care-Group/mailcraft/commit/a44a45013d1c6396229c40ab60172d1882b368d5),
by Martin Ball and the MailCraft contributors (MIT OR Apache-2.0), with their tests. MailCraft's
own date types are replaced by `chrono`, its time-zone database by the `ZoneRules` trait (the
writer's tests use rule zones built in the test), and its recurrence model by `Recurrence`.
MailCraft's event and to-do reader and writer (`read.rs`, `write.rs`), and the matching of a
`TZID` against its zone database, stay in MailCraft as the mapping onto its model.

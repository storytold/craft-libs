# craft-godot-pck

Read-only PCK 1/2/3 header, trailer and member-table parsing. No extraction, decryption, execution or filesystem access. Unknown layouts and encrypted tables remain explicitly unread.

Extracted from Craft Anything `crates/formats/src/godot_pck.rs`, originally by Brandon Thomas (`f64ae3bf`), including Afloat’s embedded-trailer boundary fix (`4824e31d`). App presentation and extraction policy remain in the consumer.

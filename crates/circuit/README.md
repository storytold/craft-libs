# craft-circuit

The electrical side of a schematic, kept apart from how it is drawn. The first consumer is
CADCraft's schematic tools (storytold/cadcraft#1516); a board editor can use the same model
and read the netlist.

An app builds a `Project`: its parts, and per sheet the pins of those parts at their sheet
positions, wires as polylines, junctions and labels. Drawing symbols and placing their pins
is the app's work. From a project the crate gives:

| Function | Result |
|---|---|
| `Connectivity::build(&project)` | nets: the pins, wires, junctions and labels that are connected, each with a unique name; wire ends that touch nothing; labels that sit on nothing |
| `Netlist::new(&project, &conn)` | parts and the pins on each net, in designator order; `to_json()` (format `craft-circuit-netlist`, version 1) and `to_csv()` |
| `bom(&project, &options)`, `bom_csv` | parts-list rows grouped by value, footprint and chosen fields, with quantity and designators |
| `erc::check(&project, &conn, &settings)` | connection findings, each with a stable rule id, a level and the items it is about |
| `natural_cmp` | the order designators and pin numbers sort in: `R2` before `R10` |

## Connection rules

Our own, written from what a schematic means:

- Points closer than `Project::tolerance` (default 1e-6 drawing units) meet.
- Every point of one wire is connected.
- A wire end, pin, junction or label that lies on a wire joins that wire.
- Two wires that cross, with neither ending on the other, stay apart unless a junction sits
  on both.
- Sheet labels with the same name join on their own sheet. Global and supply labels with the
  same name join across the project.

A net takes the name of its label, preferring a supply label, then a global one, then a sheet
one. When a sheet label's name is used on several sheets, the name is qualified with the sheet's
name. Nets with no label are numbered `N$1`, `N$2`, … in the order of their lowest designator.

## Connection checks

| Rule id | Default | Finds |
|---|---|---|
| `pin_conflict` | error or warning by pin pair | pins whose kinds should not share a net, such as two outputs |
| `undriven_input` | warning | inputs on a net that nothing drives |
| `unpowered_input` | error | supply inputs on a net with no supply output and no supply label |
| `unconnected_pin` | warning | a pin that is the only pin on its net |
| `not_connected_pin_connected` | error | a pin marked "not connected" joined to another pin |
| `dangling_wire` | warning | a wire end that touches nothing |
| `floating_label` | warning | a label on no wire, pin or junction |
| `duplicate_designator` | error | parts sharing a designator |
| `missing_designator` | error | a part with no designator, or one ending in `?` |
| `several_net_names` | warning | a net carrying several label names |
| `label_case_mismatch` | warning | label names that differ only in case |
| `global_label_used_once` | warning | a global label that joins nothing |

`erc::Settings` changes rule levels and pin-pair levels per project. `Rule::summary` is the
English text an app uses as its translation key; the crate holds no other user-facing text.

## Limits

`Project::from_json` reads at most 64 MiB, 10,000 sheets, 1,000,000 parts and 4,000,000 pins,
wires, wire points, junctions and labels together. Coordinates must be finite and no larger than 1e12.
Connection building stops with `Error::TooComplex` after 400,000,000 comparisons instead of
running on. `Limits` lowers or raises each bound.

Not yet: reading a netlist back, automatic numbering, cross-references between a part's units.

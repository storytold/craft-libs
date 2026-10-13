# Consumer dependency evidence

The [JSON inventory](consumer-dependency-map.json) lists applications to inspect. All entries
start **unverified**: this PR has not audited their current manifests, lockfiles or test
commands. It deliberately supplies no suite-wide pin, adopted crates or guessed package names.
Do not use this file as a release gate or claim all listed apps consume craft-libs.

When auditing a consumer, record its exact commit, manifest paths and dependency declarations,
the resolved craft-libs revision from its lockfile, used crates/features and source links.
Record a validated command only after checking that package and test target exist. Keep
baseline/candidate comparison results, nonzero test counts, environment and durable evidence
links with the audit. A candidate integration and a merged dependency pin are separate states.

Update only entries for which there is evidence. `null` and empty lists mean unknown, not
absent or passing. The file is ordinary inventory data, not a JSON Schema or runner config.
No compatibility runner consumes it in this revision.

Application upgrades are independent. A new library release does not update their manifests.
Validate affected internal dependents and actual consumers for a change, then propose
consumer pin changes separately when needed. Keep all craft-libs dependencies within one
consumer graph on the same revision. See the
[release policy proposal](https://github.com/storytold/craft-libs/pull/41), pending merge.

# Authority and Conflict Resolution

**Status:** Canonical repository doctrine  
**Applies to:** the `craft-libs` workspace and consuming Craft applications

## Resolution order

When repository artifacts disagree, the first applicable item controls:

1. `AUTHORITY.md` and ecosystem architecture doctrine;
2. `GOVERNANCE.md` and `AGENTS.md` for project policy and autonomous execution behavior;
3. Normative shared crate contracts (`docs/shared-crate-contract.md`, `docs/shared-services-architecture.md`);
4. Master UI elements map and typed component schemas (`docs/ui-elements-map.md`, `docs/ui-schema.md`);
5. Specialized engineering rules (`.agents/rules/*.md`);
6. Canonical documentation, release policy (`VERSIONING.md`), negative results (`docs/negative-results.md`), and crate READMEs (`crates/*/README.md`);
7. Executable tooling, test suites, and compatibility harnesses (`cargo xtask compat`);
8. Package manifests, release archives, and repository metadata.

Consuming applications bound to `craft-libs` retain their own internal authority over layout, documents, and commands. This repository does not silently rewrite application semantics.

## Conflict rule

A lower layer may expose a contradiction but cannot resolve it by changing the meaning of a higher layer. Record the conflict, preserve evidence, and either correct the lower layer or approve a normative change at the controlling layer.

Generated artifacts never become authority by convenience. Edit the canonical source and regenerate.

## Evidence

Evidence is orthogonal to precedence. New evidence may challenge doctrine and trigger review, but evidence does not silently mutate the definition it evaluates.

## Supersession

A normative replacement must identify the artifact it supersedes, compatibility impact, migration rule when required, and retained provenance. Historical releases remain immutable evidence.

# Vocab Veto

A stateless, single-binary Rust HTTP service that answers one question across
many languages: *"Does this string contain a banned word?"*

The repository directory and crate name remain `banned-words-service`; **Vocab
Veto** is the product name. The `BWS_*` environment-variable prefix is part of
the binary's stable identity and is unchanged.

## Status

Pre-code. The authoritative artifacts today are the design docs:

| Document                                             | Purpose                                                                                          |
| ---------------------------------------------------- | ------------------------------------------------------------------------------------------------ |
| [DESIGN.md](./DESIGN.md)                             | Behavior spec, API surface, error table, metrics contract, threat model, deployment posture.    |
| [IMPLEMENTATION_PLAN.md](./IMPLEMENTATION_PLAN.md)   | Milestone-ordered plan with explicit exit criteria per milestone.                                |
| [CLAUDE.md](./CLAUDE.md)                             | Agent-facing working notes and cross-cutting invariants.                                         |

Build, test, and lint commands will be added to this README as Milestone 1
lands them.

## What it does (in one paragraph)

One `Arc<AhoCorasick>` per language is built at startup from a word list
compiled into the binary at build time. Requests hit `POST /v1/check` with a
bearer API key; the service normalizes the input (NFKC + caseless), scans it
against the requested languages' automatons in a single pass, and returns
matches with byte spans that point back into the caller's original text. The
hot path is lock-free and bounded — no database, no disk, no hot reload. The
image tag is the list version.

## Word list attribution

The banned-words corpus is sourced from
**[LDNOOBW — List of Dirty, Naughty, Obscene, and Otherwise Bad Words](https://github.com/LDNOOBW/List-of-Dirty-Naughty-Obscene-and-Otherwise-Bad-Words)**
by Shutterstock and contributors. Vocab Veto vendors that repository as a git
submodule pinned at a specific commit SHA; that SHA is the list version
surfaced via the `X-List-Version` response header, the `/readyz` endpoint, and
the `bws_list_version_info` Prometheus metric.

All credit for the curation, translation, and ongoing maintenance of the word
list belongs to the LDNOOBW project and its contributors. Consult the upstream
repository for the canonical list and its licensing terms before redistributing
the corpus.

## License

This service's source code is licensed under the [MIT License](./LICENSE).
The MIT License covers the code in this repository only; it does **not** cover
the LDNOOBW word list, which is governed by its own upstream terms (see
attribution above).

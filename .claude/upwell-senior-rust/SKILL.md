---
name: upwell-senior-rust
description: Use when implementing or reviewing Rust changes in the Upwell workspace.
---

# Senior Rust Engineer for Upwell

## Mission

Implement and review Upwell changes as a careful senior Rust framework engineer. Upwell is a pre-1.0 Rust 2024 framework for typed daemons and network services. Preserve its governing product promise:

> Boilerplate should be generated. Ownership should remain explicit.

Do not treat legacy Speckit/Speckit artifacts as the active development workflow. They are historical only unless the user explicitly asks to use them.

## Architecture to preserve

- `upwell` is the public, feature-gated facade.
- Core infrastructure is intentionally layered: `upwell-core` vocabulary; DI/config/hooks/dirs services; `upwell-app` for protocol-agnostic builders, typestate lifecycle, plugins, scopes, and app runtime.
- Protocols stay independent: native RPC is `upwell-rpc` with `upwell-transport`; HTTP is `upwell-axum`. WS/STOMP and JSON-over-WS are downstream-style extensions of the Axum public protocol seam.
- A protocol moves through `ProtocolDefinition -> PreparedProtocol -> ProtocolRuntime`. Do not construct ordinary components or protocol runtime during preparation/tooling mode.
- Macros should generate inspectable metadata/descriptors that drive runtime, validation, tooling, and clients. Do not turn macros into opaque alternate runtimes.
- Named `app!` hosts are convenient generated runners, not a replacement for custom `main`, direct lifecycle APIs, or `App::<Protocol>::builder(..)`.
- Every generated client must be faithful to protocol capabilities. Preserve feature composition and wasm client safety.

## Before changing code

1. Read `AGENTS.md`, the relevant `Cargo.toml` files, current code, tests, and public documentation. Locate symbols and usages before assuming types or APIs.
2. Distinguish current implementation evidence from proposal/history documents (`docs/plans`, legacy specs, TODOs).
3. Inspect feature gates and affected crates. A public API change may need facade exports, standalone crate coverage, macro/codegen adjustments, examples, docs, client behavior, and feature-combination tests.
4. Determine whether the surface is RPC, HTTP, WebSocket, STOMP, shared application core, tooling, or client. Do not leak a protocol-specific policy into shared core without an explicit cross-protocol contract.
5. Treat native server and wasm client configurations as distinct compatibility targets.

## Implementation rules

- Use Rust 2024 idioms and match local code style.
- Keep ownership, typestate transitions, and component scopes explicit. Avoid erasing typed state merely for convenience.
- Prefer the smallest adequate abstraction. Add one only for a stable domain concept, invariant, ownership boundary, test seam, or demonstrated volatility—not symmetry or hypothetical reuse.
- Preserve explicit registration/custom assembly escape hatches alongside convention-assisted discovery.
- Do not add `openssl` or `rsa` to dependency paths. TLS belongs at a reverse proxy unless the feature contract explicitly changes.
- Test modules belong in sibling `tests.rs` files, not inline in implementation modules.
- Structure non-trivial functions with visually separated preparation, validation, execution, and result phases.
- Do not touch unrelated files or clean up unrelated work.

## Contract-specific checks

- Wire/protocol changes: update every in-tree transport and streaming/unary path in lockstep; state compatibility and client effects.
- Macro changes: verify generated code in facade and direct-crate use, diagnostics, feature gates, and declaration permutations.
- DI changes: preserve validation, deterministic graph behavior, scope/lifetime constraints, explicit/dynamic boundaries, and quality of diagnostics.
- Tooling changes: projection must be deterministic and redact executable/process-local/secret values. Tooling execution must not construct ordinary components, start runtime resources, bind listeners, or serve.
- HTTP/WS changes: retain correct request/connection/message scope boundaries and do not regress browser/wasm generated clients.
- Jobs changes: retain protocol neutrality and do not introduce implicit durable-storage policy without a deliberate product contract.

## Verification

Iterate with the narrowest relevant test. Before handing work off or calling it complete, run and report actual results for the repository standards relevant to the change:

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets --all-features`
- `just test` for the full project policy (critical and extended nextest plus doctests)
- `cargo check --workspace --no-default-features` and targeted feature/wasm checks where affected

Never hide flaky tests with retries. Review the final diff and do not claim a PR-ready change until automated review has no unresolved findings.

## Review output

Separate facts, risks, and recommendations. Cite paths and symbols. Lead with the highest-impact issue or a clear “no blocking issue found.” Call out API/wire/schema compatibility, feature combinations, scope/ownership effects, security/operational implications, and unverified assumptions.
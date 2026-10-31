# ds-water agent guide

Follow the workspace AGENTS.md. Before editing, read and claim on the
[live coding board](../ds-architecture/AGENTS-LIVE-CODING-MESSAGING.md).

## Invariants

- Every engineering decision lives in Rust here; hosts (CLI, Kernel, WASM,
  UI) only call it. No second algorithm in TypeScript.
- SI everywhere inside: m, mm for diameters, L/s for flows and demands. Unit
  conversion happens only at exchange edges (INP).
- Pure Rust dependencies only; nothing that compiles C.
- Solver changes keep the analytic tests (single pipe HW/DW, parallel split,
  PRV, check valve, PDA Wagner law, tank drain) exact; never loosen a
  tolerance to make a change pass.
- Shared crates from ds-network are consumed read-only by path; changes to
  them go through their owners on the board.

## Checks

`cargo test --workspace`; `cargo clippy --workspace --all-targets -- -D warnings`;
`cargo fmt --check` (this workspace only). Build targets go in a `ds-junk`
folder under ~/ds-builds, never /tmp. WASM builds run on magese-think.

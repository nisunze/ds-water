# ds-water

The Water vertical of Data Solutions: pressurised water-supply networks from a
village scheme to a whole country, headless first, one engine for the CLI, the
Desktop and the Web (WASM unit).

Water borrows the LV process network and the MV profile: it shares
`ds-graph` (oriented trees and paths) and `ds-terrain-sampling` (metric route
sampling) with ds-network by path, and follows the same Kernel command surface.

## Crates

| Crate | Owns |
| --- | --- |
| `ds-water-model` | Network model (SI), validation, EPANET INP import/export |
| `ds-water-hydraulics` | GGA solver (Todini-Pilati), EPS, pressure-driven demand, sparse Cholesky with minimum-degree ordering |
| `ds-water-synth` | Deterministic synthetic networks up to country scale |
| `apps/ds-water-cli` | `ds-water` headless binary |

Planned, in order: horizontal layout (`ds-water-layout`), vertical profile
(`ds-water-profile`), engineering design (`ds-water-design`: pipe sizing,
pressure classes, valves), drawings (`ds-water-drawing`), the WASM unit
(`ds-water-wasm`), then the `ds water` chapter and the Water tab.

## Headless use

```sh
ds-water validate net.inp
ds-water simulate net.json --eps --out results.json
ds-water convert net.inp net.json
ds-water bench --junctions 1000000
```

## Scale (ds-server, release build, 31 Oct 2026)

| Junctions | Links | Compile | Solve | Iterations |
| ---: | ---: | ---: | ---: | ---: |
| 101,232 | 131,416 | 122 ms | 54 ms | 6 |
| 1,001,664 | 1,300,293 | 2.3 s | 0.8 s | 6 |
| 5,001,216 | 6,492,283 | 15.9 s | 4.4 s | 6 |

Factor fill stays near 3 non-zeros per link: minimum degree eliminates the
branched village schemes first and leaves only the town loops to fill.

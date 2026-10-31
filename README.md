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
| `ds-water-layout` | Horizontal: GeoJSON to network (snap, T-split, demand allocation), automatic layout along roads or by MST, orientation and paths with `ds-graph` |
| `ds-water-design` | Engineering: ISO catalogue, pressure classes from static head + surge, cost-ordered sizing to pressure and velocity, BOQ |
| `ds-water-profile` | Vertical: ground (via `ds-terrain-sampling`), pipe, HGL and static lines; air valves, washouts, isolation valves, break-pressure tanks, findings |
| `ds-water-drawing` | Sheets: network plan, plan-and-profile with data bands, schedules (ISO A-size SVG, mm) |
| `ds-water-synth` | Deterministic synthetic networks up to country scale |
| `apps/ds-water-cli` | `ds-water` headless binary |

See [docs/water-engineering.md](docs/water-engineering.md) for the concepts (LV/MV ↔ water), Rwanda's demand standard and the sources.

Next: the WASM unit (`ds-water-wasm`), the `ds water` chapter through the
Kernel, the Water tab, PDF through the Reporter's renderer, pressure zoning.

## Headless use

```sh
ds-water validate net.inp
ds-water simulate net.json --eps --out results.json
ds-water convert net.inp net.json
ds-water bench --junctions 1000000
ds-water layout features.geojson --surface dem.asc --out net.json
ds-water design net.json --out designed.json --report design.json
ds-water profile designed.json --surface dem.asc --out profile.json
ds-water scheme fixtures/hill-villages/features.geojson --surface fixtures/hill-villages/dem.asc \
  --title fixtures/hill-villages/title.json --out-dir out/hill-villages
```

`scheme` runs the whole chain (layout → design → profiles → drawings) and
writes the network as JSON and INP beside the sheets.

## Scale (ds-server, release build, 31 Oct 2026)

| Junctions | Links | Compile | Solve | Iterations |
| ---: | ---: | ---: | ---: | ---: |
| 101,232 | 131,416 | 122 ms | 54 ms | 6 |
| 1,001,664 | 1,300,293 | 2.3 s | 0.8 s | 6 |
| 5,001,216 | 6,492,283 | 15.9 s | 4.4 s | 6 |

Factor fill stays near 3 non-zeros per link: minimum degree eliminates the
branched village schemes first and leaves only the town loops to fill.

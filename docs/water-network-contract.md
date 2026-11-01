# Water network contract

This contract owns what a Water network model is, the Water engine's operations, the design rules it applies, and how Water rides the DS Grid model lifecycle that LV and MV use. The standards behind the rules are in [water-engineering.md](water-engineering.md); the ordered roadmap is ds-cli `crates/kernel/ds-command-kernel/backlog/OPEN.md` §39.

## 1. The network

A network is one `ds.water.network/v1` document (`ds-water-model::WaterNetwork`):

- `name`, `crs` (the coordinate reference of node `x`/`y` and link vertices; layouts write `EPSG:4326`) and hydraulic `options`;
- nodes: `junctions` (elevation, base demand, pattern), `reservoirs` (head) and `tanks`;
- links: `pipes` (from, to, length, internal diameter, roughness, material, status), `pumps` and `valves` (PRV, PSV, FCV, check; break-pressure tanks are modelled as PRVs).

Units are SI inside the engine: m, mm for diameters, L/s for flows and demands. Unit conversion happens only at exchange edges (EPANET `.inp`).

## 2. The model: a DS Grid model of kind `water_network`

A saved network is a governed project grid model, kept exactly like an LV transformer:

- **Container.** The `.dsgrid` package (today's Arrow container; the GeoPackage container is paused). The ordinary typed model tables are present and empty, so every package passes the standard `.dsgrid` verifier.
- **Profile.** `ds.water_network_snapshot`, version 1, written by `ds-water-package`. It holds exactly two content-addressed assets: the descriptor `water-network-v1.json` (profile, object type `water_network`, model id, revision, the network asset's length, sha256 and its junction and pipe counts) and `water-network-v1.json.zst` (the network as one zstd frame of its compact JSON). The package's coordinate system is `urn:ds:application-snapshot:water-network-v1`.
- **Identity.** The catalog model id is `water_<slug of the name>`; the manifest model id is `water-network-<sha256 of the model id>`; the manifest revision is the saved revision, from 1. Packing is deterministic: the same network packs to the same bytes on the native addon and the WASM unit.
- **Admission.** Unpacking verifies the profile, the manifest identity, every asset's length and sha256, and that no other asset is present; the network is read back through `WaterNetwork`.
- **Catalog.** Brain's project grid catalog admits the kind `water_network`. A save creates a revision through the Kernel's `grid_publication` (the same governed path as LV): the bytes are uploaded content-addressed, and the revision is fenced on the head the save read. A save appends a revision to the head's version (`water-r<revision>`); a moved head refuses.
- **Local models.** A dropped `.dsgrid` whose members include a descriptor ending `/water-network-v1.json` is a Water model on the Model tab (the shared `local_models` surface), held on the machine and never saved to a project unless saved deliberately. The Water opener is not built yet.

## 3. Engine operations

Every host (the `ds-water` addon's `invoke`, the WASM unit's `water_invoke_json`, and through them the Kernel) calls `ds_water_ops::invoke(operation, request)` with one JSON request and gets one JSON answer.

| Operation | Request | Answer |
| --- | --- | --- |
| `identity` | none | engine, version, operations |
| `validate` | `network` | `ok`, `issues` |
| `simulate` | `network`, `eps` | `ok`, hydraulic `steps` |
| `analysis` | `network`, `criteria` | per junction peak pressure and drop, per pipe flow and velocity, against the criteria |
| `layout` | `features` (GeoJSON of springs, roads, standposts, households), `options`, `surface` | `network`, mode, paths, unserved points, notes |
| `process` | `network`, `customers` (GeoJSON points), `settings` | the network with service connections, its layers, connected and unconnected counts, fittings and their bill |
| `design` | `network`, `criteria`, `catalogue` | sized `network`, cost, bill of quantities, lowest pressure, infeasible and zoning counts, warnings |
| `profile` | `network`, `path` or `links`, `options`, `surface` | length, points (ground, pipe, hydraulic grade), segments, node marks, fittings, findings and the ground source |
| `draw` | `network`, `criteria`, sheet request | `sheets` (SVG), `profiles` |
| `scheme` | `features`, `layout`, `criteria`, `catalogue`, sheet request with `surface` | layout, design summary, `network`, `inp`, design report, sheets, profiles |
| `example` | `at` (`[lon, lat]`), `seed`, `households` (1..=2000) | a hypothetical village laid out and designed |
| `pack` | `network`, `model_id`, `revision` (≥ 1), `project` | `model_kind`, `profile`, `packed` (manifest id and revision, schema version, sha256, length), `dsgrid_base64` |
| `unpack` | `dsgrid_base64` | `model_id`, `revision`, `project`, `network` |

Refusals: an unknown operation names the known ones; a request that does not decode is `invalid request`; an engineering refusal carries the engine's message. The addon prints `{"ok": false, "error": …}` and exits 1. The Kernel maps them to `water_engine_refused`, `water_network_invalid` and `water_models_input_invalid`.

Ground reaches the engine only as a `surface` (a regular elevation grid the caller sampled); the engine never reads raster files. A layout without one sets missing elevations to 0 m and says so in its notes.

## 4. Design rules

The defaults are fields of `DesignCriteria`, `ProfileOptions` and `LayoutOptions`, overridable per request and meant to be seeded as project settings:

- residual pressure at peak where water is drawn ≥ 10 m; ≥ 0 m at junctions without demand;
- static pressure ≤ 80 m, zoned with a PRV (outlet 25 m) or break-pressure tanks (1.5 m depth);
- velocity ≤ 2.0 m/s, flagged below 0.3 m/s;
- surge allowance 20 % of static pressure when choosing pipe classes; internal diameter ≥ 20 mm;
- cover to crown 1.0 m on profiles (`ProfileOptions::cover_m`).

Demand follows the MININFRA rural water supply manual (water-engineering.md §2); the other values are engineering choices that a project specification overrides. Pipes are buried; the rule that derives cover from pipe class, diameter and location, with recorded overrides, is next on the roadmap.

## 5. Commands

| Command | What it does | Hosts |
| --- | --- | --- |
| `ds water models` | the project's live Water networks, newest change first, with the engineering items | Core, page (`waterModels`) |
| `ds water run` | one engine operation, on a request or on a saved network's head (`--model`) | Core addon, page Water unit |
| `ds water save` | run an operation and save the network it answers as the next revision (`--name` or `--model`) | Core addon, page Water unit |

The in-page command line runs the same words. The engineering menu's Design ▸ Water items (Lay out, Design & zone, Simulate, Profile, Drawings) call these capabilities; the Kernel greys an item with its reason when the project holds no network.

## 6. Reference results

`apps/ds-water-cli/src/golden_tests.rs` pins the engine on three cases; a change that moves them is a deliberate engineering change:

| Case | Network | Design |
| --- | --- | --- |
| Example village, seed 1 at 29.70, −2.75 | 214 junctions, 214 pipes, 6,530.5 m | cost 68,521.58; lowest 12.83 m at S4 |
| Example village, seed 5 at 29.85, −2.60 | 220 junctions, 219 pipes, 7,558.6 m | not feasible: 5 junctions above their source, lowest −2.44 m at S4 |
| Gisovu hill villages on `fixtures/hill-villages/dem.asc` | 270 junctions, 265 pipes, 5 zone valves, 10,607.3 m | cost 118,616.44; lowest 11.73 m at J187; 11 sheets, 3 profiles |

## 7. What Water reuses

Water carries its hydraulic engine and its menu items; everything else is LV's and MV's:

- the `.dsgrid` container (`ds-grid-exchange`), the project grid catalog and the governed publication (`grid_publication`);
- `ds-graph` (oriented trees from the sources) and `ds-terrain-sampling` (metric route sampling);
- the engineering menu, the in-page command line, the Model tab's `local_models`, and component installation and updates (native addon and WASM unit).

Still to plug in, by reusing rather than forking: tiles and Style Center styling, terrain seeding, the MV profile view, Reporter printing (plan and profile sheets with materials), Sync Center, and exchange with other water software.

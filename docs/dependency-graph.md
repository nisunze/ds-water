# Water dependency graph

Owner, 31 Oct 2026: "build a wise dependency graph — we have everything that
we need", and "for profile we will use LV semantics most of the time". Water
is composed from what LV, MV, Kernel, Core and the Reporter already own.
Water itself owns only what is water physics: hydraulics, zoning, the pipe
catalogue and sizing. Everything else is a consumer edge to an existing owner.

```mermaid
graph TD
  subgraph ds-water [ds-water: water physics only]
    model[ds-water-model<br/>network, INP]
    hyd[ds-water-hydraulics<br/>GGA, sparse Cholesky]
    layout[ds-water-layout<br/>horizontal]
    process[ds-water-process<br/>customers, fittings]
    design[ds-water-design<br/>zoning, sizing]
    profile[ds-water-profile<br/>vertical]
    drawing[ds-water-drawing]
    ops[ds-water-ops<br/>one invoke]
    wasm[ds-water-wasm unit]
  end
  subgraph ds-network [ds-network: LV and MV]
    graph[ds-graph<br/>oriented tree, paths]
    sampling[ds-terrain-sampling]
    lvproc[LV processing<br/>customer sources, connection rules]
    vd[LV voltage drop<br/>radial path results, vd_ columns]
  end
  subgraph ds-cli [ds-cli: Kernel and Core]
    pgrid[ds-profile-grid<br/>profile drawing vocabulary]
    geolite[ds-geo-lite<br/>projection, index]
    kernel[Kernel capabilities<br/>water.*, routing]
    core[Core native host]
    terrain[existing raster / terrain layers<br/>the ground plugin]
  end
  reporter[ds-network-reporter<br/>print_layout, PDF]
  brain[ds-brain<br/>project component water]
  web[ds-web<br/>Water tab, thin]

  layout --> graph
  profile --> sampling
  process -. contract .-> lvproc
  profile -. semantics .-> vd
  drawing -. next .-> pgrid
  layout -. next .-> geolite
  ops --> layout & process & design & profile & drawing & hyd
  design --> hyd
  wasm --> ops
  kernel --> ops
  core --> ops
  terrain -->|GridSurface| ops
  drawing -. PDF next .-> reporter
  brain --> kernel
  web --> kernel
```

Solid arrows are in the code today. Dotted arrows are adopted contracts or
the next slices.

## Edges and their status

| Water needs | Comes from | Status |
| --- | --- | --- |
| Paths source → tip, trunk "01" first | `ds-graph` (LV/MV) | **Used**: layout, design paths, profile and sheets |
| Ground along a route | `ds-terrain-sampling` (MV profile) | **Used** in the profile |
| Ground surface itself | Existing raster/terrain layers, as a plugin | **Contract**: `GridSurface` in ops requests; no raster code in Water |
| Customer connection | LV processing | **Contract adopted**: same customer sources, `connect_unconnected_customers`, `unconnected_customers` with `network_properties.unconnected_reason` |
| Path results ("profile") | LV voltage drop (`vd_*` columns per path, limits) | **Semantics adopted** (owner): pressure drop and residual pressure per path, like voltage drop. Next: `pd_*` / `p_*` columns on the processed layers, with the same per-path summary LV shows |
| Profile drawing axes and ticks | `ds-profile-grid` (Kernel) | Next: replace Water's own tick choice |
| Projection and spatial index | `ds-geo-lite` (`projection`, `index`) | Next: replace Water's small equirectangular and grid |
| Command surface on every platform | Kernel capabilities + Core | Next: `water.*` rows routed to Core (CLI, Desktop) and the Water unit (Web), all calling `ds_water_ops::invoke` |
| Browser compute | Unit install/activation like Solar/Network | Next: The main adds a Water unit leg (built on think) |
| Printing and PDF | Reporter `print_layout` | Next: Water sheets as Reporter print documents |
| Project capability | Brain `project_components.water` (opt-in) | **Landed**: ds-brain 24977642, ds-web c2c4bfb4b |
| UI | ds-web Water tab beside LV, MV, PM | Next, thin: it renders Kernel answers |

## What Water deliberately owns

- **Hydraulics.** Pressure flow is not a radial electrical sweep: loops,
  tanks, pumps and valve regimes need the Global Gradient Algorithm. The LV
  voltage-drop solver stays electrical; Water shares its path semantics, not
  its numerics.
- **Zoning, the pipe catalogue and sizing.** These are water-specific
  engineering rules (`docs/water-engineering.md`).

## Blocked edge

ds-cli cannot depend on `ds-water` by path until `ds-water` has its GitHub
remote and think-sync. Release legs build from those checkouts, so the
Kernel and Core edges wait for the owner's repository step.

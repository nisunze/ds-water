# Water engineering reference

What the Water vertical models, why, and where each rule comes from. Written
for engineers and agents picking the vertical up; the code cites this file.

## 1. From electricity to water: the same shape, different physics

Water reuses the LV process and the MV profile. The concepts map like this:

| Electrical (LV / MV) | Water | Notes |
| --- | --- | --- |
| Substation / plant | **Source**: spring catchment, borehole, river intake, treatment works | A fixed head (reservoir in the model) |
| MV feeder | **Transmission main** | Carries water between points with *no* service connections; often gravity from a spring to a reservoir |
| MV/LV transformer | **Service reservoir / storage tank** | Fixes the head of the zone below it and buffers the daily demand curve; the equivalent "service point" of a zone |
| LV network | **Distribution network** | Branched in rural schemes, looped in towns (reliability) |
| Customer connection | **Service connection, standpost (WPT), kiosk, institution** | Demand points |
| Voltage drop limit | **Minimum residual pressure** | Too little head, no water at the tap |
| Insulation / equipment rating | **Pressure class (PN)** | Static head + surge allowance must stay under the pipe's rating |
| Voltage regulator / tap changer | **PRV, break-pressure tank** | Reset the static line to define a **pressure zone** |
| Switchgear | **Isolation valve** | At tees and at a maximum spacing |
| (no equivalent) | **Air valve, washout** | Air gathers at summits and throttles gravity mains; sags need draining |
| Feeder paths "01", "02" | Same paths, oriented from the sources with `ds-graph` | Trunk first, so profiles and sheets follow the LV/MV convention |
| Conductor sizing | **Pipe sizing** | Discrete catalogue, cost against head loss |
| Load flow | **Hydraulic solve** (GGA) | Heads ↔ voltages, flows ↔ currents; head loss is non-linear (Q^1.852) |

Urban networks are also cut into **district metered areas** (DMAs, roughly
4–8 km² with one metered inlet, fed directly from the transmission main) for
leakage management; a later slice will form DMAs the way LV forms transformer
areas.

## 2. Rwanda's demand standard

MININFRA, *Manual for Rural Water Supply Project Cycle Management*, Table 1-3
"Standard of Daily Supply Volume" (itself from GTZ, *Study Manual for
Gravity-Fed Systems*, 1996). Built into `ds-water-layout::demand`:

| Consumer | Demand |
| --- | --- |
| Water point (WPT), agglomeration habitat | 20 l/day/capita |
| Urban habitat | 80 l/day/capita |
| Cattle / goats and sheep | 40 / 5 l/head/day |
| Hospital | 150 l/bed/day |
| Health centre | 6–10 m³/day (8 used) |
| Primary / high / boarding school | 5 / 15 / 40 l/student/day |
| Youth centre, orphanage, parish, prison, military camp | 50, 30, 80, 70, 100 l/person/day |
| District / sector office, court | 500 / 100 / 200 l/day |
| Trading centre, market | 120 / 1,000 l/day |
| Slaughter house | 350 l/animal/day |
| Tea factory | 5 m³/ton of tea |

The National Water Supply Policy (MININFRA, 2016) sets the same floor: at
least 20 l/person/day rural and 80 urban. It also records that actual rural
consumption is far lower (about 5–8 l/c/d). Design to the standard and report
the measured demand next to it.

## 3. Design criteria and their status

| Criterion | Default | Status |
| --- | --- | --- |
| Demand | Table 1-3 above | **National standard** |
| Peak factor | 2.0 on average daily demand | Engineering choice: the manual gives none; standposts drawn over about 12 hours a day |
| Minimum residual pressure where water is drawn | 10 m | Engineering choice, common rural practice; set per project |
| Junctions without demand | ≥ 0 m | Engineering choice (no sub-atmospheric pressure) |
| Maximum static pressure | 80 m | Engineering choice; CPHEEO (India) uses 70 m and zones above it with PRVs |
| Zone device | PRV at minimum + 15 m (25 m); break-pressure tank on transmission mains | Common practice; selectable |
| Velocity | ≤ 2.0 m/s, flagged < 0.3 m/s | Engineering choice |
| Surge allowance on class | +20 % of static | Engineering choice |
| Cover to crown | 1.0 m | Engineering choice |
| Isolation valve spacing | 1,000 m and every tee | Engineering choice |

Every "engineering choice" is a field of `DesignCriteria`, `ProfileOptions`
or `LayoutOptions` and belongs in the project template's seeds, not in code.
A project with a WASAC specification overrides them.

## 4. Algorithms and the literature

- **Hydraulics**: the Global Gradient Algorithm of Todini and Pilati (1988),
  as in EPANET 2.2, including its status rules for check valves, pumps and
  PRV/PSV/FCV and its pressure-driven demand formulation. Linear algebra is a
  minimum-degree-ordered sparse Cholesky, analysed once per network; a
  5-million-junction synthetic country solves in seconds.
- **Pipe sizing**: optimal design of branched networks is the Alperovits and
  Shamir (1977) linear-programming gradient line, extended by Kessler and
  Shamir and by Fujiwara and Khang. LP suits branched networks but yields
  split-pipe solutions; evolutionary methods linked to an EPANET-type solver
  (genetic algorithms, and GA-LP hybrids such as Cisty 2010, which decomposes
  loops into branched sub-networks) are the common alternative. Even the
  single-diameter branched problem is NP-hard. The engine today uses a
  cost-gradient heuristic (upgrade the pipe on the critical supply path that
  buys most head per unit cost, then an economy pass), behind a request and
  result that an LP or GA method can replace.
- **Pressure zoning**: walk from the sources and insert a PRV or
  break-pressure tank where static pressure would pass 90 % of the limit,
  above the next consumer; both are modelled as PRVs, as EPANET modellers do.

## 5. What is next

1. Zoning samples the ground surface between nodes (today it interpolates
   node elevations; the profile already samples it).
2. Storage: service reservoir sizing from the demand pattern (balancing
   volume), and tank placement as the "transformer" of each zone.
3. DMA formation for towns.
4. LP sizing for branched schemes; GA for looped towns.

The Kernel chapter, the units and the DS Grid model are in place; what a
network model is and how it is saved is in
[water-network-contract.md](water-network-contract.md), and the ordered
roadmap is ds-cli OPEN §39.

## Sources

- MININFRA, [Manual for Rural Water Supply Project Cycle Management](https://www.mininfra.gov.rw/fileadmin/user_upload/Mininfra/Documents/Water_and_Sanitation_docs/10_Manual_for_Rural_Water_Supply_Project_Cycle.pdf), Table 1-3.
- MININFRA, [National Water Supply Policy and Implementation Strategy (2016)](https://www.mininfra.gov.rw/fileadmin/user_upload/Mininfra/Documents/Water_and_Sanitation_docs/NATIONAL_WATER_SUPPLY_POLICY_IMPEMENTATION_STRATEGY_DECEMBER_2016.pdf).
- CPHEEO, [Manual on Water Supply, distribution](https://infralens.in/cpheeo/cpheeo-ws-11) (70 m maximum, PRV zoning, DMAs 4–8 km²; secondary summary).
- Ferrari, Savić, Becciu, [Graph-theoretic approach and sound engineering principles for design of district metered areas](https://re.public.polimi.it/bitstream/11311/763721/3/%2528asce%2529wr%252E1943-5452%252E0000424.pdf), ASCE JWRPM.
- US Army TM 5-813-5, [Water Supply, Water Distribution](https://www.scribd.com/document/524266452/USACE3-Water-Distribution) (transmission mains have no service connections).
- Alperovits and Shamir, [Design of Optimal Water Distribution Systems (1977)](https://shamir.net.technion.ac.il/files/2012/04/1977-Design-of-Optimal-Water-Distribution-Systems.pdf).
- [Coupled Genetic Algorithm–Linear Programming Scheme for Least-Cost Pipe Sizing](https://ascelibrary.com/doi/10.1061/%28ASCE%290733-9496%282009%29135%3A4%28298%29), ASCE JWRPM 2009.

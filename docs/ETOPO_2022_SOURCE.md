# NOAA ETOPO 2022 source

The optional `--etopo` baker path uses NOAA's whole-world **ETOPO 2022 Ice Surface,
60 arc-second** elevation grid as the macro terrain source.

- Product: ETOPO 2022 Ice Surface elevation, 60 arc-second GeoTIFF
- Coverage: 21600 x 10800 pixel-centred geographic grid, north-up
- Elevation reference: EGM2008; metres; signed land topography and bathymetry
- DOI: <https://doi.org/10.25921/fd45-gt74>
- Product page: <https://www.ncei.noaa.gov/products/etopo-global-relief-model>
- Download:
  <https://www.ngdc.noaa.gov/mgg/global/relief/ETOPO2022/data/60s/60s_surface_elev_gtif/ETOPO_2022_v1_60s_N90W180_surface.tif>
- Retrieved: 1 August 2026
- File size: 465,969,062 bytes
- SHA-256: `9d27d4b8ea8e76977e2988bca667d7c8fa68b927355feffcddd6b4875a7fd08e`

NOAA's metadata states that ETOPO 2022 is not subject to copyright protection
within the United States and asks users to cite the dataset. It must not be used
for navigation. The source GeoTIFF is deliberately ignored under
`assets/source-data/`; only this provenance record and importer are committed.

The importer resamples ETOPO onto the configured south-up working grid, clamps
it to the planet's existing -5,000m to +9,000m raw height contract, and preserves
the observed heights. Bilinear samples retain coastline, ordinary hills, and
valleys. To stop narrow major summits disappearing between coarse working-grid
sample points, peak retention fades in from 4,000m and reaches the highest
observed source elevation in each target footprint at 6,000m. Restricting the
max envelope to those high ranges avoids terracing lower land. The importer
derives flow, river masks, lakes, moisture, and biomes, but skips the authored
generator's synthetic hydraulic/thermal erosion and river/glacier height
carving. Sparse seam-safe detail export and the runtime procedural ladder remain
unchanged.

Reproduce the active source download:

```bash
mkdir -p assets/source-data/etopo-2022
curl --fail --location --continue-at - \
  --output assets/source-data/etopo-2022/ETOPO_2022_v1_60s_N90W180_surface.tif \
  https://www.ngdc.noaa.gov/mgg/global/relief/ETOPO2022/data/60s/60s_surface_elev_gtif/ETOPO_2022_v1_60s_N90W180_surface.tif
sha256sum assets/source-data/etopo-2022/ETOPO_2022_v1_60s_N90W180_surface.tif
```

Bake to staging; never write a new source directly over the active outmap:

```bash
RAYON_NUM_THREADS=1 nice -n 10 \
  /home/dad/catingard-target/release/catinthegarden-baker \
  --output assets/outmaps/test-planet.etopo-staging-YYYYMMDD-HHMMSS \
  --etopo assets/source-data/etopo-2022/ETOPO_2022_v1_60s_N90W180_surface.tif \
  --width 4096 --height 2048 --dense-level 4 --max-level 18
```

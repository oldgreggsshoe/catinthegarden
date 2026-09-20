#!/bin/bash
# Is terrain fragment-bound or geometry-bound? Resolution scales fragment work;
# the chunk budget scales geometry. Same blocked design.
cd /home/dad/catingard
OUT=/tmp/claude-1000/-home-dad-catingard/4746fa67-0ac5-44b9-a38e-e000392f4542/scratchpad/terrain.jsonl
: > $OUT
for w in 1 2; do xvfb-run -a ./target/release/catinthegarden-app --scenario village_pov >/dev/null 2>&1; done
run() { # label, env assignments
  local label="$1"; shift
  env "$@" xvfb-run -a ./target/release/catinthegarden-app --scenario village_pov >/dev/null 2>&1
  local d=$(ls -dt test-runs/village_pov/* | head -1)
  python3 - "$label" "$d" >> $OUT <<'PY'
import json, sys, statistics
label, run = sys.argv[1], sys.argv[2]
rows=[json.loads(l)['fields'] for l in open(f'{run}/log.jsonl') if 'frame_time_ms' in l]
fts=[r['frame_time_ms'] for r in rows][1:]
tris=[r.get('terrain_triangles',0) for r in rows][1:]
chunks=[r.get('drawn_chunks',0) for r in rows][1:]
print(json.dumps({'label': label, 'median': statistics.median(fts),
                  'triangles': statistics.median(tris) if tris else 0,
                  'chunks': statistics.median(chunks) if chunks else 0}))
PY
}
for b in 1 2 3; do
  run "baseline_720p"        CATINGARDEN_VIEWPORT=1280x720
  run "half_res_360p"        CATINGARDEN_VIEWPORT=640x360
  run "quarter_res_180p"     CATINGARDEN_VIEWPORT=320x180
  run "budget_64_chunks"     CATINGARDEN_VIEWPORT=1280x720 CATINGARDEN_MAX_ACTIVE_CHUNKS=64
  run "budget_1024_chunks"   CATINGARDEN_VIEWPORT=1280x720 CATINGARDEN_MAX_ACTIVE_CHUNKS=1024
  run "no_terrain_720p"      CATINGARDEN_VIEWPORT=1280x720 CATINGARDEN_DISABLE=terrain
done
echo DONE >> $OUT

#!/bin/bash
# Does chunk count cost fragments or vertices?
#
# At 720p, budget 64 -> 19.0ms and budget 1024 -> 150.8ms: a 131.8ms spread.
# If that spread is fragment work (overdraw -- the same pixels shaded by several
# chunks) it must shrink with the pixel count. If it is vertex/draw work it will
# not care about resolution at all. Run the same budgets at a sixteenth of the
# pixels and see which happens.
cd /home/dad/catingard
OUT=/tmp/claude-1000/-home-dad-catingard/4746fa67-0ac5-44b9-a38e-e000392f4542/scratchpad/overdraw.jsonl
: > $OUT
for w in 1 2; do xvfb-run -a ./target/release/catinthegarden-app --scenario village_pov >/dev/null 2>&1; done
run() {
  env CATINGARDEN_VIEWPORT="$2" CATINGARDEN_MAX_ACTIVE_CHUNKS="$3" \
    xvfb-run -a ./target/release/catinthegarden-app --scenario village_pov >/dev/null 2>&1
  local d=$(ls -dt test-runs/village_pov/* | head -1)
  python3 - "$1" "$d" >> $OUT <<'PY'
import json, sys, statistics
label, run = sys.argv[1], sys.argv[2]
rows=[json.loads(l)['fields'] for l in open(f'{run}/log.jsonl') if 'frame_time_ms' in l]
fts=[r['frame_time_ms'] for r in rows][1:]
print(json.dumps({'label': label, 'median': statistics.median(fts),
                  'triangles': statistics.median([r.get('terrain_triangles',0) for r in rows][1:]),
                  'chunks': statistics.median([r.get('drawn_chunks',0) for r in rows][1:])}))
PY
}
for b in 1 2 3; do
  run "720p_b64"    1280x720  64
  run "720p_b1024"  1280x720  1024
  run "180p_b64"    320x180   64
  run "180p_b1024"  320x180   1024
done
echo DONE >> $OUT

#!/bin/bash
# Marginal costs do not sum to the frame. Establish the fixed floor -- the cost
# with nothing in the scene at all -- so terrain can be measured against that
# instead of against a frame that still pays for full-screen sky.
cd /home/dad/catingard
OUT=/tmp/claude-1000/-home-dad-catingard/4746fa67-0ac5-44b9-a38e-e000392f4542/scratchpad/floor.jsonl
: > $OUT
ALL="terrain,ocean,sky,stars,clouds,cloud_impostors,rain,forest,villages,birds,ship"
for w in 1 2; do xvfb-run -a ./target/release/catinthegarden-app --scenario village_pov >/dev/null 2>&1; done
run() {
  CATINGARDEN_DISABLE="$2" xvfb-run -a ./target/release/catinthegarden-app --scenario village_pov >/dev/null 2>&1
  local d=$(ls -dt test-runs/village_pov/* | head -1)
  python3 - "$1" "$d" >> $OUT <<'PY'
import json, sys, statistics
label, run = sys.argv[1], sys.argv[2]
fts=[json.loads(l)['fields']['frame_time_ms'] for l in open(f'{run}/log.jsonl') if 'frame_time_ms' in l][1:]
print(json.dumps({'label': label, 'median': statistics.median(fts)}))
PY
}
for b in 1 2 3; do
  run "baseline"            ""
  run "no_terrain"          "terrain"
  run "no_terrain_no_sky"   "terrain,sky"
  run "nothing_at_all"      "$ALL"
  run "only_terrain"        "ocean,sky,stars,clouds,cloud_impostors,rain,forest,villages,birds,ship"
done
echo DONE >> $OUT

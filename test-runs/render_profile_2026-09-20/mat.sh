#!/bin/bash
cd /home/dad/catingard
OUT=/tmp/claude-1000/-home-dad-catingard/4746fa67-0ac5-44b9-a38e-e000392f4542/scratchpad/mat.jsonl
: > $OUT
ALL="material,tint,cloudshadow,skylight,weather,normals,detail,aerial,fog"
for w in 1 2; do xvfb-run -a ./target/release/catinthegarden-app --scenario village_pov >/dev/null 2>&1; done
run() {
  CATINGARDEN_ABLATE="$2" xvfb-run -a ./target/release/catinthegarden-app --scenario village_pov >/dev/null 2>&1
  local d=$(ls -dt test-runs/village_pov/* | head -1)
  python3 - "$1" "$d" >> $OUT <<'PY'
import json, sys, statistics
label, run = sys.argv[1], sys.argv[2]
fts=[json.loads(l)['fields']['frame_time_ms'] for l in open(f'{run}/log.jsonl') if 'frame_time_ms' in l][1:]
print(json.dumps({'label': label, 'median': statistics.median(fts)}))
PY
}
for b in 1 2 3 4; do
  run "baseline"      ""
  run "no_material"   "material"
  run "no_tint"       "tint"
  run "no_both"       "material,tint"
  run "no_everything" "$ALL"
done
echo DONE >> $OUT

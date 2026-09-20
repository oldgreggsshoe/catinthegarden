#!/bin/bash
cd /home/dad/catingard
S=/tmp/claude-1000/-home-dad-catingard/4746fa67-0ac5-44b9-a38e-e000392f4542/scratchpad
OUT=$S/cloudshadow.jsonl
: > $OUT
for m in off on; do
  if [ $m = on ]; then export CATINGARDEN_CLOUD_SHADOW=1; else unset CATINGARDEN_CLOUD_SHADOW; fi
  xvfb-run -a ./target/release/catinthegarden-app --scenario weather_contrast >/dev/null 2>&1
  d=$(ls -dt test-runs/weather_contrast/* | head -1)
  for c in $d/screenshots/capture-*.png; do
    cp "$c" "$S/cs_${m}_$(basename $c)"
  done
done
unset CATINGARDEN_CLOUD_SHADOW
for w in 1 2; do xvfb-run -a ./target/release/catinthegarden-app --scenario village_pov >/dev/null 2>&1; done
for b in 1 2 3 4; do
  for m in off on; do
    if [ $m = on ]; then export CATINGARDEN_CLOUD_SHADOW=1; else unset CATINGARDEN_CLOUD_SHADOW; fi
    xvfb-run -a ./target/release/catinthegarden-app --scenario village_pov >/dev/null 2>&1
    d=$(ls -dt test-runs/village_pov/* | head -1)
    python3 - "$m" "$d" >> $OUT <<'PY'
import json, sys, statistics
label, run = sys.argv[1], sys.argv[2]
fts=[json.loads(l)['fields']['frame_time_ms'] for l in open(f'{run}/log.jsonl') if 'frame_time_ms' in l][1:]
print(json.dumps({'label': label, 'median': statistics.median(fts), 'n': len(fts)}))
PY
  done
done
unset CATINGARDEN_CLOUD_SHADOW
echo DONE >> $OUT

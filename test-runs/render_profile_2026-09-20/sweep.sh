#!/bin/bash
# Matched A/B sweep. Each block runs every condition once, so drift within a
# block hits all conditions alike; conditions are compared inside blocks.
cd /home/dad/catingard
OUT=/tmp/claude-1000/-home-dad-catingard/4746fa67-0ac5-44b9-a38e-e000392f4542/scratchpad/sweep.jsonl
: > $OUT
SCENARIO=village_pov
CONDS=("" "terrain" "ocean" "sky" "stars" "clouds" "cloud_impostors" "rain" "forest" "villages" "birds" "ship")
BLOCKS=4

# Warm-up: the Quadro idles at 135MHz and takes a while to boost. Two runs
# discarded before anything is recorded.
for w in 1 2; do
  xvfb-run -a ./target/release/catinthegarden-app --scenario $SCENARIO >/dev/null 2>&1
done

for b in $(seq 1 $BLOCKS); do
  for c in "${CONDS[@]}"; do
    clk=$(nvidia-smi --query-gpu=clocks.current.graphics --format=csv,noheader,nounits 2>/dev/null | head -1)
    CATINGARDEN_DISABLE="$c" xvfb-run -a ./target/release/catinthegarden-app --scenario $SCENARIO >/dev/null 2>&1
    d=$(ls -dt test-runs/$SCENARIO/* | head -1)
    python3 - "$b" "$c" "$d" "$clk" >> $OUT <<'PY'
import json, sys, statistics
block, cond, run, clk = sys.argv[1], sys.argv[2], sys.argv[3], sys.argv[4]
fts = [json.loads(l)['fields']['frame_time_ms'] for l in open(f'{run}/log.jsonl')
       if 'frame_time_ms' in l]
# The first sample of a run includes streaming and pipeline warm-up.
fts = fts[1:]
print(json.dumps({'block': int(block), 'cond': cond or 'baseline',
                  'median': statistics.median(fts), 'n': len(fts),
                  'clock_mhz': clk}))
PY
  done
done
echo DONE >> $OUT

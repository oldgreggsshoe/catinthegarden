#!/usr/bin/env bash
set -euo pipefail
cd /home/dad/catingard
label=${1:?label}
mode=${2:?mode: 0/on/mask}
scenario=${3:-alpine_survey_8_directions}
marker=$(mktemp /tmp/catingard-glacier-detail-marker.XXXXXX)
env -u DISPLAY CATINGARDEN_PRESENT_MODE=immediate CATINGARDEN_GLACIER_DETAIL="$mode" \
    timeout 600s xvfb-run -a --server-args='-screen 0 1920x1080x24' \
    ./target-glacier-detail/release/catinthegarden-app \
    --outmap /home/dad/catingard/assets/outmaps/test-planet --scenario "$scenario" \
    > "/tmp/catingard-glacier-detail-${label}.log" 2>&1
python3 - "$label" "$mode" "$scenario" "$marker" <<'PY'
import json,pathlib,statistics,sys
label,mode,scenario,marker=sys.argv[1:]
root=pathlib.Path('test-runs')
runs=[p for p in (root/scenario).iterdir() if p.is_dir() and p.stat().st_mtime>=pathlib.Path(marker).stat().st_mtime]
assert len(runs)==1,runs
p=runs[0];m=json.loads((p/'manifest.json').read_text());assert m['passed'] is True,m['failure_reasons']
v=[json.loads(s)['fields']['frame_time_ms'] for s in (p/'log.jsonl').read_text().splitlines() if '"frame_time_ms"' in s]
f=root/'peak_judging_2026-09-15/glacier-detail-trial/runs.json';d=json.loads(f.read_text()) if f.exists() else {}
d[label]=dict(mode=mode,run=str(p),median_ms=statistics.median(v),frames=len(v));f.write_text(json.dumps(d,indent=2)+'\n');print(label,d[label],flush=True)
PY

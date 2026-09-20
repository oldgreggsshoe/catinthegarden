#!/usr/bin/env python3
"""Matched replay, with explicit rejection of concurrent build/GPU work."""
import hashlib,json,os,pathlib,statistics,subprocess,sys,time
root=pathlib.Path('/home/dad/catingard');os.chdir(root)
label,mode=sys.argv[1:3];scenario=sys.argv[3] if len(sys.argv)>3 else 'alpine_survey_8_directions'
assert mode in ('0','floor','grain','both','light','contrast','mesh','mesh-light','snow-grain','snow-mesh-light')
evidence=root/'test-runs/peak_judging_2026-09-15/alpine-material-trial'
index=evidence/'runs.json';runs=json.loads(index.read_text()) if index.exists() else {};assert label not in runs
runroot=root/'test-runs'/scenario;before=set(runroot.iterdir());binary=root/'target-glacier-detail/release/catinthegarden-app'
sha=hashlib.sha256(binary.read_bytes()).hexdigest();env=dict(os.environ,CATINGARDEN_PRESENT_MODE='immediate',CATINGARDEN_ALPINE_MATERIAL_TRIAL=mode);env.pop('DISPLAY',None)
observations=[]
with (evidence/(label+'.console.txt')).open('w') as log:
 proc=subprocess.Popen(['timeout','600s','xvfb-run','-a',"--server-args=-screen 0 1920x1080x24",str(binary),'--outmap',str(root/'assets/outmaps/test-planet'),'--scenario',scenario],env=env,stdout=log,stderr=subprocess.STDOUT)
 while proc.poll() is None:
  others=[]
  for entry in pathlib.Path('/proc').iterdir():
   if not entry.name.isdigit():continue
   try:exe=os.readlink(entry/'exe')
   except OSError:continue
   if exe.endswith(('/catinthegarden-app','/rustc','/cargo')) and '/target-glacier-detail/' not in exe:
    others.append(dict(pid=int(entry.name),exe=exe))
  if others:observations.append(dict(time=time.time(),processes=others))
  time.sleep(1)
 assert proc.returncode==0,proc.returncode
new=[p for p in set(runroot.iterdir())-before if p.is_dir()];assert len(new)==1,new
run=new[0];manifest=json.loads((run/'manifest.json').read_text());assert manifest['passed'],manifest['failure_reasons']
values=[r['fields']['frame_time_ms'] for line in (run/'log.jsonl').read_text().splitlines() if 'frame_time_ms' in (r:=json.loads(line)).get('fields',{})]
record=dict(mode=mode,run=str(run.relative_to(root)),median_ms=statistics.median(values),frames=len(values),binary_sha256=sha,performance_eligible=not observations,contaminated_samples=len(observations))
(evidence/(label+'.concurrency.json')).write_text(json.dumps(observations,indent=2)+'\n');runs[label]=record;index.write_text(json.dumps(runs,indent=2)+'\n');print(label,json.dumps(record),flush=True)

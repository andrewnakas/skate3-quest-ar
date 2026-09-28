"""One isolated map conversion worker for source and frozen setup builds."""
from pathlib import Path
import argparse,json,sys,time
sys.path.insert(0,str(Path(__file__).resolve().parents[2]))
from tools.asset_pipeline.install import convert_map

if __name__=='__main__':
    p=argparse.ArgumentParser()
    for name in ('archive','stage','game-exe','result'):p.add_argument('--'+name,type=Path,required=True)
    a=p.parse_args();start=time.perf_counter()
    with (a.stage/(a.archive.stem+'-load.log')).open('w',encoding='utf-8') as log:
        entry=convert_map(a.archive,a.stage/'conversion',a.stage/'maps',a.stage,a.game_exe,log,lambda s:print(s,flush=True))
    entry['conversion_seconds']=round(time.perf_counter()-start,3)
    a.result.write_text(json.dumps(entry),encoding='utf-8')

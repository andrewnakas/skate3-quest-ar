"""Apply the retail modular-skater materials and export the native rig to GLB."""
from pathlib import Path
import runpy,sys

tools,work,private=map(Path,sys.argv[sys.argv.index('--')+1:])
sys.argv=['materials','--','--rx2-dir',str(work/'selected/models'),
          '--manifest',str(tools/'default_skater_retail_manifest.json'),
          '--material-dir',str(private/'default_skater/textures/materials'),
          '--parser-dir',str(tools/'vendor/skate3_anim'),
          '--report',str(work/'materials.json')]
runpy.run_path(str(tools/'apply_default_skater_materials.py'),run_name='__main__')
sys.argv=['targets','--',str(private/'stock/data/anim/OnBoard.abin'),str(tools/'vendor/skate3_anim')]
runpy.run_path(str(tools/'add_onboard_ik_targets.py'),run_name='__main__')
sys.argv=['export','--',str(private/'skater.glb'),str(private/'skater.manifest.txt'),
          str(private/'skater.root_motion.json')]
runpy.run_path(str(tools/'export_bevy_glb.py'),run_name='__main__')

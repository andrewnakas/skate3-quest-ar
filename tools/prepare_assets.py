"""Prepare a local extracted Skate 3 disc without a scene editor."""
from pathlib import Path
import argparse,sys
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
from tools.asset_pipeline.install import install

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--game-root',type=Path,required=True)
    parser.add_argument('--output',type=Path,required=True)
    parser.add_argument('--game-exe',type=Path,required=True)
    parser.add_argument('--audio-image',type=Path,help='private matching TU3 g_*.bin image directory for recovered player audio')
    args=parser.parse_args()
    install(None,args.output,args.game_exe.resolve(),lambda text:print(text,flush=True),args.game_root,audio_image=args.audio_image)

if __name__=='__main__':main()

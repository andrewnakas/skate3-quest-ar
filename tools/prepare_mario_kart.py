"""Prepare the user-supplied kart GLB; never downloads or commits third-party assets."""
import argparse, itertools, json, struct, zipfile
from pathlib import Path
import numpy as np
parser=argparse.ArgumentParser(); parser.add_argument("archive",type=Path);parser.add_argument("output",type=Path);args=parser.parse_args()
with zipfile.ZipFile(args.archive) as archive: data=archive.read("source/Kart de mario.glb")
n=struct.unpack_from("<I",data,12)[0]; gltf=json.loads(data[20:20+n]); binary=data[20+n:]
meshes={}; bounds={}
def visit(index,parent):
 node=gltf["nodes"][index]; x,y,z,w=node.get("rotation",[0,0,0,1]); matrix=np.eye(4)
 matrix[:3,:3]=np.array([[1-2*y*y-2*z*z,2*x*y-2*z*w,2*x*z+2*y*w],[2*x*y+2*z*w,1-2*x*x-2*z*z,2*y*z-2*x*w],[2*x*z-2*y*w,2*y*z+2*x*w,1-2*x*x-2*y*y]])@np.diag(node.get("scale",[1,1,1]))
 matrix[:3,3]=node.get("translation",[0,0,0]); matrix=parent@matrix
 if "mesh" in node:
  mesh=node["mesh"]; meshes[mesh]=matrix; points=[]
  for primitive in gltf["meshes"][mesh]["primitives"]:
   accessor=gltf["accessors"][primitive["attributes"]["POSITION"]]
   points.extend([(matrix@np.array([*point,1]))[:3] for point in itertools.product(*zip(accessor["min"],accessor["max"]))])
  points=np.array(points);bounds[mesh]=(points.min(0),points.max(0))
 for child in node.get("children",[]):visit(child,matrix)
for root in gltf["scenes"][0]["nodes"]:visit(root,np.eye(4))
# Flatten authored transforms, normalize to metres and place the chassis centre at the origin.
normalize=np.diag([3.,3.,3.,1.]);normalize[:3,3]=[-0.00856*3,0.,-0.4185*3]
nodes=[{"name":"kart","children":[]}]; wheels=[]
groups=[("wheel_rl",[3,4,5],False),("wheel_rr",[6,7,8],False),("wheel_fl",[10,11,12],True),("wheel_fr",[13,14,15],True)]
wheel_meshes={mesh for _,group,_ in groups for mesh in group}
def leaf(mesh,parent,transform):
 i=len(nodes);nodes.append({"name":f"kart_mesh_{mesh}","mesh":mesh,"matrix":transform.T.flatten().tolist()});nodes[parent].setdefault("children",[]).append(i)
for mesh,matrix in meshes.items():
 if mesh not in wheel_meshes:leaf(mesh,0,normalize@matrix)
for name,group,steering in groups:
 lo=np.min([bounds[m][0] for m in group],axis=0);hi=np.max([bounds[m][1] for m in group],axis=0)
 center=(normalize@np.array([*((lo+hi)/2),1]))[:3]; radius=float((hi[1]-lo[1])*1.5)
 index=len(nodes);nodes.append({"name":name,"translation":center.tolist(),"children":[]});nodes[0]["children"].append(index)
 inverse=np.eye(4);inverse[:3,3]=-center
 for mesh in group:leaf(mesh,index,inverse@normalize@meshes[mesh])
 wheels.append({"node":name,"position":[float(center[0]),float(center[1]+0.25),float(center[2])],"radius":radius,"steering":steering,"driven":not steering})
gltf["nodes"]=nodes;gltf["scenes"]=[{"nodes":[0]}];gltf["scene"]=0
gltf.pop("animations",None);gltf.pop("skins",None)
encoded=json.dumps(gltf,separators=(",",":")).encode();encoded+=b" "*((-len(encoded))%4)
result=struct.pack("<III",0x46546c67,2,20+len(encoded)+len(binary))+struct.pack("<II",len(encoded),0x4e4f534a)+encoded+binary
args.output.mkdir(parents=True,exist_ok=True);(args.output/"kart.glb").write_bytes(result)
print(json.dumps({"wheels":wheels,"bytes":len(result)},indent=2))

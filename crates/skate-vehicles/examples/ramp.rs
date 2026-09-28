use skate_vehicles::{rapier3d::prelude::*, *};
fn def()->VehicleDefinition{serde_json::from_str(include_str!("../../../sdk/examples/freestyle-mx/vehicle.json")).unwrap()}
/// Flat, then a ramp of `deg` rising for `len` metres, then flat again.
fn ramp(deg:f32,len:f32)->Simulation{
 let mut s=Simulation::default(); let mut t=Vec::new();
 let k=deg.to_radians().tan();
 let h=|z:f32| if z<=0. {0.} else if z<len {z*k} else {len*k};
 let mut z=-60.;
 while z<len+60. { let (z0,z1)=(z,z+0.25);
   t.push([[-40.,h(z0),z0],[40.,h(z1),z1],[40.,h(z0),z0]]);
   t.push([[-40.,h(z0),z0],[-40.,h(z1),z1],[40.,h(z1),z1]]); z=z1; }
 s.ground(t.into_iter()).unwrap(); s}
/// Ride the ramp and report the path, so two runs can be compared exactly.
fn ride(d:VehicleDefinition,deg:f32,len:f32)->(Vec<[f32;2]>,f32,bool){
 let mut s=ramp(deg,len);
 let id=s.spawn(d,[0.,1.,-20.],0.).unwrap(); s.set_occupied(id,true);
 for _ in 0..240 {s.step(1./120.);}
 let mut n=0;
 loop{ let v=s.vehicles[&id].controller.current_vehicle_speed;
   s.vehicles.get_mut(&id).unwrap().controls=Controls{throttle:1.,..Default::default()};
   s.step(1./120.); n+=1; if v>=14.||n>4000 {break} }
 let (mut path,mut peak,mut ej)=(Vec::new(),0f32,false);
 for i in 0..(3.5*120.) as u32 {
   s.vehicles.get_mut(&id).unwrap().controls=Controls{throttle:1.,..Default::default()};
   s.step(1./120.);
   if s.take_ejection(id).is_some() {ej=true;}
   let b=&s.world.bodies[s.vehicles[&id].body];
   peak=peak.max(b.translation().y);
   if i%20==0 {path.push([b.translation().z,b.translation().y]);}
 }
 (path,peak,ej)}
fn main(){
 println!("ramps and jumps must be untouched: the step assist is for risers only\n");
 println!("{:>22} {:>10} {:>10} {:>12}","","peak y off","peak y on","worst path gap");
 for (deg,len,name) in [(15.,8.,"15 deg ramp"),(25.,6.,"25 deg ramp"),
                        (35.,4.,"35 deg kicker"),(45.,3.,"45 deg kicker")] {
   let mut off=def(); off.bike.step_assist=0.;
   let (a,pa,ea)=ride(off,deg,len);
   let (b,pb,eb)=ride(def(),deg,len);
   let gap=a.iter().zip(&b).map(|(p,q)|((p[0]-q[0]).abs()).max((p[1]-q[1]).abs())).fold(0f32,f32::max);
   println!("{name:>22} {pa:>10.2} {pb:>10.2} {gap:>12.4}  {}",
     if gap<0.02 {"identical"} else {"DIFFERS"});
   assert_eq!(ea,eb,"ejection differed on {name}");
 }
 println!("\na kerb is a step and *should* be helped:");
 for h in [0.12f32,0.20,0.28] {
   let mut off=def(); off.bike.step_assist=0.;
   let (_,pa,_)=ride(off,89.,h);      // a near-vertical face h tall = a kerb
   let (_,pb,_)=ride(def(),89.,h);
   println!("   {h:.2} m kerb -> peak y {pa:.2} without, {pb:.2} with");
 }
}

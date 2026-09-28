use skate_vehicles::{rapier3d::prelude::*, *};
fn def()->VehicleDefinition{serde_json::from_str(include_str!("../../../sdk/examples/freestyle-mx/vehicle.json")).unwrap()}
fn stairs(rise:f32,run:f32,n:i32)->Simulation{
 let mut s=Simulation::default(); let mut t=Vec::new();
 let q=|x0:f32,x1:f32,z0:f32,z1:f32,y:f32,t:&mut Vec<[[f32;3];3]>|{
  t.push([[x0,y,z0],[x1,y,z1],[x1,y,z0]]); t.push([[x0,y,z0],[x0,y,z1],[x1,y,z1]]);};
 q(-40.,40.,-60.,0.,0.,&mut t);
 for i in 0..n { let y=(i+1) as f32*rise; let z0=i as f32*run; let z1=z0+run;
   t.push([[-40.,y-rise,z0],[40.,y,z0],[40.,y-rise,z0]]);
   t.push([[-40.,y-rise,z0],[-40.,y,z0],[40.,y,z0]]);
   q(-40.,40.,z0,z1,y,&mut t); }
 q(-40.,40.,n as f32*run,n as f32*run+40.,n as f32*rise,&mut t);
 s.ground(t.into_iter()).unwrap(); s}
struct Run{steps:f32,jam:f32,eject:bool}
fn climb(d:VehicleDefinition,rise:f32,run:f32,n:i32,speed:f32,phase:f32)->Run{
 let mut s=stairs(rise,run,n);
 let id=s.spawn(d,[0.,1.,-14.+phase],0.).unwrap(); s.set_occupied(id,true);
 for _ in 0..240 {s.step(1./120.);}
 let mut k=0;
 loop{ let v=s.vehicles[&id].controller.current_vehicle_speed;
   s.vehicles.get_mut(&id).unwrap().controls=Controls{throttle:1.,..Default::default()};
   s.step(1./120.); k+=1; if v>=speed||k>4000 {break} }
 let (mut far,mut jam,mut eject)=(f32::MIN,0f32,false);
 for _ in 0..(5.*120.) as u32 {
   s.vehicles.get_mut(&id).unwrap().controls=Controls{throttle:1.,..Default::default()};
   s.step(1./120.);
   if s.take_ejection(id).is_some() {eject=true;}
   let v=&s.vehicles[&id]; let b=&s.world.bodies[v.body];
   // The signature of a jam: the chassis itself taking a big shove.
   for &c in b.colliders() { for p in s.world.narrow_phase.contact_pairs_with(c) {
     jam=jam.max(p.total_impulse_magnitude()); } }
   far=far.max(b.translation().z); }
 Run{steps:(far/run).clamp(0.,n as f32+3.),jam,eject}
}
fn geo(z:f32,hy:f32,oy:f32,r:f32)->VehicleDefinition{
 let mut d=def(); d.half_extents[2]=z; d.half_extents[1]=hy;
 d.collider_offset[1]=oy; d.collider_rounding=r; d}
fn row(name:&str,d:VehicleDefinition){
 let belly=0.351+d.collider_offset[1]-d.half_extents[1]-d.collider_rounding;
 print!("  {name:<26} belly {belly:.2} |");
 for rise in [0.15f32,0.18,0.20,0.25] {
   // Average over entry phases: which part of a tread you arrive on decides
   // a single run, and one sample of that is a coin flip, not a measurement.
   let runs:Vec<Run>=(0..5).map(|i| climb(d.clone(),rise,0.32,12,8.,i as f32*0.064)).collect();
   let cleared=runs.iter().filter(|r|r.steps>11.).count();
   let mean=runs.iter().map(|r|r.steps).sum::<f32>()/5.;
   let jam=runs.iter().map(|r|r.jam).sum::<f32>()/5.;
   let ej=runs.iter().filter(|r|r.eject).count();
   print!(" {mean:>5.1}({cleared}/5,j{jam:>5.0}{}) ",if ej>0 {"!"} else {" "}); }
 println!();
}
fn tune(mut d:VehicleDefinition,v:f32,z:f32,hy:f32,oy:f32)->VehicleDefinition{
 d.bike.step_assist=v; d.half_extents[2]=z; d.half_extents[1]=hy; d.collider_offset[1]=oy; d}
fn main(){
 println!("12-step flight, 0.32 m treads, entered at 8 m/s, averaged over 5 entry phases");
 println!("mean steps climbed (runs cleared /5, mean peak chassis impulse; ! = a rider was thrown)
");
 println!("{:>40}{:>17}{:>21}{:>21}","0.15 rise","0.18","0.20","0.25");
 row("shipped, no assist",tune(def(),0.,0.90,0.25,0.30));
 row("assist only",tune(def(),0.30,0.90,0.25,0.30));
 row("geometry only",tune(def(),0.,0.70,0.18,0.40));
 row("assist + geometry",tune(def(),0.30,0.70,0.18,0.40));
 row("assist + short frame",tune(def(),0.30,0.62,0.16,0.42));
}

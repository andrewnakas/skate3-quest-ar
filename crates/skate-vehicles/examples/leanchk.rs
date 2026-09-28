use skate_vehicles::{rapier3d::prelude::*, *};
fn def()->VehicleDefinition{serde_json::from_str(include_str!("../../../sdk/examples/freestyle-mx/vehicle.json")).unwrap()}
fn main(){
 println!("full lock, held. 'shown' is the lean the model is drawn at;");
 println!("'justified' is atan(v*w/g) -- the angle that balances the turn actually happening.\n");
 println!("{:>6} {:>8} {:>9} {:>11} {:>9}","speed","rate/s","radius m","justified","shown");
 for target in [2.0f32,4.,6.,9.,12.,18.,24.] {
  let mut s=Simulation::default();
  s.ground(vec![[[-400.,0.,-400.],[400.,0.,400.],[400.,0.,-400.]],
                [[-400.,0.,-400.],[-400.,0.,400.],[400.,0.,400.]]].into_iter()).unwrap();
  let id=s.spawn(def(),[0.,1.,0.],0.).unwrap(); s.set_occupied(id,true);
  for _ in 0..240 {s.step(1./120.);}
  let mut n=0;
  loop{ let spd=s.vehicles[&id].controller.current_vehicle_speed;
    let t=if spd<target {1.} else {0.};
    let br=if spd>target+0.4 {0.5} else {0.};
    s.vehicles.get_mut(&id).unwrap().controls=Controls{throttle:t,brake:br,..Default::default()};
    s.step(1./120.); n+=1; if (spd-target).abs()<0.25||n>6000 {break} }
  // Corner for a second to settle, then measure over one more.
  for hold in 0..2 {
   let mut heading=0.; let mut last=None; let mut lean=0f32; let mut rate=0.;
   for _ in 0..120 {
    let spd=s.vehicles[&id].controller.current_vehicle_speed;
    // A real servo, not a two-step: the corner must be measured at the speed
    // asked for, and at walking pace an open throttle just accelerates away.
    let err=target-spd;
    let (t,br)=if err>0. {((err*0.6).min(1.),0.)} else {(0.,(-err*0.5).min(1.))};
    s.vehicles.get_mut(&id).unwrap().controls=Controls{throttle:t,brake:br,steering:1.,..Default::default()};
    s.step(1./120.);
    let v=&s.vehicles[&id]; let b=&s.world.bodies[v.body];
    let f=*b.rotation()*Vector::Z; let h=f.x.atan2(f.z);
    if let Some(l)=last {let mut dd:f32=h-l;
      if dd>std::f32::consts::PI {dd-=std::f32::consts::TAU} else if dd< -std::f32::consts::PI {dd+=std::f32::consts::TAU}
      heading+=dd;}
    last=Some(h); lean=s.bike_state(id).unwrap().lean; rate=0.;
   }
   if hold==0 {continue}
   let v=&s.vehicles[&id]; let spd=v.controller.current_vehicle_speed.abs();
   let w=heading.abs();
   let just=(spd*w/9.81).atan().to_degrees();
   println!("{spd:>6.1} {w:>8.2} {:>9.1} {just:>10.0}d {:>8.0}d",
     if w>0.01 {spd/w} else {0.}, lean.abs().to_degrees());
   let _=rate;
  }
 }
}

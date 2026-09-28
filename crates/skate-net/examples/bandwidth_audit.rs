//! Runs the production serializer on synthetic bounds; never starts the game or Steam.
use skate_net::{Body, Bone, Frame, Pose, Shape, Volume};

fn main() {
    let pose = Pose {
        p: [0.; 3],
        q: [0., 0., 0., 1.],
    };
    for (label, bones, volumes, name_len) in [
        (
            "30 anchors, no active collision volumes (lower bound)",
            30,
            0,
            16,
        ),
        (
            "30 anchors, 32 capsules (illustrative, not a gameplay capture)",
            30,
            32,
            16,
        ),
        ("30 anchors, 64 boxes (capture upper bound)", 30, 64, 128),
        ("32 anchors, 64 boxes (protocol upper bound)", 32, 64, 128),
    ] {
        let frame = Frame {
            map: 1,
            rig: 2,
            tick: 1,
            appearance: "x".repeat(name_len),
            root: pose,
            bones: (0..bones).map(|index| Bone { index, pose }).collect(),
            bodies: vec![
                Body {
                    pose,
                    velocity: [0.; 3],
                    angular: [0.; 3]
                };
                33
            ],
            volumes: (0..volumes)
                .map(|i| Volume {
                    body: (i % 33) as u8,
                    shape: if volumes == 32 {
                        Shape::Capsule {
                            center: [0.; 3],
                            axis: [0., 1., 0.],
                            half: 0.2,
                            radius: 0.1,
                        }
                    } else {
                        Shape::Box {
                            pose,
                            half: [0.1; 3],
                            radius: 0.01,
                        }
                    },
                })
                .collect(),
        };
        let packets = skate_net::packets(1, 2, 1, &frame).unwrap();
        let bytes: usize = packets.iter().map(Vec::len).sum();
        println!(
            "{label}: {} packets, {bytes} bytes/update including application headers",
            packets.len()
        );
        for hz in [30., 31.25] {
            println!(
                "  at {hz} Hz: {:.2} kB/s, {:.4} Mbps each direction; IPv4+UDP {:.4} Mbps (not Steam overhead)",
                bytes as f64 * hz / 1000.,
                bytes as f64 * hz * 8. / 1e6,
                (bytes + packets.len() * 28) as f64 * hz * 8. / 1e6
            );
        }
        for loss in [0.01_f64, 0.05] {
            println!(
                "  independent {:.0}% fragment loss => {:.2}% incomplete snapshots",
                loss * 100.,
                (1. - (1. - loss).powi(packets.len() as i32)) * 100.
            );
        }
    }
}

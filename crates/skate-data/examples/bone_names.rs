//! Throwaway: which cycle-clip names the animation metadata can actually resolve.
fn main() {
    let root = std::path::PathBuf::from(std::env::args().nth(1).expect("asset root"));
    let banks = skate_data::animation_banks::AnimationBanks::load(&root).unwrap();
    let metadata = banks.metadata().unwrap();
    for name in [
        "T_360FLIP_H_CYC",
        "T_360FLIP_L_CYC",
        "T_LASERFLIP_H_CYC",
        "T_LASERFLIP_L_CYC",
        "T_N_360FLIP_H_CYC",
        "T_N_360FLIP_L_CYC",
        "T_N_LASERFLIP_H_CYC",
        "T_N_LASERFLIP_L_CYC",
    ] {
        let tree = metadata.tree(name).is_ok();
        let clip = metadata.clip(name).is_ok();
        println!("{name:22} tree={tree:5} clip={clip}");
    }
}

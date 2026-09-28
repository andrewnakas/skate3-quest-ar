//! Does the guest's flush mode actually reach the conversions, once the optimizer is on?
//!
//! Two of this crate's tests assert that a denormal single is destroyed by `DAZ` and that an
//! `lfs`/`stfs` round trip quiets a signalling NaN. Both pass at `opt-level = 0` and fail at 3,
//! which leaves two possible causes with very different consequences:
//!
//! 1. the optimizer folded the conversion at compile time, where MXCSR does not exist — a test
//!    artifact, fixed by making the input opaque; or
//! 2. the optimizer moved the conversion across the `ldmxcsr` that sets the mode — a real hazard
//!    for every port in this crate that depends on the guest's flush mode.
//!
//! This program distinguishes them. Run it under both profiles:
//!
//! ```sh
//! cargo run --example flush_probe
//! cargo run --release --example flush_probe
//! ```

use skate_audio_core::{Guest, fp, vmx};

fn show(label: &str, bits: u32) {
    println!("  {label:<44} 0x{bits:08X}");
}

fn main() {
    let flush = vmx::FLUSH_ZERO | vmx::DENORMALS_ZERO;

    // 1. The conversion on a value the compiler cannot see, with the mode set directly.
    vmx::set_mxcsr(vmx::get_mxcsr() | flush);
    let mxcsr_after_set = vmx::get_mxcsr();
    let denormal_bits = std::hint::black_box(1u32);
    let widened = f32::from_bits(denormal_bits) as f64;
    show("MXCSR after set_mxcsr (FZ|DAZ = 0x8040)", mxcsr_after_set);
    show(
        "opaque denormal, direct set_mxcsr",
        (widened as f32).to_bits(),
    );

    // 2. The same through an `Fpscr` guard, which is what every port holds.
    vmx::set_mxcsr(vmx::get_mxcsr() & !flush);
    {
        let mut fpscr = vmx::Fpscr::capture();
        fpscr.disable_flush_mode_unconditional();
        show("MXCSR inside an Fpscr guard", vmx::get_mxcsr());
        let bits = std::hint::black_box(1u32);
        let value = f32::from_bits(bits) as f64;
        show(
            "opaque denormal, inside the guard",
            (value as f32).to_bits(),
        );

        // 3. And through guest memory, the way a port really reads its samples.
        let mut g = Guest::single(0x1000_0000, 64);
        g.set_u32(0x1000_0000, std::hint::black_box(1)).unwrap();
        let loaded = fp::load_single(&g, 0x1000_0000).unwrap();
        fp::store_single(&mut g, 0x1000_0020, loaded).unwrap();
        show(
            "opaque denormal, through guest memory",
            g.u32(0x1000_0020).unwrap(),
        );

        // 4. A signalling NaN through the same round trip: widening quiets it on the hardware.
        g.set_u32(0x1000_0000, std::hint::black_box(0x7FA0_0000))
            .unwrap();
        let nan = fp::load_single(&g, 0x1000_0000).unwrap();
        fp::store_single(&mut g, 0x1000_0020, nan).unwrap();
        show(
            "opaque sNaN, through guest memory",
            g.u32(0x1000_0020).unwrap(),
        );

        // 5. A denormal *result* rather than a denormal operand: FZ, not DAZ.
        let small = std::hint::black_box(1e-40f32);
        let product = fp::mul_single(f64::from(small), f64::from(std::hint::black_box(0.5f32)));
        show("denormal product (FZ, not DAZ)", (product as f32).to_bits());
    }

    println!("\n  expected with the mode in force: denormals 0x00000000, sNaN 0x7FE00000");
}

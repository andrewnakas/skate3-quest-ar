// Bounds-checked equivalent of the Python RefPack command interpreter.
fn decode(src: &[u8], out: &mut [u8], mut pos: usize, early: bool) -> Result<(), ()> {
    let mut written = 0usize;
    while pos < src.len() && !(early && written == out.len()) {
        let control = src[pos] as usize; pos += 1;
        let (literal, distance, count) = if control < 0x80 {
            let b = *src.get(pos).ok_or(())? as usize; pos += 1;
            (control & 3, ((control & 0x60) << 3) + b + 1, ((control >> 2) & 7) + 3)
        } else if control < 0xc0 {
            let b = src.get(pos..pos+2).ok_or(())?; pos += 2;
            ((b[0] >> 6) as usize, (((b[0] & 0x3f) as usize) << 8) + b[1] as usize + 1, (control & 0x3f) + 4)
        } else if control < 0xe0 {
            let b = src.get(pos..pos+3).ok_or(())?; pos += 3;
            (control & 3, ((control & 0x10) << 12) + ((b[0] as usize) << 8) + b[1] as usize + 1, ((control & 0xc) << 6) + b[2] as usize + 5)
        } else if control < 0xfc { (((control & 0x1f) << 2) + 4, 0, 0) }
        else { (control & 3, 0, 0) };
        let bytes = src.get(pos..pos+literal).ok_or(())?;
        out.get_mut(written..written+literal).ok_or(())?.copy_from_slice(bytes);
        written += literal; pos += literal;
        if count != 0 {
            if distance == 0 || distance > written || count > out.len()-written { return Err(()); }
            // Overlapping references intentionally see bytes just produced.
            for _ in 0..count { out[written] = out[written-distance]; written += 1; }
        }
        if control >= 0xfc { break; }
    }
    if written == out.len() { Ok(()) } else { Err(()) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn skate_refpack(src: *const u8, src_len: usize, out: *mut u8, out_len: usize, start: usize, early: bool) -> i32 {
    if src.is_null() || out.is_null() || src_len > isize::MAX as usize || out_len > isize::MAX as usize || start > src_len { return 1; }
    let source = unsafe { std::slice::from_raw_parts(src, src_len) };
    let output = unsafe { std::slice::from_raw_parts_mut(out, out_len) };
    match decode(source, output, start, early) { Ok(()) => 0, Err(()) => 1 }
}

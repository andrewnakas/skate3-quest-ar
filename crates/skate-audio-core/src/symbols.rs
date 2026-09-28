//! The `.csi` symbol tables at run time: installing a project, and resolving a symbol into a slot.
//!
//! **Unverified new work.** These run on the load and game threads, outside the 216 audio-thread
//! functions, so there is no C++ body to transcribe and none was ever compared. They follow the
//! lifted `sub_828E2818`, `sub_828E3148`, `sub_828E3250` and `sub_828E3358` line by line. The
//! evidence on real data is `examples/bind_banks.rs`, which installs every shipped project, resolves
//! every bank export through these functions, and checks each outcome against an independent search
//! of the parsed `.csi` files.
//!
//! A loaded `.csi` is patched in place. Its three record tables are 12, 12 and 16 bytes a record:
//!
//! ```text
//! table 0, 1  { +0 listener head, +4 name, +8 u16 name_id, +10 u16 generation }
//! table 2     { +0 listener head, +4 value, +8 name, +12 u16 name_id, +14 u16 generation }
//! project     +10/+12/+14 u16 counts, +16 u16 project id,
//!             +20/+24/+28 table pointers, +32 {next, prev} list node
//! ```
//!
//! Installing turns each name offset into a pointer, stamps each record with a fresh generation, and
//! links the project onto the list at [`PROJECT_LIST_HEAD`]. A slot is `{record*, u32 word}`, where
//! the word is the record's `{name_id, generation}`, which is how a post later tells a stale slot.
//!
//! **Lookups make two passes.** The first scans only projects whose id matches the query's; if that
//! finds nothing, the second scans every project, matching the name id and the name string alone.
//! That is why a bank built against a project that does not ship still binds by name.

use crate::{Guest, Result};

/// `lis -31988` + -16816: head of the installed project list, pointing at a project's `+32` node.
pub const PROJECT_LIST_HEAD: u32 = 0x830B_BE50;
/// `lis -31992` + 15568: the u16 generation counter.
pub const GENERATION: u32 = 0x8308_3CD0;
/// The status a lookup returns when no project holds the symbol.
pub const NOT_FOUND: i32 = -5;

/// Where a table's fields sit, for the three lookups.
#[derive(Clone, Copy)]
struct Shape {
    /// Pointer to the table, as a negative offset from the project's list node.
    table_back: u32,
    /// The u16 record count, likewise.
    count_back: u32,
    stride: u32,
    /// The name pointer within a record.
    name_at: u32,
    /// The word a slot takes: `{name_id, generation}`.
    word_at: u32,
}

const TABLE0: Shape = Shape {
    table_back: 12,
    count_back: 22,
    stride: 12,
    name_at: 4,
    word_at: 8,
};
const TABLE1: Shape = Shape {
    table_back: 8,
    count_back: 20,
    stride: 12,
    name_at: 4,
    word_at: 8,
};
const TABLE2: Shape = Shape {
    table_back: 4,
    count_back: 18,
    stride: 16,
    name_at: 8,
    word_at: 12,
};

/// What a lookup did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Lookup {
    /// `r3`: 0, or [`NOT_FOUND`].
    pub status: i32,
    /// Whether the match came from the second, project-blind pass. Not an output of the original;
    /// it only reports which path ran.
    pub second_pass: bool,
}

fn sext16(value: u64) -> i64 {
    value as u16 as i16 as i64
}

/// `sub_828E2818`: install the project at `csi`. Always returns 0 in the original.
pub fn install_project(g: &mut Guest, csi: u32) -> Result<()> {
    let count0 = g.u16(csi + 10)? as u64; // lhz r11,10(r3)
    let table0 = csi as u64 + 40; // addi r10,r3,40
    let count1 = g.u16(csi + 12)? as u64; // lhz r9,12(r3)
    g.set_u32(csi + 20, table0 as u32)?; // stw r10,20(r3)
    let table1 = table0 + ((count0 * 3) << 2); // r7 = 3*c0 ; rlwinm ...,2 ; add r10,r8,r10
    let mut generation = g.u16(GENERATION)? as u64; // lhz r11,15568(r6)
    let table2 = table1 + ((count1 * 3) << 2); // add r4,r9,r10
    g.set_u32(csi + 24, table1 as u32)?; // stw r10,24(r3)
    g.set_u32(csi + 28, table2 as u32)?; // stw r4,28(r3)

    // The three loops differ only in which table, count, stride and fields they touch. Table 0 is
    // entered on a signed `count > 0`, tables 1 and 2 on a reloaded `count != 0`.
    let tables = [
        (20u32, 10u32, 12u64, 4u32, 10u32),
        (24, 12, 12, 4, 10),
        (28, 14, 16, 8, 14),
    ];
    for (t, &(table_at, count_at, stride, name_at, gen_at)) in tables.iter().enumerate() {
        let enter = if t == 0 {
            count0 as i32 > 0
        } else {
            g.u16(csi + count_at)? != 0
        };
        if !enter {
            continue;
        }
        let mut index: i64 = 0; // li r7,0
        let mut offset: u64 = 0; // li r9,0
        loop {
            let record = (g.u32(csi + table_at)? as u64).wrapping_add(offset); // lwz ; add
            generation = sext16(generation.wrapping_add(1)) as u64; // addi r8,r11,1 ; extsh r11,r8
            let name = g.u32(record as u32 + name_at)? as u64; // lwz r8,4(r10)
            g.set_u32(
                record as u32 + name_at,
                name.wrapping_add(csi as u64) as u32,
            )?; // stw r4,4(r10)
            if (generation as i64) < 0 {
                generation = 1; // li r11,1
                g.set_u16(GENERATION, 1)?; // sth r11,15568(r6)
            }
            let record = (g.u32(csi + table_at)? as u64).wrapping_add(offset); // reloaded
            index += 1;
            offset += stride;
            g.set_u16(record as u32 + gen_at, generation as u16)?; // sth r11,10(r10)
            let count = g.u16(csi + count_at)? as i32; // lhz r8,10(r3), reloaded each trip
            if !((index as i32) < count) {
                break;
            }
        }
        g.set_u16(GENERATION, generation as u16)?; // sth r11,15568(r6)
    }

    let node = csi + 32; // addi r11,r3,32
    let head = g.u32(PROJECT_LIST_HEAD)?; // lwz r9,-16816(r10)
    g.set_u32(csi + 36, 0)?; // stw r8,36(r3)
    g.set_u32(csi + 32, head)?; // stw r9,32(r3)
    let head = g.u32(PROJECT_LIST_HEAD)?; // reloaded
    if head != 0 {
        g.set_u32(head + 4, node)?; // stw r11,4(r9)
    }
    g.set_u32(PROJECT_LIST_HEAD, node) // stw r11,-16816(r10)
}

/// The three lookups' shared body. `query` is `{char* name; u16 project; u16 name_id}`.
fn lookup(g: &mut Guest, shape: Shape, slot: u32, query: u32) -> Result<Lookup> {
    let head = g.u32(PROJECT_LIST_HEAD)?; // lwz r28,-16816(r11)
    let mut second = false; // li r29,0
    loop {
        let mut node = head; // mr r3,r28
        if node != 0 {
            let project = sext16(g.u16(query + 4)? as u64); // lhz r11,4(r4) ; extsh r30,r11
            while node != 0 {
                let id = sext16(g.u16(node.wrapping_sub(16))? as u64); // lhz r11,-16(r3)
                let table = g.u32(node.wrapping_sub(shape.table_back))?; // lwz r31,-8(r3)
                let count = g.u16(node.wrapping_sub(shape.count_back))? as i32; // lhz r6,-20(r3)
                if (id == project || second) && count > 0 {
                    let name_id = sext16(g.u16(query + 6)? as u64); // lhz r11,6(r4) ; extsh r5
                    let mut entry = table.wrapping_add(shape.name_at); // addi r8,r31,4
                    for index in 0..count {
                        let id_here = sext16(g.u16(entry.wrapping_add(4))? as u64); // lhz r11,4(r8)
                        if id_here == name_id {
                            let mut theirs = g.u32(entry)?; // lwz r10,0(r8)
                            let mut ours = g.u32(query)?; // lwz r11,0(r4)
                            let difference = loop {
                                let q = g.u8(ours)? as i32; // lbz r9,0(r11)
                                let r = g.u8(theirs)? as i32; // lbz r26,0(r10)
                                let difference = q - r; // subf r9,r26,r9
                                if q == 0 || difference != 0 {
                                    break difference;
                                }
                                ours = ours.wrapping_add(1);
                                theirs = theirs.wrapping_add(1);
                            };
                            if difference == 0 {
                                // loc_828E332C: the record, then its {name_id, generation} word.
                                let record = table.wrapping_add(index as u32 * shape.stride);
                                g.set_u32(slot, record)?; // stw r11,0(r27)
                                let word = g.u32(record.wrapping_add(shape.word_at))?; // lwz r10,8(r11)
                                g.set_u32(slot + 4, word)?; // stw r10,4(r27)
                                return Ok(Lookup {
                                    status: 0,
                                    second_pass: second,
                                });
                            }
                        }
                        entry = entry.wrapping_add(shape.stride);
                    }
                }
                node = g.u32(node)?; // lwz r3,0(r3)
            }
        }
        // loc_828E3318
        if second {
            return Ok(Lookup {
                status: NOT_FOUND,
                second_pass: true,
            });
        }
        second = true;
    }
}

/// `sub_828E3148`: resolve a table-0 symbol into `slot`.
pub fn lookup_table0(g: &mut Guest, slot: u32, query: u32) -> Result<Lookup> {
    lookup(g, TABLE0, slot, query)
}

/// `sub_828E3250`: resolve a table-1 symbol (the objects game code posts to) into `slot`.
pub fn lookup_table1(g: &mut Guest, slot: u32, query: u32) -> Result<Lookup> {
    lookup(g, TABLE1, slot, query)
}

/// `sub_828E3358`: resolve a table-2 symbol into `slot`.
pub fn lookup_table2(g: &mut Guest, slot: u32, query: u32) -> Result<Lookup> {
    lookup(g, TABLE2, slot, query)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MEM: u32 = 0x5000_0000;
    const CSI_A: u32 = MEM;
    const CSI_B: u32 = MEM + 0x400;
    const QUERY: u32 = MEM + 0x800;
    const QNAME: u32 = MEM + 0x810;
    const SLOT: u32 = MEM + 0x840;

    fn guest() -> Guest {
        let mut g = Guest::single(MEM, 0x1000);
        g.put(PROJECT_LIST_HEAD, vec![0; 4]);
        g.put(GENERATION, vec![0; 2]);
        g
    }

    /// A project: one table-1 record per name, one table-2 record for the last, names in a pool.
    fn put_project(g: &mut Guest, at: u32, project: u16, names: &[(&str, u16)]) {
        let c1 = names.len() as u32;
        g.set_u16(at + 10, 0).unwrap();
        g.set_u16(at + 12, c1 as u16).unwrap();
        g.set_u16(at + 14, 1).unwrap();
        g.set_u16(at + 16, project).unwrap();
        let mut pool = 40 + 12 * c1 + 16;
        for (i, (name, id)) in names.iter().enumerate() {
            let rec = at + 40 + 12 * i as u32;
            g.set_u32(rec + 4, pool).unwrap();
            g.set_u16(rec + 8, *id).unwrap();
            g.set_span(at + pool, name.as_bytes()).unwrap();
            g.set_u8(at + pool + name.len() as u32, 0).unwrap();
            if i + 1 == names.len() {
                let rec2 = at + 40 + 12 * c1;
                g.set_u32(rec2 + 8, pool).unwrap();
                g.set_u16(rec2 + 12, *id).unwrap();
            }
            pool += name.len() as u32 + 1;
        }
    }

    fn put_query(g: &mut Guest, name: &str, project: u16, id: u16) {
        g.set_u32(QUERY, QNAME).unwrap();
        g.set_u16(QUERY + 4, project).unwrap();
        g.set_u16(QUERY + 6, id).unwrap();
        g.set_span(QNAME, name.as_bytes()).unwrap();
        g.set_u8(QNAME + name.len() as u32, 0).unwrap();
    }

    fn installed() -> Guest {
        let mut g = guest();
        put_project(
            &mut g,
            CSI_A,
            0x64BD,
            &[("Class_grind", 0x09C5), ("Class_Flips", 0x47A3)],
        );
        put_project(&mut g, CSI_B, 0x5C48, &[("c_body_slide", 0x4A6C)]);
        install_project(&mut g, CSI_A).unwrap();
        install_project(&mut g, CSI_B).unwrap();
        g
    }

    #[test]
    fn installing_relocates_names_stamps_generations_and_links_the_list() {
        let g = installed();
        assert_eq!(
            g.u32(CSI_A + 24).unwrap(),
            CSI_A + 40,
            "table 1 follows an empty table 0"
        );
        assert_eq!(
            g.u32(CSI_A + 40 + 4).unwrap(),
            CSI_A + 40 + 24 + 16,
            "name offset became a pointer"
        );
        let gens: Vec<u16> = [
            CSI_A + 40 + 10,
            CSI_A + 52 + 10,
            CSI_A + 64 + 14,
            CSI_B + 40 + 10,
            CSI_B + 52 + 14,
        ]
        .iter()
        .map(|a| g.u16(*a).unwrap())
        .collect();
        assert_eq!(
            gens,
            vec![1, 2, 3, 4, 5],
            "one counter across tables and projects"
        );
        assert_eq!(g.u16(GENERATION).unwrap(), 5);
        assert_eq!(
            g.u32(PROJECT_LIST_HEAD).unwrap(),
            CSI_B + 32,
            "the newest project heads the list"
        );
        assert_eq!(g.u32(CSI_B + 32).unwrap(), CSI_A + 32);
        assert_eq!(g.u32(CSI_A + 36).unwrap(), CSI_B + 32, "prev link");
    }

    #[test]
    fn the_generation_counter_wraps_to_one() {
        let mut g = guest();
        g.set_u16(GENERATION, 0x7FFF).unwrap();
        put_project(&mut g, CSI_A, 1, &[("a", 1)]);
        install_project(&mut g, CSI_A).unwrap();
        assert_eq!(g.u16(CSI_A + 40 + 10).unwrap(), 1);
        assert_eq!(g.u16(GENERATION).unwrap(), 2);
    }

    #[test]
    fn a_matching_project_resolves_on_the_first_pass() {
        let mut g = installed();
        put_query(&mut g, "Class_Flips", 0x64BD, 0x47A3);
        let l = lookup_table1(&mut g, SLOT, QUERY).unwrap();
        assert_eq!(
            l,
            Lookup {
                status: 0,
                second_pass: false
            }
        );
        assert_eq!(g.u32(SLOT).unwrap(), CSI_A + 52);
        assert_eq!(g.u32(SLOT + 4).unwrap(), 0x47A3_0002);
    }

    #[test]
    fn an_unshipped_project_still_binds_by_name_on_the_second_pass() {
        let mut g = installed();
        put_query(&mut g, "Class_grind", 0x63D9, 0x09C5);
        let l = lookup_table1(&mut g, SLOT, QUERY).unwrap();
        assert_eq!(
            l,
            Lookup {
                status: 0,
                second_pass: true
            }
        );
        assert_eq!(g.u32(SLOT).unwrap(), CSI_A + 40);
    }

    #[test]
    fn the_name_must_match_as_well_as_the_id() {
        let mut g = installed();
        put_query(&mut g, "Class_grinD", 0x64BD, 0x09C5);
        assert_eq!(
            lookup_table1(&mut g, SLOT, QUERY).unwrap().status,
            NOT_FOUND
        );
        put_query(&mut g, "Class_grin", 0x64BD, 0x09C5);
        assert_eq!(
            lookup_table1(&mut g, SLOT, QUERY).unwrap().status,
            NOT_FOUND,
            "a prefix is not a match"
        );
        assert_eq!(
            g.u32(SLOT).unwrap(),
            0,
            "a failed lookup leaves the slot alone"
        );
    }

    #[test]
    fn table_two_uses_its_own_stride_and_fields() {
        let mut g = installed();
        put_query(&mut g, "c_body_slide", 0x5C48, 0x4A6C);
        assert_eq!(lookup_table2(&mut g, SLOT, QUERY).unwrap().status, 0);
        assert_eq!(g.u32(SLOT).unwrap(), CSI_B + 52);
        assert_eq!(g.u32(SLOT + 4).unwrap(), 0x4A6C_0005);
        assert_eq!(
            lookup_table0(&mut g, SLOT, QUERY).unwrap().status,
            NOT_FOUND,
            "table 0 is empty"
        );
    }

    #[test]
    fn an_empty_list_finds_nothing() {
        let mut g = guest();
        put_query(&mut g, "x", 1, 1);
        assert_eq!(
            lookup_table1(&mut g, SLOT, QUERY).unwrap(),
            Lookup {
                status: NOT_FOUND,
                second_pass: true
            }
        );
    }
}

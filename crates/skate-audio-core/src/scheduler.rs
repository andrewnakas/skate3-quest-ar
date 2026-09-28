//! The scheduler's instance and bucket-list layer.
//!
//! [`tick_bucket`] is `sub_82B48A50`: it walks one list head, publishes the running instance,
//! calls the owning plug-in through [`SchedulerHost`], records an optional timebase delta, and
//! completes self-removals. The plug-in and timebase are host boundaries because both are indirect
//! operations in the original; the list order and guest-memory effects remain recovered here.
//!
//! [`detach_instance`] and [`recycle_node`] supply the matching intrusive-list mechanics.

use crate::{Guest, Result, fp, modules, voices};

/// `addi r11,r3,112` — the scheduler is embedded in `rw_system` at `+0x70`.
pub const SYSTEM_SCHEDULER: u32 = 112;
/// `rw_scheduler.current_node` (`+0x44`), read through `r3+180`.
pub const SCHED_CURRENT: u32 = 68;
/// `+0x48`, the word `docs/rw_audio_structs.h` calls `_pad48`. `sub_82B48A50` scales it by 32 to
/// re-derive the bucket a parked removal belongs to, so it is the pending bucket index.
pub const SCHED_PENDING_BUCKET: u32 = 72;
/// `rw_scheduler.node_removed` (`+0x4C`).
pub const SCHED_NODE_REMOVED: u32 = 76;
/// `rlwinm r10,r10,5,0,26` — 32 bytes per bucket record.
pub const BUCKET_STRIDE: u32 = 32;

/// `rw_instance + 0x00`. See the module note: this is the node pointer, not a descriptor.
pub const INSTANCE_NODE: u32 = 0;
/// `rw_instance.elapsed` (`+0x10`).
pub const INSTANCE_ELAPSED: u32 = 16;
/// `+0x14`, the byte naming which scheduler bucket the instance's node sits in.
pub const INSTANCE_BUCKET: u32 = 20;
/// `li r11,3` — both the "not linked" bucket value and the value the tail stores.
pub const DETACHED: u32 = 3;

/// `node + 0x00`, the previous link.
pub const NODE_PREV: u32 = 0;
/// `node + 0x04`, the next link.
pub const NODE_NEXT: u32 = 4;
/// `node + 0x08`, the back pointer to the instance.
pub const NODE_INSTANCE: u32 = 8;
/// `node + 0x0C`, the byte selecting which of the bucket's two heads may name this node.
pub const NODE_WHICH: u32 = 12;

/// `manager + 0x0C`, the head of the bucket's free list.
pub const BUCKET_FREE_HEAD: u32 = 12;
/// `manager + 0x10`. For bucket *b* this is `scheduler + 32*b + 16`, which for bucket 0 is the
/// `buckets[0]` cell `docs/rw_audio_structs.h` documents at `+0x10` with stride `0x20`.
pub const BUCKET_HEAD_A: u32 = 16;
/// `manager + 0x14`, the bucket's second list head.
pub const BUCKET_HEAD_B: u32 = 20;
/// `manager + 0x18`, the live node count.
pub const BUCKET_COUNT: u32 = 24;

/// `sub_82B48B28`'s element fields. An element owns one scheduler node and moves it from the
/// bucket's pending list (`+16`) to its runnable list (`+20`).
pub const REQUEUE_NODE: u32 = 0;
pub const REQUEUE_CLEARED: u32 = 16;
pub const REQUEUE_STATE: u32 = 20;
/// The state value that leaves an element parked, without touching either it or its node.
pub const REQUEUE_PARKED: u8 = 3;

/// `rw_system + 0x1C`: the intrusive deferred-element list drained by
/// [`drain_deferred_sends`].
pub const DEFERRED_HEAD: u32 = 28;
/// The element's node sits at `+0x1C`, so the list's node pointer minus this value is its owner.
pub const DEFERRED_NODE: u32 = 28;
pub const DEFERRED_OWNER: u32 = 16;
pub const DEFERRED_CHILD_COUNT: u32 = 68;
pub const DEFERRED_STATE: u32 = 71;
pub const DEFERRED_CHILDREN: u32 = 80;
pub const DEFERRED_WORD_60: u32 = 60;
pub const DEFERRED_CHILD_SEND_COUNT: u32 = 43;
pub const DEFERRED_CHILD_SENDS: u32 = 24;
pub const DEFERRED_OWNER_FREE_HEAD: u32 = 24;
pub const DEFERRED_OWNER_LIVE_HEAD: u32 = 28;

/// `rw_system + 0x20`: the second deferred-object list drained during an audio update.
pub const RELEASE_HEAD: u32 = 32;

/// The two indirect operations in `sub_82B48440`.
///
/// The retail functions are not yet recoverable as guest-only operations: `sub_82B48BB8` is the
/// per-record scheduler callback and `sub_82B49280` enters the allocator. This boundary retains
/// their exact call order and all surrounding guest-memory work without inventing either service.
pub trait DeferredReleaseHost {
    /// `sub_82B48BB8(system + 112, id)` for one record id.
    fn requeue_id(&mut self, g: &mut Guest, scheduler: u32, id: u32) -> Result<()>;
    /// `sub_82B49280(object)` after its link is detached and state byte cleared.
    fn release_deferred(&mut self, g: &mut Guest, object: u32) -> Result<()>;
}

const _: () = assert!(SYSTEM_SCHEDULER == 0x70, "rw_system.scheduler");
const _: () = assert!(SCHED_CURRENT == 0x44, "rw_scheduler.current_node");
const _: () = assert!(SCHED_PENDING_BUCKET == 0x48, "rw_scheduler._pad48");
const _: () = assert!(SCHED_NODE_REMOVED == 0x4C, "rw_scheduler.node_removed");
const _: () = assert!(BUCKET_HEAD_A == 0x10, "rw_scheduler.buckets[0]");
const _: () = assert!(BUCKET_STRIDE == 0x20, "rw_scheduler.buckets stride");
const _: () = assert!(INSTANCE_ELAPSED == 0x10, "rw_instance.elapsed");
const _: () = assert!(NODE_INSTANCE == 0x08, "rw_node.instance");

/// A host for the two non-memory operations in the scheduler tick.
///
/// `process` receives the instance context and the scheduler's single-precision delta. A process
/// can call [`detach_instance`] while it runs; the tick observes that through `SCHED_NODE_REMOVED`
/// and returns the flagged node to its original bucket afterwards.
pub trait SchedulerHost {
    /// `sub_82B1F7E8`: the low 32 bits of the timebase used for optional profiling.
    fn timebase(&mut self) -> u32;
    /// The instance's indirect `+4` function, called with `instance + 8` as its context.
    fn process(&mut self, g: &mut Guest, function: u32, context: u32, delta: f32) -> Result<()>;
}

/// `sub_82B48A50`: tick one scheduler bucket.
///
/// The traversal reads the successor before invoking the process callback. The list's word at
/// `node + 0` is named [`NODE_PREV`] by the unlink routine, yet this specific walker follows that
/// word exactly as the lifted function does. It must not be changed to [`NODE_NEXT`] merely to
/// make the local field name read more naturally.
pub fn tick_bucket<H: SchedulerHost + ?Sized>(
    g: &mut Guest,
    host: &mut H,
    scheduler: u32,
    bucket: u32,
) -> Result<()> {
    let manager = scheduler.wrapping_add(bucket.wrapping_shl(5) & 0xffff_ffe0);
    let mut node = g.u32(manager + BUCKET_HEAD_A)?;
    while node != 0 {
        let started = host.timebase();
        let instance = g.u32(node + NODE_INSTANCE)?;
        // `lwz r27,0(r31)`: this is the tick successor, read before user code may unlink node.
        let next = g.u32(node + NODE_PREV)?;
        g.set_u32(scheduler + SCHED_CURRENT, instance)?;
        let delta = g.f32(scheduler + 64)?;
        g.set_u32(scheduler + SCHED_NODE_REMOVED, 0)?;
        let context = g.u32(instance + 8)?;
        let process = g.u32(instance + 4)?;
        host.process(g, process, context, delta)?;
        let removed = g.u32(scheduler + SCHED_NODE_REMOVED)?;
        g.set_u32(scheduler + SCHED_CURRENT, 0)?;
        if removed == 0 {
            let descriptor = g.u32(instance)?;
            if g.u8(descriptor + 12)? != 0 {
                g.set_u32(
                    instance + INSTANCE_ELAPSED,
                    host.timebase().wrapping_sub(started),
                )?;
            } else {
                g.set_u32(instance + INSTANCE_ELAPSED, 0)?;
            }
        } else {
            let removed_bucket = g.u32(scheduler + SCHED_PENDING_BUCKET)?;
            let removed_manager =
                scheduler.wrapping_add(removed_bucket.wrapping_shl(5) & 0xffff_ffe0);
            recycle_node(g, removed_manager, removed)?;
            g.set_u32(scheduler + SCHED_NODE_REMOVED, 0)?;
        }
        node = next;
    }
    Ok(())
}

/// `sub_82B48B28`: make an element runnable in the scheduler bucket selected by its state.
///
/// This is the small hand-off used by the system's deferred-list drain. The node's `+12` is a
/// pending-list flag here: when it is set, unlink from the bucket's `+16` head and insert at the
/// front of `+20`; when it is clear the node is already runnable and only `element + 16` is
/// cleared. State three is the one early-return path and writes nothing at all.
pub fn requeue_element(g: &mut Guest, scheduler: u32, element: u32) -> Result<()> {
    let state = g.u8(element + REQUEUE_STATE)?;
    if state == REQUEUE_PARKED {
        return Ok(());
    }

    let node = g.u32(element + REQUEUE_NODE)?;
    let manager = scheduler.wrapping_add(u32::from(state).wrapping_shl(5));
    if g.u8(node + NODE_WHICH)? != 0 {
        if g.u32(manager + BUCKET_HEAD_A)? == node {
            g.set_u32(manager + BUCKET_HEAD_A, g.u32(node + NODE_NEXT)?)?;
        }
        let previous = g.u32(node + NODE_PREV)?;
        if previous != 0 {
            g.set_u32(previous + NODE_NEXT, g.u32(node + NODE_NEXT)?)?;
        }
        let next = g.u32(node + NODE_NEXT)?;
        if next != 0 {
            g.set_u32(next + NODE_PREV, g.u32(node + NODE_PREV)?)?;
        }

        let head = g.u32(manager + BUCKET_HEAD_B)?;
        g.set_u32(node + NODE_PREV, 0)?;
        g.set_u32(node + NODE_NEXT, head)?;
        if head != 0 {
            g.set_u32(head + NODE_PREV, node)?;
        }
        g.set_u32(manager + BUCKET_HEAD_B, node)?;
        g.set_u8(node + NODE_WHICH, 0)?;
    }
    g.set_u32(element + REQUEUE_CLEARED, 0)
}

/// `sub_82B482F8`: drain the system's deferred elements.
///
/// Each element first returns every child send to the scheduler through
/// [`requeue_element`], then leaves its owner's live list. If its handle-array entry is still
/// present, the element becomes the owner's free-list head and its transient state is reset. The
/// successor is read before the unlink, matching the guest list walk.
pub fn drain_deferred_sends(g: &mut Guest, system: u32) -> Result<()> {
    let mut node = g.u32(system + DEFERRED_HEAD)?;
    if node == 0 {
        return Ok(());
    }
    let zero = fp::load_single(g, modules::ZERO)?;
    while node != 0 {
        let element = node.wrapping_sub(DEFERRED_NODE);
        let child_count = g.u8(element + DEFERRED_CHILD_COUNT)?;
        let next_walk = g.u32(node + NODE_NEXT)?;

        for child_index in 0..u32::from(child_count) {
            let child = g.u32(element + DEFERRED_CHILDREN + child_index * 4)?;
            let send_count = g.u8(child + DEFERRED_CHILD_SEND_COUNT)?;
            for send_index in 0..u32::from(send_count) {
                let send = g.u32(child + DEFERRED_CHILD_SENDS + send_index * 4)?;
                requeue_element(g, system + SYSTEM_SCHEDULER, send)?;
            }
        }

        let owner_before = g.u32(element + DEFERRED_OWNER)?;
        let self_node = element + DEFERRED_NODE;
        if g.u32(owner_before + DEFERRED_OWNER_LIVE_HEAD)? == self_node {
            g.set_u32(
                owner_before + DEFERRED_OWNER_LIVE_HEAD,
                g.u32(self_node + NODE_NEXT)?,
            )?;
        }
        let previous = g.u32(self_node + NODE_PREV)?;
        if previous != 0 {
            g.set_u32(previous + NODE_NEXT, g.u32(self_node + NODE_NEXT)?)?;
        }
        let next = g.u32(self_node + NODE_NEXT)?;
        if next != 0 {
            g.set_u32(next + NODE_PREV, g.u32(self_node + NODE_PREV)?)?;
        }
        g.set_u8(element + DEFERRED_STATE, 0)?;

        if voices::remove_handle(g, element)? & 0xff != 0 {
            let owner = g.u32(element + DEFERRED_OWNER)?;
            let free_head = g.u32(owner + DEFERRED_OWNER_FREE_HEAD)?;
            g.set_u32(self_node + NODE_PREV, 0)?;
            g.set_u32(self_node + NODE_NEXT, free_head)?;
            let free_head_again = g.u32(owner + DEFERRED_OWNER_FREE_HEAD)?;
            if free_head_again != 0 {
                g.set_u32(free_head_again + NODE_PREV, self_node)?;
            }
            g.set_u32(owner + DEFERRED_OWNER_FREE_HEAD, self_node)?;
            fp::store_single(g, element, zero)?;
            fp::store_single(g, element + 4, zero)?;
            g.set_u8(element + DEFERRED_STATE, 4)?;
            fp::store_single(g, element + 8, zero)?;
            g.set_u32(element + DEFERRED_WORD_60, 0)?;
        }
        node = next_walk;
    }
    Ok(())
}

/// `sub_82B48440`: drain the second deferred-object list.
///
/// Every object's record ids are offered to [`DeferredReleaseHost::requeue_id`] before the object
/// is unlinked. The walk reads its successor first, so a release may reclaim the current object
/// without skipping the next one. The record-id and object-slot counts are reloaded at the same
/// loop boundaries as the guest code.
pub fn drain_deferred_releases<H: DeferredReleaseHost + ?Sized>(
    g: &mut Guest,
    host: &mut H,
    system: u32,
) -> Result<()> {
    let mut link = g.u32(system + RELEASE_HEAD)?;
    while link != 0 {
        let object = link.wrapping_sub(DEFERRED_NODE);
        let slots = g.u8(object + DEFERRED_CHILD_COUNT)?;
        let next_walk = g.u32(link + NODE_NEXT)?;

        if slots != 0 {
            let mut slot_index = 0u32;
            loop {
                let record = g.u32(object + DEFERRED_CHILDREN + slot_index * 4)?;
                if g.u8(record + DEFERRED_CHILD_SEND_COUNT)? != 0 {
                    let mut id_index = 0u32;
                    loop {
                        // The guest biases the record by +20 then pre-increments before its load,
                        // so id zero is stored at +24.
                        let id = g.u32(record + 24 + id_index * 4)?;
                        host.requeue_id(g, system + SYSTEM_SCHEDULER, id)?;
                        id_index += 1;
                        // This count is reloaded after each callback; both shortening and growing
                        // a record's list are visible to the same pass.
                        if id_index >= u32::from(g.u8(record + DEFERRED_CHILD_SEND_COUNT)?) {
                            break;
                        }
                    }
                }
                slot_index += 1;
                // The containing slot count is also reloaded after every record.
                if slot_index >= u32::from(g.u8(object + DEFERRED_CHILD_COUNT)?) {
                    break;
                }
            }
        }

        let owner = g.u32(object + DEFERRED_OWNER)?;
        let node = object + DEFERRED_NODE;
        if g.u32(owner + RELEASE_HEAD)? == node {
            g.set_u32(owner + RELEASE_HEAD, g.u32(node + NODE_NEXT)?)?;
        }
        let previous = g.u32(node + NODE_PREV)?;
        if previous != 0 {
            g.set_u32(previous + NODE_NEXT, g.u32(node + NODE_NEXT)?)?;
        }
        let next = g.u32(node + NODE_NEXT)?;
        if next != 0 {
            g.set_u32(next + NODE_PREV, g.u32(node + NODE_PREV)?)?;
        }
        g.set_u8(object + DEFERRED_STATE, 0)?;
        host.release_deferred(g, object)?;
        link = next_walk;
    }
    Ok(())
}

/// `sub_82B39690`: unlink a node from whichever of the manager's two lists names it, then push it
/// onto the manager's free list and drop the live count by one.
///
/// Writes: one bucket head when the node *is* that head; `next + 0` and `prev + 4` when those
/// neighbours are non-null; the node's own two link words; `free_head + 4` when the free list is
/// non-empty; the free head; and the count. Seven spans at most.
///
/// **Two reloads are reproduced rather than tidied.** The values written into the neighbours are
/// re-read from the node between the two stores, and the free head is read a second time after
/// the node's link words are written. The original's compiler could not prove those stores miss
/// the cells it reloads, and neither can this: if the free-head cell lived inside the node's two
/// link words the second read would return a value produced *during* the call. The C++
/// `Windows()` predicate refuses exactly that input as gate 2; there is no analogue of that
/// refusal here, because nothing in Rust is being bracketed, but the reload is what makes the two
/// agree if a caller ever lays memory out that way.
///
/// The count's `addi r11,r11,-1` is 64-bit in the lifted form and only its low word is stored, so
/// a wrapping 32-bit subtract leaves memory byte-identical. Nothing observes the register.
pub fn recycle_node(g: &mut Guest, manager: u32, node: u32) -> Result<()> {
    // lbz r11,12(r4) — which list head is allowed to name this node.
    let which = g.u8(node + NODE_WHICH)?;
    let head_field = if which != 0 {
        BUCKET_HEAD_A
    } else {
        BUCKET_HEAD_B
    };
    let head = g.u32(manager + head_field)?;
    if node == head {
        // stw r11,16/20(r3) — the head steps back to the node's prev link.
        let stepped = g.u32(head + NODE_PREV)?;
        g.set_u32(manager + head_field, stepped)?;
    }

    // `cmpwi cr6,rX,0` is an equality against zero, so the signed form the original uses cannot
    // differ from an unsigned test; the cast is kept so the transcription reads one-to-one.
    let next = g.u32(node + NODE_NEXT)?;
    if next as i32 != 0 {
        let prev_link = g.u32(node + NODE_PREV)?; // re-read, as the original does
        g.set_u32(next + NODE_PREV, prev_link)?; // stw r10,0(r11)
    }
    let prev = g.u32(node + NODE_PREV)?;
    if prev as i32 != 0 {
        let next_link = g.u32(node + NODE_NEXT)?; // re-read
        g.set_u32(prev + NODE_NEXT, next_link)?; // stw r10,4(r11)
    }

    // Push onto the free list.
    let free_head = g.u32(manager + BUCKET_FREE_HEAD)?;
    g.set_u32(node + NODE_NEXT, 0)?; // stw r10,4(r4)
    g.set_u32(node + NODE_PREV, free_head)?; // stw r11,0(r4)
    let free_again = g.u32(manager + BUCKET_FREE_HEAD)?; // lwz r11,12(r3) again
    if free_again != 0 {
        g.set_u32(free_again + NODE_NEXT, node)?; // stw r4,4(r11)
    }
    g.set_u32(manager + BUCKET_FREE_HEAD, node)?; // stw r4,12(r3)
    let count = g.u32(manager + BUCKET_COUNT)?;
    g.set_u32(manager + BUCKET_COUNT, count.wrapping_sub(1))?;
    Ok(())
}

/// `sub_82B489D0`: detach a scheduler instance — unlink its node now, or park the removal for the
/// tick to finish when the instance is the one the tick is currently running.
///
/// `system` is the guest's **full 64-bit `r3`**, not a guest address, because the return value
/// depends on its high half; addressing uses its low word. `instance` is `r4`.
///
/// Three paths, all selected from entry state (both tests load before the first store):
///
/// - the instance is `scheduler.current_node`: the bucket index goes to `scheduler + 72` and the
///   node to `scheduler + 76` for [the tick](sub_82B48A50) to unlink after the process function
///   returns. **There is no bucket test on this path** — the running instance is parked whatever
///   its bucket byte says, including 3;
/// - otherwise, and the bucket byte is not 3: the node is unlinked immediately through
///   [`recycle_node`];
/// - otherwise: nothing but the tail.
///
/// Writes: `instance + 16` and `instance + 20` on every path; `scheduler + 72` and
/// `scheduler + 76` on the parked path; `instance + 0` and `node + 8` on both non-trivial paths;
/// plus [`recycle_node`]'s set on the unlink path.
///
/// Returns the guest's `r3` on exit. The function is void by use, and the harness compares `r3`
/// anyway because it is reproducible and free: untouched on the parked and already-detached
/// paths, and `(bucket * 32) + (r3 + 112)` on the unlink path — where that second addend is a
/// **64-bit** `add` on `addi r11,r3,112`, so the high half of the entry `r3` is carried through
/// deliberately. Truncating it to 32 bits would leave every byte of memory identical and the
/// register wrong, which is the failure mode `counter::advance` documents.
///
/// **A null node is an error here, not a wild write.** The original would store to guest address
/// 8 and then hand null to `sub_82B39690`; the C++ `Windows()` refuses to bracket that, so it was
/// never compared in either language. Reproducing a write to address 8 would be inventing
/// behaviour, so it surfaces as an out-of-segment [`crate::Error`], the same choice
/// `eval::state::op_shuffle_bag` makes for its uncomparable input.
pub fn detach_instance(g: &mut Guest, system: u64, instance: u32) -> Result<u64> {
    // addi r11,r3,112 — kept 64-bit, because it is one operand of the 64-bit `add` that forms
    // recycle_node's first argument, so the high half of the entry r3 has to survive.
    let scheduler_reg = (system as i64).wrapping_add(SYSTEM_SCHEDULER as i64) as u64;
    let scheduler = scheduler_reg as u32;

    let current = g.u32(scheduler + SCHED_CURRENT)?; // lwz r10,180(r3)
    let bucket = g.u8(instance + INSTANCE_BUCKET)? as u32; // lbz r10,20(r4)

    // r3 is left as it arrived unless the unlink path rewrites it.
    let mut result = system;

    // cmplw cr6,r4,r10 ; bne — fall through only when this instance is the running one.
    if instance == current {
        g.set_u32(scheduler + SCHED_PENDING_BUCKET, bucket)?; // stw r10,72(r11)
        // lwz r7,0(r4) — loaded AFTER the store above, and reloaded rather than hoisted.
        let node = g.u32(instance + INSTANCE_NODE)?;
        g.set_u32(instance + INSTANCE_NODE, 0)?; // stw r8,0(r4)
        g.set_u32(node + NODE_INSTANCE, 0)?; // stw r8,8(r7)
        g.set_u32(scheduler + SCHED_NODE_REMOVED, node)?; // stw r7,76(r11)
    } else if bucket != DETACHED {
        // cmplwi cr6,r10,3 ; beq
        let node = g.u32(instance + INSTANCE_NODE)?; // lwz r4,0(r9)
        // rlwinm r10,r10,5,0,26 on a zero-extended byte: bucket*32, low word masked to ~31.
        let bucket_base = ((bucket as u64) << 5) & 0xFFFF_FFE0;
        g.set_u32(instance + INSTANCE_NODE, 0)?; // stw r8,0(r9)
        g.set_u32(node + NODE_INSTANCE, 0)?; // stw r8,8(r4)
        // add r3,r10,r11 ; bl 0x82b39690 — a 64-bit add, so the argument carries the entry r3's
        // high half, and sub_82B39690 leaves r3 alone, so this is also the returned value.
        result = bucket_base.wrapping_add(scheduler_reg);
        recycle_node(g, result as u32, node)?;
    }

    // loc_82B48A30 — the tail runs on all three paths.
    g.set_u32(instance + INSTANCE_ELAPSED, 0)?; // stw r8,16(r9)
    g.set_u8(instance + INSTANCE_BUCKET, DETACHED as u8)?; // stb r11,20(r9)
    Ok(result)
}

// ------------------------------------------------------------- sub_82B1DC00: release by key

/// `lis -31997 ; addi 28492` — a global list and the record that follows its head.
pub const KEYED_LIST: u32 = (((-31997i32 as u32) & 0xFFFF) << 16) + 28492;
/// `lwz r11,8(r8)` — the list head, and the emptiness test.
pub const KEYED_LIST_HEAD: u32 = 8;
/// `addi r4,r8,12` — the record [`detach_instance`] is handed when the list empties.
pub const KEYED_LIST_RECORD: u32 = 12;
/// `lis -31993 ; lwz 30252` — the word [`detach_instance`] is handed as its system.
pub const KEYED_OWNER_SLOT: u32 = (((-31993i32 as u32) & 0xFFFF) << 16) + 30252;
const _: () = assert!(KEYED_LIST == 0x8303_6F4C && KEYED_OWNER_SLOT == 0x8307_762C);
/// The node sits 80 bytes into its container.
pub const KEYED_NODE_IN_CONTAINER: u32 = 80;
/// `lwz r7,60(r10)` — the container's key.
pub const KEYED_CONTAINER_KEY: u32 = 60;
/// `lwz r9,8(r9)` — the key the argument asks for.
pub const KEYED_ARG_KEY: u32 = 8;
const KEYED_NEXT: u32 = 0;
const KEYED_PREV: u32 = 4;

/// Find the list entry whose container key matches the argument's, unlink it, and when that empties
/// the list hand the list's record to [`detach_instance`] (`sub_82B1DC00`). Returns 12 on every path.
///
/// The walk is unbounded, as the original's is: it stops at the match or at a null next pointer. The
/// head word is **reloaded after each fixup store**, and the node's next pointer is re-read after the
/// previous node's store, so a list whose links alias the head sees the original's values. The
/// original's 96-byte frame is not reproduced; [`detach_instance`] needs none.
pub fn release_by_key(g: &mut Guest, arg: u32) -> Result<u64> {
    let owner = g.u32(KEYED_OWNER_SLOT)?; // lwz r3,30252(r10)
    let head = g.u32(KEYED_LIST + KEYED_LIST_HEAD)?; // lwz r11,8(r8)
    let mut found = None;
    if head != 0 {
        let key = g.u32(arg.wrapping_add(KEYED_ARG_KEY))?; // lwz r9,8(r9)
        let mut cursor = head;
        loop {
            let container = cursor.wrapping_sub(KEYED_NODE_IN_CONTAINER); // addi r10,r10,-80
            if g.u32(container.wrapping_add(KEYED_CONTAINER_KEY))? == key {
                found = Some(cursor); // cmpw cr6,r9,r7 ; beq
                break;
            }
            cursor = g.u32(container.wrapping_add(KEYED_NODE_IN_CONTAINER))?; // lwz r10,80(r10)
            if cursor == 0 {
                break;
            }
        }
    }
    if let Some(node) = found {
        let mut head_now = head;
        if node == head {
            head_now = g.u32(head.wrapping_add(KEYED_NEXT))?; // lwz r11,0(r11)
            g.set_u32(KEYED_LIST + KEYED_LIST_HEAD, head_now)?; // stw r11,8(r8)
        }
        let prev = g.u32(node.wrapping_add(KEYED_PREV))?; // lwz r9,4(r10)
        if prev != 0 {
            let next = g.u32(node.wrapping_add(KEYED_NEXT))?;
            g.set_u32(prev.wrapping_add(KEYED_NEXT), next)?; // stw r11,0(r9)
            head_now = g.u32(KEYED_LIST + KEYED_LIST_HEAD)?;
        }
        let next = g.u32(node.wrapping_add(KEYED_NEXT))?; // lwz r9,0(r10) -- re-read
        if next != 0 {
            let prev = g.u32(node.wrapping_add(KEYED_PREV))?;
            g.set_u32(next.wrapping_add(KEYED_PREV), prev)?; // stw r11,4(r9)
            head_now = g.u32(KEYED_LIST + KEYED_LIST_HEAD)?;
        }
        if head_now == 0 {
            detach_instance(g, u64::from(owner), KEYED_LIST + KEYED_LIST_RECORD)?;
            // bl 0x82b489d0
        }
    }
    Ok(12) // li r3,12
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Segment;

    const SYSTEM: u32 = 0x4000_0000;
    const SCHEDULER: u32 = SYSTEM + SYSTEM_SCHEDULER;
    const INSTANCE: u32 = 0x4000_0400;
    const NODE_A: u32 = 0x4000_0500;
    const NODE_B: u32 = 0x4000_0520;
    const NODE_C: u32 = 0x4000_0540;
    const FREE: u32 = 0x4000_0560;

    fn guest() -> Guest {
        Guest::from_segments(vec![
            Segment {
                base: SYSTEM,
                bytes: vec![0; 0x1000],
            },
            Segment {
                base: modules::ZERO,
                bytes: vec![0; 4],
            },
        ])
    }

    /// `prev <- node -> next`, with the node claimed by list A.
    fn link(g: &mut Guest, node: u32, prev: u32, next: u32, which: u8) {
        g.set_u32(node + NODE_PREV, prev).unwrap();
        g.set_u32(node + NODE_NEXT, next).unwrap();
        g.set_u8(node + NODE_WHICH, which).unwrap();
    }

    #[test]
    fn a_middle_node_is_unlinked_and_pushed_onto_an_empty_free_list() {
        let mut g = guest();
        // A <-> B <-> C, head A naming node A; B is the one leaving.
        link(&mut g, NODE_A, 0, NODE_B, 1);
        link(&mut g, NODE_B, NODE_A, NODE_C, 1);
        link(&mut g, NODE_C, NODE_B, 0, 1);
        g.set_u32(SCHEDULER + BUCKET_HEAD_A, NODE_C).unwrap();
        g.set_u32(SCHEDULER + BUCKET_COUNT, 3).unwrap();

        recycle_node(&mut g, SCHEDULER, NODE_B).unwrap();

        assert_eq!(
            g.u32(SCHEDULER + BUCKET_HEAD_A).unwrap(),
            NODE_C,
            "B was not the head"
        );
        assert_eq!(
            g.u32(NODE_C + NODE_PREV).unwrap(),
            NODE_A,
            "the next node skips B"
        );
        assert_eq!(
            g.u32(NODE_A + NODE_NEXT).unwrap(),
            NODE_C,
            "the prev node skips B"
        );
        assert_eq!(g.u32(NODE_B + NODE_NEXT).unwrap(), 0, "B's next is cleared");
        assert_eq!(
            g.u32(NODE_B + NODE_PREV).unwrap(),
            0,
            "the free list was empty"
        );
        assert_eq!(
            g.u32(SCHEDULER + BUCKET_FREE_HEAD).unwrap(),
            NODE_B,
            "B is the free head"
        );
        assert_eq!(
            g.u32(SCHEDULER + BUCKET_COUNT).unwrap(),
            2,
            "the count dropped by one"
        );
    }

    #[test]
    fn unlinking_the_head_steps_it_back_to_the_nodes_prev_link() {
        let mut g = guest();
        link(&mut g, NODE_A, 0, NODE_B, 1);
        link(&mut g, NODE_B, NODE_A, 0, 1);
        g.set_u32(SCHEDULER + BUCKET_HEAD_A, NODE_B).unwrap();
        g.set_u32(SCHEDULER + BUCKET_COUNT, 2).unwrap();

        recycle_node(&mut g, SCHEDULER, NODE_B).unwrap();

        assert_eq!(
            g.u32(SCHEDULER + BUCKET_HEAD_A).unwrap(),
            NODE_A,
            "the head stepped back"
        );
        assert_eq!(
            g.u32(NODE_A + NODE_NEXT).unwrap(),
            0,
            "and A is now the tail"
        );
        assert_eq!(g.u32(SCHEDULER + BUCKET_COUNT).unwrap(), 1);
    }

    #[test]
    fn requeue_moves_a_pending_node_to_the_runnable_head() {
        const ELEMENT: u32 = SYSTEM + 0x600;
        const PENDING_NEXT: u32 = SYSTEM + 0x620;
        const RUNNABLE: u32 = SYSTEM + 0x640;
        const STATE: u8 = 2;
        let manager = SCHEDULER + u32::from(STATE) * BUCKET_STRIDE;
        let mut g = guest();
        g.set_u32(ELEMENT + REQUEUE_NODE, NODE_B).unwrap();
        g.set_u8(ELEMENT + REQUEUE_STATE, STATE).unwrap();
        g.set_u32(ELEMENT + REQUEUE_CLEARED, 0xDEAD_BEEF).unwrap();
        g.set_u32(manager + BUCKET_HEAD_A, NODE_B).unwrap();
        g.set_u32(manager + BUCKET_HEAD_B, RUNNABLE).unwrap();
        g.set_u8(NODE_B + NODE_WHICH, 1).unwrap();
        g.set_u32(NODE_B + NODE_PREV, 0).unwrap();
        g.set_u32(NODE_B + NODE_NEXT, PENDING_NEXT).unwrap();
        g.set_u32(PENDING_NEXT + NODE_PREV, NODE_B).unwrap();
        g.set_u32(RUNNABLE + NODE_PREV, 0).unwrap();

        requeue_element(&mut g, SCHEDULER, ELEMENT).unwrap();

        assert_eq!(g.u32(manager + BUCKET_HEAD_A).unwrap(), PENDING_NEXT);
        assert_eq!(g.u32(PENDING_NEXT + NODE_PREV).unwrap(), 0);
        assert_eq!(g.u32(manager + BUCKET_HEAD_B).unwrap(), NODE_B);
        assert_eq!(g.u32(NODE_B + NODE_PREV).unwrap(), 0);
        assert_eq!(g.u32(NODE_B + NODE_NEXT).unwrap(), RUNNABLE);
        assert_eq!(g.u32(RUNNABLE + NODE_PREV).unwrap(), NODE_B);
        assert_eq!(g.u8(NODE_B + NODE_WHICH).unwrap(), 0);
        assert_eq!(g.u32(ELEMENT + REQUEUE_CLEARED).unwrap(), 0);
    }

    #[test]
    fn parked_requeue_leaves_everything_untouched() {
        const ELEMENT: u32 = SYSTEM + 0x600;
        let mut g = guest();
        g.set_u32(ELEMENT + REQUEUE_NODE, NODE_B).unwrap();
        g.set_u8(ELEMENT + REQUEUE_STATE, REQUEUE_PARKED).unwrap();
        g.set_u32(ELEMENT + REQUEUE_CLEARED, 0xDEAD_BEEF).unwrap();
        g.set_u32(NODE_B + NODE_NEXT, 0xCAFE_BABE).unwrap();

        requeue_element(&mut g, SCHEDULER, ELEMENT).unwrap();

        assert_eq!(g.u32(ELEMENT + REQUEUE_CLEARED).unwrap(), 0xDEAD_BEEF);
        assert_eq!(g.u32(NODE_B + NODE_NEXT).unwrap(), 0xCAFE_BABE);
    }

    #[test]
    fn deferred_drain_requeues_child_sends_and_recycles_the_retired_element() {
        const ELEMENT: u32 = SYSTEM + 0x600;
        const CHILD: u32 = SYSTEM + 0x700;
        const SEND: u32 = SYSTEM + 0x740;
        const SEND_NODE: u32 = SYSTEM + 0x780;
        const RUNNABLE: u32 = SYSTEM + 0x7A0;
        const HANDLES: u32 = SYSTEM + 0x900;
        const STATE: u8 = 1;
        let manager = SCHEDULER + u32::from(STATE) * BUCKET_STRIDE;
        let self_node = ELEMENT + DEFERRED_NODE;
        let mut g = guest();

        g.set_u32(SYSTEM + DEFERRED_HEAD, self_node).unwrap();
        g.set_u32(ELEMENT + DEFERRED_OWNER, SYSTEM).unwrap();
        g.set_u8(ELEMENT + DEFERRED_CHILD_COUNT, 1).unwrap();
        g.set_u32(ELEMENT + DEFERRED_CHILDREN, CHILD).unwrap();
        g.set_u32(self_node + NODE_NEXT, 0).unwrap();
        g.set_u32(self_node + NODE_PREV, 0).unwrap();
        g.set_u8(ELEMENT + DEFERRED_STATE, 1).unwrap();
        g.set_u32(ELEMENT, 1.0f32.to_bits()).unwrap();
        g.set_u32(ELEMENT + 4, 2.0f32.to_bits()).unwrap();
        g.set_u32(ELEMENT + 8, 3.0f32.to_bits()).unwrap();
        g.set_u32(ELEMENT + DEFERRED_WORD_60, 0xDEAD_BEEF).unwrap();

        g.set_u8(CHILD + DEFERRED_CHILD_SEND_COUNT, 1).unwrap();
        g.set_u32(CHILD + DEFERRED_CHILD_SENDS, SEND).unwrap();
        g.set_u32(SEND + REQUEUE_NODE, SEND_NODE).unwrap();
        g.set_u8(SEND + REQUEUE_STATE, STATE).unwrap();
        g.set_u32(SEND + REQUEUE_CLEARED, 0xCAFE_BABE).unwrap();
        g.set_u32(manager + BUCKET_HEAD_A, SEND_NODE).unwrap();
        g.set_u32(manager + BUCKET_HEAD_B, RUNNABLE).unwrap();
        g.set_u8(SEND_NODE + NODE_WHICH, 1).unwrap();
        g.set_u32(SEND_NODE + NODE_PREV, 0).unwrap();
        g.set_u32(SEND_NODE + NODE_NEXT, 0).unwrap();
        g.set_u32(RUNNABLE + NODE_PREV, 0).unwrap();

        g.set_u32(SYSTEM + voices::HANDLE_ARRAY, HANDLES).unwrap();
        g.set_u16(SYSTEM + voices::HANDLE_COUNT, 1).unwrap();
        g.set_u32(HANDLES, ELEMENT).unwrap();

        drain_deferred_sends(&mut g, SYSTEM).unwrap();

        assert_eq!(g.u32(manager + BUCKET_HEAD_A).unwrap(), 0);
        assert_eq!(g.u32(manager + BUCKET_HEAD_B).unwrap(), SEND_NODE);
        assert_eq!(g.u32(SEND_NODE + NODE_NEXT).unwrap(), RUNNABLE);
        assert_eq!(g.u32(RUNNABLE + NODE_PREV).unwrap(), SEND_NODE);
        assert_eq!(g.u8(SEND_NODE + NODE_WHICH).unwrap(), 0);
        assert_eq!(g.u32(SEND + REQUEUE_CLEARED).unwrap(), 0);

        assert_eq!(g.u32(SYSTEM + DEFERRED_OWNER_LIVE_HEAD).unwrap(), 0);
        assert_eq!(g.u32(SYSTEM + DEFERRED_OWNER_FREE_HEAD).unwrap(), self_node);
        assert_eq!(g.u32(self_node + NODE_PREV).unwrap(), 0);
        assert_eq!(g.u32(self_node + NODE_NEXT).unwrap(), 0);
        assert_eq!(g.u16(SYSTEM + voices::HANDLE_COUNT).unwrap(), 0);
        assert_eq!(g.u8(ELEMENT + DEFERRED_STATE).unwrap(), 4);
        assert_eq!(g.u32(ELEMENT).unwrap(), 0);
        assert_eq!(g.u32(ELEMENT + 4).unwrap(), 0);
        assert_eq!(g.u32(ELEMENT + 8).unwrap(), 0);
        assert_eq!(g.u32(ELEMENT + DEFERRED_WORD_60).unwrap(), 0);
    }

    #[derive(Default)]
    struct ReleaseHost {
        ids: Vec<(u32, u32)>,
        released: Vec<u32>,
    }

    impl DeferredReleaseHost for ReleaseHost {
        fn requeue_id(&mut self, _g: &mut Guest, scheduler: u32, id: u32) -> Result<()> {
            self.ids.push((scheduler, id));
            Ok(())
        }

        fn release_deferred(&mut self, _g: &mut Guest, object: u32) -> Result<()> {
            self.released.push(object);
            Ok(())
        }
    }

    #[test]
    fn release_drain_requeues_all_ids_then_unlinks_and_releases() {
        const OBJECT: u32 = SYSTEM + 0x600;
        const RECORD: u32 = SYSTEM + 0x700;
        let node = OBJECT + DEFERRED_NODE;
        let mut g = guest();
        g.set_u32(SYSTEM + RELEASE_HEAD, node).unwrap();
        g.set_u32(OBJECT + DEFERRED_OWNER, SYSTEM).unwrap();
        g.set_u32(node + NODE_NEXT, 0).unwrap();
        g.set_u32(node + NODE_PREV, 0).unwrap();
        g.set_u8(OBJECT + DEFERRED_STATE, 1).unwrap();
        g.set_u8(OBJECT + DEFERRED_CHILD_COUNT, 1).unwrap();
        g.set_u32(OBJECT + DEFERRED_CHILDREN, RECORD).unwrap();
        g.set_u8(RECORD + DEFERRED_CHILD_SEND_COUNT, 2).unwrap();
        g.set_u32(RECORD + 24, 0x1111).unwrap();
        g.set_u32(RECORD + 28, 0x2222).unwrap();
        let mut host = ReleaseHost::default();

        drain_deferred_releases(&mut g, &mut host, SYSTEM).unwrap();

        assert_eq!(host.ids, vec![(SCHEDULER, 0x1111), (SCHEDULER, 0x2222)]);
        assert_eq!(host.released, vec![OBJECT]);
        assert_eq!(g.u32(SYSTEM + RELEASE_HEAD).unwrap(), 0);
        assert_eq!(g.u8(OBJECT + DEFERRED_STATE).unwrap(), 0);
    }

    #[test]
    fn the_which_byte_decides_which_head_can_name_the_node() {
        let mut g = guest();
        // The node IS head B, but its `which` byte is non-zero, so only head A is tested — and
        // head A does not name it, so neither head moves. Getting this backwards would corrupt
        // list B's head into the node's prev link.
        link(&mut g, NODE_B, NODE_A, 0, 1);
        g.set_u32(SCHEDULER + BUCKET_HEAD_A, NODE_C).unwrap();
        g.set_u32(SCHEDULER + BUCKET_HEAD_B, NODE_B).unwrap();
        g.set_u32(SCHEDULER + BUCKET_COUNT, 1).unwrap();

        recycle_node(&mut g, SCHEDULER, NODE_B).unwrap();
        assert_eq!(
            g.u32(SCHEDULER + BUCKET_HEAD_A).unwrap(),
            NODE_C,
            "head A is untouched"
        );
        assert_eq!(
            g.u32(SCHEDULER + BUCKET_HEAD_B).unwrap(),
            NODE_B,
            "head B is untouched too"
        );

        // With the byte clear, head B is the one tested, and it does name the node.
        let mut g = guest();
        link(&mut g, NODE_B, NODE_A, 0, 0);
        g.set_u32(SCHEDULER + BUCKET_HEAD_A, NODE_C).unwrap();
        g.set_u32(SCHEDULER + BUCKET_HEAD_B, NODE_B).unwrap();
        recycle_node(&mut g, SCHEDULER, NODE_B).unwrap();
        assert_eq!(
            g.u32(SCHEDULER + BUCKET_HEAD_B).unwrap(),
            NODE_A,
            "head B stepped back"
        );
        assert_eq!(
            g.u32(SCHEDULER + BUCKET_HEAD_A).unwrap(),
            NODE_C,
            "head A still untouched"
        );
    }

    #[test]
    fn a_non_empty_free_list_gets_a_back_link_to_the_recycled_node() {
        let mut g = guest();
        link(&mut g, NODE_B, 0, 0, 1);
        link(&mut g, FREE, 0, 0, 0);
        g.set_u32(SCHEDULER + BUCKET_FREE_HEAD, FREE).unwrap();
        g.set_u32(SCHEDULER + BUCKET_COUNT, 1).unwrap();

        recycle_node(&mut g, SCHEDULER, NODE_B).unwrap();

        assert_eq!(
            g.u32(NODE_B + NODE_PREV).unwrap(),
            FREE,
            "the node points at the old head"
        );
        assert_eq!(
            g.u32(FREE + NODE_NEXT).unwrap(),
            NODE_B,
            "and the old head points back"
        );
        assert_eq!(g.u32(SCHEDULER + BUCKET_FREE_HEAD).unwrap(), NODE_B);
    }

    #[test]
    fn the_count_wraps_rather_than_saturating_when_the_bucket_is_already_empty() {
        // `addi r11,r11,-1` on a zero word leaves 0xFFFFFFFF in memory. Nothing in the original
        // guards this, and a saturating subtract would differ by four bytes.
        let mut g = guest();
        link(&mut g, NODE_B, 0, 0, 1);
        g.set_u32(SCHEDULER + BUCKET_COUNT, 0).unwrap();
        recycle_node(&mut g, SCHEDULER, NODE_B).unwrap();
        assert_eq!(g.u32(SCHEDULER + BUCKET_COUNT).unwrap(), 0xFFFF_FFFF);
    }

    #[test]
    fn the_free_head_is_re_read_after_the_nodes_link_words_are_written() {
        // **This input was never compared against the original.** The C++ `Windows()` predicate
        // refuses to bracket a layout where the manager's free-head cell lives inside the node's
        // two link words, because then the reloaded head is a value produced *during* the call —
        // that is its gate-2 exit. So what this test pins is that the reload is still in the
        // transcription, not that the guest agrees with the answer; nothing establishes the
        // latter, and the C++ port would decline to run at all here.
        //
        // The layout: the node sits eight bytes into the manager, so `node + 4` IS the free head.
        // Clearing the node's next link therefore empties the free list mid-call, and the reload
        // sees that. Reusing the value read before the store would instead write a back link into
        // the old free head.
        let mut g = guest();
        let manager = SYSTEM;
        let node = manager + 8;
        g.set_u32(manager + BUCKET_HEAD_B, 0).unwrap(); // makes the `which` byte read 0
        g.set_u32(node + NODE_PREV, 0).unwrap();
        g.set_u32(manager + BUCKET_FREE_HEAD, FREE).unwrap();
        g.set_u32(FREE + NODE_NEXT, 0xFEED_FACE).unwrap();
        g.set_u32(manager + BUCKET_COUNT, 1).unwrap();

        recycle_node(&mut g, manager, node).unwrap();

        assert_eq!(
            g.u32(FREE + NODE_NEXT).unwrap(),
            0xFEED_FACE,
            "the reload saw an empty free list, so no back link was written"
        );
        assert_eq!(
            g.u32(manager + BUCKET_FREE_HEAD).unwrap(),
            node,
            "the push still happened"
        );
    }

    /// An instance linked into bucket `bucket` through `node`.
    fn instance(g: &mut Guest, node: u32, bucket: u8) {
        g.set_u32(INSTANCE + INSTANCE_NODE, node).unwrap();
        g.set_u32(INSTANCE + INSTANCE_ELAPSED, 0x1234).unwrap();
        g.set_u8(INSTANCE + INSTANCE_BUCKET, bucket).unwrap();
        g.set_u32(node + NODE_INSTANCE, INSTANCE).unwrap();
    }

    #[test]
    fn the_running_instance_parks_its_removal_for_the_tick() {
        let mut g = guest();
        instance(&mut g, NODE_B, 1);
        link(&mut g, NODE_B, NODE_A, 0, 1);
        g.set_u32(SCHEDULER + BUCKET_HEAD_A, NODE_B).unwrap();
        g.set_u32(SCHEDULER + BUCKET_COUNT, 5).unwrap();
        g.set_u32(SCHEDULER + SCHED_CURRENT, INSTANCE).unwrap();

        let r3 = detach_instance(&mut g, SYSTEM as u64, INSTANCE).unwrap();

        assert_eq!(
            g.u32(SCHEDULER + SCHED_PENDING_BUCKET).unwrap(),
            1,
            "the bucket is parked"
        );
        assert_eq!(
            g.u32(SCHEDULER + SCHED_NODE_REMOVED).unwrap(),
            NODE_B,
            "and so is the node"
        );
        assert_eq!(g.u32(INSTANCE + INSTANCE_NODE).unwrap(), 0);
        assert_eq!(
            g.u32(NODE_B + NODE_INSTANCE).unwrap(),
            0,
            "the back pointer is cleared"
        );
        // The list itself is NOT touched: that is the tick's job, once its callee returns.
        assert_eq!(
            g.u32(SCHEDULER + BUCKET_HEAD_A).unwrap(),
            NODE_B,
            "the head still names it"
        );
        assert_eq!(
            g.u32(SCHEDULER + BUCKET_COUNT).unwrap(),
            5,
            "and the count is unchanged"
        );
        assert_eq!(
            g.u32(SCHEDULER + BUCKET_FREE_HEAD).unwrap(),
            0,
            "nothing was recycled"
        );
        // The tail, on this path as on every other.
        assert_eq!(g.u32(INSTANCE + INSTANCE_ELAPSED).unwrap(), 0);
        assert_eq!(g.u8(INSTANCE + INSTANCE_BUCKET).unwrap(), 3);
        assert_eq!(r3, SYSTEM as u64, "r3 is untouched on the parked path");
    }

    #[test]
    fn the_parked_path_ignores_the_bucket_byte_entirely() {
        // An already-detached instance that happens to be the running one still parks. Adding the
        // bucket test the other path has would take the do-nothing path instead and lose the node.
        let mut g = guest();
        instance(&mut g, NODE_B, DETACHED as u8);
        g.set_u32(SCHEDULER + SCHED_CURRENT, INSTANCE).unwrap();

        detach_instance(&mut g, SYSTEM as u64, INSTANCE).unwrap();

        assert_eq!(
            g.u32(SCHEDULER + SCHED_PENDING_BUCKET).unwrap(),
            3,
            "parked as bucket 3"
        );
        assert_eq!(g.u32(SCHEDULER + SCHED_NODE_REMOVED).unwrap(), NODE_B);
        assert_eq!(g.u32(INSTANCE + INSTANCE_NODE).unwrap(), 0);
    }

    #[test]
    fn an_already_detached_instance_writes_only_the_tail() {
        let mut g = guest();
        instance(&mut g, NODE_B, DETACHED as u8);
        link(&mut g, NODE_B, NODE_A, 0, 1);
        g.set_u32(SCHEDULER + SCHED_CURRENT, 0x4000_0900).unwrap(); // some other instance
        g.set_u32(SCHEDULER + BUCKET_COUNT, 7).unwrap();

        let r3 = detach_instance(&mut g, SYSTEM as u64, INSTANCE).unwrap();

        assert_eq!(
            g.u32(INSTANCE + INSTANCE_NODE).unwrap(),
            NODE_B,
            "the node pointer stands"
        );
        assert_eq!(
            g.u32(NODE_B + NODE_INSTANCE).unwrap(),
            INSTANCE,
            "and so does the back link"
        );
        assert_eq!(
            g.u32(SCHEDULER + SCHED_PENDING_BUCKET).unwrap(),
            0,
            "nothing was parked"
        );
        assert_eq!(g.u32(SCHEDULER + SCHED_NODE_REMOVED).unwrap(), 0);
        assert_eq!(
            g.u32(SCHEDULER + BUCKET_COUNT).unwrap(),
            7,
            "and nothing was recycled"
        );
        assert_eq!(
            g.u32(INSTANCE + INSTANCE_ELAPSED).unwrap(),
            0,
            "the tail still runs"
        );
        assert_eq!(g.u8(INSTANCE + INSTANCE_BUCKET).unwrap(), 3);
        assert_eq!(r3, SYSTEM as u64);
    }

    #[test]
    fn a_linked_instance_is_unlinked_through_its_own_buckets_manager() {
        let mut g = guest();
        // Bucket 2, so the manager is scheduler + 64 and none of bucket 0's cells may move.
        instance(&mut g, NODE_B, 2);
        link(&mut g, NODE_A, 0, NODE_B, 1);
        link(&mut g, NODE_B, NODE_A, 0, 1);
        let manager = SCHEDULER + 2 * BUCKET_STRIDE;
        g.set_u32(manager + BUCKET_HEAD_A, NODE_B).unwrap();
        g.set_u32(manager + BUCKET_COUNT, 4).unwrap();
        g.set_u32(SCHEDULER + BUCKET_COUNT, 99).unwrap(); // bucket 0's count, a control
        g.set_u32(SCHEDULER + SCHED_CURRENT, 0).unwrap();

        let r3 = detach_instance(&mut g, SYSTEM as u64, INSTANCE).unwrap();

        assert_eq!(
            g.u32(manager + BUCKET_HEAD_A).unwrap(),
            NODE_A,
            "bucket 2's head stepped"
        );
        assert_eq!(
            g.u32(manager + BUCKET_FREE_HEAD).unwrap(),
            NODE_B,
            "recycled into bucket 2"
        );
        assert_eq!(g.u32(manager + BUCKET_COUNT).unwrap(), 3);
        assert_eq!(
            g.u32(SCHEDULER + BUCKET_COUNT).unwrap(),
            99,
            "bucket 0 was not touched"
        );
        assert_eq!(g.u32(INSTANCE + INSTANCE_NODE).unwrap(), 0);
        assert_eq!(g.u32(NODE_B + NODE_INSTANCE).unwrap(), 0);
        assert_eq!(
            g.u32(SCHEDULER + SCHED_PENDING_BUCKET).unwrap(),
            0,
            "nothing was parked"
        );
        assert_eq!(g.u8(INSTANCE + INSTANCE_BUCKET).unwrap(), 3);
        // r3 on the unlink path is the manager address the callee was handed.
        assert_eq!(r3, (SYSTEM + SYSTEM_SCHEDULER + 2 * BUCKET_STRIDE) as u64);
    }

    #[test]
    fn a_null_node_is_an_error_rather_than_a_write_to_guest_address_eight() {
        // A deliberate divergence, pinned here the way the crate's other three are. The original
        // would store through `node + 8` with `node` null and then hand null to `sub_82B39690`;
        // the C++ `Windows()` refuses that input, so it was never compared in either language and
        // nothing is known about what the guest does with it. Inventing a write to address 8 would
        // turn a gap in coverage into a wrong answer.
        let mut g = guest();
        instance(&mut g, NODE_B, 1);
        g.set_u32(INSTANCE + INSTANCE_NODE, 0).unwrap();
        g.set_u32(SCHEDULER + SCHED_CURRENT, 0).unwrap();

        let err = detach_instance(&mut g, SYSTEM as u64, INSTANCE).unwrap_err();
        assert_eq!(
            err.address, NODE_INSTANCE,
            "the address the original would have stored to"
        );

        // Same on the parked path, which reaches the same store.
        let mut g = guest();
        instance(&mut g, NODE_B, 1);
        g.set_u32(INSTANCE + INSTANCE_NODE, 0).unwrap();
        g.set_u32(SCHEDULER + SCHED_CURRENT, INSTANCE).unwrap();
        assert!(detach_instance(&mut g, SYSTEM as u64, INSTANCE).is_err());
    }

    #[test]
    fn the_returned_manager_address_keeps_the_entry_r3s_high_half() {
        // `addi r11,r3,112` and the `add` that scales the bucket onto it are both 64-bit. A guest
        // pointer normally arrives zero-extended, so this only shows up in the register — which is
        // exactly the shape of the bug counter::advance hit on its 120th call: every store matches
        // and r3 is wrong. Addressing still uses the low word.
        let mut g = guest();
        instance(&mut g, NODE_B, 1);
        link(&mut g, NODE_B, 0, 0, 1);
        g.set_u32(SCHEDULER + SCHED_CURRENT, 0).unwrap();

        let wide = 0x0000_0007_0000_0000u64 | SYSTEM as u64;
        let r3 = detach_instance(&mut g, wide, INSTANCE).unwrap();

        assert_eq!(
            r3,
            0x0000_0007_0000_0000u64 + (SYSTEM + SYSTEM_SCHEDULER + 32) as u64
        );
        assert!(r3 > u32::MAX as u64, "the high half survives: {r3:#x}");
        // And the low word still addressed bucket 1's manager, 32 bytes into the scheduler.
        assert_eq!(
            g.u32(SCHEDULER + BUCKET_STRIDE + BUCKET_FREE_HEAD).unwrap(),
            NODE_B
        );
    }
}

#[cfg(test)]
mod release_by_key_tests {
    use super::*;
    use crate::Segment;

    const SYSTEM: u32 = 0x4000_0000;
    const ARG: u32 = SYSTEM + 0x700;
    const RECORD: u32 = KEYED_LIST + KEYED_LIST_RECORD;

    fn node(i: u32) -> u32 {
        SYSTEM + 0x100 * (i + 1) + KEYED_NODE_IN_CONTAINER
    }

    /// A doubly linked list with these keys, head first, and the record detached already (bucket 3)
    /// so that handing it back runs only the closing stores.
    fn guest(keys: &[u32], wanted: u32) -> Guest {
        let mut g = Guest::from_segments(vec![
            Segment {
                base: SYSTEM,
                bytes: vec![0u8; 0x800],
            },
            Segment {
                base: KEYED_LIST & !0xFF,
                bytes: vec![0u8; 0x100],
            },
            Segment {
                base: KEYED_OWNER_SLOT & !0xFF,
                bytes: vec![0u8; 0x100],
            },
        ]);
        g.set_u32(KEYED_OWNER_SLOT, SYSTEM).unwrap();
        let n = keys.len() as u32;
        for (i, &key) in keys.iter().enumerate() {
            let i = i as u32;
            g.set_u32(node(i) - KEYED_NODE_IN_CONTAINER + KEYED_CONTAINER_KEY, key)
                .unwrap();
            g.set_u32(
                node(i) + KEYED_NEXT,
                if i + 1 < n { node(i + 1) } else { 0 },
            )
            .unwrap();
            g.set_u32(node(i) + KEYED_PREV, if i > 0 { node(i - 1) } else { 0 })
                .unwrap();
        }
        g.set_u32(
            KEYED_LIST + KEYED_LIST_HEAD,
            if n == 0 { 0 } else { node(0) },
        )
        .unwrap();
        g.set_u32(ARG + KEYED_ARG_KEY, wanted).unwrap();
        g.set_u32(RECORD + 16, 0xDEAD).unwrap();
        g.set_u8(RECORD + 20, 3).unwrap();
        g
    }

    #[test]
    fn an_unknown_key_changes_nothing() {
        let mut g = guest(&[5, 6], 9);
        let before = g.clone();
        assert_eq!(release_by_key(&mut g, ARG).unwrap(), 12);
        for at in [
            KEYED_LIST + KEYED_LIST_HEAD,
            node(0),
            node(0) + 4,
            node(1),
            node(1) + 4,
            RECORD + 16,
        ] {
            assert_eq!(g.u32(at).unwrap(), before.u32(at).unwrap());
        }
    }

    #[test]
    fn a_middle_entry_is_unlinked_and_the_list_kept() {
        let mut g = guest(&[5, 6, 7], 6);
        release_by_key(&mut g, ARG).unwrap();
        assert_eq!(g.u32(node(0) + KEYED_NEXT).unwrap(), node(2));
        assert_eq!(g.u32(node(2) + KEYED_PREV).unwrap(), node(0));
        assert_eq!(g.u32(KEYED_LIST + KEYED_LIST_HEAD).unwrap(), node(0));
        assert_eq!(g.u32(RECORD + 16).unwrap(), 0xDEAD, "the list is not empty");
    }

    #[test]
    fn the_head_entry_moves_the_head_on() {
        let mut g = guest(&[5, 6], 5);
        release_by_key(&mut g, ARG).unwrap();
        assert_eq!(g.u32(KEYED_LIST + KEYED_LIST_HEAD).unwrap(), node(1));
        assert_eq!(g.u32(node(1) + KEYED_PREV).unwrap(), 0);
    }

    #[test]
    fn the_last_entry_empties_the_list_and_hands_the_record_back() {
        let mut g = guest(&[5], 5);
        assert_eq!(release_by_key(&mut g, ARG).unwrap(), 12);
        assert_eq!(g.u32(KEYED_LIST + KEYED_LIST_HEAD).unwrap(), 0);
        assert_eq!(
            g.u32(RECORD + 16).unwrap(),
            0,
            "detach_instance's closing store"
        );
        assert_eq!(g.u8(RECORD + 20).unwrap(), 3);
    }

    #[test]
    fn an_empty_list_does_not_read_the_key() {
        let mut g = guest(&[], 5);
        assert_eq!(
            release_by_key(&mut g, 0xFFFF_0000).unwrap(),
            12,
            "an unmapped argument is never read"
        );
    }
}

#[cfg(test)]
mod tick_tests {
    use super::*;

    const BASE: u32 = 0x4100_0000;
    const SCHEDULER: u32 = BASE;
    const NODE_A: u32 = BASE + 0x400;
    const NODE_B: u32 = BASE + 0x420;
    const INSTANCE_A: u32 = BASE + 0x800;
    const INSTANCE_B: u32 = BASE + 0x840;
    const DESCRIPTOR: u32 = BASE + 0xC00;

    fn guest() -> Guest {
        Guest::single(BASE, 0x2000)
    }

    struct TickHost {
        stamps: std::collections::VecDeque<u32>,
        calls: Vec<(u32, u32, f32)>,
        remove: Option<u32>,
    }

    impl SchedulerHost for TickHost {
        fn timebase(&mut self) -> u32 {
            self.stamps.pop_front().unwrap()
        }
        fn process(
            &mut self,
            g: &mut Guest,
            function: u32,
            context: u32,
            delta: f32,
        ) -> Result<()> {
            self.calls.push((function, context, delta));
            if let Some(node) = self.remove.take() {
                g.set_u32(SCHEDULER + SCHED_NODE_REMOVED, node)?;
            }
            Ok(())
        }
    }

    fn linked_pair(g: &mut Guest) {
        g.set_u32(SCHEDULER + BUCKET_HEAD_A, NODE_A).unwrap();
        // The tick follows +0 while the unlinker treats the same word as its previous link.
        g.set_u32(NODE_A + NODE_PREV, NODE_B).unwrap();
        g.set_u32(NODE_B + NODE_PREV, 0).unwrap();
        g.set_u32(NODE_A + NODE_INSTANCE, INSTANCE_A).unwrap();
        g.set_u32(NODE_B + NODE_INSTANCE, INSTANCE_B).unwrap();
        for instance in [INSTANCE_A, INSTANCE_B] {
            g.set_u32(instance, DESCRIPTOR).unwrap();
        }
    }

    #[test]
    fn tick_processes_preloaded_successor_and_records_profile_time() {
        let mut g = guest();
        linked_pair(&mut g);
        g.set_u8(DESCRIPTOR + 12, 1).unwrap();
        g.set_u32(INSTANCE_A + 4, 0x1111).unwrap();
        g.set_u32(INSTANCE_A + 8, 0xaaaa).unwrap();
        g.set_u32(INSTANCE_B + 4, 0x2222).unwrap();
        g.set_u32(INSTANCE_B + 8, 0xbbbb).unwrap();
        g.set_u32(SCHEDULER + 64, 0.125f32.to_bits()).unwrap();
        let mut host = TickHost {
            stamps: [100, 109, 200, 211].into(),
            calls: vec![],
            remove: None,
        };
        tick_bucket(&mut g, &mut host, SCHEDULER, 0).unwrap();
        assert_eq!(
            host.calls,
            vec![(0x1111, 0xaaaa, 0.125), (0x2222, 0xbbbb, 0.125)]
        );
        assert_eq!(g.u32(INSTANCE_A + INSTANCE_ELAPSED).unwrap(), 9);
        assert_eq!(g.u32(INSTANCE_B + INSTANCE_ELAPSED).unwrap(), 11);
        assert_eq!(g.u32(SCHEDULER + SCHED_CURRENT).unwrap(), 0);
    }

    #[test]
    fn tick_recycles_a_self_removed_node_without_losing_the_preloaded_successor() {
        let mut g = guest();
        linked_pair(&mut g);
        g.set_u8(DESCRIPTOR + 12, 0).unwrap();
        g.set_u32(INSTANCE_A + 4, 1).unwrap();
        g.set_u32(INSTANCE_B + 4, 2).unwrap();
        g.set_u32(SCHEDULER + SCHED_PENDING_BUCKET, 0).unwrap();
        g.set_u32(SCHEDULER + BUCKET_COUNT, 2).unwrap();
        let mut host = TickHost {
            stamps: [10, 20, 30].into(),
            calls: vec![],
            remove: Some(NODE_A),
        };
        tick_bucket(&mut g, &mut host, SCHEDULER, 0).unwrap();
        assert_eq!(host.calls.len(), 2);
        assert_eq!(g.u32(SCHEDULER + BUCKET_COUNT).unwrap(), 1);
        assert_eq!(g.u32(SCHEDULER + SCHED_NODE_REMOVED).unwrap(), 0);
    }
}

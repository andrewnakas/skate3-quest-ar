//! Original action-stream validation with supplied data, without game systems.
#[path = "../../skate-game/src/apt_display.rs"]
mod apt_display;
#[path = "../../skate-game/src/apt_movie.rs"]
mod apt_movie;
#[path = "../../skate-game/src/apt_scene.rs"]
mod apt_scene;
#[path = "../../skate-game/src/apt_text.rs"]
mod apt_text;
#[path = "../../skate-game/src/apt_vm.rs"]
mod apt_vm;
#[path = "../../skate-game/src/hud_runtime.rs"]
mod hud_runtime;
#[path = "../../skate-game/src/scoring_runtime.rs"]
mod scoring_runtime;
use skate_core::{
    animation::output::attributes::AttributeName, physics::filtered_state::FilteredCategory,
};
fn frame(
    tick: u32,
    category: FilteredCategory,
    descriptor: Option<AttributeName>,
) -> scoring_runtime::Frame {
    scoring_runtime::Frame {
        tick,
        dt: 1. / 60.,
        category,
        state: 100,
        descriptor,
        grind_id: -1,
        flags: 0,
        position: [0., 0., 0.],
        velocity: [0., 0., 5.],
        forward: [0., 0., 1.],
        rider_up: [0., 1., 0.],
        rider_forward: [0., 0., 1.],
        switch: false,
        fakie: false,
        nollie: false,
        body_flip: false,
        body_flip_side: false,
        hips_position: [0., 1., 0.],
        hips_ground: None,
        hips_surface: 0,
        suspend_air: false,
        landing: Default::default(),
        teleported: false,
        reverting: false,
        manual_block_70: false,
    }
}
fn publish_hud(
    scoring: &scoring_runtime::Runtime,
    hud: &mut hud_runtime::Runtime,
) -> Result<(), String> {
    hud.update(
        scoring.hud_input(),
        scoring.new_trick,
        scoring.modified_trick,
        scoring.close_tricks,
    )
}
fn main() -> Result<(), String> {
    let root = std::env::args_os()
        .nth(1)
        .ok_or("Expected owned assets directory")?;
    let data = skate_data::collections::Collections::load(std::path::Path::new(&root))?;
    let mut scoring = scoring_runtime::Runtime::load(&data)?;
    let movie_path = std::env::args_os()
        .nth(2)
        .ok_or("Expected owned trickdisplay.json")?;
    let json: serde_json::Value =
        serde_json::from_slice(&std::fs::read(movie_path).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    let mut hud = hud_runtime::Runtime::load(&json, scoring.hud_input())?;
    let kickflip = scoring
        .data
        .by_id(96)
        .ok_or("Missing kickflip")?
        .encoded_name;
    scoring.advance(frame(0, FilteredCategory::Ground, None))?;
    for tick in 1..61 {
        scoring.advance(frame(tick, FilteredCategory::Air, Some(kickflip)))?;
        publish_hud(&scoring, &mut hud)?;
    }
    for tick in 61..70 {
        scoring.advance(frame(tick, FilteredCategory::Ground, None))?;
        publish_hud(&scoring, &mut hud)?;
        if scoring.close_tricks {
            return Err("Landing emitted the native bail/close event".into());
        }
    }
    if scoring.hud_input().sequence_score != 100 || scoring.hud_input().line_time <= 0. {
        return Err("Banked landing lost its displayed score/line timer".into());
    }
    if scoring.hud_input().line_time != scoring.session.line.points / scoring.data.line_drain {
        return Err("HUD timer is not in native seconds".into());
    }
    if !hud.vm.get(hud.bindings.movie.root, "_visible").truth() {
        return Err("Original APT root disappeared on landing".into());
    }
    let banked = scoring.session.holder.snapshot.last_reward;
    if banked != 100. {
        return Err(format!(
            "Stationary unswitched kickflip, no landing bonus: expected authored 100, got {banked}"
        ));
    }
    for tick in 70..80 {
        scoring.advance(frame(tick, FilteredCategory::Ground, None))?;
        publish_hud(&scoring, &mut hud)?;
    }
    if scoring.session.holder.snapshot.last_reward != banked {
        return Err("Idle frame published the sequence again".into());
    }
    for tick in 80..140 {
        scoring.advance(frame(tick, FilteredCategory::Air, Some(kickflip)))?;
        publish_hud(&scoring, &mut hud)?;
    }
    let mut cancelled = frame(140, FilteredCategory::Ground, None);
    cancelled.teleported = true;
    scoring.advance(cancelled)?;
    if scoring.session.holder.snapshot.last_reward != 0.
        || scoring.session.holder.has_pending_sequence()
    {
        return Err("Teleport retained pending trick rewards".into());
    }
    if scoring.session.combo.multiplier != 1. {
        return Err("Teleport retained multiplier".into());
    }
    // An independent supplied-data sequence checks natural line expiry. The window has to
    // clear the ground collector's authored near-one hold: 82DA33E0 pins the line just
    // above one point for up to [collector 0x5fc] * 60 frames while the ground collector
    // is the active one, so expiry is that many frames later than the drain alone.
    let mut expired = scoring_runtime::Runtime::load(&data)?;
    expired.advance(frame(0, FilteredCategory::Ground, None))?;
    for tick in 1..61 {
        expired.advance(frame(tick, FilteredCategory::Air, Some(kickflip)))?;
    }
    let ground_hold = (expired.data.collector.scalar(0x5fc) * 60.) as u32;
    for tick in 61..300 + ground_hold {
        expired.advance(frame(tick, FilteredCategory::Ground, None))?;
    }
    if expired.hud_input().sequence_score != 0 || expired.hud_input().line_time != 0. {
        return Err("Expired line retained its HUD score".into());
    }
    // Native Air452 suspends continuous air metrics without removing the carrier.
    let mut suspended = scoring_runtime::Runtime::load(&data)?;
    suspended.advance(frame(0, FilteredCategory::Ground, None))?;
    for tick in 1..61 {
        suspended.advance(frame(tick, FilteredCategory::Air, Some(kickflip)))?;
    }
    let held_score = suspended.hud_input().sequence_score;
    let mut freeze = frame(61, FilteredCategory::Air, Some(kickflip));
    freeze.suspend_air = true;
    freeze.position = [100., 100., 100.];
    freeze.body_flip = true;
    suspended.advance(freeze)?;
    if suspended.hud_input().sequence_score != held_score {
        return Err("Air452 suspension accrued distance/height reward".into());
    }
    // 82DA8550 banks the five air metrics by bare id. They have no authored record, so a
    // lookup that demanded one dropped every one of them and an air scored its base trick and
    // nothing else. A moving, spinning air must bank far more than its authored 100.
    let mut spun = false;
    let mut moving = scoring_runtime::Runtime::load(&data)?;
    moving.advance(frame(0, FilteredCategory::Ground, None))?;
    let airborne = 60;
    for tick in 1..=airborne {
        let t = tick as f32 / airborne as f32;
        let turn = t * std::f32::consts::TAU;
        let mut f = frame(tick, FilteredCategory::Air, Some(kickflip));
        // 10 m along Z, a 3 m arc of height, and one full rotation.
        f.position = [0., 3. * (t * std::f32::consts::PI).sin(), 10. * t];
        // The rider turns, not the deck: retail's spin accumulator measures the skeleton
        // root, so rotating the board alone must not register as a spin.
        f.rider_forward = [turn.sin(), 0., turn.cos()];
        moving.advance(f)?;
        // 825E51A0 appends the spin as a bare degree count, but only for a TrickType its
        // r23 admits. The type comes from the *named* record, so the announce has to set it:
        // when it did not, every ordinary trick carried type 0, which `decorates_spin`
        // refuses, and no spin ever reached a name.
        if moving.displayed_name().split(' ').any(|t| t.parse::<u32>().is_ok()) {
            spun = true;
        }
    }
    for tick in airborne + 1..airborne + 10 {
        moving.advance(frame(tick, FilteredCategory::Ground, None))?;
    }
    let banked = moving.session.holder.snapshot.last_reward;
    if banked <= 100. {
        return Err(format!(
            "A 10 m, 3 m-high, 360-degree air banked {banked}, which is no more than the \
             authored kickflip alone: the air metric rewards are being discarded again"
        ));
    }
    if !spun {
        return Err(
            "A full rotation never put a degree count in the trick name: the \r
             announced record's TrickType is not reaching `decorates_spin`"
                .into(),
        );
    }
    println!("Moving 360 air over 10 m and 3 m of height banked {banked:.0} (authored base 100), named with its spin");

    // 82DA93D8's one-shot class-3 bonus, worth 0x600 = 4.0 times the trick's own points.
    // 82DAC780 admits it only while the hips' ground contact lies between the deck and the
    // hips, with neither vector degenerate.
    let flipped = |contact: Option<[f32; 3]>| -> Result<f32, String> {
        let mut run = scoring_runtime::Runtime::load(&data)?;
        run.advance(frame(0, FilteredCategory::Ground, None))?;
        for tick in 1..61 {
            let mut f = frame(tick, FilteredCategory::Air, Some(kickflip));
            f.position = [0., 1., 0.];
            f.hips_position = [0., 2., 0.];
            f.hips_ground = contact;
            run.advance(f)?;
        }
        for tick in 61..70 {
            run.advance(frame(tick, FilteredCategory::Ground, None))?;
        }
        Ok(run.session.holder.snapshot.last_reward)
    };
    // Contact between deck (y=1) and hips (y=2): the two vectors oppose, so it pays.
    let with_bonus = flipped(Some([0., 1.5, 0.]))?;
    // Contact below both: the vectors agree, so it must not pay.
    let without = flipped(Some([0., 0.5, 0.]))?;
    let no_contact = flipped(None)?;
    if without != 100. || no_contact != 100. {
        return Err(format!(
            "The class-3 bonus paid without its geometric gate: {without} / {no_contact}"
        ));
    }
    if with_bonus != 500. {
        return Err(format!(
            "Expected the authored 100 plus four times it, got {with_bonus}"
        ));
    }
    println!("Class-3 one-shot bonus: kickflip banks {with_bonus:.0} through the gate, {without:.0} outside it");

    // The gap/context collector: four runs of horizontal distance, two of them gated on
    // being more than 3 m and 6 m above the ground under the hips. 20 m spent above 6 m
    // tops out both of those curves, at 900 and 1800.
    let gap = |clearance: f32| -> Result<f32, String> {
        let mut run = scoring_runtime::Runtime::load(&data)?;
        // The deck is held still and above the contact: that zeroes the air distance and
        // height metrics and keeps 82DAC780's flip bonus out of the measurement, so what
        // is left is the gap runs alone.
        let mut entry = frame(0, FilteredCategory::Ground, None);
        entry.position = [0., 10., 0.];
        entry.hips_position = [0., 10., 0.];
        run.advance(entry)?;
        for tick in 1..=60 {
            // The run starts on its first active frame, so travel must begin at zero for
            // the distance to come out at exactly 20 m.
            let travelled = 20. * (tick - 1) as f32 / 59.;
            let mut f = frame(tick, FilteredCategory::Air, Some(kickflip));
            f.position = [0., 10., 0.];
            f.hips_position = [0., 10., travelled];
            f.hips_ground = Some([0., 10. - clearance, travelled]);
            run.advance(f)?;
        }
        for tick in 61..70 {
            run.advance(frame(tick, FilteredCategory::Ground, None))?;
        }
        Ok(run.session.holder.snapshot.last_reward)
    };
    let over_a_gap = gap(7.)?;
    let low = gap(1.)?;
    if low != 100. {
        return Err(format!("A 1 m clearance must clear no gap, got {low}"));
    }
    if (over_a_gap - 2800.).abs() > 0.5 {
        return Err(format!(
            "20 m spent 7 m up should bank 100 + 900 + 1800, got {over_a_gap}"
        ));
    }
    println!("Gap collector: 20 m cleared 7 m up banks {over_a_gap:.0}, the same air 1 m up banks {low:.0}");
    // A rail -> manual -> flip line must keep its multiplier the whole way through.
    // 0x0800_0000 is NoseManual, the ground collector's slot 1 (82DAA8E0); grind5050 is
    // scorable 39. Nothing here bails, so nothing may reset the line.
    let manual_flag = 0x0800_0000u32;
    let linked = |reverting: bool| -> Result<(f32, f32, bool), String> {
        let mut run = scoring_runtime::Runtime::load(&data)?;
        run.advance(frame(0, FilteredCategory::Ground, None))?;
        let mut closed = false;
        let mut lowest = f32::MAX;
        let mut travelled = 0.;
        let mut step = |run: &mut scoring_runtime::Runtime,
                        mut f: scoring_runtime::Frame,
                        closed: &mut bool,
                        lowest: &mut f32|
         -> Result<(), String> {
            travelled += 0.1;
            f.position = [0., 0., travelled];
            run.advance(f)?;
            *closed |= run.close_tricks;
            // Only judge the multiplier once the line has actually earned one.
            if run.session.combo.multiplier > 1. {
                *lowest = lowest.min(run.session.combo.multiplier);
            }
            Ok(())
        };
        // Two banked flips first: the line is only credited once the multiplier is above
        // x1.01, so a single trick never opens a line to lose.
        for pass in 0..2u32 {
            let base = pass * 80;
            for tick in base + 1..base + 61 {
                step(
                    &mut run,
                    frame(tick, FilteredCategory::Air, Some(kickflip)),
                    &mut closed,
                    &mut lowest,
                )?;
            }
            for tick in base + 61..base + 81 {
                step(
                    &mut run,
                    frame(tick, FilteredCategory::Ground, None),
                    &mut closed,
                    &mut lowest,
                )?;
            }
        }
        let opened = run.session.line.points;
        // The rail.
        for tick in 161..221 {
            let mut f = frame(tick, FilteredCategory::Grind, None);
            f.grind_id = 39;
            step(&mut run, f, &mut closed, &mut lowest)?;
        }
        // The manual it links into, optionally across a revert.
        for tick in 221..341 {
            let mut f = frame(tick, FilteredCategory::Ground, None);
            f.flags = manual_flag;
            f.reverting = reverting;
            step(&mut run, f, &mut closed, &mut lowest)?;
        }
        // And the flip trick out of it.
        for tick in 341..401 {
            step(
                &mut run,
                frame(tick, FilteredCategory::Air, Some(kickflip)),
                &mut closed,
                &mut lowest,
            )?;
        }
        for tick in 401..421 {
            step(
                &mut run,
                frame(tick, FilteredCategory::Ground, None),
                &mut closed,
                &mut lowest,
            )?;
        }
        if opened <= 0. {
            return Err("The two banked flips never opened a line to test".into());
        }
        Ok((lowest, run.session.combo.multiplier, closed))
    };
    let (lowest, final_multiplier, closed) = linked(false)?;
    if closed {
        return Err("Linking rail -> manual -> flip emitted the native bail/close event".into());
    }
    if lowest <= 1. || final_multiplier <= 1. {
        return Err(format!(
            "The multiplier fell back to x{lowest} across rail -> manual -> flip and ended at \
             x{final_multiplier}: the line was dropped mid-combo"
        ));
    }
    // The same line across a revert. 82DAA8E0 gates the manual slot on State+70, not on
    // State+66, so a revert must not change what the manual scores.
    let (revert_lowest, revert_final, revert_closed) = linked(true)?;
    if revert_closed || revert_lowest != lowest || revert_final != final_multiplier {
        return Err(format!(
            "Reverting changed the linked line: x{revert_lowest}/x{revert_final} against \
             x{lowest}/x{final_multiplier}. The manual carrier is being dropped by the \
             revert flag again"
        ));
    }
    println!(
        "Rail -> manual -> flip holds the multiplier at x{final_multiplier} (never below \
         x{lowest}), with and without a revert"
    );

    // The exact frame pattern from a captured session, where a combo came apart: the manual
    // bit drops a few frames before the pop, the flip trick is announced with 0x01000000
    // while still grounded, and only then does the air begin. Publishing anywhere in that
    // window is what reset the on-screen combo. 82DAB2A8 answers "still going" for the
    // announced trick, and 8281DD70 answers "always" for a grind, so the whole
    // rail -> manual -> pop -> air run must be one single sequence.
    let mut linked_once = scoring_runtime::Runtime::load(&data)?;
    linked_once.advance(frame(0, FilteredCategory::Ground, None))?;
    for tick in 1..61 {
        linked_once.advance(frame(tick, FilteredCategory::Air, Some(kickflip)))?;
    }
    for tick in 61..81 {
        linked_once.advance(frame(tick, FilteredCategory::Ground, None))?;
    }
    // From here the sequence must stay open until the air is banked.
    let mut breaks = Vec::new();
    let mut step = |run: &mut scoring_runtime::Runtime,
                    f: scoring_runtime::Frame,
                    label: &str,
                    breaks: &mut Vec<String>|
     -> Result<(), String> {
        let tick = f.tick;
        run.advance(f)?;
        if !run.sequence_open() {
            breaks.push(format!("{label} at tick {tick}"));
        }
        Ok(())
    };
    for tick in 81..141 {
        let mut f = frame(tick, FilteredCategory::Grind, None);
        f.grind_id = 39;
        step(&mut linked_once, f, "the grind", &mut breaks)?;
    }
    for tick in 141..261 {
        let mut f = frame(tick, FilteredCategory::Ground, None);
        f.flags = 0x0400_0000;
        step(&mut linked_once, f, "the manual", &mut breaks)?;
    }
    // The pop: no manual bit any more, but the trick is announced.
    for tick in 261..267 {
        let mut f = frame(tick, FilteredCategory::Ground, None);
        f.flags = 0x0100_0000;
        step(&mut linked_once, f, "the pop", &mut breaks)?;
    }
    for tick in 267..327 {
        let mut f = frame(tick, FilteredCategory::Air, Some(kickflip));
        f.flags = 0x0100_0000;
        step(&mut linked_once, f, "the air out", &mut breaks)?;
    }
    if !breaks.is_empty() {
        return Err(format!(
            "The sequence was cut {} times across rail -> manual -> pop -> air, first at {}: \
             the collectors are not being asked whether it continues",
            breaks.len(),
            breaks[0]
        ));
    }
    // And it must still end once the trick is actually over.
    for tick in 327..347 {
        linked_once.advance(frame(tick, FilteredCategory::Ground, None))?;
    }
    if linked_once.sequence_open() {
        return Err("The sequence never ended after the air was landed and let go".into());
    }
    println!(
        "Rail -> manual -> pop -> air stays one sequence, and still ends when the trick does"
    );

    // A score left alone must fade. Once a sequence has been banked, 82DA37B0 stops calling
    // 82DA48B8, bit30 of [holder+1832] goes clear, and the line timer gets no hold at all:
    // it drains out and the authored Clear/outro runs. Nothing may keep re-arming the hold
    // after the last trick.
    let mut faded = scoring_runtime::Runtime::load(&data)?;
    faded.advance(frame(0, FilteredCategory::Ground, None))?;
    for pass in 0..2u32 {
        let base = pass * 80;
        for tick in base + 1..base + 61 {
            faded.advance(frame(tick, FilteredCategory::Air, Some(kickflip)))?;
        }
        for tick in base + 61..base + 81 {
            faded.advance(frame(tick, FilteredCategory::Ground, None))?;
        }
    }
    let opened = faded.session.line.points;
    if opened <= 0. {
        return Err("The fade case never opened a line".into());
    }
    let mut faded_at = None;
    for tick in 161..161 + 1200 {
        faded.advance(frame(tick, FilteredCategory::Ground, None))?;
        if faded_at.is_none()
            && faded.hud_input().line_time == 0.
            && faded.hud_input().sequence_score == 0
            && faded.hud_input().line_score == 0
        {
            faded_at = Some(tick - 161);
        }
    }
    let Some(faded_at) = faded_at else {
        return Err(format!(
            "A line of {opened:.0} points never faded after the last trick: something is \
             still holding the line timer open"
        ));
    };
    // The authored ground hold is 300 frames on its own. Fading has to be quicker than
    // re-arming that even once, or the score is being held when nothing is happening.
    let hold = (faded.data.collector.scalar(0x5fc) * 60.) as u32;
    if faded_at >= hold {
        return Err(format!(
            "The score took {faded_at} frames to fade, which is at least one whole {hold}-frame \
             ground hold: the hold is being applied after the sequence ended"
        ));
    }
    println!(
        "A {opened:.0}-point line left alone fades in {faded_at} frames, well inside the \
         {hold}-frame ground hold"
    );

    // The other end of the same mechanism: the ground collector's near-one hold is bounded
    // by [collector 0x5fc] * 60 frames, so a manual held forever must still let the line
    // expire and the display clear.
    let mut parked = scoring_runtime::Runtime::load(&data)?;
    parked.advance(frame(0, FilteredCategory::Ground, None))?;
    for pass in 0..2u32 {
        let base = pass * 80;
        for tick in base + 1..base + 61 {
            parked.advance(frame(tick, FilteredCategory::Air, Some(kickflip)))?;
        }
        for tick in base + 61..base + 81 {
            parked.advance(frame(tick, FilteredCategory::Ground, None))?;
        }
    }
    if parked.session.line.points <= 0. {
        return Err("The parked-manual case never opened a line".into());
    }
    let limit = (parked.data.collector.scalar(0x5fc) * 60.) as u32;
    // The line has to drain its remaining points at the ground collector's authored 0x610
    // rate first, and only then does the near-one hold get its bounded turn.
    let drain = parked.data.line_drain / 60. * parked.data.collector.scalar(0x610);
    // The escape takes more than one hold cycle: the frame that ends a hold only steps the
    // points down by one drain, and the hold re-arms until `previous` finally lands under
    // 82DA4C28's 1.000001. Allow several cycles rather than assuming exactly one.
    let window = (parked.session.line.points / drain) as u32 + 4 * limit + 240;
    let mut expired_at = None;
    for tick in 161..161 + window {
        let mut f = frame(tick, FilteredCategory::Ground, None);
        f.flags = manual_flag;
        parked.advance(f)?;
        if expired_at.is_none() && parked.hud_input().line_time == 0. {
            expired_at = Some(tick - 161);
        }
    }
    let Some(expired_at) = expired_at else {
        return Err(format!(
            "A manual held for {} frames never let the line expire: the ground collector's \
             near-one hold is unbounded again, so the score and trick name never clear",
            window
        ));
    };
    // Expiry settles the line, not the trick in progress: the manual is still being ridden,
    // so its own running total stays on screen. What must go is the line and its multiplier.
    if parked.session.combo.multiplier != 1. || parked.hud_input().line_score != 0 {
        return Err(format!(
            "The expired line kept x{} and a line score of {}",
            parked.session.combo.multiplier,
            parked.hud_input().line_score
        ));
    }
    println!(
        "A parked manual lets the line expire after {expired_at} frames: the drain, then \
         the authored {limit}-frame ground holds"
    );

    println!(
        "Scoring data audit: authored kickflip credited once; landing display persists; timer uses seconds; line expiry clears score; teleport cancels pending rewards and multiplier; Air452 freezes continuous metrics; air metrics reach the bank; rail-manual-flip keeps its multiplier; a parked manual still expires"
    );
    Ok(())
}

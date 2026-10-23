// Tuning table for the clean-room engine (values hand-picked for this engine).
#include "params.hpp"
#include <cstring>

namespace sk8 {

namespace {

const char *const kPop = "Pop & Jump";
const char *const kSpeed = "Gravity & Speed";
const char *const kSpin = "Spin";
const char *const kFlips = "Flips";
const char *const kBails = "Bails";
const char *const kGrinds = "Grinds";
const char *const kManuals = "Manuals";
const char *const kScore = "Scoring";

// key, label, group, min, max, default, step
const sk8_param kTable[ParamCount] = {
	{"pop_low", "Pop (quick flick), m/s", kPop, 1.0f, 12.0f, 4.55f, 0.05f},
	{"pop_high", "Pop (full crouch), m/s", kPop, 1.0f, 16.0f, 5.35f, 0.05f},
	{"crouch_charge", "Crouch to full pop, s", kPop, 0.05f, 1.5f, 0.21f, 0.01f},
	{"nollie_pop", "Nollie pop share", kPop, 0.3f, 1.5f, 1.015f, 0.005f},
	{"grind_pop", "Pop off rails share", kPop, 0.3f, 1.5f, 0.7f, 0.01f},
	{"boneless_pop", "Boneless / fastplant pop share", kPop, 0.3f, 1.5f, 0.835f, 0.01f},

	{"gravity", "Gravity, m/s^2", kSpeed, 1.0f, 40.0f, 12.55f, 0.05f},
	{"slope_gravity", "Share of gravity rolling on a slope", kSpeed, 0.1f, 1.5f, 0.78f, 0.01f},
	{"push_kick", "Push strength, share of the way to top speed", kSpeed, 0.0f, 1.0f, 0.64f, 0.01f},
	{"push_period", "Time between pushes, s", kSpeed, 0.15f, 2.0f, 0.98f, 0.01f},
	{"push_top", "Top pushing speed, m/s", kSpeed, 1.0f, 30.0f, 9.15f, 0.05f},
	{"push_cap", "Most one stroke gives, m/s", kSpeed, 0.5f, 10.0f, 3.6f, 0.05f},
	{"push_lead", "Step off before the first push, s", kSpeed, 0.0f, 1.0f, 0.4f, 0.01f},
	{"push_mongo", "Mongo push (0 off, 1 on)", kSpeed, 0.0f, 1.0f, 0.0f, 1.0f},
	{"pop_wait", "Flick to takeoff, s", kSpeed, 0.0f, 0.4f, 0.122f, 0.005f},
	{"roll_drag", "Rolling drag, m/s^2", kSpeed, 0.0f, 3.0f, 0.001f, 0.001f},
	{"air_drag", "Air drag", kSpeed, 0.0f, 0.05f, 0.0001f, 0.0001f},
	{"brake", "Brake, m/s^2", kSpeed, 0.0f, 20.0f, 4.1f, 0.05f},
	{"brake_lead", "Foot down before braking, s", kSpeed, 0.0f, 1.0f, 0.2f, 0.01f},
	{"brake_out", "Foot back up after braking, s", kSpeed, 0.0f, 1.0f, 0.42f, 0.01f},
	{"carve", "Turning, rad/s", kSpeed, 0.2f, 8.0f, 1.7f, 0.01f},
	{"carve_ease", "Turn coming on, s (each of two stages)", kSpeed, 0.0f, 0.5f, 0.11f, 0.01f},
	{"grip", "Wheel grip", kSpeed, 0.5f, 40.0f, 8.4f, 0.1f},
	{"slide_drag", "Powerslide drag, m/s^2", kSpeed, 0.0f, 25.0f, 4.2f, 0.05f},
	{"slide_soft", "Powerslide eases off below, m/s", kSpeed, 0.5f, 20.0f, 6.0f, 0.1f},
	{"ground_snap", "Stick to ground, m", kSpeed, 0.02f, 1.0f, 0.27f, 0.01f},
	{"switch_wait", "Rolling fakie before going switch, s (0 never)", kSpeed, 0.0f, 5.0f, 0.6f, 0.05f},
	{"switch_time", "Shuffling round to switch, s", kSpeed, 0.1f, 2.0f, 0.7f, 0.05f},

	{"spin_rate", "Air spin, rad/s", kSpin, 0.0f, 25.0f, 7.95f, 0.05f},
	{"spin_ease", "Spin response (1/s to come up to speed)", kSpin, 0.5f, 60.0f, 28.0f, 0.5f},
	{"air_level", "Auto-level in the air, rad/s", kSpin, 0.0f, 10.0f, 1.14f, 0.01f},

	{"flip_base", "Flip spin-up, s", kFlips, 0.02f, 1.0f, 0.12f, 0.005f},
	{"flip_per_rev", "Time per turn, s", kFlips, 0.1f, 2.0f, 0.41f, 0.005f},
	{"late_flips", "Flick in the air (0 off, 1 on)", kFlips, 0.0f, 1.0f, 1.0f, 1.0f},
	{"flip_lenience", "Flip done enough to land", kFlips, 0.3f, 1.0f, 0.83f, 0.01f},
	{"held_reach", "Held flick: how hopefully it judges the air for one more turn", kFlips, 0.5f, 3.0f, 1.26f, 0.01f},

	{"land_clean", "Clean landing, degrees", kBails, 1.0f, 89.0f, 22.0f, 0.5f},
	{"land_max", "Worst landing, degrees", kBails, 5.0f, 170.0f, 58.0f, 0.5f},
	{"side_max", "Worst sideways landing, degrees", kBails, 5.0f, 90.0f, 88.0f, 0.5f},
	{"land_skid", "Landing skid: no speed kept at, degrees", kBails, 30.0f, 180.0f, 100.0f, 1.0f},
	{"wall_bail", "Wall slam speed, m/s", kBails, 0.5f, 60.0f, 11.6f, 0.05f},
	{"impact_bail", "Hard landing speed, m/s", kBails, 1.0f, 80.0f, 13.6f, 0.1f},
	{"bail_time", "Bail to back on the board, s", kBails, 0.5f, 12.0f, 4.9f, 0.05f},
	{"bail_slide", "Body sliding along the ground, m/s^2", kBails, 0.5f, 30.0f, 5.35f, 0.05f},
	{"get_up", "Getting up at the end of a bail, s", kBails, 0.2f, 4.0f, 1.6f, 0.05f},

	{"grind_catch", "Rail catch distance, m", kGrinds, 0.05f, 2.0f, 0.42f, 0.01f},
	{"grind_drag", "Rail drag, m/s^2", kGrinds, 0.0f, 10.0f, 0.83f, 0.01f},
	{"grind_balance", "Rail balance (0 off)", kGrinds, 0.0f, 3.0f, 0.0f, 0.05f},
	{"grind_keep", "Speed kept catching a rail", kGrinds, 0.5f, 1.5f, 1.027f, 0.005f},
	{"grind_off_keep", "Speed kept rolling off a rail's end", kGrinds, 0.5f, 1.0f, 0.89f, 0.01f},
	{"plant_stall", "Longest handplant held, s", kGrinds, 0.2f, 5.0f, 1.28f, 0.01f},
	{"grind_assist", "Pull onto a rail in the air, m/s^2", kGrinds, 0.0f, 30.0f, 11.15f, 0.1f},

	{"manual_wobble", "Manual wobble", kManuals, 0.0f, 5.0f, 0.38f, 0.02f},
	{"manual_correct", "Manual correction", kManuals, 0.0f, 10.0f, 2.45f, 0.05f},
	{"manual_hold", "Right stick part-way, s to manual", kManuals, 0.0f, 1.0f, 0.18f, 0.01f},
	{"manual_drag", "Manual slow-down, m/s^2", kManuals, 0.0f, 5.0f, 0.37f, 0.01f},
	{"manual_land", "After a landing, s before a fresh manual (held into it: at once)", kManuals, 0.0f, 2.0f, 0.75f, 0.05f},

	{"score_scale", "Score multiplier", kScore, 0.0f, 100.0f, 1.0f, 0.1f},
	{"bank_wait", "Roll-away to bank, s", kScore, 0.0f, 5.0f, 1.2f, 0.05f},
	{"grind_rate", "Grind points per second", kScore, 0.0f, 1000.0f, 65.0f, 5.0f},
	{"manual_rate", "Manual points per second", kScore, 0.0f, 1000.0f, 45.0f, 5.0f},
};

} // namespace

const sk8_param *param_table() { return kTable; }

Params::Params()
{
	for (int i = 0; i < ParamCount; i++)
		v[i] = kTable[i].def;
}

} // namespace sk8

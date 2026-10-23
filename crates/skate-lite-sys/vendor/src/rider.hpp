// The rider: one body for board and skater, on host collision (written from scratch).
#pragma once
#include "flick.hpp"
#include "params.hpp"
#include "world.hpp"
#include <deque>
#include <string>
#include <vector>

namespace sk8 {

class Rider
{
public:
	Rider();
	void place(V3 position, float facing, float speed);
	void step(World &w, const sk8_input &in);
	void bail(const char *reason);
	void state(sk8_state &s) const;
	const char *trick() const { return trick_.c_str(); }

	Params p;
	unsigned flags = 0;
	std::deque<sk8_event> events;

private:
	struct Pending
	{
		std::string name;
		unsigned points;
	};

	void ground(World &w, const sk8_input &in, const Flick::Result &f);
	void air(World &w, const sk8_input &in, const Flick::Result &f);
	void grind(World &w, const sk8_input &in, const Flick::Result &f);
	void bailed(World &w);
	bool try_wallride(const Hit &h);
	void wallride(World &w, const sk8_input &in, const Flick::Result &f);
	void leave_wall(bool popped);
	void walk(World &w, const sk8_input &in);

	void pop(const TrickDef &def, float share);
	static int plant_bits(const sk8_input &in);
	std::string plant_variant(int bits) const;
	void start_flip(int trick);
	void land(const Hit &h);
	bool try_grind(World &w);
	void leave_grind(V3 velocity, bool popped, bool off_end = false);
	void move_on_ground(World &w, V3 delta);
	void follow_ground(World &w);

	void add_trick(const std::string &name, unsigned points);
	void commit_pending();
	void bank();
	void lose_combo();
	void emit(sk8_event_kind kind, const char *name = "", unsigned points = 0, int value = 0, int surface = 0);
	bool never_bail() const { return flags & SK8_FLAG_NEVER_BAIL; }
	float noise();

	// body
	V3 pos_, vel_;
	V3 fwd_{0, 0, 1}, up_ = kUp;
	sk8_mode mode_ = SK8_MODE_RIDE;
	bool fakie_ = false;
	// switch: the other foot forward (fwd_ turned round with it); rolling fakie a moment turns into
	// it, the feet shuffling round over switch_len_
	bool switch_ = false;
	float fakie_time_ = 0, switching_ = -1, switch_len_ = 1;
	bool push_queued_ = false;
	std::string stance_pre_; // "Fakie " / "Switch " in front of a pop's tricks
	void start_switch(float duration);
	std::string staged(const char *name) const { return stance_pre_ + name; }
	int surface_ = 0;

	// controls
	Flick flick_;
	float crouch_ = 0, crouch_time_ = 0, lean_ = 0;
	float push_cooldown_ = 0, push_time_ = -1, push_lead_ = 0, since_stroke_ = 9;
	bool push_mongo_ = false;
	bool foot_off_ = false;
	float last_steer_[2] = {0, 0};
	bool unslide_ = false;
	V3 plant_in_; // the way it was going when it planted (a foot/handplant pops back off that way)
	unsigned plant_hold_ = 0; // the button that keeps a plant on the edge (let go, it comes off)
	int plant_want_ = 0, plant_pose_ = 0; // a handplant's variant (sk8_plant_pose) asked for (and how long), and shown
	float plant_want_time_ = 0, plant_time_ = 0;
	// a boneless on the ground (grab trigger + push): the wind-up before the pop, and whether the
	// left stick went up on the way (a fastplant)
	float plant_wind_ = -1;
	bool plant_air_ = false; // off a plant (the slide button still held coming down isn't a revert)
	V3 vert_in_;             // off a steep wall going up: level, from the wall into the ramp
	bool wall_off_ = false;  // in the air off the top of a steep wall (until the next landing)
	bool plant_fast_ = false;
	std::string plant_side_; // "Frontside " / "Backside " (a handplant, a boneless)
	unsigned last_buttons_ = 0, prev_buttons_ = 0;
	// revert: landed holding the slide button, the board spins a half turn on its wheels
	float revert_time_ = -1;
	float revert_dir_ = 1;
	// wallride: the wall's outward normal (level), time on it
	V3 wall_n_;
	float wall_time_ = 0, since_wall_ = 9; // off a slide: the board turns back to the line of travel in the air
	int grab_ = 0, grab_style_ = 0;
	float grab_time_ = 0;
	bool grab_scored_ = false;
	bool grab_tweakable_ = false; // started with the stick centred: the stick can still tweak it

	// air
	int flip_ = -1;
	float flip_time_ = 0, flip_duration_ = 1;
	// the board's turning: turns done, its rate (turns/s) and the turns it's going to (it spins up,
	// cruises at 1 / flip_per_rev and brakes to stop on the last)
	float flip_angle_ = 0, flip_rate_ = 0, flip_turns_ = 1, flip_base_turns_ = 1;
	float flip_frac() const { return flip_ >= 0 ? clampf(flip_angle_ / std::max(flip_turns_, 1e-3f), 0, 1) : 1; }
	// a gesture done on the ground: the takeoff it waits for (the board loads up first)
	int takeoff_trick_ = -1;
	float takeoff_time_ = 0, takeoff_share_ = 1, takeoff_crouch_ = 0;
	// held flicks: the stick kept at the flick's last direction adds turns (double .. quad)
	float flip_mult_ = 1, flip_dir_[2] = {0, 0}, last_flick_[2] = {0, 0};
	int flip_extra_ = 0;
	float pop_y_ = 0; // height the last pop left from
	float varial_ = 0;
	bool foot_air_ = false; // jumped on foot: lands back on its feet, the board in hand // body varial: how far the body has turned on its own, the board staying
	std::string flip_name_;
	void extend_flip();
	// round far enough to land on: at most (1 - lenience) of a turn left, however many turns
	bool flip_landable() const { return flip_ < 0 || flip_turns_ - flip_angle_ <= (1 - p[FlipLenience]) * flip_base_turns_ + 1e-4f; }
	float spin_speed_ = 0, spin_total_ = 0;
	float air_time_ = 0;

	// manual / grind
	int manual_dir_ = 0;
	int manual_style_ = 0; // 0 a manual, 1 primo (board on its side), 2 casper (board upside down)
	// named for the stance it's ridden in, like the flips: Fakie Manual, Switch Nose Manual
	std::string manual_name() const
	{
		const char *base = manual_style_ == 1 ? "Primo" : manual_style_ == 2 ? "Casper" : manual_dir_ > 0 ? "Manual" : "Nose Manual";
		return std::string(switch_ ? "Switch " : fakie_ ? "Fakie " : "") + base;
	}
	float manual_time_ = 0;
	float balance_ = 0;
	int rail_ = -1, seg_ = 0;
	float seg_u_ = 0, rail_speed_ = 0;
	int rail_dir_ = 1;
	int grind_kind_ = 0;
	float carve_in_ = 0, carve_ = 0; // the steer, eased in two stages
	bool vert_air_ = false; // off the top of a steep wall: comes back down into it
	int settle_trick_ = -1; // a flip landed short: the board finishing its turn
	float settle_e_ = 1, settle_mult_ = 1;
	float brake_held_ = 0, brake_up_ = 0, push_gain_ = 0, skid_ = 0, skid_time_ = 0, land_settle_ = 0.8f;
	bool grind_tip_ = false;
	bool grind_slide_ = false, grind_backwards_ = false;
	float grind_transfer_ = 0; // a 50-50 near the rail's end, the front truck past it: tipped onto the tail
	float grind_time_ = 0;
	float grind_cooldown_ = 0;
	std::string grind_name_;

	// bail
	float bail_time_ = -1, bail_stage_time_ = 0;
	int bail_stage_ = 0, bail_dir_ = 1;
	std::string bail_reason_;
	V3 safe_pos_;
	float safe_facing_ = 0;
	float safe_timer_ = 0;

	// scoring
	std::vector<Pending> pending_;
	std::vector<std::string> combo_names_;
	unsigned combo_points_ = 0;
	float bank_timer_ = 0;
	unsigned long long score_ = 0;
	std::string trick_;
	unsigned seed_ = 0x2468ace1u;
	unsigned step_count_ = 0;
	float slide_amount_ = 0;
	bool manual_lock_ = false;
	// the right stick tilted part-way (1 down = manual, -1 up = nose manual), for how long, how deep 0..1
	int stick_manual_ = 0;
	float stick_manual_time_ = 0, stick_manual_depth_ = 0, since_land_ = 9;
};

} // namespace sk8

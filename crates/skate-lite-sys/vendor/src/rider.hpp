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

	void pop(const TrickDef &def, float share);
	void start_flip(int trick);
	void land(const Hit &h);
	bool try_grind(World &w);
	void leave_grind(V3 velocity, bool popped);
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
	int surface_ = 0;

	// controls
	Flick flick_;
	float crouch_ = 0, crouch_time_ = 0, lean_ = 0;
	float push_cooldown_ = 0, push_time_ = -1, push_lead_ = 0, since_stroke_ = 9;
	bool push_mongo_ = false;
	bool foot_off_ = false;
	int grab_ = 0, grab_style_ = 0;
	float grab_time_ = 0;
	bool grab_scored_ = false;

	// air
	int flip_ = -1;
	float flip_time_ = 0, flip_duration_ = 1;
	float spin_speed_ = 0, spin_total_ = 0;
	float air_time_ = 0;

	// manual / grind
	int manual_dir_ = 0;
	float manual_time_ = 0;
	float balance_ = 0;
	int rail_ = -1, seg_ = 0;
	float seg_u_ = 0, rail_speed_ = 0;
	int rail_dir_ = 1;
	int grind_kind_ = 0;
	float grind_time_ = 0;
	float grind_cooldown_ = 0;
	std::string grind_name_;

	// bail
	float bail_time_ = -1;
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

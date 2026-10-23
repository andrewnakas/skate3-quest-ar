#include <algorithm>
// The rider state machine (written from scratch; see rider.hpp).
#include "rider.hpp"
#include <cmath>
#include <cstdio>
#include <cstring>

namespace sk8 {

namespace
{
struct GrabDef
{
	const char *name;
	unsigned points;
};

// Indexed by sk8_grab_style.
const GrabDef kGrabs[] = {
	{"", 0},
	{"Indy", 150},      {"Crail", 220},     {"Tail Grab", 180}, {"Stalefish", 240},
	{"Melon", 150},     {"Nose Grab", 180}, {"Tuck Knee", 200}, {"Judo", 260},
	{"Double Grab", 200}, {"Superman", 320}, {"No Foot", 300},  {"Stiffy", 260},
	{"Christ Air", 340},  {"Method", 240},   {"Rocket Air", 300}, {"Seatbelt", 220},
	{"Mute", 180},       {"Japan", 260},      {"Nosebone", 240},  {"Tailbone", 240},
	{"Cross Bone", 250},
};

const GrabDef &grab_def(int style)
{
	return kGrabs[(style > 0 && style < int(sizeof kGrabs / sizeof kGrabs[0])) ? style : 0];
}

struct GrindDef
{
	const char *name;
	unsigned points;  // on landing it
	float along;      // where on the board the rail is (m from the centre, + toward the nose)
	float pitch, yaw, roll; // the board on the rail (degrees: nose up, nose to the left, heel edge up)
};

// Indexed by sk8_grind_kind.
const GrindDef kGrindDefs[] = {
	{"", 0, 0, 0, 0, 0},
	{"50-50", 40, 0, 0, 0, 0},
	{"Boardslide", 50, 0, 0, 0, 0},
	{"", 0, 0, 0, 0, 0},
	{"5-0", 60, -0.2f, 9, 0, 0},
	{"Nosegrind", 70, 0.2f, -9, 0, 0},
	{"Crooked Grind", 90, 0.2f, -10, 17, 6},
	{"Overcrook", 100, 0.2f, -10, -17, -6},
	{"Smith Grind", 100, -0.2f, -14, 8, 18},
	{"Feeble Grind", 90, -0.2f, -6, -16, -12},
	{"Noseslide", 70, 0.33f, -6, 0, 0},
	{"Tailslide", 70, -0.33f, 6, 0, 0},
	{"Lipslide", 80, 0, 0, 0, 0},
	{"Noseblunt", 120, 0.38f, -28, 0, 0},
	{"Bluntslide", 110, -0.38f, 28, 0, 0},
	{"Footplant", 130, 0, 0, 0, 0},
	{"Handplant", 240, 0, 0, 0, 0},
	{"Darkslide", 300, 0, 0, 0, 180},
	{"Salad Grind", 110, -0.2f, -6, -14, 8},
	{"Willy Grind", 110, -0.2f, -12, 10, -14},
	{"Suski Grind", 120, -0.2f, -4, 18, 4},
};

int grind_kind_for(bool slide, float x, float y, bool crouched)
{
	bool side = std::fabs(x) > 0.45f, up = y > 0.45f, down = y < -0.45f;
	if (slide) {
		if (up)
			return side ? SK8_GRIND_NOSEBLUNT : SK8_GRIND_NOSESLIDE;
		if (down)
			return side ? SK8_GRIND_BLUNTSLIDE : SK8_GRIND_TAILSLIDE;
		return side ? SK8_GRIND_LIPSLIDE : SK8_GRIND_BOARDSLIDE;
	}
	// the right stick, as the port: up (or up-right) nosegrind, down (or down-right) 5-0, right
	// crooked (held, it slides on into a boardslide), left lipslide, up-left overcrook, down-left
	// willy; with the left stick held back: left smith, right feeble, down-right salad, up-right suski
	bool right = x > 0.45f, left = x < -0.45f;
	if (crouched && side)
		return up ? SK8_GRIND_SUSKI : down ? SK8_GRIND_SALAD : right ? SK8_GRIND_FEEBLE : SK8_GRIND_SMITH;
	if (up)
		return left ? SK8_GRIND_OVERCROOK : SK8_GRIND_NOSEGRIND;
	if (down)
		return left ? SK8_GRIND_WILLY : SK8_GRIND_5_0;
	if (right)
		return SK8_GRIND_CROOKED;
	if (left)
		return SK8_GRIND_LIPSLIDE;
	return crouched ? SK8_GRIND_5_0 : SK8_GRIND_50_50;
}

// hands: 1 front (left trigger), 2 back (right trigger), 3 both. Stick y up is toward the nose.
int grab_style_for(int hands, float x, float y)
{
	// the right stick's direction: centre, up (and up-right), up-left, left, down-left, down,
	// down-right, right
	enum { C, U, UL, L, DL, D, DR, R };
	int dir = C;
	if (x * x + y * y > 0.25f) {
		// the nearest of the eight directions, counted round from up toward the right
		int oct = int(std::floor(std::atan2(x, y) * (4.0f / 3.14159265f) + 0.5f)) & 7;
		static const int kOct[8] = {U, U, R, DR, D, DL, L, UL}; // up, up-right (the port: up), right ...
		dir = kOct[oct];
	}
	// [direction][left, right, both]
	static const int kTable[8][3] = {
		{SK8_GRAB_MELON, SK8_GRAB_INDY, SK8_GRAB_DOUBLE},           // C
		{SK8_GRAB_TAIL, SK8_GRAB_SEATBELT, SK8_GRAB_TAIL},          // U
		{SK8_GRAB_JUDO, SK8_GRAB_CHRIST, SK8_GRAB_SUPERMAN},        // UL
		{SK8_GRAB_STIFFY, SK8_GRAB_TUCK_KNEE, SK8_GRAB_DOUBLE},     // L
		{SK8_GRAB_STIFFY, SK8_GRAB_TUCK_KNEE, SK8_GRAB_NO_FOOT},    // DL
		{SK8_GRAB_CRAIL, SK8_GRAB_NOSE, SK8_GRAB_ROCKET},           // D
		{SK8_GRAB_STALEFISH, SK8_GRAB_METHOD, SK8_GRAB_METHOD},     // DR
		{SK8_GRAB_STALEFISH, SK8_GRAB_MUTE, SK8_GRAB_MUTE},         // R
	};
	return kTable[dir][hands == 3 ? 2 : hands == 2 ? 1 : 0];
}

// A one-hand grab started with the stick centred, the stick pushed out while it's held: tweaked
// into a variant (like the port); 0 = no tweak that way
int grab_tweak_for(int style, float x, float y)
{
	if (x * x + y * y <= 0.25f)
		return 0;
	int oct = int(std::floor(std::atan2(x, y) * (4.0f / 3.14159265f) + 0.5f)) & 7; // 0 up, 2 right
	if (style == SK8_GRAB_MELON) {
		static const int k[8] = {SK8_GRAB_NOSEBONE, SK8_GRAB_NOSEBONE, SK8_GRAB_TUCK_KNEE, SK8_GRAB_TAILBONE,
			SK8_GRAB_TAILBONE, SK8_GRAB_STIFFY, SK8_GRAB_STIFFY, SK8_GRAB_STIFFY};
		return k[oct];
	}
	if (style == SK8_GRAB_INDY) {
		static const int k[8] = {SK8_GRAB_MELON, SK8_GRAB_MELON, SK8_GRAB_METHOD, SK8_GRAB_JAPAN,
			SK8_GRAB_JAPAN, SK8_GRAB_CROSS_BONE, SK8_GRAB_CROSS_BONE, SK8_GRAB_CROSS_BONE};
		return k[oct];
	}
	return 0;
}
} // namespace


namespace {

constexpr float kPushBite = 0.12f; // s: how long a stroke's foot takes to give its speed
constexpr float kSkidTime = 0.3f;
constexpr float kVertIn = 0.15f; // m: a vert air comes back down this far inside the lip
constexpr float kSettleFlip = 0.45f;
constexpr float kCrookToBoard = 0.13f;  // s a crooked holds before turning across into a boardslide
constexpr float kPlantWind = 0.605f;
constexpr float kFootOffHold = 0.1f;
constexpr float kBigAir = 1.4f, kLongAir = 9.5f; // m: an air this high (above its lower end) / long scores  // s a foot held off in the air before it counts
constexpr float kAdjustYaw = 0.5f, kAdjustPitch = 0.38f; // rad: a board adjust at full stick
constexpr float kUnderTurn = 0.34f;   // turns an underflip goes its first way before it turns back
constexpr float kUnderWindow = 0.11f; // s into a flip the opposite flick still turns it into an underflip   // s: a boneless's wind-up, push pressed to the pop
constexpr float kPlantVariant = 0.15f; // s a handplant variant's buttons / stick hold before it changes
constexpr float kGrindTransfer = 0.12f; // s the right stick holds a new way before the grind changes // s for a whole flip's worth of settling (the last bit: a few ticks)  // s: a crooked landing scrubs its speed off over this
constexpr float kSettleClean = 0.8f, kSettleSketchy = 1.26f; // s: riding away from a landing before it's settled

constexpr float kDt = SK8_DT;
constexpr float kDeg = kPi / 180.0f;
constexpr float kBoardHeight = 0.18f; // wall probes run this far above the wheels
constexpr float kRadius = 0.25f;

float smooth01(float t)
{
	t = clampf(t, 0, 1);
	return t * t * (3 - 2 * t);
}

bool ground_mode(sk8_mode m) { return m == SK8_MODE_RIDE || m == SK8_MODE_MANUAL || m == SK8_MODE_SLIDE; }
bool on_foot(sk8_mode m) { return m == SK8_MODE_WALK; }

} // namespace

Rider::Rider() { place({0, 0, 0}, 0, 0); }

float Rider::noise()
{
	seed_ = seed_ * 1664525u + 1013904223u;
	return float(seed_ >> 8) / float(1u << 23) - 1.0f;
}

void Rider::emit(sk8_event_kind kind, const char *name, unsigned points, int value, int surface)
{
	sk8_event e{};
	e.kind = kind;
	std::snprintf(e.name, sizeof e.name, "%s", name);
	e.points = points;
	e.value = value;
	e.surface = surface;
	events.push_back(e);
	if (events.size() > 64)
		events.pop_front();
}

void Rider::place(V3 position, float facing, float speed)
{
	pos_ = position;
	fwd_ = {std::sin(facing), 0, std::cos(facing)};
	up_ = kUp;
	vel_ = fwd_ * speed;
	mode_ = SK8_MODE_RIDE;
	fakie_ = false;
	switch_ = false;
	switching_ = -1;
	fakie_time_ = 0;
	push_queued_ = false;
	skid_time_ = 0;
	vert_air_ = false;
	carve_in_ = carve_ = 0;
	flick_.reset();
	crouch_ = crouch_time_ = 0;
	flip_ = -1;
	spin_speed_ = spin_total_ = 0;
	air_time_ = 0;
	bail_time_ = -1;
	bail_stage_ = 0;
	rail_ = -1;
	grind_kind_ = 0;
	manual_dir_ = 0;
	balance_ = 0;
	pending_.clear();
	trick_.clear();
	safe_pos_ = position;
	safe_facing_ = facing;
	grind_cooldown_ = 0;
}

// ---------------------------------------------------------------- scoring

void Rider::add_trick(const std::string &name, unsigned points)
{
	// the same trick again in one line is worth less each time (as the port: grinds keep their
	// value, rails being what a line is made of)
	bool exempt = mode_ == SK8_MODE_GRIND && grind_kind_ != SK8_GRIND_FOOTPLANT && grind_kind_ != SK8_GRIND_HANDPLANT;
	if (!exempt) {
		size_t again = size_t(std::count(combo_names_.begin(), combo_names_.end(), name));
		static const float kRepeat[] = {1.0f, 0.7f, 0.5f, 0.35f, 0.25f, 0.15f};
		points = unsigned(float(points) * kRepeat[std::min(again, size_t(5))] + 0.5f);
	}
	combo_names_.push_back(name);
	combo_points_ += points;
	bank_timer_ = 0;
	emit(SK8_EV_TRICK, name.c_str(), points);
}

void Rider::commit_pending()
{
	for (auto &t : pending_)
		add_trick(t.name, t.points);
	pending_.clear();
}

void Rider::bank()
{
	if (combo_names_.empty())
		return;
	double total = double(combo_points_) * double(combo_names_.size()) * double(p[ScoreScale]);
	auto pts = (unsigned long long)(total + 0.5);
	score_ += pts;
	emit(SK8_EV_BANK, "", unsigned(pts));
	best(best_line_, float(pts), "Line score", 1);
	best(best_tricks_, float(combo_names_.size()), "Tricks in a line", 1);
	combo_names_.clear();
	combo_points_ = 0;
	bank_timer_ = 0;
}

// A personal best: kept, and announced when it beats one there already was (value in the unit
// times `scale`: points, centimetres, hundredths of a second).
void Rider::best(float &record, float value, const char *what, float scale)
{
	if (value <= record)
		return;
	bool beaten = record > 0;
	record = value;
	if (beaten)
		emit(SK8_EV_BEST, what, unsigned(value * scale + 0.5f));
}

// Lengths ridden on a rail or a manual, kept as bests when they end.
void Rider::track_runs()
{
	float sp = len(vel_);
	if (mode_ == SK8_MODE_GRIND && grind_kind_ != SK8_GRIND_FOOTPLANT && grind_kind_ != SK8_GRIND_HANDPLANT)
		grind_run_ += rail_speed_ * kDt;
	else if (grind_run_ > 0) {
		best(best_grind_, grind_run_, "Grind length", 100);
		grind_run_ = 0;
	}
	if (mode_ == SK8_MODE_MANUAL)
		manual_run_ += sp * kDt;
	else if (manual_run_ > 0) {
		best(best_manual_, manual_run_, "Manual length", 100);
		manual_run_ = 0;
	}
}

void Rider::lose_combo()
{
	combo_names_.clear();
	combo_points_ = 0;
	pending_.clear();
	bank_timer_ = 0;
}

// ---------------------------------------------------------------- step

void Rider::step(World &w, const sk8_input &in)
{
	if (settle_trick_ >= 0) {
		settle_e_ += kDt / (kSettleFlip * std::max(settle_mult_, 1.0f));
		if (settle_e_ >= 1 || !ground_mode(mode_))
			settle_trick_ = -1;
	}
	step_count_++;
	Flick::Result f;
	if (mode_ != SK8_MODE_BAIL)
		f = flick_.feed(in.flick[0], in.flick[1]);

	// crouch: holding the first token of a pop (down, or up for a nollie)
	int lead = flick_.lead();
	if (flick_.holding_lead() && (lead == TDown || lead == TUp) && mode_ != SK8_MODE_AIR) {
		crouch_time_ += kDt;
		crouch_ = clampf(crouch_time_ / std::max(p[CrouchCharge], 0.01f), 0, 1);
	} else {
		crouch_time_ = 0;
		if (f.trick < 0)
			crouch_ = approach(crouch_, 0, 6 * kDt);
	}
	lean_ = approach(lean_, clampf(in.steer[0], -1, 1), 5 * kDt);
	if (grind_cooldown_ > 0)
		grind_cooldown_ -= kDt;

	since_land_ += kDt;
	since_wall_ += kDt;
	// off the board / back on: a press of the button
	bool off_press = (in.buttons & SK8_OFF_BOARD) && !(last_buttons_ & SK8_OFF_BOARD);
	prev_buttons_ = last_buttons_;
	last_buttons_ = in.buttons;
	if (off_press && mode_ == SK8_MODE_RIDE && takeoff_trick_ < 0) {
		commit_pending();
		if (switch_) { // off the board the feet are the rider's own again (the frame turns back with them)
			fwd_ = fwd_ * -1.0f;
			fakie_ = !fakie_;
			switch_ = false;
		}
		switching_ = -1;
		push_queued_ = false;
		mode_ = SK8_MODE_WALK;
		push_time_ = -1;
		vel_ *= 0.55f; // steps off running it out
		emit(SK8_EV_LAND, "off board", 0, 0, surface_);
	} else if (off_press && mode_ == SK8_MODE_WALK) {
		fakie_ = false;
		if (len(vel_) > 2.6f) {
			// running: the board thrown down ahead and jumped onto (a caveman), carrying the run
			vel_ = fwd_ * (len(vel_) * 1.2f);
			mode_ = SK8_MODE_AIR;
			air_time_ = 0;
			spin_total_ = spin_speed_ = 0;
			pop_y_ = pos_.y;
	varial_ = 0;
			vel_ += up_ * 2.6f;
			pos_ += up_ * 0.02f;
			emit(SK8_EV_POP, "Caveman", 0, 0, surface_);
			pending_.push_back({"Caveman", 160});
		} else {
			mode_ = SK8_MODE_RIDE; // drops the board and steps on
			emit(SK8_EV_LAND, "on board", 0, 0, surface_);
		}
	}
	last_flick_[0] = in.flick[0];
	last_flick_[1] = in.flick[1];
	last_steer_[0] = in.steer[0];
	last_steer_[1] = in.steer[1];
	foot_off_ = (in.buttons & SK8_FOOT_OFF) && mode_ == SK8_MODE_RIDE;
	// Manuals from the right stick: tilted part-way down (manual) or up (nose manual) and held there.
	{
		float x = in.flick[0], y = in.flick[1], r = std::sqrt(x * x + y * y);
		int zone = r >= kManualIn && std::fabs(y) > std::fabs(x) * 0.8f ? (y < 0 ? 1 : -1) : 0;
		// pushing on to the rim from a manual is crouching to pop out of it, still the same manual
		if (zone != 0 && r >= kRimIn && mode_ != SK8_MODE_MANUAL)
			zone = 0;
		stick_manual_time_ = zone != 0 && zone == stick_manual_ ? stick_manual_time_ + kDt : 0;
		stick_manual_ = zone;
		stick_manual_depth_ = clampf((r - kManualIn) / (kRimIn - kManualIn), 0, 1);
	}

	if (!ground_mode(mode_))
		takeoff_trick_ = -1;
	// a stroke cut short (a pop, a grind, a bail) is over: it doesn't pick up again on landing
	if (mode_ != SK8_MODE_RIDE && push_time_ >= 0) {
		push_time_ = -1;
		push_cooldown_ = 0;
	}

	switch (mode_) {
	case SK8_MODE_RIDE:
	case SK8_MODE_MANUAL:
	case SK8_MODE_SLIDE: ground(w, in, f); break;
	case SK8_MODE_AIR: air(w, in, f); break;
	case SK8_MODE_GRIND: grind(w, in, f); break;
	case SK8_MODE_BAIL: bailed(w); break;
	case SK8_MODE_WALLRIDE: wallride(w, in, f); break;
	case SK8_MODE_WALK: walk(w, in); break;
	}
	float target = mode_ == SK8_MODE_SLIDE ? 1.0f : 0.0f;
	slide_amount_ = approach(slide_amount_, target, 6 * kDt);
}

// ---------------------------------------------------------------- ground

void Rider::ground(World &w, const sk8_input &in, const Flick::Result &f)
{
	if (takeoff_trick_ >= 0) {
		takeoff_time_ += kDt;
		crouch_ = takeoff_crouch_; // the pop keeps the crouch the flick had
		under_wanted_ = under_wanted_ || opposite_flick(takeoff_dir_);
		if (takeoff_time_ >= p[PopWait]) {
			int trick = takeoff_trick_;
			takeoff_trick_ = -1;
			if (mode_ == SK8_MODE_MANUAL) {
				add_trick(manual_name(), 50 + unsigned(manual_time_ * p[ManualRate]));
				manual_dir_ = 0;
			}
			const TrickDef &def = trick_table()[size_t(trick)];
			pop(def, takeoff_share_);
			if (flip_ < 0 && !def.flat)
				start_flip(trick);
			else if (def.flat)
				pending_.push_back({staged(def.name), def.points});
			return;
		}
	}
	if (f.trick >= 0) {
		if (mode_ == SK8_MODE_MANUAL) {
			add_trick(manual_name(),
				50 + unsigned(manual_time_ * p[ManualRate]));
			manual_dir_ = 0;
		}
		int trick = f.trick;
		float share = trick_table()[size_t(trick)].nollie ? p[NolliePop] : 1.0f;
		// the foot-off button: the front foot steps down off the board first (old school)
		float old_pop = 1;
		int old = (in.buttons & SK8_FOOT_OFF) && mode_ == SK8_MODE_RIDE ? foot_off_variant(trick, &old_pop) : -1;
		if (old >= 0) {
			trick = old;
			share = old_pop;
		}
		if (p[PopWait] > 0 && takeoff_trick_ < 0) {
			// the board loads up before it leaves the ground (the takeoff), then pops
			takeoff_trick_ = trick;
			takeoff_share_ = share;
			takeoff_time_ = 0;
			takeoff_crouch_ = crouch_;
			flick_dir(takeoff_dir_);
			under_wanted_ = false;
			return;
		}
		const TrickDef &def = trick_table()[size_t(trick)];
		pop(def, share);
		if (flip_ < 0 && !def.flat)
			start_flip(trick);
		else if (def.flat)
			pending_.push_back({staged(def.name), def.points});
		return;
	}

	// Hippie jump: the foot-off button held through a crouch let go with no flick: the skater jumps
	// and the board rolls on underneath (the host keeps it on the ground)
	if (f.crouch_let_go && (in.buttons & SK8_FOOT_OFF) && mode_ == SK8_MODE_RIDE) {
		const TrickDef &def = trick_table()[size_t(find_trick("Hippie Jump"))];
		crouch_ = 0.6f;
		pop(def, 0.85f);
		pending_.push_back({staged(def.name), def.points});
		return;
	}

	float speed = len(vel_);
	float along = dot(vel_, fwd_);
	if (along < -0.3f)
		fakie_ = true;
	else if (along > 0.3f)
		fakie_ = false;
	// Rolling fakie a moment with nothing else going on, the feet shuffle round: switch, rolling
	// forwards on the other foot (the crouch held keeps it fakie, ready for a fakie trick)
	if (switching_ >= 0) {
		switching_ += kDt / switch_len_;
		if (switching_ >= 1 || mode_ != SK8_MODE_RIDE)
			switching_ = -1;
	}
	// (after a landing: once it has settled from it, longer from a sketchy one)
	bool settled = mode_ == SK8_MODE_RIDE && takeoff_trick_ < 0 && crouch_ < 0.2f && !flick_.holding_lead() &&
		push_time_ < 0 && !foot_off_ && revert_time_ < 0 && since_land_ > land_settle_ && speed > 0.3f;
	if (fakie_ && settled && switching_ < 0 && p[SwitchWait] > 0) {
		fakie_time_ += kDt;
		if (fakie_time_ >= p[SwitchWait])
			start_switch(p[SwitchTime]);
	} else {
		fakie_time_ = 0;
	}
	V3 travel = fakie_ ? -fwd_ : fwd_;
	float steer = clampf(in.steer[0], -1, 1);

	// manual: hold to balance on two wheels
	// (buttons, or the right stick part-way: held a moment so a flick passing through doesn't count;
	// landing with it held goes straight in)
	int stick = stick_manual_ != 0 &&
			(stick_manual_time_ >= p[ManualHold] || (mode_ == SK8_MODE_MANUAL && stick_manual_ == manual_dir_) ||
				since_land_ < 0.1f)
		? stick_manual_ : 0;
	bool want_manual = (in.buttons & (SK8_MANUAL | SK8_NOSE_MANUAL)) || stick != 0;
	if (!want_manual)
		manual_lock_ = false;
	// caught on the landing (held into it) or, pressed fresh, only once the landing has settled
	bool settling = since_land_ >= 0.1f && since_land_ < p[ManualLand];
	if (mode_ == SK8_MODE_RIDE && want_manual && !manual_lock_ && speed > 0.6f && !settling) {
		mode_ = SK8_MODE_MANUAL;
		manual_dir_ = stick != 0 ? stick : (in.buttons & SK8_MANUAL) ? 1 : -1;
		// with the foot-off button: the board flipped onto its side (primo) or upside down (casper)
		manual_style_ = (in.buttons & SK8_FOOT_OFF) ? (manual_dir_ > 0 ? 1 : 2) : 0;
		manual_time_ = 0;
		balance_ = 0;
	}
	if (mode_ == SK8_MODE_MANUAL) {
		manual_time_ += kDt;
		// A held manual settles back toward level (the stick held steady keeps it up); the wobble
		// and the stick push it off. Balance with the left stick, or with how far the right stick
		// is tilted: half way is level, right out at the rim tips it over.
		float drift = noise() * p[ManualWobble] - balance_ * 0.6f;
		float lean = in.steer[1];
		if (stick == manual_dir_)
			lean += (0.5f - stick_manual_depth_) * 0.65f;
		balance_ += (drift - lean * p[ManualCorrect] * float(manual_dir_)) * kDt;
		trick_ = manual_name();
		if (!want_manual || std::fabs(balance_) > 1.0f || speed < 0.3f) {
			float share = std::fabs(balance_) > 1.0f ? 0.5f : 1.0f;
			add_trick(trick_, unsigned((50 + manual_time_ * p[ManualRate]) * share));
			manual_lock_ = std::fabs(balance_) > 1.0f;
			mode_ = SK8_MODE_RIDE;
			manual_dir_ = 0;
			balance_ = 0;
			trick_.clear();
		}
	}

	// powerslide: the left stick pulled back to a diagonal (the port's way: back-right slides
	// right), or the slide button held while steering
	bool stick_slide = in.steer[1] < -0.45f && std::fabs(in.steer[0]) > 0.4f;
	// (the button only on the flat: held riding up a ramp, it's for a handplant at the top)
	bool want_slide = stick_slide || ((in.buttons & SK8_SLIDE) && std::fabs(steer) > 0.4f && up_.y > 0.95f);
	if (mode_ == SK8_MODE_RIDE && want_slide && speed > 1.8f)
		mode_ = SK8_MODE_SLIDE;
	else if (mode_ == SK8_MODE_SLIDE && ((!want_slide && !(in.buttons & SK8_SLIDE)) || speed < 0.6f))
		mode_ = SK8_MODE_RIDE;

	// carve: the turn comes on over a moment (two easing stages: a gentle start, like the port)
	float ease = 1 - std::exp(-kDt / std::max(p[CarveEase], 0.001f));
	carve_in_ += (steer - carve_in_) * ease;
	carve_ += (carve_in_ - carve_) * ease;
	float rate = p[Carve] * carve_ * clampf(speed / 2.5f, 0.35f, 1.0f);
	if (mode_ == SK8_MODE_SLIDE)
		rate *= 0.5f;
	fwd_ = unit(flatten(rotate(fwd_, up_, -rate * kDt), up_), fwd_);
	// The wheels carry the motion round with the board (a carve keeps its speed).
	if (mode_ != SK8_MODE_SLIDE)
		vel_ = rotate(vel_, up_, -rate * kDt);

	// gravity along the surface
	vel_ += flatten(V3{0, -p[Gravity] * p[SlopeGravity], 0}, up_) * kDt; // rolling: less than the air's pull

	// push: a step off the board (first stroke only), then strokes; the speed comes on while the
	// foot is on the ground driving back, not the moment the button goes down
	if (push_cooldown_ > 0)
		push_cooldown_ -= kDt;
	if (push_time_ >= 0) {
		float period = std::max(p[PushPeriod], 0.01f);
		float t0 = push_lead_ + SK8_PUSH_PLANT * period, t1 = push_lead_ + SK8_PUSH_RELEASE * period;
		// the speed comes on in a burst as the foot bites (the first part of the plant): a share of
		// what's left to top speed, so strokes give less and less as it gets going
		t1 = std::min(t1, t0 + kPushBite);
		float drive = std::min(push_time_ + kDt, t1) - std::max(push_time_, t0);
		if (push_time_ <= t0)
			push_gain_ = std::min(p[PushCap], std::max(0.0f, p[PushTop] - std::fabs(dot(vel_, travel))) * p[PushKick]);
		if (mode_ == SK8_MODE_RIDE && drive > 0)
			vel_ += travel * (push_gain_ * drive / (t1 - t0));
		push_time_ += kDt;
		if (push_time_ > push_lead_ + period || mode_ != SK8_MODE_RIDE) {
			push_time_ = -1;
			since_stroke_ = 0;
		}
	} else {
		since_stroke_ += kDt;
	}
	// boneless (the port's way in): a grab trigger held with push pressed: no stroke, the front foot
	// steps down off the board for a wind-up, then the pop. The left stick up on the way: a fastplant.
	const unsigned grab_bits = SK8_GRAB_LEFT | SK8_GRAB_RIGHT, push_bits = SK8_PUSH | SK8_PUSH_MONGO;
	bool grab_held = (in.buttons & grab_bits) != 0;
	bool plant_press = grab_held && (in.buttons & push_bits) &&
		(((in.buttons & ~prev_buttons_) & push_bits) || !(prev_buttons_ & grab_bits));
	if (plant_press && plant_wind_ < 0 && mode_ == SK8_MODE_RIDE && takeoff_trick_ < 0 && switching_ < 0) {
		plant_wind_ = 0;
		// as the port: the mongo button (the front foot's, so the other one rolling fakie) is a
		// fastplant; the left trigger frontside, the right backside (swapped riding the other way)
		unsigned fresh = (in.buttons & ~prev_buttons_) & push_bits;
		bool mongo = fresh ? (fresh & SK8_PUSH_MONGO) && !(fresh & SK8_PUSH) : !(in.buttons & SK8_PUSH);
		plant_fast_ = mongo != fakie_;
		bool left = (in.buttons & SK8_GRAB_LEFT) && (!(in.buttons & SK8_GRAB_RIGHT) || !(prev_buttons_ & SK8_GRAB_LEFT));
		plant_side_ = left != (fakie_ != switch_) ? "Frontside " : "Backside ";
		push_time_ = -1;
		push_queued_ = false;
	}
	if (plant_wind_ >= 0 && mode_ != SK8_MODE_RIDE)
		plant_wind_ = -1;
	if (plant_wind_ >= 0) {
		plant_wind_ += kDt;
		crouch_ = std::min(0.85f, plant_wind_ / kPlantWind); // loads up as the foot goes down
		if (plant_wind_ >= kPlantWind) {
			const TrickDef &def = trick_table()[size_t(find_trick(plant_fast_ ? "Fastplant" : "Boneless"))];
			pop(def, p[BonelessPop]);
			pending_.push_back({staged((plant_side_ + def.name).c_str()), def.points});
			return;
		}
	}
	// a trigger held rolling along (no push): crouched, a hand on the board (as the port: a pose; a
	// manual or a powerslide lets go of it)
	{
		bool lt = in.buttons & SK8_GRAB_LEFT, rt = in.buttons & SK8_GRAB_RIGHT;
		if (switch_)
			std::swap(lt, rt);
		bool can = mode_ == SK8_MODE_RIDE && plant_wind_ < 0 && !(in.buttons & push_bits);
		ground_grab_ = can ? (lt ? 1 : 0) | (rt ? 2 : 0) : 0;
	}
	// pushing while fakie: a quick shuffle round to switch first, then the push
	bool push_button = ((in.buttons & push_bits) && !grab_held) || push_queued_;
	if (push_button && fakie_ && mode_ == SK8_MODE_RIDE && switching_ < 0 && push_time_ < 0 && p[SwitchWait] > 0) {
		start_switch(p[SwitchTime] * 0.5f);
		push_queued_ = true;
	}
	if (mode_ != SK8_MODE_RIDE)
		push_queued_ = false;
	// after a brake the foot comes back up onto the board first
	if (in.buttons & SK8_BRAKE)
		brake_up_ = brake_held_ > p[BrakeLead] ? p[BrakeOut] : 0;
	else if (brake_up_ > 0)
		brake_up_ -= kDt;
	if (push_button && switching_ < 0 && mode_ == SK8_MODE_RIDE && push_cooldown_ <= 0 && push_time_ < 0 && plant_wind_ < 0 &&
		brake_up_ <= 0 && !(in.buttons & SK8_BRAKE) &&
		std::fabs(dot(vel_, travel)) < p[PushTop]) {
		push_queued_ = false;
		bool mongo = (in.buttons & SK8_PUSH_MONGO) || p[PushMongo] > 0.5f;
		// straight on from the last stroke (same foot) the foot is already off the board
		push_lead_ = since_stroke_ < 0.05f && mongo == push_mongo_ ? 0.0f : p[PushLead];
		push_mongo_ = mongo;
		push_cooldown_ = push_lead_ + p[PushPeriod];
		push_time_ = 0;
		emit(SK8_EV_PUSH);
	}

	// drag and brake
	speed = len(vel_);
	float dec = p[RollDrag] * kDt + p[AirDrag] * speed * speed * kDt;
	// the foot goes down to the ground first, then drags
	brake_held_ = (in.buttons & SK8_BRAKE) ? brake_held_ + kDt : 0;
	if (brake_held_ > p[BrakeLead])
		dec += p[Brake] * kDt;
	if (skid_time_ > 0) {
		dec += skid_ * kDt;
		skid_time_ -= kDt;
	}
	if (mode_ == SK8_MODE_SLIDE) // bites in as the board turns across, eases off as it slows
		dec += p[SlideDrag] * slide_amount_ * slide_amount_ * std::min(1.0f, speed / p[SlideSoft]) * kDt;
	if (mode_ == SK8_MODE_MANUAL)
		dec += (manual_style_ == 1 ? 2.7f : p[ManualDrag]) * kDt; // two wheels rolling; a primo scrapes its edge
	vel_ = speed > dec ? vel_ * ((speed - dec) / speed) : V3{};

	// a revert turns the board half way round in kRevertTime, easing in and out; the motion carries
	// on (the wheels slide round, they don't steer it)
	bool reverting = revert_time_ >= 0 && mode_ == SK8_MODE_RIDE;
	if (reverting) {
		constexpr float kRevertTime = 0.34f;
		float t0 = revert_time_ / kRevertTime, t1 = (revert_time_ + kDt) / kRevertTime;
		fwd_ = unit(rotate(fwd_, up_, revert_dir_ * kPi * (smooth01(t1) - smooth01(t0))), fwd_);
		revert_time_ += kDt;
		if (revert_time_ >= kRevertTime)
			revert_time_ = -1;
	} else {
		revert_time_ = -1;
	}

	// wheels resist sideways motion
	V3 side = unit(cross(up_, fwd_), V3{1, 0, 0});
	float lateral = dot(vel_, side);
	float grip = 1 - std::exp(-p[Grip] * kDt * (mode_ == SK8_MODE_SLIDE || reverting ? 0.08f : 1.0f));
	vel_ -= side * (lateral * grip);
	vel_ = flatten(vel_, up_);

	move_on_ground(w, vel_ * kDt);
	if (mode_ == SK8_MODE_BAIL)
		return;
	follow_ground(w);
	if (!ground_mode(mode_))
		return;

	// a place to come back to after a bail
	if (mode_ == SK8_MODE_RIDE && up_.y > 0.95f && len(vel_) < 12) {
		safe_timer_ += kDt;
		if (safe_timer_ > 0.5f) {
			safe_timer_ = 0;
			safe_pos_ = pos_;
			safe_facing_ = std::atan2(fwd_.x, fwd_.z);
		}
	}
	track_runs();
	if (mode_ == SK8_MODE_RIDE && !combo_names_.empty()) {
		bank_timer_ += kDt;
		if (bank_timer_ >= p[BankWait])
			bank();
	}
	if (mode_ == SK8_MODE_SLIDE)
		trick_ = "Powerslide";
	else if (mode_ == SK8_MODE_RIDE)
		trick_.clear();
}

void Rider::move_on_ground(World &w, V3 delta)
{
	float L = len(delta);
	if (L < 1e-6f)
		return;
	V3 dir = delta * (1.0f / L);
	Hit h;
	if (w.cast(pos_ + up_ * kBoardHeight, dir * (L + kRadius), h)) {
		float facing_up = dot(h.normal, up_);
		if (facing_up < 0.55f) { // a wall
			float into = -dot(vel_, h.normal);
			if (into > p[WallBail] && !never_bail()) {
				// bounced back off it: the body lands away from the wall, not in it
				vel_ -= h.normal * (dot(vel_, h.normal) * 1.8f);
				bail("Wall");
				return;
			}
			float ok = std::max(0.0f, h.fraction * (L + kRadius) - kRadius);
			pos_ += dir * ok;
			vel_ -= h.normal * dot(vel_, h.normal);
			vel_ *= 0.75f;
			V3 along = unit(flatten(flatten(fwd_, h.normal), up_), fwd_);
			fwd_ = along;
			return;
		}
		// a ramp rising ahead: climb onto it
		pos_ = h.point;
		float sp = len(vel_);
		up_ = h.normal;
		fwd_ = unit(flatten(fwd_, up_), fwd_);
		vel_ = unit(flatten(vel_, up_), fwd_) * sp;
		surface_ = h.surface;
		return;
	}
	pos_ += delta;
}

void Rider::follow_ground(World &w)
{
	float speed = len(vel_);
	float snap = p[GroundSnap] + speed * kDt * 0.3f;
	Hit h;
	if (w.cast(pos_ + up_ * 0.35f, up_ * -(0.35f + snap), h) && angle_between(h.normal, up_) < 55 * kDeg) {
		pos_ = h.point;
		up_ = h.normal;
		surface_ = h.surface;
		fwd_ = unit(flatten(fwd_, up_), fwd_);
		vel_ = unit(flatten(vel_, up_), fwd_) * speed;
		if (up_.y < 0.25f && speed < 1.2f) { // too slow to hold a near-vertical wall
			mode_ = SK8_MODE_AIR;
			air_time_ = 0;
			wall_off_ = true;
			vert_in_ = unit(flatten(up_, kUp), up_);
			vel_ += up_ * 0.4f;
		}
		return;
	}
	// rolled off the top of a steep wall going up (a vert air): no drift out over the lip - the
	// motion toward or away from the wall is let go, with a little back toward it, so it comes down
	// into the transition again (as the port does)
	if (up_.y < 0.5f && vel_.y > 0.5f) {
		V3 in = flatten(up_, kUp);
		if (len(in) > 0.2f) {
			in = unit(in, in);
			// enough to come down ~kVertIn inside the lip over the air it will have
			float t_air = 2 * vel_.y / std::max(p[Gravity], 1.0f);
			vel_ = vel_ - in * dot(vel_, in) + in * clampf(kVertIn / std::max(t_air, 0.1f), 0.05f, 0.6f);
			vert_air_ = true;
			wall_off_ = true;
			vert_in_ = in;
		}
	}
	// rolled off an edge
	if (mode_ == SK8_MODE_MANUAL) {
		add_trick(manual_name(), 50 + unsigned(manual_time_ * p[ManualRate]));
		manual_dir_ = 0;
		balance_ = 0;
	}
	mode_ = SK8_MODE_AIR;
	air_time_ = 0;
	spin_total_ = 0;
	spin_speed_ = 0;
}

// ---------------------------------------------------------------- air

void Rider::pop(const TrickDef &def, float share)
{
	float charge = std::max(crouch_, 0.35f);
	float v = (p[PopLow] + (p[PopHigh] - p[PopLow]) * charge) * share;
	pop_y_ = pos_.y;
	push_time_ = -1; // a stroke ends at the pop
	skid_time_ = 0;
	vert_air_ = false;
	push_cooldown_ = 0;
	vel_ += up_ * v;
	pos_ += up_ * 0.02f;
	mode_ = SK8_MODE_AIR;
	air_time_ = 0;
	spin_total_ = 0;
	crouch_ = crouch_time_ = 0;
	grab_scored_ = false;
	grab_time_ = 0;
	stance_pre_ = switch_ ? "Switch " : fakie_ ? "Fakie " : "";
	switching_ = -1; // a pop cuts a shuffle short
	push_queued_ = false;
	plant_wind_ = -1;
	plant_air_ = false;
	wall_off_ = false;
	air_feet_ = air_feet_want_ = 0;
	air_feet_scored_ = false;
	adjust_[0] = adjust_[1] = 0;
	emit(SK8_EV_POP, staged(def.name).c_str(), 0, 0, surface_);
}

void Rider::start_switch(float duration)
{
	fwd_ = fwd_ * -1.0f;
	fakie_ = !fakie_;
	switch_ = !switch_;
	switching_ = 0;
	switch_len_ = std::max(duration, 0.05f);
	fakie_time_ = 0;
}

void Rider::start_flip(int trick)
{
	const TrickDef &def = trick_table()[size_t(trick)];
	flip_ = trick;
	flip_time_ = 0;
	float turns = std::max({std::fabs(def.roll), std::fabs(def.shove) * 1.2f, std::fabs(def.wrap) * 1.1f});
	flip_angle_ = flip_rate_ = 0;
	flip_turns_ = flip_base_turns_ = std::max(turns, 0.3f);
	pending_.push_back({staged(def.name), def.points});
	trick_ = staged(def.name);
	flip_mult_ = 1;
	flip_extra_ = 0;
	flip_name_ = staged(def.name);
	float r = std::sqrt(last_flick_[0] * last_flick_[0] + last_flick_[1] * last_flick_[1]);
	flip_dir_[0] = r > 0.3f ? last_flick_[0] / r : 0;
	flip_dir_[1] = r > 0.3f ? last_flick_[1] / r : 0;
	under_a0_ = -1;
	flip_speed_ = 1;
	if (takeoff_dir_[0] != 0 || takeoff_dir_[1] != 0) { // (flicked before a takeoff: the flick's own way)
		flip_dir_[0] = takeoff_dir_[0];
		flip_dir_[1] = takeoff_dir_[1];
		takeoff_dir_[0] = takeoff_dir_[1] = 0;
	}
	if (under_wanted_)
		start_underflip();
	under_wanted_ = false;
}

// The stick's way, as a unit vector (zero near the centre).
void Rider::flick_dir(float out[2]) const
{
	float r = std::sqrt(last_flick_[0] * last_flick_[0] + last_flick_[1] * last_flick_[1]);
	out[0] = r > 0.3f ? last_flick_[0] / r : 0;
	out[1] = r > 0.3f ? last_flick_[1] / r : 0;
}

// The right stick out at the rim on the mirror image of a flick's way (across the stick's up-down
// line: a kickflip's up-right, up-left)
bool Rider::opposite_flick(const float dir[2]) const
{
	float r = std::sqrt(last_flick_[0] * last_flick_[0] + last_flick_[1] * last_flick_[1]);
	if (r < kRimIn || std::fabs(dir[0]) < 0.3f)
		return false;
	return (-last_flick_[0] * dir[0] + last_flick_[1] * dir[1]) / r > 0.87f;
}

// An underflip (as the port: the opposite way flicked just as the board starts to flip): the board
// turns back and goes round once the other way instead, landing when the flip would have.
void Rider::start_underflip()
{
	const TrickDef &def = trick_table()[size_t(flip_)];
	if (def.roll == 0 || def.shove != 0 || def.wrap != 0 || flip_extra_ > 0)
		return;
	// (it carries on its first way a third of a turn before it turns back)
	float a0 = std::max(flip_angle_, kUnderTurn * flip_base_turns_);
	float left = flip_turns_ - flip_angle_;
	under_a0_ = a0;
	flip_turns_ = 2 * a0 + flip_base_turns_;
	flip_speed_ = (flip_turns_ - flip_angle_) / std::max(left, 0.2f);
	flip_name_ = staged(def.name) + " Underflip";
	trick_ = flip_name_;
	if (!pending_.empty()) {
		pending_.back().name = flip_name_;
		pending_.back().points += 150;
	}
}

namespace {
const char *const kTimes[] = {"", "", "Double ", "Triple ", "Quad "};

// The name with a "Double ", "360 " ... count put in front of it (any given one dropped).
std::string counted(const std::string &base, bool halves, int count)
{
	std::string b = base;
	for (const char *pre : {"Double ", "360 ", "Frontside 360 "})
		if (b.rfind(pre, 0) == 0)
			b = (std::strcmp(pre, "Frontside 360 ") == 0 ? "Frontside " : "") + b.substr(std::strlen(pre));
	if (halves) { // shove-its: 180 per half turn
		std::string deg = std::to_string(count * 180) + " ";
		if (b.rfind("Frontside ", 0) == 0)
			return "Frontside " + deg + b.substr(10);
		if (b.rfind("Pop ", 0) == 0)
			b = b.substr(4);
		return count == 1 ? base : deg + b;
	}
	return count <= 1 ? b : kTimes[std::min(count, 4)] + b;
}
} // namespace

// Held into the air at the flick's end, a flip that's past half way gets one more turn: the board
// speeds up to fit it in without a jump in where it is.
void Rider::extend_flip()
{
	const TrickDef &def = trick_table()[size_t(flip_)];
	bool halves = def.roll == 0 && def.wrap == 0;
	float main = halves ? std::fabs(def.shove) * 2 : std::max(std::fabs(def.roll), std::fabs(def.wrap));
	if (main <= 0)
		return;
	int now = int(std::lround(main * flip_mult_));
	float mult = float(now + 1) / main;
	float turns = flip_base_turns_ * mult;
	// held, it goes for the next turn if the air looks long enough for it, judged hopefully (like
	// the port: a just-too-small air still tries it and comes down under-flipped)
	float h = std::max(pos_.y - pop_y_, 0.0f), g = p[Gravity];
	float to_land = (vel_.y + std::sqrt(std::max(vel_.y * vel_.y + 2 * g * h, 0.0f))) / g;
	if ((turns - flip_angle_) * p[FlipPerRev] > to_land * p[HeldReach])
		return;
	flip_turns_ = turns;
	flip_mult_ = mult;
	flip_extra_++;
	flip_name_ = stance_pre_ + counted(def.name, halves, now + 1);
	trick_ = flip_name_;
	if (!pending_.empty()) {
		pending_.back().name = flip_name_;
		pending_.back().points += 140;
	}
}

void Rider::air(World &w, const sk8_input &in, const Flick::Result &f)
{
	if (air_time_ <= 0) { // into the air: where from, for the air's height and length
		launch_ = pos_;
		peak_ = pos_.y;
	}
	air_time_ += kDt;
	peak_ = std::max(peak_, pos_.y);
	vel_ += V3{0, -p[Gravity], 0} * kDt;
	float sp = len(vel_);
	if (sp > 0)
		vel_ *= std::max(0.0f, 1 - p[AirDrag] * sp * kDt);

	// spin about the stance axis; near a rail the stick is picking the grind instead
	bool by_rail = !w.rails.empty() && w.nearest_rail(pos_, 1.0f).rail >= 0;
	float target = by_rail ? 0.0f : -clampf(in.steer[0], -1, 1) * p[SpinRate];
	spin_speed_ = approach(spin_speed_, target, p[SpinEase] * p[SpinRate] * kDt);
	float a = spin_speed_ * kDt;
	fwd_ = unit(rotate(fwd_, up_, a), fwd_);
	// Off a slide the board is still across the way it's going: turn it back (to whichever end is
	// nearer) so it comes down rolling.
	if (unslide_) {
		V3 travel = flatten(vel_, up_);
		if (len(travel) > 0.5f) {
			travel = unit(travel, fwd_);
			if (dot(travel, fwd_) < 0)
				travel = -travel;
			float off = angle_between(fwd_, travel);
			float step = std::min(off, 9.0f * kDt);
			float sign = dot(cross(fwd_, travel), up_) >= 0 ? 1.0f : -1.0f;
			fwd_ = unit(rotate(fwd_, up_, sign * step), fwd_);
			if (off < 2 * kDeg)
				unslide_ = false;
		}
	}
	// the foot-off button held: the body turns on its own above the board (a body varial), the
	// board keeping its heading
	if (in.buttons & SK8_FOOT_OFF)
		varial_ += a;
	else
		spin_total_ += a;

	// level out gently unless this is a vert air
	if (up_.y > 0.3f && p[AirLevel] > 0) {
		float tilt = angle_between(up_, kUp);
		if (tilt > 1e-3f) {
			V3 axis = unit(cross(up_, kUp), V3{1, 0, 0});
			float turn = std::min(tilt, p[AirLevel] * kDt);
			up_ = unit(rotate(up_, axis, turn));
			fwd_ = unit(flatten(rotate(fwd_, axis, turn), up_), fwd_);
		}
	}

	// flick a trick late
	// (not while grabbing: the stick then tweaks the grab)
	if (!foot_air_ && f.trick >= 0 && flip_ < 0 && !grab_ && p[AirFlick] >= 0.5f && !trick_table()[size_t(f.trick)].flat)
		start_flip(f.trick);
	if (flip_ >= 0 && under_a0_ < 0 && flip_time_ < kUnderWindow && opposite_flick(flip_dir_))
		start_underflip();
	if (flip_ >= 0) {
		flip_time_ += kDt;
		// spin up, cruise, brake to stop on the last turn
		float cruise = flip_speed_ / std::max(p[FlipPerRev], 0.05f);
		float accel = cruise / std::max(p[FlipBase], 0.01f);
		float left = flip_turns_ - flip_angle_;
		float brake = std::sqrt(std::max(2 * accel * left, 0.0f));
		flip_rate_ = std::min({flip_rate_ + accel * kDt, cruise, std::max(brake, 0.4f)});
		flip_angle_ = std::min(flip_turns_, flip_angle_ + flip_rate_ * kDt);
		if (flip_turns_ - flip_angle_ < 0.003f)
			flip_angle_ = flip_turns_;
		float r = std::sqrt(last_flick_[0] * last_flick_[0] + last_flick_[1] * last_flick_[1]);
		bool held = r >= kRimIn &&
			(last_flick_[0] * flip_dir_[0] + last_flick_[1] * flip_dir_[1]) / r > 0.87f;
		if (held && flip_extra_ < 3 && flip_turns_ - flip_angle_ < 0.5f && under_a0_ < 0)
			extend_flip();
	}

	// grabs: the triggers (swapped in switch) and where the right stick is held as one goes down
	// pick the style, like the port (the stick first, then the trigger)
	bool lt = in.buttons & SK8_GRAB_LEFT, rt = in.buttons & SK8_GRAB_RIGHT;
	if (switch_)
		std::swap(lt, rt);
	int hands = (lt ? 1 : 0) | (rt ? 2 : 0);
	if (hands && hands != grab_) {
		grab_style_ = grab_style_for(hands, in.flick[0], in.flick[1]);
		grab_tweakable_ = in.flick[0] * in.flick[0] + in.flick[1] * in.flick[1] <= 0.25f;
	} else if (hands && grab_tweakable_ && !grab_scored_) {
		if (int t = grab_tweak_for(grab_style_, in.flick[0], in.flick[1])) {
			grab_style_ = t;
			grab_tweakable_ = false;
		}
	}
	grab_ = hands;

	// A (front foot) or X (back foot) held in the air: that foot off the board, held a moment it
	// scores (as the port: a one-foot ollie, or a one-foot grab); B while grabbing lets go of the
	// board altogether (a dismount: a bail)
	if (!foot_air_) {
		bool a = in.buttons & SK8_PUSH, x = in.buttons & SK8_PUSH_MONGO;
		int feet = a && !x ? SK8_PLANT_FRONT_FOOT : x && !a ? SK8_PLANT_BACK_FOOT : 0;
		if (flip_ >= 0 && !grab_)
			feet = 0;
		air_feet_time_ = feet && feet == air_feet_want_ ? air_feet_time_ + kDt : 0;
		air_feet_want_ = feet;
		air_feet_ = feet && air_feet_time_ >= kFootOffHold ? feet : 0;
		if (air_feet_ && !air_feet_scored_) {
			air_feet_scored_ = true;
			std::string name = grab_ ? std::string(grab_def(grab_style_).name) + " One Foot"
				: air_feet_ == SK8_PLANT_FRONT_FOOT ? "One Foot Ollie" : "Back Foot Ollie";
			pending_.push_back({staged(name.c_str()), 120});
		}
		if (grab_ && (in.buttons & SK8_BRAKE) && !(prev_buttons_ & SK8_BRAKE) && !never_bail()) {
			bail("Dismount");
			return;
		}
	}
	// the right stick held with nothing else going on: the board adjusted under the feet (nose up /
	// down, swung to a side)
	{
		bool free = flip_ < 0 && !grab_ && !foot_air_ && air_time_ > 0.08f;
		float r = std::sqrt(in.flick[0] * in.flick[0] + in.flick[1] * in.flick[1]);
		float tx = free && r > 0.6f ? in.flick[0] / std::max(r, 1.0f) : 0, ty = free && r > 0.6f ? in.flick[1] / std::max(r, 1.0f) : 0;
		float k = 1 - std::exp(-kDt / 0.08f);
		adjust_[0] += (tx - adjust_[0]) * k;
		adjust_[1] += (ty - adjust_[1]) * k;
	}
	if (grab_) {
		grab_time_ += kDt;
		if (grab_time_ > 0.3f && !grab_scored_) {
			grab_scored_ = true;
			const GrabDef &g = grab_def(grab_style_);
			pending_.push_back({g.name, g.points});
			if (flip_ < 0)
				trick_ = g.name;
		}
	} else {
		grab_time_ = 0;
		grab_style_ = 0;
	}

	if (!foot_air_ && try_grind(w))
		return;

	V3 delta = vel_ * kDt;
	float L = len(delta);
	Hit h;
	if (L > 1e-6f && w.cast(pos_ + up_ * 0.1f, delta + delta * (0.1f / L), h)) {
		if (foot_air_ || !try_wallride(h))
			land(h);
		return;
	}
	pos_ += delta;
	if (pos_.y < -60) { // fell out of the world
		bail("Out of bounds");
	}
}

void Rider::land(const Hit &h)
{
	since_land_ = 0;
	if (foot_air_) { // back on its feet
		foot_air_ = false;
		pos_ = h.point;
		up_ = kUp;
		vel_ = flatten(vel_, kUp);
		mode_ = SK8_MODE_WALK;
		return;
	}
	V3 n = h.normal;
	float tilt = angle_between(up_, n);
	bool flip_done = flip_landable();
	V3 vflat = flatten(vel_, n);
	V3 fflat = unit(flatten(fwd_, n), fwd_);
	float side = 0;
	bool fakie = fakie_;
	if (len(vflat) > 1.0f) {
		float a = angle_between(unit(vflat), fflat);
		fakie = a > kPi / 2;
		side = fakie ? kPi - a : a;
	}
	float into = -dot(vel_, n);
	// a body varial lands square on the board again: half turns, give or take
	int vhalves = int(std::lround(std::fabs(varial_) / kPi));
	float voff = std::fabs(std::fabs(varial_) - vhalves * kPi);
	const char *bad = nullptr;
	if (voff > 40 * kDeg)
		bad = "Body varial off the board";
	if (!flip_done)
		bad = "Under-flipped";
	else if (tilt > p[LandMax] * kDeg)
		bad = "Bad angle";
	else if (side > p[SideMax] * kDeg)
		bad = "Sideways";
	else if (into > p[ImpactBail])
		bad = "Too hard";
	pos_ = h.point;
	surface_ = h.surface;
	if (bad && !never_bail()) {
		bail(bad);
		return;
	}
	// a big air (high, or long) scores for itself, on top of what was done in it
	if (!foot_air_ && air_time_ > 0.3f) {
		float hgt = peak_ - std::max(launch_.y, h.point.y), dist = len(flatten(h.point - launch_, kUp));
		if (hgt >= kBigAir)
			pending_.push_back({"Big Air", unsigned(60 * hgt * hgt)});
		if (dist >= kLongAir)
			pending_.push_back({"Long Air", unsigned(22 * dist)});
		best(best_air_, air_time_, "Air time", 100);
	}
	up_ = n;
	fwd_ = fflat;
	fakie_ = fakie;
	// line the wheels up with the motion
	V3 travel = fakie ? -fwd_ : fwd_;
	// back down into the transition out of a vert air: the speed carries on along the wall (it
	// comes down nearly along it), none lost into it
	if (vert_air_ && len(vflat) > 0.1f)
		vflat = unit(vflat, travel) * len(vel_);
	vert_air_ = false;
	// landing off the line of travel skids: the further round, the more speed it scrubs off
	float keep = std::max(0.0f, 1 - std::pow(side / (p[LandSkid] * kDeg), 1.6f));
	vel_ = travel * len(vflat);
	bool clean = tilt < p[LandClean] * kDeg && side < 20 * kDeg;

	if (vhalves > 0) {
		pending_.push_back({vhalves == 1 ? "Body Varial" : vhalves == 2 ? "360 Body Varial" : "540 Body Varial",
			unsigned(150 * vhalves)});
		if (vhalves % 2)
			fakie_ = !fakie_; // turned round on it
	}
	varial_ = 0;
	int halves = int(std::lround(std::fabs(spin_total_) / kPi));
	// landing out of a body spin costs speed (each half turn more); the speed lost skids off over
	// kSkidTime, not all at once
	if (halves > 0)
		keep *= std::max(0.5f, 1.0f - 0.15f * float(halves));
	skid_time_ = kSkidTime;
	skid_ = len(vel_) * (1 - keep) / kSkidTime;
	if (halves > 0) {
		char name[24];
		std::snprintf(name, sizeof name, "%d Spin", halves * 180);
		pending_.push_back({name, unsigned(halves * 110)});
	}
	// the slide button held into the landing: a revert (the board spun round on its wheels)
	if ((last_buttons_ & SK8_SLIDE) && !plant_air_) {
		revert_time_ = 0;
		revert_dir_ = lean_ < -0.1f ? -1.0f : 1.0f;
		pending_.push_back({"Revert", 90});
	}
	plant_air_ = false;
	wall_off_ = false;
	air_feet_ = air_feet_want_ = 0;
	air_feet_scored_ = false;
	adjust_[0] = adjust_[1] = 0;
	commit_pending();
	land_settle_ = clean ? kSettleClean : kSettleSketchy;
	if (!clean)
		emit(SK8_EV_LAND, "sketchy", 0, 1, h.surface);
	else
		emit(SK8_EV_LAND, "clean", 0, 2, h.surface);
	// landed a little short of the flip's end: the board finishes the last of its turn under the
	// feet over a moment instead of snapping level
	settle_trick_ = flip_ >= 0 && flip_frac() < 1 && under_a0_ < 0 ? flip_ : -1;
	settle_e_ = flip_ >= 0 ? flip_frac() : 1;
	settle_mult_ = flip_mult_;
	flip_ = -1;
	spin_total_ = spin_speed_ = 0;
	air_time_ = 0;
	grab_ = 0;
	grab_style_ = 0;
	mode_ = SK8_MODE_RIDE;
	trick_.clear();
}

// ---------------------------------------------------------------- walk

namespace {
constexpr float kWalkSpeed = 1.437f, kRunSpeed = 3.7f, kWalkTurn = 2.6f; // m/s, m/s, rad/s
} // namespace

// On foot: the left stick turns (x) and walks or runs (y, past 0.8 a run), the board in the hand.
void Rider::walk(World &w, const sk8_input &in)
{
	float x = clampf(in.steer[0], -1, 1), y = clampf(in.steer[1], -1, 1);
	// A: a jump on foot
	if ((in.buttons & SK8_PUSH) && !(prev_buttons_ & SK8_PUSH)) {
		mode_ = SK8_MODE_AIR;
		foot_air_ = true;
		air_time_ = 0;
		pop_y_ = pos_.y;
		vel_ += kUp * 3.6f;
		pos_ += kUp * 0.02f;
		return;
	}
	fwd_ = unit(flatten(rotate(fwd_, up_, -x * kWalkTurn * kDt), up_), fwd_);
	float want = std::fabs(y) > 0.8f ? kRunSpeed : kWalkSpeed * std::fabs(y) / 0.8f;
	V3 target = fwd_ * (want * (y < 0 ? -0.5f : 1.0f));
	if (std::fabs(y) < 0.15f)
		target = V3{};
	V3 flat = flatten(vel_, up_);
	vel_ = flat + (target - flat) * std::min(1.0f, 7.0f * kDt);
	vel_ += flatten(V3{0, -p[Gravity], 0}, up_) * kDt * 0.1f;
	move_on_ground(w, vel_ * kDt);
	if (mode_ != SK8_MODE_WALK)
		return;
	follow_ground(w);
	if (mode_ != SK8_MODE_WALK) {
		// stepped off an edge: dropping (lands back on the board)
		return;
	}
	up_ = kUp;
}

// ---------------------------------------------------------------- wallride

namespace {
constexpr float kWallGravity = 0.68f; // share of gravity pulling the rider down the wall
constexpr float kWallMax = 1.5f;      // longest ride, s
} // namespace

bool Rider::try_wallride(const Hit &h)
{
	if (std::fabs(h.normal.y) > 0.4f)
		return false;
	V3 n = unit(V3{h.normal.x, 0, h.normal.z}, V3{0, 0, 1});
	if (dot(n, vel_) > 0)
		n = -n;
	V3 along = flatten(vel_, n);
	float into = -dot(vel_, n);
	bool flip_done = flip_landable();
	// a glancing hit at speed rolls on along it; head-on is a slam
	if (len(along) < 3.0f || into > len(along) * 1.1f || !flip_done || grab_)
		return false;
	commit_pending();
	mode_ = SK8_MODE_WALLRIDE;
	wall_n_ = n;
	wall_time_ = 0;
	pos_ = h.point + n * 0.01f;
	vel_ = along;
	up_ = n;
	fwd_ = unit(flatten(fwd_, n), unit(along, fwd_));
	if (dot(fwd_, along) < 0 && !fakie_)
		fwd_ = -fwd_;
	flip_ = -1;
	spin_speed_ = 0;
	trick_ = "Wallride";
	emit(SK8_EV_GRIND, "Wallride", 0, 0, h.surface);
	return true;
}

void Rider::leave_wall(bool popped)
{
	add_trick("Wallride", 120 + unsigned(wall_time_ * p[GrindRate]));
	emit(SK8_EV_GRIND_END, "Wallride");
	mode_ = SK8_MODE_AIR;
	since_wall_ = 0;
	vel_ += wall_n_ * (popped ? 0.0f : 1.2f);
	up_ = kUp;
	fwd_ = unit(flatten(fwd_, kUp), fwd_);
	air_time_ = 0;
	spin_total_ = 0;
	trick_.clear();
	grind_cooldown_ = 0.3f;
}

void Rider::wallride(World &w, const sk8_input &in, const Flick::Result &f)
{
	wall_time_ += kDt;
	if (f.trick >= 0) {
		// pop off the wall: away from it and up
		const TrickDef &def = trick_table()[size_t(f.trick)];
		leave_wall(true);
		up_ = unit(kUp + wall_n_ * 0.8f, kUp);
		pop(def, 0.8f);
		up_ = kUp;
		if (!def.flat)
			start_flip(f.trick);
		else
			pending_.push_back({"Wallie", 140});
		return;
	}
	vel_ += V3{0, -p[Gravity] * kWallGravity, 0} * kDt;
	float sp = len(vel_);
	if (sp > 0)
		vel_ *= std::max(0.0f, 1 - (p[RollDrag] + 0.35f) * kDt / std::max(sp, 0.5f));
	if (len(vel_) > 0.3f)
		fwd_ = unit(flatten(vel_, up_), fwd_) * (fakie_ ? -1.0f : 1.0f);
	V3 next = pos_ + vel_ * kDt;
	// still a wall under the wheels?
	Hit h;
	bool on = w.cast(next + wall_n_ * 0.3f, wall_n_ * -0.6f, h) && std::fabs(h.normal.y) < 0.5f;
	if (!on || wall_time_ > kWallMax || len(vel_) < 1.2f || vel_.y < -4.5f) {
		pos_ = next;
		leave_wall(false);
		return;
	}
	pos_ = h.point + wall_n_ * 0.01f;
	// the floor under a wallride that comes down to it
	Hit g;
	if (vel_.y < 0 && w.cast(pos_ + V3{0, 0.3f, 0}, V3{0, -0.35f, 0}, g) && g.normal.y > 0.7f) {
		leave_wall(false);
		land(g);
	}
}

// ---------------------------------------------------------------- grind

int Rider::plant_bits(const sk8_input &in)
{
	unsigned b = in.buttons;
	bool back = b & SK8_PUSH, front = b & SK8_PUSH_MONGO;
	int bits = (b & SK8_BRAKE) ? SK8_PLANT_BACK_FOOT | SK8_PLANT_FRONT_FOOT
		: front && !back       ? SK8_PLANT_FRONT_FOOT
		: back && !front       ? SK8_PLANT_BACK_FOOT
							   : 0;
	if (in.flick[1] < -0.6f)
		bits |= SK8_PLANT_DOWN;
	else if (std::fabs(in.flick[0]) > 0.6f)
		bits |= in.flick[0] > 0 ? SK8_PLANT_RIGHT : SK8_PLANT_LEFT;
	return bits;
}

std::string Rider::plant_variant(int bits) const
{
	const int feet = SK8_PLANT_BACK_FOOT | SK8_PLANT_FRONT_FOOT;
	std::string name = (bits & feet) == feet ? "No Foot "
		: (bits & SK8_PLANT_FRONT_FOOT)      ? "One Foot "
		: (bits & SK8_PLANT_BACK_FOOT)       ? "Back Foot "
											 : "";
	name = plant_side_ + name + "Handplant";
	if (bits & SK8_PLANT_DOWN)
		name += " Down";
	else if (bits & (SK8_PLANT_LEFT | SK8_PLANT_RIGHT))
		name += (bits & SK8_PLANT_RIGHT) ? " Right" : " Left";
	return name;
}

// Which grab a trigger makes on a rail: the free end of the board, or the edge by that hand.
static const char *grind_grab_name(int kind, int hands)
{
	if (!hands)
		return "";
	bool back_hand = hands == 2;
	switch (kind) {
	case SK8_GRIND_NOSEGRIND:
	case SK8_GRIND_CROOKED:
	case SK8_GRIND_OVERCROOK:
	case SK8_GRIND_NOSESLIDE:
	case SK8_GRIND_NOSEBLUNT: // on the nose: the tail's free
		return back_hand ? " Backside Grab" : " Tail Grab";
	case SK8_GRIND_5_0:
	case SK8_GRIND_SMITH:
	case SK8_GRIND_FEEBLE:
	case SK8_GRIND_TAILSLIDE:
	case SK8_GRIND_BLUNTSLIDE:
	case SK8_GRIND_SALAD:
	case SK8_GRIND_WILLY:
	case SK8_GRIND_SUSKI: // on the tail: the nose is free
		return back_hand ? " Nose Grab" : " Frontside Grab";
	default:
		return back_hand ? " Backside Grab" : " Frontside Grab";
	}
}

std::string Rider::grind_full_name() const
{
	return std::string(switch_ ? "Switch " : grind_backwards_ ? "Fakie " : "") + grind_side_ +
		kGrindDefs[grind_kind_].name + grind_grab_name(grind_kind_, grind_grab_);
}

bool Rider::try_grind(World &w)
{
	// the slide button held going up off a ramp: a handplant on its edge (the port's way in)
	bool hand = (last_buttons_ & SK8_SLIDE) && (wall_off_ || vel_.y > 0.5f);
	if (grind_cooldown_ > 0 || w.rails.empty() || (vel_.y > 2.5f && !hand))
		return false;
	RailSpot s = w.nearest_rail(pos_, p[GrindCatch]);
	if (s.rail < 0 || pos_.y - s.point.y < -0.15f)
		return false;
	float vt = dot(vel_, s.tangent);
	// a plant (foot-off held) catches the edge from any way, even straight at it
	bool plant = (last_buttons_ & SK8_FOOT_OFF) != 0 || hand;
	// a handplant's side: carried along the edge toward the toes, frontside; toward the heels (or
	// straight in), backside (taken before the catch squares the board up)
	V3 toe = cross(fwd_, up_) * (switch_ ? -1.0f : 1.0f);
	float plant_lateral = dot(s.tangent, toe) * dot(vel_, s.tangent);
	if (std::fabs(vt) < 0.8f && !plant)
		return false;
	// over the rail, along it: coming down, the board is drawn onto it over a few ticks (no snap
	// from high up). Across it (a slide) the moment over it is brief: caught at once.
	float gap = pos_.y - s.point.y;
	float along_cos = std::fabs(dot(unit(flatten(vel_, kUp), fwd_), unit(flatten(s.tangent, kUp), s.tangent)));
	if (gap > 0.09f && !plant && along_cos > 0.57f) {
		if (vel_.y < 0) {
			vel_.y = std::min(vel_.y, -gap * 10.7f);
			V3 lat = flatten(s.point - pos_, kUp);
			lat = lat - s.tangent * dot(lat, s.tangent);
			vel_ += lat * (9.0f * kDt);
		}
		return false;
	}
	// half way round a flip, across the rail: it lands upside down on it (a darkslide)
	bool dark = false;
	if (flip_ >= 0 && trick_table()[size_t(flip_)].roll != 0 && flip_frac() > 0.3f && flip_frac() < 0.75f) {
		V3 tt = s.tangent;
		float aa = angle_between(unit(flatten(fwd_, kUp), fwd_), unit(flatten(tt, kUp), tt));
		aa = std::min(aa, kPi - aa);
		dark = aa > 55 * kDeg;
	}
	if (!dark && flip_ >= 0 && !flip_landable() && !never_bail()) {
		bail("Under-flipped");
		return true;
	}
	V3 pre_pos = pos_, pre_vel = vel_;
	rail_ = s.rail;
	seg_ = s.seg;
	seg_u_ = s.u;
	rail_dir_ = vt >= 0 ? 1 : -1;
	rail_speed_ = std::fabs(vt) * p[GrindKeep];
	pos_ = s.point;
	up_ = kUp;
	fwd_ = unit(flatten(fwd_, kUp), fwd_);
	V3 t = s.tangent * float(rail_dir_);
	float a = angle_between(fwd_, unit(flatten(t, kUp), t));
	bool backwards = a > kPi / 2;
	if (backwards)
		a = kPi - a;
	bool slide = a > 55 * kDeg;
	// crouched for the catch: loaded on the ground, or the right stick held down in the air
	bool crouched = crouch_ > 0.5f || last_steer_[1] < -0.6f; // loaded, or the left stick held back
	grind_slide_ = slide;
	grind_transfer_ = 0;
	grind_kind_ = grind_kind_for(slide, last_flick_[0], last_flick_[1], crouched);
	if (dark)
		grind_kind_ = SK8_GRIND_DARKSLIDE;
	if (plant) {
		grind_kind_ = hand || (last_buttons_ & (SK8_GRAB_LEFT | SK8_GRAB_RIGHT)) ? SK8_GRIND_HANDPLANT : SK8_GRIND_FOOTPLANT;
		plant_hold_ = hand ? SK8_SLIDE : SK8_FOOT_OFF;
		plant_side_ = hand ? (plant_lateral > 0.5f ? "Frontside " : "Backside ") : "";
		plant_want_ = 0;
		plant_pose_ = 0;
		plant_want_time_ = 0;
		plant_time_ = 0;
		// (off a ramp's lip going up: the way up the wall, so it comes back down into the ramp)
		plant_in_ = wall_off_ ? vert_in_ * -1.0f : unit(flatten(vel_, kUp), fwd_);
		rail_speed_ = 0;
	}
	// named for how it's ridden: switch stance, or riding the rail backwards (fakie)
	grind_backwards_ = backwards && !slide;
	// frontside / backside as the port names it: the rail on the right of the way it's going,
	// backside; on the left, frontside (the other way round rolling fakie or switch). Coming
	// straight down onto it, by which way it was drifting.
	{
		V3 travel = fakie_ ? fwd_ * -1.0f : fwd_;
		V3 right = cross(travel, kUp);
		float off = dot(flatten(s.point - pre_pos, kUp), right);
		if (std::fabs(off) < 0.04f)
			off = dot(flatten(pre_vel, kUp), right);
		bool on_right = off >= 0;
		grind_side_ = on_right != (fakie_ != switch_) ? "Backside " : "Frontside ";
	}
	grind_grab_ = 0;
	grind_grab_want_ = 0;
	grind_grab_time_ = 0;
	grind_name_ = grind_full_name();
	if (plant && hand)
		grind_name_ = plant_side_ + kGrindDefs[grind_kind_].name;
	fakie_ = backwards && !slide;
	grind_time_ = 0;
	balance_ = 0;
	flip_ = -1;
	commit_pending();
	mode_ = SK8_MODE_GRIND;
	trick_ = grind_name_;
	emit(SK8_EV_GRIND, grind_name_.c_str(), 0, grind_kind_, surface_);
	return true;
}

void Rider::leave_grind(V3 velocity, bool popped, bool off_end)
{
	add_trick(grind_name_, kGrindDefs[grind_kind_].points + unsigned(grind_time_ * p[GrindRate]));
	unslide_ = grind_kind_ == SK8_GRIND_BOARDSLIDE || grind_kind_ == SK8_GRIND_DARKSLIDE ||
		(grind_kind_ >= SK8_GRIND_NOSESLIDE && grind_kind_ <= SK8_GRIND_BLUNTSLIDE);
	emit(SK8_EV_GRIND_END, grind_name_.c_str());
	mode_ = SK8_MODE_AIR;
	vel_ = velocity;
	if (off_end) // the tail drags over the end as it tips off: no hop, some speed lost
		vel_ = vel_ * p[GrindOffKeep];
	else if (!popped)
		vel_ += V3{0, 0.6f, 0};
	up_ = kUp;
	rail_ = -1;
	grind_kind_ = 0;
	grind_tip_ = false;
	grind_cooldown_ = 0.35f;
	air_time_ = 0;
	spin_total_ = 0;
	trick_.clear();
}

void Rider::grind(World &w, const sk8_input &in, const Flick::Result &f)
{
	if (rail_ < 0 || size_t(rail_) >= w.rails.size()) {
		leave_grind(vel_, false);
		return;
	}
	// a plant: a stall on the edge, then back off it the way it came, up
	if (grind_kind_ == SK8_GRIND_FOOTPLANT || grind_kind_ == SK8_GRIND_HANDPLANT) {
		grind_time_ += kDt;
		plant_time_ += kDt;
		// held on a handplant: a foot (or both) off, or the body dropped / swung with the right
		// stick, held a moment (the one left is scored)
		if (grind_kind_ == SK8_GRIND_HANDPLANT && plant_hold_ == SK8_SLIDE) {
			int bits = plant_bits(in);
			plant_want_time_ = bits == plant_want_ ? plant_want_time_ + kDt : 0;
			plant_want_ = bits;
			if (bits != plant_pose_ && plant_want_time_ >= kPlantVariant) {
				add_trick(grind_name_, kGrindDefs[grind_kind_].points + unsigned(grind_time_ * p[GrindRate]));
				plant_pose_ = bits;
				grind_name_ = trick_ = plant_variant(bits);
				grind_time_ = 0;
			}
		}
		float stall = grind_kind_ == SK8_GRIND_HANDPLANT ? p[PlantStall] : 0.65f;
		if (f.trick >= 0 || plant_time_ > stall || (plant_time_ > 0.15f && !(in.buttons & plant_hold_))) {
			if (wall_off_ && f.trick < 0) {
				// on a ramp's lip: the body swings back over and it drops straight in, nose first,
				// onto the wall it came up (comes down along it, as a vert air does)
				leave_grind(kUp * -1.6f + vert_in_ * 0.3f, true);
				up_ = vert_in_;
				fwd_ = fakie_ ? kUp : kUp * -1.0f;
				vert_air_ = true;
				plant_air_ = true;
				Hit h;
				if (w.cast(pos_ + vert_in_ * 0.3f - kUp * 0.1f, vert_in_ * -0.6f, h))
					land(h);
				else
					pos_ += vert_in_ * 0.06f - kUp * 0.04f;
				return;
			} else {
				pos_ += kUp * 0.05f;
				leave_grind(plant_in_ * -1.2f + kUp * 3.3f, true);
			}
			plant_air_ = true;
		}
		return;
	}
	const auto &r = w.rails[size_t(rail_)];
	V3 tangent = unit(r[size_t(seg_) + 1] - r[size_t(seg_)]);
	V3 t = tangent * float(rail_dir_);
	if (f.trick >= 0) {
		const TrickDef &def = trick_table()[size_t(f.trick)];
		leave_grind(t * rail_speed_, true);
		pop(def, p[GrindPop]);
		if (!def.flat)
			start_flip(f.trick);
		return;
	}
	// a trigger held on the rail: a grab on it (as the port: which grab by where the board's weighted
	// and which hand), held a moment; the one left is scored
	{
		bool lt = in.buttons & SK8_GRAB_LEFT, rt = in.buttons & SK8_GRAB_RIGHT;
		if (switch_)
			std::swap(lt, rt);
		int hands = rt ? 2 : lt ? 1 : 0;
		grind_grab_time_ = hands == grind_grab_want_ ? grind_grab_time_ + kDt : 0;
		grind_grab_want_ = hands;
		if (hands != grind_grab_ && grind_grab_time_ >= kGrindTransfer) {
			add_trick(grind_name_, kGrindDefs[grind_kind_].points + unsigned(grind_time_ * p[GrindRate]));
			grind_grab_ = hands;
			grind_name_ = trick_ = grind_full_name();
			grind_time_ = 0;
		}
	}
	// the right stick moved and held a moment: over into that grind (the one left is scored), as
	// the port does mid-rail
	// (let go to the centre, it stays in the grind it's in; a crooked held on turns across into a
	// boardslide)
	bool held = in.flick[0] * in.flick[0] + in.flick[1] * in.flick[1] > 0.2f;
	if (grind_kind_ != SK8_GRIND_DARKSLIDE && held) {
		int want = grind_kind_for(grind_slide_, in.flick[0], in.flick[1], in.steer[1] < -0.6f);
		if (want == SK8_GRIND_CROOKED && grind_kind_ == SK8_GRIND_CROOKED && grind_time_ >= kCrookToBoard)
			want = SK8_GRIND_BOARDSLIDE;
		else if (want == SK8_GRIND_CROOKED && grind_kind_ == SK8_GRIND_BOARDSLIDE)
			want = SK8_GRIND_BOARDSLIDE;
		if (want != grind_kind_) {
			grind_transfer_ += kDt;
			if (grind_transfer_ >= kGrindTransfer) {
				add_trick(grind_name_, kGrindDefs[grind_kind_].points + unsigned(grind_time_ * p[GrindRate]));
				grind_kind_ = want;
				grind_name_ = grind_full_name();
				grind_time_ = 0;
				grind_transfer_ = 0;
				trick_ = grind_name_;
				emit(SK8_EV_GRIND, grind_name_.c_str(), 0, grind_kind_, surface_);
			}
		} else {
			grind_transfer_ = 0;
		}
	} else {
		grind_transfer_ = 0;
	}
	rail_speed_ += -p[Gravity] * t.y * kDt;
	rail_speed_ -= p[GrindDrag] * kDt;
	grind_time_ += kDt;
	if (p[GrindBalance] > 0) {
		balance_ += (noise() * p[GrindBalance] + balance_ * 0.8f - in.steer[0] * 2.2f) * kDt;
		if (std::fabs(balance_) > 1) {
			V3 off = unit(cross(kUp, t)) * (balance_ > 0 ? 1.5f : -1.5f);
			leave_grind(t * rail_speed_ + off, false);
			return;
		}
	}
	if (rail_speed_ < 0.35f) {
		leave_grind(t * rail_speed_, false);
		return;
	}
	// walk the polyline
	float d = rail_speed_ * kDt * float(rail_dir_);
	for (int guard = 0; guard < 64; guard++) {
		float L = w.segment_length(rail_, seg_);
		float nu = seg_u_ + d;
		if (nu > L) {
			if (size_t(seg_) + 2 < r.size()) {
				d = nu - L;
				seg_++;
				seg_u_ = 0;
				continue;
			}
			pos_ = r.back();
			leave_grind(t * rail_speed_, false, true);
			return;
		}
		if (nu < 0) {
			if (seg_ > 0) {
				d = nu;
				seg_--;
				seg_u_ = w.segment_length(rail_, seg_);
				continue;
			}
			pos_ = r.front();
			leave_grind(t * rail_speed_, false, true);
			return;
		}
		seg_u_ = nu;
		break;
	}
	V3 a = r[size_t(seg_)], b = r[size_t(seg_) + 1];
	V3 tan = unit(b - a);
	pos_ = a + tan * seg_u_;
	float to_end = rail_dir_ > 0 ? (size_t(seg_) + 2 == r.size() ? w.segment_length(rail_, seg_) - seg_u_ : 1e9f)
	                             : (seg_ == 0 ? seg_u_ : 1e9f);
	grind_tip_ = grind_kind_ == SK8_GRIND_50_50 && to_end < 0.3f;
	vel_ = tan * (rail_speed_ * float(rail_dir_));
	trick_ = grind_name_;
}

// ---------------------------------------------------------------- bail

void Rider::bail(const char *reason)
{
	if (mode_ == SK8_MODE_BAIL)
		return;
	if (mode_ == SK8_MODE_GRIND)
		emit(SK8_EV_GRIND_END, grind_name_.c_str());
	lose_combo();
	// thrown forward unless the board shot out ahead (nose up) or it bounced off a wall
	V3 travel = fakie_ ? -fwd_ : fwd_;
	bool slipped = std::strcmp(reason, "Wall") == 0 ||
		(std::strcmp(reason, "Bad angle") == 0 && dot(up_, travel) < -0.05f);
	bail_dir_ = slipped ? -1 : 1;
	bail_reason_ = reason;
	mode_ = SK8_MODE_BAIL;
	vert_air_ = false;
	bail_time_ = 0;
	bail_stage_ = SK8_BAIL_FLAIL;
	bail_stage_time_ = 0;
	skid_time_ = 0;
	flip_ = -1;
	rail_ = -1;
	grind_kind_ = 0;
	manual_dir_ = 0;
	trick_.clear();
	vel_ *= slipped ? 0.55f : 0.92f;
	emit(SK8_EV_BAIL, reason, 0, 0, surface_);
}

void Rider::bailed(World &w)
{
	bail_time_ += kDt;
	bail_stage_time_ += kDt;
	auto stage = [&](int st) {
		bail_stage_ = st;
		bail_stage_time_ = 0;
	};
	// the body: thrown, then sliding along the ground to a stop
	vel_ += V3{0, -p[Gravity], 0} * kDt;
	V3 delta = vel_ * kDt;
	Hit h;
	bool ground = false;
	if (len(delta) > 1e-6f && w.cast(pos_ + V3{0, 0.05f, 0}, delta + V3{0, -0.05f, 0}, h)) {
		pos_ = h.point;
		vel_ = flatten(vel_, h.normal);
		up_ = h.normal;
		ground = true;
	} else {
		pos_ += delta;
	}
	if (ground) {
		float sp = len(vel_), dec = p[BailSlide] * kDt;
		vel_ = sp > dec ? vel_ * ((sp - dec) / sp) : V3{};
	}
	bool respawns = !(flags & SK8_FLAG_NO_RESPAWN);
	float getup_at = p[BailTime] - p[Rise];
	switch (bail_stage_) {
	case SK8_BAIL_FLAIL:
		if (ground && bail_stage_time_ > 0.1f)
			stage(SK8_BAIL_TUMBLE);
		break;
	case SK8_BAIL_TUMBLE:
		if (bail_stage_time_ > 0.45f && len(vel_) < 0.8f)
			stage(SK8_BAIL_LIE);
		break;
	case SK8_BAIL_LIE:
		if (respawns && bail_time_ >= getup_at && bail_stage_time_ > 0.35f)
			stage(SK8_BAIL_GETUP);
		break;
	case SK8_BAIL_GETUP:
		vel_ = V3{};
		if (bail_stage_time_ >= p[Rise]) {
			// back on the board where the body came to rest (somewhere flat), else at the last safe spot
			Hit g;
			bool flat = bail_reason_ != "Out of bounds" && pos_.y > -50 &&
				w.cast(pos_ + V3{0, 0.5f, 0}, V3{0, -1.0f, 0}, g) && g.normal.y > 0.85f;
			V3 f = flatten(fwd_, kUp);
			if (flat && len(f) > 0.1f)
				place(g.point, std::atan2(f.x, f.z), 0);
			else
				place(safe_pos_, safe_facing_, 0);
			emit(SK8_EV_RESPAWN);
		}
		return;
	}
	// no ground to stop on (out of bounds), or a long time down: on to getting up anyway
	if (respawns && bail_stage_ != SK8_BAIL_GETUP && bail_time_ >= getup_at + 1.0f)
		stage(SK8_BAIL_GETUP);
}

// ---------------------------------------------------------------- output

void Rider::state(sk8_state &s) const
{
	std::memset(&s, 0, sizeof s);
	pos_.store(s.position);
	vel_.store(s.velocity);
	up_.store(s.up);
	s.speed = len(vel_);
	// onto and off a wall the board turns between level and the wall over a moment, not in a step
	V3 up = up_;
	if (mode_ == SK8_MODE_WALLRIDE)
		up = unit(kUp + (wall_n_ - kUp) * smooth01(wall_time_ / 0.1f), up_);
	else if (since_wall_ < 0.14f)
		up = unit(wall_n_ + (kUp - wall_n_) * smooth01(since_wall_ / 0.14f), up_);
	V3 right = unit(cross(up, fwd_), V3{1, 0, 0});
	V3 fwd = unit(cross(right, up), fwd_);
	Q stance = from_basis(right, up, fwd);
	Q board = stance;
	if (mode_ == SK8_MODE_WALLRIDE || since_wall_ < 0.14f) {
		// the board flat on the wall; the rider leans out from it only part way
		V3 lean_up = unit(kUp + up * 0.75f, up);
		V3 r2 = unit(cross(lean_up, fwd_), right);
		stance = from_basis(r2, lean_up, unit(cross(r2, lean_up), fwd));
	}
	stance.store(s.rider_rot);

	if (flip_ >= 0) {
		const TrickDef &def = trick_table()[size_t(flip_)];
		float e = flip_frac();
		// switch is the other stance: the board flips and shoves the other way round
		float m = flip_mult_, g = switch_ ? -1.0f : 1.0f;
		// an underflip: the roll out to where it turned back, then back past level and round
		float roll = under_a0_ >= 0 ? (flip_angle_ <= under_a0_ ? flip_angle_ : 2 * under_a0_ - flip_angle_) / flip_base_turns_
								   : m * e;
		board = board * Q::axis_angle({0, 1, 0}, -def.shove * g * m * 2 * kPi * e) *
			Q::axis_angle({0, 0, 1}, def.roll * g * roll * 2 * kPi) * Q::axis_angle({1, 0, 0}, -def.wrap * m * 2 * kPi * e);
	}
	else if (settle_trick_ >= 0) { // the flip's last bit of roll / wrap (the shove-it's yaw is symmetric)
		const TrickDef &def = trick_table()[size_t(settle_trick_)];
		float e = std::min(settle_e_, 1.0f), m = settle_mult_, g = switch_ ? -1.0f : 1.0f;
		board = board * Q::axis_angle({0, 0, 1}, def.roll * g * m * 2 * kPi * e) *
			Q::axis_angle({1, 0, 0}, -def.wrap * m * 2 * kPi * e);
	}
	if (mode_ == SK8_MODE_AIR && (adjust_[0] != 0 || adjust_[1] != 0)) // a board adjust: nose up / swung
		board = board * Q::axis_angle({0, 1, 0}, -adjust_[0] * kAdjustYaw) * Q::axis_angle({1, 0, 0}, -adjust_[1] * kAdjustPitch);
	if (slide_amount_ > 0)
		board = board * Q::axis_angle({0, 1, 0}, (lean_ >= 0 ? -1.0f : 1.0f) * slide_amount_ * 1.4f);
	if (mode_ == SK8_MODE_MANUAL && manual_style_ == 1)
		board = board * Q::axis_angle({0, 0, 1}, (switch_ ? -kPi : kPi) / 2); // primo: on its side, heel edge up
	else if (mode_ == SK8_MODE_MANUAL && manual_style_ == 2)
		board = board * Q::axis_angle({0, 0, 1}, kPi) * Q::axis_angle({1, 0, 0}, 0.38f); // casper: upside down, tail up
	else if (mode_ == SK8_MODE_MANUAL)
		board = board * Q::axis_angle({1, 0, 0}, -0.13f * float(manual_dir_));
	if (mode_ == SK8_MODE_AIR && varial_ != 0)
		board = board * Q::axis_angle({0, 1, 0}, -varial_); // the board stays while the body turns
	if (mode_ == SK8_MODE_GRIND && grind_kind_ > 0) {
		// the board sits on the rail the way this grind holds it, the rail under the part it
		// rides on (local x right = heel edge, y up, z the nose)
		const GrindDef &g = kGrindDefs[grind_tip_ ? SK8_GRIND_5_0 : grind_kind_];
		float side = switch_ ? -1.0f : 1.0f; // the heel edge is on the other side in switch
		board = board * Q::axis_angle({0, 1, 0}, -g.yaw * side * kDeg) * Q::axis_angle({1, 0, 0}, -g.pitch * kDeg) *
			Q::axis_angle({0, 0, 1}, g.roll * side * kDeg);
		(pos_ - fwd * g.along).store(s.position);
	}
	board.store(s.board_rot);

	s.mode = mode_;
	s.fakie = fakie_;
	s.crouch = crouch_;
	s.lean = lean_;
	s.push_phase = push_time_ >= push_lead_ ? clampf((push_time_ - push_lead_) / std::max(p[PushPeriod], 0.01f), 0, 1) : -1;
	s.push_start = push_time_ >= 0 && push_time_ < push_lead_ ? push_time_ / push_lead_ : -1;
	s.push_mongo = push_mongo_;
	s.foot_off = foot_off_;
	s.revert = revert_time_ >= 0 ? clampf(revert_time_ / 0.34f, 0, 1) : -1;
	s.grab = ground_mode(mode_) ? ground_grab_ : grab_;
	s.air_feet = mode_ == SK8_MODE_AIR ? air_feet_ : 0;
	s.grind_grab = mode_ == SK8_MODE_GRIND ? grind_grab_ : 0;
	s.grab_style = grab_style_;
	// (still settling after landing short: the board finishing its turn under the feet)
	s.flip_progress = flip_ >= 0 ? flip_frac() : settle_trick_ >= 0 ? std::min(settle_e_, 1.0f) : -1;
	s.balance = clampf(balance_, -1, 1);
	s.air_time = air_time_;
	s.bail_time = mode_ == SK8_MODE_BAIL ? bail_time_ : -1;
	s.bail_stage = mode_ == SK8_MODE_BAIL ? bail_stage_ : 0;
	s.bail_dir = bail_dir_;
	s.bail_stage_time = mode_ == SK8_MODE_BAIL ? bail_stage_time_ : 0;
	s.plant_wind = plant_wind_ >= 0 ? clampf(plant_wind_ / kPlantWind, 0, 1) : -1;
	s.plant_pose = mode_ == SK8_MODE_GRIND && grind_kind_ == SK8_GRIND_HANDPLANT ? plant_pose_ : 0;
	s.grind_kind = grind_tip_ ? SK8_GRIND_5_0 : grind_kind_;
	s.combo_points = combo_points_;
	s.combo_count = unsigned(combo_names_.size());
	s.score = score_;
	s.best_line = unsigned(best_line_);
	s.best_air = best_air_;
	s.best_grind = best_grind_;
	s.best_manual = best_manual_;
	s.best_tricks = unsigned(best_tricks_);
	s.manual_dir = mode_ == SK8_MODE_MANUAL ? manual_dir_ : 0;
	s.manual_style = mode_ == SK8_MODE_MANUAL ? manual_style_ : 0;
	s.on_foot = mode_ == SK8_MODE_WALK || foot_air_;
	s.stance = switch_ ? 1 : 0;
	s.stance_change = switching_ >= 0 ? clampf(switching_, 0, 1) : -1;
}

} // namespace sk8

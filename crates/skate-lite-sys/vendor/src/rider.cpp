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
	{"Christ Air", 340},  {"Method", 240},   {"Rocket Air", 300},
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
	if (up)
		return side ? (x < 0 ? SK8_GRIND_CROOKED : SK8_GRIND_OVERCROOK) : SK8_GRIND_NOSEGRIND;
	if (down)
		return side ? (x < 0 ? SK8_GRIND_SMITH : SK8_GRIND_FEEBLE) : SK8_GRIND_5_0;
	return crouched ? SK8_GRIND_5_0 : SK8_GRIND_50_50;
}

// hands: 1 front (left trigger), 2 back (right trigger), 3 both. Stick y up is toward the nose.
int grab_style_for(int hands, float x, float y)
{
	int dir = 0; // 0 centre, 1 nose, 2 tail, 3 sideways
	if (x * x + y * y > 0.25f)
		dir = std::fabs(y) >= std::fabs(x) ? (y > 0 ? 1 : 2) : 3;
	// sideways the other way (stick left): one more style per hand
	if (dir == 3 && x < 0)
		return hands == 2 ? SK8_GRAB_CHRIST : hands == 1 ? SK8_GRAB_METHOD : SK8_GRAB_ROCKET;
	int base = hands == 2 ? SK8_GRAB_INDY : hands == 1 ? SK8_GRAB_MELON : SK8_GRAB_DOUBLE;
	return base + dir;
}
} // namespace


namespace {

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
	flick_.reset();
	crouch_ = crouch_time_ = 0;
	flip_ = -1;
	spin_speed_ = spin_total_ = 0;
	air_time_ = 0;
	bail_time_ = -1;
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
	combo_names_.clear();
	combo_points_ = 0;
	bank_timer_ = 0;
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
	last_buttons_ = in.buttons;
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
	}
	float target = mode_ == SK8_MODE_SLIDE ? 1.0f : 0.0f;
	slide_amount_ = approach(slide_amount_, target, 6 * kDt);
}

// ---------------------------------------------------------------- ground

void Rider::ground(World &w, const sk8_input &in, const Flick::Result &f)
{
	if (f.trick >= 0) {
		if (mode_ == SK8_MODE_MANUAL) {
			add_trick(manual_dir_ > 0 ? "Manual" : "Nose Manual",
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
		const TrickDef &def = trick_table()[size_t(trick)];
		pop(def, share);
		if (flip_ < 0 && !def.flat)
			start_flip(trick);
		else if (def.flat)
			pending_.push_back({def.name, def.points});
		return;
	}

	float speed = len(vel_);
	float along = dot(vel_, fwd_);
	if (along < -0.3f)
		fakie_ = true;
	else if (along > 0.3f)
		fakie_ = false;
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
	if (mode_ == SK8_MODE_RIDE && want_manual && !manual_lock_ && speed > 0.6f) {
		mode_ = SK8_MODE_MANUAL;
		manual_dir_ = stick != 0 ? stick : (in.buttons & SK8_MANUAL) ? 1 : -1;
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
		trick_ = manual_dir_ > 0 ? "Manual" : "Nose Manual";
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

	// powerslide: hold the slide button and steer
	if (mode_ == SK8_MODE_RIDE && (in.buttons & SK8_SLIDE) && std::fabs(steer) > 0.4f && speed > 1.8f)
		mode_ = SK8_MODE_SLIDE;
	else if (mode_ == SK8_MODE_SLIDE && (!(in.buttons & SK8_SLIDE) || speed < 0.6f))
		mode_ = SK8_MODE_RIDE;

	// carve
	float rate = p[Carve] * steer * clampf(speed / 2.5f, 0.35f, 1.0f);
	if (mode_ == SK8_MODE_SLIDE)
		rate *= 0.5f;
	fwd_ = unit(flatten(rotate(fwd_, up_, -rate * kDt), up_), fwd_);
	// The wheels carry the motion round with the board (a carve keeps its speed).
	if (mode_ != SK8_MODE_SLIDE)
		vel_ = rotate(vel_, up_, -rate * kDt);

	// gravity along the surface
	vel_ += flatten(V3{0, -p[Gravity], 0}, up_) * kDt;

	// push: a step off the board (first stroke only), then strokes; the speed comes on while the
	// foot is on the ground driving back, not the moment the button goes down
	if (push_cooldown_ > 0)
		push_cooldown_ -= kDt;
	if (push_time_ >= 0) {
		float period = std::max(p[PushPeriod], 0.01f);
		float t0 = push_lead_ + SK8_PUSH_PLANT * period, t1 = push_lead_ + SK8_PUSH_RELEASE * period;
		float drive = std::min(push_time_ + kDt, t1) - std::max(push_time_, t0);
		if (mode_ == SK8_MODE_RIDE && drive > 0 && std::fabs(dot(vel_, travel)) < p[PushTop])
			vel_ += travel * (p[PushKick] * drive / (t1 - t0));
		push_time_ += kDt;
		if (push_time_ > push_lead_ + period || mode_ != SK8_MODE_RIDE) {
			push_time_ = -1;
			since_stroke_ = 0;
		}
	} else {
		since_stroke_ += kDt;
	}
	if ((in.buttons & (SK8_PUSH | SK8_PUSH_MONGO)) && mode_ == SK8_MODE_RIDE && push_cooldown_ <= 0 && push_time_ < 0 &&
		std::fabs(dot(vel_, travel)) < p[PushTop]) {
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
	if (in.buttons & SK8_BRAKE)
		dec += p[Brake] * kDt;
	if (mode_ == SK8_MODE_SLIDE)
		dec += p[SlideDrag] * kDt;
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
			vel_ += up_ * 0.4f;
		}
		return;
	}
	// rolled off an edge
	if (mode_ == SK8_MODE_MANUAL) {
		add_trick(manual_dir_ > 0 ? "Manual" : "Nose Manual", 50 + unsigned(manual_time_ * p[ManualRate]));
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
	vel_ += up_ * v;
	pos_ += up_ * 0.02f;
	mode_ = SK8_MODE_AIR;
	air_time_ = 0;
	spin_total_ = 0;
	crouch_ = crouch_time_ = 0;
	grab_scored_ = false;
	grab_time_ = 0;
	emit(SK8_EV_POP, def.name, 0, 0, surface_);
}

void Rider::start_flip(int trick)
{
	const TrickDef &def = trick_table()[size_t(trick)];
	flip_ = trick;
	flip_time_ = 0;
	float turns = std::max(std::fabs(def.roll), std::fabs(def.shove) * 1.2f);
	flip_duration_ = p[FlipBase] + p[FlipPerRev] * turns;
	pending_.push_back({def.name, def.points});
	trick_ = def.name;
}

void Rider::air(World &w, const sk8_input &in, const Flick::Result &f)
{
	air_time_ += kDt;
	vel_ += V3{0, -p[Gravity], 0} * kDt;
	float sp = len(vel_);
	if (sp > 0)
		vel_ *= std::max(0.0f, 1 - p[AirDrag] * sp * kDt);

	// spin about the stance axis
	float target = -clampf(in.steer[0], -1, 1) * p[SpinRate];
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
	if (f.trick >= 0 && flip_ < 0 && p[AirFlick] >= 0.5f && !trick_table()[size_t(f.trick)].flat)
		start_flip(f.trick);
	if (flip_ >= 0)
		flip_time_ += kDt;

	// grabs: the hands come from the triggers, the style from where the left stick points when the
	// grab starts (it keeps spinning the board after that)
	int hands = ((in.buttons & SK8_GRAB_LEFT) ? 1 : 0) | ((in.buttons & SK8_GRAB_RIGHT) ? 2 : 0);
	if (hands && hands != grab_)
		grab_style_ = grab_style_for(hands, in.steer[0], in.steer[1]);
	grab_ = hands;
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

	if (try_grind(w))
		return;

	V3 delta = vel_ * kDt;
	float L = len(delta);
	Hit h;
	if (L > 1e-6f && w.cast(pos_ + up_ * 0.1f, delta + delta * (0.1f / L), h)) {
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
	V3 n = h.normal;
	float tilt = angle_between(up_, n);
	bool flip_done = flip_ < 0 || flip_time_ / flip_duration_ >= p[FlipLenience];
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
	const char *bad = nullptr;
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
	up_ = n;
	fwd_ = fflat;
	fakie_ = fakie;
	// line the wheels up with the motion
	V3 travel = fakie ? -fwd_ : fwd_;
	float along = dot(vflat, travel);
	vel_ = travel * along + (vflat - travel * along) * 0.15f;
	bool clean = tilt < p[LandClean] * kDeg && side < 20 * kDeg;

	int halves = int(std::lround(std::fabs(spin_total_) / kPi));
	if (halves > 0) {
		char name[24];
		std::snprintf(name, sizeof name, "%d Spin", halves * 180);
		pending_.push_back({name, unsigned(halves * 110)});
	}
	// the slide button held into the landing: a revert (the board spun round on its wheels)
	if (last_buttons_ & SK8_SLIDE) {
		revert_time_ = 0;
		revert_dir_ = lean_ < -0.1f ? -1.0f : 1.0f;
		pending_.push_back({"Revert", 90});
	}
	commit_pending();
	if (!clean)
		emit(SK8_EV_LAND, "sketchy", 0, 1, h.surface);
	else
		emit(SK8_EV_LAND, "clean", 0, 2, h.surface);
	flip_ = -1;
	spin_total_ = spin_speed_ = 0;
	air_time_ = 0;
	grab_ = 0;
	grab_style_ = 0;
	mode_ = SK8_MODE_RIDE;
	trick_.clear();
}

// ---------------------------------------------------------------- grind

bool Rider::try_grind(World &w)
{
	if (grind_cooldown_ > 0 || w.rails.empty() || vel_.y > 2.5f)
		return false;
	RailSpot s = w.nearest_rail(pos_, p[GrindCatch]);
	if (s.rail < 0 || pos_.y - s.point.y < -0.15f)
		return false;
	float vt = dot(vel_, s.tangent);
	if (std::fabs(vt) < 0.8f)
		return false;
	if (flip_ >= 0 && flip_time_ / flip_duration_ < p[FlipLenience] && !never_bail()) {
		bail("Under-flipped");
		return true;
	}
	rail_ = s.rail;
	seg_ = s.seg;
	seg_u_ = s.u;
	rail_dir_ = vt >= 0 ? 1 : -1;
	rail_speed_ = std::fabs(vt) * 0.95f;
	pos_ = s.point;
	up_ = kUp;
	fwd_ = unit(flatten(fwd_, kUp), fwd_);
	V3 t = s.tangent * float(rail_dir_);
	float a = angle_between(fwd_, unit(flatten(t, kUp), t));
	bool backwards = a > kPi / 2;
	if (backwards)
		a = kPi - a;
	bool slide = a > 55 * kDeg;
	grind_kind_ = grind_kind_for(slide, last_steer_[0], last_steer_[1], crouch_ > 0.5f);
	grind_name_ = kGrindDefs[grind_kind_].name;
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

void Rider::leave_grind(V3 velocity, bool popped)
{
	add_trick(grind_name_, kGrindDefs[grind_kind_].points + unsigned(grind_time_ * p[GrindRate]));
	unslide_ = grind_kind_ == SK8_GRIND_BOARDSLIDE || grind_kind_ >= SK8_GRIND_NOSESLIDE;
	emit(SK8_EV_GRIND_END, grind_name_.c_str());
	mode_ = SK8_MODE_AIR;
	vel_ = velocity;
	if (!popped)
		vel_ += V3{0, 0.6f, 0};
	up_ = kUp;
	rail_ = -1;
	grind_kind_ = 0;
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
			leave_grind(t * rail_speed_, false);
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
			leave_grind(t * rail_speed_, false);
			return;
		}
		seg_u_ = nu;
		break;
	}
	V3 a = r[size_t(seg_)], b = r[size_t(seg_) + 1];
	V3 tan = unit(b - a);
	pos_ = a + tan * seg_u_;
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
	mode_ = SK8_MODE_BAIL;
	bail_time_ = 0;
	flip_ = -1;
	rail_ = -1;
	grind_kind_ = 0;
	manual_dir_ = 0;
	trick_.clear();
	vel_ *= 0.6f;
	emit(SK8_EV_BAIL, reason, 0, 0, surface_);
}

void Rider::bailed(World &w)
{
	bail_time_ += kDt;
	vel_ += V3{0, -p[Gravity], 0} * kDt;
	V3 delta = vel_ * kDt;
	Hit h;
	if (len(delta) > 1e-6f && w.cast(pos_ + V3{0, 0.05f, 0}, delta, h)) {
		pos_ = h.point;
		vel_ = flatten(vel_, h.normal) * 0.6f;
		up_ = h.normal;
	} else {
		pos_ += delta;
	}
	vel_ *= std::max(0.0f, 1 - 1.5f * kDt);
	if (bail_time_ >= p[BailTime] && !(flags & SK8_FLAG_NO_RESPAWN)) {
		place(safe_pos_, safe_facing_, 0);
		emit(SK8_EV_RESPAWN);
	}
}

// ---------------------------------------------------------------- output

void Rider::state(sk8_state &s) const
{
	std::memset(&s, 0, sizeof s);
	pos_.store(s.position);
	vel_.store(s.velocity);
	up_.store(s.up);
	s.speed = len(vel_);
	V3 right = unit(cross(up_, fwd_), V3{1, 0, 0});
	V3 fwd = unit(cross(right, up_), fwd_);
	Q stance = from_basis(right, up_, fwd);
	stance.store(s.rider_rot);

	Q board = stance;
	if (flip_ >= 0) {
		const TrickDef &def = trick_table()[size_t(flip_)];
		float e = smooth01(flip_time_ / flip_duration_);
		board = board * Q::axis_angle({0, 1, 0}, -def.shove * 2 * kPi * e) *
			Q::axis_angle({0, 0, 1}, def.roll * 2 * kPi * e);
	}
	if (slide_amount_ > 0)
		board = board * Q::axis_angle({0, 1, 0}, (lean_ >= 0 ? -1.0f : 1.0f) * slide_amount_ * 1.4f);
	if (mode_ == SK8_MODE_MANUAL)
		board = board * Q::axis_angle({1, 0, 0}, -0.21f * float(manual_dir_));
	if (mode_ == SK8_MODE_GRIND && grind_kind_ > 0) {
		// the board sits on the rail the way this grind holds it, the rail under the part it
		// rides on (local x right = heel edge, y up, z the nose)
		const GrindDef &g = kGrindDefs[grind_kind_];
		board = board * Q::axis_angle({0, 1, 0}, -g.yaw * kDeg) * Q::axis_angle({1, 0, 0}, -g.pitch * kDeg) *
			Q::axis_angle({0, 0, 1}, g.roll * kDeg);
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
	s.grab = grab_;
	s.grab_style = grab_style_;
	s.flip_progress = flip_ >= 0 ? clampf(flip_time_ / flip_duration_, 0, 1) : -1;
	s.balance = clampf(balance_, -1, 1);
	s.air_time = air_time_;
	s.bail_time = mode_ == SK8_MODE_BAIL ? bail_time_ : -1;
	s.grind_kind = grind_kind_;
	s.combo_points = combo_points_;
	s.combo_count = unsigned(combo_names_.size());
	s.score = score_;
	s.manual_dir = mode_ == SK8_MODE_MANUAL ? manual_dir_ : 0;
}

} // namespace sk8

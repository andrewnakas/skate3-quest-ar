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
	// (named as the port names them: the plain right-trigger grab is a backside grab, the left a
	// frontside one)
	{"Backside Grab", 150}, {"Crail Grab", 220}, {"Tail Grab", 180}, {"Stalefish", 240},
	{"Frontside Grab", 150}, {"Nose Grab", 180}, {"Tuck Knee", 200}, {"Judo", 260},
	{"Double Grab", 200}, {"Superman", 320}, {"No Foot", 300},  {"Stiffy", 260},
	{"Christ Air", 340},  {"Method", 240},   {"Rocket Air", 300}, {"Seatbelt Grab", 220},
	{"Mute Grab", 180},  {"Japan", 260},      {"Nosebone", 240},  {"Tailbone", 240},
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

constexpr float kGrindRim = 0.8f; // the right stick this far out on a rail: right out (part way, 0.6, isn't)

int grind_kind_for(bool slide, float x, float y, bool crouched)
{
	// (as the port, probed on a rail: a 5-0 from just over half way down, a nosegrind from two thirds
	// up, a crooked from half way to the side; short of that it's the 50-50 / boardslide)
	bool side = std::fabs(x) > 0.52f, up = y > 0.66f, down = y < -0.52f;
	if (slide) {
		if (up)
			return side ? SK8_GRIND_NOSEBLUNT : SK8_GRIND_NOSESLIDE;
		if (down)
			return side ? SK8_GRIND_BLUNTSLIDE : SK8_GRIND_TAILSLIDE;
		return side ? SK8_GRIND_LIPSLIDE : SK8_GRIND_BOARDSLIDE;
	}
	// The right stick held into the catch (as the port, swept round on a rail every 15 deg, right out
	// and part way; the left stick held back makes no difference): right out, round from up -
	// nosegrind, crooked, boardslide, salad, 5-0, smith, lipslide, overcrook; part way the board
	// turns less - a willy (by way of a crooked) to the board side, a smith (by way of a 5-0) to the
	// lip side, a feeble (by way of a salad) down and to the board side.
	(void)crouched;
	float r = std::sqrt(x * x + y * y);
	if (r < 0.45f)
		return SK8_GRIND_50_50;
	float a = std::atan2(x, y) * 57.2957795f; // 0 up, 90 right
	if (a < 0)
		a += 360;
	if (r >= kGrindRim) {
		if (a < 22.4f || a >= 337.5f) return SK8_GRIND_NOSEGRIND;
		if (a < 67.5f) return SK8_GRIND_CROOKED; // (past ~55 it goes on round into a boardslide)
		if (a < 127.5f) return SK8_GRIND_BOARDSLIDE;
		if (a < 142.5f) return SK8_GRIND_SALAD;
		if (a < 217.5f) return SK8_GRIND_5_0;
		if (a < 232.5f) return SK8_GRIND_SMITH;
		if (a < 307.5f) return SK8_GRIND_LIPSLIDE;
		return SK8_GRIND_OVERCROOK;
	}
	if (a >= 37.65f && a < 52.5f) return SK8_GRIND_NOSEGRIND;
	if (a >= 52.5f && a < 112.5f) return SK8_GRIND_WILLY;
	if (a >= 112.5f && a < 127.5f) return SK8_GRIND_FEEBLE;
	if (a >= 127.5f && a < 217.5f) return SK8_GRIND_5_0;
	if (a >= 217.5f && a < 307.5f) return SK8_GRIND_SMITH;
	if (a >= 307.5f && a < 322.5f) return SK8_GRIND_NOSEGRIND;
	return SK8_GRIND_50_50;
}

// A grind as which truck is down (-1 the back, 0 both, 1 the front) and how far the board is
// turned off the rail (+ toward the board side, - the lip side; one step a one-truck grind, the
// last step across into a slide) - how the port's grinds go over into one another: the right stick
// tips it onto a truck and, right out to a side, turns it round step by step.
static int grind_of_pose(int tip, int rot)
{
	if (rot >= 3) return SK8_GRIND_BOARDSLIDE;
	if (rot <= -2) return SK8_GRIND_LIPSLIDE;
	if (rot == 2) return tip < 0 ? SK8_GRIND_FEEBLE : SK8_GRIND_WILLY;
	if (rot == 1) return tip < 0 ? SK8_GRIND_SALAD : SK8_GRIND_CROOKED;
	if (rot == -1) return tip > 0 ? SK8_GRIND_OVERCROOK : SK8_GRIND_SMITH;
	return tip > 0 ? SK8_GRIND_NOSEGRIND : tip < 0 ? SK8_GRIND_5_0 : SK8_GRIND_50_50;
}

// The pose a grind is in (false for the ones that aren't on that ladder: slides caught across, plants).
static bool pose_of_grind(int kind, int &tip, int &rot)
{
	switch (kind) {
	case SK8_GRIND_50_50: tip = 0, rot = 0; return true;
	case SK8_GRIND_NOSEGRIND: tip = 1, rot = 0; return true;
	case SK8_GRIND_5_0: tip = -1, rot = 0; return true;
	case SK8_GRIND_CROOKED: case SK8_GRIND_SUSKI: tip = 1, rot = 1; return true;
	case SK8_GRIND_SALAD: tip = -1, rot = 1; return true;
	case SK8_GRIND_WILLY: tip = 1, rot = 2; return true;
	case SK8_GRIND_FEEBLE: tip = -1, rot = 2; return true;
	case SK8_GRIND_BOARDSLIDE: tip = 0, rot = 3; return true;
	case SK8_GRIND_OVERCROOK: tip = 1, rot = -1; return true;
	case SK8_GRIND_SMITH: tip = -1, rot = -1; return true;
	case SK8_GRIND_LIPSLIDE: tip = 0, rot = -2; return true;
	default: return false;
	}
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
constexpr float kVertLipUp = 0.1f; // the wall's up below this at the lip: a vert air (a 68 deg quarter pipe carries over onto its deck, as the port)
constexpr float kLipCatch = 0.16f; // m: an air off a lip comes down this close to its coping to grind it
constexpr float kLipAlongKeep = 0.33f; // of the way along the coping kept, popped off the lip
constexpr float kLipInSpeed = 0.45f;   // m/s back in over the ramp, popped off the lip
constexpr float kLipVertLift = 0.7f; // of a lip pop's lift, turned back in as a vert air
constexpr float kVertPull = 0.8f;   // of gravity on a vert air
constexpr float kVertIn = 0.15f; // m: a vert air comes back down this far inside the lip
constexpr float kSettleFlip = 0.45f;
constexpr float kTipTime = 0.166f;  // s the right stick holds up or down before it's over onto that truck
constexpr float kTurnFirst = 0.282f; // s held right out to a side before the board first turns round
constexpr float kTurnStep = 0.134f;  // s each step round after that
constexpr float kFakieTurnPush = 0.6f; // m/s rolling fakie under this, a push goes the board's way
constexpr float kPushTap = 1.045f;   // m/s a tapped push gives (held, up to a full one)
constexpr float kPushFull = 9.0f;
constexpr float kPushRepeat = 0.61f; // a held-on stroke's share   // steps more the button's held for a full one
constexpr float kLeanTip = 0.034f;   // s the left stick leans it off both trucks onto one (at once)
constexpr float kPlantWind = 0.605f;
constexpr float kFootOffHold = 0.1f;
constexpr double kLineMultiplier = 1.5; // a line's points banked at this (the port's never went higher)
constexpr float kFingerFlipSpeed = 2.3f;
constexpr float kSpinUp = 0.07f;      // s into the air before the stick spins it (not off the pop)
constexpr float kSpinKick = 3.3f;     // rad/s it's quickly up to; then it gathers speed
constexpr float kSpinKickEase = 64.0f; // 1/s: how quickly it's up to that
constexpr float kSpinAccel = 37.0f;   // rad/s^2 on from there to the full rate
// An ollie off flat ground isn't a plain arc (as the port's board, measured and fitted): the board
// comes up off the tail over a few ticks, carries on up, is pulled up short near the top, hangs there
// a moment, then is pushed back down hard. A quick flick's pops less, slows more like an arc, and
// hangs the longer.
constexpr float kOllieFlatTap = 0.685f, kOllieFlatFull = 0.76f; // its pop, of the pop's (quick / loaded)
constexpr float kOllieKickNow = 0.44f;  // share of the pop at once, the rest over a few ticks
constexpr float kOllieKickTicks = 3;
constexpr float kOllieCarry = 0.1f;     // s at the pop's speed before it starts to slow
constexpr float kOllieRise = 0.29f;     // s of the climb at most
constexpr float kOllieRiseFull = 0.35f, kOllieRiseTap = 0.78f; // the climb's pull of gravity
constexpr float kOllieBrake = 3.4f;     // times gravity, pulled up short
constexpr float kOllieHangVy = 0.4f;    // m/s up when it's hanging
constexpr float kOllieHang = 0.25f;     // s it hangs at the top
constexpr float kOllieHangTap = 0.16f;  // s longer, a quick flick's
constexpr float kRailHang = 0.1f;       // s it hangs, popped off a rail
constexpr float kOllieHangPull = 0.45f; // times gravity, hanging
constexpr float kOllieDrop = 3.0f;      // times gravity, pushed back down
constexpr float kOllieDropTo = 4.2f;    // m/s down, then falling as any air
constexpr float kPlantDropIn = 6.2f; // m/s down the wall dropping back in off a handplant (the port's fall from ~1.5 m over the coping)
constexpr float kWallUp = 0.97f;    // the surface's up below this: a ramp's face
constexpr float kWallYaw = 1.335f;   // rad/s the board turns after the way it's going across one
constexpr float kWallScrub = 6.0f;  // m/s^2 the wheels take off sideways scrubbing across one
constexpr float kWallDrag = 0.3f;   // of a powerslide's drag, scrubbing across one
constexpr float kSlideAngle = 1.2f;  // rad (~69 deg) off the rail: a slide, not a grind
constexpr float kSpinCoast = 0.42f;  // s: a spin let go slows over about this long
constexpr float kAngledGrind = 0.44f; // rad (~25 deg): a board landing this far off the rail is a turned grind (19 off was still a nosegrind, recorded)
constexpr float kAssistReach = 0.44f; // m: how far to the side a rail pulls an air onto it
constexpr float kAssistAbove = 0.62f; // m: a rail this far above the board still pulls it, rising to it
constexpr float kAssistSpeed = 0.47f; // m/s at most it drifts across toward the rail (as the port: aimed at it, not yanked)
constexpr float kCrossCatch = 1.7f;  // the catch distance's reach for a board coming across the rail
constexpr float kAssistAlong = 0.976f; // cos: going this near along the rail (~12 deg) for it to aim
constexpr float kBackfootHold = 0.1f, kBackfootSnap = 0.14f; // s held up, s to get down from there
constexpr float kKickturnTop = 4.6f;  // m/s: slower than this, the stick hard over is a kickturn
constexpr float kKickturnRate = 3.1f; // rad/s once it's turning (it eases in over kKickturnEase)
constexpr float kKickturnEase = 0.45f;
constexpr float kBigAir = 1.4f, kLongAir = 9.5f; // m: an air this high (above its lower end) / long scores  // s a foot held off in the air before it counts
constexpr float kAdjustYaw = 1.1f, kAdjustPitch = 0.38f; // rad: a board adjust at full stick
constexpr float kCatchEase = 0.12f;  // s: drawn, the body and board ease round to a board-adjusted catch
constexpr float kAdjustCatch = 1.57f; // rad: how far a full sideways adjust turns the board for a rail
constexpr float kUnderTurn = 0.34f;   // turns an underflip goes its first way before it turns back
constexpr float kUnderWindow = 0.11f; // s into a flip the opposite flick still turns it into an underflip   // s: a boneless's wind-up, push pressed to the pop
constexpr float kPlantVariant = 0.15f; // s a handplant variant's buttons / stick hold before it changes
constexpr float kGrindTransfer = 0.1525f; // s the right stick holds a new way before the grind changes
constexpr float kGrindSettle = 0.1525f;  // s into a grind before the stick can change it
constexpr int kCatchStick = 8;          // steps the stick held its way before a catch, to pick the grind // s for a whole flip's worth of settling (the last bit: a few ticks)  // s: a crooked landing scrubs its speed off over this
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

// Put the rider in the air, moving (test harnesses: an air from a recording, started at its takeoff).
void Rider::launch(V3 position, V3 velocity, float facing)
{
	place(position, facing, 0);
	ollie_ = 0;
	vel_ = velocity;
	mode_ = SK8_MODE_AIR;
	pop_y_ = position.y;
	launch_ = position;
	launch_vel_ = velocity;
	peak_ = position.y;
}

// Test harnesses: ride switch and / or fakie from here (the facing given is the way it's going).
void Rider::set_stance(bool switch_stance, bool fakie)
{
	switch_ = switch_stance;
	fakie_ = fakie;
	if (fakie)
		fwd_ = fwd_ * -1.0f;
	stance_pre_ = switch_ ? "Switch " : fakie_ ? "Fakie " : "";
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
	double total = double(combo_points_) * kLineMultiplier * double(p[ScoreScale]);
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

// ---------------------------------------------------------------- stick curves

// x on a measured curve of (in, out) points, rising from (0, 0); odd (the sign is kept).
template <size_t N> static float on_curve(const float (&curve)[N][2], float x)
{
	float a = std::fabs(x), y = curve[N - 1][1];
	for (size_t i = 1; i < N; i++)
		if (a <= curve[i][0]) {
			float t = (a - curve[i - 1][0]) / (curve[i][0] - curve[i - 1][0]);
			y = curve[i - 1][1] + (curve[i][1] - curve[i - 1][1]) * t;
			break;
		}
	return x < 0 ? -y : y;
}

// How hard the board turns for the (pad-read) left stick: barely at all part-way, coming on fast
// toward full. Measured on the port as the turn rate held at each stick deflection while rolling.
static const float kSteerCurve[][2] = {
	{0.0f, 0.0f}, {0.143f, 0.0019f}, {0.214f, 0.0079f}, {0.286f, 0.0268f}, {0.357f, 0.0587f},
	{0.429f, 0.1206f}, {0.5f, 0.2073f}, {0.571f, 0.3296f}, {0.643f, 0.4609f}, {0.714f, 0.6098f},
	{0.786f, 0.6655f}, {0.857f, 0.7508f}, {0.929f, 0.8797f}, {1.0f, 1.0f}};

// How hard an air spin goes for the (pad-read) left stick, measured on the port as how far it
// spins through an ollie with the stick held at each deflection (less the turn after landing):
// a little part-way, about half a turn from two thirds out, and the full turn only right out.
static const float kSpinCurve[][2] = {
	{0.0f, 0.0f}, {0.214f, 0.056f}, {0.357f, 0.127f}, {0.5f, 0.399f}, {0.643f, 0.5376f}, {0.786f, 0.597f},
	{0.929f, 0.602f}, {1.0f, 0.999f}};

// ---------------------------------------------------------------- board tilt

// The board's tilt in a manual and a 5-0 / nosegrind goes by how far the right stick is tipped, and
// eases there (as the port, measured: degrees for the stick's reach).
static const float kManualTilt[][2] = {{0.0f, 9.3f}, {0.35f, 9.3f}, {0.55f, 14.61f}, {0.85f, 22.3f}, {1.0f, 22.3f}};
constexpr float kManualTiltButton = 14.61f; // held with a button, no stick: the middle
constexpr float kTiltRate = 27.0f;         // degrees per second at most
constexpr float kTiltEase = 0.25f;         // s, settling at the end
// a 5-0 / nosegrind caught with the stick that far over, and where it settles held there
static const float kTipCatch[][2] = {{0.0f, 11.11f}, {0.55f, 11.11f}, {0.7f, 16.71f}, {1.0f, 23.8f}};
static const float kTipHold[][2] = {{0.0f, 10.51f}, {0.55f, 10.51f}, {0.7f, 16.0f}, {1.0f, 16.0f}};
static const float kNoseCatch[][2] = {{0.0f, 12.8f}, {0.7f, 12.8f}, {1.0f, 18.5f}};
static const float kNoseHold[][2] = {{0.0f, 17.0f}, {0.7f, 17.0f}, {1.0f, 16.5f}};
constexpr float kTipSettle = 0.5f; // s
constexpr float kNoseTiltWait = 0.45f; // s a nose manual stays level

static float ease_tilt(float tilt, float target, float tau)
{
	float d = (target - tilt) * (1 - std::exp(-kDt / tau));
	return tilt + clampf(d, -kTiltRate * kDt, kTiltRate * kDt);
}

constexpr float kPopAfterLand = 9.5f / 60; // s after touching down before a flick pops

// ---------------------------------------------------------------- ramps

// Popping up a ramp (as the port, measured on its quarter pipes): popped on the face it hardly
// changes the air, popped at the lip (just off it) it lifts the air by about half an ollie.
constexpr float kRampPopLift = 0.6f;   // share of a ramp-face pop turned to straight up
constexpr float kRampPop = 0.06f;      // a ramp-face pop's size, of an ollie's
constexpr float kLipPop = 0.28f;        // a lip pop's (straight up), of an ollie's
constexpr float kLipPopWindow = 0.22f; // s off a lip in which a flick (or its takeoff) still pops

// The pump down a transition (see ground)
constexpr float kPumpBend = 0.15f;  // rad per metre: the surface curving (a radius under ~7 m)
constexpr float kPumpSteep = 0.995f; // (not on the flat)
constexpr float kPumpGain = 0.58f;  // speed stored per unit of the curve's press (v^2 * bend)
constexpr float kPumpRate = 8.0f;   // m/s^2 it comes out at


// ---------------------------------------------------------------- powerslide

// as the port, probed with the left stick held at many reaches and angles while rolling
constexpr float kSlideReach = 0.9f;                   // pad-read reach it takes
constexpr float kSlideFromBack[2] = {21.0f, 52.0f};   // degrees off straight back
constexpr float kSlideGrip = 10.92f;                   // m/s^2 the way it's going swings round
constexpr float kSlideLead = 0.8f;                    // board lead per radian of the stick above level
constexpr float kSlideButtonLead = 0.68f;             // radians, slid with the button
constexpr float kSlideEase = 0.22f;                   // s
constexpr float kSlideOut = 2.6f;                     // rad/s easing back in line

// ---------------------------------------------------------------- crouch

// How much pop a crouch has charged, by how far it got (crouch_: the share of crouch_charge held).
// Solved point by point so ollie heights match the port's, measured at each crouch length (scripted
// holds of 1..24 ticks), together with the ollie's rise-and-hang shape (which goes by crouch time,
// see pop): a quick flick pops low, it builds slowly for a few ticks, then fast, then levels off.
// A part-way crouch (short of kDeepIn) never charges: it pops as a quick flick.
static const float kCrouchCurve[][2] = {
	{0.0000f, 0.7717f}, {0.0498f, 0.7717f}, {0.0995f, 0.7766f}, {0.1492f, 0.7964f}, {0.1990f, 0.8555f}, {0.2488f, 0.8344f}, {0.2985f, 0.8400f}, {0.3483f, 0.8425f}, {0.3980f, 0.8343f}, {0.4478f, 0.7908f}, {0.4975f, 0.7919f}, {0.5473f, 0.7567f}, {0.5970f, 0.7269f}, {0.6965f, 0.8251f}, {0.7960f, 0.9413f}, {0.8955f, 1.0319f}, {1.0f, 1.071f}};

static float crouch_charge(float u)
{
	const int n = int(sizeof kCrouchCurve / sizeof kCrouchCurve[0]);
	if (u <= kCrouchCurve[0][0])
		return kCrouchCurve[0][1];
	for (int i = 1; i < n; i++)
		if (u <= kCrouchCurve[i][0]) {
			float t = (u - kCrouchCurve[i - 1][0]) / (kCrouchCurve[i][0] - kCrouchCurve[i - 1][0]);
			return kCrouchCurve[i - 1][1] + (kCrouchCurve[i][1] - kCrouchCurve[i - 1][1]) * t;
		}
	return kCrouchCurve[n - 1][1];
}

// ---------------------------------------------------------------- step

// A stick as a game pad reads it: a round dead zone at the centre, then the rest of the throw
// scaled so it reaches full a little before the rim. Amounts (steering, spin, balance, board
// adjust) ride on this, so half a stick does what half a stick does on a pad.
constexpr float kStickDead = 0.25f;
constexpr float kStickFull = 0.95f;

static void stick_pad(const float in[2], float out[2])
{
	float r = std::sqrt(in[0] * in[0] + in[1] * in[1]);
	float k = r <= kStickDead ? 0.0f : clampf((r - kStickDead) / (kStickFull - kStickDead), 0, 1) / r;
	out[0] = in[0] * k;
	out[1] = in[1] * k;
}

void Rider::step(World &w, const sk8_input &in)
{
	stick_pad(in.steer, steer_pad_);
	stick_pad(in.flick, flick_pad_);
	if (mode_ != SK8_MODE_AIR)
		ollie_ = 0, ollie_kick_ = 0;
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
	// (only pulled right down does the crouch charge: part-way it pops low, as the port)
	float flick_r = std::sqrt(in.flick[0] * in.flick[0] + in.flick[1] * in.flick[1]);
	if (flick_.holding_lead() && (lead == TDown || lead == TUp) && mode_ != SK8_MODE_AIR && flick_r >= kDeepIn) {
		crouch_time_ += kDt;
		crouch_ = clampf(crouch_time_ / std::max(p[CrouchCharge], 0.01f), 0, 1);
	} else {
		crouch_time_ = 0;
		if (f.trick < 0 && lead >= 0) {
			// let out of the crouch on the way to the flick: as the port (probed with pauses at the
			// stick's centre), the crouch keeps for two ticks, then what the pop can still have
			// falls away fast, whatever the crouch was
			static const float kAfterLead[] = {1.0f, 1.0f, 0.65f, 0.485f, 0.355f, 0.215f, 0.0f};
			const int n = int(sizeof kAfterLead / sizeof kAfterLead[0]);
			crouch_ = std::min(crouch_, kAfterLead[std::min(since_lead_, n - 1)]);
		} else if (f.trick < 0) {
			crouch_ = approach(crouch_, 0, 6 * kDt);
		}
	}
	since_lead_ = flick_.holding_lead() ? 0 : since_lead_ + 1;
	// how far down the skater crouches, to see: with the stick, part-way too (the pop's charge,
	// crouch_, only builds pulled right down)
	{
		float look = flick_.holding_lead() && (lead == TDown || lead == TUp) && ground_mode(mode_)
			? clampf((flick_r - kLeadIn) / (kDeepIn - kLeadIn), 0, 1)
			: 0.0f;
		if (takeoff_trick_ >= 0)
			look = crouch_look_; // (held through the takeoff)
		crouch_look_ = approach(crouch_look_, std::max(look, crouch_), 9 * kDt);
	}
	lean_ = approach(lean_, clampf(steer_pad_[0], -1, 1), 5 * kDt);
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
	{
		// how long the right stick has pointed the same grind's way (a grind takes the stick only
		// once it has been held a moment, as the port, probed)
		const int zone = grind_kind_for(false, in.flick[0], in.flick[1], false);
		flick_zone_steps_ = zone == flick_zone_ ? flick_zone_steps_ + 1 : 0;
		flick_zone_ = zone;
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
		if (zone != 0 && r >= kDeepIn && mode_ != SK8_MODE_MANUAL)
			zone = 0;
		stick_manual_time_ = zone != 0 && zone == stick_manual_ ? stick_manual_time_ + kDt : 0;
		stick_manual_ = zone;
		stick_manual_depth_ = clampf((r - kManualIn) / (kDeepIn - kManualIn), 0, 1);
	}

	if (!ground_mode(mode_) && (popped_air_ || mode_ != SK8_MODE_AIR || air_time_ > kLipPopWindow))
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
	{
		float reach = std::sqrt(in.flick[0] * in.flick[0] + in.flick[1] * in.flick[1]);
		if (mode_ == SK8_MODE_MANUAL) {
			float target = stick_manual_ == manual_dir_ && reach >= kManualIn ? on_curve(kManualTilt, reach) : kManualTiltButton;
			// (a nose manual stays level a moment before it tips: as the port)
			if (manual_dir_ > 0 || manual_time_ >= kNoseTiltWait)
				manual_tilt_ = ease_tilt(manual_tilt_, target, kTiltEase);
		} else {
			manual_tilt_ = 0;
		}
		bool tip = mode_ == SK8_MODE_GRIND && (grind_kind_ == SK8_GRIND_5_0 || grind_kind_ == SK8_GRIND_NOSEGRIND);
		bool nose = grind_kind_ == SK8_GRIND_NOSEGRIND;
		float y = std::fabs(in.flick[1]);
		if (tip && tip_kind_ != grind_kind_)
			grind_tilt_ = nose ? on_curve(kNoseCatch, y) : on_curve(kTipCatch, y);
		else if (tip)
			grind_tilt_ = ease_tilt(grind_tilt_, nose ? on_curve(kNoseHold, y) : on_curve(kTipHold, y), kTipSettle);
		tip_kind_ = tip ? grind_kind_ : -1;
	}
}

// ---------------------------------------------------------------- ground

void Rider::ground(World &w, const sk8_input &in, const Flick::Result &f)
{
	if (takeoff_trick_ >= 0) {
		if (f.revised >= 0) // the flick carried on round: read again as the whole of it (as the port)
			takeoff_trick_ = f.revised;
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
	// just landed, a flick doesn't pop yet: one finished within kPopAfterLand of touching down is
	// dropped (as the port, probed: up 9 ticks after landing pops nothing, 10 ticks after pops)
	if (f.trick >= 0 && since_land_ >= 0 && since_land_ < kPopAfterLand && takeoff_trick_ < 0)
		return;
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
	if (push_time_ < 0)
		push_turn_ = false;
	if (push_turn_)
		fakie_ = false; // (pushing off the board's way out of a crawl backwards: through the stop and away)
	else if (along < -0.3f)
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
	float steer = clampf(steer_pad_[0], -1, 1);

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
		float lean = steer_pad_[1];
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

	// powerslide: the left stick pushed right out to a back diagonal (the port's way, probed: back-
	// right slides right; part-way, or swung round toward the side, it's only a carve), or the
	// slide button held while steering
	float slide_reach = std::sqrt(steer_pad_[0] * steer_pad_[0] + steer_pad_[1] * steer_pad_[1]);
	float from_back = std::atan2(std::fabs(steer_pad_[0]), -steer_pad_[1]) / kDeg; // 0 straight back
	bool stick_slide = slide_reach >= kSlideReach && from_back >= kSlideFromBack[0] && from_back <= kSlideFromBack[1];
	// (the button only on the flat: held riding up a ramp, it's for a handplant at the top)
	bool want_slide = stick_slide || ((in.buttons & SK8_SLIDE) && std::fabs(steer) > 0.4f && up_.y > 0.95f);
	if (mode_ == SK8_MODE_RIDE && want_slide && speed > 1.8f) {
		mode_ = SK8_MODE_SLIDE;
		slide_time_ = 0;
		slide_side_ = (stick_slide ? steer_pad_[0] : steer) > 0 ? 1.0f : -1.0f;
		// named as the port does: swung out to the left (the stick back-left), backside
		slide_name_ = (steer < 0) != fakie_ ? "Backside Powerslide" : "Powerslide";
	} else if (mode_ == SK8_MODE_SLIDE && ((!want_slide && !(in.buttons & SK8_SLIDE)) || speed < 0.6f)) {
		mode_ = SK8_MODE_RIDE;
		if (slide_time_ > 0.35f)
			add_trick(staged(slide_name_.c_str()), 60 + unsigned(slide_time_ * 40));
	}
	if (mode_ == SK8_MODE_SLIDE)
		slide_time_ += kDt;

	// carve: the turn comes on over a moment (two easing stages: a gentle start, like the port)
	float ease = 1 - std::exp(-kDt / std::max(p[CarveEase], 0.001f));
	carve_in_ += (on_curve(kSteerCurve, steer) - carve_in_) * ease;
	carve_ += (carve_in_ - carve_) * ease;
	float rate = p[Carve] * carve_ * clampf(speed / 2.5f, 0.35f, 1.0f);
	if (mode_ == SK8_MODE_SLIDE)
		rate = 0; // (a powerslide turns by its own rule, below)
	// a kickturn (as the port: slow, the left stick hard over): the nose lifts and the board pivots
	// on its back wheels, easing in, then turning steadily for as long as it's held
	if (mode_ == SK8_MODE_RIDE && speed < kKickturnTop && std::fabs(steer) > 0.7f && push_time_ < 0 &&
		plant_wind_ < 0 && takeoff_trick_ < 0) {
		kickturn_ += kDt;
		float k = std::min(kickturn_ / kKickturnEase, 1.0f);
		rate = kKickturnRate * k * k * (steer > 0 ? 1.0f : -1.0f);
		carve_ = carve_in_ = steer; // (no carve still to come out of when it stops)
	} else {
		kickturn_ = 0;
	}
	fwd_ = unit(flatten(rotate(fwd_, up_, -rate * kDt), up_), fwd_);
	// The wheels carry the motion round with the board (a carve keeps its speed).
	if (mode_ != SK8_MODE_SLIDE)
		vel_ = rotate(vel_, up_, -rate * kDt);

	// A powerslide (as the port, measured): the way it's going swings round with a steady sideways
	// pull, quicker the slower it goes, and the board (the skater with it) leads that way round by
	// an angle the stick sets - the further back the stick, the further across - eased in; let go,
	// it eases back in line.
	{
		V3 going = flatten(vel_, up_);
		float v = len(going);
		if (mode_ == SK8_MODE_SLIDE && v > 0.3f) {
			float side = slide_side_;
			float above = std::atan2(std::fabs(steer_pad_[1]), std::fabs(steer_pad_[0]));
			float target = in.buttons & SK8_SLIDE && !stick_slide ? kSlideButtonLead : kSlideLead * above;
			slide_off_ += (target - slide_off_) * (1 - std::exp(-kDt / kSlideEase));
			vel_ = rotate(vel_, up_, -side * kSlideGrip / std::max(v, 1.0f) * kDt);
			going = unit(flatten(vel_, up_), travel);
			V3 lead = unit(rotate(going, up_, -side * slide_off_), going);
			fwd_ = fakie_ ? -lead : lead;
		} else if (slide_off_ > 0) {
			float back = std::min(slide_off_, kSlideOut * kDt);
			slide_off_ -= back;
			if (v > 0.3f) {
				V3 g = unit(going, travel);
				float turn = dot(cross(g, travel), up_) >= 0 ? -1.0f : 1.0f; // toward the way it's going
				fwd_ = unit(rotate(fwd_, up_, turn * back), fwd_);
			}
		}
		if (mode_ != SK8_MODE_SLIDE && slide_off_ <= 0)
			slide_off_ = 0;
	}

	// gravity along the surface
	vel_ += flatten(V3{0, -p[Gravity] * p[SlopeGravity], 0}, up_) * kDt; // rolling: less than the air's pull
	// Coming down a curved transition the skater pumps it (as the port, measured on its half pipe:
	// it comes out of the bottom faster than it went up; a straight slope gives nothing extra)
	{
		float moved = len(pos_ - pump_pos_);
		float bend = moved > 1e-3f ? angle_between(pump_up_, up_) / moved : 0.0f; // rad per metre
		V3 going = flatten(vel_, up_);
		float v = len(going);
		// it builds with how hard the curve presses the board (speed squared by the bend) on the
		// way down, and comes out as speed over the next moments, the last of it on the flat
		// (by the speed the drop gives, from the top of the climb: scrubbed off sideways coming round
		// at the top, it still comes out of the bottom as fast, as the port)
		if (vel_.y > 0 || up_.y >= kPumpSteep) {
			pump_top_y_ = pos_.y;
			pump_top_v_ = len(vel_);
		}
		float fall = std::sqrt(pump_top_v_ * pump_top_v_ + 2 * p[Gravity] * p[SlopeGravity] * std::max(0.0f, pump_top_y_ - pos_.y));
		float vp = std::max(v, fall);
		if (bend > kPumpBend && vel_.y < -0.2f && up_.y < kPumpSteep && v > 0.5f)
			pump_store_ += kPumpGain * vp * vp * bend * kDt;
		else if (vel_.y > 0.3f)
			pump_store_ = 0; // (climbing again: nothing carried up the next wall)
		float give = std::min(pump_store_, kPumpRate * kDt);
		if (give > 0 && v > 0.5f) {
			vel_ += unit(going, travel) * give;
			pump_store_ -= give;
		}
		pump_pos_ = pos_;
		pump_up_ = up_;
	}

	// push: a step off the board (first stroke only), then strokes; the speed comes on while the
	// foot is on the ground driving back, not the moment the button goes down
	if (push_cooldown_ > 0)
		push_cooldown_ -= kDt;
	// (a push is as hard as the button's held for it: a tap a light one, held ~10 steps a full one,
	// as the port's)
	if (push_time_ >= 0 && push_counting_) {
		if (in.buttons & (SK8_PUSH | SK8_PUSH_MONGO))
			push_held_++;
		else
			push_counting_ = false;
	}
	if (push_time_ >= 0) {
		float period = std::max(p[PushPeriod], 0.01f);
		float t0 = push_lead_ + SK8_PUSH_PLANT * period, t1 = push_lead_ + SK8_PUSH_RELEASE * period;
		// the speed comes on in a burst as the foot bites (the first part of the plant): a share of
		// what's left to top speed, so strokes give less and less as it gets going
		t1 = std::min(t1, t0 + kPushBite);
		float drive = std::min(push_time_ + kDt, t1) - std::max(push_time_, t0);
		if (push_time_ <= t0) {
			float full = std::min(p[PushCap], std::max(0.0f, p[PushTop] - std::fabs(dot(vel_, travel))) * p[PushKick]);
			float tap = std::min(full, kPushTap);
			push_gain_ = tap + (full - tap) * std::min(1.0f, float(std::max(push_held_ - 1, 0)) / kPushFull);
			if (push_lead_ <= 0) // (held on stroke after stroke, each gives less than a fresh push, as the port)
				push_gain_ *= kPushRepeat;
		}
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
	// (barely rolling, though, it pushes off the way the board points, back the other way: as the port)
	if (push_button && fakie_ && mode_ == SK8_MODE_RIDE && switching_ < 0 && push_time_ < 0 && speed < kFakieTurnPush)
		fakie_ = false, push_turn_ = true;
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
		push_held_ = 0;
		push_counting_ = true;
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
	if (mode_ == SK8_MODE_SLIDE) { // bites in as the board turns across, eases off as it slows
		float across = std::sin(slide_off_);
		dec += p[SlideDrag] * across * across * std::min(1.0f, speed / p[SlideSoft]) * kDt;
	} else if (mode_ == SK8_MODE_RIDE && up_.y < kWallUp && speed > 0.2f) {
		// scrubbing sideways across a ramp's face drags the same way (as the port: slow at the top of a
		// quarter pipe, ridden up at an angle, it comes round and down without gathering speed)
		float across = len(cross(unit(flatten(vel_, up_), fwd_), fwd_));
		dec += kWallDrag * p[SlideDrag] * across * across * std::min(1.0f, speed / p[SlideSoft]) * kDt;
	}
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

	// Across a ramp's face the board turns after the way it's going (as the port, measured on its
	// quarter pipe: ridden up at an angle it comes round downhill at a steady rate and rides back
	// down forward; ridden straight up it rolls back fakie). Slow at the top, the way it's going
	// swings round quicker than the board: it scrubs sideways until the board catches up.
	bool on_wall = up_.y < kWallUp && mode_ == SK8_MODE_RIDE && !reverting && slide_off_ <= 0 && kickturn_ <= 0;
	if (on_wall) {
		V3 going = flatten(vel_, up_), travel = fakie_ ? -fwd_ : fwd_;
		if (len(going) > 0.2f) {
			float off = std::atan2(dot(cross(travel, going), up_), dot(travel, unit(going, travel)) * len(going));
			if (std::fabs(off) < kPi / 2) {
				float turn = std::copysign(std::min(std::fabs(off), kWallYaw * kDt), off);
				fwd_ = unit(flatten(rotate(fwd_, up_, turn), up_), fwd_);
			}
		}
	}
	// wheels resist sideways motion
	V3 side = unit(cross(up_, fwd_), V3{1, 0, 0});
	float lateral = dot(vel_, side);
	float grip = mode_ == SK8_MODE_SLIDE || slide_off_ > 0
		? 0.0f // (sliding, the board's way is the slide's own, above)
		: 1 - std::exp(-p[Grip] * kDt * (reverting ? 0.08f : 1.0f));
	float take = lateral * grip;
	if (on_wall) // (scrubbing across the face: as much as the wheels' sideways friction takes)
		take = std::copysign(std::min(std::fabs(take), kWallScrub * kDt), take);
	vel_ -= side * take;
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
	if (up_.y < 0.9f && vel_.y > 0.5f) {
		wall_off_ = true; // (up off a ramp's lip)
		V3 in = flatten(up_, kUp);
		lip_in_ = len(in) > 0.1f ? unit(in, in) : V3{};
	}
	if (up_.y < kVertLipUp && vel_.y > 0.5f) {
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
	popped_air_ = false;
}

// ---------------------------------------------------------------- air

constexpr float kOllieLoadFull = 0.21f;

void Rider::pop(const TrickDef &def, float share, bool off_rail)
{
	bool from_air = mode_ == SK8_MODE_AIR && !off_rail; // (a lip pop, just off a ramp)
	float charge = crouch_charge(crouch_);
	float v = (p[PopLow] + (p[PopHigh] - p[PopLow]) * charge) * share;
	pop_y_ = pos_.y;
	push_time_ = -1; // a stroke ends at the pop
	skid_time_ = 0;
	vert_air_ = false;
	push_cooldown_ = 0;
	mode_ = SK8_MODE_AIR;
	ollie_ = up_.y > 0.95f ? 1 : 0; // (off flat ground: the ollie's own rise and fall)
	popped_air_ = true;
	// (the ollie's rise-and-hang shape goes by how long the crouch was held: fitted against the port's
	// arcs with a full load after kOllieLoadFull seconds)
	float held = std::min(1.0f, crouch_ * p[CrouchCharge] / kOllieLoadFull);
	ollie_load_ = clampf((std::max(held, 0.35f) - 0.35f) / 0.65f, 0, 1);
	if (ollie_)
		v *= kOllieFlatTap + (kOllieFlatFull - kOllieFlatTap) * ollie_load_;
	// (off a rail it hangs only a moment at the top and comes back down onto it, as the port)
	ollie_hang_ = off_rail ? kRailHang : kOllieHang + (1 - ollie_load_) * kOllieHangTap;
	// (an ollie's board comes up off the tail over a few ticks, not at once)
	ollie_kick_ = ollie_ ? v * (1 - kOllieKickNow) : 0;
	// (up a ramp, see kRampPop: on the face a small pop mostly up, just off the lip a lift)
	V3 dir = up_;
	if (from_air) {
		dir = kUp;
		v *= kLipPop;
		ollie_kick_ = 0;
		// popped just off a steep lip: a vert air, back in over the ramp (as the port: the drift out
		// over the deck turns back in, and most of the way along the coping goes)
		if (wall_off_ && len(lip_in_) > 0.5f) {
			V3 flat = flatten(vel_, kUp);
			V3 along = flat - lip_in_ * dot(flat, lip_in_);
			vel_ = V3{0, vel_.y, 0} + along * kLipAlongKeep + lip_in_ * kLipInSpeed;
			vert_air_ = true;
			vert_in_ = lip_in_;
			v *= kLipVertLift; // (less lift on top: the board mostly turns back in)
		}
	} else if (up_.y <= 0.95f) {
		dir = unit(up_ * (1 - kRampPopLift) + kUp * kRampPopLift, up_);
		v *= kRampPop;
		ollie_kick_ = 0;
	}
	vel_ += dir * (v - ollie_kick_);
	pos_ += up_ * 0.02f;
	air_time_ = 0;
	spin_total_ = 0;
	spin_hold_ = 0;
	spin_lag_[0] = spin_lag_[1] = 0;
	crouch_ = crouch_time_ = 0;
	grab_scored_ = false;
	grab_time_ = 0;
	stance_pre_ = switch_ ? "Switch " : fakie_ ? "Fakie " : "";
	switching_ = -1; // a pop cuts a shuffle short
	push_queued_ = false;
	plant_wind_ = -1;
	plant_air_ = false;
	wall_off_ = false;
	flick_.rearm();
	air_up_ = air_down_in_ = 0;
	air_feet_ = air_feet_want_ = 0;
	air_feet_scored_ = false;
	adjust_[0] = adjust_[1] = 0;
	airwalk_ = shifty_scored_ = air_footplant_ = false;
	shifty_ = shifty_time_ = 0;
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
	dark_used_ = false;
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
	if (r < kHeldIn || std::fabs(dir[0]) < 0.3f)
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
	float to_land = air_left();
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

// The pull of gravity through an ollie off flat ground (as a share of it), stepping its phase on.
static float ollie_pull(int &ollie, float &hang, float air_time, float vy, float load, float hang_for)
{
	if (ollie <= 0)
		return 1;
	if (ollie == 1 && (air_time > kOllieRise || vy < kOllieHangVy))
		ollie = 2;
	if (ollie == 2 && vy <= kOllieHangVy) {
		ollie = 3;
		hang = 0;
	}
	if (ollie == 3 && (hang += kDt) > hang_for)
		ollie = 4;
	if (ollie == 4 && vy < -kOllieDropTo)
		ollie = 0;
	// (the pop's first moment carries it straight up: the board still coming up off the tail)
	return ollie == 1 ? (air_time < kOllieCarry ? 0.0f : kOllieRiseTap + (kOllieRiseFull - kOllieRiseTap) * load) : ollie == 2 ? kOllieBrake : ollie == 3 ? kOllieHangPull : ollie == 4 ? kOllieDrop : 1.0f;
}

// How long until it's back down to the height it left from, the ollie's way down and all.
float Rider::air_left() const
{
	int ollie = ollie_;
	float hang = hang_, t = air_time_, vy = vel_.y + ollie_kick_, y = pos_.y, g = p[Gravity], left = 0;
	while (left < 4 && (vy > 0 || y > pop_y_)) {
		t += kDt;
		vy -= g * ollie_pull(ollie, hang, t, vy, ollie_load_, ollie_hang_) * kDt;
		y += vy * kDt;
		left += kDt;
	}
	return left;
}

void Rider::air(World &w, const sk8_input &in, const Flick::Result &f)
{
	if (air_time_ <= 0) { // into the air: where from, for the air's height and length
		launch_ = pos_;
		launch_vel_ = vel_;
		peak_ = pos_.y;
	}
	air_time_ += kDt;
	peak_ = std::max(peak_, pos_.y);
	// Just off a lip (rolled off, not popped): a flick there, or one whose takeoff began on the
	// ramp, still pops (as the port, probed on its quarter pipes) - straight up, as from the lip.
	if (!popped_air_ && air_time_ <= kLipPopWindow) {
		if (takeoff_trick_ < 0 && f.trick >= 0) {
			takeoff_trick_ = f.trick;
			takeoff_share_ = trick_table()[size_t(f.trick)].nollie ? p[NolliePop] : 1.0f;
			takeoff_time_ = 0;
			takeoff_crouch_ = crouch_;
			flick_dir(takeoff_dir_);
		}
		if (takeoff_trick_ >= 0) {
			takeoff_time_ += kDt;
			if (takeoff_time_ >= p[PopWait]) {
				int trick = takeoff_trick_;
				takeoff_trick_ = -1;
				bool vert = vert_air_;
				V3 vert_in = vert_in_;
				crouch_ = takeoff_crouch_;
				const TrickDef &def = trick_table()[size_t(trick)];
				pop(def, takeoff_share_);
				if (vert) // (still a vert air: it comes back into the ramp)
					vert_air_ = true, vert_in_ = vert_in;
				if (flip_ < 0 && !def.flat)
					start_flip(trick);
				else if (def.flat)
					pending_.push_back({staged(def.name), def.points});
				return;
			}
		}
	} else if (!popped_air_) {
		takeoff_trick_ = -1; // too late: it's just an air
	}
	float pull = 1;
	if (ollie_kick_ > 0) {
		float add = std::min(ollie_kick_, (ollie_kick_ + 1e-3f) / std::max(kOllieKickTicks - air_time_ / kDt + 1, 1.0f));
		vel_.y += add;
		ollie_kick_ -= add;
	}
	pull = ollie_pull(ollie_, hang_, air_time_, vel_.y, ollie_load_, ollie_hang_);
	if (vert_air_ && vel_.y > 0)
		pull *= kVertPull; // (a vert air floats a little on the way up: as the port's)
	vel_ += V3{0, -p[Gravity] * pull, 0} * kDt;
	float sp = len(vel_);
	if (sp > 0)
		vel_ *= std::max(0.0f, 1 - p[AirDrag] * sp * kDt);

	// spin about the stance axis; near a rail, the left stick held straight back is picking a crouched
	// grind instead (sideways it spins the board as anywhere: that's how a slide is set up onto a
	// rail, as in the port)
	bool by_rail = in.steer[1] < -0.6f && std::fabs(in.steer[0]) < 0.4f && !w.rails.empty() &&
		w.nearest_rail(pos_, 1.0f).rail >= 0;
	// (grabbing with the right stick down, the left stick is a shifty, not a spin)
	bool shifting = grab_ && in.flick[1] < -0.5f;
	// as the port (probed): the stick has to be held over a moment before a spin starts (a tap
	// does nothing), then it comes up to speed quickly, and let go it carries on, slowing gradually
	float stick = by_rail || shifting ? 0.0f : on_curve(kSpinCurve, clampf(steer_pad_[0], -1, 1));
	bool varial = in.buttons & SK8_FOOT_OFF;
	// (as the port, probed: the board answers the stick a couple of ticks late, going over and
	// letting go alike - so even a tap turns it some; a body varial, ours, answers at once)
	float late = spin_lag_[1];
	spin_lag_[1] = spin_lag_[0];
	spin_lag_[0] = stick;
	if (!varial)
		stick = late;
	bool over = std::fabs(stick) > 0.02f; // (the pad dead zone already keeps a resting stick out)
	spin_hold_ = over && (spin_hold_ == 0 || (stick > 0) == (spin_dir_ > 0)) ? spin_hold_ + kDt : 0;
	if (over)
		spin_dir_ = stick > 0 ? 1.0f : -1.0f;
	// (quickly to about a third of the full rate, then gathering speed; held through the pop, it
	// waits until the board's up)
	bool up_yet = varial || air_time_ > kSpinUp;
	float target = over && up_yet ? -stick * p[SpinRate] : 0.0f;
	bool speeding = std::fabs(target) > std::fabs(spin_speed_) || target * spin_speed_ < 0;
	float ease = 1 - std::exp(-kDt * std::max(p[SpinEase], 0.1f));
	if (varial) // (a body varial: at once)
		spin_speed_ += (target - spin_speed_) * ease;
	else if (!speeding) // let go: it carries on round, slowing
		spin_speed_ += (target - spin_speed_) * (1 - std::exp(-kDt / kSpinCoast));
	else if (target * spin_speed_ < 0 || std::fabs(spin_speed_) < kSpinKick) {
		float kick = std::min(std::fabs(target), kSpinKick * 1.1f);
		spin_speed_ += ((target > 0 ? kick : -kick) - spin_speed_) * (1 - std::exp(-kDt * kSpinKickEase));
	} else
		spin_speed_ = approach(spin_speed_, target, kSpinAccel * kDt);
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
	if (!vert_air_ && p[AirLevel] > 0) {
		float tilt = angle_between(up_, kUp);
		if (tilt > 1e-3f) {
			V3 axis = unit(cross(up_, kUp), V3{1, 0, 0});
			float turn = std::min(tilt, p[AirLevel] * kDt);
			V3 h0 = flatten(fwd_, kUp);
			up_ = unit(rotate(up_, axis, turn));
			fwd_ = unit(flatten(rotate(fwd_, axis, turn), up_), fwd_);
			// (it levels without turning: the nose keeps pointing the way it did, as the port's off a
			// steep lip - the shortest way level would swing it round)
			V3 h1 = flatten(fwd_, kUp);
			if (len(h0) > 0.2f && len(h1) > 0.2f) {
				float yaw = std::atan2(dot(cross(h1, h0), kUp), dot(h1, h0));
				fwd_ = unit(flatten(rotate(fwd_, up_, yaw), up_), fwd_);
			}
		}
	}

	// dark: the slide button held while a flip is turning catches the board upside down under the
	// feet (as the port); flicked again, it goes on round the rest of the way (dark to light)
	if (flip_ >= 0 && !dark_ && !dark_used_ && (in.buttons & SK8_SLIDE) && under_a0_ < 0 && flip_extra_ == 0) {
		const TrickDef &d = trick_table()[size_t(flip_)];
		float e = flip_frac();
		if (d.roll != 0 && d.wrap == 0 && e > 0.15f && e < 0.6f) {
			dark_ = dark_used_ = true;
			flip_turns_ = 0.5f * flip_base_turns_;
			flip_mult_ = 0.5f;
			flip_name_ = "Darkcatch " + flip_name_;
			trick_ = flip_name_;
			if (!pending_.empty()) {
				pending_.back().name = flip_name_;
				pending_.back().points += 120;
			}
		}
	}
	if (dark_ && flip_ >= 0 && f.trick >= 0 && trick_table()[size_t(f.trick)].roll != 0) {
		dark_ = false;
		flip_turns_ = flip_base_turns_;
		flip_mult_ = 1;
		const TrickDef &d = trick_table()[size_t(flip_)];
		std::string name = stance_pre_ + "Dark to Light " + d.name;
		pending_.push_back({name, 160});
		trick_ = flip_name_ = name;
	}
	// flick a trick late
	// (not while grabbing: the stick then tweaks the grab)
	if (!foot_air_ && f.trick >= 0 && flip_ < 0 && !grab_ && p[AirFlick] >= 0.5f && !trick_table()[size_t(f.trick)].flat &&
		takeoff_trick_ < 0) {
		// flicked from up (a nollie's way) in the air: the back foot flips it (a late backfoot flip)
		const TrickDef &d = trick_table()[size_t(f.trick)];
		bool backfoot = d.nollie && d.roll != 0 && d.wrap == 0;
		int trick = backfoot ? find_trick(d.roll > 0 ? "Kickflip" : "Heelflip") : f.trick;
		start_flip(trick);
		std::string base = trick_table()[size_t(trick)].name;
		flip_name_ = stance_pre_ + (backfoot ? "Late Backfoot " : "Late ") + base;
		trick_ = flip_name_;
		if (!pending_.empty())
			pending_.back().name = flip_name_;
	}
	// a late backfoot flip as the port reads one: the stick held up (or up to a side) in the air, then
	// snapped down: straight down or down-right a kickflip, down-left a heelflip
	{
		float r = std::sqrt(in.flick[0] * in.flick[0] + in.flick[1] * in.flick[1]);
		float deg = std::atan2(in.flick[0], in.flick[1]) * 180 / kPi; // 0 up, clockwise
		bool up = r > 0.8f && deg > -70 && deg < 70;
		bool kick = r > 0.8f && (deg > 112 || deg < -158), heel = r > 0.8f && deg >= -158 && deg < -112;
		if (up) {
			air_up_ += kDt;
			air_down_in_ = 0;
		} else {
			air_down_in_ += kDt;
		}
		if (air_up_ >= kBackfootHold && air_down_in_ <= kBackfootSnap && (kick || heel) && flip_ < 0 && !grab_ &&
			!foot_air_ && mode_ == SK8_MODE_AIR) {
			start_flip(find_trick(kick ? "Kickflip" : "Heelflip"));
			flip_name_ = stance_pre_ + (kick ? "Late Backfoot Kickflip" : "Late Backfoot Heelflip");
			trick_ = flip_name_;
			if (!pending_.empty())
				pending_.back().name = flip_name_;
			air_up_ = 0;
		}
		if (!up && air_down_in_ > kBackfootSnap)
			air_up_ = 0;
	}
	// a flick while grabbing (a sweep, not a held tweak): the hand spins the board (a finger flip)
	if (!foot_air_ && f.trick >= 0 && flip_ < 0 && grab_ && grab_time_ > 0.2f) {
		const TrickDef &d = trick_table()[size_t(f.trick)];
		if (!d.flat && (d.roll != 0 || d.shove != 0) && !d.nollie) {
			start_flip(f.trick);
			flip_speed_ = kFingerFlipSpeed; // the hand spins it round much quicker than a foot flick
			flip_name_ = std::string(grab_def(grab_style_).name) + " Finger Flip";
			trick_ = flip_name_;
			if (!pending_.empty()) {
				pending_.back().name = flip_name_;
				pending_.back().points += 100;
			}
		}
	}
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
		bool held = r >= kHeldIn &&
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
	// a trigger already down while the board flips doesn't grab: only a fresh pull once the flip is
	// caught does (as the port, probed: held through the flip, no grab; pulled after it, a grab)
	if (flip_ >= 0 && hands && !grab_)
		grab_held_through_flip_ = true;
	if (!hands)
		grab_held_through_flip_ = false;
	if (grab_held_through_flip_)
		hands = 0;
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
		// B with a grab (as the port): a nose / tail grab's legs walk off the board (an airwalk), a
		// double grab flies out flat (a superman); any other grab lets go of the board (a dismount)
		bool b_press = (in.buttons & SK8_BRAKE) && !(prev_buttons_ & SK8_BRAKE);
		if (grab_ && b_press && (grab_style_ == SK8_GRAB_NOSE || grab_style_ == SK8_GRAB_TAIL)) {
			airwalk_ = true;
			pending_.push_back({std::string(grab_def(grab_style_).name) + " Airwalk", 220});
		} else if (grab_ == 3 && b_press && grab_style_ != SK8_GRAB_SUPERMAN) {
			grab_style_ = SK8_GRAB_SUPERMAN;
			pending_.push_back({grab_def(grab_style_).name, grab_def(grab_style_).points});
		} else if (grab_ && b_press && !airwalk_ && !never_bail()) {
			bail("Dismount");
			return;
		}
		if (airwalk_ && (!grab_ || !(in.buttons & SK8_BRAKE)))
			airwalk_ = false;
		if (airwalk_)
			air_feet_ = SK8_PLANT_FRONT_FOOT | SK8_PLANT_BACK_FOOT;
		// a shifty (as the port): grabbing, the right stick down and the left stick swung to a side,
		// held a moment: the board twisted out that way
		bool shift = grab_ && in.flick[1] < -0.5f && std::fabs(in.steer[0]) > 0.6f;
		shifty_time_ = shift ? shifty_time_ + kDt : 0;
		if (shift && shifty_time_ > 0.2f && !shifty_scored_) {
			shifty_scored_ = true;
			bool fs = (in.steer[0] > 0) != fakie_;
			pending_.push_back({std::string(fs ? "Frontside Shifty " : "Backside Shifty ") + grab_def(grab_style_).name, 140});
		}
		shifty_ = shift ? (in.steer[0] > 0 ? 1.0f : -1.0f) : 0.0f;
		// a one-foot grab coming down onto the ground: the free foot planted, a hop back up (a footplant)
		if (grab_ && air_feet_ && !airwalk_ && vel_.y < 0 && !air_footplant_) {
			Hit g;
			if (w.cast(pos_, V3{0, -0.3f, 0}, g) && g.normal.y > 0.8f) {
				air_footplant_ = true;
				vel_.y = 3.4f;
				pending_.push_back({std::string(grab_def(grab_style_).name) + " Footplant", 180});
				emit(SK8_EV_POP, "Footplant", 0, 0, g.surface);
			}
		}
	}
	// the right stick held with nothing else going on: the board adjusted under the feet (nose up /
	// down, swung to a side)
	{
		bool free = flip_ < 0 && !grab_ && !foot_air_ && air_time_ > 0.08f;
		float r = std::sqrt(in.flick[0] * in.flick[0] + in.flick[1] * in.flick[1]);
		float tx = free && r > 0.6f ? flick_pad_[0] : 0, ty = free && r > 0.6f ? flick_pad_[1] : 0;
		if (shifty_ != 0) // a shifty swings the board under the feet the way the left stick went
			tx = shifty_;
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

	// grind assist (as the port: in a recorded session an ollie beside a ledge drifted ~0.8 m onto
	// it with the stick centred): a rail running along the way it's going, below and within reach,
	// pulls the board sideways onto its line
	// (not up off a ramp's own lip: an air off a quarter pipe comes down where it goes, onto the deck
	// past the coping, as the port - not drawn back onto the coping)
	if (!foot_air_ && grind_cooldown_ <= 0 && !w.rails.empty() && p[GrindAssist] > 0 && !(wall_off_ && !popped_air_)) {
		// (from the pop on: the rail it's coming down on - looked for along the way it's going, out to
		// where it lands - up to a pop's height above, by how far it is off to the side; as the port,
		// which aims at it from the pop and then only drifts across)
		V3 flat_v = flatten(vel_, kUp);
		float t_left = std::max(air_left(), 0.1f);
		RailSpot s{};
		s.rail = -1;
		for (int k = 1; k <= 4 && s.rail < 0; k++) {
			V3 probe = pos_ + flat_v * (t_left * float(k) / 4);
			RailSpot q = w.nearest_rail(probe, kAssistReach + kAssistAbove + len(flat_v) * t_left * 0.25f);
			if (q.rail >= 0 && q.point.y < pos_.y + kAssistAbove) {
				// (by how far its line is off to the side of the board now)
				V3 qt = unit(flatten(q.tangent, kUp), q.tangent), qo = flatten(q.point - pos_, kUp);
				if (len(qo - qt * dot(qo, qt)) <= kAssistReach)
					s = q;
			}
		}
		if (s.rail >= 0) {
			V3 flat_t = unit(flatten(s.tangent, kUp), s.tangent);
			float along = len(flat_v) > 0.5f ? std::fabs(dot(unit(flat_v, flat_v), flat_t)) : 0;
			if (along > kAssistAlong) {
				V3 off = flatten(s.point - pos_, kUp);
				off = off - flat_t * dot(off, flat_t);
				float d = len(off);
				if (d > 0.02f) {
					V3 dir = off * (1 / d);
					// toward the line, easing off as it gets there (no overshoot past it)
					float want = std::min(p[GrindAssist] * kDt, std::max(0.0f, std::min(d * 6, kAssistSpeed) - dot(vel_, dir)));
					vel_ += dir * want;
				}
			}
		}
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
	// (by which way it's going, however slow: as the port, a slow 180 still rolls away fakie)
	if (len(vflat) > 0.1f) {
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
	// landed with the board caught upside down: the skater steps off and runs it out (as the port)
	if (dark_) {
		dark_ = false;
		flip_ = -1;
		commit_pending();
		pos_ = h.point;
		up_ = kUp;
		vel_ = flatten(vel_, kUp) * 0.55f;
		mode_ = SK8_MODE_WALK;
		emit(SK8_EV_LAND, "dark", 0, 1, h.surface);
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
		// named as the port does: on the trick done in the air ("Kickflip 180", "Stiffy 360"), or with
		// nothing else done, by the way it spun (the left stick left: backside, right: frontside)
		std::string deg = std::to_string(halves * 180);
		Pending *last = nullptr;
		for (auto it = pending_.rbegin(); it != pending_.rend(); ++it)
			if (it->name != "Big Air" && it->name != "Long Air") {
				last = &*it;
				break;
			}
		if (last) {
			last->name += " " + deg;
			last->points += unsigned(halves * 110);
		} else {
			bool backside = (spin_total_ > 0) != fakie_;
			pending_.push_back({std::string(backside ? "Backside " : "Frontside ") + deg, unsigned(halves * 110)});
		}
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
	airwalk_ = shifty_scored_ = air_footplant_ = false;
	shifty_ = shifty_time_ = 0;
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
	float x = clampf(steer_pad_[0], -1, 1), y = clampf(steer_pad_[1], -1, 1);
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
	// the right stick's way, as the port's (probed on a quarter pipe): nearer an axis, that one; an
	// exact diagonal to the left is left, to the right up / down
	float x = in.flick[0], y = in.flick[1];
	if (x * x + y * y > 0.36f) {
		if (x < 0 && -x >= std::fabs(y))
			bits |= SK8_PLANT_LEFT;
		else if (x > 0 && x > std::fabs(y))
			bits |= SK8_PLANT_RIGHT;
		else
			bits |= y < 0 ? SK8_PLANT_DOWN : SK8_PLANT_UP;
	}
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
	else if (bits & SK8_PLANT_UP)
		name += " Up";
	else if (bits & (SK8_PLANT_LEFT | SK8_PLANT_RIGHT))
		name += (bits & SK8_PLANT_RIGHT) ? " Right" : " Left";
	return name;
}

// A slide by the right stick centred / up / down (and the diagonals the port reads as the same
// family: on the lip side up-right is a noseblunt, up-left a tailslide, down-right a lipslide; on the board side up-left a
// noseslide), in the family the board was turned into.
static int slide_kind(bool lip, float x, float y)
{
	bool up = y > 0.45f, down = y < -0.45f;
	if (lip) // (up-left well over: a tailslide, as the port's recorded one)
		return up ? (x < -0.45f ? SK8_GRIND_TAILSLIDE : SK8_GRIND_NOSEBLUNT)
			: down && x < 0.45f ? SK8_GRIND_TAILSLIDE : SK8_GRIND_LIPSLIDE;
	return up ? SK8_GRIND_NOSESLIDE : down ? SK8_GRIND_BLUNTSLIDE : SK8_GRIND_BOARDSLIDE;
}

// Does the stick pick within a slide's family (else, sideways, the grind table: the port turns the
// board back along the rail)?
static bool slide_family_stick(bool lip, float x, float y)
{
	if (std::fabs(x) < 0.45f)
		return true;
	return lip ? (x > 0 && std::fabs(y) > 0.45f) || (x < 0 && y > 0.45f) : x < 0 && y > 0.45f;
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

// Which grind / slide the board is in, from how it lies on the rail and the right stick at the
// catch. Returns whether it's ridden backwards along the rail.
bool Rider::classify_grind(V3 tangent, V3 from)
{
	V3 t = tangent * float(rail_dir_);
	float a = angle_between(fwd_, unit(flatten(t, kUp), t));
	bool backwards = a > kPi / 2;
	if (backwards)
		a = kPi - a;
	// (as the port, recorded: ~60 deg off is still a grind - crooked, 5-0 - ~75 a slide)
	bool slide = a > kSlideAngle;
	bool crouched = catch_crouched_;
	grind_slide_ = slide;
	grind_transfer_ = 0;
	// (in switch the stick's sides swap, as the port reads a switch flick mirrored)
	float gx = switch_ ? -last_flick_[0] : last_flick_[0], gy = last_flick_[1];
	// a stick only just pushed at the catch doesn't pick the grind yet (it changes over a moment
	// later, as the port: pushed 6 steps before the catch is a 50-50 first, 8 is the 5-0 straight away)
	if (!slide && flick_zone_steps_ < kCatchStick)
		gx = gy = 0;
	// (the stick held sideways turned the board across in the air: that's spent on the turn - which
	// way it turned picks the family, a boardslide or lipslide, as the port's)
	if (slide && catch_adjusted_)
		gx = 0;
	grind_kind_ = grind_kind_for(slide, gx, gy, crouched);
	// a slide with the right stick centred, down or up: which family by which way the board was
	// turned onto the rail (as the port, probed: turned with the left stick left, the lip side - lipslide,
	// tailslide, noseblunt; right, the board side - boardslide, bluntslide, noseslide)
	// The lip side is the hard way: the back of the board (the back foot's end: the nose in switch)
	// swung over the rail, past it from the side it came from; the board side, the front end over.
	// (coming straight along it, by which way the board was turned against the rail's direction)
	{
		V3 across = flatten(cross(unit(flatten(t, kUp), t), kUp), kUp);
		float came = dot(flatten(from - pos_, kUp), across);
		if (std::fabs(came) > 0.05f)
			grind_lip_ = (dot(fwd_, across) * came > 0) != switch_;
		else
			grind_lip_ = (dot(cross(unit(flatten(t, kUp), t), fwd_), kUp) > 0) != switch_;
	}
	if (slide && slide_family_stick(grind_lip_, gx, gy))
		grind_kind_ = slide_kind(grind_lip_, gx, gy);
	// landed on it turned part way (not square across): the nose over one side - the board side,
	// stick up a crooked, else a salad; the lip side, stick up an overcrook (as the port, probed)
	if (!slide && !crouched && a > kAngledGrind && std::fabs(gx) < 0.45f) {
		bool up = gy > 0.45f;
		if (up)
			grind_kind_ = grind_lip_ ? SK8_GRIND_OVERCROOK : SK8_GRIND_CROOKED;
		else if (!grind_lip_)
			grind_kind_ = SK8_GRIND_SALAD;
	}
	return backwards;
}

bool Rider::try_grind(World &w)
{
	// the slide button held going up off a ramp: a handplant on its edge (the port's way in)
	// (off the top of a ramp only: RB held through an ollie by a ledge is a darkcatch, not a plant)
	bool hand = (last_buttons_ & SK8_SLIDE) && wall_off_;
	if (grind_cooldown_ > 0 || w.rails.empty() || (vel_.y > 2.5f && !hand))
		return false;
	RailSpot s = w.nearest_rail(pos_, p[GrindCatch] * kCrossCatch);
	if (s.rail < 0 || pos_.y - s.point.y < -0.15f)
		return false;
	// along the rail it's caught only right over it (as the port: ~0.25 m to the side it lands beside
	// it); coming across it (a slide) it's over the rail only a moment: caught from further
	{
		V3 ft = unit(flatten(s.tangent, kUp), s.tangent), fv = flatten(vel_, kUp);
		bool along_rail = len(fv) > 0.5f && std::fabs(dot(unit(fv, fv), ft)) > 0.57f;
		if (along_rail && s.distance > p[GrindCatch])
			return false;
	}
	// an air rolled up off a ramp's lip catches its coping only coming down right onto it (as the
	// port: a little way past, onto the deck, it lands)
	if (wall_off_ && !popped_air_ && !hand && len(flatten(s.point - pos_, kUp)) > kLipCatch)
		return false;
	// still rising past the rail (an ollie up beside it, or just over it at the top): not yet - it
	// lands on it coming down (as the port, recorded: caught as the board comes down onto it)
	if (vel_.y > 0.05f && !(last_buttons_ & SK8_SLIDE))
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
	// over the rail, along it: coming down, the board is drawn in over it until it comes down onto it
	// (no snap from high up; the ollie's own drop brings it down, as the port's). Across it (a slide)
	// the moment over it is brief: caught at once.
	float gap = pos_.y - s.point.y;
	float along_cos = std::fabs(dot(unit(flatten(vel_, kUp), fwd_), unit(flatten(s.tangent, kUp), s.tangent)));
	if (gap > 0.09f && !plant && along_cos > 0.57f) {
		if (vel_.y < 0) {
			V3 lat = flatten(s.point - pos_, kUp);
			lat = lat - s.tangent * dot(lat, s.tangent);
			vel_ += lat * (9.0f * kDt);
		}
		return false;
	}
	// half way round a flip, across the rail: it lands upside down on it (a darkslide)
	bool dark = false;
	if (flip_ >= 0 && trick_table()[size_t(flip_)].roll != 0 && ((flip_frac() > 0.3f && flip_frac() < 0.75f) || dark_)) {
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
	int mode_before = mode_;
	// a board adjust held into the catch (the right stick turning the board under the body, as the
	// port's): the board lands turned that way on the rail - a slide set up without spinning the body
	// (not with the left stick held back: then the right stick sideways picks a crouched grind)
	// (the stick pushed up or down picks a grind, nosegrind / crooked / 5-0, and doesn't turn it)
	// (drawn, the body keeps facing where it was and the board stays where it showed in the air,
	// both easing round to the catch's over a moment: a catch doesn't snap either)
	catch_body_ = catch_board_ = 0;
	catch_adjusted_ = false;
	if (std::fabs(adjust_[0]) > 0.3f && last_steer_[1] > -0.6f && std::fabs(last_flick_[1]) < 0.45f &&
		flick_zone_steps_ >= kCatchStick) { // (pushed only just before the catch: not turned yet)
		catch_adjusted_ = true;
		fwd_ = unit(flatten(rotate(fwd_, up_, -adjust_[0] * kAdjustCatch), kUp), fwd_);
		catch_body_ = adjust_[0] * kAdjustCatch;
		catch_board_ = adjust_[0] * (kAdjustCatch - kAdjustYaw);
	}
	rail_ = s.rail;
	seg_ = s.seg;
	seg_u_ = s.u;
	rail_dir_ = vt >= 0 ? 1 : -1;
	rail_speed_ = std::fabs(vt) * p[GrindKeep];
	pos_ = s.point;
	up_ = kUp;
	fwd_ = unit(flatten(fwd_, kUp), fwd_);
	// crouched for the catch: loaded on the ground, or the left stick held back
	catch_crouched_ = crouch_ > 0.5f || last_steer_[1] < -0.6f;
	bool backwards = classify_grind(s.tangent, mode_before == SK8_MODE_AIR ? launch_ : pre_pos);
	spin_speed_ = 0; // (the board stops turning as it lands on the rail, as the port's)
	bool slide = grind_slide_;
	if (dark)
		grind_kind_ = SK8_GRIND_DARKSLIDE;
	if (plant) {
		grind_kind_ = hand || (last_buttons_ & (SK8_GRAB_LEFT | SK8_GRAB_RIGHT)) ? SK8_GRIND_HANDPLANT : SK8_GRIND_FOOTPLANT;
		plant_hold_ = hand ? SK8_SLIDE : SK8_FOOT_OFF;
		// (as the port, probed: going straight in, or drifting toward the toes, frontside; drifting
		// toward the heels, backside)
		plant_side_ = hand ? (plant_lateral > -0.3f ? "Frontside " : "Backside ") : "";
		// the stick already held for a variant: straight into it (as the port names it)
		sk8_input held{};
		held.buttons = last_buttons_;
		held.flick[0] = last_flick_[0];
		held.flick[1] = last_flick_[1];
		plant_want_ = plant_pose_ = hand ? plant_bits(held) : 0;
		plant_want_time_ = 0;
		plant_time_ = 0;
		// (off a ramp's lip going up: the way up the wall, so it comes back down into the ramp)
		plant_in_ = wall_off_ ? vert_in_ * -1.0f : unit(flatten(vel_, kUp), fwd_);
		rail_speed_ = 0;
	}
	// named for how it's ridden: switch stance, or riding the rail backwards (fakie)
	grind_backwards_ = backwards && !slide;
	// frontside / backside as the port names it: the rail on the right of the way it's going,
	// backside; on the left, frontside (the other way round rolling fakie). Right over it, by
	// which way it was drifting.
	{
		// (judged from the takeoff: the way it went up, and where the rail lay off that path -
		// the pull onto the rail and a spin in the air don't change it)
		V3 heading = fakie_ ? fwd_ * -1.0f : fwd_;
		V3 up_vel = mode_before == SK8_MODE_AIR ? launch_vel_ : pre_vel;
		V3 travel = len(flatten(up_vel, kUp)) > 0.5f ? unit(flatten(up_vel, kUp), heading) : heading;
		V3 right = cross(travel, kUp);
		V3 from = mode_before == SK8_MODE_AIR ? launch_ : pre_pos;
		float off = dot(flatten(s.point - from, kUp), right);
		if (std::fabs(off) < 0.05f)
			off = dot(flatten(pre_vel, kUp), right);
		bool on_right = off >= 0;
		// (riding fakie swaps it; switch doesn't - the port's switch-fakie catches with the rail
		// on the left were all backside)
		grind_side_ = on_right != fakie_ ? "Backside " : "Frontside ";
	}
	grind_grab_ = 0;
	grind_grab_want_ = 0;
	grind_grab_time_ = 0;
	grind_name_ = grind_full_name();
	if (plant && hand)
		grind_name_ = plant_pose_ ? plant_variant(plant_pose_) : plant_side_ + kGrindDefs[grind_kind_].name;
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
	catch_body_ = catch_board_ = 0;
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
		bool handplant = grind_kind_ == SK8_GRIND_HANDPLANT;
		float stall = handplant ? p[PlantStall] : 0.65f;
		// (a handplant, once caught, is the whole move - up over the coping and back down - however
		// long the button's held, as the port's; a footplant stalls while it's held)
		bool let_go = !handplant && plant_time_ > 0.15f && !(in.buttons & plant_hold_);
		if (f.trick >= 0 || plant_time_ > stall || let_go) {
			if (wall_off_ && f.trick < 0) {
				// on a ramp's lip: the body swings back over and it drops straight in, nose first,
				// onto the wall it came up (comes down along it, as a vert air does)
				// (off a handplant, falling back from up over the coping: as fast as that drop)
				leave_grind(kUp * -(handplant ? kPlantDropIn : 1.6f) + vert_in_ * 0.3f, true);
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
	catch_body_ *= std::exp(-kDt / kCatchEase);
	catch_board_ *= std::exp(-kDt / kCatchEase);
	const auto &r = w.rails[size_t(rail_)];
	V3 tangent = unit(r[size_t(seg_) + 1] - r[size_t(seg_)]);
	V3 t = tangent * float(rail_dir_);
	// off a rail only with a flick that ends the popping way (up; for a nollie, down): the stick moved
	// about to change grinds (down, centre, down again) doesn't pop it off (as the port)
	bool pops_off = f.trick >= 0 && (trick_table()[size_t(f.trick)].nollie ? last_flick_[1] < -0.3f : last_flick_[1] > 0.3f);
	if (pops_off) {
		const TrickDef &def = trick_table()[size_t(f.trick)];
		bool was_dark = grind_kind_ == SK8_GRIND_DARKSLIDE;
		leave_grind(t * rail_speed_, true);
		pop(def, p[GrindPop], true);
		if (was_dark) {
			// off a darkslide: the board flipped back over the rest of the way (a darkslide out)
			int roll = def.roll != 0 && def.wrap == 0 ? f.trick : find_trick("Kickflip");
			start_flip(roll);
			flip_angle_ = 0.5f * flip_base_turns_;
			flip_name_ = stance_pre_ + "Darkslide Out";
			trick_ = flip_name_;
			if (!pending_.empty())
				pending_.back() = {flip_name_, 200};
		} else if (!def.flat)
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
	// the right stick mid-rail (as the port, probed): pushed up or down and held, over onto that truck
	// (~10 steps); right out to a side and held, the board turns round that way a step at a time (the
	// first ~17 steps, then ~8 each): onto the front truck and round - crooked, willy, boardslide; on the
	// back truck - salad, feeble, boardslide; the other way smith, or from the front overcrook, then
	// lipslide. Part way to the side it settles back onto both trucks. Toward the centre, or a
	// diagonal, it stays as it is. (A slide caught across the rail goes by its own family, below.)
	auto change_to = [&](int want) {
		add_trick(grind_name_, kGrindDefs[grind_kind_].points + unsigned(grind_time_ * p[GrindRate]));
		grind_kind_ = want;
		grind_name_ = grind_full_name();
		grind_time_ = 0;
		grind_transfer_ = 0;
		trick_ = grind_name_;
		emit(SK8_EV_GRIND, grind_name_.c_str(), 0, grind_kind_, surface_);
	};
	const float fx = switch_ ? -in.flick[0] : in.flick[0], fy = in.flick[1];
	int tip = 0, rot = 0;
	if (grind_kind_ != SK8_GRIND_DARKSLIDE && !grind_slide_ && pose_of_grind(grind_kind_, tip, rot)) {
		float r = std::sqrt(fx * fx + fy * fy);
		float a = std::atan2(fx, fy) / kDeg;
		if (a < 0)
			a += 360;
		// (the way round to a side leans toward the truck it's on: on the nose, up-right turns it too;
		// on the tail, down-right; on both, only near straight across)
		float board_c = 90 - 15 * float(tip), lip_c = tip == 0 ? 270.0f : 285.0f;
		bool right = std::fabs(a - board_c) <= 22.4f, left = std::fabs(a - lip_c) <= 22.4f;
		bool up = fy > 0.6f && std::fabs(fx) <= fy * 1.0275f, down = fy < -0.52f && std::fabs(fx) <= -fy * 1.0275f;
		int want_tip = tip, want_rot = rot;
		float turn_need = rot == 0 ? kTurnFirst : kTurnStep;
		if (r >= kGrindRim && right)
			want_rot = std::min(rot + 1, 3), want_tip = tip == 0 && rot == 0 ? 1 : tip;
		else if (r >= kGrindRim && left && tip == 0 && rot == 0)
			want_rot = -2, turn_need = kTurnFirst + 2 * kTurnStep; // (off both trucks straight round into a lipslide)
		else if (r >= kGrindRim && left)
			want_rot = std::max(rot - 1, -2), want_tip = tip,
			turn_need = rot == 0 && tip < 0 ? kTurnFirst * 1.5f : turn_need; // (the hard way round off the tail)
		else if (r >= 0.45f && (right || left) && rot == 0)
			want_tip = 0;
		else if (up)
			want_tip = 1;
		else if (down)
			want_tip = -1;
		// the left stick held right over to a side and the right stick not moving it: the board leans
		// that way a step (as the port: off both trucks onto the nose that way, the tail the other; then
		// one step round - crooked, salad, overcrook - no further)
		const float lx = switch_ ? -in.steer[0] : in.steer[0];
		if (want_tip == tip && want_rot == rot && std::fabs(lx) > 0.6f) {
			if (tip == 0 && rot == 0) {
				if (std::fabs(in.steer[1]) < 0.4f) // (straight over, not a diagonal: as the port)
					want_tip = lx > 0 ? 1 : -1, turn_need = kLeanTip;
			}
			else if (lx > 0 && rot == 0)
				want_rot = 1, turn_need = kTurnFirst;
			else if (lx < 0 && rot == 0 && tip > 0)
				want_rot = -1, turn_need = kTurnFirst;
			grind_lean_ = want_tip != tip || want_rot != rot;
		} else {
			grind_lean_ = false;
		}
		if (rot >= 3 || rot <= -2)
			want_tip = tip; // (across in a slide: no truck to tip onto)
		int want = grind_of_pose(want_rot != rot && rot == 0 && want_tip != tip ? want_tip : want_tip, want_rot);
		if (want_rot == rot && want_tip == tip)
			want = grind_kind_;
		if (want != grind_kind_) {
			grind_transfer_ += kDt;
			// (onto a truck ~10 steps; the first turn ~17, then ~8 a step)
			float need = want_rot == rot && !grind_lean_ ? kTipTime : turn_need;
			if (grind_transfer_ >= need && grind_time_ >= kGrindSettle)
				change_to(want);
		} else {
			grind_transfer_ = 0;
		}
	} else if (grind_kind_ != SK8_GRIND_DARKSLIDE && grind_slide_ &&
		in.flick[0] * in.flick[0] + in.flick[1] * in.flick[1] > 0.2f) {
		int want = grind_kind_for(true, fx, fy, false);
		if (slide_family_stick(grind_lip_, fx, fy))
			want = slide_kind(grind_lip_, fx, fy);
		if (std::fabs(fx) >= 0.45f && std::fabs(fy) < 0.45f)
			want = grind_kind_; // (held on across, the way it turned onto the rail: it stays)
		if (want != grind_kind_) {
			grind_transfer_ += kDt;
			if (grind_transfer_ >= kGrindTransfer && grind_time_ >= kGrindSettle)
				change_to(want);
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
		balance_ += (noise() * p[GrindBalance] + balance_ * 0.8f - steer_pad_[0] * 2.2f) * kDt;
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
	if (mode_ == SK8_MODE_GRIND && catch_body_ != 0)
		stance = stance * Q::axis_angle({0, 1, 0}, catch_body_);
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
	if (kickturn_ > 0) // a kickturn: the nose up off the ground, pivoting on the back wheels
		board = board * Q::axis_angle({1, 0, 0}, -std::min(kickturn_ / 0.15f, 1.0f) * 0.32f);
	if (mode_ == SK8_MODE_AIR && (adjust_[0] != 0 || adjust_[1] != 0)) // a board adjust: nose up / swung
		board = board * Q::axis_angle({0, 1, 0}, -adjust_[0] * kAdjustYaw) * Q::axis_angle({1, 0, 0}, -adjust_[1] * kAdjustPitch);
	if (mode_ == SK8_MODE_MANUAL && manual_style_ == 1)
		board = board * Q::axis_angle({0, 0, 1}, (switch_ ? -kPi : kPi) / 2); // primo: on its side, heel edge up
	else if (mode_ == SK8_MODE_MANUAL && manual_style_ == 2)
		board = board * Q::axis_angle({0, 0, 1}, kPi) * Q::axis_angle({1, 0, 0}, 0.38f); // casper: upside down, tail up
	else if (mode_ == SK8_MODE_MANUAL)
		board = board * Q::axis_angle({1, 0, 0}, -manual_tilt_ * kDeg * float(manual_dir_));
	if (mode_ == SK8_MODE_AIR && varial_ != 0)
		board = board * Q::axis_angle({0, 1, 0}, -varial_); // the board stays while the body turns
	if (mode_ == SK8_MODE_GRIND && catch_board_ != 0)
		board = board * Q::axis_angle({0, 1, 0}, catch_board_);
	if (mode_ == SK8_MODE_GRIND && grind_kind_ > 0) {
		// the board sits on the rail the way this grind holds it, the rail under the part it
		// rides on (local x right = heel edge, y up, z the nose)
		const GrindDef &g = kGrindDefs[grind_tip_ ? SK8_GRIND_5_0 : grind_kind_];
		float side = switch_ ? -1.0f : 1.0f; // the heel edge is on the other side in switch
		// (a 5-0 / nosegrind tips as far as the stick says: see grind_tilt_)
		float pitch = tip_kind_ >= 0 && !grind_tip_ ? (g.pitch < 0 ? -grind_tilt_ : grind_tilt_) : g.pitch;
		board = board * Q::axis_angle({0, 1, 0}, -g.yaw * side * kDeg) * Q::axis_angle({1, 0, 0}, -pitch * kDeg) *
			Q::axis_angle({0, 0, 1}, g.roll * side * kDeg);
		(pos_ - fwd * g.along).store(s.position);
	}
	board.store(s.board_rot);

	s.mode = mode_;
	s.fakie = fakie_;
	s.crouch = std::max(crouch_, crouch_look_);
	s.lean = lean_;
	s.push_phase = push_time_ >= push_lead_ ? clampf((push_time_ - push_lead_) / std::max(p[PushPeriod], 0.01f), 0, 1) : -1;
	s.push_start = push_time_ >= 0 && push_time_ < push_lead_ ? push_time_ / push_lead_ : -1;
	s.push_mongo = push_mongo_;
	s.foot_off = foot_off_;
	s.varial = mode_ == SK8_MODE_AIR ? varial_ : 0;
	s.revert = revert_time_ >= 0 ? clampf(revert_time_ / 0.34f, 0, 1) : -1;
	s.grab = ground_mode(mode_) ? ground_grab_ : grab_;
	s.air_feet = mode_ == SK8_MODE_AIR ? air_feet_ : 0;
	s.dark = dark_ ? 1 : 0;
	s.kickturn = kickturn_;
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

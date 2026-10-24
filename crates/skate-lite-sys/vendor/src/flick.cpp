// Flick recogniser and trick catalogue (written from scratch; see flick.hpp).
#include "flick.hpp"
#include "vec.hpp"
#include <algorithm>
#include <cmath>
#include <cstring>

namespace sk8 {

namespace {

// Gesture vocabulary authored for this engine. Down then up through the centre is the
// basic pop; sweeping along the rim spins the deck; ending on a diagonal flips it.
const std::vector<TrickDef> kTricks = {
	{"Ollie", {TDown, TCentre, TUp}, 80, 0, 0, false, true},
	{"Nollie", {TUp, TCentre, TDown}, 120, 0, 0, true, true},
	// Which flick does what follows the port (the faithful rewrite), found by trying every stick
	// path in it (regular stance): a flick goes by where it gets to - up-right a kickflip, up-left
	// a heelflip - while a slow sweep round the rim spins the board (a shove-it), and doubling back
	// past the crouch adds the 360. Sectors only passed through on the way (under kSweepDwell
	// ticks) don't count; see kAliases for the other ways in.
	{"Kickflip", {TDown, TCentre, TUpRight}, 240, 1, 0, false, false},
	{"Heelflip", {TDown, TCentre, TUpLeft}, 260, -1, 0, false, false},
	{"Pop Shove-it", {TDown, TDownRight, TRight, TUp}, 180, 0, 0.5f, false, false},
	{"Frontside Shove-it", {TDown, TDownLeft, TLeft, TCentre}, 200, 0, -0.5f, false, false},
	{"Tre Flip", {TDown, TLeft, TDown, TRight}, 520, 1, 1, false, false},
	{"Laser Flip", {TDown, TRight, TDown, TUpLeft}, 560, -1, -1, false, false},
	{"Varial Kickflip", {TDown, TDownRight, TCentre, TUp}, 360, 1, 0.5f, false, false},
	{"Varial Heelflip", {TDown, TDownLeft, TCentre, TUp}, 380, -1, -0.5f, false, false},
	{"Hardflip", {TDown, TDownLeft, TCentre, TUpRight}, 420, 1, -0.5f, false, false},
	{"Inward Heelflip", {TDown, TDownRight, TCentre, TUpLeft}, 440, -1, 0.5f, false, false},
	{"360 Shove-it", {TDown, TDownLeft, TDown, TRight}, 340, 0, 1, false, false},
	{"Frontside 360 Shove-it", {TDown, TRight, TDown, TLeft}, 360, 0, -1, false, false},
	{"360 Hardflip", {TDown, TRight, TDown, TUp}, 600, 1, -1, false, false},
	{"360 Inward Heelflip", {TDown, TLeft, TDown, TUpLeft}, 620, -1, 1, false, false},
	{"Nollie Kickflip", {TUp, TCentre, TDownRight}, 300, 1, 0, true, false},
	{"Nollie Heelflip", {TUp, TCentre, TDownLeft}, 320, -1, 0, true, false},
	{"Nollie Shove-it", {TUp, TUpRight, TDownRight}, 220, 0, 0.5f, true, false},
	{"Nollie Frontside Shove-it", {TUp, TUpLeft, TDownLeft}, 240, 0, -0.5f, true, false},
	{"Nollie Varial Kickflip", {TUp, TUpRight, TCentre, TDown}, 420, 1, 0.5f, true, false},
	{"Nollie Varial Heelflip", {TUp, TUpLeft, TCentre, TDown}, 440, -1, -0.5f, true, false},
	{"Nollie Hardflip", {TUp, TLeft, TCentre, TDownRight}, 480, 1, -0.5f, true, false},
	{"Nollie Inward Heelflip", {TUp, TUpRight, TDownLeft}, 500, -1, 0.5f, true, false},
	{"Nollie Tre Flip", {TUp, TLeft, TUp, TDownRight}, 620, 1, 1, true, false},
	{"Nollie 360 Shove-it", {TUp, TLeft, TUp, TRight}, 400, 0, 1, true, false},
	{"Nollie Frontside 360 Shove-it", {TUp, TRight, TUp, TLeft}, 420, 0, -1, true, false},
	{"Nollie 360 Hardflip", {TUp, TRight, TUp, TDownRight}, 680, 1, -1, true, false},
	// ours (the port has no flick for it): down, back through the centre, down again
	{"Impossible", {TDown, TCentre, TDown}, 480, 0, 0, false, false, 1},
	// Old school: no gesture of their own, the foot-off button turns a flick into one (see
	// foot_off_variant). Kept after the gestures; sk8_trick_count() covers only those.
	{"No Comply", {}, 160, 0, 0, false, true},
	{"Boneless", {}, 200, 0, 0, false, true},
	// a grab trigger held and push pressed on the ground (no flick): the boneless wind-up; the left
	// stick pushed up on the way, a fastplant
	{"Fastplant", {}, 220, 0, 0, false, true},
	{"No Comply 180", {}, 260, 0, 0.5f, false, false},
	{"Frontside No Comply 180", {}, 280, 0, -0.5f, false, false},
	{"Hippie Jump", {}, 150, 0, 0, false, true},
};

// The other ways into a trick, as the port takes them.
const struct
{
	std::vector<unsigned char> tokens;
	const char *name;
} kAliases[] = {
	{{TDown, TUp}, "Ollie"}, // straight across, too quick for a centre sample
	{{TUp, TDown}, "Nollie"},
	{{TDown, TUpRight}, "Kickflip"},
	{{TDown, TRight, TUp}, "Kickflip"},
	{{TDown, TRight, TUpRight}, "Kickflip"},
	{{TDown, TRight, TUpLeft}, "Kickflip"},
	{{TDown, TDownRight, TUpRight}, "Kickflip"},
	{{TDown, TUpLeft}, "Heelflip"},
	{{TDown, TLeft, TUp}, "Heelflip"},
	{{TDown, TLeft, TUpLeft}, "Heelflip"},
	{{TDown, TLeft, TUpRight}, "Heelflip"},
	{{TDown, TLeft, TRight}, "Heelflip"},
	{{TDown, TDownLeft, TUpLeft}, "Heelflip"},
	{{TDown, TDownLeft, TLeft}, "Frontside Shove-it"},
	{{TDown, TDownLeft, TLeft, TUp}, "Frontside Shove-it"},
	{{TDown, TDownRight, TRight, TUpRight}, "Pop Shove-it"},
	{{TDown, TDownRight, TUp}, "Varial Kickflip"},
	{{TDown, TDownLeft, TUp}, "Varial Heelflip"},
	{{TDown, TDownRight, TLeft}, "Varial Heelflip"},
	{{TDown, TDownRight, TUpLeft}, "Varial Heelflip"},
	{{TDown, TLeft, TCentre, TUpRight}, "Hardflip"},
	{{TDown, TRight, TCentre, TUpLeft}, "Inward Heelflip"},
	{{TDown, TLeft, TDown, TUpRight}, "Tre Flip"},
	{{TDown, TRight, TDownRight, TUpLeft}, "Laser Flip"},
	{{TDown, TDownRight, TDown, TLeft}, "Frontside 360 Shove-it"},
	{{TDown, TRight, TDown, TUpRight}, "360 Hardflip"},
	{{TDown, TLeft, TDownRight, TUpLeft}, "360 Inward Heelflip"},
	{{TUp, TDownRight}, "Nollie Kickflip"},
	{{TUp, TRight, TDown}, "Nollie Kickflip"},
	{{TUp, TRight, TDownRight}, "Nollie Kickflip"},
	{{TUp, TRight, TDownLeft}, "Nollie Kickflip"},
	{{TUp, TUpLeft, TDownRight}, "Nollie Kickflip"},
	{{TUp, TDownLeft}, "Nollie Heelflip"},
	{{TUp, TLeft, TDown}, "Nollie Heelflip"},
	{{TUp, TLeft, TDownLeft}, "Nollie Heelflip"},
	{{TUp, TLeft, TDownRight}, "Nollie Heelflip"},
	{{TUp, TUpRight, TRight, TDown}, "Nollie Shove-it"},
	{{TUp, TUpLeft, TLeft, TDown}, "Nollie Frontside Shove-it"},

	{{TUp, TUpLeft, TCentre, TDownRight}, "Nollie Hardflip"},
	{{TUp, TRight, TCentre, TDownLeft}, "Nollie Inward Heelflip"},
	{{TUp, TLeft, TUpLeft, TDownRight}, "Nollie Tre Flip"},
	{{TUp, TRight, TUpLeft, TDownRight}, "Nollie 360 Hardflip"},
};

int ring_distance(int a, int b);

// One rule: a token string and the trick it is (kTricks' own, then the aliases).
struct Rule
{
	std::vector<unsigned char> tokens;
	int trick;
};

std::vector<unsigned char> as_heard(const std::vector<unsigned char> &t) { return t; }

const std::vector<Rule> &rules()
{
	static const std::vector<Rule> r = [] {
		std::vector<Rule> out;
		for (size_t i = 0; i < kTricks.size(); i++)
			if (!kTricks[i].tokens.empty())
				out.push_back({as_heard(kTricks[i].tokens), int(i)});
		// (an alias heard the same as an earlier rule - a jump across heard as through the centre -
		// gives way to it)
		for (const auto &a : kAliases)
			for (size_t i = 0; i < kTricks.size(); i++)
				if (std::strcmp(kTricks[i].name, a.name) == 0) {
					auto t = as_heard(a.tokens);
					if (std::none_of(out.begin(), out.end(), [&](const Rule &r) { return r.tokens == t; }))
						out.push_back({t, int(i)});
				}
		return out;
	}();
	return r;
}

// base gesture trick -> old-school variant with the foot-off button held
const struct
{
	const char *base, *variant;
	float pop; // share of an ollie's pop
} kFootOff[] = {
	{"Ollie", "No Comply", 0.8f},
	{"Nollie", "Boneless", 0.88f},
	{"Pop Shove-it", "No Comply 180", 0.82f},
	{"Frontside Shove-it", "Frontside No Comply 180", 0.82f},
};

bool is_prefix(const std::vector<unsigned char> &a, const std::vector<unsigned char> &b)
{
	if (a.size() > b.size())
		return false;
	return std::equal(a.begin(), a.end(), b.begin());
}

int ring_distance(int a, int b)
{
	int d = std::abs(a - b) % 8;
	return d > 4 ? 8 - d : d;
}

} // namespace

const std::vector<TrickDef> &trick_table() { return kTricks; }

int gesture_trick_count()
{
	int n = 0;
	while (n < int(kTricks.size()) && !kTricks[size_t(n)].tokens.empty())
		n++;
	return n;
}

int foot_off_variant(int trick, float *pop)
{
	if (trick < 0)
		return -1;
	for (const auto &f : kFootOff)
		if (std::strcmp(kTricks[size_t(trick)].name, f.base) == 0) {
			if (pop)
				*pop = f.pop;
			return find_trick(f.variant);
		}
	return -1;
}

int find_trick(const char *name)
{
	for (size_t i = 0; i < kTricks.size(); i++)
		if (std::strcmp(kTricks[i].name, name) == 0)
			return int(i);
	return -1;
}

// Up is wider toward the kick side than the heel side: in the port (a recorded session, 36 flicks)
// a flick ending up to ~38 degrees right of up was still an ollie, one ~18 degrees left already a
// heelflip. The other sectors stay 45 degrees, centred on their directions.
constexpr float kUpHeelEdge = -18.5f, kUpKickEdge = 38.5f; // degrees from up

constexpr int kRevise = 6;
constexpr int kCrouchHeld = 12; // steps on one rim sector before a sweep: that's the crouch // steps after a trick the stick's carrying on can still change it

int sector_of(float x, float y)
{
	float a = std::atan2(x, y); // 0 = up, clockwise positive
	float deg = a * 180 / kPi;
	if (deg >= kUpHeelEdge && deg < kUpKickEdge)
		return 0;
	if (deg >= kUpKickEdge && deg < 67.5f)
		return 1;
	if (deg >= -67.5f && deg < kUpHeelEdge)
		return 7;
	int s = int(std::lround(a / (kPi / 4)));
	return ((s % 8) + 8) % 8;
}

void Flick::reset()
{
	tokens_.clear();
	raw_.clear();
	pending_ = -1;
	on_rim_ = false;
	centre_steps_ = 0;
	last_sector_ = -1;
}

void Flick::push(unsigned char t)
{
	if (!tokens_.empty() && tokens_.back() == t)
		return;
	if (t == TCentre && tokens_.empty())
		return;
	tokens_.push_back(t);
}

Flick::Result Flick::resolve(bool ending)
{
	Result r;
	if (tokens_.empty())
		return r;
	bool extendable = false;
	int exact = -1;
	for (const Rule &rule : rules()) {
		if (rule.tokens == tokens_)
			exact = rule.trick;
		else if (is_prefix(tokens_, rule.tokens))
			extendable = true;
	}
	if (exact >= 0 && !(extendable && !ending)) {
		pending_ = -1;
		r.trick = exact;
		return r;
	}
	if (exact >= 0) { // complete, but a longer gesture starts the same way: wait for the next move
		pending_ = exact;
		return r;
	}
	if (pending_ >= 0 && (!extendable || ending || tokens_.back() == TCentre)) {
		// went on somewhere that isn't the longer gesture (or let go): the shorter one it was
		r.trick = pending_;
		pending_ = -1;
		return r;
	}
	if (extendable && !ending)
		return r;
	// Not a known string: as the port, it goes by where the flick got to. Touching the far
	// diagonal flips it that way (kickflip up-right, heelflip up-left; mirrored off the nose), and
	// crossing over to the far side straight after a turn to one side varials it. Anything else
	// pops nothing.
	if (tokens_.size() >= 2) {
		bool pop = tokens_[0] == TDown, nol = tokens_[0] == TUp;
		size_t n = tokens_.size();
		unsigned char last = tokens_[n - 1];
		auto ends = [&](unsigned char a, unsigned char b, unsigned char c) {
			return n >= 4 && tokens_[n - 3] == a && tokens_[n - 2] == b && tokens_[n - 1] == c;
		};
		auto ends2 = [&](unsigned char a, unsigned char b) {
			return n >= 3 && tokens_[n - 2] == a && tokens_[n - 1] == b;
		};
		const char *name = nullptr;
		if (pop && last == TUpRight) name = "Kickflip";
		else if (pop && last == TUpLeft) name = "Heelflip";
		else if (pop && (ends(TDownLeft, TCentre, TUp) || ends2(TDownLeft, TUp))) name = "Varial Heelflip";
		else if (pop && (ends(TDownRight, TCentre, TUp) || ends2(TDownRight, TUp))) name = "Varial Kickflip";
		else if (nol && last == TDownRight) name = "Nollie Kickflip";
		else if (nol && last == TDownLeft) name = "Nollie Heelflip";
		else if (nol && (ends(TUpRight, TCentre, TDown) || ends2(TUpRight, TDown))) name = "Nollie Varial Kickflip";
		else if (nol && (ends(TUpLeft, TCentre, TDown) || ends2(TUpLeft, TDown))) name = "Nollie Varial Heelflip";
		if (name)
			r.trick = find_trick(name);
	}
	// garbage: start over from the next rim touch (unless the stick is only passing through the
	// newest sector: that one may yet drop out)
	if (r.trick < 0 && !extendable && !young_)
		reset();
	return r;
}

Flick::Result Flick::feed(float x, float y)
{
	float r = std::sqrt(x * x + y * y);
	Result out;
	if (revise_steps_ > 0) {
		revise_steps_--;
		if (r >= kRimIn) {
			int s = sector_of(x, y);
			if (!raw_.empty() && raw_.back().first == s)
				raw_.back().second++;
			else {
				raw_.push_back({(unsigned char)s, 1});
				push((unsigned char)s);
				Result again = swept_shove();
				if (again.trick >= 0 && again.trick != revise_trick_) {
					out.revised = revise_trick_ = again.trick;
				}
			}
		}
		if (revise_steps_ == 0 || r < kCentreIn) {
			revise_steps_ = 0;
			reset();
			spent_ = r >= kCentreIn;
		}
		return out;
	}
	if (spent_) {
		if (r < kCentreIn)
			spent_ = false;
		return out;
	}
	if (r >= (tokens_.empty() ? kLeadIn : kRimIn)) {
		int s = sector_of(x, y);
		if (last_sector_ >= 0 && s != last_sector_ && !tokens_.empty() && tokens_.back() == last_sector_ &&
			tokens_.size() >= 2 && rim_steps_ + 1 < kSweepDwell &&
			(ring_distance(s, last_sector_) <= 2 || tokens_.size() >= 3)) {
			// only passed through, sliding on round the rim (a quick sweep): it doesn't count. (Jumped
			// across straight from it as the first move off the crouch it does: the way the flick
			// went before it crossed - a varial.)
			tokens_.pop_back();
		}
		// (jumped straight across without a centre sample: no centre token - the port tells a quick
		// flick across from one that stops in the middle)
		rim_steps_ = s == last_sector_ ? rim_steps_ + 1 : 0;
		young_ = rim_steps_ + 1 < kSweepDwell && tokens_.size() >= 1;
		if (!raw_.empty() && raw_.back().first == s)
			raw_.back().second++;
		else
			raw_.push_back({(unsigned char)s, 1});
		arc_centre_ = 0;
		if (!arc_.empty() && arc_.back().first == s)
			arc_.back().second++;
		else
			arc_.push_back({(unsigned char)s, 1});
		push((unsigned char)s);
		last_sector_ = s;
		on_rim_ = true;
		centre_steps_ = 0;
		out = crouch_arc();
		if (out.trick < 0)
			out = swept_shove();
		if (out.trick < 0)
			out = resolve(false);
		// A shove-it's sweep held at its sideways end (instead of let go to the centre): it pops
		// as the shove-it, and kept there it turns on (held: 360, 540)
		if (out.trick < 0 && rim_steps_ >= 9) {
			// (the diagonal on the way there counts or not: swept quickly through, it doesn't)
			static const struct { unsigned char a, b, c; const char *name; } held[] = {
				{TDown, TDownRight, TRight, "Pop Shove-it"}, {TDown, TDownLeft, TLeft, "Frontside Shove-it"},
				{TUp, TUpRight, TRight, "Nollie Shove-it"}, {TUp, TUpLeft, TLeft, "Nollie Frontside Shove-it"}};
			for (const auto &h : held)
				if ((tokens_.size() == 3 && tokens_[0] == h.a && tokens_[1] == h.b && tokens_[2] == h.c) ||
					(tokens_.size() == 2 && tokens_[0] == h.a && tokens_[1] == h.c))
					out.trick = find_trick(h.name);
		}
	} else if (r < kCentreIn) {
		on_rim_ = false;
		young_ = false;
		if (++arc_centre_ >= 3) // let go (not just crossing the middle in a sweep)
			arc_.clear();
		if (!tokens_.empty()) {
			push(TCentre);
			last_sector_ = -1;
			out = resolve(false);
			// (let go this long, the flick is over: the port still pops after 10 ticks, not 11)
			if (out.trick < 0 && ++centre_steps_ > 10) {
				bool lone = tokens_.size() == 2 && tokens_[0] == TDown && tokens_[1] == TCentre;
				out = resolve(true);
				if (out.trick < 0) {
					out.crouch_let_go = lone;
					reset();
				}
			}
		}
	}
	if (out.trick >= 0) {
		arc_.clear();
		if (r >= kRimIn) { // still on the rim: it may carry on round (read again for a few steps)
			revise_steps_ = kRevise;
			revise_trick_ = out.trick;
		} else {
			reset();
			spent_ = r >= kCentreIn;
		}
	}
	return out;
}

// A crouch held somewhere other than straight down (to the left, down-left, down-right, right), then
// the stick swept round the rim from there: named by where it has swept to, the moment it gets there,
// as the port names them (probed with the port on swept paths; the tre / varial kickflip / 360 inward
// heelflip ones also from a recorded session, where the crouch was held to the left).
Flick::Result Flick::crouch_arc()
{
	Result out;
	const auto &raw = arc_;
	if (raw.size() < 3 || raw[0].second < kCrouchHeld)
		return out;
	const unsigned char c = raw[0].first;
	std::vector<unsigned char> path;
	for (size_t i = 1; i < raw.size(); i++)
		path.push_back(raw[i].first);
	// `seq` passed through in order (others may come between)
	auto went = [&](std::initializer_list<unsigned char> seq) {
		size_t k = 0;
		for (unsigned char x : path)
			if (k < seq.size() && x == *(seq.begin() + k))
				k++;
		return k == seq.size();
	};
	unsigned char last = path.back();
	const char *name = nullptr;
	if (c == TLeft) {
		if (went({TDownLeft, TDown, TDownRight, TDown}) && (last == TUp || last == TUpRight))
			name = "360 Inward Heelflip";
		else if (went({TDownLeft, TDown, TDownRight, TRight}) || went({TDownLeft, TDown, TDownRight, TUpRight}))
			name = "Tre Flip"; // (on round the right, or across to up-right)
		else if (went({TDownLeft, TDown}) && last == TUp)
			name = "Varial Kickflip"; // (round to down, then across to straight up)
		else if (went({TUpLeft, TUp, TUpRight, TRight}))
			name = "Nollie 360 Shove-it";
	} else if (c == TDownLeft) {
		if (went({TDown, TDownRight, TRight}))
			name = "360 Shove-it";
		else if (went({TLeft, TUpLeft, TUp}))
			name = "Ollie";
	} else if (c == TDownRight) {
		if (went({TDown, TDownLeft, TLeft}))
			name = "Frontside Shove-it";
		else if (went({TRight, TUpRight, TUp}))
			name = "Ollie";
	} else if (c == TRight) {
		if (went({TDownRight, TDown, TDownLeft, TLeft}))
			name = "Frontside Shove-it";
		else if (went({TUpRight, TUp, TUpLeft, TLeft}))
			name = "Nollie Frontside 360 Shove-it";
	} else if (c == TDown) {
		// swept (not held on the way) from down round the left past left and on up: a frontside
		// shove-it (back down from left instead, it's on its way to a tre)
		bool swept = raw.size() >= 4 && raw[1].first == TDownLeft && raw[1].second <= 3 && raw[2].first == TLeft &&
			(raw[3].first == TUpLeft || raw[3].first == TUp);
		if (swept)
			name = "Frontside Shove-it";
	}
	if (name)
		out.trick = find_trick(name);
	return out;
}

// A quick sweep round the rim from the crouch to the other end (down round a side to up, or up
// round a side to down for a nollie), each sector on the way passed straight through: a shove-it,
// the way the port reads one (in a recorded session the fast sweeps were shove-its, even though no
// sector on the way was held). Through the right side, the pop shove-it; the left, frontside.
Flick::Result Flick::swept_shove()
{
	Result out;
	if (raw_.size() < 4)
		return out;
	unsigned char end = raw_.back().first;
	if (end != TUp && end != TDown)
		return out;
	unsigned char from = end == TUp ? TDown : TUp;
	// back from the end: the arc of quickly passed sectors, then the crouch it started from
	int n = int(raw_.size()) - 1, i = n - 1, dir = 0, steps = 0;
	for (; i >= 0; i--) {
		unsigned char s = raw_[size_t(i)].first;
		if (s == from)
			break;
		int d = ((raw_[size_t(i) + 1].first - s) + 8) % 8; // around from this sector to the next
		int way = d == 1 ? 1 : d == 7 ? -1 : 0;
		// (the sectors on the way passed in a step each - slower, the port reads a kickflip; the last
		// one before the end may be held a moment)
		int allow = i == n - 1 ? kSweepDwell : 1;
		if (way == 0 || (dir != 0 && way != dir) || raw_[size_t(i)].second > allow)
			return out;
		dir = way;
		steps++;
	}
	if (i < 0 || steps < 2 || raw_[size_t(i)].second < 3) // a crouch held, then at least two passed through
		return out;
	if (((raw_[size_t(i) + 1].first - from + 8) % 8) != (dir == 1 ? 1 : 7))
		return out;
	// clockwise from down (round the right) is the pop shove-it; for a nollie, round the right too
	// (sectors count clockwise from up: down round the right is down, down-right, right... falling)
	bool right = end == TUp ? dir == -1 : dir == 1;
	const char *name = end == TUp ? (right ? "Pop Shove-it" : "Frontside Shove-it")
								  : (right ? "Nollie Shove-it" : "Nollie Frontside Shove-it");
	out.trick = find_trick(name);
	return out;
}

const char *classify_path(const float *xy, int count)
{
	Flick f;
	for (int i = 0; i < count; i++) {
		Flick::Result r = f.feed(xy[2 * i], xy[2 * i + 1]);
		if (r.trick >= 0)
			return kTricks[size_t(r.trick)].name;
	}
	// let a trailing release resolve
	for (int i = 0; i < 16; i++) {
		Flick::Result r = f.feed(0, 0);
		if (r.trick >= 0)
			return kTricks[size_t(r.trick)].name;
	}
	return nullptr;
}

int demo_path(int trick, float *xy, int max)
{
	if (trick < 0 || size_t(trick) >= kTricks.size())
		return 0;
	int n = 0;
	auto put = [&](float x, float y) {
		if (n < max) {
			xy[2 * n] = x;
			xy[2 * n + 1] = y;
			n++;
		}
	};
	auto rim = [&](float sector) {
		float a = sector * kPi / 4;
		put(std::sin(a), std::cos(a));
	};
	const auto &t = kTricks[size_t(trick)].tokens;
	int prev = -1;
	for (unsigned char tok : t) {
		if (tok == TCentre) {
			put(0, 0);
			put(0, 0);
			prev = -1;
			continue;
		}
		if (prev >= 0 && ring_distance(tok, prev) >= 3) {
			// straight across the stick (quick: no sample in the middle)
		} else if (prev >= 0) { // sweep along the rim
			int d = ((tok - prev + 8) % 8) <= 4 ? 1 : -1;
			for (int k = 1; k <= 3; k++)
				rim(prev + d * k / 4.0f);
		} else {
			put(0, 0);
		}
		for (int k = 0; k <= kSweepDwell; k++) // held long enough to count
			rim(tok);
		prev = tok;
	}
	return n;
}

} // namespace sk8

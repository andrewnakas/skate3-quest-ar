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
	{"Kickflip", {TDown, TCentre, TUpLeft}, 240, 1, 0, false, false},
	{"Heelflip", {TDown, TCentre, TUpRight}, 260, -1, 0, false, false},
	{"Pop Shove-it", {TDown, TDownLeft, TLeft, TCentre}, 180, 0, 0.5f, false, false},
	{"Frontside Shove-it", {TDown, TDownRight, TRight, TCentre}, 200, 0, -0.5f, false, false},
	{"Tre Flip", {TDown, TDownLeft, TLeft, TUpLeft}, 520, 1, 1, false, false},
	{"Laser Flip", {TDown, TDownRight, TRight, TUpRight}, 560, -1, -1, false, false},
	{"Varial Kickflip", {TDown, TDownLeft, TCentre, TUpRight}, 360, 1, 0.5f, false, false},
	{"Varial Heelflip", {TDown, TDownRight, TCentre, TUpLeft}, 380, -1, -0.5f, false, false},
	{"Hardflip", {TDown, TDownRight, TCentre, TUp}, 420, 1, -0.5f, false, false},
	{"Inward Heelflip", {TDown, TDownLeft, TCentre, TUp}, 440, -1, 0.5f, false, false},
	{"Nollie Kickflip", {TUp, TCentre, TDownLeft}, 300, 1, 0, true, false},
	{"Nollie Heelflip", {TUp, TCentre, TDownRight}, 320, -1, 0, true, false},
	{"Nollie Shove-it", {TUp, TUpLeft, TLeft, TCentre}, 220, 0, 0.5f, true, false},
	{"Nollie Frontside Shove-it", {TUp, TUpRight, TRight, TCentre}, 240, 0, -0.5f, true, false},
	// Old school: no gesture of their own, the foot-off button turns a flick into one (see
	// foot_off_variant). Kept after the gestures; sk8_trick_count() covers only those.
	{"No Comply", {}, 160, 0, 0, false, true},
	{"Boneless", {}, 200, 0, 0, false, true},
	{"No Comply 180", {}, 260, 0, 0.5f, false, false},
	{"Frontside No Comply 180", {}, 280, 0, -0.5f, false, false},
};

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

int sector_of(float x, float y)
{
	float a = std::atan2(x, y); // 0 = up, clockwise positive
	int s = int(std::lround(a / (kPi / 4)));
	return ((s % 8) + 8) % 8;
}

void Flick::reset()
{
	tokens_.clear();
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
	for (size_t i = 0; i < kTricks.size(); i++) {
		if (kTricks[i].tokens == tokens_) {
			r.trick = int(i);
			return r;
		}
		if (is_prefix(tokens_, kTricks[i].tokens))
			extendable = true;
	}
	if (extendable && !ending)
		return r;
	// Not a known string. A crouch followed by a flick through the centre still pops.
	if (tokens_.size() >= 3 && tokens_[1] == TCentre) {
		r.trick = tokens_[0] == TDown ? 0 : tokens_[0] == TUp ? 1 : -1;
		r.fallback = r.trick >= 0;
	}
	if (r.trick < 0 && !extendable)
		reset(); // garbage: start over from the next rim touch
	return r;
}

Flick::Result Flick::feed(float x, float y)
{
	float r = std::sqrt(x * x + y * y);
	Result out;
	if (spent_) {
		if (r < kCentreIn)
			spent_ = false;
		return out;
	}
	if (r >= kRimIn) {
		int s = sector_of(x, y);
		if (last_sector_ >= 0 && !tokens_.empty() && tokens_.back() != TCentre) {
			int d = ring_distance(s, last_sector_);
			if (d >= 3)
				push(TCentre); // jumped across without a centre sample
			else if (d == 2) {
				// fast sweep skipped a sector: fill it in on the short way round
				int step = ((s - last_sector_ + 8) % 8) <= 4 ? 1 : -1;
				push((unsigned char)((last_sector_ + step + 8) % 8));
			}
		}
		push((unsigned char)s);
		last_sector_ = s;
		on_rim_ = true;
		centre_steps_ = 0;
		out = resolve(false);
	} else if (r < kCentreIn) {
		on_rim_ = false;
		if (!tokens_.empty()) {
			push(TCentre);
			last_sector_ = -1;
			out = resolve(false);
			if (out.trick < 0 && ++centre_steps_ > 12) {
				out = resolve(true);
				if (out.trick < 0)
					reset();
			}
		}
	}
	if (out.trick >= 0) {
		reset();
		spent_ = r >= kCentreIn;
	}
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
		if (prev >= 0) { // sweep along the rim
			int d = ((tok - prev + 8) % 8) <= 4 ? 1 : -1;
			for (int k = 1; k <= 3; k++)
				rim(prev + d * k / 4.0f);
		} else {
			put(0, 0);
		}
		for (int k = 0; k < 4; k++)
			rim(tok);
		prev = tok;
	}
	return n;
}

} // namespace sk8

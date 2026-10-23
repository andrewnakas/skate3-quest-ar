// Flick recogniser and trick catalogue for the clean-room engine (written from scratch).
//
// The right stick is read as a string of tokens: eight rim sectors (0 = up, clockwise:
// 1 up-right, 2 right, 3 down-right, 4 down, 5 down-left, 6 left, 7 up-left) and 8 for a
// pass through the centre. A trick is an exact token string; the recogniser fires the
// moment the string read so far equals a trick's.
#pragma once
#include <string>
#include <vector>

namespace sk8 {

enum Token : unsigned char { TUp = 0, TUpRight, TRight, TDownRight, TDown, TDownLeft, TLeft, TUpLeft, TCentre };

struct TrickDef
{
	const char *name;
	std::vector<unsigned char> tokens;
	unsigned points;
	float roll;   // deck flips about its long axis (revolutions; + = toe side leaves first)
	float shove;  // deck spins about its up axis (revolutions; + = clockwise seen from above)
	bool nollie;  // pops off the nose
	bool flat;    // a plain ollie/nollie (no flip to finish before landing)
	float wrap = 0; // deck turns end over end about its width (revolutions; + = nose up first)
};

const std::vector<TrickDef> &trick_table();
// tricks with a gesture of their own (the front of trick_table)
int gesture_trick_count();
// the old-school trick a gesture trick becomes with the foot-off button held (-1 none); its pop share
int foot_off_variant(int trick, float *pop);
int find_trick(const char *name);

// Rim / centre thresholds on stick radius.
// The rim sits far out so a part-way tilt is left for manuals (between kManualIn and the rim).
constexpr float kRimIn = 0.88f;
constexpr float kManualIn = 0.3f;
constexpr float kCentreIn = 0.30f;
// A rim sector the stick stays in fewer ticks than this was only swept through (doesn't count).
constexpr int kSweepDwell = 5;

int sector_of(float x, float y);

class Flick
{
public:
	struct Result
	{
		int trick = -1;     // index into trick_table(), -1 = nothing yet
		bool fallback = false;
		bool crouch_let_go = false; // held down (the crouch), let go to the centre and no flick followed
		int revised = -1;   // a trick already returned, read again as the stick carried on (see feed)
	};

	// Feed one stick sample. Returns a trick the moment one completes.
	Result feed(float x, float y);
	void reset();
	Result swept_shove();
	Result crouch_arc();
	// in the air after a pop: a new flick may start from wherever the stick is held (as the port:
	// held up after the ollie, then flicked down, a late backfoot flip)
	void rearm()
	{
		reset();
		spent_ = false;
		revise_steps_ = 0;
	}

	// The first token held so far (the crouch), or -1.
	int lead() const { return tokens_.empty() ? -1 : tokens_[0]; }
	bool holding_lead() const { return tokens_.size() == 1 && on_rim_; }
	const std::vector<unsigned char> &tokens() const { return tokens_; }
	// After a fire the stick must come back to the centre before a new gesture starts.
	bool spent() const { return spent_; }

private:
	void push(unsigned char t);
	Result resolve(bool ending);

	std::vector<unsigned char> tokens_;
	// every rim sector the stick has passed through since the last reset (no dwell filtering),
	// with how many steps it stayed in each: for quick sweeps round the rim
	std::vector<std::pair<unsigned char, int>> raw_;
	// the same since the stick left the centre, kept through the token resets (a crouch held off
	// to a side and swept from: crouch_arc)
	std::vector<std::pair<unsigned char, int>> arc_;
	int arc_centre_ = 0;
	bool on_rim_ = false;
	bool spent_ = false;
	// after a trick: a few more steps the stick may carry on (a sweep on round to the end) and the
	// trick be read again, as the port reads the whole flick by the takeoff
	int revise_steps_ = 0, revise_trick_ = -1;
	int centre_steps_ = 0;
	int rim_steps_ = 0; // steps the stick has stayed in one rim sector
	bool young_ = false; // the newest rim sector is only just reached (may be a sweep passing through)
	int last_sector_ = -1;
	// a finished gesture that's also the start of a longer one: fired when the stick goes on
	// somewhere else (or back to the centre) rather than straight away
	int pending_ = -1;
};

const char *classify_path(const float *xy, int count);
int demo_path(int trick, float *xy, int max);

} // namespace sk8

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

int sector_of(float x, float y);

class Flick
{
public:
	struct Result
	{
		int trick = -1;     // index into trick_table(), -1 = nothing yet
		bool fallback = false;
		bool crouch_let_go = false; // held down (the crouch), let go to the centre and no flick followed
	};

	// Feed one stick sample. Returns a trick the moment one completes.
	Result feed(float x, float y);
	void reset();

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
	bool on_rim_ = false;
	bool spent_ = false;
	int centre_steps_ = 0;
	int last_sector_ = -1;
	// a finished gesture that's also the start of a longer one: fired when the stick goes on
	// somewhere else (or back to the centre) rather than straight away
	int pending_ = -1;
};

const char *classify_path(const float *xy, int count);
int demo_path(int trick, float *xy, int max);

} // namespace sk8

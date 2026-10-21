/*
SK8.H — clean-room skateboarding engine (C++17 inside, C ABI outside).

Provenance: everything under clean/ was written from scratch on 2026-10-17 for the public
tier of the Quest AR app. It is not derived from the retail game, from src/ (the "lite"
engine) or from src/s3 (the faithful port): no retail data, constants, names, tables or
algorithms. Its feel is tuned by hand. scripts/cleanroom/audit.py in skate3-quest-ar checks it.

Space: metres, +Y up, right-handed. Board nose points along +Z when the heading is 0.
Time: always steps at 1/60 s (SK8_DT); hosts with slower frames call sk8_rider_step
several times per frame (that is also how pause, slow motion and frame step work).
*/

#ifndef SK8_H
#define SK8_H

#ifdef __cplusplus
extern "C" {
#endif

#define SK8_DT (1.0f / 60.0f)

typedef struct sk8_world sk8_world;
typedef struct sk8_rider sk8_rider;

/* ---------- collision supplied by the host */

typedef struct sk8_hit
{
	float point[3];
	float normal[3];  /* unit, facing the ray origin */
	float fraction;   /* 0..1 along delta */
	int surface;      /* host material id, passed through to events */
} sk8_hit;

/* Cast a segment from origin to origin + delta. Return nonzero and fill *hit on a hit. */
typedef int (*sk8_raycast_fn)(void *user, const float origin[3], const float delta[3], sk8_hit *hit);

sk8_world *sk8_world_new(sk8_raycast_fn raycast, void *user);
void sk8_world_delete(sk8_world *world);

/* Grind rails are host-authored polylines (3 floats per point, at least 2 points). */
int sk8_world_add_rail(sk8_world *world, const float *points, int point_count);
void sk8_world_clear_rails(sk8_world *world);
int sk8_world_rail_count(const sk8_world *world);

/* ---------- input, sampled once per step */

enum
{
	SK8_PUSH = 1 << 0,        /* kick-push */
	SK8_BRAKE = 1 << 1,       /* foot brake */
	SK8_GRAB_LEFT = 1 << 2,   /* hold in the air: grab with the left hand */
	SK8_GRAB_RIGHT = 1 << 3,  /* hold in the air: grab with the right hand */
	SK8_MANUAL = 1 << 4,      /* hold on the ground: manual (tail) */
	SK8_NOSE_MANUAL = 1 << 5, /* hold on the ground: nose manual */
	SK8_SLIDE = 1 << 6,       /* hold while steering: powerslide */
	SK8_PUSH_MONGO = 1 << 7,  /* kick-push with the front foot (mongo), whatever push_mongo says */
	SK8_OFF_BOARD = 1 << 9,   /* press: step off the board and walk; press again: back on it */
	SK8_FOOT_OFF = 1 << 8,    /* hold while flicking: old school (no comply, boneless, 180 no comply) */
};

typedef struct sk8_input
{
	float steer[2]; /* left stick, x right / y up: carve, air spin, manual balance */
	float flick[2]; /* right stick, x right / y up: crouch and flick tricks */
	unsigned buttons;
} sk8_input;

/* ---------- rider */

typedef enum sk8_mode
{
	SK8_MODE_RIDE,
	SK8_MODE_MANUAL,
	SK8_MODE_SLIDE,
	SK8_MODE_AIR,
	SK8_MODE_GRIND,
	SK8_MODE_BAIL,
	SK8_MODE_WALLRIDE,  /* rolling along a wall (up = the wall's normal) */
	SK8_MODE_WALK,      /* off the board on foot, carrying it */
} sk8_mode;

typedef enum sk8_event_kind
{
	SK8_EV_POP,      /* name = trick that popped (or "Ollie") */
	SK8_EV_TRICK,    /* name, points */
	SK8_EV_LAND,     /* value: 2 clean, 1 sketchy */
	SK8_EV_BAIL,     /* name = reason */
	SK8_EV_GRIND,    /* name = grind kind, start */
	SK8_EV_GRIND_END,
	SK8_EV_BANK,     /* points banked */
	SK8_EV_PUSH,
	SK8_EV_RESPAWN,
} sk8_event_kind;

typedef struct sk8_event
{
	sk8_event_kind kind;
	char name[40];
	unsigned points;
	int value;
	int surface;
} sk8_event;

typedef struct sk8_state
{
	float position[3];   /* point under the wheels (or on the rail) */
	float velocity[3];
	float board_rot[4];  /* quaternion x,y,z,w: the deck in world space, flips included */
	float rider_rot[4];  /* quaternion: the rider's stance frame (no flip/shuv) */
	float up[3];         /* surface / stance up */
	float speed;
	int mode;            /* sk8_mode */
	int fakie;
	float crouch;        /* 0..1 */
	float lean;          /* -1..1 */
	float push_phase;    /* 0..1 through a stroke (foot plants at SK8_PUSH_PLANT, lifts at
	                      * SK8_PUSH_RELEASE), -1 otherwise */
	int grab;            /* 0 none, 1 left, 2 right, 3 both */
	float flip_progress; /* 0..1 during a flip trick, -1 otherwise */
	float balance;       /* -1..1 manual / grind balance */
	float air_time;
	float bail_time;     /* seconds since the bail started, -1 when riding */
	int grind_kind;      /* sk8_grind_kind while grinding, 0 otherwise */
	unsigned combo_points;
	unsigned combo_count;
	unsigned long long score;
	int manual_dir;      /* 1 manual (tail down), -1 nose manual, 0 none */
	int grab_style;      /* sk8_grab_style while grabbing, 0 otherwise */
	float push_start;    /* 0..1 through the step off the board before the first stroke, -1 otherwise */
	int push_mongo;      /* 1 when the front foot pushes (mongo), 0 for the back foot: this push's (or
	                      * the last one's, while stepping back on) */
	int foot_off;        /* the foot-off button held on the ground (the front foot is stepped down
	                      * for an old-school pop) */
	float revert;        /* 0..1 through a revert (the board spinning round on landing), -1 otherwise */
} sk8_state;

/* Where in a stroke (push_phase) the pushing foot is on the ground; the push's speed is added
 * between the two, while the foot drives. */
#define SK8_PUSH_PLANT 0.18f
#define SK8_PUSH_RELEASE 0.62f

/* Grinds and slides: the board's angle to the rail when it lands on it picks a grind (along it) or
 * a slide (across it); the left stick then picks which (centre, toward the nose = up, toward the
 * tail = down, sideways, and the diagonals). */
typedef enum sk8_grind_kind
{
	SK8_GRIND_NONE = 0,
	SK8_GRIND_50_50,       /* both trucks (centre) */
	SK8_GRIND_BOARDSLIDE,  /* across the rail, middle of the deck (centre) */
	SK8_GRIND_UNUSED,
	SK8_GRIND_5_0,         /* back truck (down, or crouched) */
	SK8_GRIND_NOSEGRIND,   /* front truck (up) */
	SK8_GRIND_CROOKED,     /* front truck, nose turned over the near side (up + left) */
	SK8_GRIND_OVERCROOK,   /* front truck, nose turned over the far side (up + right) */
	SK8_GRIND_SMITH,       /* back truck, front dipped below the near side (down + left) */
	SK8_GRIND_FEEBLE,      /* back truck, front truck over the far side (down + right) */
	SK8_GRIND_NOSESLIDE,   /* across, on the nose (up) */
	SK8_GRIND_TAILSLIDE,   /* across, on the tail (down) */
	SK8_GRIND_LIPSLIDE,    /* across, middle, the board turned the other way over the rail (sideways) */
	SK8_GRIND_NOSEBLUNT,   /* across, nose pressed down over the rail (up + sideways) */
	SK8_GRIND_BLUNTSLIDE,  /* across, tail pressed down over the rail (down + sideways) */
	SK8_GRIND_FOOTPLANT,   /* foot-off held as it reaches an edge: the front foot planted on it, a stall */
	SK8_GRIND_HANDPLANT,   /* foot-off and a grab held: a hand planted on the edge, upside down */
	SK8_GRIND_DARKSLIDE,   /* across, the board caught upside down mid-flip: sliding on its grip */
} sk8_grind_kind;

/* Grab styles: the trigger(s) held pick the hand, the left stick's direction when the grab starts
 * picks the style (centre, toward the nose, toward the tail, sideways right, sideways left). */
typedef enum sk8_grab_style
{
	SK8_GRAB_NONE = 0,
	SK8_GRAB_INDY,        /* back hand, centre */
	SK8_GRAB_CRAIL,       /* back hand, nose */
	SK8_GRAB_TAIL,        /* back hand, tail */
	SK8_GRAB_STALEFISH,   /* back hand, sideways right */
	SK8_GRAB_MELON,       /* front hand, centre */
	SK8_GRAB_NOSE,        /* front hand, nose */
	SK8_GRAB_TUCK_KNEE,   /* front hand, tail */
	SK8_GRAB_JUDO,        /* front hand, sideways right */
	SK8_GRAB_DOUBLE,      /* both, centre */
	SK8_GRAB_SUPERMAN,    /* both, nose */
	SK8_GRAB_NO_FOOT,     /* both, tail */
	SK8_GRAB_STIFFY,      /* both, sideways right */
	SK8_GRAB_CHRIST,      /* back hand, sideways left: board held out, arms spread */
	SK8_GRAB_METHOD,      /* front hand, sideways left: heel edge, board arched up behind */
	SK8_GRAB_ROCKET,      /* both, sideways left: both hands on the nose, feet on the tail */
} sk8_grab_style;

sk8_rider *sk8_rider_new(void);
void sk8_rider_delete(sk8_rider *rider);

/* Put the rider on the board, standing at position, nose along facing (radians), rolling at speed. */
void sk8_rider_place(sk8_rider *rider, const float position[3], float facing, float speed);
void sk8_rider_step(sk8_rider *rider, sk8_world *world, const sk8_input *input);
void sk8_rider_state(const sk8_rider *rider, sk8_state *state);
int sk8_rider_poll_event(sk8_rider *rider, sk8_event *event); /* 0 when empty */
const char *sk8_rider_trick(const sk8_rider *rider);          /* trick in progress, or "" */
void sk8_rider_bail(sk8_rider *rider);

enum
{
	SK8_FLAG_NEVER_BAIL = 1 << 0, /* land everything, never fall */
	SK8_FLAG_NO_RESPAWN = 1 << 1, /* host handles the return after a bail (markers) */
};
void sk8_rider_set_flags(sk8_rider *rider, unsigned flags);
unsigned sk8_rider_flags(const sk8_rider *rider);

/* ---------- tuning: every value the mod menu can change */

typedef struct sk8_param
{
	const char *key;   /* stable id, used in saved settings */
	const char *label; /* menu text */
	const char *group; /* menu tab */
	float min, max, def, step;
} sk8_param;

int sk8_param_count(void);
const sk8_param *sk8_param_at(int index);
int sk8_param_find(const char *key); /* -1 when unknown */
float sk8_rider_param(const sk8_rider *rider, int index);
void sk8_rider_set_param(sk8_rider *rider, int index, float value); /* clamped to min..max */

/* ---------- flick recogniser, for tests and tutorials */

/* Classify a right-stick path (x,y pairs, one per step). Returns the trick name or NULL. */
const char *sk8_flick_classify(const float *xy, int count);
int sk8_trick_count(void);
const char *sk8_trick_name(int index);
/* Writes a sample stick path for the trick (up to max pairs); returns the pair count. */
int sk8_trick_demo_path(int index, float *xy, int max);

#ifdef __cplusplus
}
#endif

#endif

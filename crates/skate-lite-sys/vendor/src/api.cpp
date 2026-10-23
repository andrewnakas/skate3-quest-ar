// C API of the clean-room engine (written from scratch).
#include "../include/sk8.h"
#include "flick.hpp"
#include "params.hpp"
#include "rider.hpp"
#include "world.hpp"
#include <cstring>

struct sk8_world
{
	sk8::World w;
};

struct sk8_rider
{
	sk8::Rider r;
};

extern "C" {

sk8_world *sk8_world_new(sk8_raycast_fn raycast, void *user)
{
	auto *w = new sk8_world;
	w->w.raycast = raycast;
	w->w.user = user;
	return w;
}

void sk8_world_delete(sk8_world *world) { delete world; }

int sk8_world_add_rail(sk8_world *world, const float *points, int point_count)
{
	if (!world || !points || point_count < 2)
		return -1;
	std::vector<sk8::V3> r;
	for (int i = 0; i < point_count; i++)
		r.emplace_back(points + 3 * i);
	world->w.rails.push_back(std::move(r));
	return int(world->w.rails.size()) - 1;
}

void sk8_world_clear_rails(sk8_world *world)
{
	if (world)
		world->w.rails.clear();
}

int sk8_world_rail_count(const sk8_world *world) { return world ? int(world->w.rails.size()) : 0; }

sk8_rider *sk8_rider_new(void) { return new sk8_rider; }
void sk8_rider_delete(sk8_rider *rider) { delete rider; }

void sk8_rider_place(sk8_rider *rider, const float position[3], float facing, float speed)
{
	rider->r.place(sk8::V3(position), facing, speed);
}

void sk8_rider_launch(sk8_rider *rider, const float position[3], const float velocity[3], float facing)
{
	rider->r.launch(sk8::V3(position), sk8::V3(velocity), facing);
}

void sk8_rider_set_stance(sk8_rider *rider, int switch_stance, int fakie)
{
	rider->r.set_stance(switch_stance != 0, fakie != 0);
}

void sk8_rider_step(sk8_rider *rider, sk8_world *world, const sk8_input *input)
{
	rider->r.step(world->w, *input);
}

void sk8_rider_state(const sk8_rider *rider, sk8_state *state) { rider->r.state(*state); }

int sk8_rider_poll_event(sk8_rider *rider, sk8_event *event)
{
	if (rider->r.events.empty())
		return 0;
	*event = rider->r.events.front();
	rider->r.events.pop_front();
	return 1;
}

const char *sk8_rider_trick(const sk8_rider *rider) { return rider->r.trick(); }
void sk8_rider_bail(sk8_rider *rider) { rider->r.bail("Bailed"); }
void sk8_rider_set_flags(sk8_rider *rider, unsigned flags) { rider->r.flags = flags; }
unsigned sk8_rider_flags(const sk8_rider *rider) { return rider->r.flags; }

int sk8_param_count(void) { return sk8::ParamCount; }

const sk8_param *sk8_param_at(int index)
{
	return index >= 0 && index < sk8::ParamCount ? &sk8::param_table()[index] : nullptr;
}

int sk8_param_find(const char *key)
{
	for (int i = 0; i < sk8::ParamCount; i++)
		if (std::strcmp(sk8::param_table()[i].key, key) == 0)
			return i;
	return -1;
}

float sk8_rider_param(const sk8_rider *rider, int index)
{
	return index >= 0 && index < sk8::ParamCount ? rider->r.p.v[index] : 0.0f;
}

void sk8_rider_set_param(sk8_rider *rider, int index, float value)
{
	if (index < 0 || index >= sk8::ParamCount)
		return;
	const sk8_param &d = sk8::param_table()[index];
	rider->r.p.v[index] = sk8::clampf(value, d.min, d.max);
}

const char *sk8_flick_classify(const float *xy, int count) { return sk8::classify_path(xy, count); }
int sk8_trick_count(void) { return sk8::gesture_trick_count(); }

const char *sk8_trick_name(int index)
{
	return index >= 0 && index < sk8_trick_count() ? sk8::trick_table()[size_t(index)].name : nullptr;
}

int sk8_trick_demo_path(int index, float *xy, int max) { return sk8::demo_path(index, xy, max); }

} // extern "C"

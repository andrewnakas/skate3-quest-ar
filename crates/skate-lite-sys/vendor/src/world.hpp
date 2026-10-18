// Host collision and grind rails for the clean-room engine (written from scratch).
#pragma once
#include "../include/sk8.h"
#include "vec.hpp"
#include <vector>

namespace sk8 {

struct Hit
{
	V3 point, normal;
	float fraction = 1;
	int surface = 0;
};

struct RailSpot
{
	int rail = -1;
	int seg = 0;
	float u = 0;      // metres along the segment
	V3 point, tangent;
	float distance = 1e9f;
};

struct World
{
	sk8_raycast_fn raycast = nullptr;
	void *user = nullptr;
	std::vector<std::vector<V3>> rails;

	bool cast(V3 origin, V3 delta, Hit &out) const;
	RailSpot nearest_rail(V3 p, float max) const;
	float segment_length(int rail, int seg) const;
};

} // namespace sk8

// Host collision and grind rails (written from scratch).
#include "world.hpp"

namespace sk8 {

bool World::cast(V3 origin, V3 delta, Hit &out) const
{
	if (!raycast)
		return false;
	float o[3], d[3];
	origin.store(o);
	delta.store(d);
	sk8_hit h{};
	if (!raycast(user, o, d, &h))
		return false;
	out.point = V3(h.point);
	out.normal = unit(V3(h.normal));
	out.fraction = h.fraction;
	out.surface = h.surface;
	return true;
}

float World::segment_length(int rail, int seg) const
{
	const auto &r = rails[size_t(rail)];
	return len(r[size_t(seg) + 1] - r[size_t(seg)]);
}

RailSpot World::nearest_rail(V3 p, float max) const
{
	RailSpot best;
	for (size_t ri = 0; ri < rails.size(); ri++) {
		const auto &r = rails[ri];
		for (size_t si = 0; si + 1 < r.size(); si++) {
			V3 a = r[si], b = r[si + 1], ab = b - a;
			float L = len(ab);
			if (L < 1e-4f)
				continue;
			V3 t = ab * (1.0f / L);
			float u = clampf(dot(p - a, t), 0, L);
			V3 q = a + t * u;
			float d = len(p - q);
			if (d < max && d < best.distance) {
				best.rail = int(ri);
				best.seg = int(si);
				best.u = u;
				best.point = q;
				best.tangent = t;
				best.distance = d;
			}
		}
	}
	return best;
}

} // namespace sk8

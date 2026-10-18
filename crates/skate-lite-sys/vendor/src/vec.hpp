// Vector and quaternion helpers for the clean-room engine (written from scratch).
#pragma once
#include <algorithm>
#include <cmath>

namespace sk8 {

constexpr float kPi = 3.14159265f;

struct V3
{
	float x = 0, y = 0, z = 0;
	constexpr V3() = default;
	constexpr V3(float a, float b, float c) : x(a), y(b), z(c) {}
	explicit V3(const float *p) : x(p[0]), y(p[1]), z(p[2]) {}
	V3 operator+(V3 o) const { return {x + o.x, y + o.y, z + o.z}; }
	V3 operator-(V3 o) const { return {x - o.x, y - o.y, z - o.z}; }
	V3 operator-() const { return {-x, -y, -z}; }
	V3 operator*(float s) const { return {x * s, y * s, z * s}; }
	V3 &operator+=(V3 o) { return *this = *this + o; }
	V3 &operator-=(V3 o) { return *this = *this - o; }
	V3 &operator*=(float s) { return *this = *this * s; }
	void store(float *p) const { p[0] = x; p[1] = y; p[2] = z; }
};

inline float dot(V3 a, V3 b) { return a.x * b.x + a.y * b.y + a.z * b.z; }
inline V3 cross(V3 a, V3 b) { return {a.y * b.z - a.z * b.y, a.z * b.x - a.x * b.z, a.x * b.y - a.y * b.x}; }
inline float len(V3 a) { return std::sqrt(dot(a, a)); }
inline V3 unit(V3 a, V3 fallback = {0, 1, 0})
{
	float l = len(a);
	return l > 1e-6f ? a * (1.0f / l) : fallback;
}
// Component of v lying in the plane with normal n (n unit).
inline V3 flatten(V3 v, V3 n) { return v - n * dot(v, n); }
inline float clampf(float v, float lo, float hi) { return std::min(std::max(v, lo), hi); }
inline float approach(float v, float target, float rate)
{
	return v < target ? std::min(v + rate, target) : std::max(v - rate, target);
}
// Angle between unit vectors, radians.
inline float angle_between(V3 a, V3 b) { return std::acos(clampf(dot(a, b), -1.0f, 1.0f)); }
// Rotate v about unit axis by angle (Rodrigues).
inline V3 rotate(V3 v, V3 axis, float angle)
{
	float c = std::cos(angle), s = std::sin(angle);
	return v * c + cross(axis, v) * s + axis * (dot(axis, v) * (1 - c));
}

constexpr V3 kUp{0, 1, 0};

struct Q
{
	float x = 0, y = 0, z = 0, w = 1;
	static Q axis_angle(V3 axis, float angle)
	{
		float s = std::sin(angle * 0.5f);
		return {axis.x * s, axis.y * s, axis.z * s, std::cos(angle * 0.5f)};
	}
	Q operator*(Q o) const
	{
		return {w * o.x + x * o.w + y * o.z - z * o.y, w * o.y - x * o.z + y * o.w + z * o.x,
			w * o.z + x * o.y - y * o.x + z * o.w, w * o.w - x * o.x - y * o.y - z * o.z};
	}
	void store(float *p) const { p[0] = x; p[1] = y; p[2] = z; p[3] = w; }
};

// Rotation whose columns are right, up, fwd (an orthonormal right-handed basis with fwd = +Z).
inline Q from_basis(V3 right, V3 up, V3 fwd)
{
	float m00 = right.x, m01 = up.x, m02 = fwd.x;
	float m10 = right.y, m11 = up.y, m12 = fwd.y;
	float m20 = right.z, m21 = up.z, m22 = fwd.z;
	float tr = m00 + m11 + m22;
	Q q;
	if (tr > 0) {
		float s = std::sqrt(tr + 1.0f) * 2;
		q = {(m21 - m12) / s, (m02 - m20) / s, (m10 - m01) / s, 0.25f * s};
	} else if (m00 > m11 && m00 > m22) {
		float s = std::sqrt(1.0f + m00 - m11 - m22) * 2;
		q = {0.25f * s, (m01 + m10) / s, (m02 + m20) / s, (m21 - m12) / s};
	} else if (m11 > m22) {
		float s = std::sqrt(1.0f + m11 - m00 - m22) * 2;
		q = {(m01 + m10) / s, 0.25f * s, (m12 + m21) / s, (m02 - m20) / s};
	} else {
		float s = std::sqrt(1.0f + m22 - m00 - m11) * 2;
		q = {(m02 + m20) / s, (m12 + m21) / s, 0.25f * s, (m10 - m01) / s};
	}
	return q;
}

} // namespace sk8

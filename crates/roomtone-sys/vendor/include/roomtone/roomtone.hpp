// Roomtone: room light estimation for mixed reality.
//
// A headset's passthrough camera, reduced to a small grid of linear colour
// per frame with its exposure, and the headset's pose, become the room's
// light as a renderer wants it: spherical harmonics (L2) for the soft light
// from every direction, up to two key lights with their place in the room,
// and a level that matches the brightness passthrough shows the room at.
// Depth (raycast hits, depth images) gives the lights their place, and the
// room's surfaces theirs, so the light is the light at the point asked
// about (where the virtual content is), not where the head was.
//
// Coordinates: tracking space, metres, +Y up (OpenXR's). Not thread-safe:
// callers serialize (one mutex around a Room is enough).
#pragma once

#include <array>
#include <cstdint>
#include <memory>
#include <string>
#include <vector>

namespace roomtone {

struct Vec3 {
  float x = 0.f, y = 0.f, z = 0.f;
};
struct Quat {
  float x = 0.f, y = 0.f, z = 0.f, w = 1.f;
};
struct Pose {
  Quat orientation;
  Vec3 position;
};
using Rgb = std::array<float, 3>;

// The camera, as Camera2 describes it (Meta's Passthrough Camera API).
struct Camera {
  // LENS_POSE_ROTATION: turns the head's axes into the camera's.
  Quat lensRotation;
  // LENS_INTRINSIC_CALIBRATION (fx, fy, cx, cy), in the pixels of the sensor
  // array (SENSOR_INFO_PRE_CORRECTION_ACTIVE_ARRAY_SIZE).
  float fx = 0.f, fy = 0.f, cx = 0.f, cy = 0.f;
  int sensorWidth = 0, sensorHeight = 0;
  // The stream: the whole sensor array, scaled.
  int width = 0, height = 0;
};

// A camera frame reduced to a grid of cols x rows cells, row-major from the
// top: each cell's mean linear r, g, b (0-1 as the camera gave them) and the
// share of its pixels that clipped.
struct Frame {
  const float* cells = nullptr; // cols * rows * 4
  int cols = 0, rows = 0;
  float exposureMs = 0.f, iso = 0.f; // a frame without them is left out
  Pose head;                         // the head when it was taken
  int64_t timeNs = 0;                // a steady clock
};

// A depth image (XR_META_environment_depth's convention): OpenGL depth (0-1
// over 65535, rows from the bottom), nearZ / (1 - d) along the view's axis
// with an infinite far plane.
struct DepthImage {
  const uint16_t* depth = nullptr;
  int width = 0, height = 0, stride = 0; // stride in values
  float nearZ = 0.1f, farZ = 0.f;        // farZ infinite or the far plane
  float tanLeft = -1.f, tanRight = 1.f, tanUp = 1.f, tanDown = -1.f;
  Pose view;
  int64_t timeNs = 0; // the steady clock frames use; frames more than 0.5 s later don't use it
};

struct Light {
  Rgb color{};          // irradiance over pi, as the SH (up-facing light 1)
  Vec3 dir{0.f, 1.f, 0.f}; // toward the light, from the point asked about
  bool placed = false;  // whether it has a place (pos) in the room
  Vec3 pos;
  // A placed light's distance from the point, where its colour applies: a
  // renderer lighting a surface elsewhere scales it by range^2 / d^2.
  float range = 0.f;
};

// The light at a point.
struct Lighting {
  // SH L2 irradiance over pi, each coefficient times its basis constant, so
  // a shader evaluates the bare polynomials 1, y, z, x, xy, yz, 3z^2 - 1,
  // xz, x^2 - y^2 at the unit normal (tracking space). In the units of the
  // reference, the room as the head saw it, where an up-facing surface gets
  // 1 (lights included): without depth the light at any point is the
  // reference's; with it, content nearer a light gets more than 1 from it
  // and farther less. Normalize by the reference (mean, 1 for up), not by
  // the light at the point, or a light's falloff cancels.
  std::array<Rgb, 9> sh{};
  std::array<Light, 2> lights{};
  float mean = 1.f;  // the reference's mean light over every direction, without its lights (as the level)
  float level = 1.f; // the reference's brightness as passthrough shows it (ABL)
};

// A bright spot the capture has seen (Room::spots): seen cells over twice
// the room's mean, or mostly clipped, chained with their bright neighbours
// (within 17 degrees) as one (a fixture's bulbs, a window and its glare),
// whose brightest is over 4 times the mean or clipped. The key lights are
// picked among the brightest cells; a player told what the capture found
// counts these.
struct Spot {
  Vec3 dir{0.f, 0.f, -1.f}; // toward it, from where the head was (tracking space, unit)
  float brightness = 0.f;   // its brightest cell's, times the room's mean
  bool clipped = false;     // most of its brightest cell's samples clipped
};

// What a frame did.
struct FrameResult {
  bool used = false;      // false: no exposure, no camera or an empty grid
  bool restarted = false; // the room changed: the capture started over
  float change = 1.f;     // the frame against the capture, by its median
};

class Room {
public:
  Room();
  ~Room();
  Room(const Room&) = delete;
  Room& operator=(const Room&) = delete;

  // Drops everything seen (a new capture).
  void reset();
  void setCamera(const Camera& camera);
  bool hasCamera() const;
  FrameResult addFrame(const Frame& frame);
  // A depth hit along a ray from origin (a raycast): gives the lights a place.
  void addHit(Vec3 origin, Vec3 point);
  // The latest depth image: the room's surfaces get a place (frames after it
  // land on them).
  void addDepth(const DepthImage& image);
  // The tracking space moved under the room (a recenter): (p0, yaw0) to
  // (p1, yaw1), yaw about +Y. True when the light was made again.
  bool moved(Vec3 p0, float yaw0, Vec3 p1, float yaw1);
  // Makes the light from what's been seen now (addFrame does every few
  // frames).
  void make();
  bool made() const;
  uint32_t frames() const;
  size_t hits() const;
  // The points the light is made for (where the content is: one, or a few
  // across it to blend between); the first sets the level's exposure.
  // Cheap, for a render thread: with depth, a point moved 5 cm makes the
  // light stale (the placed lights re-aimed meanwhile), and the next frame
  // or make() makes it from there (a make costs one build per point).
  void setPoints(const Vec3* points, size_t count);
  // The light at points[i]; false while none is made.
  bool lightAt(size_t i, Lighting& out) const;
  // One point: setPoints(&point, 1), then lightAt(0).
  bool lightAt(Vec3 point, Lighting& out);
  // Whether a make is due for a moved point.
  bool stale() const;
  size_t voxels() const;
  // A line of diagnostics (coverage, lights, level, brightest cells).
  std::string summary() const;
  // Every bright spot as the light was last made, brightest first (empty
  // before a make, and after the room changed until the next).
  const std::vector<Spot>& spots() const;
  // The seen room's median brightness as passthrough shows it (0-1 display;
  // 0 before a make): a dark room's is low.
  float seenMedian() const;

  struct Impl;

private:
  std::unique_ptr<Impl> d_;
};

} // namespace roomtone

// The room's light from camera frames (roomtone.hpp).
//
// Each frame's grid of mean colour, over its exposure times sensitivity
// (relative radiance), lands by the ray of each cell (the lens's
// intrinsics, its pose against the head, the head's pose) in a cube of
// kN x kN cells a face, each of kSub x kSub parts. A part's radiance is the
// running mean of its samples, the clipped ones boosted by how much clipped
// (a lower bound; a lamp seen only clipped would otherwise vanish into the
// sky). A cell's radiance is its parts' by solid angle, the unseen ones its
// hemisphere's seen surfaces (lights left out), so a lamp seen at the
// frame's edge lights by the part it covers, not the whole cell, and a
// monitor ahead doesn't tint the directions the camera never saw.
//
// The light is made every few frames: up to two lights, the brightest cells
// at least 45 degrees apart, by their excess over the mean (a window and a
// ceiling light both count), the rest as SH L2. A light seen by depth gets a
// place in the room. Its level is the room's as passthrough shows it: the
// seen room's median brightness in the frames' own pixel scale (the
// relative radiance times the exposure of the frames that looked at the
// point asked about), by kMedian.
//
// The constants were tuned on a Quest 3 in one user's rooms (2026-10-20 to
// 22, in Xelee, a Melee port, where the stage sits on a table).
#include "roomtone/roomtone.hpp"

#include <algorithm>
#include <chrono>
#include <cmath>
#include <cstdio>
#include <limits>
#include <tuple>
#include <unordered_map>
#include <utility>
#include <vector>

namespace roomtone {
namespace {

constexpr int kN = 8, kSub = 3, kF = kN * kSub;
constexpr int kCells = 6 * kN * kN, kParts = 6 * kF * kF;
// The level per unit of the seen room's median brightness (by solid angle,
// in passthrough's scale). The light from above moved the level 3x on
// whether the camera caught the ceiling light, the room the same; the
// walls' median hardly moves with a fixture in or out of view. The user's
// picks put the level at 9.6 times the median with no light; then 1.3 times
// that over a day of rooms. (It was also dimmed by the lights' share of the
// reference's light, down to half: picked when every point got a light at
// the head's strength. With each point's light its own, a stage far from a
// light lost half its light the moment a capture found the light, 2-3x
// with the mean's share, 2026-10-23.)
constexpr float kMedian = 12.5f, kLevelMin = 0.15f, kLevelMax = 1.6f;
// A frame a bright light dominates (a monitor ahead in a lit room) exposed
// passthrough darker than the room: its exposure counts as up to kGlareMax
// brighter, by the square root of its mean over its mean without the light
// (samples over twice the median).
constexpr float kGlareMax = 1.4f;
// The room changed (a light switched off): frames whose samples, where the
// capture has seen well (kChangeSeen samples a part), read kChange times
// brighter or darker by their median, for kChangeNs (a hand before the
// camera passes sooner), start the capture over, its parts and frames
// dropped (its depth hits kept): the old light and the new would fight.
constexpr double kChange = 3.0, kChangeSeen = 20.0;
constexpr int64_t kChangeNs = 1'000'000'000;
constexpr size_t kChangeMinSamples = 64;
// The frames whose exposure sets the level: the camera's middle within 40
// degrees of the point, the closest 30, at least 10.
constexpr float kPointCos = 0.766f;
constexpr size_t kPointFrames = 30, kPointMinFrames = 10;
// An unseen part takes its hemisphere's seen parts up to this times their
// median (the rest are lights, not the room's surfaces).
constexpr float kFillMax = 2.f;
// A light takes the excess of the bright cells within 17 degrees of its brightest.
constexpr float kMergeCos = 0.956f;
// A bright spot (spots()): the seen cells over kSpotOver times the mean (as
// the lights take them), or with over kSpotClipped of their samples
// clipped, from at least kSpotSamples samples (a cell glimpsed once at the
// frame's edge is noise), chained with their bright neighbours; one counts
// when its brightest is over kSpotPeak times the mean, or clipped. In the
// user's dark room with a monitor (2026-10-25's recording), 2 times with
// the 17 degrees from the brightest alone made 12 spots of the monitor, its
// glare and the desk under it; chained, with the peak, one.
constexpr double kSpotOver = 2.0, kSpotPeak = 4.0;
constexpr float kSpotClipped = 0.5f, kSpotSamples = 8.f;
constexpr float kClipped = 0.02f, kClipBoost = 4.f;
// Each part a running mean of about its last 600 samples.
constexpr double kPartMax = 600.0;
// Frames kept for their exposure (the level), and depth hits.
constexpr size_t kViewsMax = 600, kHitsMax = 16384;
// The light made again every so many frames.
constexpr uint32_t kMakeEvery = 5;
// Depth: the room's surfaces as kVoxel voxels, from the camera samples a
// depth image (within kDepthFreshNs of the frame) puts a distance behind:
// the image's points (kDepthGrid a side, within the Depth API's reliable
// kDepthNear-kDepthFar) projected into the frame's grid, each cell taking
// its points' median distance. When the light is made, the voxels are
// drawn into the cube from the point asked about, the nearest in each part
// (and those within kDepthTol of it) winning, so near surfaces light it
// from where they are, and falling off with distance by the solid angle
// they cover. Voxels nearer the point than kNearest are left out (the
// table under it); directions no voxel covers keep the head's view, as
// light from a distance (windows, bulbs, what depth doesn't reach).
constexpr float kVoxel = 0.1f, kDepthNear = 0.2f, kDepthFar = 4.5f, kNearest = 0.15f, kDepthTol = 1.15f;
constexpr int kDepthGrid = 80;
constexpr int64_t kDepthFreshNs = 500'000'000;
constexpr double kVoxelMax = 300.0, kVoxelMinSamples = 3.0;
constexpr size_t kVoxelsMax = 60000;
constexpr double kPi = 3.14159265358979323846;

Vec3 operator+(Vec3 a, Vec3 b) { return {a.x + b.x, a.y + b.y, a.z + b.z}; }
Vec3 operator-(Vec3 a, Vec3 b) { return {a.x - b.x, a.y - b.y, a.z - b.z}; }
Vec3 operator*(Vec3 a, float k) { return {a.x * k, a.y * k, a.z * k}; }
float dot(Vec3 a, Vec3 b) { return a.x * b.x + a.y * b.y + a.z * b.z; }
Vec3 cross(Vec3 a, Vec3 b) { return {a.y * b.z - a.z * b.y, a.z * b.x - a.x * b.z, a.x * b.y - a.y * b.x}; }
Vec3 rotate(const Quat& q, Vec3 v) {
  const Vec3 u{q.x, q.y, q.z};
  const Vec3 t = cross(u, v) * 2.f;
  return v + t * q.w + cross(u, t);
}
Quat conj(const Quat& q) { return {-q.x, -q.y, -q.z, q.w}; }
Quat mul(const Quat& a, const Quat& b) {
  return {a.w * b.x + a.x * b.w + a.y * b.z - a.z * b.y, a.w * b.y - a.x * b.z + a.y * b.w + a.z * b.x,
          a.w * b.z + a.x * b.y - a.y * b.x + a.z * b.w, a.w * b.w - a.x * b.x - a.y * b.y - a.z * b.z};
}
Quat yaw_quat(float a) { return {0.f, std::sin(a / 2.f), 0.f, std::cos(a / 2.f)}; }
// Distance along the view's axis, from a depth value (OpenGL's convention).
float depth_linear(uint16_t v, float nearZ, float farZ) {
  const float dn = static_cast<float>(v) / 65535.f;
  if (!std::isfinite(farZ) || !(farZ > nearZ))
    return dn < 1.f ? nearZ / (1.f - dn) : std::numeric_limits<float>::infinity();
  const float z = 2.f * dn - 1.f;
  return 2.f * nearZ * farZ / (farZ + nearZ - z * (farZ - nearZ));
}
int64_t voxel_key(Vec3 p) {
  const auto q = [](float v) { return static_cast<int64_t>(std::floor(v / kVoxel)) & 0x1FFFFF; };
  return (q(p.x) << 42) | (q(p.y) << 21) | q(p.z);
}
// By `a` radians about +Y (counter-clockwise seen from above).
Vec3 rot_y(Vec3 v, float a) {
  const float c = std::cos(a), s = std::sin(a);
  return {c * v.x + s * v.z, v.y, -s * v.x + c * v.z};
}
double lum(const std::array<double, 3>& v) { return 0.2126 * v[0] + 0.7152 * v[1] + 0.0722 * v[2]; }
float lumf(const float* v) { return 0.2126f * v[0] + 0.7152f * v[1] + 0.0722f * v[2]; }

// A direction's cube cell (of n x n a face), and a cell's direction
// (unnormalized) and solid angle.
int cube_cell(Vec3 d, int n) {
  const float ax = std::abs(d.x), ay = std::abs(d.y), az = std::abs(d.z);
  int face;
  float u, v, m;
  if (ax >= ay && ax >= az)
    face = d.x > 0.f ? 0 : 1, m = ax, u = d.z, v = d.y;
  else if (ay >= az)
    face = d.y > 0.f ? 2 : 3, m = ay, u = d.x, v = d.z;
  else
    face = d.z > 0.f ? 4 : 5, m = az, u = d.x, v = d.y;
  if (!(m > 0.f))
    return -1;
  const int i = std::clamp(static_cast<int>((u / m + 1.f) * 0.5f * n), 0, n - 1);
  const int j = std::clamp(static_cast<int>((v / m + 1.f) * 0.5f * n), 0, n - 1);
  return (face * n + j) * n + i;
}
Vec3 cube_dir(int cell, float& dOmega, int n) {
  const int face = cell / (n * n), j = (cell / n) % n, i = cell % n;
  const float u = (i + 0.5f) / n * 2.f - 1.f, v = (j + 0.5f) / n * 2.f - 1.f;
  const float s = face % 2 == 0 ? 1.f : -1.f;
  const Vec3 d = face < 2 ? Vec3{s, v, u} : face < 4 ? Vec3{u, s, v} : Vec3{u, v, s};
  const float r2 = 1.f + u * u + v * v;
  dOmega = (4.f / (n * n)) / (r2 * std::sqrt(r2));
  return d;
}
Vec3 unit(Vec3 v) {
  const float n = std::sqrt(dot(v, v));
  return n > 0.f ? v * (1.f / n) : Vec3{0.f, 1.f, 0.f};
}

} // namespace

struct Room::Impl {
  bool haveCamera = false;
  Camera cam;
  // The capture's own frame in tracking space (p = pos + rot_y(p_capture,
  // yaw)): parts, frames and hits are kept in it, and moved() moves it, so
  // the lights keep their place in the room; the light made from them is in
  // tracking space.
  Vec3 pos;
  float yaw = 0.f;
  // Each part's r, g, b sums and their count: unclipped samples, then
  // clipped (boosted).
  std::vector<std::array<double, 8>> parts = std::vector<std::array<double, 8>>(kParts);
  // Each cell's luminance and how it was seen (clear and clipped samples,
  // parts seen), for the summary.
  std::array<std::array<float, 4>, kCells> seenAs{};
  uint32_t frames = 0, sinceMade = 0;
  // Recent frames' exposure (ms) times ISO over 100 (with the glare's), and
  // where the head was and the camera looked (the capture's frame).
  struct View {
    float factor;
    Vec3 head, dir;
    float glare;
  };
  std::vector<View> views;
  // Depth hits: the direction from the head (unit), then the point; the
  // oldest replaced once full.
  std::vector<std::array<float, 6>> hits;
  size_t hitNext = 0;
  int64_t changedNs = 0; // since when the room read changed; 0: it doesn't
  // The latest depth image's points (the capture's frame), and the voxels:
  // each one's r, g, b sums and counts as a part's, and its samples'
  // positions (sum and count).
  std::vector<Vec3> depth;
  int64_t depthNs = 0;
  struct Voxel {
    std::array<double, 8> e{};
    double p[3]{};
    double n = 0.0;
  };
  std::unordered_map<int64_t, Voxel> voxels;
  float voxelShare = 0.f; // the parts the voxels covered at the last make
  bool stale = false;     // the point moved since: the next frame (or make) makes the light again
  float makeMs = 0.f, makeMaxMs = 0.f; // the last make's time, and the longest

  // The light last made (made: any), at each point asked about, in
  // tracking space.
  bool made = false;
  struct At {
    std::array<Rgb, 9> sh{};
    std::array<Vec3, 2> lightDir{Vec3{0.f, 1.f, 0.f}, Vec3{0.f, 1.f, 0.f}};
    std::array<Rgb, 2> light{};
    std::array<bool, 2> lightAt{};
    std::array<Vec3, 2> lightPos{};
    std::array<int, 2> lightHits{};
    std::array<float, 2> lightVoxelShare{}; // of a placed light's brightness, what the point's voxels saw
    std::array<float, 2> lightRange{}; // a placed light's distance from where its colour was measured
    float upAt = 1.f;                  // the light from above there, as the reference's (1)
  };
  std::vector<At> at = std::vector<At>(1);
  float mean = 1.f, level = 1.f, coverage = 0.f;
  float display = 0.f;             // the light from above, in passthrough's scale
  float seen = 0.f, middle = 0.f;  // the seen parts' mean and median luminance, as display
  float radiance = 0.f; // the median's relative radiance
  float unclamped = 0.f;
  std::vector<Spot> spots; // the head's view's, at the last make (tracking space)
  std::array<Rgb, 2> fill{}; // the unseen parts' colour (luminance 1), upper and lower
  // The points the light was last made for (tracking space; the first sets
  // the level's exposure), and from how many frames looking at it (0: all
  // of them).
  bool havePoint = false;
  std::vector<Vec3> points;
  int pointFrames = 0;
  float pointGlare = 1.f;

  Vec3 toCapture(Vec3 p) const { return rot_y(p - pos, -yaw); }
  Vec3 dirToCapture(Vec3 d) const { return rot_y(d, -yaw); }
  Vec3 fromCapture(Vec3 p) const { return pos + rot_y(p, yaw); }

  float factor(const Vec3* point, int* used, float* glare) const;
  void makeLevel();
  struct Built {
    std::array<std::array<double, 3>, 9> s{};
    std::array<std::array<double, 3>, 2> lt{};
    std::array<Vec3, 2> ld{Vec3{0.f, 1.f, 0.f}, Vec3{0.f, 1.f, 0.f}};
    int picked[2] = {-1, -1};
    std::vector<int> lightCells[2];
    double upLum = 0.0, mean = 0.0;
    std::vector<Spot> spots; // tracking space
  };
  Built build(const std::vector<std::array<double, 3>>& P, const std::vector<bool>& partSeen);
  void make();
  Vec3 evalPoint(size_t i) const;
  void splat(Vec3 at, std::vector<std::array<double, 3>>& P, std::vector<bool>& partSeen,
             std::vector<Vec3>& partPos, std::vector<bool>& fromVoxel);
  std::string brightest() const;
};

Room::Room() : d_(std::make_unique<Impl>()) {}
Room::~Room() = default;

void Room::reset() {
  auto& c = *d_;
  c.pos = {};
  c.yaw = 0.f;
  c.parts.assign(kParts, {});
  c.frames = c.sinceMade = 0;
  c.made = false;
  c.coverage = 0.f;
  c.views.clear();
  c.hits.clear();
  c.hitNext = 0;
  c.at.assign(1, {});
  c.changedNs = 0;
  c.depth.clear();
  c.voxels.clear();
  c.voxelShare = 0.f;
  c.makeMaxMs = 0.f;
  c.spots.clear();
  c.middle = 0.f;
}

void Room::setCamera(const Camera& camera) {
  d_->cam = camera;
  d_->haveCamera = camera.width > 0 && camera.height > 0 && camera.sensorWidth > 0 && camera.sensorHeight > 0 &&
                   camera.fx > 0.f && camera.fy > 0.f;
}
bool Room::hasCamera() const { return d_->haveCamera; }
bool Room::made() const { return d_->made; }
uint32_t Room::frames() const { return d_->frames; }
size_t Room::hits() const { return d_->hits.size(); }
size_t Room::voxels() const { return d_->voxels.size(); }
bool Room::stale() const { return d_->stale; }
const std::vector<Spot>& Room::spots() const { return d_->spots; }
float Room::seenMedian() const { return d_->made ? d_->middle : 0.f; }

// The median exposure of the frames looking at the point (tracking space;
// null: of all frames), as passthrough showed the room there: it shares the
// camera's auto exposure, so a monitor or a window in view darkened both,
// and the user looks at the content, not at them. The closest kPointFrames
// within kPointCos; fewer than kPointMinFrames, all of them.
float Room::Impl::factor(const Vec3* point, int* used, float* glare) const {
  if (used)
    *used = 0;
  if (glare)
    *glare = 1.f;
  if (views.empty())
    return 1.f;
  std::vector<std::pair<float, const View*>> near; // cos, frame
  if (point) {
    const Vec3 s = toCapture(*point);
    for (const auto& v : views) {
      const Vec3 d = s - v.head;
      const float n = std::sqrt(dot(d, d));
      if (n > 0.05f)
        if (const float cs = dot(d, v.dir) / n; cs > kPointCos)
          near.emplace_back(cs, &v);
    }
  }
  std::vector<const View*> chosen;
  if (near.size() >= kPointMinFrames) {
    const size_t k = std::min(near.size(), kPointFrames);
    std::partial_sort(near.begin(), near.begin() + k, near.end(),
                      [](const auto& a, const auto& b) { return a.first > b.first; });
    for (size_t i = 0; i < k; ++i)
      chosen.push_back(near[i].second);
    if (used)
      *used = static_cast<int>(k);
  } else {
    for (const auto& v : views)
      chosen.push_back(&v);
  }
  const auto median_of = [&](auto get) {
    std::vector<float> f;
    for (const auto* v : chosen)
      f.push_back(get(*v));
    std::nth_element(f.begin(), f.begin() + f.size() / 2, f.end());
    return f[f.size() / 2];
  };
  if (glare)
    *glare = median_of([](const View& v) { return v.glare; });
  return median_of([](const View& v) { return v.factor; });
}

// The level, for the point where it last was.
void Room::Impl::makeLevel() {
  const float f = factor(havePoint ? &points[0] : nullptr, &pointFrames, &pointGlare);
  unclamped = kMedian * radiance * f;
  level = std::clamp(unclamped, kLevelMin, kLevelMax);
}

void Room::make() { d_->make(); }

// The light from parts (seen or not, as P and partSeen give them): unseen
// parts filled from their hemisphere's seen surfaces, the cells, up to two
// lights, SH L2 (unscaled, in tracking space), and the light on an
// up-facing surface. Also the summary's fill and seenAs.
Room::Impl::Built Room::Impl::build(const std::vector<std::array<double, 3>>& P,
                                    const std::vector<bool>& partSeen) {
  Built b;
  std::vector<std::array<double, 3>> L(kCells);
  std::array<bool, kCells> seenCell{};
  std::vector<std::tuple<double, double, int>> hemiParts[2];
  for (int f = 0; f < kParts; ++f) {
    if (!partSeen[f])
      continue;
    float dw;
    const Vec3 d = cube_dir(f, dw, kF);
    hemiParts[d.y >= 0.f ? 0 : 1].emplace_back(lum(P[f]), dw, f);
  }
  // What unseen parts take: their hemisphere's seen surfaces, by solid angle,
  // without its lights (parts over kFillMax times its median). A monitor
  // filling the view ahead turned the whole room its blue, as if it lit
  // every direction the camera hadn't seen. A hemisphere unseen takes half
  // the other's.
  std::array<double, 3> hemi[2]{};
  bool hemiSeen[2]{};
  for (int h = 0; h < 2; ++h) {
    auto& v = hemiParts[h];
    if (v.empty())
      continue;
    std::sort(v.begin(), v.end());
    double total = 0.0;
    for (const auto& [l, dw, f] : v)
      total += dw;
    double half = 0.5 * total, med = std::get<0>(v.back());
    for (const auto& [l, dw, f] : v)
      if ((half -= dw) <= 0.0) {
        med = l;
        break;
      }
    double w = 0.0;
    for (const auto& [l, dw, f] : v) {
      if (l > kFillMax * med)
        break;
      for (int k = 0; k < 3; ++k)
        hemi[h][k] += P[f][k] * dw;
      w += dw;
    }
    if (!(w > 0.0))
      continue;
    for (int k = 0; k < 3; ++k)
      hemi[h][k] /= w;
    hemiSeen[h] = true;
  }
  for (int h = 0; h < 2; ++h)
    if (!hemiSeen[h])
      for (int k = 0; k < 3; ++k)
        hemi[h][k] = hemi[1 - h][k] * 0.5;
  for (int h = 0; h < 2; ++h) {
    const double l = lum(hemi[h]);
    for (int k = 0; k < 3; ++k)
      fill[h][k] = static_cast<float>(l > 0.0 ? hemi[h][k] / l : 0.0);
  }
  // The cells: their parts by solid angle, the unseen ones their hemisphere's.
  std::array<double, kCells> cellW{};
  seenAs = {};
  for (int f = 0; f < kParts; ++f) {
    const int face = f / (kF * kF), j = (f / kF) % kF, i = f % kF;
    const int cell = (face * kN + j / kSub) * kN + i / kSub;
    float dw;
    const Vec3 d = cube_dir(f, dw, kF);
    const auto& v = partSeen[f] ? P[f] : hemi[d.y >= 0.f ? 0 : 1];
    for (int k = 0; k < 3; ++k)
      L[cell][k] += v[k] * dw;
    cellW[cell] += dw;
    if (partSeen[f]) {
      seenCell[cell] = true;
      seenAs[cell][1] += static_cast<float>(parts[f][3]);
      seenAs[cell][2] += static_cast<float>(parts[f][7]);
      seenAs[cell][3] += 1.f;
    }
  }
  for (int i = 0; i < kCells; ++i) {
    for (int k = 0; k < 3; ++k)
      L[i][k] /= cellW[i];
    seenAs[i][0] = static_cast<float>(lum(L[i]));
  }
  // The lights: the brightest seen cells, at least 45 degrees apart, each
  // its excess over the mean of the room taken out of the sky as a
  // directional light of that irradiance over pi.
  const auto unit_dir = [](int cell) {
    float dw;
    return unit(cube_dir(cell, dw, kN));
  };
  double meanLum = 0.0;
  for (int i = 0; i < kCells; ++i)
    meanLum += lum(L[i]) / kCells;
  // The bright spots, brightest first: the bright cells grouped by their
  // bright neighbours (within kMergeCos of one another, chained), so a
  // window and the glare round it are one spot, two lamps apart are two.
  {
    std::vector<std::pair<double, int>> bright;
    for (int i = 0; i < kCells; ++i) {
      const float samples = seenAs[i][1] + seenAs[i][2];
      if (!seenCell[i] || samples < kSpotSamples)
        continue;
      const double v = lum(L[i]);
      if (v > kSpotOver * meanLum || seenAs[i][2] > kSpotClipped * samples)
        bright.emplace_back(v, i);
    }
    std::sort(bright.begin(), bright.end(), std::greater<>());
    std::vector<bool> taken(bright.size());
    for (size_t k = 0; k < bright.size(); ++k) {
      if (taken[k])
        continue;
      const int top = bright[k].second;
      Vec3 dsum{};
      std::vector<size_t> open{k};
      taken[k] = true;
      while (!open.empty()) {
        const size_t at = open.back();
        open.pop_back();
        const int cell = bright[at].second;
        float dw;
        cube_dir(cell, dw, kN);
        dsum = dsum + unit_dir(cell) * static_cast<float>(std::max(bright[at].first - meanLum, 0.1 * meanLum) * dw);
        for (size_t m = k + 1; m < bright.size(); ++m)
          if (!taken[m] && dot(unit_dir(bright[m].second), unit_dir(cell)) > kMergeCos)
            taken[m] = true, open.push_back(m);
      }
      const float samples = seenAs[top][1] + seenAs[top][2];
      const double peak = bright[k].first / std::max(meanLum, 1e-12);
      const bool clipped = seenAs[top][2] > kSpotClipped * samples;
      if (peak > kSpotPeak || clipped)
        b.spots.push_back({unit(rot_y(unit(dsum), yaw)), static_cast<float>(peak), clipped});
    }
  }
  auto& lt = b.lt;
  auto& ld = b.ld;
  auto& picked = b.picked;
  auto& lightCells = b.lightCells;
  for (int l = 0; l < 2; ++l) {
    int best = -1;
    double bestLum = 2.0 * meanLum;
    for (int i = 0; i < kCells; ++i) {
      if (!seenCell[i] || i == picked[0])
        continue;
      if (l == 1 && picked[0] >= 0 && dot(unit_dir(i), unit_dir(picked[0])) > 0.7071f)
        continue;
      if (const double v = lum(L[i]); v > bestLum)
        bestLum = v, best = i;
    }
    if (best < 0)
      break;
    picked[l] = best;
    // With its bright neighbours (within kMergeCos): a light where cells
    // meet (straight up is where four do) would otherwise light by a
    // quarter of it, the rest blurred into the SH.
    const Vec3 bd = unit_dir(best);
    lightCells[l] = {best};
    for (int i = 0; i < kCells; ++i)
      if (i != best && seenCell[i] && lum(L[i]) > 2.0 * meanLum && dot(unit_dir(i), bd) > kMergeCos)
        lightCells[l].push_back(i);
    Vec3 dsum{};
    for (const int cell : lightCells[l]) {
      float dw;
      cube_dir(cell, dw, kN);
      const double exLum = std::max(lum(L[cell]) - meanLum, 0.0);
      dsum = dsum + unit_dir(cell) * static_cast<float>(exLum * dw);
      for (int k = 0; k < 3; ++k) {
        const double excess = std::max(L[cell][k] - meanLum, 0.0);
        lt[l][k] += excess * dw / kPi;
        L[cell][k] -= excess;
      }
    }
    ld[l] = unit(dsum);
  }
  // SH L2 (the cells turned into tracking space).
  constexpr std::array<double, 9> basis{0.282095, 0.488603, 0.488603, 0.488603, 1.092548,
                                        1.092548, 0.315392, 1.092548, 0.546274};
  constexpr std::array<double, 9> lobe{1.0, 2.0 / 3, 2.0 / 3, 2.0 / 3, 0.25, 0.25, 0.25, 0.25, 0.25};
  std::array<std::array<double, 3>, 9> acc{};
  for (int i = 0; i < kCells; ++i) {
    float dw;
    const Vec3 dv = unit(rot_y(cube_dir(i, dw, kN), yaw));
    const double x = dv.x, y = dv.y, z = dv.z;
    const std::array<double, 9> poly{1.0, y, z, x, x * y, y * z, 3.0 * z * z - 1.0, x * z, x * x - y * y};
    for (int k = 0; k < 9; ++k)
      for (int ch = 0; ch < 3; ++ch)
        acc[k][ch] += L[i][ch] * basis[k] * poly[k] * dw;
  }
  auto& s = b.s;
  for (int k = 0; k < 9; ++k)
    for (int ch = 0; ch < 3; ++ch)
      s[k][ch] = acc[k][ch] * lobe[k] * basis[k];
  // Scaled so an up-facing surface gets 1.
  std::array<double, 3> up;
  for (int ch = 0; ch < 3; ++ch)
    up[ch] = s[0][ch] + s[1][ch] - s[6][ch] - s[8][ch] + std::max(ld[0].y, 0.f) * lt[0][ch] +
             std::max(ld[1].y, 0.f) * lt[1][ch];
  b.upLum = lum(up);
  // The room's mean light without its lights, as the level (the walls'
  // median) leaves them out: with them, a stage far from a light found by
  // the capture lost its share of the head's light (2026-10-23).
  b.mean = lum(s[0]);
  return b;
}

void Room::Impl::make() {
  const auto t0 = std::chrono::steady_clock::now();
  struct Timed {
    std::chrono::steady_clock::time_point t0;
    float &ms, &max;
    ~Timed() {
      ms = std::chrono::duration<float, std::milli>(std::chrono::steady_clock::now() - t0).count();
      max = std::max(max, ms);
    }
  } timed{t0, makeMs, makeMaxMs};
  // The parts' radiance as the head saw them.
  std::vector<std::array<double, 3>> P(kParts);
  std::vector<bool> partSeen(kParts);
  double seenW = 0.0, seenLum = 0.0;
  std::vector<std::pair<double, double>> byLum; // the seen parts' luminance and solid angle
  for (int f = 0; f < kParts; ++f) {
    const auto& e = parts[f];
    if (!(e[3] + e[7] > 0.0))
      continue;
    partSeen[f] = true;
    float dw;
    cube_dir(f, dw, kF);
    for (int k = 0; k < 3; ++k)
      P[f][k] = (e[k] + e[4 + k]) / (e[3] + e[7]);
    seenW += dw;
    const double l = lum(P[f]);
    seenLum += l * dw;
    byLum.emplace_back(l, dw);
  }
  if (!(seenW > 0.0))
    return;
  // The reference: the light as the head saw it, by which everything is
  // scaled (an up-facing surface there gets 1) and the level calibrated.
  // Then with voxels the light from the point: near surfaces where they are,
  // in the reference's units, so content nearer a light is lit brighter and
  // farther dimmer (normalized at the point, a light's falloff cancelled:
  // the fighters' mean and the stage's light from above were the light's own
  // near a monitor, and farther away the rest of them brightened).
  const Built head = build(P, partSeen);
  spots = head.spots;
  if (!(head.upLum > 1e-12))
    return;
  const double scale = 1.0 / head.upLum;
  const float median = factor(nullptr, nullptr, nullptr);
  display = static_cast<float>(head.upLum * median);
  seen = static_cast<float>(seenLum / seenW * median);
  std::sort(byLum.begin(), byLum.end());
  double half = 0.5 * seenW, mid = byLum.back().first;
  for (const auto& [l, dw] : byLum)
    if ((half -= dw) <= 0.0) {
      mid = l;
      break;
    }
  middle = static_cast<float>(mid * median);
  // The level from the walls as seen, not from above: a light in or out of
  // the camera's view (it sees ~36 degrees up and down) moves the light from
  // above 3x, the room the same.
  radiance = static_cast<float>(mid);
  makeLevel();
  mean = static_cast<float>(head.mean * scale);
  // Each point's light, from there (the voxels drawn into a copy of the
  // head's parts).
  Vec3 headMean{};
  for (const auto& v : views)
    headMean = headMean + v.head * (1.f / static_cast<float>(views.size()));
  const size_t count = std::max<size_t>(points.size(), 1);
  at.resize(count);
  float share0 = 0.f;
  for (size_t pi = 0; pi < count; ++pi) {
    At& a = at[pi];
    const Vec3 here = evalPoint(pi);
    std::vector<std::array<double, 3>> Pp = P;
    std::vector<bool> seenP = partSeen;
    std::vector<Vec3> partPos(kParts);
    std::vector<bool> fromVoxel(kParts);
    splat(here, Pp, seenP, partPos, fromVoxel);
    if (pi == 0)
      share0 = voxelShare;
    const Built st = voxelShare > 0.f ? build(Pp, seenP) : head;
    a.upAt = static_cast<float>(st.upLum * scale);
    for (int k = 0; k < 9; ++k)
      for (int ch = 0; ch < 3; ++ch)
        a.sh[k][ch] = static_cast<float>(st.s[k][ch] * scale);
    for (int l = 0; l < 2; ++l) {
      for (int ch = 0; ch < 3; ++ch)
        a.light[l][ch] = static_cast<float>(st.lt[l][ch] * scale);
      a.lightDir[l] = rot_y(st.ld[l], yaw);
    }
    // Where the lights are: a voxel's, where the voxels of its parts are, by
    // brightness; else from the depth hits within 10 degrees of it (none: at
    // a distance, as daylight through a window, whose glass depth doesn't
    // see), its range from where the head was. A placed light's range is
    // where its colour was measured: the point's distance for what the
    // point's voxels saw of it, the head's for what the parts no voxel
    // covers kept of the head's view (a window's glass, which depth sees
    // through: its colour was the head's at every point, and its range from
    // the point cancelled its falloff, 2026-10-23).
    for (int l = 0; l < 2; ++l) {
      a.lightAt[l] = false;
      a.lightHits[l] = 0;
      a.lightRange[l] = 0.f;
      a.lightVoxelShare[l] = 0.f;
      if (st.picked[l] < 0)
        continue;
      const auto& cells = st.lightCells[l];
      double vw = 0.0, all = 0.0;
      Vec3 vp{};
      for (int f = 0; f < kParts; ++f) {
        const int face = f / (kF * kF), j = (f / kF) % kF, i = f % kF;
        const int cell = (face * kN + j / kSub) * kN + i / kSub;
        if (std::find(cells.begin(), cells.end(), cell) == cells.end())
          continue;
        const double w = lum(Pp[f]);
        all += w;
        if (!fromVoxel[f])
          continue;
        vp = vp + partPos[f] * static_cast<float>(w);
        vw += w;
      }
      if (vw > 0.0) {
        vp = vp * static_cast<float>(1.0 / vw);
        a.lightPos[l] = fromCapture(vp);
        a.lightAt[l] = true;
        const float share = all > 0.0 ? static_cast<float>(vw / all) : 1.f;
        a.lightVoxelShare[l] = share;
        const float dHere2 = dot(vp - here, vp - here);
        const float dHead2 = views.empty() ? dHere2 : dot(vp - headMean, vp - headMean);
        a.lightRange[l] = std::sqrt(share * dHere2 + (1.f - share) * dHead2);
        continue;
      }
      const Vec3 u = st.ld[l];
      double sum[3]{};
      int n = 0;
      for (const auto& h : hits)
        if (h[0] * u.x + h[1] * u.y + h[2] * u.z > 0.9848f)
          sum[0] += h[3], sum[1] += h[4], sum[2] += h[5], ++n;
      a.lightHits[l] = n;
      if (n < 2)
        continue;
      const Vec3 hp{static_cast<float>(sum[0] / n), static_cast<float>(sum[1] / n), static_cast<float>(sum[2] / n)};
      a.lightPos[l] = fromCapture(hp);
      a.lightAt[l] = true;
      a.lightRange[l] = views.empty() ? 0.f : std::sqrt(dot(hp - headMean, hp - headMean));
    }
  }
  voxelShare = share0;
  coverage = static_cast<float>(seenW / (4.0 * kPi));
  made = true;
  stale = false;
}

// Where the light is made for: the point last asked about, or where the
// head was on average.
Vec3 Room::Impl::evalPoint(size_t i) const {
  if (havePoint && i < points.size())
    return toCapture(points[i]);
  Vec3 head{};
  for (const auto& v : views)
    head = head + v.head * (1.f / static_cast<float>(views.size()));
  return head;
}

// The voxels into the parts seen from `at` (the capture's frame): each
// voxel covers the parts within its angular size, the nearest winning.
void Room::Impl::splat(Vec3 at, std::vector<std::array<double, 3>>& P, std::vector<bool>& partSeen,
                       std::vector<Vec3>& partPos, std::vector<bool>& fromVoxel) {
  voxelShare = 0.f;
  if (voxels.empty())
    return;
  static const std::vector<Vec3> dirs = [] {
    std::vector<Vec3> v(kParts);
    for (int f = 0; f < kParts; ++f) {
      float dw;
      v[f] = unit(cube_dir(f, dw, kF));
    }
    return v;
  }();
  struct Seen {
    Vec3 dir, pos;
    float dist;
    std::array<double, 3> rgb;
  };
  std::vector<Seen> list;
  list.reserve(voxels.size());
  for (const auto& [key, v] : voxels) {
    const double n = v.e[3] + v.e[7];
    if (n < kVoxelMinSamples || !(v.n > 0.0))
      continue;
    const Vec3 pos{static_cast<float>(v.p[0] / v.n), static_cast<float>(v.p[1] / v.n),
                   static_cast<float>(v.p[2] / v.n)};
    const Vec3 rel = pos - at;
    const float dist = std::sqrt(dot(rel, rel));
    if (dist < kNearest)
      continue;
    list.push_back({rel * (1.f / dist), pos, dist,
                    {(v.e[0] + v.e[4]) / n, (v.e[1] + v.e[5]) / n, (v.e[2] + v.e[6]) / n}});
  }
  constexpr float kPartAngle = 1.5707963f / kF;
  const auto each_part = [&](const Seen& v, auto&& fn) {
    const float r = std::atan(kVoxel * 0.6f / v.dist);
    const float cosMax = std::cos(r + kPartAngle * 0.71f);
    const int f0 = cube_cell(v.dir, kF);
    if (f0 < 0)
      return;
    const int face = f0 / (kF * kF), j0 = (f0 / kF) % kF, i0 = f0 % kF;
    const int rp = static_cast<int>(std::ceil(r / kPartAngle));
    for (int j = std::max(j0 - rp, 0); j <= std::min(j0 + rp, kF - 1); ++j)
      for (int i = std::max(i0 - rp, 0); i <= std::min(i0 + rp, kF - 1); ++i) {
        const int f = (face * kF + j) * kF + i;
        if (f == f0 || dot(dirs[f], v.dir) >= cosMax)
          fn(f);
      }
  };
  std::vector<float> nearest(kParts, std::numeric_limits<float>::infinity());
  for (const Seen& v : list)
    each_part(v, [&](int f) { nearest[f] = std::min(nearest[f], v.dist); });
  std::vector<std::array<double, 4>> sum(kParts);
  std::vector<Vec3> pos(kParts);
  for (const Seen& v : list)
    each_part(v, [&](int f) {
      if (v.dist > nearest[f] * kDepthTol)
        return;
      for (int k = 0; k < 3; ++k)
        sum[f][k] += v.rgb[k];
      sum[f][3] += 1.0;
      pos[f] = pos[f] + v.pos;
    });
  int covered = 0;
  for (int f = 0; f < kParts; ++f) {
    if (!(sum[f][3] > 0.0))
      continue;
    for (int k = 0; k < 3; ++k)
      P[f][k] = sum[f][k] / sum[f][3];
    partSeen[f] = true;
    fromVoxel[f] = true;
    partPos[f] = pos[f] * static_cast<float>(1.0 / sum[f][3]);
    ++covered;
  }
  voxelShare = static_cast<float>(covered) / kParts;
}

void Room::addDepth(const DepthImage& im) {
  auto& c = *d_;
  if (!im.depth || im.width <= 0 || im.height <= 0)
    return;
  const int stride = im.stride > 0 ? im.stride : im.width;
  const int step = std::max(1, std::max(im.width, im.height) / kDepthGrid);
  c.depth.clear();
  for (int y = step / 2; y < im.height; y += step)
    for (int x = step / 2; x < im.width; x += step) {
      const float z = depth_linear(im.depth[static_cast<size_t>(y) * stride + x], im.nearZ, im.farZ);
      if (!(z >= kDepthNear && z <= kDepthFar))
        continue;
      // Rows from the bottom.
      const float u = (x + 0.5f) / im.width, w = (y + 0.5f) / im.height;
      const Vec3 pv{(im.tanLeft + u * (im.tanRight - im.tanLeft)) * z, (im.tanDown + w * (im.tanUp - im.tanDown)) * z,
                    -z};
      c.depth.push_back(c.toCapture(im.view.position + rotate(im.view.orientation, pv)));
    }
  c.depthNs = im.timeNs;
}

FrameResult Room::addFrame(const Frame& fr) {
  auto& c = *d_;
  FrameResult out;
  if (!c.haveCamera || fr.cols <= 0 || fr.rows <= 0 || !fr.cells || !(fr.exposureMs > 0.f && fr.iso > 0.f))
    return out;
  out.used = true;
  const Camera& k = c.cam;
  // Relative radiance: over exposure times sensitivity.
  const float scale = 100.f / (fr.exposureMs * fr.iso);
  const float sx = static_cast<float>(k.sensorWidth) / k.width, sy = static_cast<float>(k.sensorHeight) / k.height;
  // Each cell's ray: OpenCV-style in the camera (x right, y down, z ahead),
  // into the head by the lens rotation's inverse, into tracking space by the
  // head's orientation, then into the capture's frame. The lens's few cm
  // from the head are left out: the room's light is at a distance.
  const Quat lensInv = conj(k.lensRotation);
  const auto ray = [&](float px, float py) {
    const Vec3 cam{(px - k.cx) / k.fx, (py - k.cy) / k.fy, 1.f};
    return c.dirToCapture(rotate(fr.head.orientation, rotate(lensInv, cam)));
  };
  const size_t n = static_cast<size_t>(fr.cols) * fr.rows;
  std::vector<int> cells(n, -1);
  std::vector<Vec3> rays(n);
  std::vector<float> lums(n);
  double sumAll = 0.0;
  for (int gy = 0; gy < fr.rows; ++gy)
    for (int gx = 0; gx < fr.cols; ++gx) {
      const int i = gy * fr.cols + gx;
      rays[i] = unit(ray((gx + 0.5f) * k.width / fr.cols * sx, (gy + 0.5f) * k.height / fr.rows * sy));
      cells[i] = cube_cell(rays[i], kF);
      lums[i] = lumf(fr.cells + i * 4);
      sumAll += lums[i];
    }
  // The samples' distances: the latest depth image's points projected into
  // the frame (from the head, in the capture's frame), each cell's median.
  const Vec3 headAt = c.toCapture(fr.head.position);
  std::vector<float> dist(n, 0.f);
  if (!c.depth.empty() && (c.depthNs == 0 || std::abs(fr.timeNs - c.depthNs) <= kDepthFreshNs)) {
    const Quat toHead = conj(mul(yaw_quat(-c.yaw), fr.head.orientation));
    std::vector<std::vector<float>> in(n);
    for (const Vec3& p : c.depth) {
      const Vec3 rel = p - headAt;
      const Vec3 cv = rotate(k.lensRotation, rotate(toHead, rel));
      if (!(cv.z > 0.05f))
        continue;
      const float px = k.fx * cv.x / cv.z + k.cx, py = k.fy * cv.y / cv.z + k.cy;
      const int gx = static_cast<int>(px / k.sensorWidth * fr.cols), gy = static_cast<int>(py / k.sensorHeight * fr.rows);
      if (gx < 0 || gx >= fr.cols || gy < 0 || gy >= fr.rows)
        continue;
      in[static_cast<size_t>(gy) * fr.cols + gx].push_back(std::sqrt(dot(rel, rel)));
    }
    for (size_t i = 0; i < n; ++i)
      if (auto& v = in[i]; !v.empty()) {
        std::nth_element(v.begin(), v.begin() + v.size() / 2, v.end());
        dist[i] = v[v.size() / 2];
      }
  }
  // The frame's glare: its mean over its mean without the light.
  std::vector<float> sorted = lums;
  std::nth_element(sorted.begin(), sorted.begin() + sorted.size() / 2, sorted.end());
  const float med = sorted[sorted.size() / 2];
  double sumRoom = 0.0;
  int nRoom = 0;
  for (const float l : lums)
    if (l <= 2.f * med)
      sumRoom += l, ++nRoom;
  const float glare =
      nRoom > 0 && sumRoom > 0.0
          ? std::clamp(std::sqrt(static_cast<float>(sumAll / n / (sumRoom / nRoom))), 1.f, kGlareMax)
          : 1.f;
  // The room changed: the samples against the parts well seen, by their median.
  std::vector<float> ratios;
  for (size_t i = 0; i < n; ++i) {
    const int cell = cells[i];
    if (cell < 0 || fr.cells[i * 4 + 3] > kClipped)
      continue;
    const auto& e = c.parts[cell];
    if (e[3] + e[7] < kChangeSeen)
      continue;
    const double was = (0.2126 * (e[0] + e[4]) + 0.7152 * (e[1] + e[5]) + 0.0722 * (e[2] + e[6])) / (e[3] + e[7]);
    if (was > 1e-6)
      ratios.push_back(static_cast<float>(lums[i] * scale / was));
  }
  if (ratios.size() >= kChangeMinSamples) {
    std::nth_element(ratios.begin(), ratios.begin() + ratios.size() / 2, ratios.end());
    const float r = ratios[ratios.size() / 2];
    out.change = r;
    if (!(r > kChange || r < 1.0 / kChange))
      c.changedNs = 0;
    else if (c.changedNs == 0)
      c.changedNs = fr.timeNs;
    if (c.changedNs != 0 && fr.timeNs - c.changedNs >= kChangeNs) {
      c.parts.assign(kParts, {});
      c.views.clear();
      c.voxels.clear();
      c.frames = c.sinceMade = 0;
      c.changedNs = 0;
      c.spots.clear();
      out.restarted = true;
    }
  }
  for (size_t i = 0; i < n; ++i) {
    const int cell = cells[i];
    if (cell < 0)
      continue;
    const float* p = fr.cells + i * 4;
    const bool clipped = p[3] > kClipped;
    const double s = scale * (clipped ? 1.0 + kClipBoost * p[3] : 1.0);
    const int o = clipped ? 4 : 0;
    auto& e = c.parts[cell];
    for (int ch = 0; ch < 3; ++ch)
      e[o + ch] += static_cast<double>(p[ch]) * s;
    e[o + 3] += 1.0;
    if (e[3] + e[7] > kPartMax) {
      const double keep = kPartMax / (e[3] + e[7]);
      for (double& x : e)
        x *= keep;
    }
    if (!(dist[i] > 0.f))
      continue;
    const Vec3 at = headAt + rays[i] * dist[i];
    const int64_t key = voxel_key(at);
    auto it = c.voxels.find(key);
    if (it == c.voxels.end()) {
      if (c.voxels.size() >= kVoxelsMax)
        continue;
      it = c.voxels.emplace(key, Impl::Voxel{}).first;
    }
    auto& v = it->second;
    for (int ch = 0; ch < 3; ++ch)
      v.e[o + ch] += static_cast<double>(p[ch]) * s;
    v.e[o + 3] += 1.0;
    v.p[0] += at.x, v.p[1] += at.y, v.p[2] += at.z;
    v.n += 1.0;
    if (v.e[3] + v.e[7] > kVoxelMax) {
      const double keep = kVoxelMax / (v.e[3] + v.e[7]);
      for (double& x : v.e)
        x *= keep;
      for (double& x : v.p)
        x *= keep;
      v.n *= keep;
    }
  }
  // Where the head was and the camera's middle looked, for the level.
  c.views.push_back({fr.exposureMs * fr.iso / 100.f * glare, c.toCapture(fr.head.position),
                     unit(ray(k.sensorWidth * 0.5f, k.sensorHeight * 0.5f)), glare});
  if (c.views.size() > kViewsMax)
    c.views.erase(c.views.begin(), c.views.begin() + kViewsMax / 2);
  ++c.frames;
  if (++c.sinceMade >= kMakeEvery || c.stale) {
    c.sinceMade = 0;
    c.make();
  }
  return out;
}

void Room::addHit(Vec3 origin, Vec3 point) {
  auto& c = *d_;
  const Vec3 d = point - origin;
  const float n = std::sqrt(dot(d, d));
  if (!(n > 0.05f))
    return;
  const Vec3 u = c.dirToCapture(d * (1.f / n)), q = c.toCapture(point);
  const std::array<float, 6> h{u.x, u.y, u.z, q.x, q.y, q.z};
  if (c.hits.size() < kHitsMax)
    c.hits.push_back(h);
  else
    c.hits[c.hitNext++ % c.hits.size()] = h;
}

bool Room::moved(Vec3 p0, float y0, Vec3 p1, float y1) {
  auto& c = *d_;
  const float dy = y1 - y0;
  c.pos = p1 + rot_y(c.pos - p0, dy);
  c.yaw += dy;
  for (auto& a : c.at)
    for (int l = 0; l < 2; ++l) {
      a.lightPos[l] = p1 + rot_y(a.lightPos[l] - p0, dy);
      a.lightDir[l] = rot_y(a.lightDir[l], dy);
    }
  for (Vec3& p : c.points)
    p = p1 + rot_y(p - p0, dy);
  // A turn of a degree or more: the SH turned with the parts.
  if (c.made && std::abs(dy) > 0.0175f) {
    c.make();
    return true;
  }
  return false;
}

void Room::setPoints(const Vec3* pts, size_t n) {
  auto& c = *d_;
  if (n == 0 || !pts)
    return;
  // A point moved (5 cm): the level from the frames looking at the first;
  // with voxels the light is stale, made from there by the next frame or
  // make() (a make costs milliseconds, too much for a render thread).
  bool moved = !c.havePoint || c.points.size() != n;
  for (size_t i = 0; i < n && !moved; ++i) {
    const Vec3 d = pts[i] - c.points[i];
    moved = dot(d, d) > 0.0025f;
  }
  if (!moved)
    return;
  c.havePoint = true;
  c.points.assign(pts, pts + n);
  if (c.voxels.empty())
    c.makeLevel();
  else
    c.stale = true;
}

bool Room::lightAt(size_t i, Lighting& out) const {
  const auto& c = *d_;
  if (!c.made)
    return false;
  const Impl::At& a = c.at[std::min(i, c.at.size() - 1)];
  const bool have = c.havePoint && i < c.points.size();
  out.sh = a.sh;
  for (int l = 0; l < 2; ++l) {
    Light& o = out.lights[l];
    o.color = a.light[l];
    o.dir = a.lightDir[l];
    o.placed = a.lightAt[l];
    o.pos = a.lightPos[l];
    o.range = 0.f;
    if (!a.lightAt[l] || !have)
      continue;
    // A light with a place: toward it from the point, by the inverse square
    // of its distance from where its colour was measured (until a make
    // measures it from here): a ceiling light a table sits farther from than
    // the head did lit it as strongly.
    const Vec3 d = a.lightPos[l] - c.points[i];
    const float n2 = dot(d, d);
    if (n2 > 0.0025f)
      o.dir = d * (1.f / std::sqrt(n2));
    if (a.lightRange[l] > 0.f && n2 >= 0.01f) {
      const float k = std::clamp(a.lightRange[l] * a.lightRange[l] / n2, 0.2f, 2.f);
      for (float& x : o.color)
        x *= k;
    }
    o.range = std::sqrt(n2);
  }
  out.mean = c.mean;
  out.level = c.level;
  return true;
}

bool Room::lightAt(Vec3 point, Lighting& out) {
  setPoints(&point, 1);
  return lightAt(size_t{0}, out);
}

// The brightest cells: where, how bright (relative to the mean), how many of
// their samples clipped, and how many of their parts were seen.
std::string Room::Impl::brightest() const {
  std::array<std::pair<float, int>, kCells> order;
  double m = 0.0;
  for (int i = 0; i < kCells; ++i) {
    order[i] = {seenAs[i][3] > 0.f ? seenAs[i][0] : 0.f, i};
    m += seenAs[i][0] / kCells;
  }
  std::partial_sort(order.begin(), order.begin() + 6, order.end(), std::greater<>());
  std::string out;
  char buf[160];
  for (int k = 0; k < 6 && order[k].first > 0.f; ++k) {
    float dw;
    const Vec3 d = unit(cube_dir(order[k].second, dw, kN));
    const auto& e = seenAs[order[k].second];
    std::snprintf(buf, sizeof buf, " %.2f,%.2f,%.2f x%.1f (%.0f clear, %.0f clipped, %.0f/%d parts);", d.x, d.y, d.z,
                  order[k].first / std::max(m, 1e-12), e[1], e[2], e[3], kSub * kSub);
    out += buf;
  }
  return out;
}

std::string Room::summary() const {
  const auto& c = *d_;
  const Impl::At& a0 = c.at[0];
  const auto at = [&](int l) {
    char b[48];
    if (!a0.lightAt[l])
      return std::string{"no"};
    std::snprintf(b, sizeof b, "%.2f,%.2f,%.2f (%.0f%% from voxels)", a0.lightPos[l].x, a0.lightPos[l].y,
                  a0.lightPos[l].z, a0.lightVoxelShare[l] * 100.f);
    return std::string{b};
  };
  char exposure[48];
  if (c.pointFrames > 0)
    std::snprintf(exposure, sizeof exposure, "%d frames looking at the point", c.pointFrames);
  else
    std::snprintf(exposure, sizeof exposure, "all frames");
  char buf[1024];
  std::snprintf(buf, sizeof buf,
                "%.0f%% of the room seen; lights %.2f,%.2f,%.2f from %.2f,%.2f,%.2f and %.2f,%.2f,%.2f from "
                "%.2f,%.2f,%.2f (placed: %s, %s; %zu hits kept, %d and %d near the lights; %zu voxels, %.0f%% of "
                "the directions from them; made in %.1f ms, at most %.1f); passthrough's up %.4f, "
                "seen %.4f, median %.4f, level %.2f (%.2f unclamped, exposure from %s, glare %.2f); light from above at the point %.2f; unseen as "
                "%.2f,%.2f,%.2f above, %.2f,%.2f,%.2f below; brightest cells (x,y,z in the capture; times the "
                "mean):",
                c.coverage * 100.f, a0.light[0][0], a0.light[0][1], a0.light[0][2], a0.lightDir[0].x, a0.lightDir[0].y,
                a0.lightDir[0].z, a0.light[1][0], a0.light[1][1], a0.light[1][2], a0.lightDir[1].x, a0.lightDir[1].y,
                a0.lightDir[1].z, at(0).c_str(), at(1).c_str(), c.hits.size(), a0.lightHits[0], a0.lightHits[1],
                c.voxels.size(), c.voxelShare * 100.f, c.makeMs, c.makeMaxMs, c.display, c.seen, c.middle, c.level, c.unclamped, exposure, c.pointGlare, a0.upAt, c.fill[0][0],
                c.fill[0][1], c.fill[0][2], c.fill[1][0], c.fill[1][1], c.fill[1][2]);
  return buf + c.brightest();
}

} // namespace roomtone

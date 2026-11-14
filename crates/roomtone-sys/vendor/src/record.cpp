// A recording: "RTONE001", then records of a type byte, a u32 size and the
// payload, little-endian as the machines that make and read them are.
#include "roomtone/record.hpp"

#include <cstring>
#include <vector>

namespace roomtone {
namespace {

constexpr char kMagic[8] = {'R', 'T', 'O', 'N', 'E', '0', '0', '1'};
enum : uint8_t { kReset = 1, kCamera, kFrame, kHit, kDepth, kMoved, kQuery, kPoints };

struct FrameHeader {
  int32_t cols, rows;
  float exposureMs, iso;
  Pose head;
  int64_t timeNs;
};
struct DepthHeader {
  int32_t width, height;
  float nearZ, farZ, tanLeft, tanRight, tanUp, tanDown;
  Pose view;
  int64_t timeNs;
};
struct MovedRecord {
  Vec3 p0;
  float yaw0;
  Vec3 p1;
  float yaw1;
};
struct QueryRecord {
  Vec3 point;
  int64_t timeNs;
};

} // namespace

Recorder::~Recorder() { close(); }

bool Recorder::open(const std::string& path) {
  close();
  f_ = std::fopen(path.c_str(), "wb");
  if (f_)
    std::fwrite(kMagic, 1, sizeof kMagic, f_);
  return f_ != nullptr;
}

void Recorder::close() {
  if (f_)
    std::fclose(f_);
  f_ = nullptr;
}

void Recorder::put2(uint8_t type, const void* a, uint32_t na, const void* b, uint32_t nb) {
  if (!f_)
    return;
  const uint32_t size = na + nb;
  std::fwrite(&type, 1, 1, f_);
  std::fwrite(&size, 4, 1, f_);
  if (na)
    std::fwrite(a, 1, na, f_);
  if (nb)
    std::fwrite(b, 1, nb, f_);
}
void Recorder::put(uint8_t type, const void* data, uint32_t size) { put2(type, data, size, nullptr, 0); }

void Recorder::reset() { put(kReset, nullptr, 0); }
void Recorder::camera(const Camera& c) { put(kCamera, &c, sizeof c); }
void Recorder::frame(const Frame& f) {
  if (!f.cells || f.cols <= 0 || f.rows <= 0)
    return;
  const FrameHeader h{f.cols, f.rows, f.exposureMs, f.iso, f.head, f.timeNs};
  put2(kFrame, &h, sizeof h, f.cells, static_cast<uint32_t>(f.cols * f.rows * 4 * sizeof(float)));
  std::fflush(f_); // an app is often killed, not closed
}
void Recorder::hit(Vec3 origin, Vec3 point) {
  const Vec3 v[2] = {origin, point};
  put(kHit, v, sizeof v);
}
void Recorder::depth(const DepthImage& d) {
  if (!d.depth || d.width <= 0 || d.height <= 0)
    return;
  const DepthHeader h{d.width, d.height, d.nearZ, d.farZ, d.tanLeft, d.tanRight, d.tanUp, d.tanDown, d.view, d.timeNs};
  std::vector<uint16_t> packed(static_cast<size_t>(d.width) * d.height);
  const int stride = d.stride > 0 ? d.stride : d.width;
  for (int y = 0; y < d.height; ++y)
    std::memcpy(packed.data() + static_cast<size_t>(y) * d.width, d.depth + static_cast<size_t>(y) * stride,
                d.width * sizeof(uint16_t));
  put2(kDepth, &h, sizeof h, packed.data(), static_cast<uint32_t>(packed.size() * sizeof(uint16_t)));
}
void Recorder::moved(Vec3 p0, float yaw0, Vec3 p1, float yaw1) {
  const MovedRecord m{p0, yaw0, p1, yaw1};
  put(kMoved, &m, sizeof m);
}
void Recorder::query(Vec3 point, int64_t timeNs) {
  const QueryRecord q{point, timeNs};
  put(kQuery, &q, sizeof q);
}
void Recorder::points(const Vec3* pts, size_t count, int64_t timeNs) {
  if (!pts || count == 0)
    return;
  put2(kPoints, &timeNs, sizeof timeNs, pts, static_cast<uint32_t>(count * sizeof(Vec3)));
}

bool replay(const std::string& path, Room& room, const std::function<void(const ReplayEvent&)>& onEvent) {
  std::FILE* f = std::fopen(path.c_str(), "rb");
  if (!f)
    return false;
  char magic[8];
  if (std::fread(magic, 1, 8, f) != 8 || std::memcmp(magic, kMagic, 8) != 0) {
    std::fclose(f);
    return false;
  }
  std::vector<uint8_t> buf;
  for (;;) {
    uint8_t type;
    uint32_t size;
    if (std::fread(&type, 1, 1, f) != 1 || std::fread(&size, 4, 1, f) != 1)
      break;
    buf.resize(size);
    if (size && std::fread(buf.data(), 1, size, f) != size)
      break;
    ReplayEvent e{};
    switch (type) {
    case kReset:
      room.reset();
      e.kind = ReplayEvent::Reset;
      break;
    case kCamera: {
      if (size != sizeof(Camera))
        continue;
      Camera c;
      std::memcpy(&c, buf.data(), sizeof c);
      room.setCamera(c);
      e.kind = ReplayEvent::Camera;
      break;
    }
    case kFrame: {
      if (size < sizeof(FrameHeader))
        continue;
      FrameHeader h;
      std::memcpy(&h, buf.data(), sizeof h);
      if (size != sizeof h + static_cast<size_t>(h.cols) * h.rows * 4 * sizeof(float))
        continue;
      std::vector<float> cells(static_cast<size_t>(h.cols) * h.rows * 4);
      std::memcpy(cells.data(), buf.data() + sizeof h, cells.size() * sizeof(float));
      const Frame fr{cells.data(), h.cols, h.rows, h.exposureMs, h.iso, h.head, h.timeNs};
      const FrameResult r = room.addFrame(fr);
      e.kind = ReplayEvent::Frame;
      e.timeNs = h.timeNs;
      e.frame = &r;
      onEvent(e);
      continue;
    }
    case kHit: {
      if (size != 2 * sizeof(Vec3))
        continue;
      Vec3 v[2];
      std::memcpy(v, buf.data(), sizeof v);
      room.addHit(v[0], v[1]);
      e.kind = ReplayEvent::Hit;
      break;
    }
    case kDepth: {
      if (size < sizeof(DepthHeader))
        continue;
      DepthHeader h;
      std::memcpy(&h, buf.data(), sizeof h);
      if (size != sizeof h + static_cast<size_t>(h.width) * h.height * sizeof(uint16_t))
        continue;
      std::vector<uint16_t> px(static_cast<size_t>(h.width) * h.height);
      std::memcpy(px.data(), buf.data() + sizeof h, px.size() * sizeof(uint16_t));
      DepthImage d;
      d.depth = px.data();
      d.width = h.width, d.height = h.height, d.stride = h.width;
      d.nearZ = h.nearZ, d.farZ = h.farZ;
      d.tanLeft = h.tanLeft, d.tanRight = h.tanRight, d.tanUp = h.tanUp, d.tanDown = h.tanDown;
      d.view = h.view;
      d.timeNs = h.timeNs;
      room.addDepth(d);
      e.kind = ReplayEvent::Depth;
      break;
    }
    case kMoved: {
      if (size != sizeof(MovedRecord))
        continue;
      MovedRecord m;
      std::memcpy(&m, buf.data(), sizeof m);
      room.moved(m.p0, m.yaw0, m.p1, m.yaw1);
      e.kind = ReplayEvent::Moved;
      break;
    }
    case kQuery: {
      if (size != sizeof(QueryRecord))
        continue;
      QueryRecord q;
      std::memcpy(&q, buf.data(), sizeof q);
      e.kind = ReplayEvent::Query;
      e.point = q.point;
      e.timeNs = q.timeNs;
      break;
    }
    case kPoints: {
      if (size < sizeof(int64_t) + sizeof(Vec3) || (size - sizeof(int64_t)) % sizeof(Vec3) != 0)
        continue;
      std::vector<Vec3> pts((size - sizeof(int64_t)) / sizeof(Vec3));
      std::memcpy(&e.timeNs, buf.data(), sizeof(int64_t));
      std::memcpy(pts.data(), buf.data() + sizeof(int64_t), pts.size() * sizeof(Vec3));
      room.setPoints(pts.data(), pts.size());
      e.kind = ReplayEvent::Points;
      e.point = pts[0];
      break;
    }
    default:
      continue;
    }
    onEvent(e);
  }
  std::fclose(f);
  return true;
}

} // namespace roomtone

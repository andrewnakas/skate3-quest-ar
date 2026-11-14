// Recording a Room's inputs, and playing them back: a session on the
// headset, replayed on a desktop (tools/roomtone_replay), tunes and tests
// the estimation without the headset.
#pragma once

#include "roomtone/roomtone.hpp"

#include <cstdio>
#include <functional>
#include <string>

namespace roomtone {

class Recorder {
public:
  Recorder() = default;
  ~Recorder();
  Recorder(const Recorder&) = delete;
  Recorder& operator=(const Recorder&) = delete;

  bool open(const std::string& path);
  bool isOpen() const { return f_ != nullptr; }
  void close();
  void reset();
  void camera(const Camera& c);
  void frame(const Frame& f);
  void hit(Vec3 origin, Vec3 point);
  void depth(const DepthImage& d);
  void moved(Vec3 p0, float yaw0, Vec3 p1, float yaw1);
  // A lightAt call's point (replay asks there too).
  void query(Vec3 point, int64_t timeNs);
  // A setPoints call's points (the first is the replay's query).
  void points(const Vec3* points, size_t count, int64_t timeNs);

private:
  void put(uint8_t type, const void* data, uint32_t size);
  void put2(uint8_t type, const void* a, uint32_t na, const void* b, uint32_t nb);
  std::FILE* f_ = nullptr;
};

// What a replay saw at each recorded query.
struct ReplayEvent {
  enum Kind { Reset, Camera, Frame, Hit, Depth, Moved, Query, Points } kind;
  int64_t timeNs = 0;      // a frame's or a query's
  Vec3 point;              // a query's (the first of a Points')
  const FrameResult* frame = nullptr; // a frame's result
};

// Feeds a recording into room; onEvent after each event. False if it
// couldn't be read (a short last record is fine: the recording was cut).
bool replay(const std::string& path, Room& room, const std::function<void(const ReplayEvent&)>& onEvent);

} // namespace roomtone

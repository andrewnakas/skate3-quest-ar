// The C interface (roomtone.h): each call converts its arguments and
// forwards to the Room or the Recorder.
#include "roomtone/roomtone.h"

#include "roomtone/record.hpp"
#include "roomtone/roomtone.hpp"

#include <algorithm>
#include <cstring>
#include <vector>

struct roomtone_room {
  roomtone::Room room;
};
struct roomtone_recorder {
  roomtone::Recorder recorder;
};

namespace {

roomtone::Vec3 vec(roomtone_vec3 v) { return {v.x, v.y, v.z}; }
roomtone_vec3 vec(roomtone::Vec3 v) { return {v.x, v.y, v.z}; }
roomtone::Quat quat(roomtone_quat q) { return {q.x, q.y, q.z, q.w}; }
roomtone::Pose pose(const roomtone_pose& p) { return {quat(p.orientation), vec(p.position)}; }

roomtone::Camera camera(const roomtone_camera& c) {
  roomtone::Camera out;
  out.lensRotation = quat(c.lens_rotation);
  out.fx = c.fx;
  out.fy = c.fy;
  out.cx = c.cx;
  out.cy = c.cy;
  out.sensorWidth = c.sensor_width;
  out.sensorHeight = c.sensor_height;
  out.width = c.width;
  out.height = c.height;
  return out;
}

roomtone::Frame frame(const roomtone_frame& f) {
  roomtone::Frame out;
  out.cells = f.cells;
  out.cols = f.cols;
  out.rows = f.rows;
  out.exposureMs = f.exposure_ms;
  out.iso = f.iso;
  out.head = pose(f.head);
  out.timeNs = f.time_ns;
  return out;
}

roomtone::DepthImage depth(const roomtone_depth_image& d) {
  roomtone::DepthImage out;
  out.depth = d.depth;
  out.width = d.width;
  out.height = d.height;
  out.stride = d.stride;
  out.nearZ = d.near_z;
  out.farZ = d.far_z;
  out.tanLeft = d.tan_left;
  out.tanRight = d.tan_right;
  out.tanUp = d.tan_up;
  out.tanDown = d.tan_down;
  out.view = pose(d.view);
  out.timeNs = d.time_ns;
  return out;
}

void lighting(const roomtone::Lighting& l, roomtone_lighting& out) {
  for (int i = 0; i < 9; ++i)
    for (int c = 0; c < 3; ++c)
      out.sh[i][c] = l.sh[i][c];
  for (int i = 0; i < 2; ++i) {
    const roomtone::Light& from = l.lights[i];
    roomtone_light& to = out.lights[i];
    for (int c = 0; c < 3; ++c)
      to.color[c] = from.color[c];
    to.dir = vec(from.dir);
    to.placed = from.placed;
    to.pos = vec(from.pos);
    to.range = from.range;
  }
  out.mean = l.mean;
  out.level = l.level;
}

std::vector<roomtone::Vec3> points(const roomtone_vec3* p, size_t count) {
  std::vector<roomtone::Vec3> out(count);
  for (size_t i = 0; i < count; ++i)
    out[i] = vec(p[i]);
  return out;
}

} // namespace

extern "C" {

roomtone_room* roomtone_room_new(void) { return new roomtone_room; }
void roomtone_room_free(roomtone_room* room) { delete room; }

void roomtone_room_reset(roomtone_room* room) { room->room.reset(); }
void roomtone_room_set_camera(roomtone_room* room, const roomtone_camera* c) { room->room.setCamera(camera(*c)); }
int roomtone_room_has_camera(const roomtone_room* room) { return room->room.hasCamera(); }

roomtone_frame_result roomtone_room_add_frame(roomtone_room* room, const roomtone_frame* f) {
  const roomtone::FrameResult r = room->room.addFrame(frame(*f));
  return {r.used, r.restarted, r.change, r.black};
}

void roomtone_room_add_hit(roomtone_room* room, roomtone_vec3 origin, roomtone_vec3 point) {
  room->room.addHit(vec(origin), vec(point));
}
void roomtone_room_add_depth(roomtone_room* room, const roomtone_depth_image* image) {
  room->room.addDepth(depth(*image));
}
int roomtone_room_moved(roomtone_room* room, roomtone_vec3 p0, float yaw0, roomtone_vec3 p1, float yaw1) {
  return room->room.moved(vec(p0), yaw0, vec(p1), yaw1);
}
void roomtone_room_make(roomtone_room* room) { room->room.make(); }
int roomtone_room_made(const roomtone_room* room) { return room->room.made(); }
uint32_t roomtone_room_frames(const roomtone_room* room) { return room->room.frames(); }
size_t roomtone_room_hits(const roomtone_room* room) { return room->room.hits(); }
size_t roomtone_room_voxels(const roomtone_room* room) { return room->room.voxels(); }
int roomtone_room_stale(const roomtone_room* room) { return room->room.stale(); }
float roomtone_room_seen_median(const roomtone_room* room) { return room->room.seenMedian(); }

void roomtone_room_set_points(roomtone_room* room, const roomtone_vec3* p, size_t count) {
  const std::vector<roomtone::Vec3> ps = points(p, count);
  room->room.setPoints(ps.data(), ps.size());
}

int roomtone_room_light_at_index(const roomtone_room* room, size_t index, roomtone_lighting* out) {
  roomtone::Lighting l;
  if (!room->room.lightAt(index, l))
    return 0;
  lighting(l, *out);
  return 1;
}

int roomtone_room_light_at(roomtone_room* room, roomtone_vec3 point, roomtone_lighting* out) {
  roomtone::Lighting l;
  if (!room->room.lightAt(vec(point), l))
    return 0;
  lighting(l, *out);
  return 1;
}

size_t roomtone_room_spots(const roomtone_room* room, roomtone_spot* out, size_t capacity) {
  const std::vector<roomtone::Spot>& spots = room->room.spots();
  for (size_t i = 0; i < std::min(spots.size(), capacity); ++i)
    out[i] = {vec(spots[i].dir), spots[i].brightness, spots[i].clipped};
  return spots.size();
}

size_t roomtone_room_summary(const roomtone_room* room, char* buffer, size_t capacity) {
  const std::string s = room->room.summary();
  if (capacity) {
    const size_t n = std::min(s.size(), capacity - 1);
    std::memcpy(buffer, s.data(), n);
    buffer[n] = '\0';
  }
  return s.size();
}

int roomtone_replay_file(const char* path, roomtone_room* room) {
  return roomtone::replay(path, room->room, [](const roomtone::ReplayEvent&) {});
}

roomtone_recorder* roomtone_recorder_open(const char* path) {
  roomtone_recorder* r = new roomtone_recorder;
  if (!r->recorder.open(path)) {
    delete r;
    return nullptr;
  }
  return r;
}
void roomtone_recorder_close(roomtone_recorder* recorder) { delete recorder; }
void roomtone_recorder_reset(roomtone_recorder* recorder) { recorder->recorder.reset(); }
void roomtone_recorder_camera(roomtone_recorder* recorder, const roomtone_camera* c) {
  recorder->recorder.camera(camera(*c));
}
void roomtone_recorder_frame(roomtone_recorder* recorder, const roomtone_frame* f) {
  recorder->recorder.frame(frame(*f));
}
void roomtone_recorder_hit(roomtone_recorder* recorder, roomtone_vec3 origin, roomtone_vec3 point) {
  recorder->recorder.hit(vec(origin), vec(point));
}
void roomtone_recorder_depth(roomtone_recorder* recorder, const roomtone_depth_image* image) {
  recorder->recorder.depth(depth(*image));
}
void roomtone_recorder_moved(roomtone_recorder* recorder, roomtone_vec3 p0, float yaw0, roomtone_vec3 p1,
                             float yaw1) {
  recorder->recorder.moved(vec(p0), yaw0, vec(p1), yaw1);
}
void roomtone_recorder_points(roomtone_recorder* recorder, const roomtone_vec3* p, size_t count, int64_t time_ns) {
  const std::vector<roomtone::Vec3> ps = points(p, count);
  recorder->recorder.points(ps.data(), ps.size(), time_ns);
}

} // extern "C"

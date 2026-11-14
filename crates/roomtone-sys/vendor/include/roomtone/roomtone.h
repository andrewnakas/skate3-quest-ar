/* Roomtone's C interface: roomtone.hpp's Room and record.hpp's Recorder for
 * callers that aren't C++ (Rust, C#, C). The types, units and rules are
 * roomtone.hpp's, which documents them: tracking space in metres, +Y up;
 * a room isn't thread-safe (callers serialize). */
#ifndef ROOMTONE_H
#define ROOMTONE_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct roomtone_room roomtone_room;
typedef struct roomtone_recorder roomtone_recorder;

typedef struct roomtone_vec3 {
  float x, y, z;
} roomtone_vec3;

/* The identity is 0, 0, 0, 1. */
typedef struct roomtone_quat {
  float x, y, z, w;
} roomtone_quat;

typedef struct roomtone_pose {
  roomtone_quat orientation;
  roomtone_vec3 position;
} roomtone_pose;

/* roomtone::Camera. */
typedef struct roomtone_camera {
  roomtone_quat lens_rotation;
  float fx, fy, cx, cy;
  int32_t sensor_width, sensor_height;
  int32_t width, height;
} roomtone_camera;

/* roomtone::Frame: cells is cols * rows * 4 floats (linear r, g, b and the
 * share clipped), row-major from the top; read during the call only. */
typedef struct roomtone_frame {
  const float* cells;
  int32_t cols, rows;
  float exposure_ms, iso;
  roomtone_pose head;
  int64_t time_ns;
} roomtone_frame;

/* roomtone::DepthImage; read during the call only. far_z 0: infinite. */
typedef struct roomtone_depth_image {
  const uint16_t* depth;
  int32_t width, height, stride;
  float near_z, far_z;
  float tan_left, tan_right, tan_up, tan_down;
  roomtone_pose view;
  int64_t time_ns;
} roomtone_depth_image;

/* roomtone::Light. */
typedef struct roomtone_light {
  float color[3];
  roomtone_vec3 dir;
  int32_t placed; /* 0 or 1 */
  roomtone_vec3 pos;
  float range;
} roomtone_light;

/* roomtone::Lighting. */
typedef struct roomtone_lighting {
  float sh[9][3];
  roomtone_light lights[2];
  float mean;
  float level;
} roomtone_lighting;

/* roomtone::Spot. */
typedef struct roomtone_spot {
  roomtone_vec3 dir;
  float brightness;
  int32_t clipped; /* 0 or 1 */
} roomtone_spot;

/* roomtone::FrameResult. */
typedef struct roomtone_frame_result {
  int32_t used;      /* 0 or 1 */
  int32_t restarted; /* 0 or 1 */
  float change;
  int32_t black;     /* 0 or 1: left out, the image was black */
} roomtone_frame_result;

roomtone_room* roomtone_room_new(void);
void roomtone_room_free(roomtone_room* room); /* NULL is fine */

void roomtone_room_reset(roomtone_room* room);
void roomtone_room_set_camera(roomtone_room* room, const roomtone_camera* camera);
int roomtone_room_has_camera(const roomtone_room* room);
roomtone_frame_result roomtone_room_add_frame(roomtone_room* room, const roomtone_frame* frame);
void roomtone_room_add_hit(roomtone_room* room, roomtone_vec3 origin, roomtone_vec3 point);
void roomtone_room_add_depth(roomtone_room* room, const roomtone_depth_image* image);
int roomtone_room_moved(roomtone_room* room, roomtone_vec3 p0, float yaw0, roomtone_vec3 p1, float yaw1);
void roomtone_room_make(roomtone_room* room);
int roomtone_room_made(const roomtone_room* room);
uint32_t roomtone_room_frames(const roomtone_room* room);
size_t roomtone_room_hits(const roomtone_room* room);
size_t roomtone_room_voxels(const roomtone_room* room);
int roomtone_room_stale(const roomtone_room* room);
float roomtone_room_seen_median(const roomtone_room* room);

/* Room::setPoints, and the light at points[index]: 0 while none is made. */
void roomtone_room_set_points(roomtone_room* room, const roomtone_vec3* points, size_t count);
int roomtone_room_light_at_index(const roomtone_room* room, size_t index, roomtone_lighting* out);
/* One point: set_points(&point, 1), then light_at_index(0). */
int roomtone_room_light_at(roomtone_room* room, roomtone_vec3 point, roomtone_lighting* out);

/* Room::spots: writes up to capacity of them (brightest first) and returns
 * how many there are. */
size_t roomtone_room_spots(const roomtone_room* room, roomtone_spot* out, size_t capacity);

/* Room::summary into buffer (always terminated when capacity > 0); returns
 * the summary's length without the terminator. */
size_t roomtone_room_summary(const roomtone_room* room, char* buffer, size_t capacity);

/* roomtone::replay: feeds a recording into room. 0 if it couldn't be read. */
int roomtone_replay_file(const char* path, roomtone_room* room);

/* roomtone::Recorder: write each input beside the room call it mirrors. */
roomtone_recorder* roomtone_recorder_open(const char* path); /* NULL if it couldn't be opened */
void roomtone_recorder_close(roomtone_recorder* recorder);   /* closes and frees; NULL is fine */
void roomtone_recorder_reset(roomtone_recorder* recorder);
void roomtone_recorder_camera(roomtone_recorder* recorder, const roomtone_camera* camera);
void roomtone_recorder_frame(roomtone_recorder* recorder, const roomtone_frame* frame);
void roomtone_recorder_hit(roomtone_recorder* recorder, roomtone_vec3 origin, roomtone_vec3 point);
void roomtone_recorder_depth(roomtone_recorder* recorder, const roomtone_depth_image* image);
void roomtone_recorder_moved(roomtone_recorder* recorder, roomtone_vec3 p0, float yaw0, roomtone_vec3 p1, float yaw1);
void roomtone_recorder_points(roomtone_recorder* recorder, const roomtone_vec3* points, size_t count,
                              int64_t time_ns);

#ifdef __cplusplus
}
#endif

#endif /* ROOMTONE_H */

/**
 * GLSL ES 3.00 for the WebGL layer. Every program draws instanced quads or
 * strips whose shape lives in static buffers (`scene.ts`); positions are
 * worked out here from a few uniforms, in device pixels, then mapped to clip
 * space. Fragments output premultiplied colour: blending is
 * `ONE, ONE_MINUS_SRC_ALPHA`, as macroquad's alpha blending over black.
 */

import { GROUP_COUNT } from './scene';

const VERTEX_HEADER = `#version 300 es
precision highp float;
uniform vec2 u_view;    // canvas size, device px
uniform vec2 u_center;  // orb centre, device px
uniform float u_size;   // shorter side, device px
uniform float u_unit;   // device px per reference px (the 480 px screen)
uniform float u_time;   // seconds, wrapped at TIME_PERIOD
vec4 clip(vec2 px) {
  vec2 c = px / u_view * 2.0 - 1.0;
  return vec4(c.x, -c.y, 0.0, 1.0);
}
vec2 dirOf(float a) { return vec2(cos(a), sin(a)); }
vec3 whiten(vec3 c, float amount) { return c + (1.0 - c) * amount; }
`;

const FRAGMENT_HEADER = `#version 300 es
precision highp float;
out vec4 o;
`;

/** Soft drifting blobs of background light: three stacked soft discs each. */
export const BLOB_VERTEX = `${VERTEX_HEADER}
uniform float u_wobbleRate;
uniform float u_saturation;
in vec2 a_corner;
in vec4 a_blob;   // distance, angle, drift, wobble phase
in vec4 a_blob2;  // radius, rgb
out vec2 v_uv;
out vec3 v_color;
void main() {
  float wobble = sin(u_time * u_wobbleRate + a_blob.w);
  float angle = a_blob.y + a_blob.z * u_time;
  vec2 centre = u_center + dirOf(angle) * a_blob.x * (1.0 + 0.05 * wobble) * u_size;
  float radius = a_blob2.x * u_size;
  v_uv = a_corner;
  vec3 rgb = a_blob2.yzw;
  float grey = dot(rgb, vec3(0.299, 0.587, 0.114));
  v_color = mix(vec3(grey), rgb, u_saturation);
  gl_Position = clip(centre + a_corner * radius);
}`;

export const BLOB_FRAGMENT = `${FRAGMENT_HEADER}
uniform float u_background;
in vec2 v_uv;
in vec3 v_color;
float disc(float r, float k) { return 1.0 - smoothstep(k * 0.7, k, r); }
void main() {
  float r = length(v_uv);
  float a = (0.03 * disc(r, 1.0) + 0.04 * disc(r, 0.68) + 0.05 * disc(r, 0.38)) * u_background;
  o = vec4(v_color * a, a);
}`;

/** Thin rays from the centre, bright near the middle and fading outwards. */
export const RAY_VERTEX = `${VERTEX_HEADER}
uniform float u_rotation;
uniform float u_start;     // px from the centre where rays begin
uniform float u_base;      // px every ray reaches
uniform float u_span;      // px of extra reach, times the ray's own reach and shimmer
uniform vec2 u_shimmer;    // shimmer = x + y * sin(...)
uniform float u_alpha;
uniform float u_alphaMax;
uniform vec3 u_color;
in vec2 a_corner;
in vec4 a_ray;   // angle, reach, brightness, white
in vec4 a_ray2;  // phase, width px, shimmer rate, unused
out float v_t;
out float v_side;
out float v_half;
out float v_alpha;
out vec3 v_color;
void main() {
  float shimmer = u_shimmer.x + u_shimmer.y * sin(u_time * a_ray2.z + a_ray2.x);
  float len = max(u_start + 1.0, u_base + u_span * a_ray.y * shimmer);
  float t = a_corner.x * 0.5 + 0.5;
  float half_width = a_ray2.y * u_unit * 0.5;
  float drawn = max(half_width, 0.5);
  float extent = drawn + 1.0;
  vec2 dir = dirOf(a_ray.x + u_rotation);
  vec2 normal = vec2(-dir.y, dir.x);
  vec2 pos = u_center + dir * mix(u_start, len, t) + normal * a_corner.y * extent;
  v_t = t;
  v_side = a_corner.y * extent;
  v_half = drawn;
  v_alpha = min(u_alpha * a_ray.z, u_alphaMax) * (half_width / drawn);
  v_color = whiten(u_color, a_ray.w);
  gl_Position = clip(pos);
}`;

export const RAY_FRAGMENT = `${FRAGMENT_HEADER}
uniform float u_split;  // where along the ray the bright part ends, 0 to 1
uniform float u_fade;   // brightness past the split
in float v_t;
in float v_side;
in float v_half;
in float v_alpha;
in vec3 v_color;
void main() {
  float across = 1.0 - smoothstep(v_half - 0.5, v_half + 0.5, abs(v_side));
  float along = mix(1.0, u_fade, smoothstep(u_split - 0.1, u_split + 0.1, v_t)) * (1.0 - smoothstep(0.75, 1.0, v_t));
  float a = v_alpha * across * along;
  o = vec4(v_color * a, a);
}`;

const GROUP_UNIFORMS = `
uniform float u_rot[${GROUP_COUNT}];
uniform float u_scale[${GROUP_COUNT}];
uniform float u_alpha[${GROUP_COUNT}];
uniform vec3 u_color[${GROUP_COUNT}];
`;

/** Segments per arc strip. */
export const ARC_SEGMENTS = 48;

/** HUD arcs, circles and dashes, and the band backing: true circular bands, antialiased on both edges. */
export const ARC_VERTEX = `${VERTEX_HEADER}${GROUP_UNIFORMS}
in vec2 a_strip;  // t along the arc 0 to 1, side -1 (inner) or 1 (outer)
in vec4 a_arc;    // radius, start, length, thickness px
in vec4 a_arc2;   // group, alpha, softness, unused
out vec2 v_local;
out float v_radius;
out float v_half;
out float v_alpha;
out float v_soft;
out vec3 v_color;
void main() {
  int g = int(a_arc2.x + 0.5);
  float radius = a_arc.x * u_size * u_scale[g];
  float half_width = a_arc.w * u_unit * 0.5;
  float drawn = max(half_width, 0.5);
  float extent = drawn + 1.5;
  float step = a_arc.z / ${ARC_SEGMENTS}.0;
  float outer = (radius + extent) / cos(min(step * 0.5, 1.2));
  float r = a_strip.y < 0.0 ? max(radius - extent, 0.0) : outer;
  vec2 local = dirOf(a_arc.y + u_rot[g] + a_strip.x * a_arc.z) * r;
  v_local = local;
  v_radius = radius;
  v_half = drawn;
  v_alpha = u_alpha[g] * a_arc2.y * (half_width / drawn);
  v_soft = a_arc2.z;
  v_color = u_color[g];
  gl_Position = clip(u_center + local);
}`;

export const ARC_FRAGMENT = `${FRAGMENT_HEADER}
in vec2 v_local;
in float v_radius;
in float v_half;
in float v_alpha;
in float v_soft;
in vec3 v_color;
void main() {
  float d = abs(length(v_local) - v_radius);
  float crisp = 1.0 - smoothstep(v_half - 0.5, v_half + 0.5, d);
  float soft = exp(-2.5 * (d / v_half) * (d / v_half));
  float a = v_alpha * mix(crisp, soft, v_soft);
  o = vec4(v_color * a, a);
}`;

/** HUD ticks and notches, and the glyph strokes: antialiased capsules placed in each ring's turning frame. */
export const STROKE_VERTEX = `${VERTEX_HEADER}${GROUP_UNIFORMS}
uniform float u_shimmerRate;
uniform float u_shimmerDepth[${GROUP_COUNT}];
in vec2 a_corner;
in vec4 a_p0;    // angle, base radius, radial offset, along offset
in vec4 a_p1;
in vec4 a_seg;   // width px, group, alpha, shimmer phase
out vec2 v_pos;
flat out vec2 v_a;
flat out vec2 v_b;
out float v_half;
out float v_alpha;
out vec3 v_color;
vec2 place(vec4 p, float rot, float scale) {
  float angle = p.x + rot;
  vec2 outward = dirOf(angle);
  vec2 along = vec2(-outward.y, outward.x);
  return u_center + outward * (p.y * u_size * scale + p.z * u_size) + along * (p.w * u_size);
}
void main() {
  int g = int(a_seg.y + 0.5);
  vec2 a = place(a_p0, u_rot[g], u_scale[g]);
  vec2 b = place(a_p1, u_rot[g], u_scale[g]);
  float half_width = a_seg.x * u_unit * 0.5;
  float drawn = max(half_width, 0.5);
  float extent = drawn + 1.0;
  vec2 delta = b - a;
  float len = length(delta);
  vec2 dir = len > 0.001 ? delta / len : vec2(1.0, 0.0);
  vec2 normal = vec2(-dir.y, dir.x);
  vec2 end = a_corner.x < 0.0 ? a - dir * extent : b + dir * extent;
  vec2 pos = end + normal * a_corner.y * extent;
  float depth = u_shimmerDepth[g];
  float shimmer = 1.0 - depth + depth * sin(u_time * u_shimmerRate + a_seg.w);
  v_pos = pos;
  v_a = a;
  v_b = b;
  v_half = drawn;
  v_alpha = u_alpha[g] * a_seg.z * shimmer * (half_width / drawn);
  v_color = u_color[g];
  gl_Position = clip(pos);
}`;

export const STROKE_FRAGMENT = `${FRAGMENT_HEADER}
in vec2 v_pos;
flat in vec2 v_a;
flat in vec2 v_b;
in float v_half;
in float v_alpha;
in vec3 v_color;
void main() {
  vec2 pa = v_pos - v_a;
  vec2 ba = v_b - v_a;
  float h = clamp(dot(pa, ba) / max(dot(ba, ba), 0.0001), 0.0, 1.0);
  float d = length(pa - ba * h);
  float a = v_alpha * (1.0 - smoothstep(v_half - 0.5, v_half + 0.5, d));
  o = vec4(v_color * a, a);
}`;

/** The warm core: the amber fill, the glowing uneven ring and the white-gold starburst glow, in one quad. */
export const CORE_VERTEX = `${VERTEX_HEADER}
uniform float u_extent;  // half the quad's side, px
in vec2 a_corner;
out vec2 v_local;
void main() {
  v_local = a_corner * u_extent;
  gl_Position = clip(u_center + v_local);
}`;

export const CORE_FRAGMENT = `${FRAGMENT_HEADER}
uniform float u_ring;       // ring radius, px
uniform float u_thickness;  // ring thickness, px
uniform float u_turn;       // ring turn, radians
uniform vec2 u_phases;      // phases of the two waves on the ring's edge
uniform float u_glow;       // the look's core glow, 0 to 1
uniform float u_lum;        // brightness with the breath
uniform vec3 u_core;
uniform vec3 u_fill;
in vec2 v_local;
// Deep amber at 0, the core colour at 0.5 and white-hot at 1.
vec3 heat(float h) {
  h = clamp(h, 0.0, 1.0);
  return h < 0.5 ? mix(u_fill, u_core, h * 2.0) : u_core + (1.0 - u_core) * ((h - 0.5) * 2.0);
}
void over(inout vec4 acc, vec3 color, float a) {
  acc = vec4(color * a, a) + acc * (1.0 - a);
}
float disc(float r, float radius, float soft) { return 1.0 - smoothstep(radius - soft, radius + soft, r); }
void main() {
  float r = length(v_local);
  float theta = atan(v_local.y, v_local.x) - u_turn;
  float wander = 0.6 * sin(5.0 * theta + u_phases.x) + 0.4 * sin(9.0 * theta + u_phases.y);
  float ring = u_ring * (1.0 + 0.022 * wander);
  float strength = u_glow * u_lum;
  vec4 acc = vec4(0.0);

  // Amber fill inside the ring: (radius as a fraction of the ring, alpha).
  float fill_alpha = (0.5 + 0.5 * u_glow) * u_lum;
  over(acc, u_fill, 0.12 * fill_alpha * disc(r, u_ring * 1.0, u_ring * 0.06));
  over(acc, u_fill, 0.10 * fill_alpha * disc(r, u_ring * 0.8, u_ring * 0.08));
  over(acc, u_fill, 0.10 * fill_alpha * disc(r, u_ring * 0.6, u_ring * 0.08));
  over(acc, u_fill, 0.08 * fill_alpha * disc(r, u_ring * 0.4, u_ring * 0.08));

  // The ring's glow, widest first: (width as a multiple of the thickness, alpha, heat).
  float d = abs(r - ring);
  float hot = 0.5 + 0.5 * u_glow;
  over(acc, heat(0.0), 0.03 * strength * disc(d, u_thickness * 2.0, u_thickness * 0.5));
  over(acc, heat(0.05 * hot), 0.04 * strength * disc(d, u_thickness * 1.65, u_thickness * 0.4));
  over(acc, heat(0.10 * hot), 0.05 * strength * disc(d, u_thickness * 1.35, u_thickness * 0.35));
  over(acc, heat(0.18 * hot), 0.06 * strength * disc(d, u_thickness * 1.1, u_thickness * 0.3));
  over(acc, heat(0.28 * hot), 0.08 * strength * disc(d, u_thickness * 0.85, u_thickness * 0.25));
  over(acc, heat(0.40 * hot), 0.12 * strength * disc(d, u_thickness * 0.65, u_thickness * 0.2));
  over(acc, heat(0.55 * hot), 0.20 * strength * disc(d, u_thickness * 0.475, u_thickness * 0.15));
  over(acc, heat(0.75 * hot), 0.35 * strength * disc(d, u_thickness * 0.31, u_thickness * 0.1));
  over(acc, heat(0.92 * hot), 0.55 * strength * disc(d, u_thickness * 0.16, u_thickness * 0.07));

  // The starburst glow, widest first: (radius as a fraction of the ring, alpha, heat).
  float star = u_ring * (0.7 + 0.5 * u_glow);
  over(acc, heat(0.10), 0.05 * strength * disc(r, star * 0.78, star * 0.12));
  over(acc, heat(0.20), 0.07 * strength * disc(r, star * 0.62, star * 0.1));
  over(acc, heat(0.35), 0.10 * strength * disc(r, star * 0.48, star * 0.08));
  over(acc, heat(0.50), 0.14 * strength * disc(r, star * 0.36, star * 0.06));
  over(acc, heat(0.65), 0.20 * strength * disc(r, star * 0.26, star * 0.05));
  over(acc, heat(0.80), 0.32 * strength * disc(r, star * 0.17, star * 0.04));
  over(acc, heat(0.92), 0.55 * strength * disc(r, star * 0.10, star * 0.03));
  over(acc, heat(0.98), 0.90 * strength * disc(r, star * 0.055, star * 0.02));
  o = acc;
}`;

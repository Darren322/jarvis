/**
 * The WebGL2 layer behind the body: the visual-jarvis D1 touches from the
 * approved macroquad orb (`crates/jarvis-orb/src/orb.rs`). Back to front:
 * drifting blobs of light, long light rays, the cool JARVIS HUD rings, the
 * band of invented glyphs, the warm Great Sage core with its glowing ring,
 * and the starburst rays. No text.
 *
 * Six instanced draw calls a frame from static buffers; a frame sets uniforms
 * only and allocates nothing.
 */

import type { Look } from '../look';
import type { OrbFrame } from '../renderer';
import { createProgram, createVertexArray, uniformLocations } from './program';
import {
  ARC_SEGMENTS,
  ARC_FRAGMENT,
  ARC_VERTEX,
  BLOB_FRAGMENT,
  BLOB_VERTEX,
  CORE_FRAGMENT,
  CORE_VERTEX,
  RAY_FRAGMENT,
  RAY_VERTEX,
  STROKE_FRAGMENT,
  STROKE_VERTEX,
} from './shaders';
import {
  ARC_STRIDE,
  BACKING_GROUP,
  BLOB_STRIDE,
  BLOB_WOBBLE_RATE,
  buildScene,
  GLYPH_GROUP,
  GLYPH_SHIMMER_RATE,
  GROUP_COUNT,
  HUD_GROUPS,
  HUD_RINGS,
  RAY_STRIDE,
  STROKE_STRIDE,
  TIME_PERIOD,
} from './scene';

/** The screen height every reference-pixel size is written for. */
const REFERENCE_SIZE = 480;
/** Brightness swing of one breath, as a fraction. */
const BREATH_BRIGHTNESS = 0.15;
/** The whole orb swells by this fraction of its radius at a full pulse. */
const PULSE_SWELL = 0.04;

// Light rays.
const RAY_START = 0.03;
const RAY_MIN_LENGTH = 0.45;
const RAY_MAX_LENGTH = 1.25;
const RAY_BASE_ALPHA = 0.22;
const RAY_STRENGTH_ALPHA = 0.78;
const RAY_MAX_ALPHA = 0.6;
const RAY_SPLIT = 0.35;
const RAY_OUTER_FADE = 0.45;

// HUD.
const HUD_FLARE_BASE = 0.6;
const HUD_FLARE_GAIN = 0.8;
const BAND_BACKING_ALPHA = 0.22;
const GLYPH_SHIMMER_DEPTH = 0.15;

// Core.
export const CORE_RING_RADIUS = 0.3;
const CORE_RING_THICKNESS = 1 / 6;
const CORE_RING_TURN_RATIO = 0.25;
/** Half the side of the core quad, as a multiple of the ring radius: wide enough for the outermost glow. */
const CORE_EXTENT = 1.6;
const STAR_RAY_START = 0.03;
const STAR_RAY_REACH = 0.85;
/** Lower than macroquad's 0.55: the body's dots now share the centre with these rays. */
const STAR_RAY_ALPHA = 0.4;

/** The four corners of a quad, as a triangle strip, from -1 to 1. */
const QUAD = new Float32Array([-1, -1, 1, -1, -1, 1, 1, 1]);

/** A strip along an arc: (t from 0 to 1, side -1 or 1) per vertex. */
function arcStrip(): Float32Array {
  const strip = new Float32Array((ARC_SEGMENTS + 1) * 4);
  for (let step = 0; step <= ARC_SEGMENTS; step += 1) {
    strip.set([step / ARC_SEGMENTS, -1, step / ARC_SEGMENTS, 1], step * 4);
  }
  return strip;
}

/** One ray draw's settings; one object is reused for every frame. */
interface RayParams {
  rotation: number;
  start: number;
  base: number;
  span: number;
  shimmerBase: number;
  shimmerDepth: number;
  alpha: number;
  alphaMax: number;
  split: number;
  fade: number;
  color: readonly [number, number, number];
}

interface Layer {
  readonly program: WebGLProgram;
  readonly vao: WebGLVertexArrayObject;
  readonly uniforms: Map<string, WebGLUniformLocation>;
  readonly instances: number;
}

export class Backdrop {
  readonly #gl: WebGL2RenderingContext;
  readonly #blobs: Layer;
  readonly #rays: Layer;
  readonly #starRays: Layer;
  readonly #arcs: Layer;
  readonly #strokes: Layer;
  readonly #core: Layer;
  readonly #ringPhases: readonly [number, number];
  readonly #programs: WebGLProgram[];
  readonly #buffers: WebGLBuffer[] = [];
  readonly #vaos: WebGLVertexArrayObject[] = [];
  // Per-group uniforms, filled each frame.
  readonly #rot = new Float32Array(GROUP_COUNT);
  readonly #scale = new Float32Array(GROUP_COUNT);
  readonly #alpha = new Float32Array(GROUP_COUNT);
  readonly #color = new Float32Array(GROUP_COUNT * 3);
  readonly #shimmerDepth = new Float32Array(GROUP_COUNT);
  readonly #ray: RayParams = {
    rotation: 0,
    start: 0,
    base: 0,
    span: 0,
    shimmerBase: 0,
    shimmerDepth: 0,
    alpha: 0,
    alphaMax: 0,
    split: 0,
    fade: 0,
    color: [0, 0, 0],
  };

  private constructor(gl: WebGL2RenderingContext, programs: Record<'blob' | 'ray' | 'arc' | 'stroke' | 'core', WebGLProgram>) {
    this.#gl = gl;
    this.#programs = Object.values(programs);
    const scene = buildScene();
    this.#ringPhases = scene.ringPhases;
    const quad = { data: QUAD, attributes: [{ name: 'a_corner', size: 2 }], perInstance: false };
    this.#blobs = this.#layer(programs.blob, [
      quad,
      { data: scene.blobs, attributes: [{ name: 'a_blob', size: 4 }, { name: 'a_blob2', size: 4 }], perInstance: true },
    ], scene.blobs.length / BLOB_STRIDE);
    const rayAttributes = [{ name: 'a_ray', size: 4 }, { name: 'a_ray2', size: 4 }];
    this.#rays = this.#layer(programs.ray, [quad, { data: scene.rays, attributes: rayAttributes, perInstance: true }], scene.rays.length / RAY_STRIDE);
    this.#starRays = this.#layer(
      programs.ray,
      [quad, { data: scene.starRays, attributes: rayAttributes, perInstance: true }],
      scene.starRays.length / RAY_STRIDE,
    );
    this.#arcs = this.#layer(programs.arc, [
      { data: arcStrip(), attributes: [{ name: 'a_strip', size: 2 }], perInstance: false },
      { data: scene.arcs, attributes: [{ name: 'a_arc', size: 4 }, { name: 'a_arc2', size: 4 }], perInstance: true },
    ], scene.arcs.length / ARC_STRIDE);
    this.#strokes = this.#layer(programs.stroke, [
      quad,
      {
        data: scene.strokes,
        attributes: [{ name: 'a_p0', size: 4 }, { name: 'a_p1', size: 4 }, { name: 'a_seg', size: 4 }],
        perInstance: true,
      },
    ], scene.strokes.length / STROKE_STRIDE);
    this.#core = this.#layer(programs.core, [quad], 1);
    this.#shimmerDepth[GLYPH_GROUP] = GLYPH_SHIMMER_DEPTH;
  }

  /** The backdrop for `gl`, or null if a shader fails (logged). */
  static create(gl: WebGL2RenderingContext): Backdrop | null {
    const blob = createProgram(gl, BLOB_VERTEX, BLOB_FRAGMENT);
    const ray = createProgram(gl, RAY_VERTEX, RAY_FRAGMENT);
    const arc = createProgram(gl, ARC_VERTEX, ARC_FRAGMENT);
    const stroke = createProgram(gl, STROKE_VERTEX, STROKE_FRAGMENT);
    const core = createProgram(gl, CORE_VERTEX, CORE_FRAGMENT);
    if (blob === null || ray === null || arc === null || stroke === null || core === null) {
      return null;
    }
    return new Backdrop(gl, { blob, ray, arc, stroke, core });
  }

  #layer(program: WebGLProgram, buffers: Parameters<typeof createVertexArray>[2], instances: number): Layer {
    const { vao, buffers: created } = createVertexArray(this.#gl, program, buffers);
    this.#buffers.push(...created);
    this.#vaos.push(vao);
    return { program, vao, uniforms: uniformLocations(this.#gl, program), instances };
  }

  /** Draws one frame into the whole drawing buffer of `width` by `height` device pixels. */
  draw(frame: OrbFrame, width: number, height: number): void {
    const gl = this.#gl;
    if (gl.isContextLost()) {
      return;
    }
    const look = frame.look;
    const size = Math.min(width, height);
    const breath = Math.sin(frame.breathPhase);
    const swell = look.breathDepth * breath + look.nodePulse * PULSE_SWELL;
    const lum = look.brightness * (1 + BREATH_BRIGHTNESS * breath);
    const time = frame.clock % TIME_PERIOD;

    gl.viewport(0, 0, width, height);
    gl.clearColor(0, 0, 0, 1);
    gl.clear(gl.COLOR_BUFFER_BIT);
    gl.enable(gl.BLEND);
    gl.blendFunc(gl.ONE, gl.ONE_MINUS_SRC_ALPHA);

    if (look.background >= 0.01) {
      const u = this.#use(this.#blobs, width, height, size, time);
      gl.uniform1f(u.get('u_wobbleRate') ?? null, BLOB_WOBBLE_RATE);
      gl.uniform1f(u.get('u_saturation') ?? null, look.blobSaturation);
      gl.uniform1f(u.get('u_background') ?? null, look.background);
      this.#drawQuads(this.#blobs);
    }

    if (look.rayStrength >= 0.01) {
      const reachPx = Math.hypot(width, height) / 2;
      const reach = Math.sqrt(look.rayStrength);
      const u = this.#use(this.#rays, width, height, size, time);
      const ray = this.#ray;
      ray.rotation = frame.rayAngle;
      ray.start = size * RAY_START;
      ray.base = reachPx * RAY_MIN_LENGTH * (1 - reach);
      ray.span = reachPx * RAY_MAX_LENGTH * reach;
      ray.shimmerBase = 0.8;
      ray.shimmerDepth = 0.2;
      ray.alpha = (RAY_BASE_ALPHA + RAY_STRENGTH_ALPHA * look.rayStrength) * lum;
      ray.alphaMax = RAY_MAX_ALPHA;
      ray.split = RAY_SPLIT;
      ray.fade = RAY_OUTER_FADE;
      ray.color = look.coreColor;
      this.#rayUniforms(u, ray);
      this.#drawQuads(this.#rays);
    }

    this.#fillGroups(look, frame, swell, lum);
    this.#drawGroups(this.#arcs, width, height, size, time, false);
    this.#drawGroups(this.#strokes, width, height, size, time, true);

    const ring = size * CORE_RING_RADIUS * (1 + swell);
    {
      const u = this.#use(this.#core, width, height, size, time);
      gl.uniform1f(u.get('u_extent') ?? null, ring * CORE_EXTENT);
      gl.uniform1f(u.get('u_ring') ?? null, ring);
      gl.uniform1f(u.get('u_thickness') ?? null, ring * CORE_RING_THICKNESS * (0.8 + 0.4 * look.coreGlow));
      gl.uniform1f(u.get('u_turn') ?? null, frame.sparkleAngle * CORE_RING_TURN_RATIO);
      gl.uniform2f(u.get('u_phases') ?? null, this.#ringPhases[0], this.#ringPhases[1]);
      gl.uniform1f(u.get('u_glow') ?? null, look.coreGlow);
      gl.uniform1f(u.get('u_lum') ?? null, lum);
      gl.uniform3f(u.get('u_core') ?? null, look.coreColor[0], look.coreColor[1], look.coreColor[2]);
      gl.uniform3f(u.get('u_fill') ?? null, look.fillColor[0], look.fillColor[1], look.fillColor[2]);
      this.#drawQuads(this.#core);
    }

    const strength = look.coreGlow * lum;
    if (strength >= 0.01) {
      const u = this.#use(this.#starRays, width, height, size, time);
      const ray = this.#ray;
      ray.rotation = frame.sparkleAngle;
      ray.start = ring * STAR_RAY_START;
      ray.base = 0;
      ray.span = ring * STAR_RAY_REACH * (0.7 + 0.5 * look.coreGlow);
      ray.shimmerBase = 0.75;
      ray.shimmerDepth = 0.25;
      ray.alpha = STAR_RAY_ALPHA * strength;
      ray.alphaMax = 1;
      ray.split = 1;
      ray.fade = 1;
      ray.color = look.coreColor;
      this.#rayUniforms(u, ray);
      this.#drawQuads(this.#starRays);
    }
    gl.bindVertexArray(null);
  }

  /** Frees every GPU object this backdrop made. */
  dispose(): void {
    const gl = this.#gl;
    for (const vao of this.#vaos) {
      gl.deleteVertexArray(vao);
    }
    for (const buffer of this.#buffers) {
      gl.deleteBuffer(buffer);
    }
    for (const program of this.#programs) {
      gl.deleteProgram(program);
    }
  }

  #use(layer: Layer, width: number, height: number, size: number, time: number): Map<string, WebGLUniformLocation> {
    const gl = this.#gl;
    const u = layer.uniforms;
    gl.useProgram(layer.program);
    gl.bindVertexArray(layer.vao);
    gl.uniform2f(u.get('u_view') ?? null, width, height);
    gl.uniform2f(u.get('u_center') ?? null, width / 2, height / 2);
    gl.uniform1f(u.get('u_size') ?? null, size);
    gl.uniform1f(u.get('u_unit') ?? null, size / REFERENCE_SIZE);
    gl.uniform1f(u.get('u_time') ?? null, time);
    return u;
  }

  #drawQuads(layer: Layer): void {
    this.#gl.drawArraysInstanced(this.#gl.TRIANGLE_STRIP, 0, 4, layer.instances);
  }

  #rayUniforms(u: Map<string, WebGLUniformLocation>, ray: Readonly<RayParams>): void {
    const gl = this.#gl;
    gl.uniform1f(u.get('u_rotation') ?? null, ray.rotation);
    gl.uniform1f(u.get('u_start') ?? null, ray.start);
    gl.uniform1f(u.get('u_base') ?? null, ray.base);
    gl.uniform1f(u.get('u_span') ?? null, ray.span);
    gl.uniform2f(u.get('u_shimmer') ?? null, ray.shimmerBase, ray.shimmerDepth);
    gl.uniform1f(u.get('u_alpha') ?? null, ray.alpha);
    gl.uniform1f(u.get('u_alphaMax') ?? null, ray.alphaMax);
    gl.uniform1f(u.get('u_split') ?? null, ray.split);
    gl.uniform1f(u.get('u_fade') ?? null, ray.fade);
    gl.uniform3f(u.get('u_color') ?? null, ray.color[0], ray.color[1], ray.color[2]);
  }

  /** Each group's turn, swell, opacity and colour for this frame. */
  #fillGroups(look: Readonly<Look>, frame: OrbFrame, swell: number, lum: number): void {
    const hudBase = look.hudAlpha * lum;
    const bandBase = look.bandAlpha * lum;
    for (let group = 0; group < HUD_GROUPS; group += 1) {
      const ring = HUD_RINGS[group];
      const flare = ring?.flare === true ? HUD_FLARE_BASE + HUD_FLARE_GAIN * look.nodeJitter : 1;
      this.#rot[group] = frame.hudAngle * (ring?.spin ?? 0);
      this.#alpha[group] = hudBase < 0.01 ? 0 : hudBase * (ring?.alpha ?? 0) * flare;
      this.#color.set(look.hudColor, group * 3);
    }
    this.#rot[GLYPH_GROUP] = frame.bandAngle;
    this.#alpha[GLYPH_GROUP] = bandBase < 0.01 ? 0 : bandBase;
    this.#color.set(look.bandColor, GLYPH_GROUP * 3);
    this.#rot[BACKING_GROUP] = 0;
    this.#alpha[BACKING_GROUP] = bandBase < 0.01 ? 0 : bandBase * BAND_BACKING_ALPHA;
    this.#color.set(look.fillColor, BACKING_GROUP * 3);
    this.#scale.fill(1 + swell);
  }

  #drawGroups(layer: Layer, width: number, height: number, size: number, time: number, strokes: boolean): void {
    const gl = this.#gl;
    const u = this.#use(layer, width, height, size, time);
    gl.uniform1fv(u.get('u_rot') ?? null, this.#rot);
    gl.uniform1fv(u.get('u_scale') ?? null, this.#scale);
    gl.uniform1fv(u.get('u_alpha') ?? null, this.#alpha);
    gl.uniform3fv(u.get('u_color') ?? null, this.#color);
    if (strokes) {
      gl.uniform1f(u.get('u_shimmerRate') ?? null, GLYPH_SHIMMER_RATE);
      gl.uniform1fv(u.get('u_shimmerDepth') ?? null, this.#shimmerDepth);
      gl.drawArraysInstanced(gl.TRIANGLE_STRIP, 0, 4, layer.instances);
    } else {
      gl.drawArraysInstanced(gl.TRIANGLE_STRIP, 0, (ARC_SEGMENTS + 1) * 2, layer.instances);
    }
  }
}

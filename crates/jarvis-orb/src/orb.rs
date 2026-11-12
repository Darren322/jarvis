//! Draws the orb from a [`Look`].
//!
//! The picture, back to front: light rays fanning out from the centre; an outer
//! lattice of points and links with a smaller lattice turning the opposite way
//! inside it; two tilted rings of invented glyphs; and a warm glow at the core.
//! No text is ever drawn. Node positions, links and glyph geometry are built once
//! in [`Orb::new`]; a frame only reuses one scratch buffer.

use std::f32::consts::{PI, TAU};

use jarvis_orb::look::{Look, Rgb};
use macroquad::prelude::*;

/// The screen height every pixel size below is written for.
const REFERENCE_SIZE: f32 = 480.0;
/// Radius of the outer lattice as a fraction of the screen's shorter side.
const SPHERE_RADIUS_FRACTION: f32 = 0.38;
/// Fixed forward lean of the lattices, so the turn shows depth.
const SPHERE_TILT: f32 = 0.38;
/// The longest step the animation takes in one frame, in seconds. A stall (for
/// example the window being dragged) must not make the orb jump.
const MAX_STEP_SECONDS: f32 = 0.1;
/// Seed for node placement noise, so the orb is the same on every start.
const SEED: u64 = 0x0b_1a_57;

const OUTER_NODES: usize = 260;
const INNER_NODES: usize = 80;
/// Each node links to this many nearest neighbours.
const LINKS_PER_NODE: usize = 3;
const INNER_RADIUS: f32 = 0.55;
/// The inner lattice turns this many times faster than the outer, the other way.
const INNER_SPIN_RATIO: f32 = 1.7;

const NODE_BASE_PX: f32 = 1.3;
const NODE_DEPTH_PX: f32 = 2.2;
const NODE_SIDES: u8 = 8;
/// A node at the back of the sphere is this fraction as bright as one at the front.
const BACK_DIMMING: f32 = 0.25;
const LINK_ALPHA: f32 = 0.55;
const LINK_WIDTH_PX: f32 = 1.0;
/// How much a fully pulsing node grows, as a fraction of its size.
const PULSE_SIZE_GAIN: f32 = 1.2;
const PULSE_BRIGHTNESS_GAIN: f32 = 0.35;
/// Speed of the ripple that runs through pulsing nodes, radians per second.
const PULSE_WAVE_RATE: f32 = 7.0;
/// How far a fully jittering node moves in and out, as a fraction of the radius.
const JITTER_AMPLITUDE: f32 = 0.10;
const WOBBLE_RATE_MIN: f32 = 4.0;
const WOBBLE_RATE_MAX: f32 = 9.0;
/// The whole orb swells by this fraction of the radius at a full node pulse.
const PULSE_SWELL: f32 = 0.04;
/// Brightness swing of one breath, as a fraction.
const BREATH_BRIGHTNESS: f32 = 0.15;

const RAY_COUNT: usize = 28;
const RAY_SEGMENTS: usize = 3;
const RAY_START: f32 = 0.16;
const RAY_MIN_LENGTH: f32 = 0.55;
const RAY_EXTRA_LENGTH: f32 = 0.85;
const RAY_ALPHA: f32 = 0.55;
const RAY_WIDTH_PX: f32 = 1.6;
const RAY_TURN_RATE: f32 = 0.05;
const RAY_SHIMMER_RATE: f32 = 2.2;

const CORE_LAYERS: usize = 9;
const CORE_RADIUS: f32 = 0.11;
const CORE_OUTER_SCALE: f32 = 3.4;
const CORE_INNER_SCALE: f32 = 0.45;
const CORE_LAYER_ALPHA_MIN: f32 = 0.04;
const CORE_LAYER_ALPHA_MAX: f32 = 0.16;
const CORE_WHITENING: f32 = 0.65;
const CORE_SIDES: u8 = 32;

const GLYPH_TRACK_STEPS: usize = 48;
const GLYPH_LINE_PX: f32 = 1.5;
const GLYPH_DOT_RADIUS: f32 = 0.16;
const GLYPH_DOT_SIDES: u8 = 6;
const ARC_STEPS: usize = 6;
/// The ring track lines sit this far (in glyph half-sizes) beyond the glyphs.
const TRACK_MARGIN: f32 = 1.35;
const TRACK_ALPHA: f32 = 0.35;
/// A glyph at the back of its ring is this fraction as bright as one at the front.
const RING_BACK_DIMMING: f32 = 0.30;

/// A glyph stroke in a box running -1 to 1 across (x, along the ring) and from the
/// ring's centre side to its outer side (y). Angles are in degrees.
#[derive(Clone, Copy)]
enum Stroke {
    Line(f32, f32, f32, f32),
    Arc {
        cx: f32,
        cy: f32,
        r: f32,
        from: f32,
        to: f32,
    },
    Dot(f32, f32),
}

use Stroke::{Arc, Dot, Line};

/// The invented alphabet: ten marks made of lines, arcs and dots. They stand for
/// nothing and are not drawn from any real script.
const GLYPHS: [&[Stroke]; 10] = [
    // A tine with two unequal branches and a dot above.
    &[
        Line(0.0, -1.0, 0.0, 0.6),
        Line(0.0, 0.1, -0.7, 0.7),
        Line(0.0, -0.2, 0.7, 0.35),
        Dot(0.0, 0.95),
    ],
    // A crescent holding a seed.
    &[
        Arc {
            cx: 0.0,
            cy: 0.0,
            r: 0.8,
            from: 120.0,
            to: 330.0,
        },
        Dot(0.0, 0.0),
    ],
    // A spine with two uneven rungs and a far dot.
    &[
        Line(-0.1, -1.0, -0.1, 1.0),
        Line(-0.1, -0.5, 0.8, -0.5),
        Line(-0.1, 0.3, -0.8, 0.3),
        Dot(0.7, 0.75),
    ],
    // Two half-rings caught on each other.
    &[
        Arc {
            cx: -0.3,
            cy: 0.0,
            r: 0.6,
            from: -90.0,
            to: 200.0,
        },
        Arc {
            cx: 0.3,
            cy: 0.0,
            r: 0.6,
            from: 90.0,
            to: 380.0,
        },
    ],
    // Two nested chevrons and a dot.
    &[
        Line(-0.8, -0.9, 0.0, -0.1),
        Line(0.0, -0.1, 0.8, -0.9),
        Line(-0.5, 0.1, 0.0, 0.6),
        Line(0.0, 0.6, 0.5, 0.1),
        Dot(0.0, 0.95),
    ],
    // A lens with a pupil.
    &[
        Arc {
            cx: 0.0,
            cy: -0.7,
            r: 1.0,
            from: 50.0,
            to: 130.0,
        },
        Arc {
            cx: 0.0,
            cy: 0.7,
            r: 1.0,
            from: 230.0,
            to: 310.0,
        },
        Dot(0.0, 0.0),
    ],
    // A slash with a tick and a small cup.
    &[
        Line(-0.6, -0.9, 0.6, 0.9),
        Line(-0.2, 0.0, 0.7, -0.5),
        Arc {
            cx: -0.4,
            cy: 0.5,
            r: 0.4,
            from: 180.0,
            to: 300.0,
        },
    ],
    // Three dots, two joined.
    &[
        Dot(0.0, 0.8),
        Dot(-0.7, -0.6),
        Dot(0.7, -0.6),
        Line(0.0, 0.8, -0.7, -0.6),
        Line(-0.7, -0.6, 0.7, -0.6),
    ],
    // A staff ending in a hook.
    &[
        Line(0.3, -1.0, 0.3, 0.4),
        Arc {
            cx: -0.1,
            cy: 0.4,
            r: 0.4,
            from: 0.0,
            to: 180.0,
        },
    ],
    // A bar with dotted ends, an arch over it, and a stem.
    &[
        Line(-0.9, 0.1, 0.9, 0.1),
        Dot(-0.9, 0.1),
        Dot(0.9, 0.1),
        Arc {
            cx: 0.0,
            cy: 0.1,
            r: 0.6,
            from: 200.0,
            to: 340.0,
        },
        Line(0.0, -0.5, 0.0, -1.0),
    ],
];

/// A glyph with its arcs flattened into segments.
struct Glyph {
    segments: Vec<[Vec2; 2]>,
    dots: Vec<Vec2>,
}

impl Glyph {
    fn bake(strokes: &[Stroke]) -> Self {
        let mut glyph = Self {
            segments: Vec::new(),
            dots: Vec::new(),
        };
        for stroke in strokes {
            match *stroke {
                Line(x1, y1, x2, y2) => glyph.segments.push([vec2(x1, y1), vec2(x2, y2)]),
                Dot(x, y) => glyph.dots.push(vec2(x, y)),
                Arc {
                    cx,
                    cy,
                    r,
                    from,
                    to,
                } => {
                    let point = |degrees: f32| {
                        let (sin, cos) = degrees.to_radians().sin_cos();
                        vec2(cx + r * cos, cy + r * sin)
                    };
                    for step in 0..ARC_STEPS {
                        // `step` is below ARC_STEPS (6), so the conversions are exact.
                        let a = from + (to - from) * (step as f32 / ARC_STEPS as f32);
                        let b = from + (to - from) * ((step + 1) as f32 / ARC_STEPS as f32);
                        glyph.segments.push([point(a), point(b)]);
                    }
                }
            }
        }
        glyph
    }
}

/// One ring of glyphs. Radius and glyph size are fractions of the sphere radius.
struct Ring {
    radius: f32,
    tilt: f32,
    roll: f32,
    glyph_count: usize,
    glyph_size: f32,
    /// Which glyph the ring starts its sequence on, so the rings differ.
    glyph_start: usize,
    /// Step through the alphabet from one glyph to the next.
    glyph_stride: usize,
}

const RINGS: [Ring; 2] = [
    Ring {
        radius: 1.24,
        tilt: 1.20,
        roll: 0.18,
        glyph_count: 26,
        glyph_size: 0.060,
        glyph_start: 0,
        glyph_stride: 3,
    },
    Ring {
        radius: 0.92,
        tilt: -0.55,
        roll: 0.95,
        glyph_count: 18,
        glyph_size: 0.050,
        glyph_start: 4,
        glyph_stride: 7,
    },
];

/// One lattice of points on a sphere.
struct Shell {
    /// Points on the unit sphere.
    nodes: Vec<Vec3>,
    /// Pairs of indices into `nodes`, each pair once.
    links: Vec<(usize, usize)>,
    /// Per node: wobble rate and phase.
    wobble: Vec<(f32, f32)>,
    /// Radius as a fraction of the orb's radius.
    radius: f32,
}

impl Shell {
    fn new(count: usize, radius: f32) -> Self {
        let nodes = fibonacci_sphere(count);
        let links = nearest_links(&nodes);
        let wobble = nodes
            .iter()
            .map(|_| {
                (
                    rand::gen_range(WOBBLE_RATE_MIN, WOBBLE_RATE_MAX),
                    rand::gen_range(0.0, TAU),
                )
            })
            .collect();
        Self {
            nodes,
            links,
            wobble,
            radius,
        }
    }
}

/// `count` points spread evenly over the unit sphere.
fn fibonacci_sphere(count: usize) -> Vec<Vec3> {
    let golden_angle = PI * (3.0 - 5f32.sqrt());
    // `count` and the indices are a few hundred, so the conversions are exact.
    let total = count as f32;
    (0..count)
        .map(|i| {
            let y = 1.0 - 2.0 * (i as f32 + 0.5) / total;
            let ring = (1.0 - y * y).sqrt();
            let angle = golden_angle * i as f32;
            vec3(ring * angle.cos(), y, ring * angle.sin())
        })
        .collect()
}

/// Links every point to its nearest neighbours, each pair once.
fn nearest_links(nodes: &[Vec3]) -> Vec<(usize, usize)> {
    let mut links = Vec::new();
    for (i, a) in nodes.iter().enumerate() {
        let mut by_distance: Vec<(f32, usize)> = nodes
            .iter()
            .enumerate()
            .filter(|&(j, _)| j != i)
            .map(|(j, b)| (a.distance_squared(*b), j))
            .collect();
        by_distance.sort_by(|x, y| x.0.total_cmp(&y.0));
        links.extend(
            by_distance
                .iter()
                .take(LINKS_PER_NODE)
                .map(|&(_, j)| (i.min(j), i.max(j))),
        );
    }
    links.sort_unstable();
    links.dedup();
    links
}

/// Sine and cosine of a turn about Y followed by a lean about X.
struct Spin {
    sin_yaw: f32,
    cos_yaw: f32,
    sin_tilt: f32,
    cos_tilt: f32,
}

impl Spin {
    fn new(yaw: f32) -> Self {
        let (sin_yaw, cos_yaw) = yaw.sin_cos();
        let (sin_tilt, cos_tilt) = SPHERE_TILT.sin_cos();
        Self {
            sin_yaw,
            cos_yaw,
            sin_tilt,
            cos_tilt,
        }
    }

    fn apply(&self, p: Vec3) -> Vec3 {
        let x = p.x * self.cos_yaw + p.z * self.sin_yaw;
        let z = -p.x * self.sin_yaw + p.z * self.cos_yaw;
        vec3(
            x,
            p.y * self.cos_tilt - z * self.sin_tilt,
            p.y * self.sin_tilt + z * self.cos_tilt,
        )
    }
}

/// A projected point: where it lands on screen and how near the viewer it is
/// (0 at the back, 1 at the front).
#[derive(Clone, Copy)]
struct Projected {
    pos: Vec2,
    depth: f32,
}

/// The per-frame values every drawing step shares.
struct Frame<'a> {
    look: &'a Look,
    center: Vec2,
    /// Radius of the outer lattice, in pixels.
    radius: f32,
    /// Pixels per reference pixel.
    unit: f32,
    /// Seconds since start, in `f64` so long runs keep their precision.
    clock: f64,
    /// Radial swell from breathing and pulsing, as a fraction of the radius.
    swell: f32,
    /// Brightness factor from breathing.
    glow: f32,
}

fn color(rgb: Rgb, alpha: f32) -> Color {
    Color::new(rgb[0], rgb[1], rgb[2], alpha.clamp(0.0, 1.0))
}

fn whiten(rgb: Rgb, amount: f32) -> Rgb {
    rgb.map(|c| c + (1.0 - c) * amount)
}

/// A sine wave of `rate` radians per second at `phase`, from -1 to 1.
fn wave(clock: f64, rate: f32, phase: f32) -> f32 {
    // sin is within -1..1, so narrowing to f32 loses nothing that matters.
    (clock * f64::from(rate) + f64::from(phase)).sin() as f32
}

/// The orb: precomputed geometry plus the angles that keep turning.
pub struct Orb {
    shells: [Shell; 2],
    glyphs: Vec<Glyph>,
    ray_lengths: [f32; RAY_COUNT],
    /// Scratch space for one lattice's projected points, reused every frame.
    projected: Vec<Projected>,
    clock: f64,
    sphere_angle: f32,
    inner_angle: f32,
    ring_angles: [f32; 2],
    breath_phase: f32,
    ray_angle: f32,
}

impl Orb {
    pub fn new() -> Self {
        rand::srand(SEED);
        let mut ray_lengths = [0.0; RAY_COUNT];
        for length in &mut ray_lengths {
            *length = rand::gen_range(0.5, 1.0);
        }
        Self {
            shells: [
                Shell::new(OUTER_NODES, 1.0),
                Shell::new(INNER_NODES, INNER_RADIUS),
            ],
            glyphs: GLYPHS.iter().map(|strokes| Glyph::bake(strokes)).collect(),
            ray_lengths,
            projected: Vec::with_capacity(OUTER_NODES),
            clock: 0.0,
            sphere_angle: 0.0,
            inner_angle: 0.0,
            ring_angles: [0.0; 2],
            breath_phase: 0.0,
            ray_angle: 0.0,
        }
    }

    /// Advances the animation by `dt` seconds and draws one frame of `look`,
    /// centred on the screen. Sizes follow the screen's shorter side.
    pub fn draw(&mut self, look: &Look, dt: f32) {
        let dt = if dt.is_nan() {
            0.0
        } else {
            dt.clamp(0.0, MAX_STEP_SECONDS)
        };
        self.advance(look, dt);

        let size = screen_width().min(screen_height());
        let breath = self.breath_phase.sin();
        let frame = Frame {
            look,
            center: vec2(screen_width() / 2.0, screen_height() / 2.0),
            radius: size * SPHERE_RADIUS_FRACTION,
            unit: size / REFERENCE_SIZE,
            clock: self.clock,
            swell: look.breath_depth * breath + look.node_pulse * PULSE_SWELL,
            glow: 1.0 + BREATH_BRIGHTNESS * breath,
        };

        self.draw_rays(&frame);
        draw_shell(
            &self.shells[0],
            self.sphere_angle,
            &frame,
            &mut self.projected,
        );
        draw_shell(
            &self.shells[1],
            self.inner_angle,
            &frame,
            &mut self.projected,
        );
        for (index, ring) in RINGS.iter().enumerate() {
            self.draw_ring(ring, self.ring_angles[index], &frame);
        }
        draw_core(&frame);
    }

    fn advance(&mut self, look: &Look, dt: f32) {
        self.clock += f64::from(dt);
        self.sphere_angle = (self.sphere_angle + look.sphere_spin * dt) % TAU;
        self.inner_angle = (self.inner_angle - look.sphere_spin * INNER_SPIN_RATIO * dt) % TAU;
        for (angle, spin) in self.ring_angles.iter_mut().zip(look.ring_spin) {
            *angle = (*angle + spin * dt) % TAU;
        }
        self.breath_phase = (self.breath_phase + TAU * look.breath_rate * dt) % TAU;
        self.ray_angle = (self.ray_angle + RAY_TURN_RATE * dt) % TAU;
    }

    fn draw_rays(&self, frame: &Frame) {
        let strength = frame.look.ray_strength;
        if strength < 0.01 {
            return;
        }
        for (index, &base_length) in self.ray_lengths.iter().enumerate() {
            // `index` is below RAY_COUNT (28), so the conversion is exact.
            let slot = index as f32;
            let angle = self.ray_angle + slot * TAU / RAY_COUNT as f32;
            let shimmer = 0.7 + 0.3 * wave(frame.clock, RAY_SHIMMER_RATE, slot * 1.7);
            let length = frame.radius
                * (RAY_MIN_LENGTH + RAY_EXTRA_LENGTH * strength * base_length * shimmer);
            let start = frame.radius * RAY_START;
            let (sin, cos) = angle.sin_cos();
            let direction = vec2(cos, sin);
            for segment in 0..RAY_SEGMENTS {
                // `segment` is below RAY_SEGMENTS (3), so the conversions are exact.
                let near = segment as f32 / RAY_SEGMENTS as f32;
                let far = (segment + 1) as f32 / RAY_SEGMENTS as f32;
                let from = frame.center + direction * (start + (length - start) * near);
                let to = frame.center + direction * (start + (length - start) * far);
                let alpha = strength * RAY_ALPHA * (1.0 - near) * (1.0 - near);
                draw_line(
                    from.x,
                    from.y,
                    to.x,
                    to.y,
                    RAY_WIDTH_PX * frame.unit,
                    color(frame.look.core_color, alpha),
                );
            }
        }
    }

    fn draw_ring(&self, ring: &Ring, angle: f32, frame: &Frame) {
        let alpha = frame.look.ring_alpha;
        if alpha < 0.01 {
            return;
        }
        let basis = RingBasis::new(ring);
        let size = frame.radius * ring.glyph_size;
        let line = GLYPH_LINE_PX * frame.unit;

        // Two thin tracks, inside and outside the glyphs.
        for side in [-1.0, 1.0] {
            let offset = side * size * TRACK_MARGIN;
            let mut previous = basis.point(ring, frame, 0.0, offset, 0.0);
            for step in 1..=GLYPH_TRACK_STEPS {
                // `step` is at most GLYPH_TRACK_STEPS (48), so the conversions are exact.
                let theta = TAU * step as f32 / GLYPH_TRACK_STEPS as f32;
                let next = basis.point(ring, frame, theta, offset, 0.0);
                let depth = (previous.depth + next.depth) / 2.0;
                draw_line(
                    previous.pos.x,
                    previous.pos.y,
                    next.pos.x,
                    next.pos.y,
                    frame.unit,
                    color(
                        frame.look.ring_color,
                        alpha * TRACK_ALPHA * frame.glow * ring_shade(depth),
                    ),
                );
                previous = next;
            }
        }

        for slot in 0..ring.glyph_count {
            let glyph =
                &self.glyphs[(ring.glyph_start + slot * ring.glyph_stride) % self.glyphs.len()];
            // `slot` is below glyph_count (26), so the conversion is exact.
            let theta = angle + TAU * slot as f32 / ring.glyph_count as f32;
            let place = |p: Vec2| basis.point(ring, frame, theta, p.y * size, p.x * size);
            for [a, b] in &glyph.segments {
                let (a, b) = (place(*a), place(*b));
                let depth = (a.depth + b.depth) / 2.0;
                draw_line(
                    a.pos.x,
                    a.pos.y,
                    b.pos.x,
                    b.pos.y,
                    line,
                    color(
                        frame.look.ring_color,
                        alpha * frame.glow * ring_shade(depth),
                    ),
                );
            }
            for dot in &glyph.dots {
                let at = place(*dot);
                draw_poly(
                    at.pos.x,
                    at.pos.y,
                    GLYPH_DOT_SIDES,
                    size * GLYPH_DOT_RADIUS,
                    0.0,
                    color(
                        frame.look.ring_color,
                        alpha * frame.glow * ring_shade(at.depth),
                    ),
                );
            }
        }
    }
}

fn ring_shade(depth: f32) -> f32 {
    RING_BACK_DIMMING + (1.0 - RING_BACK_DIMMING) * depth
}

/// Sine and cosine of a ring's tilt and roll, worked out once per ring per frame.
struct RingBasis {
    sin_tilt: f32,
    cos_tilt: f32,
    sin_roll: f32,
    cos_roll: f32,
}

impl RingBasis {
    fn new(ring: &Ring) -> Self {
        let (sin_tilt, cos_tilt) = ring.tilt.sin_cos();
        let (sin_roll, cos_roll) = ring.roll.sin_cos();
        Self {
            sin_tilt,
            cos_tilt,
            sin_roll,
            cos_roll,
        }
    }

    /// A point on the ring's tilted plane: `theta` around it, `radial` pixels out
    /// from the ring line and `along` pixels along it.
    fn point(&self, ring: &Ring, frame: &Frame, theta: f32, radial: f32, along: f32) -> Projected {
        let ring_radius = frame.radius * ring.radius * (1.0 + frame.swell);
        let (sin, cos) = theta.sin_cos();
        let reach = ring_radius + radial;
        let x = reach * cos - along * sin;
        let y = reach * sin + along * cos;
        let leaned_y = y * self.cos_tilt;
        let z = y * self.sin_tilt;
        let rolled = vec2(
            x * self.cos_roll - leaned_y * self.sin_roll,
            x * self.sin_roll + leaned_y * self.cos_roll,
        );
        Projected {
            pos: frame.center + rolled,
            depth: (z / ring_radius).clamp(-1.0, 1.0) * 0.5 + 0.5,
        }
    }
}

fn draw_shell(shell: &Shell, yaw: f32, frame: &Frame, projected: &mut Vec<Projected>) {
    let look = frame.look;
    let spin = Spin::new(yaw);
    projected.clear();
    for (node, &(rate, phase)) in shell.nodes.iter().zip(&shell.wobble) {
        let jitter = look.node_jitter * JITTER_AMPLITUDE * wave(frame.clock, rate, phase);
        let reach = shell.radius * frame.radius * (1.0 + frame.swell + jitter);
        let p = spin.apply(*node);
        projected.push(Projected {
            pos: frame.center + vec2(p.x, p.y) * reach,
            depth: (p.z + 1.0) * 0.5,
        });
    }

    let brightness = look.brightness * frame.glow;
    for &(a, b) in &shell.links {
        let (a, b) = (projected[a], projected[b]);
        let depth = (a.depth + b.depth) / 2.0;
        let alpha = brightness * LINK_ALPHA * (BACK_DIMMING + (1.0 - BACK_DIMMING) * depth);
        draw_line(
            a.pos.x,
            a.pos.y,
            b.pos.x,
            b.pos.y,
            LINK_WIDTH_PX * frame.unit,
            color(look.link_color, alpha),
        );
    }

    for (point, &(_, phase)) in projected.iter().zip(&shell.wobble) {
        let ripple = 0.5 + 0.5 * wave(frame.clock, PULSE_WAVE_RATE, phase);
        let pulse = look.node_pulse * ripple;
        let size = (NODE_BASE_PX + NODE_DEPTH_PX * point.depth)
            * frame.unit
            * (1.0 + PULSE_SIZE_GAIN * pulse);
        let alpha = brightness * (BACK_DIMMING + (1.0 - BACK_DIMMING) * point.depth)
            + PULSE_BRIGHTNESS_GAIN * pulse;
        draw_poly(
            point.pos.x,
            point.pos.y,
            NODE_SIDES,
            size,
            0.0,
            color(look.node_color, alpha),
        );
    }
}

fn draw_core(frame: &Frame) {
    let look = frame.look;
    let base = frame.radius * CORE_RADIUS * (0.8 + 0.5 * look.core_glow) * (1.0 + frame.swell);
    for layer in 0..CORE_LAYERS {
        // `layer` is below CORE_LAYERS (9), so the conversions are exact.
        let t = (layer + 1) as f32 / CORE_LAYERS as f32;
        let radius = base * (CORE_OUTER_SCALE + (CORE_INNER_SCALE - CORE_OUTER_SCALE) * t);
        let alpha = look.core_glow
            * (CORE_LAYER_ALPHA_MIN + (CORE_LAYER_ALPHA_MAX - CORE_LAYER_ALPHA_MIN) * t);
        draw_poly(
            frame.center.x,
            frame.center.y,
            CORE_SIDES,
            radius,
            0.0,
            color(whiten(look.core_color, CORE_WHITENING * t), alpha),
        );
    }
}

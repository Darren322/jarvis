//! Draws the orb from a [`Look`].
//!
//! A flat, face-on mandala that mixes two styles. The warm, living centre is
//! Great Sage: a white-gold starburst inside a thick, soft, slightly uneven glowing
//! ring, long thin rays, and a band of tall invented glyphs standing around it. The
//! cool, technical frame is Iron Man's JARVIS: crisp cyan and blue HUD rings broken
//! into arcs, dashed and ticked like a gauge, turning at different speeds. Behind it
//! all, faint out-of-focus blobs of light drift in rings. Back to front: blobs, rays,
//! HUD rings, glyph band, core ring, sparkles, starburst. No text is ever drawn. All
//! geometry is built once in [`Orb::new`]; a frame allocates nothing of its own.

use std::f32::consts::TAU;

use jarvis_orb::look::{Look, Rgb};
use macroquad::prelude::*;

/// The screen height every pixel size below is written for.
const REFERENCE_SIZE: f32 = 480.0;
/// The longest step the animation takes in one frame, in seconds. A stall (for
/// example the window being dragged) must not make the orb jump.
const MAX_STEP_SECONDS: f32 = 0.1;
/// Seed for placement noise, so the orb is the same on every start.
const SEED: u64 = 0x0b_1a_57;
/// Brightness swing of one breath, as a fraction.
const BREATH_BRIGHTNESS: f32 = 0.15;
/// The whole orb swells by this fraction of the radius at a full pulse.
const PULSE_SWELL: f32 = 0.04;
/// Speed of the ripple that runs through pulsing sparkles, radians per second.
const PULSE_WAVE_RATE: f32 = 7.0;

// Background blobs. Distances and radii are fractions of the screen's shorter side.
const BLOB_RING_RADII: [f32; 4] = [0.28, 0.50, 0.75, 1.00];
const BLOBS_PER_RING: [usize; 4] = [6, 9, 10, 11];
const BLOB_RADIUS_MIN: f32 = 0.04;
const BLOB_RADIUS_MAX: f32 = 0.11;
/// Slowest and fastest drift around the centre, radians per second.
const BLOB_DRIFT_MAX: f32 = 0.05;
const BLOB_WOBBLE_RATE: f32 = 0.30;
const BLOB_WOBBLE_DEPTH: f32 = 0.05;
/// Each blob is three stacked discs: (radius scale, alpha).
const BLOB_LAYERS: [(f32, f32); 3] = [(1.0, 0.03), (0.68, 0.04), (0.38, 0.05)];
const BLOB_BLUE: Rgb = [0.10, 0.30, 0.95];
const BLOB_TEAL: Rgb = [0.10, 0.70, 0.75];
const BLOB_GREEN: Rgb = [0.30, 0.85, 0.40];
const BLOB_AMBER: Rgb = [1.00, 0.60, 0.15];
const BLOB_PINK: Rgb = [1.00, 0.70, 0.80];
const BLOB_SIDES: u8 = 20;

// Rays.
const RAY_COUNT: usize = 64;
/// Every this-many rays is a thick, bright one.
const RAY_THICK_EVERY: usize = 8;
const RAY_START: f32 = 0.03;
const RAY_MIN_LENGTH: f32 = 0.45;
const RAY_MAX_LENGTH: f32 = 1.25;
const RAY_WIDTH_PX: f32 = 0.9;
const RAY_THICK_WIDTH_PX: f32 = 2.0;
const RAY_BASE_ALPHA: f32 = 0.22;
const RAY_STRENGTH_ALPHA: f32 = 0.78;
const RAY_MAX_ALPHA: f32 = 0.60;
/// Where along a ray the bright inner part ends, and how bright the rest is.
const RAY_SPLIT: f32 = 0.35;
const RAY_OUTER_FADE: f32 = 0.45;
const RAY_TURN_RATE: f32 = 0.012;
const RAY_SHIMMER_RATE: f32 = 2.2;
const RAY_WHITE_MAX: f32 = 0.7;

// Glyph band. Sizes are fractions of the screen's shorter side.
const BAND_BACKING_ALPHA: f32 = 0.22;
const BAND_BACKING_SIDES: u8 = 96;

// HUD rings. Radii and tick lengths are fractions of the screen's shorter side.
/// Segments per full circle for HUD arcs; an arc uses a share of them.
const HUD_SIDES: u8 = 96;
/// A faint wide arc under each arc ring gives the soft blue glow.
const HUD_GLOW_WIDTH: f32 = 4.0;
const HUD_GLOW_ALPHA: f32 = 0.16;
/// How far a notch at the start of an arc reaches past each side of the arc, px.
const HUD_NOTCH_PX: f32 = 5.0;
const HUD_NOTCH_ALPHA: f32 = 0.9;
const HUD_TICK_PX: f32 = 1.0;
const HUD_MAJOR_TICK_PX: f32 = 1.8;
const HUD_MAJOR_TICK_BOOST: f32 = 1.3;
/// Ticks this far outside the screen are skipped, in pixels.
const HUD_TICK_CULL_PX: f32 = 30.0;
/// How strongly the flaring ring follows the speech level.
const HUD_FLARE_GAIN: f32 = 0.8;
const HUD_FLARE_BASE: f32 = 0.6;
const GLYPH_SHIMMER_RATE: f32 = 1.3;
const ARC_STEPS: usize = 6;

// Core ring.
const CORE_RING_RADIUS: f32 = 0.30;
/// Ring thickness as a fraction of its radius.
const CORE_RING_THICKNESS: f32 = 1.0 / 6.0;
/// Points around the ring: enough that it reads as a curve, not a polygon.
const CORE_RING_SIDES: usize = 72;
/// How far the ring's edge strays from the true circle, as a fraction.
const CORE_RING_UNEVENNESS: f32 = 0.022;
/// The edge wanders in two slow waves around the ring, so it looks hand-drawn
/// and alive rather than machined: lobes per turn, and each wave's share.
const CORE_RING_WAVES: [(f32, f32); 2] = [(5.0, 0.6), (9.0, 0.4)];
/// Layers of the ring glow, widest first: (width as a multiple of the ring
/// thickness, alpha, heat).
const CORE_RING_LAYERS: [(f32, f32, f32); 9] = [
    (4.0, 0.03, 0.0),
    (3.3, 0.04, 0.05),
    (2.7, 0.05, 0.10),
    (2.2, 0.06, 0.18),
    (1.7, 0.08, 0.28),
    (1.3, 0.12, 0.40),
    (0.95, 0.20, 0.55),
    (0.62, 0.35, 0.75),
    (0.32, 0.55, 0.92),
];
/// The ring turns this much slower than the sparkle cloud.
const CORE_RING_TURN_RATIO: f32 = 0.25;
/// Amber fill discs inside the ring: (radius as a fraction of the ring, alpha).
const CORE_FILL_LAYERS: [(f32, f32); 4] = [(1.0, 0.12), (0.8, 0.10), (0.6, 0.10), (0.4, 0.08)];
const CORE_FILL_SIDES: u8 = 32;

// Starburst.
/// Glow discs, widest first (heat is the colour from [`glow_color`]): (radius as a fraction of the ring, alpha, heat).
const STAR_GLOW_LAYERS: [(f32, f32, f32); 8] = [
    (0.78, 0.05, 0.10),
    (0.62, 0.07, 0.20),
    (0.48, 0.10, 0.35),
    (0.36, 0.14, 0.50),
    (0.26, 0.20, 0.65),
    (0.17, 0.32, 0.80),
    (0.10, 0.55, 0.92),
    (0.055, 0.90, 0.98),
];
const STAR_GLOW_SIDES: u8 = 28;
const STAR_RAY_COUNT: usize = 40;
const STAR_RAY_START: f32 = 0.03;
const STAR_RAY_REACH: f32 = 0.85;
const STAR_RAY_ALPHA: f32 = 0.55;
const STAR_RAY_PX: f32 = 1.0;
const TWINKLE_COUNT: usize = 10;
const TWINKLE_SIZE_MIN: f32 = 0.04;
const TWINKLE_SIZE_MAX: f32 = 0.09;
const TWINKLE_RATE_MIN: f32 = 1.5;
const TWINKLE_RATE_MAX: f32 = 4.0;
/// A full pulse grows a twinkle by this fraction.
const TWINKLE_PULSE_GAIN: f32 = 0.8;
const TWINKLE_PX: f32 = 1.2;

// Sparkle cloud inside the core ring.
const SPARKLE_COUNT: usize = 70;
const SPARKLE_REACH: f32 = 0.80;
const SPARKLE_BASE_PX: f32 = 1.0;
const SPARKLE_PULSE_GAIN: f32 = 1.4;
const SPARKLE_ALPHA: f32 = 0.55;
const SPARKLE_PULSE_BRIGHTNESS: f32 = 0.35;
const SPARKLE_SIDES: u8 = 6;
const SPARKLE_WOBBLE_MIN: f32 = 4.0;
const SPARKLE_WOBBLE_MAX: f32 = 9.0;
/// How far a fully jittering sparkle moves in and out, as a fraction of the ring.
const JITTER_AMPLITUDE: f32 = 0.18;

/// A glyph stroke in a box running -1 to 1 across (x, along the ring) and from the
/// band's centre side to its outer side (y). Angles are in degrees.
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

/// The invented alphabet: ten tall marks made of lines, arcs and dots, in the
/// spirit of blackletter strokes. They stand for nothing and are not drawn from
/// any real script.
const GLYPHS: [&[Stroke]; 10] = [
    // A stem with a pointed crown, a bar and a dot below.
    &[
        Line(0.0, -1.0, 0.0, 1.0),
        Line(-0.8, 0.55, 0.0, 0.95),
        Line(0.0, 0.95, 0.8, 0.55),
        Line(-0.8, -0.3, 0.8, -0.3),
        Dot(0.0, -0.7),
    ],
    // Two uneven stems joined by a slanting bar.
    &[
        Line(-0.6, -1.0, -0.6, 1.0),
        Line(0.6, -1.0, 0.6, 0.6),
        Line(-0.6, 0.2, 0.6, -0.3),
        Dot(0.6, 0.92),
    ],
    // A diamond on a stem with a foot bar.
    &[
        Line(0.0, 1.0, 0.7, 0.5),
        Line(0.7, 0.5, 0.0, 0.0),
        Line(0.0, 0.0, -0.7, 0.5),
        Line(-0.7, 0.5, 0.0, 1.0),
        Line(0.0, 0.0, 0.0, -1.0),
        Line(-0.5, -0.6, 0.5, -0.6),
    ],
    // An arch on two legs with a seed beneath.
    &[
        Arc {
            cx: 0.0,
            cy: 0.35,
            r: 0.75,
            from: 180.0,
            to: 360.0,
        },
        Line(-0.75, 0.35, -0.75, -0.8),
        Line(0.75, 0.35, 0.75, -1.0),
        Dot(0.0, -0.2),
    ],
    // A zigzag.
    &[
        Line(-0.7, 1.0, 0.7, 0.5),
        Line(0.7, 0.5, -0.7, 0.0),
        Line(-0.7, 0.0, 0.7, -0.5),
        Line(0.7, -0.5, -0.7, -1.0),
    ],
    // A tine with two unequal branches and a dot above.
    &[
        Line(0.0, -1.0, 0.0, 0.7),
        Line(0.0, 0.2, -0.8, 0.8),
        Line(0.0, -0.2, 0.8, 0.4),
        Dot(0.0, 0.95),
    ],
    // A ladder of two rails with dots at each end.
    &[
        Line(-0.5, -1.0, -0.5, 1.0),
        Line(0.5, -1.0, 0.5, 1.0),
        Line(-0.5, 0.5, 0.5, 0.5),
        Line(-0.5, -0.2, 0.5, -0.2),
        Dot(0.0, 0.85),
        Dot(0.0, -0.7),
    ],
    // A staff ending in a hook, with a serif and a dot.
    &[
        Line(0.3, -1.0, 0.3, 0.4),
        Arc {
            cx: -0.15,
            cy: 0.45,
            r: 0.45,
            from: 0.0,
            to: 180.0,
        },
        Line(-0.6, -0.5, 0.3, -0.5),
        Dot(-0.6, -0.8),
    ],
    // A trident.
    &[
        Line(0.0, -1.0, 0.0, 0.6),
        Line(-0.8, 0.9, -0.8, 0.1),
        Line(-0.8, 0.1, 0.0, -0.1),
        Line(0.8, 0.9, 0.8, 0.1),
        Line(0.8, 0.1, 0.0, -0.1),
        Dot(0.0, 0.9),
    ],
    // A cross-brace on a stem with a foot bar.
    &[
        Line(-0.7, 0.9, 0.7, -0.2),
        Line(0.7, 0.9, -0.7, -0.2),
        Line(0.0, -0.2, 0.0, -1.0),
        Line(-0.6, -0.7, 0.6, -0.7),
        Dot(0.0, 0.95),
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

/// The band of glyphs standing around a circle, tops pointing outwards. Radius,
/// height and width are fractions of the screen's shorter side.
struct Band {
    radius: f32,
    height: f32,
    width: f32,
    glyph_count: usize,
    glyph_line_px: f32,
    /// Which glyph the band starts its sequence on.
    glyph_start: usize,
    /// Step through the alphabet from one glyph to the next.
    glyph_stride: usize,
}

/// The band's centre line is 0.42 of the shorter side from the middle, so with its
/// height it nearly touches the top and bottom of an 800x480 screen.
const BAND: Band = Band {
    radius: 0.42,
    height: 0.125,
    width: 0.034,
    glyph_count: 66,
    glyph_line_px: 1.7,
    glyph_start: 0,
    glyph_stride: 3,
};

/// What a HUD ring is made of.
enum HudKind {
    /// A full thin circle line, thickness in reference pixels.
    Circle { thickness_px: f32 },
    /// Arcs of different lengths with gaps: (start, length) in degrees. With
    /// `notches`, a short cross-line marks the start of each arc.
    Arcs {
        arcs: &'static [(f32, f32)],
        thickness_px: f32,
        notches: bool,
    },
    /// A circle cut into `count` equal dashes, each `duty` of its pitch long.
    Dashes {
        count: usize,
        duty: f32,
        thickness_px: f32,
    },
    /// Gauge ticks, every `major_every`th one longer. Lengths are fractions of
    /// the shorter side.
    Ticks {
        count: usize,
        major_every: usize,
        minor_len: f32,
        major_len: f32,
    },
}

/// One ring of the HUD. `spin` is a multiple of the look's HUD spin, so rings turn
/// at different speeds and in different directions.
struct HudRing {
    radius: f32,
    spin: f32,
    alpha: f32,
    /// True for the ring that flares with the speech level.
    flare: bool,
    kind: HudKind,
}

const ARCS_INNER: [(f32, f32); 6] = [
    (0.0, 70.0),
    (80.0, 25.0),
    (115.0, 60.0),
    (185.0, 15.0),
    (210.0, 95.0),
    (315.0, 30.0),
];
const ARCS_MIDDLE: [(f32, f32); 6] = [
    (10.0, 50.0),
    (70.0, 10.0),
    (90.0, 100.0),
    (205.0, 35.0),
    (250.0, 70.0),
    (330.0, 20.0),
];
const ARCS_OUTER: [(f32, f32); 5] = [
    (0.0, 100.0),
    (115.0, 60.0),
    (190.0, 40.0),
    (250.0, 80.0),
    (340.0, 15.0),
];

/// Inside out. The two circle and dash rings are the crisp lines bounding the
/// glyph band (centre 0.42, half-height 0.0575, with a margin).
const HUD_RINGS: [HudRing; 7] = [
    HudRing {
        radius: 0.235,
        spin: 0.5,
        alpha: 0.8,
        flare: false,
        kind: HudKind::Ticks {
            count: 72,
            major_every: 6,
            minor_len: 0.008,
            major_len: 0.016,
        },
    },
    HudRing {
        radius: 0.335,
        spin: 1.0,
        alpha: 0.85,
        flare: false,
        kind: HudKind::Arcs {
            arcs: &ARCS_INNER,
            thickness_px: 1.6,
            notches: false,
        },
    },
    HudRing {
        radius: 0.356,
        spin: 0.0,
        alpha: 0.7,
        flare: false,
        kind: HudKind::Circle { thickness_px: 1.2 },
    },
    HudRing {
        radius: 0.484,
        spin: 0.3,
        alpha: 0.8,
        flare: false,
        kind: HudKind::Dashes {
            count: 72,
            duty: 0.5,
            thickness_px: 1.4,
        },
    },
    HudRing {
        radius: 0.525,
        spin: -0.7,
        alpha: 0.85,
        flare: false,
        kind: HudKind::Arcs {
            arcs: &ARCS_MIDDLE,
            thickness_px: 3.0,
            notches: true,
        },
    },
    HudRing {
        radius: 0.575,
        spin: 0.4,
        alpha: 0.8,
        flare: false,
        kind: HudKind::Ticks {
            count: 120,
            major_every: 5,
            minor_len: 0.010,
            major_len: 0.022,
        },
    },
    HudRing {
        radius: 0.68,
        spin: 1.4,
        alpha: 0.9,
        flare: true,
        kind: HudKind::Arcs {
            arcs: &ARCS_OUTER,
            thickness_px: 5.0,
            notches: true,
        },
    },
];

/// One drifting blob of background light.
struct Blob {
    /// Distance from the centre as a fraction of the shorter side.
    distance: f32,
    angle: f32,
    /// Drift around the centre, radians per second.
    drift: f32,
    wobble_phase: f32,
    /// Radius as a fraction of the shorter side.
    radius: f32,
    color: Rgb,
}

/// One thin ray from the centre.
struct Ray {
    angle: f32,
    /// 0..1: how far this ray reaches at full strength.
    reach: f32,
    /// 0..1: how bright this ray is.
    brightness: f32,
    /// 0..1: how white (rather than gold) this ray is.
    white: f32,
    phase: f32,
    thick: bool,
}

/// A short ray of the starburst.
struct StarRay {
    angle: f32,
    reach: f32,
    phase: f32,
}

/// A four-point star that twinkles inside the ring, with its place as a fraction
/// of the ring radius.
struct Twinkle {
    pos: Vec2,
    size: f32,
    rate: f32,
    phase: f32,
}

/// One point of the sparkle cloud: a place in the unit disc, a wobble rate and phase.
struct Sparkle {
    pos: Vec2,
    rate: f32,
    phase: f32,
}

/// The per-frame values every drawing step shares.
struct Frame<'a> {
    look: &'a Look,
    center: Vec2,
    /// Width and height of the drawing area, in pixels.
    area: Vec2,
    /// The screen's shorter side, in pixels.
    size: f32,
    /// Distance from the centre to a corner of the screen, in pixels.
    reach: f32,
    /// Pixels per reference pixel.
    unit: f32,
    /// Seconds since start, in `f64` so long runs keep their precision.
    clock: f64,
    /// Radial swell from breathing and pulsing, as a fraction of the radius.
    swell: f32,
    /// Brightness of the whole mandala: the look's brightness and the breath.
    lum: f32,
}

fn color(rgb: Rgb, alpha: f32) -> Color {
    Color::new(rgb[0], rgb[1], rgb[2], alpha.clamp(0.0, 1.0))
}

fn whiten(rgb: Rgb, amount: f32) -> Rgb {
    rgb.map(|c| c + (1.0 - c) * amount)
}

/// A colour `amount` of the way from `from` to `to`.
fn blend(from: Rgb, to: Rgb, amount: f32) -> Rgb {
    [
        from[0] + (to[0] - from[0]) * amount,
        from[1] + (to[1] - from[1]) * amount,
        from[2] + (to[2] - from[2]) * amount,
    ]
}

/// The colour of a glow layer: the deep amber fill at 0, the core colour halfway,
/// and white-hot at 1.
fn glow_color(look: &Look, heat: f32) -> Rgb {
    let heat = heat.clamp(0.0, 1.0);
    if heat < 0.5 {
        blend(look.fill_color, look.core_color, heat * 2.0)
    } else {
        whiten(look.core_color, (heat - 0.5) * 2.0)
    }
}

/// Blends a colour towards its own grey: 0 is grey, 1 is the colour itself.
fn saturate(rgb: Rgb, amount: f32) -> Rgb {
    let grey = 0.299 * rgb[0] + 0.587 * rgb[1] + 0.114 * rgb[2];
    rgb.map(|c| grey + (c - grey) * amount)
}

/// A sine wave of `rate` radians per second at `phase`, from -1 to 1.
fn wave(clock: f64, rate: f32, phase: f32) -> f32 {
    // sin is within -1..1, so narrowing to f32 loses nothing that matters.
    (clock * f64::from(rate) + f64::from(phase)).sin() as f32
}

/// A random angle in 0..TAU.
fn random_angle() -> f32 {
    rand::gen_range(0.0, TAU)
}

/// A random point in the unit disc, spread evenly by area.
fn random_in_disc() -> Vec2 {
    let radius = rand::gen_range(0.0f32, 1.0).sqrt();
    let (sin, cos) = random_angle().sin_cos();
    vec2(cos, sin) * radius
}

/// The orb: precomputed geometry plus the angles that keep turning.
pub struct Orb {
    glyphs: Vec<Glyph>,
    blobs: Vec<Blob>,
    rays: Vec<Ray>,
    star_rays: Vec<StarRay>,
    twinkles: Vec<Twinkle>,
    sparkles: Vec<Sparkle>,
    /// Per vertex of the core ring: its distance from the centre as a fraction of
    /// the ring radius.
    ring_vertices: [f32; CORE_RING_SIDES],
    clock: f64,
    sparkle_angle: f32,
    band_angles: [f32; 2],
    breath_phase: f32,
    ray_angle: f32,
}

impl Orb {
    pub fn new() -> Self {
        rand::srand(SEED);
        let mut blobs = Vec::new();
        for (&distance, &count) in BLOB_RING_RADII.iter().zip(&BLOBS_PER_RING) {
            for _ in 0..count {
                let pick = rand::gen_range(0.0f32, 1.0);
                let color = if pick < 0.35 {
                    BLOB_BLUE
                } else if pick < 0.65 {
                    BLOB_TEAL
                } else if pick < 0.80 {
                    BLOB_GREEN
                } else if pick < 0.92 {
                    BLOB_AMBER
                } else {
                    BLOB_PINK
                };
                blobs.push(Blob {
                    distance,
                    angle: random_angle(),
                    drift: rand::gen_range(-BLOB_DRIFT_MAX, BLOB_DRIFT_MAX),
                    wobble_phase: random_angle(),
                    radius: rand::gen_range(BLOB_RADIUS_MIN, BLOB_RADIUS_MAX),
                    color,
                });
            }
        }
        let rays = (0..RAY_COUNT)
            .map(|index| {
                let thick = index % RAY_THICK_EVERY == 0;
                Ray {
                    angle: random_angle(),
                    reach: rand::gen_range(0.6, 1.0),
                    brightness: if thick {
                        1.0
                    } else {
                        rand::gen_range(0.3, 0.85)
                    },
                    white: rand::gen_range(0.0, RAY_WHITE_MAX),
                    phase: random_angle(),
                    thick,
                }
            })
            .collect();
        let star_rays = (0..STAR_RAY_COUNT)
            .map(|_| StarRay {
                angle: random_angle(),
                reach: rand::gen_range(0.35, 1.0),
                phase: random_angle(),
            })
            .collect();
        let twinkles = (0..TWINKLE_COUNT)
            .map(|_| Twinkle {
                pos: random_in_disc() * SPARKLE_REACH,
                size: rand::gen_range(TWINKLE_SIZE_MIN, TWINKLE_SIZE_MAX),
                rate: rand::gen_range(TWINKLE_RATE_MIN, TWINKLE_RATE_MAX),
                phase: random_angle(),
            })
            .collect();
        let sparkles = (0..SPARKLE_COUNT)
            .map(|_| Sparkle {
                pos: random_in_disc(),
                rate: rand::gen_range(SPARKLE_WOBBLE_MIN, SPARKLE_WOBBLE_MAX),
                phase: random_angle(),
            })
            .collect();
        let wave_phases = CORE_RING_WAVES.map(|_| random_angle());
        let mut ring_vertices = [1.0; CORE_RING_SIDES];
        for (index, vertex) in ring_vertices.iter_mut().enumerate() {
            // `index` is below CORE_RING_SIDES (72), so the conversions are exact.
            let theta = TAU * index as f32 / CORE_RING_SIDES as f32;
            let wander: f32 = CORE_RING_WAVES
                .iter()
                .zip(wave_phases)
                .map(|(&(lobes, share), phase)| share * (lobes * theta + phase).sin())
                .sum();
            *vertex = 1.0 + CORE_RING_UNEVENNESS * wander;
        }
        Self {
            glyphs: GLYPHS.iter().map(|strokes| Glyph::bake(strokes)).collect(),
            blobs,
            rays,
            star_rays,
            twinkles,
            sparkles,
            ring_vertices,
            clock: 0.0,
            sparkle_angle: 0.0,
            band_angles: [0.0; 2],
            breath_phase: 0.0,
            ray_angle: 0.0,
        }
    }

    /// Advances the animation by `dt` seconds and draws one frame of `look`,
    /// centred in an `area` of that many pixels. Sizes follow the shorter side.
    pub fn draw(&mut self, look: &Look, dt: f32, area: Vec2) {
        let dt = if dt.is_nan() {
            0.0
        } else {
            dt.clamp(0.0, MAX_STEP_SECONDS)
        };
        self.advance(look, dt);

        let breath = self.breath_phase.sin();
        let frame = Frame {
            look,
            center: area / 2.0,
            area,
            size: area.min_element(),
            reach: area.length() / 2.0,
            unit: area.min_element() / REFERENCE_SIZE,
            clock: self.clock,
            swell: look.breath_depth * breath + look.node_pulse * PULSE_SWELL,
            lum: look.brightness * (1.0 + BREATH_BRIGHTNESS * breath),
        };

        self.draw_blobs(&frame);
        self.draw_rays(&frame);
        self.draw_hud(&frame);
        self.draw_band(&BAND, self.band_angles[0], &frame);
        self.draw_core_ring(&frame);
        self.draw_sparkles(&frame);
        self.draw_starburst(&frame);
    }

    fn advance(&mut self, look: &Look, dt: f32) {
        self.clock += f64::from(dt);
        self.sparkle_angle = (self.sparkle_angle + look.sparkle_spin * dt) % TAU;
        for (angle, spin) in self.band_angles.iter_mut().zip(look.band_spin) {
            *angle = (*angle + spin * dt) % TAU;
        }
        self.breath_phase = (self.breath_phase + TAU * look.breath_rate * dt) % TAU;
        self.ray_angle = (self.ray_angle + RAY_TURN_RATE * dt) % TAU;
    }

    fn draw_blobs(&self, frame: &Frame) {
        let look = frame.look;
        if look.background < 0.01 {
            return;
        }
        for blob in &self.blobs {
            // Wrapped to 0..TAU in f64 first, so the narrowing keeps its precision.
            let drifted = (f64::from(blob.drift) * frame.clock) % f64::from(TAU);
            let angle = blob.angle + drifted as f32;
            let distance = blob.distance
                * (1.0
                    + BLOB_WOBBLE_DEPTH * wave(frame.clock, BLOB_WOBBLE_RATE, blob.wobble_phase));
            let (sin, cos) = angle.sin_cos();
            let at = frame.center + vec2(cos, sin) * (distance * frame.size);
            let rgb = saturate(blob.color, look.blob_saturation);
            for (scale, alpha) in BLOB_LAYERS {
                draw_poly(
                    at.x,
                    at.y,
                    BLOB_SIDES,
                    blob.radius * frame.size * scale,
                    0.0,
                    color(rgb, alpha * look.background),
                );
            }
        }
    }

    fn draw_rays(&self, frame: &Frame) {
        let strength = frame.look.ray_strength;
        if strength < 0.01 {
            return;
        }
        let reach = strength.sqrt();
        let start = frame.size * RAY_START;
        for ray in &self.rays {
            let angle = ray.angle + self.ray_angle;
            let shimmer = 0.8 + 0.2 * wave(frame.clock, RAY_SHIMMER_RATE, ray.phase);
            let longest = RAY_MAX_LENGTH * ray.reach * shimmer;
            let length = frame.reach * (RAY_MIN_LENGTH + (longest - RAY_MIN_LENGTH) * reach);
            let (sin, cos) = angle.sin_cos();
            let direction = vec2(cos, sin);
            let alpha =
                ((RAY_BASE_ALPHA + RAY_STRENGTH_ALPHA * strength) * frame.lum * ray.brightness)
                    .min(RAY_MAX_ALPHA);
            let rgb = whiten(frame.look.core_color, ray.white);
            let width = if ray.thick {
                RAY_THICK_WIDTH_PX
            } else {
                RAY_WIDTH_PX
            } * frame.unit;
            let split = start + (length - start) * RAY_SPLIT;
            let near = frame.center + direction * start;
            let middle = frame.center + direction * split;
            let far = frame.center + direction * length;
            draw_line(near.x, near.y, middle.x, middle.y, width, color(rgb, alpha));
            draw_line(
                middle.x,
                middle.y,
                far.x,
                far.y,
                width,
                color(rgb, alpha * RAY_OUTER_FADE),
            );
        }
    }

    fn draw_hud(&self, frame: &Frame) {
        let look = frame.look;
        let base = look.hud_alpha * frame.lum;
        if base < 0.01 {
            return;
        }
        let hud_angle = self.band_angles[1];
        let unit = frame.unit;
        for ring in &HUD_RINGS {
            let mut alpha = base * ring.alpha;
            if ring.flare {
                alpha *= HUD_FLARE_BASE + HUD_FLARE_GAIN * look.node_jitter;
            }
            let radius = frame.size * ring.radius * (1.0 + frame.swell);
            let turn = hud_angle * ring.spin;
            let paint = color(look.hud_color, alpha);
            let (cx, cy) = (frame.center.x, frame.center.y);
            match ring.kind {
                HudKind::Circle { thickness_px } => {
                    let thickness = thickness_px * unit;
                    draw_arc(
                        cx,
                        cy,
                        HUD_SIDES,
                        radius - thickness / 2.0,
                        0.0,
                        thickness,
                        360.0,
                        paint,
                    );
                }
                HudKind::Arcs {
                    arcs,
                    thickness_px,
                    notches,
                } => {
                    let thickness = thickness_px * unit;
                    let glow = thickness * HUD_GLOW_WIDTH;
                    let notch = HUD_NOTCH_PX * unit;
                    for &(start, length) in arcs {
                        let rotation = start + turn.to_degrees();
                        draw_arc(
                            cx,
                            cy,
                            HUD_SIDES,
                            radius - glow / 2.0,
                            rotation,
                            glow,
                            length,
                            color(look.hud_color, alpha * HUD_GLOW_ALPHA),
                        );
                        draw_arc(
                            cx, cy, HUD_SIDES, radius, rotation, thickness, length, paint,
                        );
                        if notches {
                            let (sin, cos) = rotation.to_radians().sin_cos();
                            let direction = vec2(cos, sin);
                            let from = frame.center + direction * (radius - notch);
                            let to = frame.center + direction * (radius + thickness + notch);
                            draw_line(
                                from.x,
                                from.y,
                                to.x,
                                to.y,
                                unit,
                                color(look.hud_color, alpha * HUD_NOTCH_ALPHA),
                            );
                        }
                    }
                }
                HudKind::Dashes {
                    count,
                    duty,
                    thickness_px,
                } => {
                    let thickness = thickness_px * unit;
                    // `count` is a small constant (72), so the conversion is exact.
                    let pitch = 360.0 / count as f32;
                    for slot in 0..count {
                        // `slot` is below `count`, so the conversion is exact.
                        let rotation = pitch * slot as f32 + turn.to_degrees();
                        draw_arc(
                            cx,
                            cy,
                            HUD_SIDES,
                            radius - thickness / 2.0,
                            rotation,
                            thickness,
                            pitch * duty,
                            paint,
                        );
                    }
                }
                HudKind::Ticks {
                    count,
                    major_every,
                    minor_len,
                    major_len,
                } => {
                    let limit = frame.area / 2.0 + vec2(HUD_TICK_CULL_PX, HUD_TICK_CULL_PX) * unit;
                    for slot in 0..count {
                        // `slot` and `count` are small constants (120 at most), so the conversions are exact.
                        let theta = turn + TAU * slot as f32 / count as f32;
                        let (sin, cos) = theta.sin_cos();
                        let direction = vec2(cos, sin);
                        let major = slot % major_every == 0;
                        let length = frame.size * if major { major_len } else { minor_len };
                        let from = direction * radius;
                        // Skip ticks that are entirely off screen.
                        if from.x.abs() > limit.x || from.y.abs() > limit.y {
                            continue;
                        }
                        let to = direction * (radius + length);
                        let (width, boost) = if major {
                            (HUD_MAJOR_TICK_PX, HUD_MAJOR_TICK_BOOST)
                        } else {
                            (HUD_TICK_PX, 1.0)
                        };
                        draw_line(
                            frame.center.x + from.x,
                            frame.center.y + from.y,
                            frame.center.x + to.x,
                            frame.center.y + to.y,
                            width * unit,
                            color(look.hud_color, alpha * boost),
                        );
                    }
                }
            }
        }
    }

    fn draw_band(&self, band: &Band, angle: f32, frame: &Frame) {
        let look = frame.look;
        let alpha = look.band_alpha * frame.lum;
        if alpha < 0.01 {
            return;
        }
        let rgb = look.band_color;
        let radius = frame.size * band.radius * (1.0 + frame.swell);
        let height = frame.size * band.height;
        let (half_w, half_h) = (frame.size * band.width / 2.0, height / 2.0);

        // A darker amber feel behind the glyphs; the HUD draws the lines around them.
        draw_poly_lines(
            frame.center.x,
            frame.center.y,
            BAND_BACKING_SIDES,
            radius - half_h,
            0.0,
            height,
            color(look.fill_color, alpha * BAND_BACKING_ALPHA),
        );

        let line = band.glyph_line_px * frame.unit;
        let limit = frame.area / 2.0 + vec2(height, height);
        for slot in 0..band.glyph_count {
            // `slot` is below glyph_count (58 at most), so the conversions are exact.
            let theta = angle + TAU * slot as f32 / band.glyph_count as f32;
            let (sin, cos) = theta.sin_cos();
            let outward = vec2(cos, sin);
            let along = vec2(-sin, cos);
            let middle = frame.center + outward * radius;
            // Skip glyphs that are entirely off screen.
            let offset = (middle - frame.center).abs();
            if offset.x > limit.x || offset.y > limit.y {
                continue;
            }
            let glyph =
                &self.glyphs[(band.glyph_start + slot * band.glyph_stride) % self.glyphs.len()];
            let shimmer = 0.85 + 0.15 * wave(frame.clock, GLYPH_SHIMMER_RATE, slot as f32 * 1.9);
            let glyph_color = color(rgb, alpha * shimmer);
            let place = |p: Vec2| middle + outward * (p.y * half_h) + along * (p.x * half_w);
            for [a, b] in &glyph.segments {
                let (a, b) = (place(*a), place(*b));
                draw_line(a.x, a.y, b.x, b.y, line, glyph_color);
            }
            for dot in &glyph.dots {
                let at = place(*dot);
                draw_poly(at.x, at.y, SPARKLE_SIDES, line, 0.0, glyph_color);
            }
        }
    }

    fn draw_core_ring(&self, frame: &Frame) {
        let look = frame.look;
        let ring_radius = frame.size * CORE_RING_RADIUS * (1.0 + frame.swell);
        let strength = look.core_glow * frame.lum;
        let turn = self.sparkle_angle * CORE_RING_TURN_RATIO;

        for (scale, alpha) in CORE_FILL_LAYERS {
            draw_poly(
                frame.center.x,
                frame.center.y,
                CORE_FILL_SIDES,
                ring_radius * scale,
                0.0,
                color(
                    look.fill_color,
                    alpha * (0.5 + 0.5 * look.core_glow) * frame.lum,
                ),
            );
        }

        let thickness = ring_radius * CORE_RING_THICKNESS * (0.8 + 0.4 * look.core_glow);
        for (width_scale, alpha, heat) in CORE_RING_LAYERS {
            let half = thickness * width_scale / 2.0;
            // A dim core stays amber; a bright one burns towards white.
            let paint = color(
                glow_color(look, heat * (0.5 + 0.5 * look.core_glow)),
                alpha * strength,
            );
            let point = |index: usize, offset: f32| {
                let vertex = index % CORE_RING_SIDES;
                // `vertex` is below CORE_RING_SIDES (72), so the conversions are exact.
                let theta = turn + TAU * vertex as f32 / CORE_RING_SIDES as f32;
                let (sin, cos) = theta.sin_cos();
                frame.center + vec2(cos, sin) * (ring_radius * self.ring_vertices[vertex] + offset)
            };
            for index in 0..CORE_RING_SIDES {
                let (inner_a, outer_a) = (point(index, -half), point(index, half));
                let (inner_b, outer_b) = (point(index + 1, -half), point(index + 1, half));
                draw_triangle(inner_a, outer_a, outer_b, paint);
                draw_triangle(inner_a, outer_b, inner_b, paint);
            }
        }
    }

    fn draw_sparkles(&self, frame: &Frame) {
        let look = frame.look;
        let ring_radius = frame.size * CORE_RING_RADIUS * (1.0 + frame.swell);
        let (sin, cos) = self.sparkle_angle.sin_cos();
        for sparkle in &self.sparkles {
            let jitter = look.node_jitter
                * JITTER_AMPLITUDE
                * wave(frame.clock, sparkle.rate, sparkle.phase);
            let turned = vec2(
                sparkle.pos.x * cos - sparkle.pos.y * sin,
                sparkle.pos.x * sin + sparkle.pos.y * cos,
            );
            let at = frame.center + turned * (ring_radius * SPARKLE_REACH * (1.0 + jitter));
            let ripple = 0.5 + 0.5 * wave(frame.clock, PULSE_WAVE_RATE, sparkle.phase);
            let pulse = look.node_pulse * ripple;
            let size = SPARKLE_BASE_PX * frame.unit * (1.0 + SPARKLE_PULSE_GAIN * pulse);
            let alpha = frame.lum * SPARKLE_ALPHA + SPARKLE_PULSE_BRIGHTNESS * pulse;
            draw_poly(
                at.x,
                at.y,
                SPARKLE_SIDES,
                size,
                0.0,
                color(whiten(look.core_color, 0.6), alpha),
            );
        }
    }

    fn draw_starburst(&self, frame: &Frame) {
        let look = frame.look;
        let ring_radius = frame.size * CORE_RING_RADIUS * (1.0 + frame.swell);
        let glow = 0.7 + 0.5 * look.core_glow;
        let strength = look.core_glow * frame.lum;

        for (scale, alpha, heat) in STAR_GLOW_LAYERS {
            draw_poly(
                frame.center.x,
                frame.center.y,
                STAR_GLOW_SIDES,
                ring_radius * scale * glow,
                0.0,
                color(glow_color(look, heat), alpha * strength),
            );
        }

        let rgb = whiten(look.core_color, 0.75);
        for ray in &self.star_rays {
            let shimmer = 0.75 + 0.25 * wave(frame.clock, RAY_SHIMMER_RATE, ray.phase);
            let angle = ray.angle + self.sparkle_angle;
            let (sin, cos) = angle.sin_cos();
            let direction = vec2(cos, sin);
            let from = frame.center + direction * (ring_radius * STAR_RAY_START);
            let to = frame.center
                + direction * (ring_radius * STAR_RAY_REACH * ray.reach * glow * shimmer);
            draw_line(
                from.x,
                from.y,
                to.x,
                to.y,
                STAR_RAY_PX * frame.unit,
                color(rgb, STAR_RAY_ALPHA * strength),
            );
        }

        for twinkle in &self.twinkles {
            let flash = (0.5 + 0.5 * wave(frame.clock, twinkle.rate, twinkle.phase)).powi(2);
            let arm = ring_radius
                * twinkle.size
                * (0.4 + 0.6 * flash)
                * (1.0 + TWINKLE_PULSE_GAIN * look.node_pulse);
            let at = frame.center + twinkle.pos * ring_radius;
            let paint = color(rgb, (0.2 + 0.8 * flash) * strength);
            let width = TWINKLE_PX * frame.unit;
            draw_line(at.x - arm, at.y, at.x + arm, at.y, width, paint);
            draw_line(at.x, at.y - arm, at.x, at.y + arm, width, paint);
        }
    }
}

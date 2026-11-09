//! Perspective crop: map a four-corner quad of a photo onto a flat rectangle.

/// A point in normalised image coordinates (0..1 on both axes).
pub type Pt = (f64, f64);

/// Corners in order top-left, top-right, bottom-right, bottom-left.
pub type Quad = [Pt; 4];

pub const FULL: Quad = [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)];

/// Projective map from the unit square onto a quad (Heckbert's square-to-quad).
#[derive(Debug, Clone, Copy)]
struct Homography([f64; 8]);

impl Homography {
    fn square_to_quad(q: &[Pt; 4]) -> Self {
        let [(x0, y0), (x1, y1), (x2, y2), (x3, y3)] = *q;
        let (sx, sy) = (x0 - x1 + x2 - x3, y0 - y1 + y2 - y3);
        if sx.abs() < 1e-12 && sy.abs() < 1e-12 {
            // Parallelogram: affine.
            return Self([x1 - x0, x3 - x0, x0, y1 - y0, y3 - y0, y0, 0.0, 0.0]);
        }
        let (dx1, dx2, dy1, dy2) = (x1 - x2, x3 - x2, y1 - y2, y3 - y2);
        let den = dx1 * dy2 - dx2 * dy1;
        let g = (sx * dy2 - dx2 * sy) / den;
        let h = (dx1 * sy - sx * dy1) / den;
        Self([x1 - x0 + g * x1, x3 - x0 + h * x3, x0, y1 - y0 + g * y1, y3 - y0 + h * y3, y0, g, h])
    }

    #[inline]
    fn map(&self, u: f64, v: f64) -> Pt {
        let [a, b, c, d, e, f, g, h] = self.0;
        let w = g * u + h * v + 1.0;
        ((a * u + b * v + c) / w, (d * u + e * v + f) / w)
    }
}

fn dist(a: Pt, b: Pt) -> f64 {
    ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt()
}

/// Natural size (in source pixels) of the flattened quad.
pub fn natural_size(q: &Quad, w: u32, h: u32) -> (f64, f64) {
    let p = q.map(|(x, y)| (x * w as f64, y * h as f64));
    let width = dist(p[0], p[1]).max(dist(p[3], p[2]));
    let height = dist(p[0], p[3]).max(dist(p[1], p[2]));
    (width.max(1.0), height.max(1.0))
}

/// True when the quad is (near enough) the whole image, so no warp is needed.
pub fn is_full(q: &Quad) -> bool {
    q.iter().zip(FULL.iter()).all(|(a, b)| dist(*a, *b) < 1e-3)
}

/// Warp the quad `q` of an RGBA image (`src`, `sw`×`sh`) into an `ow`×`oh`
/// RGBA image with bilinear sampling. Outside the source reads as white.
pub fn warp(src: &[u8], sw: u32, sh: u32, q: &Quad, ow: u32, oh: u32) -> Vec<u8> {
    let quad = q.map(|(x, y)| (x * sw as f64 - 0.5, y * sh as f64 - 0.5));
    let hm = Homography::square_to_quad(&quad);
    let (sw, sh) = (sw as usize, sh as usize);
    let mut out = vec![255u8; ow as usize * oh as usize * 4];
    for oy in 0..oh as usize {
        let v = (oy as f64 + 0.5) / oh as f64;
        for ox in 0..ow as usize {
            let u = (ox as f64 + 0.5) / ow as f64;
            let (x, y) = hm.map(u, v);
            if !(x > -1.0 && y > -1.0 && x < sw as f64 && y < sh as f64) {
                continue;
            }
            let (x0, y0) = (x.floor(), y.floor());
            let (fx, fy) = ((x - x0) as f32, (y - y0) as f32);
            let (x0, y0) = (x0 as isize, y0 as isize);
            let px = |xx: isize, yy: isize, ch: usize| -> f32 {
                if xx < 0 || yy < 0 || xx as usize >= sw || yy as usize >= sh {
                    255.0
                } else {
                    src[(yy as usize * sw + xx as usize) * 4 + ch] as f32
                }
            };
            let o = (oy * ow as usize + ox) * 4;
            for ch in 0..3 {
                let top = px(x0, y0, ch) * (1.0 - fx) + px(x0 + 1, y0, ch) * fx;
                let bot = px(x0, y0 + 1, ch) * (1.0 - fx) + px(x0 + 1, y0 + 1, ch) * fx;
                out[o + ch] = (top * (1.0 - fy) + bot * fy).round() as u8;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn checker(w: u32, h: u32) -> Vec<u8> {
        let mut v = Vec::new();
        for y in 0..h {
            for x in 0..w {
                let c = ((x * 7 + y * 13) % 256) as u8;
                v.extend_from_slice(&[c, c / 2, 255 - c, 255]);
            }
        }
        v
    }

    #[test]
    fn full_quad_is_identity() {
        let src = checker(40, 30);
        assert_eq!(warp(&src, 40, 30, &FULL, 40, 30), src);
        assert!(is_full(&FULL));
    }

    #[test]
    fn crops_a_sub_rectangle() {
        let src = checker(40, 30);
        let q = [(0.25, 0.5), (0.75, 0.5), (0.75, 1.0), (0.25, 1.0)];
        assert_eq!(natural_size(&q, 40, 30), (20.0, 15.0));
        let out = warp(&src, 40, 30, &q, 20, 15);
        for y in 0..15 {
            for x in 0..20 {
                let s = ((y + 15) * 40 + x + 10) * 4;
                let o = (y * 20 + x) * 4;
                assert_eq!(&out[o..o + 4], &src[s..s + 4], "at {x},{y}");
            }
        }
    }

    #[test]
    fn maps_corners_of_a_trapezoid() {
        let q = [(0.2, 0.1), (0.7, 0.0), (0.9, 1.0), (0.1, 0.8)];
        let hm = Homography::square_to_quad(&q);
        for (uv, want) in [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)].iter().zip(q) {
            let got = hm.map(uv.0, uv.1);
            assert!(dist(got, want) < 1e-9, "{got:?} vs {want:?}");
        }
    }
}

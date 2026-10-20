// Copyright (C) 2026 Toit contributors.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{fs, path::Path, process::Command};
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}
impl Rect {
    pub fn intersects(self, b: Self) -> bool {
        self.x < b.x + b.w && b.x < self.x + self.w && self.y < b.y + b.h && b.y < self.y + self.h
    }
    pub fn valid(self) -> bool {
        [self.x, self.y, self.w, self.h]
            .iter()
            .all(|v| v.is_finite())
            && self.x >= 0.0
            && self.y >= 0.0
            && self.w > 0.0
            && self.h > 0.0
    }
}
#[derive(Clone)]
pub struct GrayImage {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<u8>,
}
impl GrayImage {
    pub fn white(width: usize, height: usize) -> Result<Self> {
        ensure!(
            width > 0 && height > 0 && width.checked_mul(height).is_some_and(|n| n <= 40_000_000),
            "Image must contain 1–40,000,000 pixels"
        );
        Ok(Self {
            width,
            height,
            pixels: vec![255; width * height],
        })
    }
    pub fn read(path: &Path) -> Result<Self> {
        let bytes = if path.extension().is_some_and(|s| s == "pgm") {
            fs::read(path)?
        } else {
            let out = Command::new("magick")
                .arg(path)
                .args([
                    "-background",
                    "white",
                    "-alpha",
                    "remove",
                    "-colorspace",
                    "Gray",
                    "-depth",
                    "8",
                    "pgm:-",
                ])
                .output()
                .context("ImageMagick is needed for non-PGM images")?;
            ensure!(
                out.status.success(),
                "Image conversion failed: {}",
                String::from_utf8_lossy(&out.stderr)
            );
            out.stdout
        };
        Self::from_pgm(&bytes)
    }
    pub fn from_pgm(data: &[u8]) -> Result<Self> {
        let mut i = 0;
        fn token<'a>(data: &'a [u8], i: &mut usize) -> Result<&'a str> {
            loop {
                while *i < data.len() && data[*i].is_ascii_whitespace() {
                    *i += 1;
                }
                if data.get(*i) == Some(&b'#') {
                    while *i < data.len() && data[*i] != b'\n' {
                        *i += 1;
                    }
                } else {
                    break;
                }
            }
            let start = *i;
            while *i < data.len() && !data[*i].is_ascii_whitespace() {
                *i += 1;
            }
            ensure!(*i > start, "Truncated PGM header");
            Ok(std::str::from_utf8(&data[start..*i])?)
        }
        ensure!(token(data, &mut i)? == "P5", "Only binary PGM is supported");
        let w = token(data, &mut i)?.parse()?;
        let h = token(data, &mut i)?.parse()?;
        ensure!(token(data, &mut i)? == "255", "Expected 8-bit PGM");
        ensure!(
            data.get(i).is_some_and(u8::is_ascii_whitespace),
            "Missing PGM separator"
        );
        if data.get(i..i + 2) == Some(b"\r\n") {
            i += 2;
        } else {
            i += 1;
        }
        let mut image = Self::white(w, h)?;
        ensure!(data.len() - i == image.pixels.len(), "PGM size mismatch");
        image.pixels.copy_from_slice(&data[i..]);
        Ok(image)
    }
    pub fn save(&self, path: &Path) -> Result<()> {
        use std::io::Write;
        let mut f = fs::File::create(path)?;
        write!(f, "P5\n{} {}\n255\n", self.width, self.height)?;
        f.write_all(&self.pixels)?;
        Ok(())
    }
    pub fn crop(&self, r: Rect) -> Result<Self> {
        ensure!(r.valid(), "Invalid rectangle");
        let x = r.x.floor() as usize;
        let y = r.y.floor() as usize;
        let right = (r.x + r.w).ceil() as usize;
        let bottom = (r.y + r.h).ceil() as usize;
        ensure!(
            right <= self.width && bottom <= self.height,
            "Rectangle outside image"
        );
        let mut out = Self::white(right - x, bottom - y)?;
        for row in 0..out.height {
            out.pixels[row * out.width..(row + 1) * out.width].copy_from_slice(
                &self.pixels[(y + row) * self.width + x..(y + row) * self.width + right],
            );
        }
        Ok(out)
    }
    pub fn ink_bounds(&self, threshold: u8) -> Option<Rect> {
        let (mut l, mut t, mut r, mut b) = (self.width, self.height, 0, 0);
        for y in 0..self.height {
            for x in 0..self.width {
                if self.pixels[y * self.width + x] < threshold {
                    l = l.min(x);
                    t = t.min(y);
                    r = r.max(x + 1);
                    b = b.max(y + 1);
                }
            }
        }
        (r > l && b > t).then_some(Rect {
            x: l as f64,
            y: t as f64,
            w: r.saturating_sub(l) as f64,
            h: b.saturating_sub(t) as f64,
        })
    }
    pub fn mosaic(&self, block: usize, phase_x: usize, phase_y: usize) -> Result<Self> {
        ensure!(
            block > 0 && block <= 128 && phase_x < block && phase_y < block,
            "Invalid mosaic grid"
        );
        let mut out = self.clone();
        let mut y = -(phase_y as isize);
        while y < self.height as isize {
            let mut x = -(phase_x as isize);
            while x < self.width as isize {
                let x0 = x.max(0) as usize;
                let y0 = y.max(0) as usize;
                let x1 = (x + block as isize).min(self.width as isize) as usize;
                let y1 = (y + block as isize).min(self.height as isize) as usize;
                let mut sum = 0u64;
                for yy in y0..y1 {
                    for xx in x0..x1 {
                        sum += self.pixels[yy * self.width + xx] as u64;
                    }
                }
                let value = (sum / ((x1 - x0) * (y1 - y0)) as u64) as u8;
                for yy in y0..y1 {
                    out.pixels[yy * self.width + x0..yy * self.width + x1].fill(value);
                }
                x += block as isize;
            }
            y += block as isize;
        }
        Ok(out)
    }
    pub fn mse(&self, b: &Self) -> Result<f64> {
        ensure!(
            self.width == b.width && self.height == b.height,
            "Image sizes differ"
        );
        Ok(self
            .pixels
            .iter()
            .zip(&b.pixels)
            .map(|(a, b)| (*a as f64 - *b as f64).powi(2))
            .sum::<f64>()
            / self.pixels.len() as f64)
    }
    pub fn paste(&mut self, other: &Self, x: usize, y: usize) -> Result<()> {
        ensure!(
            x + other.width <= self.width && y + other.height <= self.height,
            "Paste outside image"
        );
        for row in 0..other.height {
            self.pixels[(y + row) * self.width + x..(y + row) * self.width + x + other.width]
                .copy_from_slice(&other.pixels[row * other.width..(row + 1) * other.width]);
        }
        Ok(())
    }
    pub fn fill(&mut self, r: Rect, value: u8) -> Result<()> {
        ensure!(
            r.valid() && r.x + r.w <= self.width as f64 && r.y + r.h <= self.height as f64,
            "Invalid fill"
        );
        for y in r.y as usize..(r.y + r.h) as usize {
            self.pixels[y * self.width + r.x as usize..y * self.width + (r.x + r.w) as usize]
                .fill(value);
        }
        Ok(())
    }
    /// Include dense gray scan fringes around a known, near-solid bar when
    /// preparing OCR input. Never modifies original pixels or expands through
    /// a white gap into neighboring text. Limit extension to eight pixels/side.
    pub fn ocr_mask_bounds(&self, r: Rect) -> Result<Rect> {
        ensure!(
            r.valid() && r.x + r.w <= self.width as f64 && r.y + r.h <= self.height as f64,
            "OCR mask outside image"
        );
        let (mut l, mut t, mut right, mut bottom) = (
            r.x.floor() as usize,
            r.y.floor() as usize,
            (r.x + r.w).ceil() as usize,
            (r.y + r.h).ceil() as usize,
        );
        let initial = (l, t, right, bottom);
        let dark = (t..bottom)
            .map(|y| {
                self.pixels[y * self.width + l..y * self.width + right]
                    .iter()
                    .filter(|p| **p < 40)
                    .count()
            })
            .sum::<usize>();
        if right - l < 20 || bottom - t < 5 || dark * 10 < (right - l) * (bottom - t) * 9 {
            return Ok(r);
        }
        let column = |x: usize, top: usize, end: usize| {
            (top..end)
                .filter(|y| self.pixels[y * self.width + x] < 224)
                .count()
                * 4
                >= (end - top) * 3
        };
        let row = |y: usize, start: usize, end: usize| {
            self.pixels[y * self.width + start..y * self.width + end]
                .iter()
                .filter(|p| **p < 224)
                .count()
                * 4
                >= (end - start) * 3
        };
        for _ in 0..8 {
            if l > 0 && initial.0 - l < 8 && column(l - 1, t, bottom) {
                l -= 1;
            }
            if right < self.width && right - initial.2 < 8 && column(right, t, bottom) {
                right += 1;
            }
            if t > 0 && initial.1 - t < 8 && row(t - 1, l, right) {
                t -= 1;
            }
            if bottom < self.height && bottom - initial.3 < 8 && row(bottom, l, right) {
                bottom += 1;
            }
        }
        Ok(Rect {
            x: l as f64,
            y: t as f64,
            w: (right - l) as f64,
            h: (bottom - t) as f64,
        })
    }

    // Conservative proposal detector: long, near-solid horizontal runs joined across rows.
    // Raw geometric proposals; callers filter against local text scale before
    // using them to exclude OCR observations.
    pub fn rectangles(&self) -> Vec<Rect> {
        let mut active: Vec<(usize, usize, usize, usize)> = vec![];
        let mut found = vec![];
        for y in 0..=self.height {
            let mut runs = vec![];
            if y < self.height {
                let mut x = 0;
                while x < self.width {
                    if self.pixels[y * self.width + x] < 40 {
                        let start = x;
                        while x < self.width && self.pixels[y * self.width + x] < 40 {
                            x += 1;
                        }
                        if x - start >= 20 {
                            runs.push((start, x));
                        }
                    } else {
                        x += 1;
                    }
                }
            }
            let mut next = vec![];
            for (l, r) in runs {
                if let Some(i) = active
                    .iter()
                    .position(|(a, b, _, _)| a.abs_diff(l) <= 2 && b.abs_diff(r) <= 2)
                {
                    let (a, b, t, _) = active.remove(i);
                    next.push((a.min(l), b.max(r), t, y + 1));
                } else {
                    next.push((l, r, y, y + 1));
                }
            }
            for (l, r, t, b) in active {
                if b - t >= 5 && r - l >= 2 * (b - t) {
                    found.push(Rect {
                        x: l as f64,
                        y: t as f64,
                        w: (r - l) as f64,
                        h: (b - t) as f64,
                    });
                }
            }
            active = next;
        }
        found
    }
}

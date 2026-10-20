// Copyright (C) 2026 Toit contributors.
use crate::raster::GrayImage;
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::{
    ffi::{CStr, CString, c_char, c_int, c_void},
    path::Path,
    ptr::NonNull,
};

/// Measurement hypothesis; ppem is the source raster grid, independent of scan scale.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MeasurementMode {
    #[default]
    Design,
    FreeTypeHinted {
        ppem: u16,
    },
}
impl MeasurementMode {
    pub fn valid(self) -> bool {
        match self {
            Self::Design => true,
            Self::FreeTypeHinted { ppem } => (6..=256).contains(&ppem),
        }
    }
}

#[repr(C)]
#[derive(Default, Debug, Clone, Copy)]
pub struct Metrics {
    pub advance: f64,
    pub left: f64,
    pub top: f64,
    pub right: f64,
    pub bottom: f64,
    pub glyphs: c_int,
    pub missing: c_int,
}
impl Metrics {
    pub fn ink_width(self) -> f64 {
        self.right - self.left
    }
}
#[repr(C)]
struct RawChar {
    codepoint: u32,
    x1: f64,
    y1: f64,
    x2: f64,
    y2: f64,
    size: f64,
    font: [c_char; 128],
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NativeChar {
    pub text: char,
    pub x1: f64,
    pub y1: f64,
    pub x2: f64,
    pub y2: f64,
    pub size: f64,
    pub font: String,
}

unsafe extern "C" {
    fn ur_font_open(data: *const u8, length: u32) -> *mut c_void;
    fn ur_font_close(f: *mut c_void);
    fn ur_measure(f: *mut c_void, text: *const c_char, kern: c_int, m: *mut Metrics) -> c_int;
    fn ur_measure_hinted(
        f: *mut c_void,
        text: *const c_char,
        kern: c_int,
        ppem: c_int,
        m: *mut Metrics,
    ) -> c_int;
    fn ur_render(
        f: *mut c_void,
        text: *const c_char,
        size: f64,
        kern: c_int,
        tracking: f64,
        x: f64,
        y: f64,
        w: c_int,
        h: c_int,
        out: *mut u8,
    ) -> c_int;
    fn ur_pdf_open(path: *const c_char, error: *mut c_char, capacity: c_int) -> *mut c_void;
    fn ur_render_vertical(
        f: *mut c_void,
        text: *const c_char,
        size: f64,
        kern: c_int,
        tracking: f64,
        x: f64,
        y: f64,
        vertical_scale: f64,
        w: c_int,
        h: c_int,
        out: *mut u8,
    ) -> c_int;
    fn ur_pdf_close(doc: *mut c_void);
    fn ur_pdf_pages(doc: *mut c_void) -> c_int;
    fn ur_pdf_size(doc: *mut c_void, index: c_int, w: *mut f64, h: *mut f64) -> c_int;
    fn ur_pdf_render(
        doc: *mut c_void,
        index: c_int,
        scale: f64,
        w: c_int,
        h: c_int,
        out: *mut u8,
    ) -> c_int;
    fn ur_pdf_chars(
        doc: *mut c_void,
        index: c_int,
        count: *mut c_int,
        image_fraction: *mut f64,
    ) -> *mut RawChar;
    fn ur_free(ptr: *mut c_void);
    fn ur_pdf_canvas(path: *const c_char, w: f64, h: f64) -> *mut c_void;
    fn ur_pdf_text(
        cr: *mut c_void,
        f: *mut c_void,
        text: *const c_char,
        size: f64,
        kern: c_int,
        tracking: f64,
        x: f64,
        y: f64,
    );
    fn ur_pdf_rect(cr: *mut c_void, x: f64, y: f64, w: f64, h: f64);
    fn ur_pdf_finish(cr: *mut c_void) -> c_int;
}
pub struct Font(NonNull<c_void>, String);
impl Font {
    pub fn open(path: &Path) -> Result<Self> {
        let bytes = std::fs::read(path).context("Unable to read font")?;
        ensure!(
            !bytes.is_empty() && bytes.len() <= 1 << 30,
            "Invalid font file size"
        );
        let fingerprint = crate::project::hash(&bytes);
        NonNull::new(unsafe { ur_font_open(bytes.as_ptr(), bytes.len() as u32) })
            .map(|pointer| Self(pointer, fingerprint))
            .context("Unable to load font (face index 0)")
    }
    pub fn sha256(&self) -> &str {
        &self.1
    }
    pub fn measure(&self, text: &str, kern: bool) -> Result<Metrics> {
        self.measure_mode(text, kern, MeasurementMode::Design)
    }
    pub fn measure_mode(&self, text: &str, kern: bool, mode: MeasurementMode) -> Result<Metrics> {
        ensure!(mode.valid(), "Hinted ppem must be 6–256");
        ensure!(
            !text.is_empty() && text.len() <= 4096,
            "Text must contain 1–4096 bytes"
        );
        let text = CString::new(text)?;
        let mut m = Metrics::default();
        let ok = unsafe {
            match mode {
                MeasurementMode::Design => {
                    ur_measure(self.0.as_ptr(), text.as_ptr(), kern.into(), &mut m)
                }
                MeasurementMode::FreeTypeHinted { ppem } => ur_measure_hinted(
                    self.0.as_ptr(),
                    text.as_ptr(),
                    kern.into(),
                    ppem.into(),
                    &mut m,
                ),
            }
        };
        ensure!(ok != 0, "Font measurement failed");
        ensure!(m.missing == 0, "Font does not cover all characters");
        Ok(m)
    }
    #[allow(
        clippy::too_many_arguments,
        reason = "Thin typed wrapper for the native text-rendering operation"
    )]
    pub fn render(
        &self,
        text: &str,
        size: f64,
        kern: bool,
        tracking: f64,
        x: f64,
        y: f64,
        w: usize,
        h: usize,
    ) -> Result<GrayImage> {
        ensure!(
            size.is_finite()
                && size > 0.0
                && size <= 2000.0
                && tracking.is_finite()
                && x.is_finite()
                && y.is_finite(),
            "Invalid rendering parameters"
        );
        self.measure(text, kern)?;
        let text = CString::new(text)?;
        let mut image = GrayImage::white(w, h)?;
        ensure!(
            unsafe {
                ur_render(
                    self.0.as_ptr(),
                    text.as_ptr(),
                    size,
                    kern.into(),
                    tracking,
                    x,
                    y,
                    w as c_int,
                    h as c_int,
                    image.pixels.as_mut_ptr(),
                )
            } != 0,
            "Rendering failed"
        );
        Ok(image)
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "Typed wrapper for independent vertical rendering scale"
    )]
    pub fn render_vertical(
        &self,
        text: &str,
        size: f64,
        kern: bool,
        tracking: f64,
        x: f64,
        y: f64,
        vertical_scale: f64,
        w: usize,
        h: usize,
    ) -> Result<GrayImage> {
        ensure!(
            vertical_scale.is_finite() && (0.25..=4.).contains(&vertical_scale),
            "Invalid vertical scale"
        );
        if vertical_scale == 1. {
            return self.render(text, size, kern, tracking, x, y, w, h);
        }
        ensure!(
            size.is_finite()
                && size > 0.
                && size <= 2000.
                && tracking.is_finite()
                && x.is_finite()
                && y.is_finite(),
            "Invalid rendering parameters"
        );
        self.measure(text, kern)?;
        let text = CString::new(text)?;
        let mut image = GrayImage::white(w, h)?;
        ensure!(
            unsafe {
                ur_render_vertical(
                    self.0.as_ptr(),
                    text.as_ptr(),
                    size,
                    kern.into(),
                    tracking,
                    x,
                    y,
                    vertical_scale,
                    w as c_int,
                    h as c_int,
                    image.pixels.as_mut_ptr(),
                )
            } != 0,
            "Rendering failed"
        );
        Ok(image)
    }
}
impl Drop for Font {
    fn drop(&mut self) {
        unsafe { ur_font_close(self.0.as_ptr()) }
    }
}
pub struct Pdf(NonNull<c_void>);
impl Pdf {
    pub fn open(path: &Path) -> Result<Self> {
        let path = path.canonicalize()?;
        let path = CString::new(path.as_os_str().as_encoded_bytes())?;
        let mut error = [0 as c_char; 1024];
        let ptr = unsafe { ur_pdf_open(path.as_ptr(), error.as_mut_ptr(), 1024) };
        match NonNull::new(ptr) {
            Some(p) => Ok(Self(p)),
            None => bail!(
                "PDF open failed: {}",
                unsafe { CStr::from_ptr(error.as_ptr()) }.to_string_lossy()
            ),
        }
    }
    pub fn pages(&self) -> usize {
        (unsafe { ur_pdf_pages(self.0.as_ptr()) }) as usize
    }
    pub fn render(&self, index: usize, dpi: f64) -> Result<GrayImage> {
        ensure!(
            index < self.pages() && (36.0..=1200.0).contains(&dpi),
            "Invalid page or DPI"
        );
        let (mut w, mut h) = (0.0, 0.0);
        ensure!(
            unsafe { ur_pdf_size(self.0.as_ptr(), index as c_int, &mut w, &mut h) } != 0,
            "Page missing"
        );
        let scale = dpi / 72.0;
        let mut image = GrayImage::white((w * scale).ceil() as usize, (h * scale).ceil() as usize)?;
        ensure!(
            unsafe {
                ur_pdf_render(
                    self.0.as_ptr(),
                    index as c_int,
                    scale,
                    image.width as c_int,
                    image.height as c_int,
                    image.pixels.as_mut_ptr(),
                )
            } != 0,
            "PDF rendering failed"
        );
        Ok(image)
    }
    pub fn chars(&self, index: usize) -> Result<(Vec<NativeChar>, f64)> {
        ensure!(index < self.pages(), "Invalid page");
        let (mut count, mut fraction) = (0, 0.0);
        let ptr =
            unsafe { ur_pdf_chars(self.0.as_ptr(), index as c_int, &mut count, &mut fraction) };
        if ptr.is_null() {
            return Ok((vec![], fraction));
        }
        let raw = unsafe { std::slice::from_raw_parts(ptr, count as usize) };
        let result = raw
            .iter()
            .filter_map(|c| {
                Some(NativeChar {
                    text: char::from_u32(c.codepoint)?,
                    x1: c.x1,
                    y1: c.y1,
                    x2: c.x2,
                    y2: c.y2,
                    size: c.size,
                    font: unsafe { CStr::from_ptr(c.font.as_ptr()) }
                        .to_string_lossy()
                        .into_owned(),
                })
            })
            .collect();
        unsafe { ur_free(ptr.cast()) };
        Ok((result, fraction))
    }
}
impl Drop for Pdf {
    fn drop(&mut self) {
        unsafe { ur_pdf_close(self.0.as_ptr()) }
    }
}
pub struct PdfCanvas(Option<NonNull<c_void>>);
impl PdfCanvas {
    pub fn new(path: &Path, w: f64, h: f64) -> Result<Self> {
        let path = CString::new(path.as_os_str().as_encoded_bytes())?;
        Ok(Self(Some(
            NonNull::new(unsafe { ur_pdf_canvas(path.as_ptr(), w, h) })
                .context("Cannot create PDF")?,
        )))
    }
    #[allow(
        clippy::too_many_arguments,
        reason = "Fixture drawing mirrors native text layout parameters"
    )]
    pub fn text(
        &self,
        font: &Font,
        text: &str,
        size: f64,
        kern: bool,
        tracking: f64,
        x: f64,
        y: f64,
    ) -> Result<()> {
        font.measure(text, kern)?;
        let text = CString::new(text)?;
        unsafe {
            ur_pdf_text(
                self.0.unwrap().as_ptr(),
                font.0.as_ptr(),
                text.as_ptr(),
                size,
                kern.into(),
                tracking,
                x,
                y,
            )
        };
        Ok(())
    }
    pub fn rect(&self, x: f64, y: f64, w: f64, h: f64) {
        unsafe { ur_pdf_rect(self.0.unwrap().as_ptr(), x, y, w, h) }
    }
    pub fn finish(mut self) -> Result<()> {
        ensure!(
            unsafe { ur_pdf_finish(self.0.take().unwrap().as_ptr()) } != 0,
            "Cannot write PDF"
        );
        Ok(())
    }
}
impl Drop for PdfCanvas {
    fn drop(&mut self) {
        if let Some(p) = self.0.take() {
            unsafe {
                ur_pdf_finish(p.as_ptr());
            }
        }
    }
}

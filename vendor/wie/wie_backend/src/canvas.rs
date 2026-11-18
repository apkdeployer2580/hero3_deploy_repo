mod lbmp;

use alloc::{borrow::Cow, boxed::Box, string::ToString, vec, vec::Vec};
use core::mem::size_of;

use ab_glyph::{Font, FontRef, ScaleFont};
use bytemuck::{Pod, cast_slice, pod_collect_to_vec};
use image::ImageReader;
use num_traits::{Num, Zero};

use wie_util::{Result, WieError};

use self::lbmp::decode_lbmp;

lazy_static::lazy_static! {
    static ref FONT: FontRef<'static> = FontRef::try_from_slice(include_bytes!("../../fonts/neodgm.ttf")).unwrap();
}

pub enum TextAlignment {
    Left,
    Center,
    Right,
}

#[derive(Clone, Copy)]
pub struct Color {
    pub a: u8,
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

pub trait Image: Send {
    fn width(&self) -> u32;
    fn height(&self) -> u32;
    fn bytes_per_pixel(&self) -> u32;
    fn get_pixel(&self, x: i32, y: i32) -> Color;
    fn raw(&self) -> Cow<'_, [u8]>;
    fn colors(&self) -> Vec<Color>;
}

pub trait ImageBuffer: Send {
    fn put_pixel(&mut self, x: i32, y: i32, color: Color);
    fn put_pixels(&mut self, x: i32, y: i32, width: u32, colors: &[Color]);
    fn xor_pixel(&mut self, x: i32, y: i32, color: Color);

    /// Fill the half-open rect [x0, x1) x [y0, y1) with a solid colour. The caller
    /// has ALREADY clamped the bounds to the buffer; the default falls back to
    /// per-pixel puts, concrete buffers can overwrite whole row spans at once.
    ///
    /// ★ Added for wie_android's heavy-map lag: `Canvas::fill_rect` used to go
    /// through `put_pixel` for every pixel (bounds check + colour conversion each),
    /// which made the game's full-screen fills (map transitions, the blackout-event
    /// darkening overlays) cost tens of millions of per-pixel calls per second.
    fn fill_span(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, color: Color) {
        for y in y0..y1 {
            for x in x0..x1 {
                self.put_pixel(x, y, color);
            }
        }
    }

    /// Copy one row of RAW pixel data (same pixel format as this buffer) to
    /// `(x, y)`. Returns `false` when the buffer cannot take raw rows (the caller
    /// then uses the per-pixel path). Bounds must already be clamped.
    fn put_row_raw(&mut self, _x: i32, _y: i32, _data: &[u8]) -> bool {
        false
    }
}

#[allow(clippy::too_many_arguments)]
pub trait Canvas: Send {
    fn image(&self) -> &dyn Image;
    fn set_xor_mode(&mut self, xor_mode: bool);
    fn copy_area(&mut self, dx: i32, dy: i32, sx: i32, sy: i32, w: u32, h: u32, clip: Clip);
    fn draw(&mut self, dx: i32, dy: i32, w: u32, h: u32, src: &dyn Image, sx: i32, sy: i32, clip: Clip);
    fn draw_line(&mut self, x1: i32, y1: i32, x2: i32, y2: i32, color: Color);
    fn draw_text(&mut self, string: &str, x: i32, y: i32, text_alignment: TextAlignment, color: Color);
    fn draw_rect(&mut self, x: i32, y: i32, w: u32, h: u32, color: Color, clip: Clip);
    fn draw_arc(&mut self, x: i32, y: i32, w: u32, h: u32, start_angle: i32, arc_angle: i32, color: Color, clip: Clip);
    fn draw_round_rect(&mut self, x: i32, y: i32, w: u32, h: u32, arc_width: u32, arc_height: u32, color: Color, clip: Clip);
    fn fill_rect(&mut self, x: i32, y: i32, w: u32, h: u32, color: Color, clip: Clip);
    fn fill_arc(&mut self, x: i32, y: i32, w: u32, h: u32, start_angle: i32, arc_angle: i32, color: Color, clip: Clip);
    fn fill_round_rect(&mut self, x: i32, y: i32, w: u32, h: u32, arc_width: u32, arc_height: u32, color: Color, clip: Clip);
    fn put_pixel(&mut self, x: i32, y: i32, color: Color);
}

pub trait PixelType: Send {
    type DataType: Copy + Pod + Num + Send;
    fn from_color(color: Color) -> Self::DataType;
    fn to_color(raw: Self::DataType) -> Color;
    fn xor_color(raw: Self::DataType, color: Color) -> Self::DataType;
}

pub struct Rgb332Pixel;

impl PixelType for Rgb332Pixel {
    type DataType = u8;

    fn from_color(color: Color) -> Self::DataType {
        let r = ((color.r as u16 * 7 + 127) / 255) as u8;
        let g = ((color.g as u16 * 7 + 127) / 255) as u8;
        let b = ((color.b as u16 * 3 + 127) / 255) as u8;

        (r << 5) | (g << 2) | b
    }

    fn to_color(raw: Self::DataType) -> Color {
        let r = (raw >> 5) & 0x7;
        let g = (raw >> 2) & 0x7;
        let b = raw & 0x3;

        Color {
            a: 0xff,
            r: r * 36,
            g: g * 36,
            b: b * 85,
        }
    }

    fn xor_color(raw: Self::DataType, color: Color) -> Self::DataType {
        raw ^ Self::from_color(color)
    }
}

pub struct Rgb565Pixel;

impl PixelType for Rgb565Pixel {
    type DataType = u16;

    fn from_color(color: Color) -> Self::DataType {
        let r = (color.r as u16) >> 3;
        let g = (color.g as u16) >> 2;
        let b = (color.b as u16) >> 3;

        (r << 11) | (g << 5) | b
    }

    fn to_color(raw: Self::DataType) -> Color {
        let r = ((raw >> 11) & 0x1f) as u8;
        let g = ((raw >> 5) & 0x3f) as u8;
        let b = (raw & 0x1f) as u8;

        let r = ((r as u32 * 255 + 15) / 31) as u8;
        let g = ((g as u32 * 255 + 31) / 63) as u8;
        let b = ((b as u32 * 255 + 15) / 31) as u8;

        Color { a: 0xff, r, g, b }
    }

    fn xor_color(raw: Self::DataType, color: Color) -> Self::DataType {
        raw ^ Self::from_color(color)
    }
}

pub struct Rgb8Pixel;

impl PixelType for Rgb8Pixel {
    type DataType = u32;

    fn from_color(color: Color) -> Self::DataType {
        ((color.r as u32) << 16) | ((color.g as u32) << 8) | color.b as u32
    }

    fn to_color(raw: Self::DataType) -> Color {
        let r = ((raw >> 16) & 0xff) as u8;
        let g = ((raw >> 8) & 0xff) as u8;
        let b = (raw & 0xff) as u8;

        Color { a: 0xff, r, g, b }
    }

    fn xor_color(raw: Self::DataType, color: Color) -> Self::DataType {
        raw ^ Self::from_color(color)
    }
}

pub struct ArgbPixel;

impl PixelType for ArgbPixel {
    type DataType = u32;

    fn from_color(color: Color) -> Self::DataType {
        ((color.a as u32) << 24) | ((color.r as u32) << 16) | ((color.g as u32) << 8) | color.b as u32
    }

    fn to_color(raw: Self::DataType) -> Color {
        let a = ((raw >> 24) & 0xff) as u8;
        let r = ((raw >> 16) & 0xff) as u8;
        let g = ((raw >> 8) & 0xff) as u8;
        let b = (raw & 0xff) as u8;

        Color { a, r, g, b }
    }

    fn xor_color(raw: Self::DataType, color: Color) -> Self::DataType {
        let source = ((color.r as u32) << 16) | ((color.g as u32) << 8) | color.b as u32;

        (raw ^ source) | 0xff000000
    }
}

pub struct AbgrPixel;

impl PixelType for AbgrPixel {
    type DataType = u32;

    fn from_color(color: Color) -> Self::DataType {
        ((color.a as u32) << 24) | ((color.b as u32) << 16) | ((color.g as u32) << 8) | color.r as u32
    }

    fn to_color(raw: Self::DataType) -> Color {
        let a = ((raw >> 24) & 0xff) as u8;
        let b = ((raw >> 16) & 0xff) as u8;
        let g = ((raw >> 8) & 0xff) as u8;
        let r = (raw & 0xff) as u8;

        Color { a, r, g, b }
    }

    fn xor_color(raw: Self::DataType, color: Color) -> Self::DataType {
        let source = ((color.b as u32) << 16) | ((color.g as u32) << 8) | color.r as u32;

        (raw ^ source) | 0xff000000
    }
}

pub struct VecImageBuffer<T>
where
    T: PixelType,
{
    width: u32,
    height: u32,
    data: Vec<T::DataType>,
}

impl<T> VecImageBuffer<T>
where
    T: PixelType,
{
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            data: vec![T::DataType::zero(); (width * height) as usize],
        }
    }

    pub fn from_raw(width: u32, height: u32, raw: Vec<T::DataType>) -> Self {
        Self { width, height, data: raw }
    }
}

impl<T> Image for VecImageBuffer<T>
where
    T: PixelType + 'static,
{
    fn width(&self) -> u32 {
        self.width
    }

    fn height(&self) -> u32 {
        self.height
    }

    fn bytes_per_pixel(&self) -> u32 {
        size_of::<T::DataType>() as u32
    }

    fn get_pixel(&self, x: i32, y: i32) -> Color {
        let raw = self.data[((y as u32) * self.width + (x as u32)) as usize];

        T::to_color(raw)
    }

    fn raw(&self) -> Cow<'_, [u8]> {
        cast_slice(&self.data).into()
    }

    fn colors(&self) -> Vec<Color> {
        self.data.iter().map(|&x| T::to_color(x)).collect()
    }
}

impl<T> ImageBuffer for VecImageBuffer<T>
where
    T: PixelType + 'static,
{
    fn put_pixel(&mut self, x: i32, y: i32, color: Color) {
        if x < 0 || y < 0 || (x as u32) >= self.width || (y as u32) >= self.height {
            return;
        }

        let raw = T::from_color(color);

        self.data[((y as u32) * self.width + (x as u32)) as usize] = raw;
    }

    fn put_pixels(&mut self, x: i32, y: i32, width: u32, colors: &[Color]) {
        for (i, color) in colors.iter().enumerate() {
            let x = x + (i as i32 % (width as i32));
            let y = y + (i as i32 / (width as i32));

            if x < 0 || y < 0 || (x as u32) >= self.width || (y as u32) >= self.height {
                continue;
            }

            let raw = T::from_color(*color);

            self.data[((y as u32) * self.width + (x as u32)) as usize] = raw;
        }
    }

    fn xor_pixel(&mut self, x: i32, y: i32, color: Color) {
        if x < 0 || y < 0 || (x as u32) >= self.width || (y as u32) >= self.height {
            return;
        }

        let offset = ((y as u32) * self.width + (x as u32)) as usize;
        self.data[offset] = T::xor_color(self.data[offset], color);
    }

    fn fill_span(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, color: Color) {
        // Bounds are pre-clamped by `Canvas::fill_rect`; guard anyway so a misuse
        // cannot panic on the slice index.
        let (w, h) = (self.width as i32, self.height as i32);
        let (x0, x1) = (x0.clamp(0, w), x1.clamp(0, w));
        let (y0, y1) = (y0.clamp(0, h), y1.clamp(0, h));
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        let raw = T::from_color(color);
        for y in y0..y1 {
            let start = (y as u32 * self.width + x0 as u32) as usize;
            self.data[start..start + (x1 - x0) as usize].fill(raw);
        }
    }

    fn put_row_raw(&mut self, x: i32, y: i32, data: &[u8]) -> bool {
        let pixel_size = size_of::<T::DataType>();
        let count = data.len() / pixel_size;
        if x < 0 || y < 0 || (y as u32) >= self.height || data.len() % pixel_size != 0 || (x as u32).saturating_add(count as u32) > self.width {
            // A mis-sized row is a caller bug; report "unsupported" so the caller's
            // per-pixel fallback (which clamps) runs instead of corrupting memory.
            return false;
        }
        let start = ((y as u32) * self.width + (x as u32)) as usize;
        let dst: &mut [u8] = bytemuck::cast_slice_mut(&mut self.data[start..start + count]);
        dst.copy_from_slice(data);
        true
    }
}

pub struct ImageBufferCanvas<T>
where
    T: ImageBuffer + Image,
{
    image_buffer: T,
    xor_mode: bool,
}

impl<T> ImageBufferCanvas<T>
where
    T: ImageBuffer + Image,
{
    pub fn new(image_buffer: T) -> Self {
        Self {
            image_buffer,
            xor_mode: false,
        }
    }

    pub fn into_inner(self) -> T {
        self.image_buffer
    }

    fn blend_pixel(&mut self, x: i32, y: i32, color: Color) {
        self.compose_pixel(x, y, color, true);
    }

    fn compose_pixel(&mut self, x: i32, y: i32, color: Color, blend: bool) {
        if x < 0 || y < 0 || (x as u32) >= self.image_buffer.width() || (y as u32) >= self.image_buffer.height() {
            return;
        }

        if self.xor_mode {
            if color.a == 0 {
                return;
            }

            let color = if blend && color.a < 255 {
                Color {
                    a: 255,
                    r: ((color.r as u16 * color.a as u16 + 127) / 255) as u8,
                    g: ((color.g as u16 * color.a as u16 + 127) / 255) as u8,
                    b: ((color.b as u16 * color.a as u16 + 127) / 255) as u8,
                }
            } else {
                color
            };
            self.image_buffer.xor_pixel(x, y, color);
            return;
        }

        if !blend {
            self.image_buffer.put_pixel(x, y, color);
            return;
        }

        let bg = self.image_buffer.get_pixel(x, y);
        let factor = color.a as f32 / 255.0;

        let computed_color = Color {
            a: 0xff,
            r: (color.r as f32 * factor + bg.r as f32 * (1.0 - factor)) as u8,
            g: (color.g as f32 * factor + bg.g as f32 * (1.0 - factor)) as u8,
            b: (color.b as f32 * factor + bg.b as f32 * (1.0 - factor)) as u8,
        };

        self.image_buffer.put_pixel(x, y, computed_color);
    }

    fn plot(&mut self, x: i32, y: i32, color: Color, clip: &Clip) {
        if x < 0 || y < 0 || (x as u32) >= self.image_buffer.width() || (y as u32) >= self.image_buffer.height() {
            return;
        }
        if x < clip.x || x >= clip.x + clip.width as i32 || y < clip.y || y >= clip.y + clip.height as i32 {
            return;
        }
        self.put_pixel(x, y, color);
    }

    #[allow(clippy::too_many_arguments)]
    fn stroke_arc(&mut self, cx: f32, cy: f32, a: f32, b: f32, start_deg: f32, sweep_deg: f32, color: Color, clip: &Clip) {
        let radius = a.max(b).max(1.0);
        let sweep_rad = sweep_deg.to_radians();
        let start_rad = start_deg.to_radians();
        let steps = ((sweep_rad.abs() * radius).ceil() as i32 * 2).max(1);

        for i in 0..=steps {
            let theta = start_rad + sweep_rad * (i as f32) / (steps as f32);
            let px = (cx + a * theta.cos()).round() as i32;
            let py = (cy - b * theta.sin()).round() as i32;
            self.plot(px, py, color, clip);
        }
    }
}

/// Whether the point lies within the pie sweep starting at `start_deg` spanning
/// `sweep_deg` degrees (counterclockwise for positive, clockwise for negative).
/// Angles follow the J2ME/WIPI convention: 0° is 3 o'clock, positive is CCW.
fn point_in_sweep(px: f32, py: f32, cx: f32, cy: f32, start_deg: f32, sweep_deg: f32) -> bool {
    if sweep_deg.abs() >= 360.0 {
        return true;
    }
    if sweep_deg == 0.0 {
        return false;
    }

    let dx = px - cx;
    let dy = py - cy;
    if dx * dx + dy * dy < 1.0 {
        return true; // apex of the pie
    }

    // screen y grows downward, so negate dy to match the math convention
    let angle = (-dy).atan2(dx).to_degrees();
    let (start, sweep) = if sweep_deg > 0.0 {
        (start_deg, sweep_deg)
    } else {
        (start_deg + sweep_deg, -sweep_deg)
    };

    (angle - start).rem_euclid(360.0) <= sweep
}

#[allow(clippy::too_many_arguments)]
impl<T> Canvas for ImageBufferCanvas<T>
where
    T: ImageBuffer + Image,
{
    fn image(&self) -> &dyn Image {
        &self.image_buffer
    }

    fn set_xor_mode(&mut self, xor_mode: bool) {
        self.xor_mode = xor_mode;
    }

    fn copy_area(&mut self, dx: i32, dy: i32, sx: i32, sy: i32, w: u32, h: u32, clip: Clip) {
        let image_width = (self.image_buffer.width() as i64).min(i32::MAX as i64);
        let image_height = (self.image_buffer.height() as i64).min(i32::MAX as i64);

        let x_start = (dx as i64).max(clip.x as i64).max(0);
        let y_start = (dy as i64).max(clip.y as i64).max(0);
        let x_end = (dx as i64 + w as i64).min(clip.x as i64 + clip.width as i64).min(image_width);
        let y_end = (dy as i64 + h as i64).min(clip.y as i64 + clip.height as i64).min(image_height);

        if x_start >= x_end || y_start >= y_end {
            return;
        }

        let mut pixels = Vec::with_capacity(((x_end - x_start) * (y_end - y_start)) as usize);
        for y_dst in y_start..y_end {
            for x_dst in x_start..x_end {
                let x_src = sx as i64 + x_dst - dx as i64;
                let y_src = sy as i64 + y_dst - dy as i64;

                if x_src < 0 || y_src < 0 || x_src >= image_width || y_src >= image_height {
                    continue;
                }

                pixels.push((x_dst as i32, y_dst as i32, self.image_buffer.get_pixel(x_src as i32, y_src as i32)));
            }
        }

        for (x, y, color) in pixels {
            self.image_buffer.put_pixel(x, y, color);
        }
    }

    fn draw(&mut self, dx: i32, dy: i32, w: u32, h: u32, src: &dyn Image, sx: i32, sy: i32, clip: Clip) {
        // Intersect all four constraints ONCE (src bounds, dst bounds, clip, the
        // requested w x h) instead of re-testing them per pixel. `x`/`y` below are
        // offsets into the requested region, exactly as in the old per-pixel loop.
        let x_lo = 0i64.max(-(sx as i64)).max(-(dx as i64)).max(clip.x as i64 - dx as i64);
        let y_lo = 0i64.max(-(sy as i64)).max(-(dy as i64)).max(clip.y as i64 - dy as i64);
        let x_hi = (w as i64)
            .min(src.width() as i64 - sx as i64)
            .min(self.image_buffer.width() as i64 - dx as i64)
            .min(clip.x as i64 + clip.width as i64 - dx as i64);
        let y_hi = (h as i64)
            .min(src.height() as i64 - sy as i64)
            .min(self.image_buffer.height() as i64 - dy as i64)
            .min(clip.y as i64 + clip.height as i64 - dy as i64);
        if x_lo >= x_hi || y_lo >= y_hi {
            return;
        }
        let (x_lo, x_hi, y_lo, y_hi) = (x_lo as i32, x_hi as i32, y_lo as i32, y_hi as i32);

        // ★ Same-format 16bpp blit = raw row copies. RGB565 carries no alpha (its
        // `to_color` always yields a = 255), so the per-pixel blend was a straight
        // overwrite spelled expensively: get_pixel + two colour conversions + an
        // f32 blend per pixel. This is the hot path of the game's world rendering —
        // every MC_grpDrawImage / MC_grpCopyFrameBuffer of the map tiles and the
        // per-frame offscreen -> screen present go through here.
        if !self.xor_mode && src.bytes_per_pixel() == 2 && self.image_buffer.bytes_per_pixel() == 2 {
            let raw = src.raw();
            let src_w = src.width() as usize;
            let row_px = (x_hi - x_lo) as usize;
            let mut ok = true;
            for y in y_lo..y_hi {
                let start = (((sy + y) as usize) * src_w + (sx + x_lo) as usize) * 2;
                ok &= self.image_buffer.put_row_raw(dx + x_lo, dy + y, &raw[start..start + row_px * 2]);
            }
            if ok {
                return;
            }
            // A buffer that cannot take raw rows falls through to the exact path.
        }

        for y in y_lo..y_hi {
            for x in x_lo..x_hi {
                let color = src.get_pixel(sx + x, sy + y);
                if self.xor_mode {
                    self.blend_pixel(dx + x, dy + y, color);
                } else if color.a == 255 {
                    // Blend factor 1.0 — the result IS the source pixel; skip the
                    // background read and the f32 mix.
                    self.image_buffer.put_pixel(dx + x, dy + y, color);
                } else if color.a != 0 {
                    self.blend_pixel(dx + x, dy + y, color);
                }
                // a == 0: blend factor 0 leaves the background exactly — skip.
            }
        }
    }

    fn draw_line(&mut self, x1: i32, y1: i32, x2: i32, y2: i32, color: Color) {
        if x1 == x2 && y1 == y2 {
            self.blend_pixel(x1 as _, y1 as _, color);
            return;
        }

        // bresenham's line drawing
        let dx = (x2 - x1).abs();
        let dy = (y2 - y1).abs();
        let sx = if x1 < x2 { 1 } else { -1 };
        let sy = if y1 < y2 { 1 } else { -1 };
        let mut err = dx - dy;

        let mut x = x1;
        let mut y = y1;

        while x != x2 || y != y2 {
            self.blend_pixel(x as _, y as _, color);

            let e2 = 2 * err;
            if e2 > -dy {
                err -= dy;
                x += sx;
            }
            if e2 < dx {
                err += dx;
                y += sy;
            }
        }
    }

    fn draw_text(&mut self, string: &str, x: i32, y: i32, text_alignment: TextAlignment, color: Color) {
        let size = 10.0; // TODO
        let font = FONT.as_scaled(FONT.pt_to_px_scale(size).unwrap());

        let total_width = string.chars().map(|c| font.h_advance(font.scaled_glyph(c).id)).sum::<f32>();
        let x = match text_alignment {
            TextAlignment::Left => x,
            TextAlignment::Center => x - (total_width / 2.0) as i32,
            TextAlignment::Right => x - total_width as i32,
        };

        let mut position = 0.0;
        for c in string.chars() {
            if c.is_control() {
                continue;
            }

            let glyph = font.scaled_glyph(c);
            let h_advance = font.h_advance(glyph.id);

            if let Some(outlined_glyph) = font.outline_glyph(glyph) {
                outlined_glyph.draw(|glyph_x: u32, glyph_y, c| {
                    let bounds = outlined_glyph.px_bounds();
                    self.blend_pixel(
                        x + (glyph_x as f32 + bounds.min.x + position) as i32,
                        y + (glyph_y as f32 + bounds.min.y + size) as i32,
                        Color {
                            a: (c * 255.0) as u8,
                            r: color.r,
                            g: color.g,
                            b: color.b,
                        },
                    )
                });
            }

            position += h_advance;
        }
    }

    fn draw_rect(&mut self, x: i32, y: i32, w: u32, h: u32, color: Color, clip: Clip) {
        // TODO use put_pixels
        for x in x..x + (w as i32) {
            if x < 0 || x >= self.image_buffer.width() as i32 {
                continue;
            }
            if x < clip.x || x >= clip.x + clip.width as i32 {
                continue;
            }
            if y < 0 || y >= self.image_buffer.height() as i32 {
                continue;
            }
            if y < clip.y || y >= clip.y + clip.height as i32 {
                continue;
            }

            self.put_pixel(x, y, color);
            self.put_pixel(x, y + (h as i32) - 1, color);
        }
        for y in y..y + (h as i32) {
            if x < 0 || x >= self.image_buffer.width() as i32 {
                continue;
            }
            if x < clip.x || x >= clip.x + clip.width as i32 {
                continue;
            }
            if y < 0 || y >= self.image_buffer.height() as i32 {
                continue;
            }
            if y < clip.y || y >= clip.y + clip.height as i32 {
                continue;
            }

            self.put_pixel(x, y, color);
            self.put_pixel(x + (w as i32) - 1, y, color);
        }
    }

    fn draw_arc(&mut self, x: i32, y: i32, w: u32, h: u32, start_angle: i32, arc_angle: i32, color: Color, clip: Clip) {
        if w == 0 || h == 0 {
            return;
        }

        let a = (w as f32 - 1.0) / 2.0;
        let b = (h as f32 - 1.0) / 2.0;
        let cx = x as f32 + a;
        let cy = y as f32 + b;

        self.stroke_arc(cx, cy, a, b, start_angle as f32, arc_angle as f32, color, &clip);
    }

    fn draw_round_rect(&mut self, x: i32, y: i32, w: u32, h: u32, arc_width: u32, arc_height: u32, color: Color, clip: Clip) {
        if w == 0 || h == 0 {
            return;
        }
        if arc_width == 0 || arc_height == 0 {
            return self.draw_rect(x, y, w, h, color, clip);
        }

        let rx = (arc_width.min(w) as f32 / 2.0).max(0.5);
        let ry = (arc_height.min(h) as f32 / 2.0).max(0.5);
        let rxi = rx.round() as i32;
        let ryi = ry.round() as i32;

        let left = x;
        let right = x + w as i32 - 1;
        let top = y;
        let bottom = y + h as i32 - 1;

        for px in (left + rxi)..=(right - rxi) {
            self.plot(px, top, color, &clip);
            self.plot(px, bottom, color, &clip);
        }
        for py in (top + ryi)..=(bottom - ryi) {
            self.plot(left, py, color, &clip);
            self.plot(right, py, color, &clip);
        }

        self.stroke_arc(right as f32 - rx, top as f32 + ry, rx, ry, 0.0, 90.0, color, &clip);
        self.stroke_arc(left as f32 + rx, top as f32 + ry, rx, ry, 90.0, 90.0, color, &clip);
        self.stroke_arc(left as f32 + rx, bottom as f32 - ry, rx, ry, 180.0, 90.0, color, &clip);
        self.stroke_arc(right as f32 - rx, bottom as f32 - ry, rx, ry, 270.0, 90.0, color, &clip);
    }

    fn fill_rect(&mut self, x: i32, y: i32, w: u32, h: u32, color: Color, clip: Clip) {
        // Clamp ONCE against the buffer and the clip, then fill row spans. The old
        // loop ran put_pixel (bounds check + colour conversion) for every pixel,
        // which made full-screen fills — map transitions, darkening overlays — one
        // of the most expensive single guest calls.
        let x0 = (x as i64).max(clip.x as i64).max(0).min(self.image_buffer.width() as i64) as i32;
        let y0 = (y as i64).max(clip.y as i64).max(0).min(self.image_buffer.height() as i64) as i32;
        let x1 = (x as i64 + w as i64)
            .min(clip.x as i64 + clip.width as i64)
            .min(self.image_buffer.width() as i64)
            .max(x0 as i64) as i32;
        let y1 = (y as i64 + h as i64)
            .min(clip.y as i64 + clip.height as i64)
            .min(self.image_buffer.height() as i64)
            .max(y0 as i64) as i32;
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        if self.xor_mode {
            for y in y0..y1 {
                for x in x0..x1 {
                    self.put_pixel(x, y, color);
                }
            }
        } else {
            self.image_buffer.fill_span(x0, y0, x1, y1, color);
        }
    }

    fn fill_arc(&mut self, x: i32, y: i32, w: u32, h: u32, start_angle: i32, arc_angle: i32, color: Color, clip: Clip) {
        if w == 0 || h == 0 {
            return;
        }

        let a = (w as f32 - 1.0) / 2.0;
        let b = (h as f32 - 1.0) / 2.0;
        let cx = x as f32 + a;
        let cy = y as f32 + b;
        let da = a.max(0.5);
        let db = b.max(0.5);
        let start = start_angle as f32;
        let sweep = arc_angle as f32;

        for py in y..y + h as i32 {
            for px in x..x + w as i32 {
                let nx = (px as f32 - cx) / da;
                let ny = (py as f32 - cy) / db;
                if nx * nx + ny * ny > 1.0 {
                    continue;
                }
                if !point_in_sweep(px as f32, py as f32, cx, cy, start, sweep) {
                    continue;
                }
                self.plot(px, py, color, &clip);
            }
        }
    }

    fn fill_round_rect(&mut self, x: i32, y: i32, w: u32, h: u32, arc_width: u32, arc_height: u32, color: Color, clip: Clip) {
        if w == 0 || h == 0 {
            return;
        }
        if arc_width == 0 || arc_height == 0 {
            return self.fill_rect(x, y, w, h, color, clip);
        }

        let rx = (arc_width.min(w) as f32 / 2.0).max(0.5);
        let ry = (arc_height.min(h) as f32 / 2.0).max(0.5);

        let left_center = x as f32 + rx;
        let right_center = (x + w as i32 - 1) as f32 - rx;
        let top_center = y as f32 + ry;
        let bottom_center = (y + h as i32 - 1) as f32 - ry;

        for py in y..y + h as i32 {
            let cy = if (py as f32) < top_center {
                top_center
            } else if (py as f32) > bottom_center {
                bottom_center
            } else {
                py as f32
            };
            for px in x..x + w as i32 {
                let cx = if (px as f32) < left_center {
                    left_center
                } else if (px as f32) > right_center {
                    right_center
                } else {
                    px as f32
                };

                let nx = (px as f32 - cx) / rx;
                let ny = (py as f32 - cy) / ry;
                if nx * nx + ny * ny <= 1.0 {
                    self.plot(px, py, color, &clip);
                }
            }
        }
    }

    fn put_pixel(&mut self, x: i32, y: i32, color: Color) {
        self.compose_pixel(x, y, color, false);
    }
}

pub struct Clip {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl Clip {
    pub fn intersect(&self, other: &Clip) -> Clip {
        let x = self.x.max(other.x);
        let y = self.y.max(other.y);
        let width = (self.x + (self.width as i32)).min(other.x + (other.width as i32)) - x;
        let height = (self.y + (self.height as i32)).min(other.y + (other.height as i32)) - y;

        Clip {
            x,
            y,
            width: width as _,
            height: height as _,
        }
    }
}

pub fn decode_image(data: &[u8]) -> Result<Box<dyn Image>> {
    extern crate std; // XXX

    use std::io::Cursor;

    if data[0] == b'L' && data[1] == b'B' && data[2] == b'M' && data[3] == b'P' {
        return decode_lbmp(data);
    }

    let image = ImageReader::new(Cursor::new(&data))
        .with_guessed_format()
        .map_err(|x| WieError::FatalError(x.to_string()))?
        .decode()
        .map_err(|x| WieError::FatalError(x.to_string()))?;
    let rgba = image.into_rgba8();

    let data = rgba.pixels().flat_map(|x| [x.0[2], x.0[1], x.0[0], x.0[3]]).collect::<Vec<_>>();

    Ok(Box::new(VecImageBuffer::<ArgbPixel>::from_raw(
        rgba.width(),
        rgba.height(),
        pod_collect_to_vec(&data),
    )) as Box<_>)
}

pub fn string_width(string: &str, pt_size: f32) -> f32 {
    let font = FONT.as_scaled(FONT.pt_to_px_scale(pt_size).unwrap());

    string.chars().map(|c| font.h_advance(font.scaled_glyph(c).id)).sum::<f32>()
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use wie_util::Result;

    use crate::canvas::{Clip, Image, ImageBufferCanvas};

    use super::{ArgbPixel, Canvas, Color, Rgb332Pixel, VecImageBuffer};

    #[test]
    fn test_canvas() -> Result<()> {
        let image_buffer = VecImageBuffer::<ArgbPixel>::new(10, 10);
        let mut canvas = ImageBufferCanvas::new(image_buffer);

        let clip = Clip {
            x: 0,
            y: 0,
            width: 10,
            height: 10,
        };
        canvas.fill_rect(0, 0, 10, 10, Color { r: 0, g: 0, b: 0, a: 255 }, clip);

        let image_buffer = canvas.into_inner();
        let raw = image_buffer.raw();

        assert_eq!(raw.len(), 10 * 10 * 4);
        for i in 0..10 * 10 {
            assert_eq!(raw[i * 4], 0);
            assert_eq!(raw[i * 4 + 1], 0);
            assert_eq!(raw[i * 4 + 2], 0);
            assert_eq!(raw[i * 4 + 3], 255);
        }

        Ok(())
    }

    fn full_clip(size: u32) -> Clip {
        Clip {
            x: 0,
            y: 0,
            width: size,
            height: size,
        }
    }

    const WHITE: Color = Color {
        r: 255,
        g: 255,
        b: 255,
        a: 255,
    };

    const BACKGROUND: Color = Color {
        a: 255,
        r: 0x12,
        g: 0x34,
        b: 0x56,
    };

    const XOR_COLOR: Color = Color {
        a: 255,
        r: 0xf0,
        g: 0x0f,
        b: 0xaa,
    };

    fn is_set(image: &impl Image, x: i32, y: i32) -> bool {
        let c = image.get_pixel(x, y);
        c.r != 0 || c.g != 0 || c.b != 0
    }

    fn assert_color(image: &dyn Image, x: i32, y: i32, expected: Color) {
        let actual = image.get_pixel(x, y);
        assert_eq!((actual.a, actual.r, actual.g, actual.b), (expected.a, expected.r, expected.g, expected.b));
    }

    #[test]
    fn test_xor_mode_fill_rect_toggles_pixels() {
        let mut canvas = ImageBufferCanvas::new(VecImageBuffer::<ArgbPixel>::new(4, 4));

        canvas.fill_rect(0, 0, 4, 4, BACKGROUND, full_clip(4));
        canvas.set_xor_mode(true);
        canvas.fill_rect(1, 1, 2, 2, XOR_COLOR, full_clip(4));

        assert_color(
            canvas.image(),
            1,
            1,
            Color {
                a: 255,
                r: BACKGROUND.r ^ XOR_COLOR.r,
                g: BACKGROUND.g ^ XOR_COLOR.g,
                b: BACKGROUND.b ^ XOR_COLOR.b,
            },
        );
        assert_color(canvas.image(), 0, 0, BACKGROUND);

        canvas.fill_rect(1, 1, 2, 2, XOR_COLOR, full_clip(4));

        assert_color(canvas.image(), 1, 1, BACKGROUND);
    }

    #[test]
    fn test_xor_mode_transparent_source_is_noop() {
        let mut canvas = ImageBufferCanvas::new(VecImageBuffer::<ArgbPixel>::new(1, 1));
        canvas.fill_rect(0, 0, 1, 1, BACKGROUND, full_clip(1));
        canvas.set_xor_mode(true);

        let src = VecImageBuffer::<ArgbPixel>::from_raw(1, 1, vec![0x00ff0000]);
        canvas.draw(0, 0, 1, 1, &src, 0, 0, full_clip(1));

        assert_color(canvas.image(), 0, 0, BACKGROUND);
    }

    #[test]
    fn test_xor_mode_rgb332_toggles_native_pixel() {
        let mut canvas = ImageBufferCanvas::new(VecImageBuffer::<Rgb332Pixel>::new(1, 1));
        let background = Color { a: 255, r: 0, g: 0, b: 0 };
        let source = Color { a: 255, r: 19, g: 0, b: 0 };

        canvas.fill_rect(0, 0, 1, 1, background, full_clip(1));
        canvas.set_xor_mode(true);
        canvas.fill_rect(0, 0, 1, 1, source, full_clip(1));
        canvas.fill_rect(0, 0, 1, 1, source, full_clip(1));

        assert_color(canvas.image(), 0, 0, background);
    }

    #[test]
    fn test_copy_area_uses_source_snapshot_for_overlap() {
        let mut canvas = ImageBufferCanvas::new(VecImageBuffer::<ArgbPixel>::new(4, 1));
        let red = Color { a: 255, r: 255, g: 0, b: 0 };
        let green = Color { a: 255, r: 0, g: 255, b: 0 };
        let blue = Color { a: 255, r: 0, g: 0, b: 255 };
        let black = Color { a: 255, r: 0, g: 0, b: 0 };

        canvas.put_pixel(0, 0, red);
        canvas.put_pixel(1, 0, green);
        canvas.put_pixel(2, 0, blue);
        canvas.put_pixel(3, 0, black);

        canvas.copy_area(1, 0, 0, 0, 3, 1, full_clip(4));

        assert_color(canvas.image(), 0, 0, red);
        assert_color(canvas.image(), 1, 0, red);
        assert_color(canvas.image(), 2, 0, green);
        assert_color(canvas.image(), 3, 0, blue);
    }

    #[test]
    fn test_copy_area_respects_clip() {
        let mut canvas = ImageBufferCanvas::new(VecImageBuffer::<ArgbPixel>::new(4, 1));
        let red = Color { a: 255, r: 255, g: 0, b: 0 };
        let green = Color { a: 255, r: 0, g: 255, b: 0 };
        let blue = Color { a: 255, r: 0, g: 0, b: 255 };
        let black = Color { a: 255, r: 0, g: 0, b: 0 };
        let clip = Clip {
            x: 2,
            y: 0,
            width: 1,
            height: 1,
        };

        canvas.put_pixel(0, 0, red);
        canvas.put_pixel(1, 0, green);
        canvas.put_pixel(2, 0, blue);
        canvas.put_pixel(3, 0, black);

        canvas.copy_area(1, 0, 0, 0, 3, 1, clip);

        assert_color(canvas.image(), 0, 0, red);
        assert_color(canvas.image(), 1, 0, green);
        assert_color(canvas.image(), 2, 0, green);
        assert_color(canvas.image(), 3, 0, black);
    }

    #[test]
    fn test_fill_arc_full_circle() {
        let mut canvas = ImageBufferCanvas::new(VecImageBuffer::<ArgbPixel>::new(32, 32));
        canvas.fill_arc(0, 0, 32, 32, 0, 360, WHITE, full_clip(32));
        let image = canvas.into_inner();

        assert!(is_set(&image, 16, 16), "center should be filled");
        assert!(!is_set(&image, 0, 0), "corner should be outside the ellipse");
        assert!(!is_set(&image, 31, 31), "corner should be outside the ellipse");
    }

    #[test]
    fn test_fill_arc_quadrant() {
        let mut canvas = ImageBufferCanvas::new(VecImageBuffer::<ArgbPixel>::new(32, 32));
        // 0..90 deg sweeps from 3 o'clock CCW to 12 o'clock => upper-right quadrant
        canvas.fill_arc(0, 0, 32, 32, 0, 90, WHITE, full_clip(32));
        let image = canvas.into_inner();

        assert!(is_set(&image, 24, 8), "upper-right quadrant should be filled");
        assert!(!is_set(&image, 8, 8), "upper-left quadrant should be empty");
        assert!(!is_set(&image, 8, 24), "lower-left quadrant should be empty");
        assert!(!is_set(&image, 24, 24), "lower-right quadrant should be empty");
    }

    #[test]
    fn test_fill_arc_negative_sweep_is_clockwise() {
        let mut canvas = ImageBufferCanvas::new(VecImageBuffer::<ArgbPixel>::new(32, 32));
        // 0..-90 deg sweeps CW from 3 o'clock to 6 o'clock => lower-right quadrant
        canvas.fill_arc(0, 0, 32, 32, 0, -90, WHITE, full_clip(32));
        let image = canvas.into_inner();

        assert!(is_set(&image, 24, 24), "lower-right quadrant should be filled");
        assert!(!is_set(&image, 24, 8), "upper-right quadrant should be empty");
    }

    #[test]
    fn test_draw_arc_endpoint() {
        let mut canvas = ImageBufferCanvas::new(VecImageBuffer::<ArgbPixel>::new(32, 32));
        canvas.draw_arc(0, 0, 32, 32, 0, 360, WHITE, full_clip(32));
        let image = canvas.into_inner();

        // bounding box 32x32 => center (15.5, 15.5), radius 15.5; nominal endpoints round to:
        assert!(is_set(&image, 31, 16), "3 o'clock (0 deg) point");
        assert!(is_set(&image, 16, 0), "12 o'clock (90 deg) point");
        assert!(is_set(&image, 0, 16), "9 o'clock (180 deg) point");
        assert!(is_set(&image, 16, 31), "6 o'clock (270 deg) point");
        assert!(!is_set(&image, 16, 16), "arc outline must not fill the interior");
    }

    #[test]
    fn test_draw_round_rect_outline() {
        let mut canvas = ImageBufferCanvas::new(VecImageBuffer::<ArgbPixel>::new(32, 32));
        canvas.draw_round_rect(0, 0, 32, 32, 16, 16, WHITE, full_clip(32));
        let image = canvas.into_inner();

        assert!(is_set(&image, 16, 0), "top straight edge");
        assert!(is_set(&image, 16, 31), "bottom straight edge");
        assert!(is_set(&image, 0, 16), "left straight edge");
        assert!(is_set(&image, 31, 16), "right straight edge");
        assert!(!is_set(&image, 0, 0), "rounded corner cut away");
        assert!(!is_set(&image, 16, 16), "outline must not fill the interior");
    }

    #[test]
    fn test_fill_arc_degenerate_width() {
        let mut canvas = ImageBufferCanvas::new(VecImageBuffer::<ArgbPixel>::new(8, 8));
        canvas.fill_arc(3, 0, 1, 8, 0, 360, WHITE, full_clip(8));
        let image = canvas.into_inner();

        for y in 0..8 {
            assert!(is_set(&image, 3, y), "degenerate 1px-wide column should be fully filled (y={y})");
        }
    }

    #[test]
    fn test_fill_round_rect_corners_empty() {
        let mut canvas = ImageBufferCanvas::new(VecImageBuffer::<ArgbPixel>::new(32, 32));
        canvas.fill_round_rect(0, 0, 32, 32, 16, 16, WHITE, full_clip(32));
        let image = canvas.into_inner();

        assert!(is_set(&image, 16, 16), "center should be filled");
        assert!(is_set(&image, 16, 0), "straight top edge should be filled");
        assert!(!is_set(&image, 0, 0), "rounded corner should be empty");
        assert!(!is_set(&image, 31, 0), "rounded corner should be empty");
    }

    /// The row-span fast path of `fill_rect` must be pixel-identical to the old
    /// per-pixel loop, including clip intersection and out-of-bounds clamping.
    #[test]
    fn test_fill_rect_fast_path_matches_per_pixel() {
        use super::Rgb565Pixel;

        let cases: &[(i32, i32, u32, u32, Clip)] = &[
            (2, 3, 5, 4, full_clip(16)),
            (-3, -2, 8, 6, full_clip(16)),
            (10, 12, 20, 20, full_clip(16)),
            (
                0,
                0,
                16,
                16,
                Clip {
                    x: 4,
                    y: 5,
                    width: 6,
                    height: 3,
                },
            ),
        ];
        for (x, y, w, h, clip) in cases {
            let mut fast = ImageBufferCanvas::new(VecImageBuffer::<Rgb565Pixel>::new(16, 16));
            fast.fill_rect(0, 0, 16, 16, BACKGROUND, full_clip(16));
            fast.fill_rect(
                *x,
                *y,
                *w,
                *h,
                WHITE,
                Clip {
                    x: clip.x,
                    y: clip.y,
                    width: clip.width,
                    height: clip.height,
                },
            );

            // Reference: the old shape — put_pixel per pixel with inline tests.
            let mut reference = ImageBufferCanvas::new(VecImageBuffer::<Rgb565Pixel>::new(16, 16));
            reference.fill_rect(0, 0, 16, 16, BACKGROUND, full_clip(16));
            for py in *y..*y + (*h as i32) {
                for px in *x..*x + (*w as i32) {
                    if px < 0 || py < 0 || px >= 16 || py >= 16 {
                        continue;
                    }
                    if px < clip.x || px >= clip.x + clip.width as i32 || py < clip.y || py >= clip.y + clip.height as i32 {
                        continue;
                    }
                    reference.put_pixel(px, py, WHITE);
                }
            }

            assert_eq!(
                fast.image().raw(),
                reference.image().raw(),
                "fill_rect({x}, {y}, {w}, {h}) diverged from the per-pixel reference"
            );
        }
    }

    /// The 16bpp raw-row-copy fast path of `draw` must place exactly the source
    /// pixels an opaque per-pixel blend would have placed, and touch nothing else —
    /// including with negative offsets and a clip.
    #[test]
    fn test_draw_565_row_copy_matches_per_pixel() {
        use super::{PixelType, Rgb565Pixel};

        // Source: a deterministic gradient.
        let mut src_data = alloc::vec::Vec::new();
        for i in 0..8 * 8u32 {
            src_data.push((i * 2557 % 65536) as u16);
        }
        let src = VecImageBuffer::<Rgb565Pixel>::from_raw(8, 8, src_data);

        let cases: &[(i32, i32, i32, i32, Clip)] = &[
            (3, 4, 0, 0, full_clip(16)),
            (-2, -3, 0, 0, full_clip(16)),
            (14, 14, 0, 0, full_clip(16)),
            (0, 0, -2, 3, full_clip(16)),
            (
                2,
                2,
                1,
                1,
                Clip {
                    x: 3,
                    y: 3,
                    width: 4,
                    height: 2,
                },
            ),
        ];
        for (dx, dy, sx, sy, clip) in cases {
            let mut fast = ImageBufferCanvas::new(VecImageBuffer::<Rgb565Pixel>::new(16, 16));
            fast.fill_rect(0, 0, 16, 16, BACKGROUND, full_clip(16));
            fast.draw(
                *dx,
                *dy,
                8,
                8,
                &src,
                *sx,
                *sy,
                Clip {
                    x: clip.x,
                    y: clip.y,
                    width: clip.width,
                    height: clip.height,
                },
            );

            let mut reference = ImageBufferCanvas::new(VecImageBuffer::<Rgb565Pixel>::new(16, 16));
            reference.fill_rect(0, 0, 16, 16, BACKGROUND, full_clip(16));
            for y in 0..8i32 {
                for x in 0..8i32 {
                    if sx + x < 0 || sy + y < 0 || sx + x >= 8 || sy + y >= 8 {
                        continue;
                    }
                    if dx + x < 0 || dy + y < 0 || dx + x >= 16 || dy + y >= 16 {
                        continue;
                    }
                    if dx + x < clip.x || dx + x >= clip.x + clip.width as i32 || dy + y < clip.y || dy + y >= clip.y + clip.height as i32 {
                        continue;
                    }
                    // The old path: blend of an opaque colour = plain overwrite.
                    reference.put_pixel(dx + x, dy + y, src.get_pixel(sx + x, sy + y));
                }
            }

            assert_eq!(
                fast.image().raw(),
                reference.image().raw(),
                "draw({dx}, {dy}, src {sx}, {sy}) diverged from the per-pixel reference"
            );
        }
        // The pixel round-trip the row copy skips must be the identity, or the two
        // paths could not be equal: every 565 value survives to_color -> from_color.
        for v in 0..=u16::MAX {
            assert_eq!(Rgb565Pixel::from_color(Rgb565Pixel::to_color(v)), v);
        }
    }

    /// Transparent (a == 0) source pixels leave the destination pixel alone, and
    /// opaque ones land exactly — the two blend fast paths.
    #[test]
    fn test_draw_alpha_fast_paths() {
        let mut canvas = ImageBufferCanvas::new(VecImageBuffer::<ArgbPixel>::new(2, 1));
        canvas.fill_rect(0, 0, 2, 1, BACKGROUND, full_clip(2));

        let src = VecImageBuffer::<ArgbPixel>::from_raw(2, 1, vec![0x00123456, 0xffabcdef]);
        canvas.draw(0, 0, 2, 1, &src, 0, 0, full_clip(2));

        assert_color(canvas.image(), 0, 0, BACKGROUND);
        assert_color(
            canvas.image(),
            1,
            0,
            Color {
                a: 255,
                r: 0xab,
                g: 0xcd,
                b: 0xef,
            },
        );
    }

    #[test]
    fn test_arc_respects_clip() {
        let mut canvas = ImageBufferCanvas::new(VecImageBuffer::<ArgbPixel>::new(32, 32));
        let clip = Clip {
            x: 0,
            y: 0,
            width: 16,
            height: 32,
        };
        canvas.fill_arc(0, 0, 32, 32, 0, 360, WHITE, clip);
        let image = canvas.into_inner();

        assert!(is_set(&image, 8, 16), "inside clip should be filled");
        assert!(!is_set(&image, 24, 16), "outside clip must not be filled");
    }
}

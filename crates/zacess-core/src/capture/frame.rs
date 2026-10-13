//! Grey frames of a capture, shrunk and with the ink separated from the board.
//!
//! A phone photo of a whiteboard changes from one moment to the next even when nothing was
//! written: the light moves, the hand shakes, the camera picks new exposure. So a capture is
//! reduced to what matters for change detection, the ink. The frame is shrunk, the board's own
//! brightness is estimated from the brightest pixels around each point and a pixel counts as ink
//! when it is clearly darker than that. Something large and dark in the way, such as the lecturer,
//! is marked as hidden rather than read as ink.

use super::CaptureError;
use image::{DynamicImage, ImageDecoder, ImageReader};
use std::path::Path;

/// A grey image, row by row, one byte per pixel.
#[derive(Debug, Clone, PartialEq)]
pub struct Frame {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<u8>,
    /// How many pixels of the original each pixel here stands for, along each side.
    pub scale: usize,
}

impl Frame {
    /// Reads a PNG or JPEG file as grey. A phone stores a photo the way the sensor saw it with a
    /// note of how to turn it, so the orientation note is applied; transparent pixels are laid on
    /// white rather than read as black.
    pub fn load(path: &Path) -> Result<Frame, CaptureError> {
        Ok(Frame::from_image(&Frame::load_turned(path)?.0))
    }

    /// The photo turned the way its orientation note says. The flag says if it had to be turned.
    pub fn load_turned(path: &Path) -> Result<(DynamicImage, bool), CaptureError> {
        let fail =
            |e: &dyn std::fmt::Display| CaptureError::Image(format!("{}: {e}", path.display()));
        let mut decoder = ImageReader::open(path)
            .map_err(|e| fail(&e))?
            .with_guessed_format()
            .map_err(|e| fail(&e))?
            .into_decoder()
            .map_err(|e| fail(&e))?;
        let orientation = decoder
            .orientation()
            .unwrap_or(image::metadata::Orientation::NoTransforms);
        let mut image = DynamicImage::from_decoder(decoder).map_err(|e| fail(&e))?;
        let turned = orientation != image::metadata::Orientation::NoTransforms;
        image.apply_orientation(orientation);
        Ok((image, turned))
    }

    /// Grey from any colour layout, with transparency laid on white.
    pub fn from_image(image: &DynamicImage) -> Frame {
        let rgba = image.to_rgba8();
        let (width, height) = (rgba.width() as usize, rgba.height() as usize);
        let pixels = rgba
            .pixels()
            .map(|p| {
                let [r, g, b, a] = p.0;
                let luma = 0.2126 * r as f32 + 0.7152 * g as f32 + 0.0722 * b as f32;
                let alpha = a as f32 / 255.0;
                (luma * alpha + 255.0 * (1.0 - alpha))
                    .round()
                    .clamp(0.0, 255.0) as u8
            })
            .collect();
        Frame {
            width,
            height,
            pixels,
            scale: 1,
        }
    }

    pub fn new(width: usize, height: usize, pixels: Vec<u8>) -> Frame {
        assert_eq!(
            pixels.len(),
            width * height,
            "a frame needs one pixel per cell"
        );
        Frame {
            width,
            height,
            pixels,
            scale: 1,
        }
    }

    pub fn at(&self, x: usize, y: usize) -> u8 {
        self.pixels[y * self.width + x]
    }

    /// Shrinks by a whole number so the longer side is at most `max_side`, averaging each cell.
    /// A frame already small enough is returned as it is.
    pub fn shrunk_to(&self, max_side: usize) -> Frame {
        let longer = self.width.max(self.height);
        let factor = longer.div_ceil(max_side.max(1)).max(1);
        if factor == 1 {
            return self.clone();
        }
        // a very narrow frame keeps at least one row or column, averaged over what is there
        let (width, height) = ((self.width / factor).max(1), (self.height / factor).max(1));
        let mut pixels = Vec::with_capacity(width * height);
        for cy in 0..height {
            for cx in 0..width {
                let mut sum = 0u32;
                let mut count = 0u32;
                for y in cy * factor..((cy + 1) * factor).min(self.height) {
                    for x in cx * factor..((cx + 1) * factor).min(self.width) {
                        sum += self.pixels[y * self.width + x] as u32;
                        count += 1;
                    }
                }
                pixels.push((sum / count.max(1)) as u8);
            }
        }
        Frame {
            width,
            height,
            pixels,
            scale: self.scale * factor,
        }
    }

    /// Running totals for averaging any rectangle in constant time.
    pub fn integral(&self) -> Vec<u64> {
        let stride = self.width + 1;
        let mut out = vec![0u64; stride * (self.height + 1)];
        for y in 0..self.height {
            let mut row = 0u64;
            for x in 0..self.width {
                row += self.pixels[y * self.width + x] as u64;
                out[(y + 1) * stride + x + 1] = out[y * stride + x + 1] + row;
            }
        }
        out
    }

    /// The mean brightness of a grid laid over a rectangle of the frame (left, top, right and
    /// bottom edges), row by row. `integral` comes from [`Frame::integral`]. Every cell holds at
    /// least one pixel, however small the rectangle.
    pub fn grid_means(
        &self,
        integral: &[u64],
        region: (usize, usize, usize, usize),
        cols: usize,
        rows: usize,
    ) -> Vec<f32> {
        if self.width == 0 || self.height == 0 {
            return vec![0.0; cols * rows];
        }
        let stride = self.width + 1;
        let (left, top) = (
            region.0.min(self.width.saturating_sub(1)),
            region.1.min(self.height.saturating_sub(1)),
        );
        let (right, bottom) = (
            region.2.clamp(left + 1, self.width),
            region.3.clamp(top + 1, self.height),
        );
        let (width, height) = (right - left, bottom - top);
        let mut out = Vec::with_capacity(cols * rows);
        for r in 0..rows {
            let y0 = top + r * height / rows;
            let y1 = (top + (r + 1) * height / rows).max(y0 + 1).min(self.height);
            let y0 = y0.min(y1 - 1);
            for c in 0..cols {
                let x0 = left + c * width / cols;
                let x1 = (left + (c + 1) * width / cols).max(x0 + 1).min(self.width);
                let x0 = x0.min(x1 - 1);
                let sum = integral[y1 * stride + x1] + integral[y0 * stride + x0]
                    - integral[y0 * stride + x1]
                    - integral[y1 * stride + x0];
                out.push(sum as f32 / ((x1 - x0) * (y1 - y0)) as f32);
            }
        }
        out
    }

    /// How sharp the frame is: the variance of its Laplacian, which is high where edges are crisp
    /// and low where the focus is soft. Frames of the same scene can be ranked by it.
    pub fn sharpness(&self) -> f32 {
        if self.width < 3 || self.height < 3 {
            return 0.0;
        }
        let (mut sum, mut sum_sq, mut n) = (0f64, 0f64, 0f64);
        for y in 1..self.height - 1 {
            for x in 1..self.width - 1 {
                let c = self.pixels[y * self.width + x] as f64;
                let l = self.pixels[y * self.width + x - 1] as f64
                    + self.pixels[y * self.width + x + 1] as f64
                    + self.pixels[(y - 1) * self.width + x] as f64
                    + self.pixels[(y + 1) * self.width + x] as f64
                    - 4.0 * c;
                sum += l;
                sum_sq += l * l;
                n += 1.0;
            }
        }
        let mean = sum / n;
        ((sum_sq / n) - mean * mean).max(0.0) as f32
    }

    /// The spread of brightness across the frame, as a standard deviation in grey levels.
    pub fn contrast(&self) -> f32 {
        if self.pixels.is_empty() {
            return 0.0;
        }
        let n = self.pixels.len() as f64;
        let mean = self.pixels.iter().map(|&p| p as f64).sum::<f64>() / n;
        let var = self
            .pixels
            .iter()
            .map(|&p| (p as f64 - mean).powi(2))
            .sum::<f64>()
            / n;
        var.sqrt() as f32
    }

    /// Where the frame has structure, whatever its colours: pixels at an edge, after a light blur
    /// has taken the camera noise out. A light slide and a dark one both show their text, their
    /// diagrams. A wall with noise shows nothing. Lines that run down more than three fifths
    /// of the frame, such as the edge of the projection, are left out. The mask is `pool` times
    /// smaller than the frame in each direction, a cell being set when any pixel in it is, so
    /// the strokes of a line of text run together into a solid band.
    pub fn content_mask(&self, edge_floor: f32, pool: usize) -> InkMask {
        let pool = pool.max(1);
        let (w, h) = (self.width, self.height);
        if w < 4 || h < 4 {
            return InkMask::new(1, 1, vec![false]);
        }
        let integral = self.integral();
        let stride = w + 1;
        let blurred: Vec<f32> = (0..w * h)
            .map(|i| {
                let (x, y) = (i % w, i / w);
                let (x0, x1) = (x.saturating_sub(1), (x + 2).min(w));
                let (y0, y1) = (y.saturating_sub(1), (y + 2).min(h));
                let sum = integral[y1 * stride + x1] + integral[y0 * stride + x0]
                    - integral[y0 * stride + x1]
                    - integral[y1 * stride + x0];
                sum as f32 / ((x1 - x0) * (y1 - y0)) as f32
            })
            .collect();
        let (mw, mh) = ((w / pool).max(1), (h / pool).max(1));
        let mut bits = vec![false; mw * mh];
        for y in 1..h - 1 {
            for x in 1..w - 1 {
                let gx = (blurred[y * w + x + 1] - blurred[y * w + x - 1]).abs();
                let gy = (blurred[(y + 1) * w + x] - blurred[(y - 1) * w + x]).abs();
                if gx + gy > edge_floor && x / pool < mw && y / pool < mh {
                    bits[(y / pool) * mw + x / pool] = true;
                }
            }
        }
        // the edge of the projection and of the screen run down most of the frame and would
        // join every line of text into one band, so tall straight lines are cleared. A rule
        // across the frame is a band of its own and does no harm. Clearing is skipped when tall
        // columns cover over half the width, as on a photograph or a dense texture, where
        // clearing them would leave nothing to compare
        if mh >= 10 {
            let mut column = vec![0usize; mw];
            for y in 0..mh {
                for x in 0..mw {
                    if bits[y * mw + x] {
                        column[x] += 1;
                    }
                }
            }
            let tall: Vec<usize> = (0..mw).filter(|&x| column[x] * 10 > mh * 6).collect();
            if tall.len() * 2 <= mw {
                for x in tall {
                    for y in 0..mh {
                        bits[y * mw + x] = false;
                    }
                }
            }
        }
        InkMask::new(mw, mh, bits)
    }

    /// The board behind the ink: the brightest value within `radius` of each pixel. A shadow or
    /// a lighting gradient changes slowly, so it survives into the background and can be divided
    /// out, while ink strokes are thin and do not.
    pub fn background(&self, radius: usize) -> Frame {
        let rows = max_filter(&self.pixels, self.width, self.height, radius, true);
        let pixels = max_filter(&rows, self.width, self.height, radius, false);
        Frame {
            width: self.width,
            height: self.height,
            pixels,
            scale: self.scale,
        }
    }
}

/// A running maximum along rows or columns, which is enough for a square window when applied
/// both ways.
fn max_filter(
    pixels: &[u8],
    width: usize,
    height: usize,
    radius: usize,
    along_rows: bool,
) -> Vec<u8> {
    let mut out = vec![0u8; pixels.len()];
    let (outer, inner) = if along_rows {
        (height, width)
    } else {
        (width, height)
    };
    let index = |o: usize, i: usize| {
        if along_rows {
            o * width + i
        } else {
            i * width + o
        }
    };
    for o in 0..outer {
        for i in 0..inner {
            let lo = i.saturating_sub(radius);
            let hi = (i + radius + 1).min(inner);
            out[index(o, i)] = (lo..hi).map(|j| pixels[index(o, j)]).max().unwrap_or(0);
        }
    }
    out
}

/// Grows a set of flags by `radius` in every direction.
fn dilate(flags: &[bool], width: usize, height: usize, radius: usize) -> Vec<bool> {
    let bytes: Vec<u8> = flags.iter().map(|&f| if f { 255 } else { 0 }).collect();
    let rows = max_filter(&bytes, width, height, radius, true);
    max_filter(&rows, width, height, radius, false)
        .into_iter()
        .map(|b| b > 0)
        .collect()
}

/// Which pixels of a shrunk frame carry ink. Some are hidden behind something.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InkMask {
    pub width: usize,
    pub height: usize,
    pub scale: usize,
    pub bits: Vec<bool>,
    /// Pixels covered by something large and dark, such as a person in front of the board,
    /// which neither count as ink nor as board.
    pub hidden: Vec<bool>,
}

impl InkMask {
    /// A pixel is ink when it is darker than the board around it by at least `darker_by`, a
    /// fraction of the board's brightness there. The board itself is taken as the brightest tenth
    /// of the background, so it is found even when someone covers most of the frame. A region
    /// whose own background is below half the board's brightness, wider than the window, is
    /// something dark in front of the board and is hidden, with a margin so its edge does not
    /// read as a stroke. A board darker than a sixth of full brightness, such as a blackboard,
    /// is not handled by this rule.
    pub fn from_frame(frame: &Frame, radius: usize, darker_by: f32) -> InkMask {
        let background = frame.background(radius);
        let mut sorted = background.pixels.clone();
        sorted.sort_unstable();
        let board = sorted.get(sorted.len() * 9 / 10).copied().unwrap_or(0) as f32;
        let dark: Vec<bool> = background
            .pixels
            .iter()
            .map(|&b| (b as f32) < board * 0.5)
            .collect();
        let hidden = dilate(&dark, frame.width, frame.height, radius + 2);
        let bits = frame
            .pixels
            .iter()
            .zip(&background.pixels)
            .zip(&hidden)
            .map(|((&p, &b), &h)| {
                let b = b as f32;
                !h && b >= 40.0 && (p as f32) < b * (1.0 - darker_by)
            })
            .collect();
        InkMask {
            width: frame.width,
            height: frame.height,
            scale: frame.scale,
            bits,
            hidden,
        }
    }

    pub fn new(width: usize, height: usize, bits: Vec<bool>) -> InkMask {
        assert_eq!(bits.len(), width * height, "a mask needs one bit per cell");
        let hidden = vec![false; bits.len()];
        InkMask {
            width,
            height,
            scale: 1,
            bits,
            hidden,
        }
    }

    pub fn at(&self, x: i64, y: i64) -> bool {
        if x < 0 || y < 0 || x >= self.width as i64 || y >= self.height as i64 {
            return false;
        }
        self.bits[y as usize * self.width + x as usize]
    }

    /// How many pixels are ink.
    pub fn count(&self) -> usize {
        self.bits.iter().filter(|b| **b).count()
    }

    /// The share of the frame that is ink, from 0 to 1.
    pub fn coverage(&self) -> f32 {
        if self.bits.is_empty() {
            0.0
        } else {
            self.count() as f32 / self.bits.len() as f32
        }
    }

    /// The share of the frame hidden behind something, from 0 to 1.
    pub fn hidden_share(&self) -> f32 {
        if self.hidden.is_empty() {
            0.0
        } else {
            self.hidden.iter().filter(|h| **h).count() as f32 / self.hidden.len() as f32
        }
    }

    /// The mask with the given pixels taken out, for comparing only what both photos show.
    pub fn without(&self, taken_out: &[bool]) -> InkMask {
        let bits = self
            .bits
            .iter()
            .zip(taken_out)
            .map(|(&b, &t)| b && !t)
            .collect();
        InkMask {
            bits,
            ..self.clone()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A board lit from one side, with a dark square drawn on it.
    fn lit_board() -> Frame {
        let (w, h) = (80, 40);
        let mut pixels = Vec::with_capacity(w * h);
        for y in 0..h {
            for x in 0..w {
                let board = 250 - (x as u32 * 90 / w as u32) as u8;
                let ink = (20..30).contains(&x) && (10..20).contains(&y);
                pixels.push(if ink { board / 4 } else { board });
            }
        }
        Frame::new(w, h, pixels)
    }

    #[test]
    fn shrinking_averages_whole_cells() {
        let frame = Frame::new(4, 2, vec![0, 255, 0, 255, 0, 255, 0, 255]);
        let small = frame.shrunk_to(2);
        assert_eq!((small.width, small.height, small.scale), (2, 1, 2));
        assert_eq!(small.pixels, vec![127, 127]);
        assert_eq!(frame.shrunk_to(10), frame);
        let narrow = Frame::new(9, 2, vec![100; 18]).shrunk_to(4);
        assert_eq!((narrow.width, narrow.height), (3, 1));
        assert_eq!(narrow.pixels, vec![100, 100, 100]);
    }

    #[test]
    fn ink_is_found_under_a_lighting_gradient() {
        let frame = lit_board();
        let mask = InkMask::from_frame(&frame, 6, 0.2);
        assert_eq!(
            mask.count(),
            100,
            "only the square is ink, not the dark side of the board"
        );
        assert!(mask.at(25, 15) && !mask.at(70, 15) && !mask.at(5, 5));
        assert!((mask.coverage() - 100.0 / 3200.0).abs() < 1e-6);
        assert_eq!(mask.hidden_share(), 0.0);
    }

    #[test]
    fn a_person_in_front_of_the_board_is_hidden_not_ink() {
        let mut frame = lit_board();
        // a dark figure 30 pixels wide over the right half, far wider than the window
        for y in 0..frame.height {
            for x in 45..75 {
                frame.pixels[y * frame.width + x] = 30;
            }
        }
        let mask = InkMask::from_frame(&frame, 6, 0.2);
        assert_eq!(mask.count(), 100, "the figure and its edges are not ink");
        assert!(
            mask.hidden[15 * 80 + 60] && mask.hidden[15 * 80 + 44],
            "the figure and a margin are hidden"
        );
        assert!(!mask.hidden[15 * 80 + 25]);
        assert!(
            (0.3..0.6).contains(&mask.hidden_share()),
            "{}",
            mask.hidden_share()
        );
        let bare = InkMask::from_frame(&lit_board(), 6, 0.2);
        assert_eq!(bare.without(&mask.hidden).count(), 100);
    }

    #[test]
    fn a_near_black_frame_has_no_ink() {
        let frame = Frame::new(10, 10, vec![10; 100]);
        assert_eq!(InkMask::from_frame(&frame, 3, 0.2).count(), 0);
    }

    /// A JPEG with an EXIF orientation note spliced in after its start marker.
    fn jpeg_with_orientation(image: &image::RgbImage, orientation: u16) -> Vec<u8> {
        let mut plain = std::io::Cursor::new(Vec::new());
        image
            .write_to(&mut plain, image::ImageFormat::Jpeg)
            .unwrap();
        let plain = plain.into_inner();
        let mut tiff = b"MM\x00\x2a\x00\x00\x00\x08\x00\x01".to_vec();
        tiff.extend_from_slice(&[0x01, 0x12, 0x00, 0x03, 0x00, 0x00, 0x00, 0x01]);
        tiff.extend_from_slice(&orientation.to_be_bytes());
        tiff.extend_from_slice(&[0x00, 0x00, 0x00, 0x00, 0x00, 0x00]);
        let mut exif = b"Exif\x00\x00".to_vec();
        exif.extend(tiff);
        let mut out = plain[..2].to_vec();
        out.extend_from_slice(&[0xff, 0xe1]);
        out.extend_from_slice(&((exif.len() + 2) as u16).to_be_bytes());
        out.extend(exif);
        out.extend_from_slice(&plain[2..]);
        out
    }

    #[test]
    fn a_photo_is_turned_the_way_the_phone_says() {
        // 64 wide and 32 tall, dark on the left half
        let mut image = image::RgbImage::from_pixel(64, 32, image::Rgb([235, 235, 235]));
        for y in 0..32 {
            for x in 0..32 {
                image.put_pixel(x, y, image::Rgb([20, 20, 20]));
            }
        }
        let dir = tempfile::tempdir().unwrap();
        let upright = dir.path().join("upright.jpg");
        std::fs::write(&upright, jpeg_with_orientation(&image, 1)).unwrap();
        let frame = Frame::load(&upright).unwrap();
        assert_eq!((frame.width, frame.height), (64, 32));
        assert!(
            frame.at(5, 16) < 80 && frame.at(58, 16) > 180,
            "dark on the left"
        );

        // orientation 6: the sensor image must be turned a quarter clockwise, so the left half
        // ends up at the top of a frame that is now taller than wide
        let turned = dir.path().join("turned.jpg");
        std::fs::write(&turned, jpeg_with_orientation(&image, 6)).unwrap();
        let frame = Frame::load(&turned).unwrap();
        assert_eq!(
            (frame.width, frame.height),
            (32, 64),
            "the orientation note was applied"
        );
        assert!(
            frame.at(16, 5) < 80 && frame.at(16, 58) > 180,
            "dark at the top"
        );
    }

    #[test]
    fn a_dense_frame_keeps_its_content_and_a_lone_tall_line_is_cleared() {
        // a lone vertical edge down a quiet frame is the edge of the projection
        let mut line = vec![200u8; 200 * 100];
        for y in 0..100 {
            for x in 100..104 {
                line[y * 200 + x] = 20;
            }
        }
        let mask = Frame::new(200, 100, line).content_mask(10.0, 2);
        assert_eq!(mask.count(), 0, "the edge was cleared");
        // stripes down the whole frame are all tall lines so clearing them would leave nothing
        let mut stripes = vec![200u8; 200 * 100];
        for y in 0..100 {
            for x in 0..200 {
                if (x / 4) % 2 == 0 {
                    stripes[y * 200 + x] = 20;
                }
            }
        }
        let mask = Frame::new(200, 100, stripes).content_mask(10.0, 2);
        assert!(mask.count() > 500, "{}", mask.count());
        assert_eq!(
            Frame::new(1, 1, vec![9])
                .grid_means(&[0, 0, 0, 9], (0, 0, 1, 1), 2, 2)
                .len(),
            4
        );
    }

    #[test]
    fn transparent_pixels_read_as_white() {
        let mut image = image::RgbaImage::new(4, 1);
        image.put_pixel(0, 0, image::Rgba([0, 0, 0, 0]));
        image.put_pixel(1, 0, image::Rgba([0, 0, 0, 255]));
        image.put_pixel(2, 0, image::Rgba([255, 255, 255, 255]));
        image.put_pixel(3, 0, image::Rgba([0, 0, 0, 128]));
        let frame = Frame::from_image(&DynamicImage::ImageRgba8(image));
        assert_eq!(frame.pixels, vec![255, 0, 255, 127]);
    }
}

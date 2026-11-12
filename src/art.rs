//! 8-bit pixel art for the app: the logo and the button icons, as small bitmaps that the TUI
//! scales by whole numbers and shows as real images on terminals that can draw them.
use image::{Rgba, RgbaImage};

type Rgb = [u8; 3];

fn glyph(letter: char) -> [&'static str; 7] {
    match letter {
        '3' => [
            "####.", "....#", "....#", ".###.", "....#", "....#", "####.",
        ],
        'Y' => [
            "#...#", "#...#", ".#.#.", "..#..", "..#..", "..#..", "..#..",
        ],
        'I' => ["###", ".#.", ".#.", ".#.", ".#.", ".#.", "###"],
        'D' => [
            "####.", "#...#", "#...#", "#...#", "#...#", "#...#", "####.",
        ],
        'S' => [
            ".####", "#....", "#....", ".###.", "....#", "....#", "####.",
        ],
        'G' => [
            ".###.", "#...#", "#....", "#.###", "#...#", "#...#", ".###.",
        ],
        'C' => [
            ".###.", "#...#", "#....", "#....", "#....", "#...#", ".###.",
        ],
        'P' => [
            "####.", "#...#", "#...#", "####.", "#....", "#....", "#....",
        ],
        'U' => [
            "#...#", "#...#", "#...#", "#...#", "#...#", "#...#", ".###.",
        ],
        'O' => [
            ".###.", "#...#", "#...#", "#...#", "#...#", "#...#", ".###.",
        ],
        'N' => [
            "#...#", "##..#", "##..#", "#.#.#", "#..##", "#..##", "#...#",
        ],
        'T' => [
            "#####", "..#..", "..#..", "..#..", "..#..", "..#..", "..#..",
        ],
        'R' => [
            "####.", "#...#", "#...#", "####.", "#.#..", "#..#.", "#...#",
        ],
        'B' => [
            "####.", "#...#", "#...#", "####.", "#...#", "#...#", "####.",
        ],
        'E' => [
            "#####", "#....", "#....", "####.", "#....", "#....", "#####",
        ],
        _ => [
            "#...#", "#...#", "#...#", "#####", "#...#", "#...#", "#...#",
        ],
    }
}

fn shade(color: Rgb, factor: f32) -> Rgba<u8> {
    let c = |v: u8| (v as f32 * factor).min(255.0) as u8;
    Rgba([c(color[0]), c(color[1]), c(color[2]), 255])
}

/// Flat pixels; `depth` adds a dark drop shadow that many pixels down and right.
fn finish(face: &[Vec<Option<Rgb>>], depth: u32) -> RgbaImage {
    let (h, w) = (face.len() as u32, face[0].len() as u32);
    let at = |x: u32, y: u32| face[y as usize][x as usize];
    let mut image = RgbaImage::new(w, h);
    for y in 0..h {
        for x in 0..w {
            if let Some(color) = at(x, y) {
                image.put_pixel(x, y, shade(color, 1.0));
                for d in 1..=depth {
                    if x + d < w && y + d < h && at(x + d, y + d).is_none() {
                        image.put_pixel(x + d, y + d, shade(color, 0.35));
                    }
                }
            }
        }
    }
    image
}

/// "3YIDDISH": the 3 in `record`, the letters in `accent`.
pub fn logo(accent: Rgb, record: Rgb) -> RgbaImage {
    let word = "3YIDDISH";
    let width: usize = word.chars().map(|c| glyph(c)[0].len() + 1).sum::<usize>() - 1;
    let mut face = vec![vec![None; width * 2]; 14];
    let mut x = 0;
    for letter in word.chars() {
        let rows = glyph(letter);
        let color = if letter == '3' { record } else { accent };
        for (r, row) in rows.iter().enumerate() {
            for (c, v) in row.chars().enumerate() {
                if v == '#' {
                    for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                        face[r * 2 + dy][(x + c) * 2 + dx] = Some(color);
                    }
                }
            }
        }
        x += rows[0].len() + 1;
    }
    finish(&face, 0)
}

/// A 12x12 icon: plus, mic, stop, send, refresh, phone or close.
pub fn icon(name: &str, color: Rgb) -> RgbaImage {
    let rows: [&str; 12] = match name {
        "plus" => [
            "............",
            ".....##.....",
            ".....##.....",
            ".....##.....",
            ".....##.....",
            ".##########.",
            ".##########.",
            ".....##.....",
            ".....##.....",
            ".....##.....",
            ".....##.....",
            "............",
        ],
        "mic" => [
            "....####....",
            "...######...",
            "...######...",
            "...######...",
            "...######...",
            ".#.######.#.",
            ".#..####..#.",
            ".##......##.",
            "..########..",
            ".....##.....",
            ".....##.....",
            "...######...",
        ],
        "stop" => [
            "............",
            ".##########.",
            ".##########.",
            ".##########.",
            ".##########.",
            ".##########.",
            ".##########.",
            ".##########.",
            ".##########.",
            ".##########.",
            ".##########.",
            "............",
        ],
        "send" => [
            "............",
            "##..........",
            "####........",
            "######......",
            "########....",
            "###########.",
            "###########.",
            "########....",
            "######......",
            "####........",
            "##..........",
            "............",
        ],
        "pause" => [
            "............",
            "..###..###..",
            "..###..###..",
            "..###..###..",
            "..###..###..",
            "..###..###..",
            "..###..###..",
            "..###..###..",
            "..###..###..",
            "..###..###..",
            "..###..###..",
            "............",
        ],
        "play" => [
            "............",
            "...##.......",
            "...####.....",
            "...######...",
            "...########.",
            "...#########",
            "...#########",
            "...########.",
            "...######...",
            "...####.....",
            "...##.......",
            "............",
        ],
        "phone" => [
            "...######...",
            "..########..",
            "..##....##..",
            "..##....##..",
            "..##....##..",
            "..##....##..",
            "..##....##..",
            "..##....##..",
            "..########..",
            "..###..###..",
            "..########..",
            "...######...",
        ],
        // The playground (the phone page's check mark).
        "check" => [
            "............",
            "............",
            "..........##",
            ".........###",
            "........###.",
            ".......###..",
            "##....###...",
            "###..###....",
            ".######.....",
            "..####......",
            "...##.......",
            "............",
        ],
        "refresh" => [
            "....####.#..",
            "..##....###.",
            ".#.....####.",
            ".#..........",
            "#...........",
            "#..........#",
            "#..........#",
            "#..........#",
            ".#........#.",
            ".#........#.",
            "..##....##..",
            "....####....",
        ],
        _ => [
            "##........##",
            "###......###",
            ".###....###.",
            "..###..###..",
            "...######...",
            "....####....",
            "....####....",
            "...######...",
            "..###..###..",
            ".###....###.",
            "###......###",
            "##........##",
        ],
    };
    let face: Vec<Vec<Option<Rgb>>> = rows
        .iter()
        .map(|row| row.chars().map(|v| (v == '#').then_some(color)).collect())
        .collect();
    finish(&face, 0)
}

/// An icon inside a thin rounded box of `size` pixels (16 or more): the button look.
pub fn tile(name: &str, color: Rgb, size: u32) -> RgbaImage {
    let icon = icon(name, color);
    let mut image = RgbaImage::new(size, size);
    let edge = shade(color, 0.55);
    for i in 1..size - 1 {
        for (x, y) in [(i, 0), (i, size - 1), (0, i), (size - 1, i)] {
            image.put_pixel(x, y, edge);
        }
    }
    let inset = ((size - 12) / 2) as i64;
    image::imageops::overlay(&mut image, &icon, inset, inset);
    image
}

/// One end of a message bubble, the size of a screen cell: a filled cell whose outer corners
/// are rounded (smoothly, by sampling each pixel 4x4) at the top and/or bottom.
pub fn cap(width: u32, height: u32, color: Rgb, left: bool, top: bool, bottom: bool) -> RgbaImage {
    let radius = (width as f32).min(height as f32 / 2.0);
    let mut image = RgbaImage::new(width, height);
    for y in 0..height {
        for x in 0..width {
            let mut inside = 0;
            for sy in 0..4 {
                for sx in 0..4 {
                    // Mirror so the rounded side is always at x = 0.
                    let px = (x as f32 + (sx as f32 + 0.5) / 4.0).min(width as f32);
                    let px = if left { px } else { width as f32 - px };
                    let py = y as f32 + (sy as f32 + 0.5) / 4.0;
                    let corner = if top && py < radius {
                        Some(radius)
                    } else if bottom && py > height as f32 - radius {
                        Some(height as f32 - radius)
                    } else {
                        None
                    };
                    let out = corner
                        .is_some_and(|cy| px < radius && (px - radius).hypot(py - cy) > radius);
                    inside += u32::from(!out);
                }
            }
            image.put_pixel(
                x,
                y,
                Rgba([color[0], color[1], color[2], (inside * 255 / 16) as u8]),
            );
        }
    }
    image
}

/// A corner of a fat bubble: a cell filled only in the half that touches the bubble (the lower
/// half for a top corner), with the outer corner of that half rounded.
pub fn half_cap(width: u32, height: u32, color: Rgb, left: bool, top: bool) -> RgbaImage {
    let half = height as f32 / 2.0;
    let radius = (width as f32).min(half);
    let mut image = RgbaImage::new(width, height);
    for y in 0..height {
        for x in 0..width {
            let mut inside = 0;
            for sy in 0..4 {
                for sx in 0..4 {
                    let px = x as f32 + (sx as f32 + 0.5) / 4.0;
                    let px = if left { px } else { width as f32 - px };
                    let py = y as f32 + (sy as f32 + 0.5) / 4.0;
                    // Distance into the filled half, measured from its outer edge.
                    let depth = if top { py - half } else { half - py };
                    let filled = depth >= 0.0
                        && !(px < radius
                            && depth < radius
                            && (px - radius).hypot(depth - radius) > radius);
                    inside += u32::from(filled);
                }
            }
            image.put_pixel(
                x,
                y,
                Rgba([color[0], color[1], color[2], (inside * 255 / 16) as u8]),
            );
        }
    }
    image
}

/// A filled rectangle with smoothly rounded corners (each pixel sampled 4x4) on a clear canvas.
fn plate(
    (width, height): (u32, u32),
    (left, top): (f32, f32),
    (wide, tall): (f32, f32),
    radius: f32,
    fill: Rgb,
) -> RgbaImage {
    let mut image = RgbaImage::new(width, height);
    for y in 0..height {
        for x in 0..width {
            let mut inside = 0;
            for sy in 0..4 {
                for sx in 0..4 {
                    let px = x as f32 + (sx as f32 + 0.5) / 4.0 - left;
                    let py = y as f32 + (sy as f32 + 0.5) / 4.0 - top;
                    // Fold onto one corner.
                    let (fx, fy) = (px.min(wide - px), py.min(tall - py));
                    let filled = fx >= 0.0
                        && fy >= 0.0
                        && !(fx < radius
                            && fy < radius
                            && (fx - radius).hypot(fy - radius) > radius);
                    inside += u32::from(filled);
                }
            }
            image.put_pixel(
                x,
                y,
                Rgba([fill[0], fill[1], fill[2], (inside * 255 / 16) as u8]),
            );
        }
    }
    image
}

/// A round button: a filled rounded square two thirds of the canvas tall, centered, with the
/// icon scaled by a whole number in its middle.
pub fn button(width: u32, height: u32, name: &str, color: Rgb, fill: Rgb) -> RgbaImage {
    let tile = (height * 2 / 3).min(width) as f32;
    let (left, top) = ((width as f32 - tile) / 2.0, (height as f32 - tile) / 2.0);
    let mut image = plate(
        (width, height),
        (left, top),
        (tile, tile),
        tile * 0.32,
        fill,
    );
    let icon = icon(name, color);
    let scale = ((tile * 0.62) as u32 / icon.width()).max(1);
    let scaled = image::imageops::resize(
        &icon,
        icon.width() * scale,
        icon.height() * scale,
        image::imageops::FilterType::Nearest,
    );
    image::imageops::overlay(
        &mut image,
        &scaled,
        ((width - scaled.width().min(width)) / 2) as i64,
        ((height - scaled.height().min(height)) / 2) as i64,
    );
    image
}

/// A chip's letters on a bar `height` pixels tall: their whole-number scale (about half the
/// plate's height) and the width of the word.
fn chip_word(height: u32, word: &str) -> (u32, u32) {
    let scale = (height / 3 / 7).max(1);
    let letters: u32 = word.chars().map(|l| glyph(l)[0].len() as u32 + 1).sum();
    (scale, letters.saturating_sub(1) * scale)
}

/// How wide the engine chip's plate is at the least: the word and a quarter of the plate's
/// height either side of it.
pub fn chip_width(height: u32, word: &str) -> u32 {
    chip_word(height, word).1 + height / 3
}

/// A word chip (the engine's "GPU" or "CPU", "CONTRIBUTE") in the logo's letters: a plate as tall and as round as
/// the buttons beside it, `inset` pixels in from both sides of the canvas.
pub fn chip(width: u32, height: u32, inset: u32, word: &str, color: Rgb, fill: Rgb) -> RgbaImage {
    let tile = (height * 2 / 3) as f32;
    let mut image = plate(
        (width, height),
        (inset as f32, (height as f32 - tile) / 2.0),
        (width.saturating_sub(2 * inset) as f32, tile),
        tile * 0.32,
        fill,
    );
    let (scale, wide) = chip_word(height, word);
    let mut x = width.saturating_sub(wide) / 2;
    let y = height.saturating_sub(7 * scale) / 2;
    for letter in word.chars() {
        for (r, row) in glyph(letter).iter().enumerate() {
            for (c, _) in row.chars().enumerate().filter(|(_, v)| *v == '#') {
                for dy in 0..scale {
                    for dx in 0..scale {
                        let (px, py) = (x + c as u32 * scale + dx, y + r as u32 * scale + dy);
                        if px < width && py < height {
                            image.put_pixel(px, py, shade(color, 1.0));
                        }
                    }
                }
            }
        }
        x += (glyph(letter)[0].len() as u32 + 1) * scale;
    }
    image
}

/// The background of a piece of a fat bubble: three rows tall, filled in the middle two
/// thirds (the text row plus half a row either side), clear above and below.
fn band(width: u32, height: u32, background: Rgb) -> RgbaImage {
    let mut image = RgbaImage::new(width, height);
    for y in height / 6..height - height / 6 {
        for x in 0..width {
            image.put_pixel(x, y, shade(background, 1.0));
        }
    }
    image
}

/// An icon inside a fat bubble, centered, scaled by a whole number.
pub fn icon_on(width: u32, height: u32, name: &str, color: Rgb, background: Rgb) -> RgbaImage {
    let mut image = band(width, height, background);
    let icon = icon(name, color);
    let scale = ((height * 2 / 5) / icon.height()).max(1);
    let scaled = image::imageops::resize(
        &icon,
        icon.width() * scale,
        icon.height() * scale,
        image::imageops::FilterType::Nearest,
    );
    image::imageops::overlay(
        &mut image,
        &scaled,
        (width.saturating_sub(scaled.width()) / 2) as i64,
        (height.saturating_sub(scaled.height()) / 2) as i64,
    );
    image
}

/// A voice note's wave inside a fat bubble: thin bars centered on the line, the first
/// `done` (0 to 1) of them in `played`, the rest in `rest`. With no peaks (an attached file)
/// it is a slider: a line with a round knob.
pub fn wave(
    width: u32,
    height: u32,
    peaks: &[u8],
    done: f32,
    (played, rest, background): (Rgb, Rgb, Rgb),
) -> RgbaImage {
    let mut image = band(width, height, background);
    let middle = height as f32 / 2.0;
    if peaks.is_empty() {
        let knob = (done.clamp(0.0, 1.0) * (width as f32 - 8.0) + 4.0) as i64;
        for x in 2..width.saturating_sub(2) {
            let color = if (x as i64) < knob { played } else { rest };
            for y in (middle as u32).saturating_sub(1)..=(middle as u32) {
                image.put_pixel(x, y, shade(color, 1.0));
            }
        }
        for y in 0..height {
            for x in 0..width {
                let d = ((x as i64 - knob) as f32).hypot(y as f32 + 0.5 - middle);
                if d <= 4.5 {
                    image.put_pixel(x, y, shade(played, 1.0));
                }
            }
        }
        return image;
    }
    // Bars 3 pixels wide, 2 apart.
    let count = (width / 5).max(1);
    for i in 0..count {
        let level = peaks[(i as usize * peaks.len() / count as usize).min(peaks.len() - 1)];
        let tall = (level as f32 / 255.0 * (height as f32 * 2.0 / 3.0 - 10.0)).max(3.0);
        let color = if (i as f32 + 0.5) / count as f32 <= done {
            played
        } else {
            rest
        };
        let (top, bottom) = ((middle - tall / 2.0) as u32, (middle + tall / 2.0) as u32);
        for y in top..bottom.min(height) {
            for dx in 0..3 {
                // Clip the corners so each bar has soft ends.
                let end = y == top || y + 1 == bottom;
                if end && dx != 1 && tall > 4.0 {
                    continue;
                }
                let x = i * 5 + 1 + dx;
                if x < width {
                    image.put_pixel(x, y, shade(color, 1.0));
                }
            }
        }
    }
    image
}

/// One end of a fat bar, one cell wide and three rows tall: filled in the middle two thirds
/// with both outer corners rounded, clear above and below.
pub fn bar_end(width: u32, height: u32, color: Rgb, left: bool) -> RgbaImage {
    let (top, bottom) = (height as f32 / 6.0, height as f32 * 5.0 / 6.0);
    let radius = (width as f32).min((bottom - top) / 2.0);
    let mut image = RgbaImage::new(width, height);
    for y in 0..height {
        for x in 0..width {
            let mut inside = 0;
            for sy in 0..4 {
                for sx in 0..4 {
                    let px = x as f32 + (sx as f32 + 0.5) / 4.0;
                    let px = if left { px } else { width as f32 - px };
                    let py = y as f32 + (sy as f32 + 0.5) / 4.0;
                    // Distance from the nearer horizontal edge of the bar.
                    let depth = (py - top).min(bottom - py);
                    let filled = depth >= 0.0
                        && !(px < radius
                            && depth < radius
                            && (px - radius).hypot(depth - radius) > radius);
                    inside += u32::from(filled);
                }
            }
            image.put_pixel(
                x,
                y,
                Rgba([color[0], color[1], color[2], (inside * 255 / 16) as u8]),
            );
        }
    }
    image
}

#[cfg(test)]
mod tests {
    #[test]
    fn art_has_pixels() {
        let logo = super::logo([0, 255, 255], [255, 0, 0]);
        assert_eq!(logo.dimensions(), (86, 14));
        assert!(logo.pixels().any(|p| p[3] > 0));
        for name in ["plus", "mic", "stop", "send", "refresh", "phone", "close"] {
            assert_eq!(super::icon(name, [255; 3]).dimensions(), (12, 12));
            assert_eq!(super::tile(name, [255; 3], 20).dimensions(), (20, 20));
        }
    }
    #[test]
    fn chip_word_is_centered_inside_its_plate() {
        // Cells of 10x22: a bar 66 tall, a plate 44 tall, letters three pixels to the dot.
        let width = super::chip_width(66, "GPU") + 6;
        assert_eq!(width, 79);
        let (ink, fill) = ([1, 2, 3], [9, 9, 9]);
        let chip = super::chip(80, 66, 3, "GPU", ink, fill);
        let inked: Vec<(u32, u32)> = chip
            .enumerate_pixels()
            .filter(|(_, _, p)| p.0 == [1, 2, 3, 255])
            .map(|(x, y, _)| (x, y))
            .collect();
        let xs = inked.iter().map(|p| p.0);
        let ys = inked.iter().map(|p| p.1);
        assert_eq!((xs.clone().min(), xs.max()), (Some(14), Some(64)));
        assert_eq!((ys.clone().min(), ys.max()), (Some(22), Some(42)));
        // The plate: clear above and beside, filled between.
        assert_eq!(chip.get_pixel(40, 5)[3], 0);
        assert_eq!(chip.get_pixel(1, 33)[3], 0);
        assert_eq!(chip.get_pixel(4, 33).0, [9, 9, 9, 255]);
        assert_ne!(super::chip(80, 66, 3, "CPU", ink, fill), chip);
    }
}

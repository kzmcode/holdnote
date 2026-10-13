//! Asset generator: `cargo run --bin gen_assets` (see Makefile `assets`).
//! Produces holdnote-icon-1024.png, the iconset PNGs and menu bar template icons.
//! `cargo run --bin gen_assets -- inspect` prints alpha info of the source icon.
use image::{imageops::FilterType, Rgba, RgbaImage};
use std::path::Path;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let src = Path::new("assets/brand/holdnote-icon.png");
    let img = image::open(src).expect("open source icon").to_rgba8();
    if args.get(1).map(|s| s.as_str()) == Some("inspect") {
        let (w, h) = img.dimensions();
        for (x, y) in [(0, 0), (w - 1, 0), (0, h - 1), (w - 1, h - 1), (w / 2, 0), (0, h / 2), (30, 30), (w / 2, h / 2)] {
            println!("({x},{y}) = {:?}", img.get_pixel(x, y));
        }
        let transparent = img.pixels().filter(|p| p[3] < 8).count();
        println!("{}x{} transparent px: {transparent}", w, h);
        return;
    }
    std::fs::create_dir_all("assets/menubar").unwrap();
    let icon = make_icon_1024(img);
    icon.save("assets/brand/holdnote-icon-1024.png").unwrap();
    let _ = std::fs::remove_dir_all("assets/AppIcon.iconset");
    std::fs::create_dir_all("assets/AppIcon.iconset").unwrap();
    for (name, px) in [
        ("icon_16x16", 16), ("icon_16x16@2x", 32), ("icon_32x32", 32), ("icon_32x32@2x", 64),
        ("icon_128x128", 128), ("icon_128x128@2x", 256), ("icon_256x256", 256),
        ("icon_256x256@2x", 512), ("icon_512x512", 512), ("icon_512x512@2x", 1024),
    ] {
        image::imageops::resize(&icon, px, px, FilterType::Lanczos3)
            .save(format!("assets/AppIcon.iconset/{name}.png")).unwrap();
    }
    menubar(18).save("assets/menubar/holdnote-template.png").unwrap();
    menubar(36).save("assets/menubar/holdnote-template@2x.png").unwrap();
    println!("assets generated");
}

/// Clear outer white, crop to the artwork bounding box and place it at 824x824 inside 1024x1024.
fn make_icon_1024(mut img: RgbaImage) -> RgbaImage {
    clear_outer_white(&mut img);
    let (w, h) = img.dimensions();
    let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0, 0);
    for (x, y, p) in img.enumerate_pixels() {
        if p[3] > 128 { x0 = x0.min(x); y0 = y0.min(y); x1 = x1.max(x); y1 = y1.max(y); }
    }
    eprintln!("artwork bbox: {x0},{y0} - {x1},{y1}");
    let crop = image::imageops::crop_imm(&img, x0, y0, x1 - x0 + 1, y1 - y0 + 1).to_image();
    let (cw, ch) = crop.dimensions();
    let k = 824.0 / cw.max(ch) as f32; // uniform scale, no distortion
    let (nw, nh) = ((cw as f32 * k).round() as u32, (ch as f32 * k).round() as u32);
    let art = image::imageops::resize(&crop, nw, nh, FilterType::Lanczos3);
    let mut out = RgbaImage::from_pixel(1024, 1024, Rgba([0, 0, 0, 0]));
    image::imageops::overlay(&mut out, &art, ((1024 - nw) / 2) as i64, ((1024 - nh) / 2) as i64);
    out
}

/// Flood-fill near-white pixels connected to the image border to alpha 0.
fn clear_outer_white(img: &mut RgbaImage) {
    let (w, h) = img.dimensions();
    let is_bg = |p: &Rgba<u8>| p[3] < 8 || (p[0] > 235 && p[1] > 235 && p[2] > 235);
    let mut seen = vec![false; (w * h) as usize];
    let mut stack = vec![];
    for x in 0..w { stack.push((x, 0)); stack.push((x, h - 1)); }
    for y in 0..h { stack.push((0, y)); stack.push((w - 1, y)); }
    while let Some((x, y)) = stack.pop() {
        let i = (y * w + x) as usize;
        if seen[i] || !is_bg(img.get_pixel(x, y)) { continue; }
        seen[i] = true;
        if x > 0 { stack.push((x - 1, y)); }
        if y > 0 { stack.push((x, y - 1)); }
        if x + 1 < w { stack.push((x + 1, y)); }
        if y + 1 < h { stack.push((x, y + 1)); }
    }
    let mut changed = 0u64;
    for y in 0..h { for x in 0..w {
        if seen[(y * w + x) as usize] { img.get_pixel_mut(x, y)[3] = 0; changed += 1; }
    } }
    // soften the fringe: light pixels adjacent to cleared area get half alpha
    let snap = img.clone();
    for y in 1..h - 1 { for x in 1..w - 1 {
        let p = snap.get_pixel(x, y);
        if p[3] == 0 { continue; }
        let near = [(x - 1, y), (x + 1, y), (x, y - 1), (x, y + 1)].iter().any(|&(a, b)| snap.get_pixel(a, b)[3] == 0);
        if near && p[0] > 200 && p[1] > 200 && p[2] > 200 { img.get_pixel_mut(x, y)[3] = p[3] / 2; }
    } }
    eprintln!("cleared {changed} outer background px");
}

// ---- menu bar template icon (black + alpha), supersampled ----
fn menubar(size: u32) -> RgbaImage {
    const SS: u32 = 8;
    let n = size * SS;
    let s = n as f32 / 18.0; // design units: 18x18
    let mut cov = vec![0f32; (n * n) as usize];
    let cx = 9.0 * s;
    for y in 0..n { for x in 0..n {
        let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
        let mut on = false;
        // mic capsule
        let (hw, top, bot) = (1.5 * s, 4.6 * s + 1.5 * s, 9.0 * s - 1.5 * s);
        let dx = (px - cx).abs();
        on |= if py < top { (dx * dx + (py - top).powi(2)).sqrt() <= hw }
            else if py > bot { (dx * dx + (py - bot).powi(2)).sqrt() <= hw }
            else { dx <= hw };
        // stand
        on |= dx <= 0.55 * s && py >= 11.0 * s && py <= 13.6 * s;
        on |= dx <= 2.2 * s && py >= 13.2 * s && py <= 14.3 * s;
        // cradle arc
        let (cax, cay) = (px - cx, py - 7.6 * s);
        let cr = (cax * cax + cay * cay).sqrt();
        on |= cay >= 0.0 && cr >= 3.1 * s && cr <= 4.1 * s;
        let (ax, ay) = (px - cx, py - 9.0 * s);
        let r = (ax * ax + ay * ay).sqrt();
        // outer ring with gap near upper-right dot
        let ang = ay.atan2(ax);
        let gap = (ang + 0.785).abs() < 0.42;
        on |= r >= 7.0 * s && r <= 8.1 * s && !gap;
        // dot
        let (ddx, ddy) = (px - 14.9 * s, py - 3.1 * s);
        on |= (ddx * ddx + ddy * ddy).sqrt() <= 1.45 * s;
        if on { cov[(y * n + x) as usize] = 1.0; }
    } }
    let mut out = RgbaImage::new(size, size);
    for y in 0..size { for x in 0..size {
        let mut sum = 0.0;
        for j in 0..SS { for i in 0..SS { sum += cov[((y * SS + j) * n + x * SS + i) as usize]; } }
        let a = (sum / (SS * SS) as f32 * 255.0).round() as u8;
        out.put_pixel(x, y, Rgba([0, 0, 0, a]));
    } }
    out
}

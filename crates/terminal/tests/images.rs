//! Kitty graphics through a [`Vt`]: the images a snapshot lists, where, in which layer, with
//! which pixels, and what happens to them as the screen scrolls, the program deletes them or
//! an animation plays (`docs/research/terminal-rendering.md` "Kitty graphics").
//!
//! `streams/kitty-icat.bin` is `kitten icat` 0.51.1 drawing `streams/kitty-images.py`'s test
//! images on an 80 x 30 screen of 17 x 37 px cells, in each layer, scaled and with unicode
//! placeholders, then a row of commands sent by hand. Recorded from `streams/`, with `kitten`
//! from kitty's release page:
//!
//! ```sh
//! echo "sleep 4" | python3 -I record.py kitty-icat.bin 80 30 -- \
//!     python3 -I kitty-images.py show /path/to/kitten
//! ```

use std::sync::Arc;

use terminal::{Config, ImageLayer, ImageRect, Placement, Snapshot, Vt};

fn stream(name: &str) -> Vec<u8> {
    let path = format!("{}/tests/streams/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}"))
}

/// An 80 x 30 terminal of 17 x 37 px cells, as the recording was laid out for.
fn vt() -> Vt {
    Vt::new(&Config {
        cols: 80,
        rows: 30,
        cell_width: 17,
        cell_height: 37,
        ..Config::default()
    })
}

fn icat() -> Snapshot {
    let mut vt = vt();
    vt.write(&stream("kitty-icat.bin"));
    vt.snapshot()
}

/// Where a placement draws: its layer, z, top-left cell, offset, size and source rectangle.
type Layout = (
    ImageLayer,
    i32,
    (i32, i32),
    (u32, u32),
    (u32, u32),
    ImageRect,
);

fn layout(p: &Placement) -> Layout {
    (
        p.layer,
        p.z,
        (p.col, p.row),
        (p.offset_x, p.offset_y),
        (p.width, p.height),
        p.source,
    )
}

/// RGBA at `(x, y)` of a placement's image.
fn pixel(p: &Placement, x: u32, y: u32) -> [u8; 4] {
    let i = ((y * p.image.width + x) * 4) as usize;
    p.image.rgba[i..i + 4].try_into().unwrap()
}

/// The placements of image `id`, top to bottom.
fn of(s: &Snapshot, id: u32) -> Vec<&Placement> {
    let mut found: Vec<&Placement> = s.images.iter().filter(|p| p.image.id == id).collect();
    found.sort_by_key(|p| p.row);
    found
}

/// The placement whose top-left cell is `(col, row)`.
fn at(s: &Snapshot, col: i32, row: i32) -> &Placement {
    s.images
        .iter()
        .find(|p| (p.col, p.row) == (col, row))
        .unwrap_or_else(|| panic!("no image at {col},{row}: {:?}", s.images))
}

fn rect(x: u32, y: u32, width: u32, height: u32) -> ImageRect {
    ImageRect {
        x,
        y,
        width,
        height,
    }
}

/// kitty's APC command, `keys` and a base64 payload.
fn apc(keys: &str, payload: &[u8]) -> Vec<u8> {
    const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut data = String::new();
    for chunk in payload.chunks(3) {
        let n = chunk.iter().fold(0u32, |n, &b| n << 8 | u32::from(b)) << (8 * (3 - chunk.len()));
        for i in 0..4 {
            data.push(if i <= chunk.len() {
                B64[(n >> (18 - 6 * i) & 63) as usize] as char
            } else {
                '='
            });
        }
    }
    let sep = if data.is_empty() { "" } else { ";" };
    format!("\x1b_G{keys}{sep}{data}\x1b\\").into_bytes()
}

/// Every placement icat and the hand-written row made, in its layer, at its cell and size: the
/// images icat scaled itself at their own size, placeholders one placement per row, the crop
/// stretched over its cells after its offset, and nothing of the deleted image.
#[test]
fn the_recording_lists_each_placement_where_it_draws() {
    let s = icat();
    let layers: Vec<(ImageLayer, i32)> = s.images.iter().map(|p| (p.layer, p.z)).collect();
    let mut sorted = layers.clone();
    sorted.sort();
    assert_eq!(layers, sorted, "drawing order: by layer, then z");

    let full = |w, h| rect(0, 0, w, h);
    // photo.png at its own size at the cursor, and as icat scaled it into 16 x 4 cells.
    assert_eq!(
        layout(at(&s, 0, 1)),
        (
            ImageLayer::AboveText,
            0,
            (0, 1),
            (0, 0),
            (136, 74),
            full(136, 74)
        )
    );
    assert_eq!(
        layout(at(&s, 10, 1)),
        (
            ImageLayer::AboveText,
            0,
            (10, 1),
            (0, 0),
            (272, 148),
            full(272, 148)
        )
    );
    // Centred in its 20 x 3 box by icat, under the text and over the backgrounds.
    assert_eq!(
        layout(at(&s, 32, 1)),
        (
            ImageLayer::BelowText,
            -1,
            (32, 1),
            (0, 0),
            (204, 111),
            full(204, 111)
        )
    );
    assert_eq!(
        layout(at(&s, 56, 1)),
        (
            ImageLayer::BelowBackground,
            -1_073_741_825,
            (56, 1),
            (0, 0),
            (204, 111),
            full(204, 111)
        )
    );
    assert_eq!(layout(at(&s, 12, 6)).4, (68, 37));
    assert_eq!(layout(at(&s, 46, 6)).4, (34, 37));

    // Placeholders: one placement per row of cells, each a row of the image, below the text.
    let small = of(&s, at(&s, 0, 10).image.id);
    assert_eq!(
        small.iter().map(|p| layout(p)).collect::<Vec<_>>(),
        vec![
            (
                ImageLayer::BelowText,
                -1,
                (0, 10),
                (0, 0),
                (136, 37),
                rect(0, 0, 136, 37)
            ),
            (
                ImageLayer::BelowText,
                -1,
                (0, 11),
                (0, 0),
                (136, 37),
                rect(0, 37, 136, 37)
            ),
        ]
    );
    let big = of(&s, at(&s, 20, 10).image.id);
    assert_eq!(big.len(), 4);
    for (i, p) in big.iter().enumerate() {
        let y = i as u32 * 37;
        assert_eq!(
            layout(p),
            (
                ImageLayer::BelowText,
                -1,
                (20, 10 + i as i32),
                (0, 0),
                (272, 37),
                rect(0, y, 272, 37)
            )
        );
    }

    // By hand: the green quadrant over 3 x 1 cells from 5, 6 px in; the zlib RGBA image at its
    // own size; nothing of image 42, deleted right after it was placed.
    assert_eq!(
        layout(of(&s, 40)[0]),
        (
            ImageLayer::AboveText,
            0,
            (1, 21),
            (5, 6),
            (51 - 5, 37 - 6),
            rect(16, 0, 16, 16)
        )
    );
    assert_eq!(
        layout(of(&s, 41)[0]),
        (
            ImageLayer::AboveText,
            0,
            (7, 21),
            (0, 0),
            (32, 32),
            full(32, 32)
        )
    );
    assert!(of(&s, 42).is_empty());
    assert_eq!(s.images.len(), 14);
}

/// Every format arrives as straight-alpha RGBA: PNG (decoded by our decoder), zlib RGB from
/// icat's scaling, raw RGB and zlib RGBA by hand.
#[test]
fn pixels_arrive_as_rgba() {
    let s = icat();
    let photo = at(&s, 0, 1);
    assert_eq!((photo.image.width, photo.image.height), (136, 74));
    assert_eq!(photo.image.rgba.len(), 136 * 74 * 4);
    assert_eq!(pixel(photo, 0, 0), [255, 255, 255, 255], "the frame");
    assert_eq!(
        pixel(photo, 1, 1),
        [253, 160, 1, 255],
        "the gradient's red end"
    );
    assert_eq!(pixel(photo, 134, 1), [1, 160, 253, 255], "its blue end");
    let glass = at(&s, 12, 6);
    assert_eq!(pixel(glass, 0, 0), [255, 0, 255, 255]);
    assert_eq!(
        pixel(glass, 40, 0),
        [255, 255, 255, 128],
        "straight alpha, not premultiplied"
    );
    let dots = at(&s, 46, 6);
    assert_eq!(
        pixel(dots, 0, 0),
        [255, 220, 0, 255],
        "RGB PNG gains an opaque alpha"
    );
    assert_eq!(pixel(dots, 1, 0), [0, 0, 0, 255]);
    let quads = of(&s, 40)[0];
    assert_eq!(pixel(quads, 0, 0), [255, 0, 0, 255], "raw RGB");
    assert_eq!(pixel(quads, 31, 31), [255, 255, 255, 255]);
    assert_eq!(pixel(of(&s, 41)[0], 20, 0), [0, 255, 0, 255], "zlib RGBA");
}

/// Images that don't change keep their list and their pixels' `Arc`s from one snapshot to the
/// next, so the view uploads nothing again.
#[test]
fn unchanged_images_are_shared_between_snapshots() {
    let mut vt = vt();
    vt.write(&stream("kitty-icat.bin"));
    let one = vt.snapshot();
    vt.write(b"echo hi");
    let two = vt.snapshot();
    assert!(Arc::ptr_eq(&one.images, &two.images));
    // A new image comes in: the list is new, the pixels of the others are the same.
    vt.write(&apc("a=T,f=24,s=1,v=1,i=7,q=2", &[1, 2, 3]));
    let three = vt.snapshot();
    assert!(!Arc::ptr_eq(&two.images, &three.images));
    let photo = |s: &Snapshot| at(s, 0, 1).image.rgba.clone();
    assert!(Arc::ptr_eq(&photo(&two), &photo(&three)));
    assert_eq!(three.images.len(), 15);
}

/// Images move with their cells: three lines of output scroll the photo off the top and leave
/// the scaled one starting two rows above the viewport, at its full size for the view to clip.
#[test]
fn images_scroll_with_the_text() {
    let mut vt = vt();
    vt.write(&stream("kitty-icat.bin"));
    vt.write(b"\r\n\r\n\r\n");
    let s = vt.snapshot();
    assert!(
        s.images.iter().all(|p| (p.col, p.row) != (0, -2)),
        "the photo scrolled away"
    );
    let scaled = at(&s, 10, -2);
    assert_eq!((scaled.width, scaled.height), (272, 148));
    assert_eq!(scaled.source, rect(0, 0, 272, 148));
    // Placeholders scroll as the text they are.
    assert_eq!(at(&s, 0, 7).layer, ImageLayer::BelowText);
    // And back through the scrollback.
    vt.scroll(terminal::Scroll::Top);
    let top = vt.snapshot();
    assert_eq!(at(&top, 0, 1).image.width, 136);
}

/// Deleting images takes them off the screen, and each screen keeps its own: the alternate
/// screen starts with none, and the primary's come back with it. As in kitty, deleting every
/// placement (`d=A`) leaves the virtual ones that placeholders show; deleting their images by
/// id takes those too.
#[test]
fn deleted_images_go_and_each_screen_has_its_own() {
    let mut vt = vt();
    vt.write(&stream("kitty-icat.bin"));
    vt.write(b"\x1b[?1049h");
    assert!(vt.snapshot().images.is_empty());
    vt.write(b"\x1b[?1049l");
    let s = vt.snapshot();
    assert_eq!(s.images.len(), 14);
    let placeholders = [at(&s, 0, 10).image.id, at(&s, 20, 10).image.id];
    vt.write(&apc("a=d,d=A,q=2", b""));
    let left = vt.snapshot();
    assert_eq!(left.images.len(), 6);
    assert!(
        left.images
            .iter()
            .all(|p| placeholders.contains(&p.image.id))
    );
    for id in placeholders {
        vt.write(&apc(&format!("a=d,d=I,i={id},q=2"), b""));
    }
    assert!(vt.snapshot().images.is_empty());
}

/// A render hold keeps the images of the frame it began with until it ends.
#[test]
fn a_render_hold_keeps_its_images() {
    let mut vt = vt();
    vt.write(&stream("kitty-icat.bin"));
    assert_eq!(vt.snapshot().images.len(), 14);
    vt.write(b"\x1b[?2026h");
    vt.write(&apc("a=d,d=A,q=2", b""));
    assert_eq!(vt.snapshot().images.len(), 14);
    vt.write(b"\x1b[?2026l");
    assert_eq!(
        vt.snapshot().images.len(),
        6,
        "the placeholders' images stay"
    );
}

/// With no cell size there is nothing to size an image by, so nothing is listed; a PNG that
/// doesn't decode is refused.
#[test]
fn no_cell_or_a_bad_png_draws_nothing() {
    let mut vt = Vt::new(&Config::default());
    vt.write(&stream("kitty-icat.bin"));
    assert!(vt.snapshot().images.is_empty());
    let mut vt = self::vt();
    vt.write(&apc("a=T,f=100,i=9", b"not a png"));
    assert!(vt.snapshot().images.is_empty());
    assert!(String::from_utf8_lossy(&vt.take_replies()).contains("i=9;E"));
}

/// An animation plays on its own clock: the snapshot after a frame is due shows that frame's
/// pixels, and the terminal says when the next one is due. libghostty-vt's own example: a red
/// frame for 30 ms, then a blue one for 40 ms, looping.
#[test]
fn animations_play() {
    let mut vt = vt();
    vt.write(&apc("a=T,f=32,i=2,s=1,v=1,q=2", &[255, 0, 0, 255]));
    vt.write(&apc("a=f,f=32,i=2,s=1,v=1,z=40,q=2", &[0, 0, 255, 255]));
    vt.write(&apc("a=a,i=2,r=1,z=30,q=2", b""));
    vt.write(&apc("a=a,i=2,s=3,q=2", b""));
    let first = vt.snapshot();
    let due = vt.animation_deadline().expect("an animation plays");
    assert_eq!(pixel(&first.images[0], 0, 0), [255, 0, 0, 255]);
    std::thread::sleep(due.saturating_duration_since(std::time::Instant::now()));
    let second = vt.snapshot();
    assert_eq!(pixel(&second.images[0], 0, 0), [0, 0, 255, 255]);
    assert_ne!(
        first.images[0].image.generation,
        second.images[0].image.generation
    );
    vt.write(&apc("a=a,i=2,s=1,q=2", b""));
    vt.snapshot();
    assert_eq!(vt.animation_deadline(), None, "stopped");
}

/// A `width` x `height` PNG of `color`, 8 bits a sample, from `data` (and `palette`, for an
/// indexed one).
fn png(width: u32, height: u32, color: png::ColorType, data: &[u8], palette: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, width, height);
    encoder.set_color(color);
    encoder.set_depth(png::BitDepth::Eight);
    if !palette.is_empty() {
        encoder.set_palette(palette.to_vec());
    }
    let mut writer = encoder.write_header().unwrap();
    writer.write_image_data(data).unwrap();
    writer.finish().unwrap();
    out
}

/// Every kind of PNG our decoder hands libghostty-vt arrives as RGBA: gray, gray with alpha
/// and a palette, besides the RGB and RGBA icat sent.
#[test]
fn every_kind_of_png_decodes() {
    use png::ColorType::{Grayscale, GrayscaleAlpha, Indexed};
    let mut vt = vt();
    let pngs = [
        (1, png(2, 1, Grayscale, &[0, 200], &[])),
        (2, png(1, 1, GrayscaleAlpha, &[90, 30], &[])),
        (3, png(2, 1, Indexed, &[1, 0], &[10, 20, 30, 40, 50, 60])),
    ];
    for (id, bytes) in &pngs {
        vt.write(&apc(&format!("a=T,f=100,i={id},C=1,q=2"), bytes));
    }
    let s = vt.snapshot();
    let first = |id| pixel(of(&s, id)[0], 0, 0);
    assert_eq!(first(1), [0, 0, 0, 255]);
    assert_eq!(pixel(of(&s, 1)[0], 1, 0), [200, 200, 200, 255]);
    assert_eq!(first(2), [90, 90, 90, 30]);
    assert_eq!(first(3), [40, 50, 60, 255]);
}

/// A PNG whose header asks for more pixels than libghostty-vt lets one image have is refused
/// before it's decoded.
#[test]
fn a_png_too_big_for_the_storage_is_refused() {
    // The header of a 30000 x 30000 RGBA image (3.6 GB decoded), with one row of data.
    let mut bytes = png(1, 1, png::ColorType::Rgba, &[0, 0, 0, 0], &[]);
    bytes[16..24].copy_from_slice(&[0, 0, 0x75, 0x30, 0, 0, 0x75, 0x30]);
    let crc = crc32(&bytes[12..29]);
    bytes[29..33].copy_from_slice(&crc.to_be_bytes());
    let mut vt = vt();
    vt.write(&apc("a=T,f=100,i=5", &bytes));
    assert!(vt.snapshot().images.is_empty());
    assert!(String::from_utf8_lossy(&vt.take_replies()).contains("i=5;E"));
}

/// The CRC-32 a PNG chunk ends with.
fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &b in bytes {
        crc ^= u32::from(b);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                crc >> 1 ^ 0xedb8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

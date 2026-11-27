//! Kitty graphics: the render state's image list with each image's pixels, animation playback,
//! and the PNG decoder libghostty-vt asks its embedder for (`docs/research/terminal-rendering.md`
//! "Kitty graphics", libghostty-vt's `guide-images`).
//!
//! The list is read again only when the render state's placements generation changes. Pixels are
//! cached by image generation, which changes whenever they do and is never reused, so an image
//! that stays on screen is read and converted once, and keeps its `Arc` from one snapshot to the
//! next. An image no longer listed (deleted, scrolled out, cleared with the screen) leaves the
//! cache with the next list.

use std::collections::HashMap;
use std::ffi::c_void;
use std::ptr;
use std::sync::{Arc, Once};
use std::time::{Duration, Instant};

use ghostty_vt_sys::*;

use super::ok;
use crate::snapshot::{Image, ImageLayer, ImageRect, Placement};

/// Ghostty's `image-storage-limit` default, in bytes, per screen.
pub const STORAGE_LIMIT: u64 = 320_000_000;

/// Animation frames come no closer together than this, as on a 120 Hz display.
const MIN_FRAME: Duration = Duration::from_millis(8);

/// A terminal's images, as its last snapshot listed them.
pub(super) struct Images {
    iter: GhosttyRenderStateImageIterator,
    /// The placements generation `list` was read at; `None` before the first read.
    read_at: Option<u64>,
    list: Arc<[Placement]>,
    /// The pixels of the images in `list`, by image generation.
    pixels: HashMap<u64, Image>,
    /// The animation clock's zero.
    epoch: Instant,
    /// When the next animation frame is due, while one plays.
    next_frame: Option<Instant>,
}

impl Images {
    pub(super) fn new() -> Images {
        let mut iter: GhosttyRenderStateImageIterator = ptr::null_mut();
        // SAFETY: the out pointer is valid; the iterator is freed in `free`.
        ok(
            unsafe { ghostty_render_state_image_iterator_new(ptr::null(), &mut iter) },
            "ghostty_render_state_image_iterator_new",
        );
        Images {
            iter,
            read_at: None,
            list: Arc::from([]),
            pixels: HashMap::new(),
            epoch: Instant::now(),
            next_frame: None,
        }
    }

    /// The images the render state's last update found, with their pixels.
    ///
    /// # Safety
    ///
    /// `state` and `term` are live, and `state` was updated from `term`.
    pub(super) unsafe fn list(
        &mut self,
        state: GhosttyRenderState,
        term: GhosttyTerminal,
    ) -> Arc<[Placement]> {
        let mut generation = 0u64;
        // SAFETY: the key's output type is `uint64_t`.
        let read = unsafe {
            ghostty_render_state_get(
                state,
                GHOSTTY_RENDER_STATE_DATA_IMAGE_PLACEMENTS_GENERATION,
                (&raw mut generation).cast(),
            )
        };
        if read != GHOSTTY_SUCCESS || self.read_at == Some(generation) {
            return self.list.clone();
        }
        self.read_at = Some(generation);
        // SAFETY: as the caller promised.
        self.list = unsafe { self.read(state, term) }.into();
        let listed: Vec<u64> = self.list.iter().map(|p| p.image.generation).collect();
        self.pixels.retain(|g, _| listed.contains(g));
        self.list.clone()
    }

    unsafe fn read(&mut self, state: GhosttyRenderState, term: GhosttyTerminal) -> Vec<Placement> {
        // SAFETY: the caller passes live handles; every out pointer has its key's type.
        unsafe {
            ok(
                ghostty_render_state_get(
                    state,
                    GHOSTTY_RENDER_STATE_DATA_IMAGE_ITERATOR,
                    (&raw mut self.iter).cast(),
                ),
                "RENDER_STATE_DATA_IMAGE_ITERATOR",
            );
            let mut graphics: GhosttyKittyGraphics = ptr::null_mut();
            let storage = ghostty_terminal_get(
                term,
                GHOSTTY_TERMINAL_DATA_KITTY_GRAPHICS,
                (&raw mut graphics).cast(),
            ) == GHOSTTY_SUCCESS;
            let mut list = Vec::new();
            while ghostty_render_state_image_iterator_next(self.iter) {
                let mut protocol: GhosttyImageProtocol = GHOSTTY_IMAGE_PROTOCOL_MAX_VALUE;
                ghostty_render_state_image_get(
                    self.iter,
                    GHOSTTY_RENDER_STATE_IMAGE_DATA_PROTOCOL,
                    (&raw mut protocol).cast(),
                );
                // Later versions may list other protocols: skip what isn't kitty's.
                if protocol != GHOSTTY_IMAGE_PROTOCOL_KITTY || !storage {
                    continue;
                }
                let mut id = 0u32;
                let mut image_generation = 0u64;
                let mut layer: GhosttyImageLayer = GHOSTTY_IMAGE_LAYER_ABOVE_TEXT;
                let (mut z, mut col, mut row) = (0i32, 0i32, 0i32);
                let (mut offset_x, mut offset_y, mut width, mut height) = (0u32, 0u32, 0u32, 0u32);
                let mut source = ImageRect::default();
                let keys = [
                    GHOSTTY_RENDER_STATE_IMAGE_DATA_IMAGE_ID,
                    GHOSTTY_RENDER_STATE_IMAGE_DATA_IMAGE_GENERATION,
                    GHOSTTY_RENDER_STATE_IMAGE_DATA_LAYER,
                    GHOSTTY_RENDER_STATE_IMAGE_DATA_Z,
                    GHOSTTY_RENDER_STATE_IMAGE_DATA_VIEWPORT_COL,
                    GHOSTTY_RENDER_STATE_IMAGE_DATA_VIEWPORT_ROW,
                    GHOSTTY_RENDER_STATE_IMAGE_DATA_CELL_OFFSET_X,
                    GHOSTTY_RENDER_STATE_IMAGE_DATA_CELL_OFFSET_Y,
                    GHOSTTY_RENDER_STATE_IMAGE_DATA_PIXEL_WIDTH,
                    GHOSTTY_RENDER_STATE_IMAGE_DATA_PIXEL_HEIGHT,
                    GHOSTTY_RENDER_STATE_IMAGE_DATA_SOURCE_X,
                    GHOSTTY_RENDER_STATE_IMAGE_DATA_SOURCE_Y,
                    GHOSTTY_RENDER_STATE_IMAGE_DATA_SOURCE_WIDTH,
                    GHOSTTY_RENDER_STATE_IMAGE_DATA_SOURCE_HEIGHT,
                ];
                let mut values: [*mut c_void; 14] = [
                    (&raw mut id).cast(),
                    (&raw mut image_generation).cast(),
                    (&raw mut layer).cast(),
                    (&raw mut z).cast(),
                    (&raw mut col).cast(),
                    (&raw mut row).cast(),
                    (&raw mut offset_x).cast(),
                    (&raw mut offset_y).cast(),
                    (&raw mut width).cast(),
                    (&raw mut height).cast(),
                    (&raw mut source.x).cast(),
                    (&raw mut source.y).cast(),
                    (&raw mut source.width).cast(),
                    (&raw mut source.height).cast(),
                ];
                ok(
                    ghostty_render_state_image_get_multi(
                        self.iter,
                        keys.len(),
                        keys.as_ptr(),
                        values.as_mut_ptr(),
                        ptr::null_mut(),
                    ),
                    "ghostty_render_state_image_get_multi",
                );
                if width == 0 || height == 0 || source.width == 0 || source.height == 0 {
                    continue;
                }
                let image = match self.pixels.get(&image_generation) {
                    Some(image) => image.clone(),
                    None => match pixels(graphics, id, image_generation) {
                        Some(image) => {
                            self.pixels.insert(image_generation, image.clone());
                            image
                        }
                        // During a render hold the storage may already hold newer pixels than
                        // the frame being shown, or none: that image waits for the next frame.
                        None => continue,
                    },
                };
                list.push(Placement {
                    image,
                    layer: match layer {
                        GHOSTTY_IMAGE_LAYER_BELOW_BACKGROUND => ImageLayer::BelowBackground,
                        GHOSTTY_IMAGE_LAYER_BELOW_TEXT => ImageLayer::BelowText,
                        _ => ImageLayer::AboveText,
                    },
                    z,
                    col,
                    row,
                    offset_x,
                    offset_y,
                    width,
                    height,
                    source,
                });
            }
            // The iterator's order isn't the drawing order (render.h).
            list.sort_by_key(|p| (p.layer, p.z));
            list
        }
    }

    /// Moves playing animations to the frame due `now`, as libghostty-vt asks once per frame
    /// drawn, and notes when the next one is due.
    ///
    /// # Safety
    ///
    /// `term` is live. The call changes the image storage, like a write.
    pub(super) unsafe fn tick(&mut self, term: GhosttyTerminal, now: Instant) {
        // SAFETY: as the caller promised; every out pointer has its key's type.
        unsafe {
            let mut graphics: GhosttyKittyGraphics = ptr::null_mut();
            if ghostty_terminal_get(
                term,
                GHOSTTY_TERMINAL_DATA_KITTY_GRAPHICS,
                (&raw mut graphics).cast(),
            ) != GHOSTTY_SUCCESS
            {
                self.next_frame = None;
                return;
            }
            // Generation 0: nothing was ever stored, so nothing plays.
            let mut stored = 0u64;
            ghostty_kitty_graphics_get(
                graphics,
                GHOSTTY_KITTY_GRAPHICS_DATA_GENERATION,
                (&raw mut stored).cast(),
            );
            if stored == 0 {
                self.next_frame = None;
                return;
            }
            let now_ms = now.saturating_duration_since(self.epoch).as_millis() as u64;
            let mut next_ms = 0u64;
            self.next_frame =
                match ghostty_kitty_graphics_animation_tick(graphics, now_ms, &mut next_ms) {
                    GHOSTTY_SUCCESS => {
                        let delay = Duration::from_millis(next_ms.saturating_sub(now_ms));
                        Some(now + delay.max(MIN_FRAME))
                    }
                    _ => None,
                };
        }
    }

    /// When the next animation frame is due, while one plays.
    pub(super) fn next_frame(&self) -> Option<Instant> {
        self.next_frame
    }

    /// # Safety
    ///
    /// Called once, from the `Vt`'s `Drop`.
    pub(super) unsafe fn free(&mut self) {
        // SAFETY: the iterator was made in `new` and is freed only here.
        unsafe { ghostty_render_state_image_iterator_free(self.iter) };
    }
}

/// Image `id`'s pixels from the storage, as RGBA, if the storage holds them at `generation`.
unsafe fn pixels(graphics: GhosttyKittyGraphics, id: u32, generation: u64) -> Option<Image> {
    // SAFETY: the caller passes a live storage; every out pointer has its key's type, and the
    // pixels are copied before the next terminal call.
    unsafe {
        let image = ghostty_kitty_graphics_image(graphics, id);
        if image.is_null() {
            return None;
        }
        let mut stored = 0u64;
        let (mut width, mut height) = (0u32, 0u32);
        let mut format: GhosttyKittyImageFormat = GHOSTTY_KITTY_IMAGE_FORMAT_RGBA;
        let mut data = GhosttyString {
            ptr: ptr::null(),
            len: 0,
        };
        let keys = [
            GHOSTTY_KITTY_GRAPHICS_IMAGE_DATA_GENERATION,
            GHOSTTY_KITTY_GRAPHICS_IMAGE_DATA_WIDTH,
            GHOSTTY_KITTY_GRAPHICS_IMAGE_DATA_HEIGHT,
            GHOSTTY_KITTY_GRAPHICS_IMAGE_DATA_FORMAT,
            GHOSTTY_KITTY_GRAPHICS_IMAGE_DATA_PIXELS,
        ];
        let mut values: [*mut c_void; 5] = [
            (&raw mut stored).cast(),
            (&raw mut width).cast(),
            (&raw mut height).cast(),
            (&raw mut format).cast(),
            (&raw mut data).cast(),
        ];
        // PIXELS is NO_VALUE while the image is still arriving.
        if ghostty_kitty_graphics_image_get_multi(
            image,
            keys.len(),
            keys.as_ptr(),
            values.as_mut_ptr(),
            ptr::null_mut(),
        ) != GHOSTTY_SUCCESS
            || stored != generation
            || data.ptr.is_null()
        {
            return None;
        }
        let bytes = std::slice::from_raw_parts(data.ptr, data.len);
        let rgba = to_rgba(format, bytes, width as usize * height as usize)?;
        Some(Image {
            id,
            generation,
            width,
            height,
            rgba: rgba.into(),
        })
    }
}

/// `pixels` pixels of `format` as straight-alpha RGBA. `None` if the data is the wrong size.
fn to_rgba(format: GhosttyKittyImageFormat, data: &[u8], pixels: usize) -> Option<Vec<u8>> {
    let mut rgba = vec![0; pixels.checked_mul(4)?];
    expand(format, data, &mut rgba)?;
    Some(rgba)
}

/// Writes `data`, pixels of `format`, into `rgba` as straight-alpha RGBA. `None` if `data` isn't
/// as many pixels as `rgba` holds.
fn expand(format: GhosttyKittyImageFormat, data: &[u8], rgba: &mut [u8]) -> Option<()> {
    let channels = match format {
        GHOSTTY_KITTY_IMAGE_FORMAT_RGBA => 4,
        GHOSTTY_KITTY_IMAGE_FORMAT_RGB => 3,
        GHOSTTY_KITTY_IMAGE_FORMAT_GRAY_ALPHA => 2,
        GHOSTTY_KITTY_IMAGE_FORMAT_GRAY => 1,
        _ => return None,
    };
    if !rgba.len().is_multiple_of(4) || data.len() != rgba.len() / 4 * channels {
        return None;
    }
    let out = rgba.as_chunks_mut::<4>().0.iter_mut();
    match channels {
        4 => rgba.copy_from_slice(data),
        3 => {
            for (o, &[r, g, b]) in out.zip(data.as_chunks::<3>().0) {
                *o = [r, g, b, 255];
            }
        }
        2 => {
            for (o, &[g, a]) in out.zip(data.as_chunks::<2>().0) {
                *o = [g, g, g, a];
            }
        }
        _ => {
            for (o, &g) in out.zip(data) {
                *o = [g, g, g, 255];
            }
        }
    }
    Some(())
}

/// Installs the PNG decoder, once per process, before the first terminal is made: libghostty-vt
/// rejects PNG images without one, and `ghostty_sys_set` mustn't race another thread's use of
/// the library.
pub(super) fn install_png_decoder() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let decode: unsafe extern "C" fn(_, _, _, _, _) -> bool = decode_png;
        // SAFETY: the value is the callback itself, with `GhosttySysDecodePngFn`'s signature.
        ok(
            unsafe { ghostty_sys_set(GHOSTTY_SYS_OPT_DECODE_PNG, decode as *const c_void) },
            "ghostty_sys_set(DECODE_PNG)",
        );
    });
}

/// Decodes a PNG into RGBA pixels in a buffer from `allocator`, which libghostty-vt then owns.
/// The allocator caps an image's size, so the buffer is asked for from the header, before any
/// pixel is decoded: an image it refuses costs nothing.
unsafe extern "C" fn decode_png(
    _: *mut c_void,
    allocator: *const GhosttyAllocator,
    data: *const u8,
    len: usize,
    out: *mut GhosttySysImage,
) -> bool {
    // SAFETY: the library passes `len` readable bytes and a writable `out`, valid for this call.
    let bytes = unsafe { std::slice::from_raw_parts(data, len) };
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let Ok(mut reader) = decoder.read_info() else {
        return false;
    };
    let (width, height) = (reader.info().width, reader.info().height);
    let Some(size) = (width as usize)
        .checked_mul(height as usize)
        .and_then(|n| n.checked_mul(4))
    else {
        return false;
    };
    // SAFETY: `allocator` is the one the library passed. The buffer is zeroed before it's read
    // as a slice, and freed with the same allocator and size if decoding fails.
    unsafe {
        let buffer = ghostty_alloc(allocator, size);
        if buffer.is_null() {
            return false;
        }
        ptr::write_bytes(buffer, 0, size);
        if decode_into(&mut reader, std::slice::from_raw_parts_mut(buffer, size)).is_none() {
            ghostty_free(allocator, buffer, size);
            return false;
        }
        *out = GhosttySysImage {
            width,
            height,
            data: buffer,
            data_len: size,
        };
    }
    true
}

/// Decodes a PNG's first frame into `rgba`, its pixels as 8-bit straight-alpha RGBA: straight
/// in when the PNG is RGBA, else through one buffer of its own pixels.
fn decode_into<R: std::io::BufRead + std::io::Seek>(
    reader: &mut png::Reader<R>,
    rgba: &mut [u8],
) -> Option<()> {
    let (color, _) = reader.output_color_type();
    if color == png::ColorType::Rgba && reader.output_buffer_size() == Some(rgba.len()) {
        reader.next_frame(rgba).ok()?;
        return Some(());
    }
    let mut buf = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    let format = match info.color_type {
        png::ColorType::Rgba => GHOSTTY_KITTY_IMAGE_FORMAT_RGBA,
        png::ColorType::Rgb => GHOSTTY_KITTY_IMAGE_FORMAT_RGB,
        png::ColorType::GrayscaleAlpha => GHOSTTY_KITTY_IMAGE_FORMAT_GRAY_ALPHA,
        png::ColorType::Grayscale => GHOSTTY_KITTY_IMAGE_FORMAT_GRAY,
        // `normalize_to_color8` expands palettes.
        png::ColorType::Indexed => return None,
    };
    expand(format, &buf[..info.buffer_size()], rgba)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vt::{Config, Vt};

    /// The worker lets go of an image's pixels once no snapshot lists it.
    #[test]
    fn deleted_images_leave_the_cache() {
        let mut vt = Vt::new(&Config {
            cell_width: 10,
            cell_height: 20,
            ..Config::default()
        });
        // A 1 x 1 RGB image, placed, and another stored but never placed.
        vt.write(b"\x1b_Ga=T,f=24,s=1,v=1,i=1,q=2;AQID\x1b\\");
        vt.write(b"\x1b_Ga=t,f=24,s=1,v=1,i=2,q=2;AQID\x1b\\");
        assert_eq!(vt.snapshot().images.len(), 1);
        assert_eq!(vt.images.pixels.len(), 1);
        vt.write(b"\x1b_Ga=d,d=I,i=1,q=2\x1b\\");
        assert!(vt.snapshot().images.is_empty());
        assert!(vt.images.pixels.is_empty());
    }

    #[test]
    fn formats_become_rgba() {
        assert_eq!(
            to_rgba(GHOSTTY_KITTY_IMAGE_FORMAT_RGB, &[1, 2, 3], 1),
            Some(vec![1, 2, 3, 255])
        );
        assert_eq!(
            to_rgba(GHOSTTY_KITTY_IMAGE_FORMAT_GRAY_ALPHA, &[9, 128], 1),
            Some(vec![9, 9, 9, 128])
        );
        assert_eq!(
            to_rgba(GHOSTTY_KITTY_IMAGE_FORMAT_GRAY, &[7, 8], 2),
            Some(vec![7, 7, 7, 255, 8, 8, 8, 255])
        );
        assert_eq!(
            to_rgba(GHOSTTY_KITTY_IMAGE_FORMAT_RGBA, &[1, 2, 3], 1),
            None
        );
    }
}

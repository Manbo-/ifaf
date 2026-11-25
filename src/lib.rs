#![cfg(target_os = "windows")]

use png::{ColorType, Decoder, Limits, Transformations};
use std::ffi::{c_char, c_int, c_void};
use std::fs::File;
use std::io::{Cursor, Read, Seek, SeekFrom};
use std::os::windows::ffi::OsStringExt;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::PathBuf;
use std::ptr;

const AFFINITY_MAGIC: &[u8; 4] = b"\x00\xFFKA";
const PNG_SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";
const INPUT_MEMORY: u32 = 1;
const MAX_FILE_SIZE: u64 = 512 * 1024 * 1024;
const MAX_DECODED_BYTES: usize = 128 * 1024 * 1024;
const MAX_DIMENSION: u32 = 32_768;
const MAX_BITMAP_BYTES: usize = 128 * 1024 * 1024;
const MAX_PNG_CANDIDATES: usize = 1_024;
const BI_RGB: u32 = 0;
const LMEM_FIXED: u32 = 0;

#[repr(C, packed)]
pub struct SusiePictureInfo {
    pub left: i32,
    pub top: i32,
    pub width: i32,
    pub height: i32,
    pub x_density: u16,
    pub y_density: u16,
    pub color_depth: i16,
    pub padding: [u8; 2],
    pub h_info: *mut c_void,
}

#[repr(C)]
struct BitmapInfoHeader {
    size: u32,
    width: i32,
    height: i32,
    planes: u16,
    bit_count: u16,
    compression: u32,
    size_image: u32,
    x_pixels_per_meter: i32,
    y_pixels_per_meter: i32,
    colors_used: u32,
    colors_important: u32,
}

struct DecodedImage {
    width: u32,
    height: u32,
    dib_pixels: Vec<u8>,
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn LocalAlloc(flags: u32, bytes: usize) -> *mut c_void;
    fn LocalFree(memory: *mut c_void) -> *mut c_void;
    fn MultiByteToWideChar(
        code_page: u32,
        flags: u32,
        input: *const c_char,
        input_len: i32,
        output: *mut u16,
        output_len: i32,
    ) -> i32;
}

fn guarded<T>(fallback: T, f: impl FnOnce() -> T) -> T {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or(fallback)
}

fn has_affinity_magic(bytes: &[u8]) -> bool {
    bytes.starts_with(AFFINITY_MAGIC)
}

fn read_bounded(mut file: File, offset: u64) -> Result<Vec<u8>, String> {
    let metadata = file.metadata().map_err(|e| e.to_string())?;
    if offset > metadata.len() || metadata.len().saturating_sub(offset) > MAX_FILE_SIZE {
        return Err("file exceeds the configured size limit".into());
    }
    file.seek(SeekFrom::Start(offset))
        .map_err(|e| e.to_string())?;
    let capacity = (metadata.len() - offset).min(MAX_FILE_SIZE) as usize;
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(capacity)
        .map_err(|e| format!("cannot allocate input buffer: {e}"))?;
    file.take(MAX_FILE_SIZE + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_FILE_SIZE {
        return Err("file grew beyond the configured size limit".into());
    }
    Ok(bytes)
}

fn read_path(path: PathBuf, offset: u64) -> Result<Vec<u8>, String> {
    let file = File::open(path).map_err(|e| e.to_string())?;
    read_bounded(file, offset)
}

unsafe fn read_input(
    buffer: *const c_void,
    len: isize,
    flag: u32,
    path: Option<PathBuf>,
) -> Result<Vec<u8>, String> {
    let input_kind = flag & 0x7;
    if input_kind > INPUT_MEMORY {
        return Err("unsupported Susie input flag".into());
    }
    if input_kind == INPUT_MEMORY {
        if buffer.is_null() || len <= 0 || len as u64 > MAX_FILE_SIZE {
            return Err("invalid memory input".into());
        }
        let input = std::slice::from_raw_parts(buffer.cast::<u8>(), len as usize);
        let mut copy = Vec::new();
        copy.try_reserve_exact(input.len())
            .map_err(|e| format!("cannot allocate input buffer: {e}"))?;
        copy.extend_from_slice(input);
        return Ok(copy);
    }
    let path = path.ok_or_else(|| "missing input path".to_string())?;
    if !extension_is_af(&path) {
        return Err("unsupported file extension".into());
    }
    let offset = usize::try_from(len).map_err(|_| "invalid file offset".to_string())? as u64;
    read_path(path, offset)
}

fn decode_candidate(candidate: &[u8]) -> Result<DecodedImage, String> {
    let mut decoder = Decoder::new(Cursor::new(candidate));
    decoder.set_ignore_text_chunk(true);
    decoder.set_ignore_iccp_chunk(true);
    decoder.set_limits(Limits {
        bytes: MAX_DECODED_BYTES,
    });
    decoder.set_transformations(Transformations::EXPAND | Transformations::STRIP_16);

    let mut reader = decoder.read_info().map_err(|e| e.to_string())?;
    let width = reader.info().width;
    let height = reader.info().height;
    if width == 0 || height == 0 || width > MAX_DIMENSION || height > MAX_DIMENSION {
        return Err("PNG dimensions are outside supported limits".into());
    }

    let output_size = reader
        .output_buffer_size()
        .ok_or_else(|| "PNG output size overflow".to_string())?;
    if output_size == 0 || output_size > MAX_DECODED_BYTES {
        return Err("PNG decoded data exceeds the configured limit".into());
    }
    let mut decoded = Vec::new();
    decoded
        .try_reserve_exact(output_size)
        .map_err(|e| format!("cannot allocate decoded PNG buffer: {e}"))?;
    decoded.resize(output_size, 0);
    let info = reader.next_frame(&mut decoded).map_err(|e| e.to_string())?;
    reader.finish().map_err(|e| e.to_string())?;
    if info.width != width || info.height != height {
        return Err("unsupported PNG frame dimensions".into());
    }
    decoded.truncate(info.buffer_size());

    let channels = match info.color_type {
        ColorType::Grayscale => 1usize,
        ColorType::GrayscaleAlpha => 2,
        ColorType::Rgb => 3,
        ColorType::Rgba => 4,
        ColorType::Indexed => return Err("indexed PNG was not expanded".into()),
    };
    let expected = (width as usize)
        .checked_mul(height as usize)
        .and_then(|pixels| pixels.checked_mul(channels))
        .ok_or_else(|| "PNG pixel count overflow".to_string())?;
    if expected != decoded.len() {
        return Err("unexpected decoded PNG buffer length".into());
    }

    let unpadded_stride = (width as usize)
        .checked_mul(3)
        .ok_or_else(|| "bitmap row size overflow".to_string())?;
    let row_stride = unpadded_stride
        .checked_add(3)
        .ok_or_else(|| "bitmap row padding overflow".to_string())?
        & !3;
    let dib_size = row_stride
        .checked_mul(height as usize)
        .ok_or_else(|| "bitmap size overflow".to_string())?;
    if dib_size > MAX_BITMAP_BYTES || dib_size > u32::MAX as usize {
        return Err("bitmap exceeds the configured limit".into());
    }
    let mut dib_pixels = Vec::new();
    dib_pixels
        .try_reserve_exact(dib_size)
        .map_err(|e| format!("cannot allocate bitmap buffer: {e}"))?;
    dib_pixels.resize(dib_size, 0);

    for y in 0..height as usize {
        let source_row = y * width as usize * channels;
        let dest_y = height as usize - 1 - y; // Susie DIBs use bottom-up rows.
        let dest_row = dest_y * row_stride;
        for x in 0..width as usize {
            let si = source_row + x * channels;
            let (r, g, b, a) = match info.color_type {
                ColorType::Grayscale => (decoded[si], decoded[si], decoded[si], 255),
                ColorType::GrayscaleAlpha => {
                    (decoded[si], decoded[si], decoded[si], decoded[si + 1])
                }
                ColorType::Rgb => (decoded[si], decoded[si + 1], decoded[si + 2], 255),
                ColorType::Rgba => (
                    decoded[si],
                    decoded[si + 1],
                    decoded[si + 2],
                    decoded[si + 3],
                ),
                ColorType::Indexed => unreachable!(),
            };
            let alpha = a as u16;
            let white_blend = |channel: u8| -> u8 {
                ((channel as u16 * alpha + 255 * (255 - alpha) + 127) / 255) as u8
            };
            let di = dest_row + x * 3;
            dib_pixels[di] = white_blend(b);
            dib_pixels[di + 1] = white_blend(g);
            dib_pixels[di + 2] = white_blend(r);
        }
    }

    Ok(DecodedImage {
        width,
        height,
        dib_pixels,
    })
}

fn decode_first_preview(bytes: &[u8]) -> Result<DecodedImage, String> {
    if !has_affinity_magic(bytes) {
        return Err("missing Affinity file signature".into());
    }
    let mut search_from = AFFINITY_MAGIC.len();
    let mut candidates = 0usize;
    while search_from <= bytes.len().saturating_sub(PNG_SIGNATURE.len()) {
        let Some(relative) = bytes[search_from..]
            .windows(PNG_SIGNATURE.len())
            .position(|window| window == PNG_SIGNATURE)
        else {
            break;
        };
        let start = search_from + relative;
        search_from = start + PNG_SIGNATURE.len();
        candidates += 1;
        if candidates > MAX_PNG_CANDIDATES {
            return Err("too many PNG candidates in one file".into());
        }
        if let Ok(image) = decode_candidate(&bytes[start..]) {
            return Ok(image);
        }
    }
    Err("no valid embedded PNG preview found".into())
}

unsafe fn ansi_path(path: *const c_char) -> Option<PathBuf> {
    if path.is_null() {
        return None;
    }
    let required = MultiByteToWideChar(0, 0, path, -1, ptr::null_mut(), 0);
    if required <= 1 || required > 32_768 {
        return None;
    }
    let mut wide = vec![0u16; required as usize];
    let converted = MultiByteToWideChar(0, 0, path, -1, wide.as_mut_ptr(), required);
    if converted != required {
        return None;
    }
    wide.pop();
    Some(PathBuf::from(std::ffi::OsString::from_wide(&wide)))
}

unsafe fn wide_path(path: *const u16) -> Option<PathBuf> {
    if path.is_null() {
        return None;
    }
    let mut length = 0usize;
    while length < 32_768 && *path.add(length) != 0 {
        length += 1;
    }
    if length == 0 || length == 32_768 {
        return None;
    }
    let wide = std::slice::from_raw_parts(path, length);
    Some(PathBuf::from(std::ffi::OsString::from_wide(wide)))
}

fn extension_is_af(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("af"))
}

unsafe fn supported(header: *const c_void, path: Option<PathBuf>) -> bool {
    let Some(path) = path else { return false };
    if !extension_is_af(&path) || header.is_null() {
        return false;
    }
    let bytes = std::slice::from_raw_parts(header.cast::<u8>(), 4);
    has_affinity_magic(bytes)
}

fn plugin_info(infono: c_int) -> Option<&'static [u8]> {
    match infono {
        0 => Some(b"00IN\0"),
        1 => Some(b"Affinity .af preview (Rust)\0"),
        2 => Some(b"*.af\0"),
        3 => Some(b"Affinity .af image\0"),
        _ => None,
    }
}

#[no_mangle]
pub unsafe extern "system" fn GetPluginInfo(
    infono: c_int,
    buffer: *mut c_char,
    buffer_len: c_int,
) -> c_int {
    guarded(0, || {
        let Some(value) = plugin_info(infono) else {
            return 0;
        };
        if buffer.is_null() || buffer_len <= 0 {
            return 0;
        }
        let count = value.len().min(buffer_len as usize);
        ptr::copy_nonoverlapping(value.as_ptr(), buffer.cast::<u8>(), count);
        (count.saturating_sub(1)) as c_int
    })
}

#[no_mangle]
pub unsafe extern "system" fn GetPluginInfoW(
    infono: c_int,
    buffer: *mut u16,
    buffer_len: c_int,
) -> c_int {
    guarded(0, || {
        let Some(value) = plugin_info(infono) else {
            return 0;
        };
        if buffer.is_null() || buffer_len <= 0 {
            return 0;
        }
        let text = std::str::from_utf8(&value[..value.len() - 1]).unwrap_or("");
        let wide: Vec<u16> = text.encode_utf16().collect();
        let count = wide.len().saturating_add(1).min(buffer_len as usize);
        let chars_to_copy = count.saturating_sub(1);
        ptr::copy_nonoverlapping(wide.as_ptr(), buffer, chars_to_copy);
        *buffer.add(chars_to_copy) = 0;
        chars_to_copy as c_int
    })
}

#[no_mangle]
pub unsafe extern "system" fn IsSupported(
    filename: *const c_char,
    data_window: *const c_void,
) -> c_int {
    guarded(0, || {
        if supported(data_window, ansi_path(filename)) {
            1
        } else {
            0
        }
    })
}

#[no_mangle]
pub unsafe extern "system" fn IsSupportedW(
    filename: *const u16,
    data_window: *const c_void,
) -> c_int {
    guarded(0, || {
        if supported(data_window, wide_path(filename)) {
            1
        } else {
            0
        }
    })
}

unsafe fn picture_info(
    buffer: *const c_void,
    len: isize,
    flag: u32,
    output: *mut SusiePictureInfo,
    path: Option<PathBuf>,
) -> c_int {
    if output.is_null() {
        return 8;
    }
    if flag & 0x7 > INPUT_MEMORY
        || (flag & 0x7 == 0 && !path.as_deref().is_some_and(extension_is_af))
    {
        return 2;
    }
    let bytes = match read_input(buffer, len, flag, path) {
        Ok(bytes) => bytes,
        Err(_) => return 6,
    };
    if !has_affinity_magic(&bytes) {
        return 2;
    }
    let image = match decode_first_preview(&bytes) {
        Ok(image) => image,
        Err(_) => return 3,
    };
    let info = SusiePictureInfo {
        left: 0,
        top: 0,
        width: image.width as i32,
        height: image.height as i32,
        x_density: 0,
        y_density: 0,
        color_depth: 24,
        padding: [0; 2],
        h_info: ptr::null_mut(),
    };
    ptr::write_unaligned(output, info);
    0
}

#[no_mangle]
pub unsafe extern "system" fn GetPictureInfo(
    buffer: *const c_char,
    len: isize,
    flag: u32,
    output: *mut SusiePictureInfo,
) -> c_int {
    guarded(-1, || {
        picture_info(
            buffer.cast(),
            len,
            flag,
            output,
            if flag & 0x7 == INPUT_MEMORY {
                None
            } else {
                ansi_path(buffer)
            },
        )
    })
}

#[no_mangle]
pub unsafe extern "system" fn GetPictureInfoW(
    buffer: *const u16,
    len: isize,
    flag: u32,
    output: *mut SusiePictureInfo,
) -> c_int {
    guarded(-1, || {
        picture_info(
            buffer.cast(),
            len,
            flag,
            output,
            if flag & 0x7 == INPUT_MEMORY {
                None
            } else {
                wide_path(buffer)
            },
        )
    })
}

type SusieProgress = unsafe extern "system" fn(c_int, c_int, isize) -> c_int;

unsafe fn picture(
    buffer: *const c_void,
    len: isize,
    flag: u32,
    out_info: *mut *mut c_void,
    out_bits: *mut *mut c_void,
    progress: Option<SusieProgress>,
    data: isize,
    path: Option<PathBuf>,
) -> c_int {
    if out_info.is_null() || out_bits.is_null() {
        return 8;
    }
    *out_info = ptr::null_mut();
    *out_bits = ptr::null_mut();
    if flag & 0x7 > INPUT_MEMORY
        || (flag & 0x7 == 0 && !path.as_deref().is_some_and(extension_is_af))
    {
        return 2;
    }
    let bytes = match read_input(buffer, len, flag, path) {
        Ok(bytes) => bytes,
        Err(_) => return 6,
    };
    if !has_affinity_magic(&bytes) {
        return 2;
    }
    if let Some(callback) = progress {
        if callback(0, 1, data) != 0 {
            return 1;
        }
    }
    let image = match decode_first_preview(&bytes) {
        Ok(image) => image,
        Err(_) => return 3,
    };
    let bitmap_header = BitmapInfoHeader {
        size: std::mem::size_of::<BitmapInfoHeader>() as u32,
        width: image.width as i32,
        height: image.height as i32,
        planes: 1,
        bit_count: 24,
        compression: BI_RGB,
        size_image: image.dib_pixels.len() as u32,
        x_pixels_per_meter: 0,
        y_pixels_per_meter: 0,
        colors_used: 0,
        colors_important: 0,
    };
    let header_size = std::mem::size_of::<BitmapInfoHeader>();
    let info_handle = LocalAlloc(LMEM_FIXED, header_size);
    if info_handle.is_null() {
        return 4;
    }
    ptr::write_unaligned(info_handle.cast::<BitmapInfoHeader>(), bitmap_header);
    let bits_handle = LocalAlloc(LMEM_FIXED, image.dib_pixels.len());
    if bits_handle.is_null() {
        LocalFree(info_handle);
        return 4;
    }
    ptr::copy_nonoverlapping(
        image.dib_pixels.as_ptr(),
        bits_handle.cast::<u8>(),
        image.dib_pixels.len(),
    );
    *out_info = info_handle;
    *out_bits = bits_handle;
    if let Some(callback) = progress {
        if callback(1, 1, data) != 0 {
            LocalFree(info_handle);
            LocalFree(bits_handle);
            *out_info = ptr::null_mut();
            *out_bits = ptr::null_mut();
            return 1;
        }
    }
    0
}

#[no_mangle]
pub unsafe extern "system" fn GetPicture(
    buffer: *const c_char,
    len: isize,
    flag: u32,
    out_info: *mut *mut c_void,
    out_bits: *mut *mut c_void,
    progress: Option<SusieProgress>,
    data: isize,
) -> c_int {
    guarded(-1, || {
        picture(
            buffer.cast(),
            len,
            flag,
            out_info,
            out_bits,
            progress,
            data,
            if flag & 0x7 == INPUT_MEMORY {
                None
            } else {
                ansi_path(buffer)
            },
        )
    })
}

#[no_mangle]
pub unsafe extern "system" fn GetPictureW(
    buffer: *const u16,
    len: isize,
    flag: u32,
    out_info: *mut *mut c_void,
    out_bits: *mut *mut c_void,
    progress: Option<SusieProgress>,
    data: isize,
) -> c_int {
    guarded(-1, || {
        picture(
            buffer.cast(),
            len,
            flag,
            out_info,
            out_bits,
            progress,
            data,
            if flag & 0x7 == INPUT_MEMORY {
                None
            } else {
                wide_path(buffer)
            },
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(width: u32, height: u32, color: [u8; 4]) -> Vec<u8> {
        let mut bytes = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut bytes, width, height);
            encoder.set_color(ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().unwrap();
            let pixels = color.repeat((width * height) as usize);
            writer.write_image_data(&pixels).unwrap();
        }
        bytes
    }

    fn affinity_file(images: &[&[u8]]) -> Vec<u8> {
        let mut bytes = AFFINITY_MAGIC.to_vec();
        bytes.extend_from_slice(b"\x0c\0\0\0nsrP#Inf");
        for image in images {
            bytes.extend_from_slice(image);
        }
        bytes
    }

    #[test]
    fn rejects_wrong_magic_and_missing_preview() {
        assert!(decode_first_preview(b"not an Affinity file").is_err());
        assert!(decode_first_preview(&affinity_file(&[])).is_err());
    }

    #[test]
    fn returns_the_first_decodable_png_in_file_order() {
        let first = png(3, 2, [10, 20, 30, 255]);
        let second = png(7, 9, [90, 80, 70, 255]);
        let bytes = affinity_file(&[&first, &second]);
        let image = decode_first_preview(&bytes).unwrap();
        assert_eq!((image.width, image.height), (3, 2));
    }

    #[test]
    fn skips_a_bad_png_candidate_and_uses_the_next_valid_one() {
        let good = png(4, 5, [1, 2, 3, 255]);
        let bytes = affinity_file(&[PNG_SIGNATURE, &good]);
        let image = decode_first_preview(&bytes).unwrap();
        assert_eq!((image.width, image.height), (4, 5));
    }

    #[test]
    fn skips_a_png_with_an_invalid_iend_crc() {
        let mut corrupt = png(2, 2, [3, 4, 5, 255]);
        let last = corrupt.len() - 1;
        corrupt[last] ^= 0x01;
        let good = png(6, 3, [6, 7, 8, 255]);
        let bytes = affinity_file(&[&corrupt, &good]);
        let image = decode_first_preview(&bytes).unwrap();
        assert_eq!((image.width, image.height), (6, 3));
    }

    #[test]
    fn accepts_only_the_af_extension() {
        assert!(extension_is_af(std::path::Path::new("design.af")));
        assert!(extension_is_af(std::path::Path::new("DESIGN.AF")));
        assert!(!extension_is_af(std::path::Path::new("design.afphoto")));
        assert!(!extension_is_af(std::path::Path::new("design.png")));
    }

    #[test]
    fn enforces_dimension_limits() {
        assert!(decode_candidate(&png(MAX_DIMENSION + 1, 1, [0, 0, 0, 255])).is_err());
    }

    #[test]
    fn sample_fixture_contains_a_decodable_preview_when_available() {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("sample.af");
        if !path.exists() {
            return;
        }
        let bytes = read_path(path, 0).unwrap();
        let image = decode_first_preview(&bytes).unwrap();
        assert_eq!((image.width, image.height), (841, 1024));
    }
}

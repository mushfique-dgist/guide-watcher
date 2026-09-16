//! One strict PNG trust boundary for source renders, context frames, and
//! learner-facing visual inputs.

use std::io::Cursor;

const PNG_SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";
pub const MAX_PNG_INPUT_BYTES: u64 = 25 * 1024 * 1024;
pub const MAX_PNG_DECODED_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug)]
pub struct DecodedPng {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

pub fn inspect_rgba_size(bytes: &[u8], max_dimension: u32) -> Result<(u32, u32, usize), String> {
    if bytes.len() as u64 > MAX_PNG_INPUT_BYTES {
        return Err("PNG exceeds the 25 MiB compressed-input limit".to_string());
    }
    validate_envelope(bytes)?;
    let ihdr = bytes
        .get(8..33)
        .ok_or_else(|| "PNG IHDR chunk is truncated".to_string())?;
    if u32::from_be_bytes(ihdr[..4].try_into().expect("four-byte slice")) != 13
        || &ihdr[4..8] != b"IHDR"
    {
        return Err("PNG must begin with one 13-byte IHDR chunk".to_string());
    }
    let width = u32::from_be_bytes(ihdr[8..12].try_into().expect("four-byte slice"));
    let height = u32::from_be_bytes(ihdr[12..16].try_into().expect("four-byte slice"));
    if width == 0 || height == 0 || width > max_dimension || height > max_dimension {
        return Err(format!("PNG dimensions are invalid: {width}x{height}"));
    }
    let decoded_bytes = usize::try_from(width)
        .ok()
        .and_then(|width| {
            usize::try_from(height)
                .ok()
                .and_then(|height| width.checked_mul(height))
        })
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| "PNG decoded byte count overflows the platform size".to_string())?;
    if decoded_bytes > MAX_PNG_DECODED_BYTES {
        return Err("decoded PNG exceeds the 64 MiB limit".to_string());
    }
    Ok((width, height, decoded_bytes))
}

pub fn decode_rgba(bytes: &[u8], max_dimension: u32) -> Result<DecodedPng, String> {
    let (declared_width, declared_height, expected_rgba_bytes) =
        inspect_rgba_size(bytes, max_dimension)?;

    let mut decoder = png::Decoder::new_with_limits(
        Cursor::new(bytes),
        png::Limits {
            bytes: MAX_PNG_DECODED_BYTES,
        },
    );
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder
        .read_info()
        .map_err(|error| format!("PNG metadata is invalid: {error}"))?;
    let size = reader.output_buffer_size();
    if size > MAX_PNG_DECODED_BYTES {
        return Err("decoded PNG exceeds the 64 MiB limit".to_string());
    }
    let mut decoded = vec![0_u8; size];
    let info = reader
        .next_frame(&mut decoded)
        .map_err(|error| format!("PNG image data is invalid: {error}"))?;
    reader
        .finish()
        .map_err(|error| format!("PNG trailing chunks or data are invalid: {error}"))?;
    if info.width != declared_width || info.height != declared_height {
        return Err("decoded PNG dimensions differ from its IHDR declaration".to_string());
    }

    let pixels = &decoded[..info.buffer_size()];
    let rgba = match info.color_type {
        png::ColorType::Rgba => pixels.to_vec(),
        png::ColorType::Rgb => pixels
            .chunks_exact(3)
            .flat_map(|pixel| [pixel[0], pixel[1], pixel[2], 255])
            .collect(),
        png::ColorType::Grayscale => pixels
            .iter()
            .flat_map(|value| [*value, *value, *value, 255])
            .collect(),
        png::ColorType::GrayscaleAlpha => pixels
            .chunks_exact(2)
            .flat_map(|pixel| [pixel[0], pixel[0], pixel[0], pixel[1]])
            .collect(),
        png::ColorType::Indexed => {
            return Err("indexed PNG was not expanded by the decoder".to_string())
        }
    };
    if rgba.len() != expected_rgba_bytes {
        return Err("PNG decoded byte count is invalid".to_string());
    }
    Ok(DecodedPng {
        width: info.width,
        height: info.height,
        rgba,
    })
}

fn validate_envelope(bytes: &[u8]) -> Result<(), String> {
    if bytes.get(..PNG_SIGNATURE.len()) != Some(PNG_SIGNATURE) {
        return Err("PNG signature is missing or invalid".to_string());
    }

    let mut offset = PNG_SIGNATURE.len();
    while offset < bytes.len() {
        let header_end = offset
            .checked_add(8)
            .ok_or_else(|| "PNG chunk header overflows the input size".to_string())?;
        let header = bytes
            .get(offset..header_end)
            .ok_or_else(|| "PNG chunk header is truncated".to_string())?;
        let data_len =
            u32::from_be_bytes(header[..4].try_into().expect("four-byte slice")) as usize;
        let chunk_type = &header[4..8];
        let chunk_end = header_end
            .checked_add(data_len)
            .and_then(|end| end.checked_add(4))
            .ok_or_else(|| "PNG chunk size overflows the input size".to_string())?;
        if chunk_end > bytes.len() {
            return Err("PNG chunk data or CRC is truncated".to_string());
        }
        if chunk_type == b"IEND" {
            if data_len != 0 {
                return Err("PNG IEND chunk must be empty".to_string());
            }
            if chunk_end != bytes.len() {
                return Err("PNG contains bytes after IEND".to_string());
            }
            return Ok(());
        }
        offset = chunk_end;
    }

    Err("PNG IEND chunk is missing".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png_fixture() -> Vec<u8> {
        let mut bytes = Vec::new();
        let mut encoder = png::Encoder::new(&mut bytes, 1, 1);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(&[1, 2, 3, 255]).unwrap();
        writer.finish().unwrap();
        bytes
    }

    #[test]
    fn rejects_payload_after_iend() {
        let mut bytes = png_fixture();
        bytes.extend_from_slice(b"trailing payload");
        assert!(decode_rgba(&bytes, 8192)
            .unwrap_err()
            .contains("after IEND"));
    }
}

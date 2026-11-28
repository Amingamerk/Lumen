//! Turning a downloaded picture into thumbnails.

use std::io::Cursor;

use image::codecs::jpeg::JpegEncoder;
use image::imageops::{self, FilterType};
use image::{ImageReader, Limits};

use super::{ArtError, Size};

/// The widest or tallest picture that is decoded. Covers are at most 1,280.
const MAX_SIDE: u32 = 2_048;
/// What decoding may allocate at most.
const MAX_ALLOC: u64 = 48 * 1024 * 1024;
const QUALITY: u8 = 82;

/// Both thumbnails of a picture as JPEG, each cut square from the middle.
pub fn thumbnails(source: &[u8]) -> Result<Vec<(Size, Vec<u8>)>, ArtError> {
    let decode = |error: image::ImageError| ArtError::Decode(error.to_string());
    let mut reader = ImageReader::new(Cursor::new(source))
        .with_guessed_format()
        .map_err(|error| ArtError::Decode(error.to_string()))?;
    let mut limits = Limits::no_limits();
    limits.max_image_width = Some(MAX_SIDE);
    limits.max_image_height = Some(MAX_SIDE);
    limits.max_alloc = Some(MAX_ALLOC);
    reader.limits(limits);
    let picture = reader.decode().map_err(decode)?.to_rgb8();

    let side = picture.width().min(picture.height());
    let square = imageops::crop_imm(
        &picture,
        (picture.width() - side) / 2,
        (picture.height() - side) / 2,
        side,
        side,
    )
    .to_image();
    drop(picture);

    Size::ALL
        .into_iter()
        .map(|size| {
            let scaled = imageops::resize(&square, size.px(), size.px(), FilterType::Lanczos3);
            let mut jpeg = Vec::new();
            JpegEncoder::new_with_quality(&mut jpeg, QUALITY)
                .encode_image(&scaled)
                .map_err(decode)?;
            Ok((size, jpeg))
        })
        .collect()
}

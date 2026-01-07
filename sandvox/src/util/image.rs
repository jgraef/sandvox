use std::{
    ops::Deref,
    path::Path,
};

use nalgebra::Vector2;

pub trait ImageSizeExt {
    fn size(&self) -> Vector2<u32>;
}

impl<Pixel, Container> ImageSizeExt for image::ImageBuffer<Pixel, Container>
where
    Pixel: image::Pixel,
    Container: Deref<Target = [Pixel::Subpixel]>,
{
    fn size(&self) -> Vector2<u32> {
        Vector2::new(self.width(), self.height())
    }
}

pub trait ImageLoadExt: Sized {
    fn from_path<P: AsRef<Path>>(path: P) -> Result<Self, image::ImageError>;
}

impl ImageLoadExt for image::RgbaImage {
    fn from_path<P: AsRef<Path>>(path: P) -> Result<Self, image::ImageError> {
        let image = image::ImageReader::open(path)?.decode()?;
        Ok(image.to_rgba8())
    }
}

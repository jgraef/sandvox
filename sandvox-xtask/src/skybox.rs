use std::{
    f32::consts::{
        PI,
        TAU,
    },
    path::Path,
};

use color_eyre::eyre::{
    Error,
    bail,
};
use image::{
    GenericImageView,
    ImageReader,
    Pixel,
    Rgb,
    RgbImage,
    Rgba,
    imageops::sample_bilinear,
};
use nalgebra::{
    Vector2,
    Vector3,
};
use rayon::iter::{
    IndexedParallelIterator,
    IntoParallelRefMutIterator,
    ParallelIterator,
};

pub fn make_skybox(
    stars: impl AsRef<Path>,
    layers: impl IntoIterator<Item = impl AsRef<Path>>,
    size: u32,
    output: impl AsRef<Path>,
) -> Result<(), Error> {
    // use these (celestial) as input: https://svs.gsfc.nasa.gov/4851
    //
    // cubemap layout https://gpuweb.github.io/gpuweb/#texture-view-creation

    let output = output.as_ref();
    if !output.exists() {
        std::fs::create_dir_all(&output)?;
    }
    else if !output.is_dir() {
        bail!("--output must be a directory");
    }

    // the exr file we use would be a ImageRgb32F
    // for now we'll convert it to rgb8 (don't know how to properly convert after
    // sampling)
    tracing::debug!(path = ?stars.as_ref(), "Loading stars");
    let stars = ImageReader::open(stars)?.decode()?.to_rgb8();

    let layers = layers
        .into_iter()
        .map(|layer| {
            tracing::debug!(path = ?layer.as_ref(), "Loading layer");
            // the layers are ImageLuma8
            Ok(ImageReader::open(layer)?.decode()?.to_luma8())
        })
        .collect::<Result<Vec<_>, Error>>()?;

    let overlay_color: Rgb<u8> = Rgb([255, 255, 255]);

    let uv_scale = 1.0 / (size - 1) as f32;
    let mut face_images: [_; 6] = std::array::from_fn(|_| RgbImage::new(size, size));

    face_images
        .par_iter_mut()
        .enumerate()
        .for_each(|(face, face_image)| {
            face_image
                .par_enumerate_pixels_mut()
                .for_each(|(x, y, target_pixel)| {
                    let target_uv = Vector2::new(x, y).cast::<f32>() * uv_scale;
                    let source_uv = map_uv(target_uv, face);

                    let mut pixel = sample(&stars, source_uv).to_rgba();

                    for layer in &layers {
                        pixel.blend(&Rgba([
                            overlay_color[0],
                            overlay_color[1],
                            overlay_color[2],
                            sample(layer, source_uv).0[0],
                        ]));
                    }

                    *target_pixel = pixel.to_rgb();
                });
        });

    const FILENAMES: [&str; 6] = ["px.png", "nx.png", "py.png", "ny.png", "pz.png", "nz.png"];

    for i in 0..6 {
        let path = output.join(FILENAMES[i]);
        tracing::debug!(?path, "Saving image");
        face_images[i].save(&path)?;
    }

    Ok(())
}

fn map_uv(uv: Vector2<f32>, face: usize) -> Vector2<f32> {
    // map UV to [-1, 1]^2
    let uv = 2.0 * uv - Vector2::repeat(1.0);

    // vector pointing to UV coordinate in each face
    // https://gpuweb.github.io/gpuweb/#texture-view-creation
    let v: Vector3<f32> = match face {
        0 => [1.0, -uv.y, -uv.x],
        1 => [-1.0, -uv.y, uv.x],
        2 => [uv.x, 1.0, uv.y],
        3 => [uv.x, -1.0, -uv.y],
        4 => [uv.x, -uv.y, 1.0],
        5 => [-uv.x, -uv.y, -1.0],
        _ => unreachable!("invalid face index: {face}"),
    }
    .into();

    // convert to declination (latitude) and right ascension (longitude)
    // https://mechref.engr.illinois.edu/dyn/rvs.html

    let radius = v.norm();
    let right_ascension = v.x.atan2(v.z);
    let declination = (v.y / radius).asin();

    let mut uv = Vector2::new(right_ascension / TAU, -declination / PI + 0.5);
    if uv.x < 0.0 {
        uv.x += 1.0;
    }

    //tracing::debug!(?v, ?right_ascension, ?declination, ?uv);

    uv
}

#[inline]
fn sample<P>(image: &impl GenericImageView<Pixel = P>, uv: Vector2<f32>) -> P
where
    P: Pixel,
{
    sample_bilinear(image, uv.x, uv.y).unwrap_or_else(|| panic!("Can't sample: {uv:?}"))
}

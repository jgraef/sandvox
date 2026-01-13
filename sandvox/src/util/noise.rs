use std::ops::Mul;

use nalgebra::Point;
use rand::{
    Rng,
    distr::{
        Distribution,
        StandardUniform,
    },
};

pub trait NoiseFn<Point> {
    fn evaluate_at(&self, point: Point) -> f32;
}

impl<Point> NoiseFn<Point> for f32 {
    fn evaluate_at(&self, point: Point) -> f32 {
        let _ = point;
        *self
    }
}

pub trait NoiseFnExt {
    fn with_amplitude(self, amplitude: f32) -> WithAmplitude<Self>
    where
        Self: Sized;

    fn with_bias(self, bias: f32) -> WithBias<Self>
    where
        Self: Sized;

    fn with_frequency<Frequency>(self, frequency: Frequency) -> WithFrequency<Self, Frequency>
    where
        Self: Sized;
}

impl<T> NoiseFnExt for T {
    fn with_amplitude(self, amplitude: f32) -> WithAmplitude<Self>
    where
        Self: Sized,
    {
        WithAmplitude {
            amplitude,
            inner: self,
        }
    }

    fn with_bias(self, bias: f32) -> WithBias<Self>
    where
        Self: Sized,
    {
        WithBias { bias, inner: self }
    }

    fn with_frequency<Frequency>(self, frequency: Frequency) -> WithFrequency<Self, Frequency>
    where
        Self: Sized,
    {
        WithFrequency {
            frequency,
            inner: self,
        }
    }
}

macro_rules! impl_wrapper {
    ($name:ident => $inner:ident) => {
        #[derive(Clone, Copy, Debug)]
        pub struct $name {
            inner: ::noise::$inner,
        }

        impl NoiseFn<f32> for $name {
            fn evaluate_at(&self, point: f32) -> f32 {
                ::noise::NoiseFn::get(&self.inner, [point as f64]) as f32
            }
        }

        impl_wrapper!(@impl_for($name, 1));
        impl_wrapper!(@impl_for($name, 2));
        impl_wrapper!(@impl_for($name, 3));
        impl_wrapper!(@impl_for($name, 4));


        impl Distribution<$name> for StandardUniform {
            fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> $name {
                $name {
                    inner: ::noise::$inner::new(rng.random()),
                }
            }
        }

    };
    (@impl_for($name:ident, $n:expr)) => {
        impl NoiseFn<Point<f32, $n>> for $name {
            fn evaluate_at(&self, point: Point<f32, $n>) -> f32 {
                ::noise::NoiseFn::get(&self.inner, point.cast::<f64>().into()) as f32
            }
        }
    }
}

impl_wrapper!(PerlinNoise => Perlin);

#[derive(Clone, Copy, Debug)]
pub struct WithAmplitude<Inner> {
    pub amplitude: f32,
    pub inner: Inner,
}

impl<Inner> WithAmplitude<Inner> {
    pub fn new(amplitude: f32, inner: Inner) -> Self {
        Self { amplitude, inner }
    }
}

impl<Inner, Point> NoiseFn<Point> for WithAmplitude<Inner>
where
    Inner: NoiseFn<Point>,
{
    fn evaluate_at(&self, point: Point) -> f32 {
        self.amplitude * self.inner.evaluate_at(point)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct WithBias<Inner> {
    pub bias: f32,
    pub inner: Inner,
}

impl<Inner> WithBias<Inner> {
    pub fn new(bias: f32, inner: Inner) -> Self {
        Self { bias, inner }
    }
}

impl<Inner, Point> NoiseFn<Point> for WithBias<Inner>
where
    Inner: NoiseFn<Point>,
{
    fn evaluate_at(&self, point: Point) -> f32 {
        self.bias + self.inner.evaluate_at(point)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct WithFrequency<Inner, Frequency> {
    pub frequency: Frequency,
    pub inner: Inner,
}

impl<Inner, Frequency, Point> NoiseFn<Point> for WithFrequency<Inner, Frequency>
where
    Inner: NoiseFn<Point>,
    Frequency: Mul<Point, Output = Point> + Copy,
{
    fn evaluate_at(&self, point: Point) -> f32 {
        self.inner.evaluate_at(self.frequency * point)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Sum<A, B>(pub A, pub B);

impl<A, B, Point> NoiseFn<Point> for Sum<A, B>
where
    Point: Copy,
    A: NoiseFn<Point>,
    B: NoiseFn<Point>,
{
    fn evaluate_at(&self, point: Point) -> f32 {
        self.0.evaluate_at(point) + self.1.evaluate_at(point)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Product<A, B>(pub A, pub B);

impl<A, B, Point> NoiseFn<Point> for Product<A, B>
where
    Point: Copy,
    A: NoiseFn<Point>,
    B: NoiseFn<Point>,
{
    fn evaluate_at(&self, point: Point) -> f32 {
        self.0.evaluate_at(point) * self.1.evaluate_at(point)
    }
}

#[derive(Clone, Debug)]
pub struct FractalNoise<Inner> {
    pub octaves: Box<[Octave<Inner>]>,
}

impl<Inner> FractalNoise<Inner> {
    pub fn new(
        mut inner: impl FnMut() -> Inner,
        octaves: usize,
        base_frequency: f32,
        lacunarity: f32,
        persistence: f32,
    ) -> Self {
        let mut frequency = base_frequency;
        let mut amplitude = 1.0;

        Self {
            octaves: (0..octaves)
                .map(|_| {
                    let octave = inner().with_frequency(frequency).with_amplitude(amplitude);

                    frequency *= lacunarity;
                    amplitude *= persistence;

                    octave
                })
                .collect(),
        }
    }
}

impl<Inner, Point> NoiseFn<Point> for FractalNoise<Inner>
where
    Point: Copy,
    Inner: NoiseFn<Point>,
    f32: Mul<Point, Output = Point>,
{
    fn evaluate_at(&self, point: Point) -> f32 {
        self.octaves
            .iter()
            .map(|inner| inner.evaluate_at(point))
            .sum()
    }
}

pub type Octave<T> = WithAmplitude<WithFrequency<T, f32>>;

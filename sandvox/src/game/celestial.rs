use std::f64::consts::PI;

use chrono::{
    DateTime,
    Datelike,
    NaiveTime,
    Utc,
};
use nalgebra::{
    Point3,
    UnitQuaternion,
    Vector3,
};
use serde::{
    Deserialize,
    Serialize,
};

/// Geographic longitude and latitude (in radians)
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
pub struct GeoCoords<T> {
    pub longitude: T,
    pub latitude: T,
}

#[derive(Clone, Copy, Debug)]
pub struct CelestialFrame {
    jd: f64,
    mean_obliqueness: f64,
    hour_angle: f64,
    observer_position: GeoCoords<f64>,
}

impl CelestialFrame {
    pub fn new(observer_position: GeoCoords<f64>, time: DateTime<Utc>) -> Self {
        let jd = utc_to_jd(time);
        let mean_sidereal = astro::time::mn_sidr(jd);
        let mean_obliqueness = astro::ecliptic::mn_oblq_IAU(jd);
        let hour_angle = mean_sidereal + observer_position.longitude;

        Self {
            jd,
            mean_obliqueness,
            hour_angle,
            observer_position,
        }
    }

    fn ecliptic_to_world_rotation(&self, ecl_pos: astro::coords::EclPoint) -> UnitQuaternion<f32> {
        // equatorial
        let right_ascension =
            astro::coords::asc_frm_ecl(ecl_pos.long, ecl_pos.lat, self.mean_obliqueness);
        let declination =
            astro::coords::dec_frm_ecl(ecl_pos.long, ecl_pos.lat, self.mean_obliqueness);

        let hour_angle = self.hour_angle - right_ascension;

        // horizontal
        let altitude =
            astro::coords::alt_frm_eq(hour_angle, declination, self.observer_position.latitude);
        let azimuth =
            astro::coords::az_frm_eq(hour_angle, declination, self.observer_position.latitude);

        // not sure why we need that + PI, but then the sun is where it is supposed to
        // be :3
        (UnitQuaternion::from_axis_angle(&Vector3::y_axis(), azimuth + PI)
            * UnitQuaternion::from_axis_angle(&Vector3::x_axis(), -altitude))
        .cast()
    }

    pub fn sky(&self) -> UnitQuaternion<f32> {
        (UnitQuaternion::from_axis_angle(&Vector3::y_axis(), -self.hour_angle)
            * UnitQuaternion::from_axis_angle(&Vector3::x_axis(), -self.observer_position.latitude))
        .cast()
    }

    pub fn sun(&self) -> UnitQuaternion<f32> {
        let (ecl_pos, _distance_au) = astro::sun::geocent_ecl_pos(self.jd);
        self.ecliptic_to_world_rotation(ecl_pos)
    }

    pub fn moon(&self) -> UnitQuaternion<f32> {
        let (ecl_pos, _distance_km) = astro::lunar::geocent_ecl_pos(self.jd);
        self.ecliptic_to_world_rotation(ecl_pos)
    }
}

/// Calculates longitude and latitude from a world position
///
/// Just pretend we live on earth and ignore the fact that the surface is
/// flat and we can go straight forever without getting back to where we
/// started.
pub fn world_to_geo(world: Point3<f32>, origin: GeoCoords<f64>) -> GeoCoords<f64> {
    // kind of like living on a donut xD
    const WORLD_RADIUS: f64 = 6.371e6;
    let xy = world.coords.xy().cast() / WORLD_RADIUS;
    GeoCoords {
        longitude: xy.x + origin.longitude,
        latitude: xy.y + origin.latitude,
    }
}

#[inline]
fn utc_to_jd(time: DateTime<Utc>) -> f64 {
    astro::time::julian_day(&utc_to_astro_date(time))
}

fn utc_to_astro_date(time: DateTime<Utc>) -> astro::time::Date {
    let time = time.naive_utc();
    let start_of_day = time.date();
    let start_of_next_day = start_of_day.succ_opt().unwrap();
    let seconds_in_this_day = (start_of_next_day - start_of_day).as_seconds_f64();
    let seconds_since_start_of_day =
        (time - start_of_day.and_time(NaiveTime::default())).as_seconds_f64();

    astro::time::Date {
        year: time.year().try_into().unwrap(),
        month: time.month().try_into().unwrap(),
        decimal_day: time.day() as f64 + (seconds_since_start_of_day / seconds_in_this_day),
        cal_type: astro::time::CalType::Gregorian,
    }
}

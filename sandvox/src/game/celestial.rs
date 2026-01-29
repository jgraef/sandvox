use chrono::{
    DateTime,
    Datelike,
    NaiveTime,
    Utc,
};
use nalgebra::{
    Point3,
    UnitQuaternion,
    UnitVector3,
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

pub fn sun_orientation(observer: GeoCoords<f64>, time: DateTime<Utc>) -> UnitQuaternion<f32> {
    // time
    let jd = utc_to_jd(time);
    let mean_sidereal = astro::time::mn_sidr(jd);

    // ecliptic coordinates
    let (ecl_pos, _distance_au) = astro::sun::geocent_ecl_pos(jd);
    let mn_oblq = astro::ecliptic::mn_oblq_IAU(jd);

    // equatorial
    let right_ascension = astro::coords::asc_frm_ecl(ecl_pos.long, ecl_pos.lat, mn_oblq);
    let declination = astro::coords::dec_frm_ecl(ecl_pos.long, ecl_pos.lat, mn_oblq);
    let hour_angle = astro::coords::hr_angl_frm_observer_long(
        mean_sidereal,
        observer.longitude,
        right_ascension,
    );

    // horizontal
    let altitude = astro::coords::alt_frm_eq(hour_angle, declination, observer.latitude);
    let azimuth = astro::coords::az_frm_eq(hour_angle, declination, observer.latitude);

    let sun_vector = Vector3::new(azimuth.sin(), altitude.sin(), azimuth.cos());
    let sun_vector = UnitVector3::new_normalize(sun_vector);

    UnitQuaternion::from_axis_angle(&sun_vector, hour_angle).cast()
}

pub fn sky_orientation(observer: GeoCoords<f64>, time: DateTime<Utc>) -> UnitQuaternion<f32> {
    // time
    let jd = utc_to_jd(time);
    let mean_sidereal = astro::time::mn_sidr(jd);

    // observer's hour angle
    let hour_angle =
        astro::coords::hr_angl_frm_observer_long(mean_sidereal, observer.longitude, 0.0);

    (UnitQuaternion::from_axis_angle(&Vector3::y_axis(), -hour_angle)
        * UnitQuaternion::from_axis_angle(&Vector3::x_axis(), -observer.latitude))
    .cast()
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
    let time = time.naive_local();
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

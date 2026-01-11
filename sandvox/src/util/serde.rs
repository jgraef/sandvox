pub fn default_true() -> bool {
    true
}

pub mod power_preference {
    use std::borrow::Cow;

    use serde::{
        Deserialize,
        Deserializer,
        Serialize,
        Serializer,
        de::Error,
    };
    use wgpu::PowerPreference;

    pub fn serialize<S>(value: &PowerPreference, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let s = match value {
            PowerPreference::None => "none",
            PowerPreference::LowPower => "low",
            PowerPreference::HighPerformance => "high",
        };
        s.serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<PowerPreference, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s: Cow<'de, str> = Deserialize::deserialize(deserializer)?;
        match s.to_lowercase().as_str() {
            "low" => Ok(PowerPreference::LowPower),
            "high" => Ok(PowerPreference::HighPerformance),
            "none" => Ok(PowerPreference::None),
            _ => Err(Error::custom(format!("Invalid power preference: {s}"))),
        }
    }
}

pub mod backends {
    use std::borrow::Cow;

    use serde::{
        Deserialize,
        Deserializer,
        Serialize,
        Serializer,
        de::Error,
    };
    use wgpu::Backends;

    pub fn serialize<S>(value: &Backends, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        value
            .iter_names()
            .map(|(name, _)| name)
            .collect::<Vec<_>>()
            .serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Backends, D::Error>
    where
        D: Deserializer<'de>,
    {
        let v: Vec<Cow<'de, str>> = Deserialize::deserialize(deserializer)?;
        let mut b = Backends::empty();

        for v in v {
            b |= Backends::from_name(&v)
                .ok_or_else(|| Error::custom(format!("Invalid backend: {v}")))?;
        }

        Ok(b)
    }
}

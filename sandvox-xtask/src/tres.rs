use std::{
    collections::BTreeMap,
    path::Path,
};

use color_eyre::eyre::{
    Error,
    OptionExt,
};
use serde::Serialize;

#[derive(Clone, Copy, Debug, Serialize)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct Margin {
    pub left: u32,
    pub top: u32,
    pub right: u32,
    pub bottom: u32,
}

#[derive(Clone, Copy, Debug, Serialize)]
pub struct Asset {
    pub source: Rect,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub margin: Option<Margin>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_margin: Option<Margin>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expand_margin: Option<Margin>,
}

fn extract_asset(tres: &str) -> Option<Asset> {
    let mut source = None;
    let mut margin = None;
    let mut content_margin = None;
    let mut expand_margin = None;

    fn parse_margin(k: &str, v: &str, output: &mut Option<Margin>) {
        if let Ok(value) = v.parse::<f32>() {
            let output = output.get_or_insert_default();
            let output = match k {
                "left" => &mut output.left,
                "right" => &mut output.right,
                "top" => &mut output.top,
                "bottom" => &mut output.bottom,
                _ => return,
            };
            *output = value.max(0.0) as u32;
        }
    }

    for line in tres.lines() {
        if let Some((k, v)) = line.split_once('=') {
            let k = k.trim();
            let v = v.trim();

            if k == "region_rect" || k == "region" {
                let v = v.strip_prefix("Rect2(").unwrap().strip_suffix(")").unwrap();

                let mut values = v.split(", ").map(|s| s.parse::<u32>().unwrap());

                source = Some(Rect {
                    x: values.next().unwrap(),
                    y: values.next().unwrap(),
                    width: values.next().unwrap(),
                    height: values.next().unwrap(),
                });
            }
            else if let Some(k) = k.strip_prefix("content_margin_") {
                parse_margin(k, v, &mut content_margin);
            }
            else if let Some(k) = k.strip_prefix("margin_") {
                parse_margin(k, v, &mut margin);
            }
            else if let Some(k) = k.strip_prefix("expand_margin_") {
                parse_margin(k, v, &mut expand_margin);
            }
        }
    }

    Some(Asset {
        source: source?,
        margin,
        content_margin,
        expand_margin,
    })
}

fn parse_single(path: impl AsRef<Path>) -> Result<Option<(String, Asset)>, Error> {
    let path = path.as_ref();
    tracing::debug!(path = %path.display(), "parsing file");

    let file_name = path
        .file_stem()
        .ok_or_eyre("No filestem")?
        .to_str()
        .ok_or_eyre("Invalid filename")?;
    let tres = std::fs::read_to_string(&path)?;
    Ok(extract_asset(&tres).map(|rect| (file_name.to_owned(), rect)))
}

fn parse_recursive(
    path: impl AsRef<Path>,
    output: &mut BTreeMap<String, Asset>,
) -> Result<(), Error> {
    for result in std::fs::read_dir(path)? {
        let dir_entry = result?;
        let child_path = dir_entry.path();

        if dir_entry.file_type()?.is_dir() {
            parse_recursive(&child_path, output)?;
        }
        else if child_path.extension().is_some_and(|ext| ext == "tres") {
            output.extend(parse_single(&child_path)?);
        }
    }
    Ok(())
}

pub fn parse_tres(path: impl AsRef<Path>, recursive: bool) -> Result<(), Error> {
    let mut output = BTreeMap::new();

    if recursive {
        parse_recursive(path, &mut output)?;
    }
    else {
        let (name, rect) = parse_single(path)?.ok_or_eyre("No Rect found")?;
        if output.insert(name.clone(), rect).is_some() {
            tracing::warn!(name, "duplicate");
        }
    }

    let json = serde_json::to_string_pretty(&output)?;
    println!("{json}");

    Ok(())
}

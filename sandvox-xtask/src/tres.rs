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
    pub w: u32,
    pub h: u32,
}

fn extract_rect(mut tres: &str) -> Option<Rect> {
    const START_PATTERNS: &[&str] = &["region_rect = Rect2(", "region = Rect2("];

    for pat in START_PATTERNS {
        if let Some(pos) = tres.find(pat) {
            tres = &tres[(pos + pat.len())..];
            let end = tres.find(')').unwrap();
            let rect_str = &tres[..end];

            let mut values = rect_str.split(", ").map(|s| s.parse::<u32>().unwrap());

            return Some(Rect {
                x: values.next().unwrap(),
                y: values.next().unwrap(),
                w: values.next().unwrap(),
                h: values.next().unwrap(),
            });
        }
    }

    None
}

fn parse_single(path: impl AsRef<Path>) -> Result<Option<(String, Rect)>, Error> {
    let path = path.as_ref();
    tracing::debug!(path = %path.display(), "parsing file");

    let file_name = path
        .file_stem()
        .ok_or_eyre("No filestem")?
        .to_str()
        .ok_or_eyre("Invalid filename")?;
    let tres = std::fs::read_to_string(&path)?;
    Ok(extract_rect(&tres).map(|rect| (file_name.to_owned(), rect)))
}

fn parse_recursive(
    path: impl AsRef<Path>,
    output: &mut BTreeMap<String, Rect>,
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
        output.insert(name, rect);
    }

    let json = serde_json::to_string_pretty(&output)?;
    println!("{json}");

    Ok(())
}

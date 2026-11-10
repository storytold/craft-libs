//! Renders a [`ProjectReport`] as plain text for the terminal.

use super::report::{Outcome, ProjectReport};
use super::tally::{Tally, Verification};
use super::units::megabytes;
use std::fmt::{self, Display, Formatter};

/// How many document classes to list.
const TOP_CLASSES: usize = 15;

pub struct TextReport<'a>(pub &'a ProjectReport);

impl Display for TextReport<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let r = self.0;
        writeln!(f, "{}", r.path.display())?;
        writeln!(f, "  {} · {} datasets", megabytes(r.file_bytes), r.datasets)?;
        section(f, "Saved with version", &r.settings, |f, s| match s {
            Some(s) => {
                let v = |v: Option<_>| v.map_or("unknown".to_string(), |v: crate::decode::settings::SavedWithVersion| v.to_string());
                writeln!(f, "    created with {}, last saved with {}", v(s.created_with), v(s.last_saved_with))
            }
            None => writeln!(f, "    not recorded (older project)"),
        })?;
        section(f, "Mesh", &r.mesh, |f, m| {
            for mesh in &m.meshes {
                let set = m.materials.get(mesh.material_index as usize).map_or("?", |s| s.name.as_str());
                writeln!(f, "    {:<28} {:>9} verts {:>9} faces  {} UV  → {}", mesh.name, mesh.vertices, mesh.faces, mesh.uv_channels, set)?;
            }
            writeln!(f, "    texture sets: {}", m.materials.iter().map(|s| s.name.as_str()).collect::<Vec<_>>().join(", "))
        })?;
        section(f, "Resource index", &r.toc, |f, toc| match toc {
            Some(t) => writeln!(f, "    {} named entries", t.entries.len()),
            None => writeln!(f, "    none"),
        })?;
        section(f, "Resources", &r.resources, |f, s| {
            tally(f, &s.kinds)?;
            writeln!(f, "    images by format:")?;
            tally(f, &s.image_formats)?;
            failures(f, "unreadable image headers", &s.unreadable_headers)?;
            pixels(f, &s.pixels)
        })?;
        section(f, "Texture cache", &r.cache, |f, s| {
            tally(f, &s.formats)?;
            writeln!(f, "    entries by channel bit: {}", s.channels.rows().iter().map(|(k, n, _)| format!("{k}×{n}")).collect::<Vec<_>>().join("  "))?;
            failures(f, "unreadable cache headers", &s.unreadable_headers)?;
            pixels(f, &s.pixels)
        })?;
        section(f, "Layer document", &r.document, |f, c| {
            writeln!(f, "    {} objects in {} classes", c.total(), c.by_frequency().len())?;
            for (class, n) in c.by_frequency().into_iter().take(TOP_CLASSES) {
                writeln!(f, "    {n:>8}  {class}")?;
            }
            Ok(())
        })
    }
}

fn section<T>(f: &mut Formatter<'_>, title: &str, o: &Outcome<T>, body: impl FnOnce(&mut Formatter<'_>, &T) -> fmt::Result) -> fmt::Result {
    writeln!(f, "  {title}")?;
    match o {
        Ok(v) => body(f, v),
        Err(e) => writeln!(f, "    FAILED: {e}"),
    }
}

fn tally(f: &mut Formatter<'_>, t: &Tally) -> fmt::Result {
    for (label, n, bytes) in t.rows() {
        writeln!(f, "    {n:>6}  {label:<34} {}", megabytes(bytes))?;
    }
    Ok(())
}

fn failures(f: &mut Formatter<'_>, title: &str, items: &[(String, String)]) -> fmt::Result {
    if !items.is_empty() {
        writeln!(f, "    {title}: {}", items.len())?;
        for (item, reason) in items {
            writeln!(f, "      {item}: {reason}")?;
        }
    }
    Ok(())
}

fn pixels(f: &mut Formatter<'_>, v: &Option<Verification>) -> fmt::Result {
    match v {
        Some(v) => {
            writeln!(f, "    pixels decompressed: {}/{}", v.passed, v.checked())?;
            failures(f, "pixel failures", &v.failed)
        }
        None => Ok(()),
    }
}

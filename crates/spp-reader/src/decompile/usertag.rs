//! The `usertag` of a graph's image inputs and outputs: `key=value` pairs separated by `;`, read from the package
//! description.

/// Whether an image input's or output's `usertag` declares the working colour space (`colorspace=$working`): the
/// reference app gives such an input linear values on a colour channel and takes such an output back as linear.
pub fn working_space(usertag: &str) -> bool {
    usertag.split(';').any(|kv| kv.trim() == "colorspace=$working")
}

/// The colour in an image input's `usertag` (`defaultcolor=(128,128,255)`, 8-bit). `None` unless the first three fields
/// are numbers from 0 to 255; further fields (an alpha) are ignored.
pub fn default_color(usertag: &str) -> Option<[f32; 3]> {
    let (_, rest) = usertag.split_once("defaultcolor=(")?;
    let (list, _) = rest.split_once(')')?;
    let mut fields = list.split(',').map(|c| c.trim().parse::<f32>().ok().filter(|v| (0.0..=255.0).contains(v)));
    let (r, g, b) = (fields.next()??, fields.next()??, fields.next()??);
    Some([r / 255.0, g / 255.0, b / 255.0])
}

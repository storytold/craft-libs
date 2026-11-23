//! Decode the video of a Bink file to raw I420 (`binkdec in.bik out.yuv`), for comparison with
//! other decoders. Prints `ok N` for each frame that decodes and `ERR N message` for each that
//! does not; a frame that fails writes no picture.

use std::io::Write;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let [_, input, output] = args.as_slice() else {
        return Err("usage: binkdec in.bik out.yuv".into());
    };
    // creating the output truncates it: refuse when it is the input under another name
    if let (Ok(a), Ok(b)) = (std::fs::canonicalize(input), std::fs::canonicalize(output))
        && a == b
    {
        return Err("the output would overwrite the input".into());
    }
    let file = std::fs::File::open(input)?;
    let bink = craft_media::bink::open(&file)?;
    let mut dec = craft_video::bink::Decoder::from_params(&bink.video_params())?;
    let mut out = std::io::BufWriter::new(std::fs::File::create(output)?);
    for i in 0..bink.frames.len() {
        let frame = bink.read_frame(&file, i)?;
        match dec.decode(bink.split_frame(&frame)?.video) {
            Ok(pic) => {
                out.write_all(&pic.y)?;
                out.write_all(&pic.u)?;
                out.write_all(&pic.v)?;
                println!("ok {i}");
            }
            Err(e) => println!("ERR {i} {e}"),
        }
    }
    out.flush()?;
    Ok(())
}
